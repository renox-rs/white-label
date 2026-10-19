//! Settling a published owner statement: pay out a positive balance, carry
//! forward a balance of zero or less, or invoice a negative balance to the
//! owner (01 Property §4.4 step 3 and rows "The owner's balance is negative"
//! and "The owner is blocked", §7.1 *Owner statement*, §3.3 `PRP.ACL.08`,
//! `PRP.ACL.21`, `PTY.ACL.03`; BL-PRP-20).
//!
//! | Transition | By | Guards | What it does |
//! |---|---|---|---|
//! | `pay_out` | the accountant (`owner_statements.prepare`) | balance above 0; not disputed; the owner not blocked; approved by a property manager other than the requester | after the commit the payout of the full balance is recorded as an outgoing payment (`FIN.PAY.01`); no partial payouts |
//! | `carry_forward` | the accountant | balance of 0 or less | the balance is brought forward on the next statement |
//! | `invoice_owner` | a property manager (`owner_statements.approve`) | balance below 0; the agreement allows it (`may_invoice_negative_balance`) or the latest three statements that count, this one included, ended negative; approved by a property manager other than the requester | after the commit an invoice for the negative balance goes to the owner from the management company's entity |
//!
//! **Approvals.** `pay_out` and `invoice_owner` are deferred by the kernel as
//! approval requests of one step by a property manager, never the requester
//! (`PRP.ACL.08`, [`payout_policy`], [`invoice_policy`]); at the final approval
//! the kernel runs `pay_out_approved` or `invoice_owner_approved`, which have
//! the same guards on the statement as it is then. The block guard of the owner
//! is checked when `pay_out` is asked ([`pay_out`]) and again at the final
//! approval (the kernel's re-check for a guarded transition): a blocked owner
//! is refused with the neutral text, and an approved `PTY.ACL.03` exception
//! lets one payout run ([`PayOutOutcome::ExceptionRequested`]).
//!
//! **The money** follows the commit, as for a lease termination: the payment
//! and the invoice are made by [`after`], once, from the event of the
//! transition; the statement is `settled` and keeps the amount
//! ([`OwnerStatement::payout_amount`]) even if recording the payment fails
//! and the listener has to be run again.
//!
//! Decided where the spec is silent: the VAT of the owner invoice comes from
//! the charge type *recharged costs*, configured by the client (no platform
//! default while A-08 is open), and the invoice is refused until it is; the
//! "latest three" statements are the three with the latest period that are not
//! superseded and do not end after this one; `carry_forward` is not stopped by
//! a dispute (only the payout is, `PRP.ACL.21`).

use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{Outcome, StateMachine, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::block_guard::{self, GuardedError};
use wl_kernel::entities::LegalEntity;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, invoicer};
use wl_kernel::payments::{PaymentMethod, RecordPayment, payment_recorder};

use crate::agreement::ManagementAgreement;
use crate::building::refused;
use crate::charge_type::ChargeType;
use crate::statement::{OwnerStatement, StatementStatus};
use crate::statement_machine::{INVOICE_OWNER, INVOICE_OWNER_APPROVED, PAY_OUT, PAY_OUT_APPROVED};
use crate::texts::english;

/// The code of the charge type the owner invoice is made with.
pub const RECHARGED_COSTS: &str = "recharged_costs";

/// How many statements in a row that ended negative let the property manager
/// invoice the owner without the agreement's clause.
pub const NEGATIVE_STATEMENTS: usize = 3;

/// What the service gathered for the guards of `invoice_owner` (they have no
/// database): whether the agreement allows it, or the statements ended
/// negative often enough. It is also the approval request's payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceOwnerInput {
    /// Whether the clause or the three negative statements allow the invoice.
    #[serde(default)]
    pub allowed: bool,
}

/// The input of an `invoice_owner` run.
pub fn invoice_input(ctx: &renox::state_machine::TransitionCtx) -> InvoiceOwnerInput {
    ctx.input_as::<InvoiceOwnerInput>().unwrap_or_default()
}

/// Why `statement` cannot be paid out, if it cannot: the balance is above 0
/// and no dispute is open (`PRP.ACL.21`).
pub fn payout_problem(statement: &OwnerStatement) -> Option<String> {
    if statement.balance() <= 0 {
        return Some(english("property.payout.not_positive", &[]));
    }
    if statement.disputed {
        return Some(english("property.payout.disputed", &[]));
    }
    None
}

/// Why `statement` cannot be carried forward, if it cannot: the balance is 0
/// or less.
pub fn carry_problem(statement: &OwnerStatement) -> Option<String> {
    if statement.balance() > 0 {
        return Some(english("property.payout.positive", &[]));
    }
    None
}

/// Why `statement` cannot be invoiced to the owner with `input`, if it
/// cannot: the balance is below 0 and the agreement or the three negative
/// statements allow it.
pub fn invoice_problem(statement: &OwnerStatement, input: &InvoiceOwnerInput) -> Option<String> {
    if statement.balance() >= 0 {
        return Some(english("property.payout.not_negative", &[]));
    }
    if !input.allowed {
        return Some(english("property.payout.not_allowed", &[]));
    }
    None
}

/// Whether the owner may be invoiced for the negative balance of `statement`:
/// the agreement's clause, or the latest [`NEGATIVE_STATEMENTS`] statements of
/// the agreement that count (not superseded, none ending after `statement`),
/// `statement` among them, all ended negative.
pub fn invoice_allowed(
    agreement: &ManagementAgreement,
    statement: &OwnerStatement,
    others: &[OwnerStatement],
) -> bool {
    if agreement.may_invoice_negative_balance {
        return true;
    }
    let mut recent: Vec<&OwnerStatement> = others
        .iter()
        .filter(|s| {
            s.id != statement.id
                && s.status != StatementStatus::Superseded
                && s.period_end <= statement.period_end
        })
        .collect();
    recent.sort_by_key(|s| std::cmp::Reverse((s.period_end, s.revision)));
    let mut latest = vec![statement];
    latest.extend(recent.into_iter().take(NEGATIVE_STATEMENTS - 1));
    latest.len() == NEGATIVE_STATEMENTS && latest.iter().all(|s| s.balance() < 0)
}

/// What asking a payout did.
pub enum PayOutOutcome {
    /// The run: done, or deferred as an approval request.
    Ran(Outcome<OwnerStatement>),
    /// The owner is blocked: an exception request for the approver the block
    /// guard names; no payout is made unless it is approved.
    ExceptionRequested(ApprovalRequest),
}

/// `user` asks to pay out the published statement `id`. The kernel defers the
/// run as an approval request for a property manager other than `user`.
/// With `ask_exception`, a blocked owner becomes an exception request
/// instead of a refusal.
///
/// # Errors
///
/// 403 without `owner_statements.prepare`; 422 unless the statement is
/// published, for a balance that is not above 0, a statement that is disputed,
/// and a blocked owner (the neutral text "on hold") unless `ask_exception`.
pub async fn pay_out(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    ask_exception: bool,
) -> std::result::Result<PayOutOutcome, GuardedError> {
    let statement = OwnerStatement::find(&state.db, id)
        .await?
        .ok_or_else(|| GuardedError::from(Error::NotFound))?;
    match block_guard::run_guarded(state, user, &statement, PAY_OUT, json!({})).await {
        Err(GuardedError::Blocked(_)) if ask_exception => {
            let mut tx = state.db.begin().await?;
            let request = block_guard::request_exception_in(
                state,
                &mut tx,
                user,
                &statement.subject(),
                PAY_OUT,
                json!({}),
            )
            .await?;
            tx.commit().await?;
            approvals::announce_request(state, &request).await?;
            Ok(PayOutOutcome::ExceptionRequested(request))
        }
        other => other.map(PayOutOutcome::Ran),
    }
}

/// `user` carries the balance of the published statement `id` forward to the
/// next statement.
///
/// # Errors
///
/// 403 without `owner_statements.prepare`; 422 unless the statement is
/// published and its balance is 0 or less.
pub async fn carry_forward(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<(), renox::state_machine::TransitionError> {
    let statement = OwnerStatement::find(&state.db, id)
        .await?
        .ok_or(renox::state_machine::TransitionError::Failed(Error::NotFound))?;
    statement
        .transition(crate::statement_machine::CARRY_FORWARD)
        .by(user)
        .run(state)
        .await?;
    Ok(())
}

/// `user` asks to invoice the negative balance of the published statement `id`
/// to the owner. A property manager other than `user` approves.
///
/// # Errors
///
/// 403 without `owner_statements.approve`; 422 unless the statement is
/// published with a balance below 0, when neither the agreement nor three
/// negative statements allow it, and while the charge type *recharged costs*
/// is not configured.
pub async fn invoice_owner(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Outcome<OwnerStatement>, renox::state_machine::TransitionError> {
    use renox::state_machine::TransitionError;
    let statement = OwnerStatement::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let agreement = ManagementAgreement::find(&state.db, statement.agreement_id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let others = OwnerStatement::of_agreement(&state.db, agreement.id).await?;
    let input = InvoiceOwnerInput {
        allowed: invoice_allowed(&agreement, &statement, &others),
    };
    if ChargeType::active_by_code(&state.db, RECHARGED_COSTS)
        .await?
        .is_none()
    {
        return Err(TransitionError::Guard(english(
            "property.payout.no_charge_type",
            &[],
        )));
    }
    let outcome = statement
        .transition(INVOICE_OWNER)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    approvals::announce(state, &outcome)
        .await
        .map_err(TransitionError::Failed)?;
    Ok(outcome)
}

/// The user who asked for the run that `event` is about: the actor of a
/// direct run, else the requester of the approval request that was approved.
async fn requester(state: &AppState, event: &Transitioned<OwnerStatement>) -> Result<Option<i64>> {
    if event.transition == PAY_OUT || event.transition == INVOICE_OWNER {
        return Ok(event.actor_id);
    }
    let Some(request) = event
        .meta
        .get(approvals::submit::REQUEST_KEY)
        .and_then(Value::as_i64)
    else {
        return Ok(None);
    };
    Ok(ApprovalRequest::find(&state.db, request)
        .await?
        .map(|r| r.requester_id))
}

/// The management company's legal entity for `agreement`.
async fn entity_of(state: &AppState, agreement: &ManagementAgreement) -> Result<i64> {
    match agreement.legal_entity_id {
        Some(id) => Ok(id),
        None => Ok(LegalEntity::default_entity(&state.db).await?.id),
    }
}

/// Records the payout of the settled statement `statement`, once.
async fn record_payout(state: &AppState, statement: &OwnerStatement, by: Option<i64>) -> Result {
    let Some(amount) = statement.payout_amount else {
        return Ok(());
    };
    if statement.payout_payment_id.is_some() || amount <= 0 {
        return Ok(());
    }
    let Some(agreement) = ManagementAgreement::find(&state.db, statement.agreement_id).await?
    else {
        return Ok(());
    };
    let entity = entity_of(state, &agreement).await?;
    let request = RecordPayment::new(
        PartyId(statement.owner_party_id),
        entity,
        PaymentMethod::Transfer,
        amount,
    )
    .outgoing()
    .reference(&statement.number());
    let payment = payment_recorder(state).record(state, request, by).await?;
    // Not the status column, so outside the machine's guard.
    OwnerStatement::query()
        .where_(OwnerStatement::ID.eq(statement.id))
        .update(
            &state.db,
            &[
                ("payout_payment_id", &Some(payment.id.0)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// Makes the invoice to the owner for the negative balance of `statement`,
/// once, from the management company's entity.
async fn make_owner_invoice(state: &AppState, statement: &OwnerStatement) -> Result {
    let owed = -statement.balance();
    if statement.owner_invoice_id.is_some() || owed <= 0 {
        return Ok(());
    }
    let Some(agreement) = ManagementAgreement::find(&state.db, statement.agreement_id).await?
    else {
        return Ok(());
    };
    let Some(charge_type) = ChargeType::active_by_code(&state.db, RECHARGED_COSTS).await? else {
        return Err(refused(english("property.payout.no_charge_type", &[])));
    };
    let text = english(
        "property.payout.invoice_line",
        &[("statement", &statement.number())],
    );
    let mut draft = InvoiceDraft::new(PartyId(statement.owner_party_id), statement.subject())
        .line(DraftLine::new(&text, owed, charge_type.vat_category).charge_type(RECHARGED_COSTS));
    if let Some(entity) = agreement.legal_entity_id {
        draft = draft.legal_entity(entity);
    }
    let Some(id) = invoicer(state).draft(&state.db, draft).await? else {
        return Ok(());
    };
    OwnerStatement::query()
        .where_(OwnerStatement::ID.eq(statement.id))
        .update(
            &state.db,
            &[
                ("owner_invoice_id", &Some(id.0)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// What follows the commit of a settling transition: the payout is recorded,
/// or the owner is invoiced.
///
/// # Errors
///
/// The database's, the recorder's and the invoicer's errors.
pub async fn after(state: &AppState, event: &Transitioned<OwnerStatement>) -> Result {
    let name = event.transition.as_str();
    if name != PAY_OUT && name != PAY_OUT_APPROVED && name != INVOICE_OWNER
        && name != INVOICE_OWNER_APPROVED
    {
        return Ok(());
    }
    let Some(statement) = OwnerStatement::find(&state.db, event.key).await? else {
        return Ok(());
    };
    if name == PAY_OUT || name == PAY_OUT_APPROVED {
        let by = requester(state, event).await?;
        record_payout(state, &statement, by).await
    } else {
        make_owner_invoice(state, &statement).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::NaiveDate;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2027, m, d).unwrap()
    }

    fn statement(id: i64, month: u32, collected: i64) -> OwnerStatement {
        OwnerStatement {
            id,
            period_start: day(month, 1),
            period_end: day(month, 28),
            collected,
            status: StatementStatus::Published,
            ..Default::default()
        }
    }

    #[test]
    fn only_a_positive_undisputed_balance_is_paid_out() {
        assert_eq!(payout_problem(&statement(1, 1, 4_200_000)), None);
        assert!(payout_problem(&statement(1, 1, 0)).is_some());
        assert!(payout_problem(&statement(1, 1, -350_000)).is_some());
        let disputed = OwnerStatement {
            disputed: true,
            ..statement(1, 1, 4_200_000)
        };
        assert!(payout_problem(&disputed).unwrap().contains("disputed"));
    }

    #[test]
    fn a_balance_of_zero_or_less_is_carried_forward() {
        assert_eq!(carry_problem(&statement(1, 1, 0)), None);
        assert_eq!(carry_problem(&statement(1, 1, -350_000)), None);
        assert!(carry_problem(&statement(1, 1, 1)).is_some());
    }

    #[test]
    fn only_a_negative_balance_is_invoiced_and_only_when_allowed() {
        let allowed = InvoiceOwnerInput { allowed: true };
        let not = InvoiceOwnerInput::default();
        assert_eq!(invoice_problem(&statement(1, 1, -350_000), &allowed), None);
        assert!(invoice_problem(&statement(1, 1, -350_000), &not).is_some());
        assert!(invoice_problem(&statement(1, 1, 0), &allowed).is_some());
        assert!(invoice_problem(&statement(1, 1, 10), &allowed).is_some());
    }

    #[test]
    fn three_negative_statements_in_a_row_allow_the_invoice() {
        let agreement = ManagementAgreement::default();
        let all = vec![
            statement(1, 1, -100_000),
            statement(2, 2, -200_000),
            statement(3, 3, -350_000),
        ];
        assert!(invoice_allowed(&agreement, &all[2], &all));
        // Only two negative ones.
        let two = vec![
            statement(1, 1, 50_000),
            statement(2, 2, -200_000),
            statement(3, 3, -350_000),
        ];
        assert!(!invoice_allowed(&agreement, &two[2], &two));
        // Fewer than three statements.
        assert!(!invoice_allowed(&agreement, &all[1], &all[..2]));
        // A superseded one does not count.
        let mut superseded = all.clone();
        superseded[1].status = StatementStatus::Superseded;
        assert!(!invoice_allowed(&agreement, &superseded[2], &superseded));
        // The agreement's clause allows it whatever the history.
        let clause = ManagementAgreement {
            may_invoice_negative_balance: true,
            ..Default::default()
        };
        assert!(invoice_allowed(&clause, &two[2], &two));
    }
}
