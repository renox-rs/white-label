//! Settling the deposit of a cancelled agreement (wl-payments §4 *Deposit*
//! `start_settlement`, 01 Property `PRP.ACL.18`, issue #251).
//!
//! The vertical cancels a lease that never started after money was received.
//! [`settle`] starts the settlement of the held deposit as the system and
//! publishes the first version of the deductions to the party: the part
//! forfeited, where the signed terms allow it, as one line whose evidence is
//! the clause, else a version with no line. The refund of the rest is the
//! accountant's, after the dispute window, as for any settlement.

use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine, TransitionError};
use wl_kernel::Subject;
use wl_kernel::deposits::{CancelSettlement, SettlementStarted};

use crate::deposit::{Deposit, DepositStatus, start_settlement};
use crate::deposit_machine::START_SETTLEMENT_BY_SYSTEM;
use crate::deposit_deductions::{NewLine, PublishRequest, publish_by};

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The line the forfeited part is published as: its description names the
/// clause, which is also the evidence.
pub(crate) fn forfeit_line(forfeit: i64, clause: &str) -> NewLine {
    NewLine {
        description: format!("Forfeited under {}", clause.trim()),
        amount: forfeit,
        evidence: clause.trim().to_owned(),
        billable: false,
    }
}

/// Why `request` cannot be settled against a deposit holding `held` fils.
pub(crate) fn request_problem(request: &CancelSettlement, held: i64) -> Option<String> {
    if request.forfeit < 0 {
        return Some("The forfeited amount cannot be negative.".to_owned());
    }
    if request.forfeit == 0 {
        return None;
    }
    if request
        .clause
        .as_deref()
        .is_none_or(|c| c.trim().is_empty())
    {
        return Some("A forfeit needs the clause of the signed terms that allows it.".to_owned());
    }
    if request.forfeit > held {
        return Some("The forfeited amount cannot exceed the deposit.".to_owned());
    }
    None
}

/// Starts the settlement of the deposit held for `request.source` and
/// publishes its first version, on behalf of the user `by`. `None` when the
/// source holds no deposit.
///
/// # Errors
///
/// 422 for a forfeit as [`request_problem`] says, or a deposit that is not
/// held; nothing is written then.
pub(crate) async fn settle(
    state: &AppState,
    request: CancelSettlement,
    by: Option<i64>,
) -> Result<Option<SettlementStarted>> {
    let Some(deposit) = Deposit::query()
        .where_(Deposit::SOURCE_TYPE.eq(request.source.subject_type.as_str()))
        .where_(Deposit::SOURCE_ID.eq(request.source.subject_id))
        .where_(Deposit::STATUS.eq(DepositStatus::Held))
        .order_by_desc(Deposit::ID)
        .first(&state.db)
        .await?
    else {
        return Ok(None);
    };
    if let Some(problem) = request_problem(&request, deposit.held_amount) {
        return Err(refused(problem));
    }
    let started = start_settlement(state, None, &deposit)
        .await
        .map_err(TransitionError::into_error)?;
    let lines = match request.clause.as_deref() {
        Some(clause) if request.forfeit > 0 => vec![forfeit_line(request.forfeit, clause)],
        _ => Vec::new(),
    };
    let published = PublishRequest {
        lines,
        forfeiture_reason: None,
    };
    let version = publish_by(state, by, &started, &published)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(Some(SettlementStarted {
        deposit: started.deposit_id(),
        held: started.held_amount,
        forfeit: request.forfeit,
        version: version.version,
    }))
}

/// Starts the settlement of the deposit held for `source` as the system,
/// because its agreement ended, and publishes nothing yet. `None` when the
/// source holds no deposit.
///
/// # Errors
///
/// The database's errors, or the machine's refusal.
pub(crate) async fn begin(state: &AppState, source: &Subject) -> Result<Option<SettlementStarted>> {
    // The variant in a transaction of its own.
    let mut tx = state.db.begin().await?;
    let Some((started, outcome)) = begin_in(&mut tx, source).await? else {
        return Ok(None);
    };
    tx.commit().await?;
    outcome.emit(state).await?;
    Ok(Some(started))
}

/// [`begin`] inside the caller's transaction `tx` (the kernel's
/// `Deposits::start_settlement_in`, BL-KRN-11): the deposit is found and
/// moved to `settlement_pending` in `tx`, so it is only once the caller
/// commits. Nothing is committed; the outcome is returned for the caller to
/// emit after its commit. `None` when the source holds no deposit.
///
/// # Errors
///
/// The database's errors, or the machine's refusal.
pub(crate) async fn begin_in(
    tx: &mut renox::db::Transaction,
    source: &Subject,
) -> Result<Option<(SettlementStarted, Outcome<Deposit>)>> {
    let Some(deposit) = Deposit::query()
        .where_(Deposit::SOURCE_TYPE.eq(source.subject_type.as_str()))
        .where_(Deposit::SOURCE_ID.eq(source.subject_id))
        .where_(Deposit::STATUS.eq(DepositStatus::Held))
        .order_by_desc(Deposit::ID)
        .first(&mut *tx)
        .await?
    else {
        return Ok(None);
    };
    let outcome = deposit
        .transition(START_SETTLEMENT_BY_SYSTEM)
        .by_system()
        .run_in(&mut *tx)
        .await
        .map_err(TransitionError::into_error)?;
    let started = Deposit::find_or_404(&mut *tx, deposit.id).await?;
    Ok(Some((
        SettlementStarted {
            deposit: started.deposit_id(),
            held: started.held_amount,
            forfeit: 0,
            version: 0,
        },
        outcome,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settlement(forfeit: i64, clause: Option<&str>) -> CancelSettlement {
        let mut request = CancelSettlement::new(Subject::new("property.lease", 9));
        request.forfeit = forfeit;
        request.clause = clause.map(str::to_owned);
        request
    }

    #[test]
    fn nothing_forfeited_needs_nothing() {
        assert_eq!(request_problem(&settlement(0, None), 500_000), None);
    }

    #[test]
    fn a_forfeit_needs_its_clause_and_stays_within_the_deposit() {
        assert!(request_problem(&settlement(200_000, None), 500_000).is_some());
        assert!(request_problem(&settlement(200_000, Some("  ")), 500_000).is_some());
        assert!(request_problem(&settlement(600_000, Some("§4")), 500_000).is_some());
        assert!(request_problem(&settlement(-1, Some("§4")), 500_000).is_some());
        assert_eq!(
            request_problem(&settlement(200_000, Some("§4")), 500_000),
            None
        );
    }

    #[test]
    fn the_forfeited_part_is_a_line_with_its_clause_as_evidence() {
        let line = forfeit_line(200_000, " Reservation form §4 ");
        assert_eq!(line.description, "Forfeited under Reservation form §4");
        assert_eq!(line.evidence, "Reservation form §4");
        assert_eq!(line.amount, 200_000);
        assert!(!line.billable);
    }
}
