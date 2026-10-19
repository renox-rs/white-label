//! Writing off a bounced cheque through the invoice write-off (wl-payments §4
//! `write_off`, §5.3 step 4, §6 `InvoiceWrittenOff`, issue #106).
//!
//! When the open amount cannot be collected, a user with `cheques.manage`
//! gives a reason and the open balance of every invoice the cheque pays is
//! written off through wl-invoicing (`Invoicer::write_off`, one request per
//! invoice; the invoice's own approval limit, `FIN.ACL.15`, applies to each).
//! The cheque moves to `written_off` only when the invoice write-offs have
//! been made, never before:
//!
//! - **Linked charge not yet invoiced:** invoiced first
//!   ([`ChargeInvoicer`](wl_kernel::cheques::ChargeInvoicer)) and the link of
//!   the cheque points to the invoice. Without anything that can invoice the
//!   charge, the write-off is refused.
//! - **Balances must match:** the open balances of the linked invoices (plus
//!   what this cheque already wrote off on them) must equal the cheque's open
//!   amount; otherwise the message says to write the invoices off directly and
//!   settle the cheque another way (`settle_part`, `replace`).
//! - **A disputed invoice** refuses the write-off (the invoice's own guard).
//! - **Requests:** one [`ChequeWriteOff`] row per invoice. A write-off that
//!   needs no approval is made at once; one above the limit waits. When
//!   `InvoiceWrittenOff` arrives for an invoice with a waiting row
//!   ([`invoice_written_off`]) the row is marked, and when the rows written off
//!   add up to the open amount the system runs `write_off_by_system`
//!   ([`complete`]). A request that is rejected, expires, is withdrawn or fails
//!   (the party paid meanwhile, so the balance changed) marks its row failed
//!   and gives the accountant a task and a notice ([`approval_decided`]); the
//!   cheque stays where it is, and write-offs already made stay. The user may
//!   run `write_off` again for the invoices left.
//!
//! The effect closes the follow-up tasks of the cheque; the cheque, its
//! certificate and the case records are kept as evidence. After the commit
//! `ChequeBounceResolved` (written off) is emitted ([`after_write_off`]).
//! The credit hold is not lifted: wl-sales lifts it for cleared or replaced
//! cheques only, and releases a written-off account by hand.
//!
//! Decisions on what the spec leaves open: the invoice write-off is made with
//! the reason code `other` and the cheque's reason as its text, since the
//! configured codes are the invoicing module's; the user needs
//! `invoices.write_off` as well as `cheques.manage` (the invoice's permission
//! is checked by wl-invoicing). An issued cheque has no invoices to write off
//! and is refused. A second change to the open amount while a request waits
//! (`settle_part`) changes the total the rows must reach.

use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{ApprovalDecided, ApprovalEnd};
use wl_kernel::cheques::{CHARGE_SUBJECT_TYPE, ChequeOutcome, ChequeStatus, charge_invoicer};
use wl_kernel::invoicing::{
    INVOICE_SUBJECT_TYPE, InvoiceId, InvoiceSummary, InvoiceWrittenOff, WriteOffAnswer, invoicer,
};

use crate::cheque::{Cheque, ChequeLink};
use crate::cheque_events;
use crate::followup::{FollowupKind, FollowupStatus, FollowupTask, money_text};
use crate::notices;
use crate::permissions;

/// The `write_off` transition's name (a user).
pub const WRITE_OFF: &str = "write_off";
/// The `write_off_by_system` transition's name: the invoice write-offs the
/// user asked for arrived (an approval can take days).
pub const WRITE_OFF_BY_SYSTEM: &str = "write_off_by_system";

/// The reason code the invoice write-off is made with; the cheque's reason
/// is its text.
pub const INVOICE_REASON_CODE: &str = "other";

/// Where one invoice write-off of a cheque stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WriteOffStatus {
    /// Asked for; waiting for the approval or for `InvoiceWrittenOff`.
    #[default]
    Pending,
    /// The invoice is written off.
    WrittenOff,
    /// Rejected, expired, withdrawn or failed.
    Failed,
}

/// The write-off of one linked invoice of a cheque.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_cheque_write_offs",
    index(cheque_id),
    index(invoice_id)
)]
pub struct ChequeWriteOff {
    /// The primary key.
    pub id: i64,
    /// The cheque.
    pub cheque_id: i64,
    /// The invoice.
    pub invoice_id: i64,
    /// The balance asked to be written off, in fils.
    pub amount: i64,
    /// The reason the user gave.
    pub reason: String,
    /// The user who asked.
    pub requested_by: Option<i64>,
    /// The approval request, when the write-off is above the limit.
    pub approval_request_id: Option<i64>,
    /// Where it stands.
    pub status: WriteOffStatus,
    /// Why it failed.
    pub failure: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ChequeWriteOff {
    /// The write-offs of `cheque_id`, oldest first.
    pub async fn of_cheque(db: &Db, cheque_id: i64) -> Result<Vec<ChequeWriteOff>> {
        ChequeWriteOff::query()
            .where_(ChequeWriteOff::CHEQUE_ID.eq(cheque_id))
            .order_by(ChequeWriteOff::ID)
            .get(db)
            .await
    }

    /// The fils of `cheque_id` already written off.
    pub async fn written_off_total(db: &Db, cheque_id: i64) -> Result<i64> {
        ChequeWriteOff::query()
            .where_(ChequeWriteOff::CHEQUE_ID.eq(cheque_id))
            .where_(ChequeWriteOff::STATUS.eq(WriteOffStatus::WrittenOff))
            .sum::<i64, _>(db, "amount")
            .await
    }

    /// The waiting write-offs of `cheque_id`.
    pub async fn pending_of_cheque(db: &Db, cheque_id: i64) -> Result<Vec<ChequeWriteOff>> {
        ChequeWriteOff::query()
            .where_(ChequeWriteOff::CHEQUE_ID.eq(cheque_id))
            .where_(ChequeWriteOff::STATUS.eq(WriteOffStatus::Pending))
            .order_by(ChequeWriteOff::ID)
            .get(db)
            .await
    }
}

/// What a `write_off` run carries to its guards, gathered by [`write_off`] or
/// [`complete`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct WriteOffInput {
    /// The reason, trimmed.
    pub reason: String,
    /// What the invoice write-offs of the cheque add up to, in fils.
    pub written_off: i64,
}

fn write_off_input(ctx: &TransitionCtx) -> Option<WriteOffInput> {
    ctx.input_as::<WriteOffInput>().ok()
}

/// Why the cheque cannot move to `written_off` with `input`, if it cannot:
/// no reason, or the invoice write-offs made do not add up to the open amount.
pub(crate) fn transition_problem(cheque: &Cheque, input: Option<&WriteOffInput>) -> Option<String> {
    let Some(input) = input else {
        return Some("The action was not prepared by the cheque service.".to_owned());
    };
    if input.reason.trim().is_empty() {
        return Some("A reason is required.".to_owned());
    }
    if cheque.open_amount <= 0 || input.written_off != cheque.open_amount {
        return Some(
            "The invoice write-offs are not all made yet: the cheque stays as it is.".to_owned(),
        );
    }
    None
}

/// The facts [`request_problem`] decides on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Balances {
    /// The open balances of the linked invoices that can be written off, in
    /// fils.
    pub open: i64,
    /// What this cheque already wrote off, in fils.
    pub written_off: i64,
    /// Invoices still waiting for an approval of this cheque's request.
    pub waiting: usize,
    /// Invoices that can be written off now and have no waiting request.
    pub to_request: usize,
}

/// Why the write-off of a cheque with `open_amount` fils open cannot be
/// requested, if it cannot.
pub(crate) fn request_problem(open_amount: i64, balances: &Balances) -> Option<String> {
    if balances.open + balances.written_off != open_amount {
        return Some(format!(
            "The open balances of the linked invoices ({}) differ from the cheque's open amount ({}): write the invoices off directly and settle the cheque another way.",
            money_text("AED", balances.open + balances.written_off),
            money_text("AED", open_amount)
        ));
    }
    if balances.to_request == 0 {
        return Some(if balances.waiting > 0 {
            "The write-off is already waiting for approval.".to_owned()
        } else {
            "There is no open invoice balance to write off.".to_owned()
        });
    }
    None
}

/// `write_off` and `write_off_by_system`, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Cheque>> {
    vec![
        rules(
            Transition::new(WRITE_OFF)
                .label("Write off")
                .tone("danger")
                .permission(permissions::CHEQUES_MANAGE),
        ),
        rules(
            Transition::new(WRITE_OFF_BY_SYSTEM)
                .label("Write off (invoices written off)")
                .system(),
        ),
    ]
}

/// What both transitions share: the states, the guards and the effect.
fn rules(transition: Transition<Cheque>) -> Transition<Cheque> {
    transition
        .from([ChequeStatus::Bounced, ChequeStatus::LegalCase])
        .to(ChequeStatus::WrittenOff)
        .guard("invoices_written_off", |cheque: &Cheque, ctx| {
            let problem = transition_problem(cheque, write_off_input(ctx).as_ref());
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|cheque: &Cheque, _ctx, tx| {
            let id = cheque.id;
            Box::pin(async move {
                // The work on the cheque is over; the records stay.
                for kind in [
                    FollowupKind::FollowUpCheque,
                    FollowupKind::LegalCase,
                    FollowupKind::WriteOffFailed,
                ] {
                    FollowupTask::query()
                        .where_(FollowupTask::CHEQUE_ID.eq(id))
                        .where_(FollowupTask::KIND.eq(kind))
                        .where_(FollowupTask::STATUS.eq(FollowupStatus::Open))
                        .update(
                            &mut *tx,
                            &[
                                ("status", &FollowupStatus::Done.as_str().to_owned()),
                                ("done_at", &Some(renox::db::now())),
                                ("updated_at", &renox::db::now()),
                            ],
                        )
                        .await?;
                }
                Cheque::query()
                    .where_(Cheque::ID.eq(id))
                    .update(&mut *tx, &[("updated_at", &renox::db::now())])
                    .await?;
                Ok(())
            })
        })
}

fn guard(message: impl Into<String>) -> TransitionError {
    TransitionError::Guard(message.into())
}

/// A 422 from another module is a refusal of this action; anything else is a
/// failure.
fn refusal(error: Error) -> TransitionError {
    match error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message) => TransitionError::Guard(message),
        other => TransitionError::Failed(other),
    }
}

async fn reload(state: &AppState, cheque: &Cheque) -> std::result::Result<Cheque, TransitionError> {
    Cheque::find_or_404(&state.db, cheque.id)
        .await
        .map_err(TransitionError::Failed)
}

/// Invoices every linked charge not yet invoiced and points the link of the
/// cheque at the invoice; returns the linked invoices, oldest first.
async fn linked_invoices(
    state: &AppState,
    cheque: &Cheque,
) -> std::result::Result<Vec<InvoiceId>, TransitionError> {
    let mut invoices = Vec::new();
    for link in cheque.links(&state.db).await? {
        if link.subject_type == INVOICE_SUBJECT_TYPE {
            invoices.push(InvoiceId(link.subject_id));
        } else if link.subject_type == CHARGE_SUBJECT_TYPE {
            let charge = link.subject();
            let invoiced = charge_invoicer(state)
                .invoice(state, &charge)
                .await
                .map_err(refusal)?;
            let Some(invoice) = invoiced else {
                return Err(guard(format!(
                    "The charge {charge} is not invoiced yet and cannot be invoiced from here: invoice it first."
                )));
            };
            ChequeLink::query()
                .where_(ChequeLink::ID.eq(link.id))
                .update(
                    &state.db,
                    &[
                        ("subject_type", &INVOICE_SUBJECT_TYPE.to_owned()),
                        ("subject_id", &invoice.0),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await?;
            invoices.push(invoice);
        }
    }
    Ok(invoices)
}

/// `user` writes off the bounced (or `legal_case`) received `cheque` for
/// `reason`: one invoice write-off per linked invoice with an open balance.
/// The cheque becomes `written_off` at once when every write-off was made
/// without an approval; otherwise it stays where it is until
/// `InvoiceWrittenOff` has arrived for all of them ([`invoice_written_off`]).
/// Returns the cheque as it is now.
///
/// # Errors
///
/// `Forbidden` without `cheques.manage`; `NotFrom` unless `bounced` or
/// `legal_case`; `Guard` (422) without a reason, for an issued cheque, a
/// charge that cannot be invoiced, a disputed invoice, an invoice that cannot
/// be written off, a request already waiting, or open balances that differ
/// from the cheque's open amount. A refusal of the second invoice leaves the
/// first request in place.
pub async fn write_off(
    state: &AppState,
    user: &AuthUser,
    cheque: &Cheque,
    reason: &str,
) -> std::result::Result<Cheque, TransitionError> {
    if !user.has_permission(permissions::CHEQUES_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let cheque = reload(state, cheque).await?;
    if !cheque.status.is_owed() {
        return Err(TransitionError::NotFrom(cheque.status.as_str().to_owned()));
    }
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(guard("A reason is required."));
    }
    if !cheque.is_received() {
        return Err(guard(
            "An issued cheque has no invoices to write off: replace it.",
        ));
    }
    let ids = linked_invoices(state, &cheque).await?;
    let waiting = ChequeWriteOff::pending_of_cheque(&state.db, cheque.id).await?;
    let written_off = ChequeWriteOff::written_off_total(&state.db, cheque.id).await?;
    let mut balances = Balances {
        written_off,
        ..Balances::default()
    };
    let mut requests: Vec<InvoiceSummary> = Vec::new();
    for id in ids {
        let Some(invoice) = invoicer(state).find(&state.db, id).await? else {
            return Err(guard(format!("Invoice {id} was not found.")));
        };
        if !invoice.status.is_open() || invoice.balance <= 0 {
            continue;
        }
        if invoice.disputed {
            return Err(guard(format!(
                "Invoice {} is disputed: resolve the dispute before writing it off.",
                invoice.number.clone().unwrap_or_else(|| id.to_string())
            )));
        }
        balances.open += invoice.balance;
        if waiting.iter().any(|w| w.invoice_id == id.0) {
            balances.waiting += 1;
        } else {
            balances.to_request += 1;
            requests.push(invoice);
        }
    }
    if let Some(problem) = request_problem(cheque.open_amount, &balances) {
        return Err(guard(problem));
    }
    for invoice in requests {
        let mut row = ChequeWriteOff {
            cheque_id: cheque.id,
            invoice_id: invoice.id.0,
            amount: invoice.balance,
            reason: reason.to_owned(),
            requested_by: Some(user.id),
            status: WriteOffStatus::Pending,
            ..Default::default()
        };
        row.insert(&state.db).await?;
        let answer = invoicer(state)
            .write_off(state, user, invoice.id, INVOICE_REASON_CODE, Some(reason))
            .await;
        match answer {
            Ok(WriteOffAnswer::WrittenOff) => {
                // The event of the invoice may have marked the row already.
                let mut current = ChequeWriteOff::find_or_404(&state.db, row.id).await?;
                if current.status == WriteOffStatus::Pending {
                    current.status = WriteOffStatus::WrittenOff;
                    current.save(&state.db).await?;
                }
            }
            Ok(WriteOffAnswer::Pending { request }) => {
                let mut current = ChequeWriteOff::find_or_404(&state.db, row.id).await?;
                current.approval_request_id = Some(request);
                current.save(&state.db).await?;
            }
            Err(error) => {
                row.status = WriteOffStatus::Failed;
                row.failure = Some(error.to_string());
                row.save(&state.db).await?;
                return Err(refusal(error));
            }
        }
    }
    complete(state, cheque.id, Some(user)).await?;
    reload(state, &cheque).await
}

/// Runs the cheque's `write_off` once the invoice write-offs made add up to
/// its open amount and none waits: by `actor` when a user is at hand
/// (`write_off`), else by the system (`write_off_by_system`). Returns whether
/// the cheque moved. A cheque that already left `bounced` and `legal_case` is
/// left alone.
///
/// # Errors
///
/// The database's errors, or the refusal of the transition's guards.
pub async fn complete(
    state: &AppState,
    cheque_id: i64,
    actor: Option<&AuthUser>,
) -> std::result::Result<bool, TransitionError> {
    let cheque = Cheque::find_or_404(&state.db, cheque_id)
        .await
        .map_err(TransitionError::Failed)?;
    if !cheque.status.is_owed() {
        return Ok(false);
    }
    let rows = ChequeWriteOff::of_cheque(&state.db, cheque.id).await?;
    let Some(last) = rows.last() else {
        return Ok(false);
    };
    if rows.iter().any(|r| r.status == WriteOffStatus::Pending) {
        return Ok(false);
    }
    let written_off = ChequeWriteOff::written_off_total(&state.db, cheque.id).await?;
    if cheque.open_amount <= 0 || written_off != cheque.open_amount {
        return Ok(false);
    }
    let input = WriteOffInput {
        reason: last.reason.clone(),
        written_off,
    };
    let run = match actor {
        Some(user) => cheque.transition(WRITE_OFF).by(user),
        None => cheque.transition(WRITE_OFF_BY_SYSTEM).by_system(),
    };
    match run
        .reason(input.reason.clone())
        .input(&input)
        .run(state)
        .await
    {
        Ok(_) => Ok(true),
        // Someone moved it first.
        Err(TransitionError::NotFrom(_) | TransitionError::Conflict) => Ok(false),
        Err(other) => Err(other),
    }
}

/// After `InvoiceWrittenOff`: the waiting write-offs of that invoice are
/// marked done, and each cheque they belong to is completed when all its
/// invoices are written off.
///
/// # Errors
///
/// The database's errors.
pub async fn invoice_written_off(state: &AppState, event: &InvoiceWrittenOff) -> Result<()> {
    let waiting = ChequeWriteOff::query()
        .where_(ChequeWriteOff::INVOICE_ID.eq(event.invoice.0))
        .where_(ChequeWriteOff::STATUS.eq(WriteOffStatus::Pending))
        .order_by(ChequeWriteOff::ID)
        .get(&state.db)
        .await?;
    for mut row in waiting {
        row.status = WriteOffStatus::WrittenOff;
        if event.amount > 0 {
            row.amount = event.amount;
        }
        row.save(&state.db).await?;
        complete(state, row.cheque_id, None)
            .await
            .map_err(TransitionError::into_error)?;
    }
    Ok(())
}

/// The text of how an approval request ended.
fn end_text(end: ApprovalEnd) -> &'static str {
    match end {
        ApprovalEnd::Approved => "approved",
        ApprovalEnd::Rejected => "rejected",
        ApprovalEnd::Returned => "returned for changes",
        ApprovalEnd::Withdrawn => "withdrawn",
        ApprovalEnd::Expired => "expired",
        ApprovalEnd::Failed => "failed because the invoice changed",
    }
}

/// After `ApprovalDecided` for an invoice: a write-off request of a cheque
/// that was rejected, expired, was withdrawn or failed (the balance changed:
/// the party paid meanwhile) marks its row failed, and the accountant gets a
/// task and a notice. The cheque stays where it is. An approval needs
/// nothing here (`InvoiceWrittenOff` follows) and a request sent back for
/// changes stays waiting for its resubmission.
///
/// # Errors
///
/// The database's errors, or a notice that failed.
pub async fn approval_decided(state: &AppState, event: &ApprovalDecided) -> Result<()> {
    if event.subject.subject_type != INVOICE_SUBJECT_TYPE
        || matches!(event.decision, ApprovalEnd::Approved | ApprovalEnd::Returned)
    {
        return Ok(());
    }
    let rows = ChequeWriteOff::query()
        .where_(ChequeWriteOff::APPROVAL_REQUEST_ID.eq(event.request))
        .where_(ChequeWriteOff::STATUS.eq(WriteOffStatus::Pending))
        .order_by(ChequeWriteOff::ID)
        .get(&state.db)
        .await?;
    for mut row in rows {
        let why = end_text(event.decision);
        row.status = WriteOffStatus::Failed;
        row.failure = Some(why.to_owned());
        row.save(&state.db).await?;
        let cheque = Cheque::find_or_404(&state.db, row.cheque_id).await?;
        let text = format!(
            "The write-off of {} for cheque {} was {why}: the cheque stays {}.",
            money_text(&cheque.currency, row.amount),
            cheque.number,
            cheque.status.as_str().replace('_', " ")
        );
        let task = FollowupTask::write_off_failed(&cheque, text);
        if let Some(task) = FollowupTask::create_once(&state.db, task).await? {
            notices::cheque_task(state, &cheque, &task).await?;
        }
    }
    Ok(())
}

/// After a committed `write_off` of `cheque_id`: `ChequeBounceResolved`
/// (written off) is emitted. The credit hold stays until wl-sales releases it.
///
/// # Errors
///
/// The database's errors.
pub(crate) async fn after_write_off(state: &AppState, cheque_id: i64) -> Result<()> {
    let cheque = Cheque::find_or_404(&state.db, cheque_id).await?;
    if cheque.status != ChequeStatus::WrittenOff {
        return Ok(());
    }
    cheque_events::resolved(state, &cheque, ChequeOutcome::WrittenOff).await
}

/// Registers the listeners of `InvoiceWrittenOff` and `ApprovalDecided`.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: InvoiceWrittenOff, state: AppState| async move {
        invoice_written_off(&state, &event).await
    });
    app.listen(|event: ApprovalDecided, state: AppState| async move {
        approval_decided(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounced() -> Cheque {
        Cheque {
            id: 1,
            amount: 1_000_000,
            open_amount: 1_000_000,
            presentations: 1,
            status: ChequeStatus::Bounced,
            ..Default::default()
        }
    }

    fn input(reason: &str, written_off: i64) -> WriteOffInput {
        WriteOffInput {
            reason: reason.into(),
            written_off,
        }
    }

    #[test]
    fn write_off_leaves_bounced_and_legal_case_to_written_off() {
        let machine = Cheque::machine();
        for (name, system) in [(WRITE_OFF, false), (WRITE_OFF_BY_SYSTEM, true)] {
            let found = machine.find(name).expect("transition");
            assert_eq!(found.is_system(), system, "{name}");
            assert!(found.check_from(ChequeStatus::Bounced).is_ok(), "{name}");
            assert!(found.check_from(ChequeStatus::LegalCase).is_ok(), "{name}");
            for other in [
                ChequeStatus::Deposited,
                ChequeStatus::Replaced,
                ChequeStatus::Cleared,
                ChequeStatus::WrittenOff,
            ] {
                assert!(found.check_from(other).is_err(), "{name} {other:?}");
            }
            assert_eq!(found.to_state(), Some("written_off"));
            assert_eq!(found.guard_names(), ["invoices_written_off"]);
        }
    }

    #[test]
    fn the_cheque_moves_only_when_the_write_offs_add_up_to_the_open_amount() {
        let cheque = bounced();
        assert_eq!(
            transition_problem(&cheque, Some(&input("untraceable", 1_000_000))),
            None
        );
        assert!(transition_problem(&cheque, Some(&input("untraceable", 600_000))).is_some());
        assert!(transition_problem(&cheque, Some(&input("untraceable", 0))).is_some());
        assert!(transition_problem(&cheque, Some(&input("  ", 1_000_000))).is_some());
        assert!(transition_problem(&cheque, None).is_some());
        let nothing_open = Cheque {
            open_amount: 0,
            ..bounced()
        };
        assert!(transition_problem(&nothing_open, Some(&input("x", 0))).is_some());
    }

    #[test]
    fn the_balances_of_the_invoices_must_equal_the_open_amount() {
        let balances = |open, written_off, waiting, to_request| Balances {
            open,
            written_off,
            waiting,
            to_request,
        };
        assert_eq!(
            request_problem(1_000_000, &balances(1_000_000, 0, 0, 1)),
            None
        );
        // Two invoices, one already written off by this cheque.
        assert_eq!(
            request_problem(1_000_000, &balances(400_000, 600_000, 0, 1)),
            None
        );
        let message = request_problem(1_000_000, &balances(1_250_000, 0, 0, 1)).expect("refused");
        assert!(message.contains("write the invoices off directly"), "{message}");
        assert!(message.contains("AED 12,500.00"), "{message}");
        assert!(request_problem(1_000_000, &balances(400_000, 0, 0, 1)).is_some());
    }

    #[test]
    fn nothing_is_requested_twice() {
        let waiting = Balances {
            open: 1_000_000,
            written_off: 0,
            waiting: 1,
            to_request: 0,
        };
        let message = request_problem(1_000_000, &waiting).expect("refused");
        assert!(message.contains("already waiting"), "{message}");
        let none = Balances {
            open: 0,
            ..Balances::default()
        };
        assert!(request_problem(0, &none).is_some());
    }

    #[renox::test]
    async fn the_guard_reads_the_run_input() {
        let machine = Cheque::machine();
        let found = machine.find(WRITE_OFF).expect("transition");
        let ctx = |input: &WriteOffInput| {
            TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
        };
        assert!(found
            .check_guards(&bounced(), &ctx(&input("untraceable", 1_000_000)))
            .await
            .is_ok());
        assert!(matches!(
            found
                .check_guards(&bounced(), &ctx(&input("untraceable", 400_000)))
                .await,
            Err(TransitionError::Guard(_))
        ));
        assert!(matches!(
            found.check_guards(&bounced(), &TransitionCtx::new(None)).await,
            Err(TransitionError::Guard(_))
        ));
    }

    #[test]
    fn a_failed_request_is_told_in_words() {
        assert_eq!(end_text(ApprovalEnd::Rejected), "rejected");
        assert_eq!(end_text(ApprovalEnd::Expired), "expired");
        assert!(end_text(ApprovalEnd::Failed).contains("invoice changed"));
    }

    #[test]
    fn a_new_write_off_is_pending() {
        assert_eq!(WriteOffStatus::default(), WriteOffStatus::Pending);
        assert_eq!(WriteOffStatus::WrittenOff.as_str(), "written_off");
        assert_eq!(INVOICE_REASON_CODE, "other");
    }
}
