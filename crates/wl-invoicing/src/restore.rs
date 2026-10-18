//! Giving back what a credit note's use took (wl-invoicing §4 `reopen`, the
//! paragraph after the table, #71 A10 and A11).
//!
//! A failed refund taken from a credit note, or a removed allocation of it,
//! gives the amount back to the remaining balance:
//!
//! - the restore is the lower of the amount asked and what the target
//!   consumed (its taken total minus its restored total), and never more
//!   than the credit note's gross; 0 does nothing;
//! - a movement keyed on (credit note, kind, target, attempt) records it, so
//!   the same failure delivered twice changes nothing;
//! - on an `applied` credit note the system `reopen` runs. Its effect writes
//!   the movement and raises the remaining, and `CreditNoteReopened` is
//!   emitted after the commit ([`crate::events`]);
//! - on an `issued` credit note the same writes happen without a transition
//!   and without the event;
//! - the refund's requester and the holders of the `accountant` role are
//!   told after the commit (`invoicing.credit_note_restored`).
//!
//! Lines, totals, VAT, number and snapshots are never written.

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{StateMachine, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::invoicing::{CREDIT_NOTE_SUBJECT_TYPE, CreditNoteStatus};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::allocate::amount_text;
use crate::allocation::Allocation;
use crate::credit_note::CreditNote;
use crate::credit_note_machine::REOPEN;
use crate::credit_note_movement::{
    CreditNoteMovement, MovementKind, consumed_in, exists_in, requester_in,
};
use crate::texts::english;

/// The notification type of a restore.
pub const RESTORED: &str = "invoicing.credit_note_restored";

/// The subject type of an allocation, the target of its ledger rows.
pub const ALLOCATION_SUBJECT_TYPE: &str = "invoicing.allocation";

/// The module that sends the notice.
const MODULE: &str = "invoicing";

/// The row of the wl-notify §5.4 default table the type belongs to.
const GROUP: &str = "invoices";

/// The role whose holders are told besides the requester.
const ACCOUNTANT: &str = "accountant";

/// What gives the amount back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cause {
    /// The refund that took it failed.
    RefundFailed,
    /// The allocation that used it was removed.
    AllocationRemoved,
}

impl Cause {
    /// The ledger kind of the restore.
    pub fn restored(self) -> MovementKind {
        match self {
            Cause::RefundFailed => MovementKind::RefundRestored,
            Cause::AllocationRemoved => MovementKind::AllocationRestored,
        }
    }

    /// The ledger kind of the take it gives back.
    pub fn taken(self) -> MovementKind {
        match self {
            Cause::RefundFailed => MovementKind::RefundTaken,
            Cause::AllocationRemoved => MovementKind::InvoiceAllocated,
        }
    }

    /// The key of the text that says what happened.
    fn text_key(self) -> &'static str {
        match self {
            Cause::RefundFailed => "wl_invoicing.restored.cause.refund_failed",
            Cause::AllocationRemoved => "wl_invoicing.restored.cause.allocation_removed",
        }
    }
}

/// A restore to run: `amount` fils given back to credit note
/// `credit_note`, because of `cause` on `target` (the refund, or the
/// allocation).
#[derive(Debug, Clone)]
pub struct Restore {
    /// The credit note.
    pub credit_note: i64,
    /// What gives it back.
    pub cause: Cause,
    /// The failed refund, or the removed allocation.
    pub target: Subject,
    /// The refund attempt number; `None` for an allocation.
    pub attempt: Option<i64>,
    /// The amount asked, in fils; the restore is capped at what the target
    /// consumed.
    pub amount: i64,
    /// Why (the failure reason), told in the notices.
    pub reason: Option<String>,
}

/// What a restore did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    /// The amount given back, in fils.
    pub amount: i64,
    /// The credit note's remaining balance after it.
    pub remaining: i64,
    /// Whether the credit note was `applied` and is `issued` again.
    pub reopened: bool,
}

/// What a `reopen` run carries to its guard and effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RestoreInput {
    /// What gives the amount back.
    pub cause: Cause,
    /// The failed refund, or the removed allocation.
    pub target: Subject,
    /// The refund attempt number.
    pub attempt: Option<i64>,
    /// The amount to give back, in fils.
    pub amount: i64,
    /// The user whose action caused it.
    pub ran_by: Option<i64>,
}

/// Writes the restore of `input` on `note` (as loaded in `tx`): the ledger
/// row, and the remaining raised only from the remaining that was loaded.
/// The effect of `reopen`; for an `issued` credit note it runs on its own.
pub(crate) async fn write_in(
    tx: &mut renox::db::Transaction,
    note: &CreditNote,
    input: &RestoreInput,
) -> Result<()> {
    if input.amount <= 0 || note.remaining + input.amount > note.gross_total {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            "The amount is above what the credit note gave out.".to_owned(),
        ));
    }
    CreditNoteMovement::new(
        note.id,
        input.cause.restored(),
        Some(&input.target),
        input.amount,
        input.attempt,
        input.ran_by,
    )
    .insert(&mut *tx)
    .await?;
    let moved = renox::db::sql(format!(
        "UPDATE {} SET remaining = remaining + ?, updated_at = ? WHERE id = ? AND remaining = ?",
        CreditNote::TABLE
    ))
    .bind(input.amount)
    .bind(renox::db::now())
    .bind(note.id)
    .bind(note.remaining)
    .execute(&mut *tx)
    .await?;
    if moved == 0 {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            "The credit note changed; try again.".to_owned(),
        ));
    }
    Ok(())
}

/// Gives back to credit note `request.credit_note` what its `target`
/// consumed, up to `request.amount` (see the module). Returns what was
/// restored, or `None` when nothing was: the credit note is a `draft`, the
/// target consumed nothing (a failure of an attempt the retry never took
/// again, or the same failure delivered twice), or this attempt was already
/// restored.
///
/// # Errors
///
/// A 404 for an unknown credit note; a 422 when the credit note changed
/// under the run. Nothing is written then.
pub async fn restore(
    state: &AppState,
    request: Restore,
) -> std::result::Result<Option<Restored>, TransitionError> {
    let mut tx = state.db.begin().await?;
    let note = CreditNote::find_or_404(&mut tx, request.credit_note).await?;
    if ![CreditNoteStatus::Issued, CreditNoteStatus::Applied].contains(&note.status) {
        return Ok(None);
    }
    let kind = request.cause.restored();
    if exists_in(&mut tx, note.id, kind, &request.target, request.attempt).await? {
        return Ok(None);
    }
    let consumed = consumed_in(&mut tx, note.id, request.cause.taken(), &request.target).await?;
    let amount = request
        .amount
        .min(consumed)
        .min(note.gross_total - note.remaining);
    if amount <= 0 {
        return Ok(None);
    }
    let requester = requester_in(&mut tx, note.id, request.cause.taken(), &request.target).await?;
    let input = RestoreInput {
        cause: request.cause,
        target: request.target.clone(),
        attempt: request.attempt,
        amount,
        ran_by: requester,
    };
    let reopened = note.status == CreditNoteStatus::Applied;
    let outcome = if reopened {
        let meta = json!({ "cause": request.target.to_string(), "amount": amount });
        Some(
            note.transition(REOPEN)
                .by_system()
                .input(&input)
                .meta(meta)
                .run_in(&mut tx)
                .await?,
        )
    } else {
        write_in(&mut tx, &note, &input).await?;
        None
    };
    tx.commit().await?;
    if let Some(outcome) = outcome {
        outcome.emit(state).await.map_err(TransitionError::Failed)?;
    }
    let after = CreditNote::find_or_404(&state.db, note.id).await?;
    notify_restored(state, &after, &request, amount, requester).await?;
    Ok(Some(Restored {
        amount,
        remaining: after.remaining,
        reopened,
    }))
}

/// Gives back to its credit note what allocation `allocation_id` took, up
/// to `amount`: #70's `unallocate` calls it for an allocation whose source
/// is a credit note. `None` for another source, or when nothing is given
/// back (see [`restore`]); a second call for the same allocation changes
/// nothing.
///
/// # Errors
///
/// A 404 for an unknown allocation; the errors of [`restore`].
pub async fn restore_allocation(
    state: &AppState,
    allocation_id: i64,
    amount: i64,
    reason: Option<String>,
) -> std::result::Result<Option<Restored>, TransitionError> {
    let allocation = Allocation::find_or_404(&state.db, allocation_id).await?;
    if allocation.source_type != CREDIT_NOTE_SUBJECT_TYPE {
        return Ok(None);
    }
    restore(
        state,
        Restore {
            credit_note: allocation.source_id,
            cause: Cause::AllocationRemoved,
            target: Subject::new(ALLOCATION_SUBJECT_TYPE, allocation.id),
            attempt: None,
            amount,
            reason,
        },
    )
    .await
}

/// The values the notice texts are filled with.
fn data(note: &CreditNote, cause: Cause, amount: i64, reason: Option<&str>) -> Value {
    let party_name = note
        .party_snapshot
        .as_ref()
        .map(|snapshot| snapshot.0.legal_name_en.clone())
        .unwrap_or_default();
    let reason = reason
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .unwrap_or("no reason given");
    json!({
        "number": note.number.clone().unwrap_or_default(),
        "party_name": party_name,
        "amount": amount_text(&note.currency, amount),
        "remaining": amount_text(&note.currency, note.remaining),
        "cause": english(cause.text_key()),
        "reason": reason,
        "link": format!("/credit-notes/{}", note.id),
    })
}

/// Tells the requester, and the holders of the `accountant` role, that the
/// credit is available again: one notice each.
async fn notify_restored(
    state: &AppState,
    note: &CreditNote,
    request: &Restore,
    amount: i64,
    requester: Option<i64>,
) -> Result {
    let data = data(note, request.cause, amount, request.reason.as_deref());
    let notifier = notify::notifier(state);
    if let Some(id) = requester {
        let notice = NotifyRequest::new(RESTORED, note.subject())
            .to_user(id)
            .data(data.clone());
        notifier.notify(state, notice).await?;
    }
    let mut ids: Vec<i64> = users_with_role_in(&state.db, ACCOUNTANT, &Scope::global())
        .await?
        .iter()
        .map(|user| user.id)
        .filter(|id| Some(*id) != requester)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(());
    }
    let mut notice = NotifyRequest::new(RESTORED, note.subject()).data(data);
    for id in ids {
        notice = notice.to_user(id);
    }
    notifier.notify(state, notice).await?;
    Ok(())
}

/// The notification type [`RESTORED`]: staff, transactional, by e-mail (and
/// the bell).
pub fn notification_type() -> NotificationTypeDef {
    NotificationTypeDef::new(RESTORED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_invoicing.restored.subject")),
            english("wl_invoicing.restored.body"),
        )
}

/// Registers [`RESTORED`] with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(notification_type());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cause_names_its_ledger_kinds() {
        assert_eq!(Cause::RefundFailed.taken(), MovementKind::RefundTaken);
        assert_eq!(Cause::RefundFailed.restored(), MovementKind::RefundRestored);
        assert_eq!(
            Cause::AllocationRemoved.taken(),
            MovementKind::InvoiceAllocated
        );
        assert_eq!(
            Cause::AllocationRemoved.restored(),
            MovementKind::AllocationRestored
        );
        for cause in [Cause::RefundFailed, Cause::AllocationRemoved] {
            assert_eq!(cause.taken().restored_by(), Some(cause.restored()));
        }
    }

    #[test]
    fn the_notice_carries_the_figures_and_the_reason() {
        let note = CreditNote {
            id: 7,
            number: Some("CN-PGP-2026-0001".into()),
            currency: "AED".into(),
            gross_total: 50_000,
            remaining: 50_000,
            ..Default::default()
        };
        let filled = data(&note, Cause::RefundFailed, 50_000, Some("card expired"));
        assert_eq!(filled["number"], "CN-PGP-2026-0001");
        assert_eq!(filled["amount"], "AED 500.00");
        assert_eq!(filled["remaining"], "AED 500.00");
        assert_eq!(filled["reason"], "card expired");
        assert_eq!(filled["link"], "/credit-notes/7");
        let none = data(&note, Cause::AllocationRemoved, 100, Some("  "));
        assert_eq!(none["reason"], "no reason given");
    }

    #[test]
    fn the_type_is_transactional_with_an_english_email() {
        let def = notification_type();
        assert_eq!(def.key, RESTORED);
        assert_eq!(def.category, Category::Transactional);
        assert_eq!(def.channels, [NotifyChannel::Email]);
    }
}
