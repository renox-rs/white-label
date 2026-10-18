//! The kernel events the Invoice machine emits (wl-invoicing §6), from a
//! listener on [`Transitioned<Invoice>`], so only after a commit and never
//! on a rollback:
//!
//! - `issue` → [`InvoiceIssued`];
//! - `allocate_in_full` (the balance reached 0) → [`InvoicePaid`];
//! - `recover` / `recover_in_full` → `BadDebtRecovered` and the accountant
//!   notice ([`crate::recover`]); `recover_in_full` also → [`InvoicePaid`];
//! - `mark_overdue` → [`InvoiceOverdue`].
//!
//! `InvoiceDisputed` is not a transition: [`crate::dispute::raise`] emits it
//! itself after its commit.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceIssued, InvoiceOverdue, InvoicePaid};

use crate::allocate::ALLOCATE_IN_FULL;
use crate::invoice::Invoice;
use crate::issue::ISSUE;
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

/// Registers the listener.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Invoice>, state: AppState| async move {
        let known = [
            ISSUE,
            ALLOCATE_IN_FULL,
            MARK_OVERDUE,
            RECOVER,
            RECOVER_IN_FULL,
        ];
        if !known.contains(&event.transition.as_str()) {
            return Ok(());
        }
        // The committed row: the number, dates and balance the run wrote.
        let invoice = Invoice::find_or_404(&state.db, event.key).await?;
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
}
