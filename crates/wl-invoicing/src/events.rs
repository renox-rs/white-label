//! The kernel events the Invoice machine emits (wl-invoicing §6), from a
//! listener on [`Transitioned<Invoice>`], so only after a commit and never
//! on a rollback:
//!
//! - `issue` → [`InvoiceIssued`];
//! - `allocate_in_full` (the balance reached 0) → [`InvoicePaid`];
//! - `recover` / `recover_in_full` → `BadDebtRecovered` and the accountant
//!   notice ([`crate::recover`]); `recover_in_full` also → [`InvoicePaid`];
//! - `mark_overdue` → [`InvoiceOverdue`];
//! - `credit` / `credit_system` → [`InvoiceCredited`] (the credit note and
//!   the release of origins come from the run's `meta`, [`crate::credit`]);
//! - a credit note's `issue` / `issue_system` (a listener on
//!   [`Transitioned<CreditNote>`]) → [`CreditNoteIssued`];
//! - a credit note's `reopen` → [`CreditNoteReopened`] (the amount and the
//!   cause come from the run's `meta`, [`crate::restore`]).
//!
//! `InvoiceDisputed` is not a transition: [`crate::dispute::raise`] emits it
//! itself after its commit.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::Transitioned;
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{
    CreditNoteIssued, CreditNoteReopened, InvoiceCredited, InvoiceIssued, InvoiceOverdue,
    InvoicePaid,
};

use crate::allocate::ALLOCATE_IN_FULL;
use crate::credit::{CREDIT, CREDIT_SYSTEM, credit_note_of, releases_origins};
use crate::credit_note::CreditNote;
use crate::credit_note_machine::{ISSUE as ISSUE_NOTE, ISSUE_SYSTEM as ISSUE_NOTE_SYSTEM, REOPEN};
use crate::invoice::Invoice;
use crate::invoicer::lines_of;
use crate::issue::ISSUE;
use crate::line::InvoiceLine;
use crate::overdue::MARK_OVERDUE;
use crate::recover::{RECOVER, RECOVER_IN_FULL};

/// The event a committed transition of `invoice` (as it is now) gives, if
/// any.
enum InvoiceEvent {
    Issued(InvoiceIssued),
    Paid(InvoicePaid),
    Overdue(InvoiceOverdue),
}

/// Days from `due` to `on`.
fn days_after(due: NaiveDate, on: NaiveDate) -> i64 {
    (on - due).num_days()
}

/// The event `transition` gives for `invoice`, as it is after the commit.
fn event_for(transition: &str, invoice: &Invoice, on: NaiveDate) -> Option<InvoiceEvent> {
    match transition {
        ISSUE => Some(InvoiceEvent::Issued(InvoiceIssued {
            invoice: invoice.invoice_id(),
            party: invoice.party(),
            source: invoice.source(),
            total: invoice.gross_total,
            // `issue` always sets it; the issue date is the fallback.
            due_date: invoice.due_date.or(invoice.issue_date).unwrap_or(on),
        })),
        ALLOCATE_IN_FULL | RECOVER_IN_FULL => Some(InvoiceEvent::Paid(InvoicePaid {
            invoice: invoice.invoice_id(),
            party: invoice.party(),
            source: invoice.source(),
        })),
        MARK_OVERDUE => Some(InvoiceEvent::Overdue(InvoiceOverdue {
            invoice: invoice.invoice_id(),
            party: invoice.party(),
            balance: invoice.balance,
            days_overdue: invoice.due_date.map_or(0, |due| days_after(due, on)),
        })),
        _ => None,
    }
}

/// `CreditNoteIssued` for `note`, as it is after the issue commit.
fn credit_note_issued(note: &CreditNote) -> CreditNoteIssued {
    CreditNoteIssued {
        credit_note: note.credit_note_id(),
        invoice: note.invoice(),
        party: note.party(),
        amount: note.gross_total,
        reason: note.reason.clone(),
    }
}

/// `CreditNoteReopened` for `note` after a `reopen` run with `meta`: the
/// amount given back and the cause (a subject such as `payments.refund:9`).
/// `None` when the run's `meta` does not carry them.
fn credit_note_reopened(note: &CreditNote, meta: &Value) -> Option<CreditNoteReopened> {
    let amount_restored = meta.get("amount").and_then(Value::as_i64)?;
    let cause = meta
        .get("cause")
        .and_then(Value::as_str)
        .and_then(|text| text.parse::<Subject>().ok())?;
    Some(CreditNoteReopened {
        credit_note: note.credit_note_id(),
        party: note.party(),
        amount_restored,
        cause,
    })
}

/// `InvoiceCredited` for `invoice` credited by `note`, with the origins of
/// `lines` when the run released them.
fn invoice_credited(
    invoice: &Invoice,
    note: &CreditNote,
    reason: Option<String>,
    lines: &[InvoiceLine],
    release: bool,
) -> InvoiceCredited {
    InvoiceCredited {
        invoice: invoice.invoice_id(),
        credit_note: note.credit_note_id(),
        amount: note.gross_total,
        reason: reason.unwrap_or_else(|| note.reason.clone()),
        released_origins: if release {
            lines.iter().filter_map(InvoiceLine::origin).collect()
        } else {
            Vec::new()
        },
    }
}

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<CreditNote>, state: AppState| async move {
            let name = event.transition.as_str();
            if ![ISSUE_NOTE, ISSUE_NOTE_SYSTEM, REOPEN].contains(&name) {
                return Ok(());
            }
            let note = CreditNote::find_or_404(&state.db, event.key).await?;
            if name == REOPEN {
                return match credit_note_reopened(&note, &event.meta) {
                    Some(reopened) => state.emit(reopened).await,
                    None => Ok(()),
                };
            }
            state.emit(credit_note_issued(&note)).await
        },
    );
    app.listen(|event: Transitioned<Invoice>, state: AppState| async move {
        let known = [
            ISSUE,
            ALLOCATE_IN_FULL,
            MARK_OVERDUE,
            RECOVER,
            RECOVER_IN_FULL,
            CREDIT,
            CREDIT_SYSTEM,
        ];
        if !known.contains(&event.transition.as_str()) {
            return Ok(());
        }
        // The committed row: the number, dates and balance the run wrote.
        let invoice = Invoice::find_or_404(&state.db, event.key).await?;
        if [CREDIT, CREDIT_SYSTEM].contains(&event.transition.as_str()) {
            let Some(note_id) = credit_note_of(&event.meta) else {
                return Ok(());
            };
            let note = CreditNote::find_or_404(&state.db, note_id).await?;
            let lines = lines_of(&state.db, invoice.id).await?;
            let credited = invoice_credited(
                &invoice,
                &note,
                event.reason.clone(),
                &lines,
                releases_origins(&event.meta),
            );
            return state.emit(credited).await;
        }
        if [RECOVER, RECOVER_IN_FULL].contains(&event.transition.as_str()) {
            crate::recover::after_commit(&state, &invoice, &event.meta).await?;
        }
        match event_for(&event.transition, &invoice, today()) {
            Some(InvoiceEvent::Issued(issued)) => state.emit(issued).await,
            Some(InvoiceEvent::Paid(paid)) => state.emit(paid).await,
            Some(InvoiceEvent::Overdue(overdue)) => state.emit(overdue).await,
            None => Ok(()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::invoicing::InvoiceId;
    use wl_kernel::{PartyId, Subject};

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, day).unwrap()
    }

    fn invoice() -> Invoice {
        Invoice {
            id: 4,
            party_id: 7,
            source_type: "property.lease".into(),
            source_id: 9,
            currency: "AED".into(),
            gross_total: 105_000,
            balance: 63_000,
            issue_date: Some(date(1)),
            due_date: Some(date(3)),
            ..Default::default()
        }
    }

    #[test]
    fn issue_gives_invoice_issued_with_the_total_and_due_date() {
        let Some(InvoiceEvent::Issued(issued)) = event_for(ISSUE, &invoice(), date(1)) else {
            panic!("no InvoiceIssued");
        };
        assert_eq!(issued.invoice, InvoiceId(4));
        assert_eq!(issued.party, PartyId(7));
        assert_eq!(issued.source, Subject::new("property.lease", 9));
        assert_eq!(issued.total, 105_000);
        assert_eq!(issued.due_date, date(3));
    }

    #[test]
    fn allocating_in_full_gives_invoice_paid() {
        let Some(InvoiceEvent::Paid(paid)) = event_for(ALLOCATE_IN_FULL, &invoice(), date(2))
        else {
            panic!("no InvoicePaid");
        };
        assert_eq!((paid.invoice, paid.party), (InvoiceId(4), PartyId(7)));
        assert!(event_for(crate::allocate::ALLOCATE, &invoice(), date(2)).is_none());
    }

    #[test]
    fn recovering_in_full_gives_invoice_paid_and_a_part_does_not() {
        let Some(InvoiceEvent::Paid(paid)) = event_for(RECOVER_IN_FULL, &invoice(), date(2)) else {
            panic!("no InvoicePaid");
        };
        assert_eq!(paid.invoice, InvoiceId(4));
        assert!(event_for(RECOVER, &invoice(), date(2)).is_none());
    }

    #[test]
    fn mark_overdue_gives_the_balance_and_days_overdue() {
        let Some(InvoiceEvent::Overdue(overdue)) = event_for(MARK_OVERDUE, &invoice(), date(10))
        else {
            panic!("no InvoiceOverdue");
        };
        assert_eq!(overdue.balance, 63_000);
        assert_eq!(overdue.days_overdue, 7);
    }

    #[test]
    fn issuing_a_credit_note_gives_credit_note_issued() {
        let note = CreditNote {
            id: 7,
            invoice_id: 4,
            party_id: 7,
            gross_total: 21_000,
            reason: "Damaged".into(),
            ..Default::default()
        };
        let issued = credit_note_issued(&note);
        assert_eq!(issued.credit_note.0, 7);
        assert_eq!((issued.invoice, issued.party), (InvoiceId(4), PartyId(7)));
        assert_eq!((issued.amount, issued.reason.as_str()), (21_000, "Damaged"));
    }

    #[test]
    fn reopening_gives_credit_note_reopened_with_the_cause() {
        let note = CreditNote {
            id: 7,
            party_id: 2,
            ..Default::default()
        };
        let meta = renox::serde_json::json!({ "cause": "payments.refund:9", "amount": 50_000 });
        let reopened = credit_note_reopened(&note, &meta).expect("an event");
        assert_eq!(reopened.credit_note.0, 7);
        assert_eq!(reopened.party, PartyId(2));
        assert_eq!(reopened.amount_restored, 50_000);
        assert_eq!(reopened.cause, Subject::new("payments.refund", 9));
        assert!(credit_note_reopened(&note, &Value::Null).is_none());
    }

    #[test]
    fn crediting_gives_invoice_credited_with_released_origins_on_request() {
        let note = CreditNote {
            id: 7,
            gross_total: 105_000,
            reason: "Billed twice".into(),
            ..Default::default()
        };
        let lines = [
            InvoiceLine {
                origin_type: Some("property.charge".into()),
                origin_id: Some(4),
                ..Default::default()
            },
            InvoiceLine::default(),
        ];
        let kept = invoice_credited(&invoice(), &note, None, &lines, false);
        assert_eq!(kept.invoice, InvoiceId(4));
        assert_eq!((kept.credit_note.0, kept.amount), (7, 105_000));
        assert_eq!(kept.reason, "Billed twice");
        assert!(kept.released_origins.is_empty());
        let released = invoice_credited(&invoice(), &note, Some("Why".into()), &lines, true);
        assert_eq!(released.reason, "Why");
        assert_eq!(
            released.released_origins,
            [Subject::new("property.charge", 4)]
        );
    }
}
