//! How payroll uses the pay accounts (wl-hr §4 *Pay account*, last paragraph,
//! §5.6, issues #210 and #211).
//!
//! - **At `submit`** a run uses the account that is `active` at that moment
//!   ([`crate::sif::generate_in`] reads it and stores it on the SIF line): a
//!   `cooling` account is never used. When the bank reported the active
//!   account closed ([`mark_closed_in`], from `record_partial`) and the new one
//!   is still cooling, the line is held with [`HELD_ACCOUNT_CLOSED`] before the
//!   file is made ([`hold_closed_in`]) and is paid off-cycle once the new
//!   account is active.
//! - **When an account becomes active** ([`on_activated`], after `activate` or
//!   an MD's waiver) the failed lines and the lines held for a closed account
//!   of the employee are fixed through [`crate::release::line_fixed`]: a line
//!   in a run not yet sent is `included` again, a failed line is `included`
//!   for the next SIF version, and a held line of a sent or paid run is
//!   proposed into an off-cycle run (`HRM.RUL.12`). Decision: every failed
//!   line of the employee is taken as fixed by the new account; the preparer
//!   reviews the lines before `regenerate`.

use renox::db::{Transaction, now};
use renox::prelude::*;
use wl_kernel::documents::today;

use crate::pay_account::{PayAccount, PayAccountStatus};
use crate::payroll::{HELD_ACCOUNT_CLOSED, LineStatus, PayrollLine};

/// Whether a bank's failure reason says the account is closed.
pub fn is_closed_reason(reason: &str) -> bool {
    let reason = reason.to_lowercase();
    reason.contains("account closed") || reason.contains("closed account")
}

/// The bank reported the active account of `employee_id` closed: it is marked,
/// inside `tx`.
pub(crate) async fn mark_closed_in(tx: &mut Transaction, employee_id: i64) -> Result {
    PayAccount::query()
        .where_(PayAccount::EMPLOYEE_ID.eq(employee_id))
        .where_(PayAccount::STATUS.eq(PayAccountStatus::Active))
        .update(
            &mut *tx,
            &[
                ("closed_reported_on", &Some(today())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// Holds the `included` lines of run `run_id` whose employee's active account
/// is closed while a new account is still `cooling`, inside `tx`. Returns the
/// lines held.
pub(crate) async fn hold_closed_in(tx: &mut Transaction, run_id: i64) -> Result<Vec<i64>> {
    let lines = PayrollLine::query()
        .where_(PayrollLine::RUN_ID.eq(run_id))
        .where_(PayrollLine::STATUS.eq(LineStatus::Included))
        .order_by(PayrollLine::ID)
        .get(&mut *tx)
        .await?;
    let mut held = Vec::new();
    for line in lines {
        let closed = PayAccount::active_in(&mut *tx, line.employee_id)
            .await?
            .is_some_and(|account| account.closed_reported_on.is_some());
        if !closed {
            continue;
        }
        let cooling = PayAccount::query()
            .where_(PayAccount::EMPLOYEE_ID.eq(line.employee_id))
            .where_(PayAccount::STATUS.eq(PayAccountStatus::Cooling))
            .exists(&mut *tx)
            .await?;
        if !cooling {
            continue;
        }
        PayrollLine::query()
            .where_(PayrollLine::ID.eq(line.id))
            .update(
                &mut *tx,
                &[
                    ("status", &LineStatus::Held.as_str().to_owned()),
                    ("held_reason", &Some(HELD_ACCOUNT_CLOSED.to_owned())),
                    ("updated_at", &now()),
                ],
            )
            .await?;
        held.push(line.id);
    }
    Ok(held)
}

/// `account` became active: the lines of its employee that the old account
/// failed, or held as *account closed*, are fixed ([`crate::release::line_fixed`]).
pub(crate) async fn on_activated(state: &AppState, account: &PayAccount) -> Result {
    let lines = PayrollLine::query()
        .where_(PayrollLine::EMPLOYEE_ID.eq(account.employee_id))
        .where_(PayrollLine::STATUS.is_in([LineStatus::Failed, LineStatus::Held]))
        .where_(PayrollLine::SOURCE_LINE_ID.is_null())
        .where_(PayrollLine::RELEASED_INTO.is_null())
        .order_by(PayrollLine::ID)
        .get(&state.db)
        .await?;
    for line in lines {
        let fixed = line.status == LineStatus::Failed
            || line.held_reason.as_deref() == Some(HELD_ACCOUNT_CLOSED);
        if fixed {
            crate::release::line_fixed(state, line.id).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_account_is_recognised_in_the_banks_words() {
        assert!(is_closed_reason("Account closed"));
        assert!(is_closed_reason("closed account"));
        assert!(!is_closed_reason("invalid IBAN"));
    }
}
