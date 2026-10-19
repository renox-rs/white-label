//! Handing back cheques that were never deposited (wl-payments §4 `return`,
//! 01 Property `PRP.ACL.18`, issue #251).
//!
//! A `received` or `due_for_deposit` cheque goes back to the party against a
//! handover receipt: `return` for a user with `cheques.manage` (with a
//! reason), `return_by_system` for a vertical that ends the agreement the
//! cheque was received with (a cancelled lease, a handover to a new manager;
//! [`return_undeposited`], the kernel's `Cheques::return_undeposited`). A
//! cheque already deposited, or with a final outcome, is never touched.
//!
//! One call hands back all the cheques it can against **one** receipt, whose
//! reference is stored in the history row of every cheque it returned. The
//! receipt PDF itself comes with document generation (`DOC.GEN.01`).

use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{StateMachine, Transition, TransitionError};
use wl_kernel::cheques::{ChequeId, ChequeStatus, ReturnCheques, ReturnedCheques};
use wl_kernel::documents::today;

use crate::cheque::Cheque;
use crate::permissions;

/// The `return` transition's name (a user).
pub const RETURN: &str = "return";
/// The `return_by_system` transition's name.
pub const RETURN_BY_SYSTEM: &str = "return_by_system";

/// `return` and `return_by_system`, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Cheque>> {
    vec![
        Transition::new(RETURN)
            .label("Return")
            .permission(permissions::CHEQUES_MANAGE)
            .reason_required()
            .from([ChequeStatus::Received, ChequeStatus::DueForDeposit])
            .to(ChequeStatus::Returned),
        Transition::new(RETURN_BY_SYSTEM)
            .label("Return")
            .system()
            .from([ChequeStatus::Received, ChequeStatus::DueForDeposit])
            .to(ChequeStatus::Returned),
    ]
}

/// The reference of a handover receipt made on `day` for a batch that starts
/// with cheque `first`: `HOR-20261104-12`.
pub fn receipt_reference(day: renox::chrono::NaiveDate, first: i64) -> String {
    format!("HOR-{}-{first}", day.format("%Y%m%d"))
}

/// Hands back the cheques of `request` that are not deposited, against one
/// receipt, in one transaction; `None` when none could be returned.
///
/// # Errors
///
/// 422 for a blank reason; nothing is written then.
pub async fn return_undeposited(
    state: &AppState,
    request: &ReturnCheques,
    by: Option<i64>,
) -> Result<Option<ReturnedCheques>> {
    let reason = request.reason.trim();
    if reason.is_empty() {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            "A reason is needed to hand cheques back.".to_owned(),
        ));
    }
    let first = request.cheques.first().map_or(0, |id| id.0);
    let receipt = receipt_reference(today(), first);
    let mut tx = state.db.begin().await?;
    let mut returned: Vec<ChequeId> = Vec::new();
    let mut skipped: Vec<ChequeId> = Vec::new();
    let mut outcomes = Vec::new();
    for id in &request.cheques {
        let found = Cheque::find(&mut tx, id.0).await?;
        let Some(cheque) = found.filter(|c| {
            matches!(c.status, ChequeStatus::Received | ChequeStatus::DueForDeposit)
        }) else {
            skipped.push(*id);
            continue;
        };
        let outcome = cheque
            .transition(RETURN_BY_SYSTEM)
            .by_system()
            .reason(reason)
            .meta(json!({ "receipt": receipt, "requested_by": by }))
            .run_in(&mut tx)
            .await
            .map_err(TransitionError::into_error)?;
        outcomes.push(outcome);
        returned.push(*id);
    }
    if returned.is_empty() {
        return Ok(None);
    }
    tx.commit().await?;
    for outcome in outcomes {
        outcome.emit(state).await?;
    }
    Ok(Some(ReturnedCheques {
        receipt,
        returned,
        skipped,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::NaiveDate;

    #[test]
    fn both_transitions_leave_the_undeposited_states_for_returned() {
        let machine = Cheque::machine();
        for (name, system) in [(RETURN, false), (RETURN_BY_SYSTEM, true)] {
            let found = machine.find(name).expect("transition");
            assert_eq!(found.from_states(), ["received", "due_for_deposit"]);
            assert_eq!(found.to_state(), Some("returned"));
            assert_eq!(found.is_system(), system);
        }
        assert!(machine.find(RETURN).unwrap().needs_reason());
    }

    #[test]
    fn a_receipt_reference_names_the_day_and_the_first_cheque() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 4).unwrap();
        assert_eq!(receipt_reference(day, 12), "HOR-20261104-12");
    }
}
