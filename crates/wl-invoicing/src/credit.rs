//! Crediting an invoice in full (wl-invoicing §4 `credit`).
//!
//! `credit` moves an `issued` or `overdue` invoice to `credited` (terminal)
//! for a user holding `invoices.credit`, with a reason. Its guard is that
//! nothing is allocated (by payments or credit notes), so a credit note
//! never credits more than the gross; a partly paid invoice is corrected
//! with a partial credit note, or by unallocating first. Like `issue` of the
//! credit note, it has a system twin, `credit_system`, for callers with
//! only a user id.
//!
//! [`credit_in_full`] does the whole job in one transaction: it drafts a
//! `full` credit note with every line at its full net, issues it, uses all
//! of its remaining on cancelling the invoice (not an allocation: the
//! invoice's balance goes to 0 and no payment is involved), applies it and
//! runs `credit` on the invoice. After the commit the listeners emit
//! `CreditNoteIssued` and `InvoiceCredited` once each ([`crate::events`]).
//! The credit note is named in the run's `meta`, with whether to release
//! the lines' origins for re-billing.

use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{Transition, TransitionError};
use wl_kernel::LegalEntity;
use wl_kernel::invoicing::{CreditLine, InvoiceStatus};

use crate::credit_note::{CreditNote, CreditNoteKind};
use crate::credit_note_machine::IssueInput;
use crate::credit_note_movement::{CreditNoteMovement, MovementKind};
use crate::credit_notes::{
    By, Outcomes, apply_in, insert_in, issue_in, lower_remaining_in, prepare, run_of,
};
use crate::invoice::Invoice;
use crate::permissions;

/// The user transition.
pub const CREDIT: &str = "credit";
/// The system twin of [`CREDIT`].
pub const CREDIT_SYSTEM: &str = "credit_system";

/// The `credit` and `credit_system` transitions, for the machine.
pub(crate) fn transitions() -> [Transition<Invoice>; 2] {
    [
        rules(
            Transition::new(CREDIT)
                .label("Credit in full")
                .tone("danger")
                .permission(permissions::CREDIT),
        ),
        rules(
            Transition::new(CREDIT_SYSTEM)
                .label("Credit in full (system)")
                .system(),
        ),
    ]
}

/// What both share: the states, the reason, the guard and the effect.
fn rules(transition: Transition<Invoice>) -> Transition<Invoice> {
    transition
        .from([InvoiceStatus::Issued, InvoiceStatus::Overdue])
        .to(InvoiceStatus::Credited)
        .reason_required()
        .guard("nothing_allocated", |invoice: &Invoice, _ctx| {
            let allocated = invoice.allocated_total;
            Box::pin(async move {
                if allocated == 0 {
                    Ok(())
                } else {
                    Err("Money is allocated to the invoice: credit it partly instead.".to_owned())
                }
            })
        })
        .effect(|invoice: &Invoice, _ctx, tx| {
            let (id, balance) = (invoice.id, invoice.balance);
            Box::pin(async move {
                // The credit note cancels the invoice: nothing is left to
                // collect. Only if nothing was allocated meanwhile; not the
                // status column, so outside the machine's guard.
                let moved = renox::db::sql(format!(
                    "UPDATE {} SET balance = 0, updated_at = ? \
                     WHERE id = ? AND allocated_total = 0 AND balance = ?",
                    Invoice::TABLE
                ))
                .bind(renox::db::now())
                .bind(id)
                .bind(balance)
                .execute(&mut *tx)
                .await?;
                if moved == 0 {
                    return Err(Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "The invoice changed; try again.".to_owned(),
                    ));
                }
                Ok(())
            })
        })
}

/// The credit note a `credit` run names in its `meta`.
pub(crate) fn credit_note_of(meta: &Value) -> Option<i64> {
    meta.get("credit_note").and_then(Value::as_i64)
}

/// Whether a `credit` run released the lines' origins.
pub(crate) fn releases_origins(meta: &Value) -> bool {
    meta.get("release_origins")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Credits `invoice` in full as `by`, with `reason`, in one transaction (see
/// the module). When `release_origins` is set, `InvoiceCredited` carries the
/// origins of the invoice's lines so the vertical can bill them again.
/// Returns the applied credit note.
///
/// # Errors
///
/// `Forbidden` without `invoices.credit`; `ReasonRequired` or a 422 for a
/// blank reason; `NotFrom` when the invoice is not `issued` or `overdue`
/// (a `credited` one is not credited twice); `Guard` (422) when anything is
/// allocated to it. Nothing is written then and no number is used.
pub async fn credit_in_full(
    state: &AppState,
    invoice: &Invoice,
    reason: &str,
    release_origins: bool,
    by: By<'_>,
) -> std::result::Result<CreditNote, TransitionError> {
    let mut outcomes = Outcomes::default();
    let mut tx = state.db.begin().await?;
    let applied = credit_in_full_in(
        &mut tx,
        invoice,
        reason,
        release_origins,
        &by,
        &mut outcomes,
    )
    .await?;
    tx.commit().await?;
    outcomes.emit(state).await?;
    Ok(CreditNote::find_or_404(&state.db, applied.id).await?)
}

/// [`credit_in_full`] inside the caller's transaction `tx` (the Invoicer's
/// `credit_in_full_in`, BL-KRN-11): every read and write is in `tx`, the
/// number comes from the series in `tx`, and the outcomes are left in
/// `outcomes` to emit after the caller's commit. Nothing is committed.
/// Returns the applied credit note.
pub(crate) async fn credit_in_full_in(
    tx: &mut renox::db::Transaction,
    invoice: &Invoice,
    reason: &str,
    release_origins: bool,
    by: &By<'_>,
    outcomes: &mut Outcomes,
) -> std::result::Result<CreditNote, TransitionError> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(TransitionError::ReasonRequired);
    }
    let now = Invoice::find_or_404(&mut *tx, invoice.id).await?;
    if ![InvoiceStatus::Issued, InvoiceStatus::Overdue].contains(&now.status) {
        return Err(TransitionError::NotFrom(now.status.to_string()));
    }
    let invoice_lines = crate::invoicer::lines_of(&mut *tx, invoice.id).await?;
    let requested: Vec<CreditLine> = invoice_lines
        .iter()
        .filter(|line| line.net_amount > 0)
        .map(|line| CreditLine::new(line.position, line.net_amount))
        .collect();
    let mut prepared = prepare(
        &mut *tx,
        invoice.id,
        &requested,
        reason,
        CreditNoteKind::Full,
        by.id(),
    )
    .await?;
    prepared.note.release_origins = release_origins;
    let entity = match prepared.invoice.legal_entity_id {
        Some(id) => LegalEntity::find(&mut *tx, id).await?,
        None => None,
    };
    let current = prepared.invoice.clone();
    let input = IssueInput {
        invoice_status: Some(current.status),
        invoice_entity: current.legal_entity_id,
        entity_active: entity.as_ref().is_some_and(LegalEntity::is_active),
        lines: prepared.lines.len(),
        // `prepare` planned every line against the credit already issued.
        line_problem: None,
    };
    let (note, _) = insert_in(tx, prepared).await?;
    let issued = issue_in(tx, &note, &input, by, outcomes).await?;
    cancel_in(tx, &issued, &current, reason, release_origins, by, outcomes).await
}

/// Cancels `current` with issued credit note `issued` in `tx`: all of the
/// note's remaining is used on cancelling the invoice (not an allocation:
/// the balance goes to 0 and no payment is involved), the note is applied
/// and `credit` runs on the invoice with `reason`. Returns the applied note.
pub(crate) async fn cancel_in(
    tx: &mut renox::db::Transaction,
    issued: &CreditNote,
    current: &Invoice,
    reason: &str,
    release_origins: bool,
    by: &By<'_>,
    outcomes: &mut Outcomes,
) -> std::result::Result<CreditNote, TransitionError> {
    // All of it is used on cancelling the invoice, not on an allocation.
    lower_remaining_in(tx, issued, issued.remaining).await?;
    CreditNoteMovement::new(
        issued.id,
        MovementKind::InvoiceCredited,
        Some(&current.subject()),
        issued.remaining,
        None,
        by.id(),
    )
    .insert(&mut *tx)
    .await?;
    let used = CreditNote::find_or_404(&mut *tx, issued.id).await?;
    let applied = apply_in(tx, &used, by, outcomes).await?;

    let meta = json!({ "credit_note": applied.id, "release_origins": release_origins });
    let credited = run_of(current, by, CREDIT, CREDIT_SYSTEM, meta)
        .reason(reason)
        .run_in(&mut *tx)
        .await?;
    outcomes.invoices.push(credited);
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::state_machine::{StateMachine, TransitionCtx};

    fn issued(allocated: i64) -> Invoice {
        Invoice {
            id: 1,
            currency: "AED".into(),
            gross_total: 105_000,
            allocated_total: allocated,
            balance: 105_000 - allocated,
            status: InvoiceStatus::Issued,
            ..Default::default()
        }
    }

    #[test]
    fn credit_leaves_the_open_states_and_has_a_system_twin() {
        let machine = Invoice::machine();
        for name in [CREDIT, CREDIT_SYSTEM] {
            let credit = machine.find(name).expect("credit");
            assert_eq!(credit.from_states(), ["issued", "overdue"]);
            assert_eq!(credit.to_state(), Some("credited"));
            assert!(credit.needs_reason());
            assert_eq!(credit.is_system(), name == CREDIT_SYSTEM);
            assert_eq!(credit.guard_names(), ["nothing_allocated"]);
        }
    }

    #[renox::test]
    async fn only_an_invoice_with_nothing_allocated_is_credited() {
        let machine = Invoice::machine();
        let credit = machine.find(CREDIT).expect("credit");
        let ctx = TransitionCtx::new(None).with_reason("Billed twice".to_owned());
        assert!(credit.check_guards(&issued(0), &ctx).await.is_ok());
        assert!(matches!(
            credit.check_guards(&issued(1), &ctx).await,
            Err(TransitionError::Guard(_))
        ));
    }

    #[test]
    fn the_run_meta_names_the_credit_note() {
        let meta = json!({ "credit_note": 7, "release_origins": true });
        assert_eq!(credit_note_of(&meta), Some(7));
        assert!(releases_origins(&meta));
        assert_eq!(credit_note_of(&Value::Null), None);
        assert!(!releases_origins(&json!({})));
    }
}
