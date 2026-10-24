//! The account hold after a bounced cheque (wl-sales §3 *Customer account*,
//! §4.2 *Account hold*, §5.7 step 4, §6 `SAL.RUL.05`, `SAL.ACL.03`; BL-SAL-12).
//!
//! `SAL.RUL.05` on `ChequeBounced` (`wl-payments`), when the drawer has a
//! customer account:
//! - the account (and, with *hold the whole credit group* on, every account
//!   of its group) gets an [`AccountHoldSource`] naming the cheque; the same
//!   cheque twice adds one, a further cheque adds another, and the flag stays
//!   while any source is open;
//! - every order on account of those customers that still has open quantity
//!   (`confirmed`, `partially_fulfilled`) goes on `credit_hold` through
//!   `hold`, keeping its reservations; dispatched goods and cancelled or
//!   delivered orders are not touched;
//! - the sales rep and the accountants are told.
//!
//! While the flag is set `submit` of an order on account and a counter sale
//! on account are refused (the order stays `draft`; cash, card or online
//! payment is still allowed), and the credit check of `accept` and the
//! re-check at dispatch fail ([`crate::credit`]). The flag is removed only by
//! *Clear account hold* ([`crate::account_hold_clearing`]), requested with
//! [`request_clearing`] and approved by another credit releaser.
//!
//! Decisions:
//! - The group is resolved here from the drawer's account: `wl-payments`
//!   sends no credit group in `ChequeBounced`.
//! - The rule is not switched by the credit-check setting: a bounced cheque
//!   holds the account whether or not limits are checked.
//! - Roles without `credit.view` never see the reason: the refusals say only
//!   *account on hold* and the pages show the sources to `credit.view`.

use renox::audit::{self, Entry};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::approvals::{self, ApprovalDecided, ApprovalEnd};
use wl_kernel::cheques::ChequeBounced;

use crate::account_hold_clearing::{
    AccountHoldClearing, CLEARING_SUBJECT_TYPE, ClearingStatus, KIND_OTHER, KINDS, REJECT, REQUEST,
    REQUEST_APPROVED,
};
use crate::config::config_of;
use crate::credit;
use crate::customer_account::{self, AccountHoldSource, CustomerAccount, REASON_BOUNCED_CHEQUE};
use crate::notices;
use crate::sale_line::{KIND_ITEM, SaleLine};
use crate::sales_order::{SalesOrder, SalesOrderStatus};
use crate::sales_order_machine::HoldInput;
use crate::texts::english;
use wl_kernel::sales::CreditFailure;

/// A 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// Whether the customer `party` has an account that is on hold.
pub async fn is_on_hold(db: &Db, party: i64) -> Result<bool> {
    match CustomerAccount::of_party(db, party).await? {
        Some(account) => account.is_on_hold(db).await,
        None => Ok(false),
    }
}

/// Whether `order` still has open quantity to dispatch.
fn has_open_quantity(lines: &[SaleLine]) -> bool {
    lines
        .iter()
        .any(|line| line.kind == KIND_ITEM && line.open_milli() > 0)
}

/// `SAL.RUL.05`: the cheque `event` bounced. Returns the accounts newly put
/// on hold.
pub async fn on_cheque_bounced(
    state: &AppState,
    event: &ChequeBounced,
) -> Result<Vec<CustomerAccount>> {
    let Some(drawer) = CustomerAccount::of_party(&state.db, event.party.0).await? else {
        return Ok(Vec::new());
    };
    let accounts = if config_of(state).bounced_cheque_hold_group {
        drawer.group_members(&state.db).await?
    } else {
        vec![drawer]
    };
    let source = event.cheque.subject();
    let mut flagged = Vec::new();
    for account in accounts {
        if customer_account::add_source(&state.db, &account, &source, REASON_BOUNCED_CHEQUE).await?
        {
            audit::record(
                &state.db,
                Entry::new("sales.account_hold_set")
                    .subject(CustomerAccount::TABLE, account.id)
                    .data(json!({
                        "source": source.to_string(),
                        "reason": REASON_BOUNCED_CHEQUE,
                    })),
            )
            .await?;
            flagged.push(account);
        }
    }
    for account in &flagged {
        let held = hold_open_orders(state, account).await?;
        notices::account_on_hold(state, account, &source.to_string(), held).await?;
    }
    Ok(flagged)
}

/// Puts the orders on account of `account`'s customer that are not yet
/// dispatched on credit hold; returns how many.
async fn hold_open_orders(state: &AppState, account: &CustomerAccount) -> Result<i64> {
    let mut held = 0;
    for order in SalesOrder::query()
        .where_(SalesOrder::PARTY_ID.eq(account.party_id))
        .where_(SalesOrder::STATUS.is_in([
            SalesOrderStatus::Confirmed,
            SalesOrderStatus::PartiallyFulfilled,
        ]))
        .order_by(SalesOrder::ID)
        .get(&state.db)
        .await?
    {
        if !order.is_on_account() {
            continue;
        }
        let lines = SaleLine::of(&state.db, &order.subject()).await?;
        if !has_open_quantity(&lines) {
            continue;
        }
        let found = credit::assess(
            state,
            account,
            credit::undelivered_value(&lines),
            Some(order.id),
        )
        .await?;
        let hold = found.hold().unwrap_or(HoldInput {
            reason: CreditFailure::AccountOnHold,
            open_balance: found.open_balance(),
            overdue: found.overdue,
            limit: found.limit,
        });
        credit::hold_order(state, &order, hold).await?;
        held += 1;
    }
    Ok(held)
}

/// A holder of `credit.release`, `by`, asks to clear the open source
/// `source` of `account`'s hold, for `kind` (`cheque_replaced`,
/// `balance_paid`, `other`) with a `note` (needed for `other`). Another
/// holder approves.
///
/// # Errors
///
/// 422 when the reason is unknown, `other` has no note, the source is not an
/// open one of the account, or a clearing of it already waits.
pub async fn request_clearing(
    state: &AppState,
    by: i64,
    account: i64,
    source: i64,
    kind: &str,
    note: &str,
) -> Result<AccountHoldClearing> {
    if !KINDS.contains(&kind) {
        return Err(refused("wl_sales.credit.problem.kind"));
    }
    if kind == KIND_OTHER && note.trim().is_empty() {
        return Err(refused("wl_sales.credit.problem.note"));
    }
    let held = AccountHoldSource::find(&state.db, source)
        .await?
        .filter(|s| s.account_id == account && s.is_open())
        .ok_or_else(|| refused("wl_sales.credit.problem.no_source"))?;
    let waiting = AccountHoldClearing::query()
        .where_(AccountHoldClearing::HOLD_SOURCE_ID.eq(held.id))
        .where_(AccountHoldClearing::STATUS.eq(ClearingStatus::PendingApproval))
        .first(&state.db)
        .await?;
    if waiting.is_some() {
        return Err(refused("wl_sales.credit.problem.clearing_pending"));
    }
    let saved = AccountHoldClearing::create(
        &state.db,
        AccountHoldClearing {
            account_id: account,
            hold_source_id: held.id,
            reason_kind: kind.to_owned(),
            note: (!note.trim().is_empty()).then(|| note.trim().to_owned()),
            requested_by: by,
            ..AccountHoldClearing::default()
        },
    )
    .await?;
    let id = saved.id;
    let mut tx = state.db.begin().await?;
    let outcome = saved
        .transition(REQUEST)
        .by_system()
        .run_in(&mut tx)
        .await
        .map_err(|e| e.into_error());
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            drop(tx);
            AccountHoldClearing::query()
                .where_(AccountHoldClearing::ID.eq(id))
                .where_(AccountHoldClearing::STATUS.eq(ClearingStatus::Draft))
                .delete(&state.db)
                .await?;
            return Err(error);
        }
    };
    tx.commit().await?;
    approvals::announce(state, &outcome).await?;
    outcome.emit(state).await?;
    audit::record(
        &state.db,
        Entry::new("sales.account_hold_clear_requested")
            .user(by)
            .subject(CustomerAccount::TABLE, account)
            .data(json!({
                "clearing": id,
                "source": held.source().to_string(),
                "reason": kind,
                "note": note.trim(),
            })),
    )
    .await?;
    AccountHoldClearing::find_or_404(&state.db, id).await
}

/// A request that ended without a yes runs `reject`.
async fn decided(state: &AppState, event: &ApprovalDecided) -> Result {
    if event.subject.subject_type != CLEARING_SUBJECT_TYPE
        || event.state_returned_to.is_none()
        || event.decision == ApprovalEnd::Approved
    {
        return Ok(());
    }
    let Some(clearing) = AccountHoldClearing::find(&state.db, event.subject.subject_id).await?
    else {
        return Ok(());
    };
    if clearing.status != ClearingStatus::Draft {
        return Ok(());
    }
    let mut run = clearing.transition(REJECT).by_system();
    if let Some(reason) = clearing.outcome_reason.as_deref() {
        run = run.reason(reason);
    }
    let _ = run.run(state).await.map_err(|e| e.into_error())?;
    Ok(())
}

/// Audits the clearing after it was approved.
async fn cleared(state: &AppState, event: &Transitioned<AccountHoldClearing>) -> Result {
    if event.transition != REQUEST_APPROVED {
        return Ok(());
    }
    let Some(clearing) = AccountHoldClearing::find(&state.db, event.key).await? else {
        return Ok(());
    };
    let mut entry = Entry::new("sales.account_hold_cleared")
        .subject(CustomerAccount::TABLE, clearing.account_id)
        .data(json!({
            "clearing": clearing.id,
            "source": clearing.hold_source_id,
            "reason": clearing.reason_kind,
            "requested_by": clearing.requested_by,
        }));
    if let Some(approver) = clearing.approved_by {
        entry = entry.user(approver);
    }
    audit::record(&state.db, entry).await?;
    Ok(())
}

/// Registers the clearing's machine and policy, the bounced cheque rule and
/// the listeners.
pub(crate) fn register(app: &mut Registry) {
    crate::account_hold_clearing::subject_machine().register();
    renox::state_machine::register::<AccountHoldClearing>();
    crate::policy::clear_hold_policy().register();
    approvals::register_subject_link(CLEARING_SUBJECT_TYPE, |id| {
        format!("/credit/clearings/{id}")
    });
    app.listen(|event: ChequeBounced, state: AppState| async move {
        on_cheque_bounced(&state, &event).await.map(|_| ())
    });
    app.listen(
        |event: ApprovalDecided, state: AppState| async move { decided(&state, &event).await },
    );
    app.listen(
        |event: Transitioned<AccountHoldClearing>, state: AppState| async move {
            cleared(&state, &event).await
        },
    );
}
