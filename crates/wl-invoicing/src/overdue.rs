//! Marking invoices overdue (wl-invoicing §4 `mark_overdue`, `FIN.RUL.02`).
//!
//! `mark_overdue` is a system transition from `issued` and
//! `partially_paid` to `overdue`, guarded by "the due date has passed"
//! (today in Asia/Dubai is after it) and "a balance is left". [`run`] picks
//! the invoices it applies to and runs it on each; the daily task
//! [`TASK`] calls it. `InvoiceOverdue` is emitted after each commit
//! ([`crate::events`]).

use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionError};
use wl_kernel::documents::today;
use wl_kernel::invoicing::InvoiceStatus;

use crate::invoice::Invoice;

/// The transition's name.
pub const MARK_OVERDUE: &str = "mark_overdue";

/// The daily scheduled task that runs [`run`].
pub const TASK: &str = "invoicing:mark-overdue";

/// When [`TASK`] runs, in Asia/Dubai: just after midnight, so an invoice
/// due yesterday is overdue from the start of the day.
pub const TASK_AT: &str = "00:30";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The states `mark_overdue` leaves.
const FROM: [InvoiceStatus; 2] = [InvoiceStatus::Issued, InvoiceStatus::PartiallyPaid];

/// The `mark_overdue` transition, for the machine.
pub(crate) fn transition() -> Transition<Invoice> {
    Transition::new(MARK_OVERDUE)
        .label("Mark overdue")
        .tone("danger")
        .system()
        .from(FROM)
        .to(InvoiceStatus::Overdue)
        .guard("due_date_passed", |invoice: &Invoice, _ctx| {
            Box::pin(async move {
                match invoice.due_date {
                    Some(due) if due < today() => Ok(()),
                    _ => Err("The due date has not passed.".to_owned()),
                }
            })
        })
        .guard("has_balance", |invoice: &Invoice, _ctx| {
            Box::pin(async move {
                if invoice.balance > 0 {
                    Ok(())
                } else {
                    Err("Nothing is left to pay.".to_owned())
                }
            })
        })
}

/// Moves every `issued` or `partially_paid` invoice whose due date has
/// passed (today in Asia/Dubai) and that has a balance to `overdue`, as the
/// system, one transaction each. Returns how many moved.
///
/// An invoice that changed between the query and its run (paid or moved in
/// the meantime: `NotFrom`, `Guard`, `Conflict`) is left as it is; any
/// other failure stops the run with its error.
pub async fn run(state: &AppState) -> Result<usize> {
    let candidates = Invoice::query()
        .where_(Invoice::STATUS.is_in(FROM))
        .where_(Invoice::DUE_DATE.lt(today()))
        .where_(Invoice::BALANCE.gt(0))
        .order_by(Invoice::ID)
        .get(&state.db)
        .await?;
    let mut moved = 0;
    for invoice in candidates {
        match invoice
            .transition(MARK_OVERDUE)
            .by_system()
            .run(state)
            .await
        {
            Ok(outcome) => moved += usize::from(!outcome.is_pending()),
            Err(
                TransitionError::NotFrom(_) | TransitionError::Guard(_) | TransitionError::Conflict,
            ) => {}
            Err(error) => return Err(error.into_error()),
        }
    }
    Ok(moved)
}

/// Registers the daily [`TASK`].
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::Days;

    fn issued(due_in_days: i64, balance: i64) -> Invoice {
        let today = today();
        let due = if due_in_days >= 0 {
            today.checked_add_days(Days::new(due_in_days.unsigned_abs()))
        } else {
            today.checked_sub_days(Days::new(due_in_days.unsigned_abs()))
        };
        Invoice {
            currency: "AED".into(),
            due_date: due,
            balance,
            status: InvoiceStatus::Issued,
            ..Default::default()
        }
    }

    #[test]
    fn mark_overdue_goes_from_issued_or_partly_paid_to_overdue() {
        let machine = Invoice::machine();
        let transition = machine.find(MARK_OVERDUE).expect("transition");
        assert_eq!(transition.from_states(), ["issued", "partially_paid"]);
        assert_eq!(transition.to_state(), Some("overdue"));
        assert!(transition.is_system());
        assert_eq!(transition.guard_names(), ["due_date_passed", "has_balance"]);
    }

    #[renox::test]
    async fn only_a_passed_due_date_with_a_balance_is_overdue() {
        let machine = Invoice::machine();
        let transition = machine.find(MARK_OVERDUE).expect("transition");
        let ctx = renox::state_machine::TransitionCtx::new(None);
        assert!(
            transition
                .check_guards(&issued(-1, 100), &ctx)
                .await
                .is_ok()
        );
        for invoice in [issued(0, 100), issued(5, 100), issued(-1, 0)] {
            assert!(matches!(
                transition.check_guards(&invoice, &ctx).await,
                Err(TransitionError::Guard(_))
            ));
        }
        let no_due_date = Invoice {
            due_date: None,
            ..issued(-1, 100)
        };
        assert!(transition.check_guards(&no_due_date, &ctx).await.is_err());
    }
}
