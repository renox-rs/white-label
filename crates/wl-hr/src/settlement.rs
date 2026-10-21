//! The final settlement as far as an exit needs it (wl-hr §4 *Final
//! settlement*, §5.4, issue #197): a draft that exits create, and `cancel`
//! for a notice withdrawn or a worker reinstated (BL-HRM-06, BL-HRM-08). The
//! amounts, `approve`, `pay`, `dispute`, `resolve` and `close` come with
//! BL-HRM-22 to 24.
//!
//! [`draft_or_relabel_in`] is what `report_absconding` uses: it drafts the
//! settlement with reason *absconding*, or, when the employee already has one
//! open (`draft` or `approved`, e.g. the one `give_notice` drafted), gives
//! that one the reason and drafts no second.

use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition};
use serde::{Deserialize, Serialize};

use crate::permissions;

/// The reason of the settlement of an absconded employee.
pub const REASON_ABSCONDING: &str = "absconding";

/// Payee of a settlement: the employee.
pub const PAYEE_EMPLOYEE: &str = "employee";

/// Not needed any more (notice withdrawn, worker reinstated).
pub const CANCEL: &str = "cancel";

/// Where a final settlement stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SettlementStatus {
    /// Calculated from exit data; editable.
    #[default]
    Draft,
    /// Approved by a second user.
    Approved,
    /// The worker contests the amounts.
    Disputed,
    /// Paid (or nothing payable).
    Paid,
    /// No dispute open; the period to dispute passed or ended.
    Closed,
    /// Not needed.
    Cancelled,
}

/// The money owed at the end of an employment.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_hr_final_settlements",
    state_machine,
    index(employee_id, status)
)]
pub struct FinalSettlement {
    /// The primary key.
    pub id: i64,
    /// The employee.
    #[model(references = "wl_hr_employees")]
    pub employee_id: i64,
    /// Why: `resignation`, `absconding`, `death`, ...
    pub reason: String,
    /// `employee` or `estate`.
    pub payee: String,
    /// The recoveries added while it is not paid, AED fils: a negative
    /// difference of a supplementary run for the employee who left
    /// ([`crate::supplementary`]). The amounts of the settlement itself come
    /// with BL-HRM-22 to 24.
    pub recoveries: i64,
    /// Where it stands; changed only by the machine.
    pub status: SettlementStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl StateMachine for FinalSettlement {
    type State = SettlementStatus;
    const COLUMN: renox::db::Col<FinalSettlement, SettlementStatus> = FinalSettlement::STATUS;

    fn machine() -> Machine<Self> {
        use SettlementStatus::{Approved, Cancelled, Closed, Disputed, Draft, Paid};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Approved)
            .tone("success")
            .state(Disputed)
            .tone("warning")
            .state(Paid)
            .tone("success")
            .state(Closed)
            .tone("success")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(CANCEL)
                    .label("Cancel")
                    .permission(permissions::SETTLEMENTS_PREPARE)
                    .from([Draft, Approved])
                    .to(Cancelled),
            )
    }
}

impl FinalSettlement {
    /// The open settlement of `employee` (`draft` or `approved`), newest
    /// first, inside `tx`.
    pub async fn open_in(tx: &mut Transaction, employee: i64) -> Result<Option<FinalSettlement>> {
        FinalSettlement::query()
            .where_(FinalSettlement::EMPLOYEE_ID.eq(employee))
            .where_(
                FinalSettlement::STATUS
                    .is_in([SettlementStatus::Draft, SettlementStatus::Approved]),
            )
            .order_by_desc(FinalSettlement::ID)
            .first(&mut *tx)
            .await
    }

    /// The settlements of `employee`, oldest first.
    pub async fn of_employee(db: &Db, employee: i64) -> Result<Vec<FinalSettlement>> {
        FinalSettlement::query()
            .where_(FinalSettlement::EMPLOYEE_ID.eq(employee))
            .order_by(FinalSettlement::ID)
            .get(db)
            .await
    }
}

/// Drafts the settlement of `employee_id` with `reason` and `payee`, inside
/// `tx`; when one is already open, it gets `reason` and no second one is
/// drafted. Returns the settlement.
pub async fn draft_or_relabel_in(
    tx: &mut Transaction,
    employee_id: i64,
    reason: &str,
    payee: &str,
) -> Result<FinalSettlement> {
    if let Some(mut open) = FinalSettlement::open_in(&mut *tx, employee_id).await? {
        FinalSettlement::query()
            .where_(FinalSettlement::ID.eq(open.id))
            .update(
                &mut *tx,
                &[("reason", &reason.to_owned()), ("updated_at", &now())],
            )
            .await?;
        open.reason = reason.to_owned();
        return Ok(open);
    }
    let mut settlement = FinalSettlement {
        employee_id,
        reason: reason.to_owned(),
        payee: payee.to_owned(),
        status: SettlementStatus::Draft,
        ..FinalSettlement::default()
    };
    settlement.insert(&mut *tx).await?;
    Ok(settlement)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_settlement_starts_as_a_draft_and_cancel_leaves_draft_and_approved() {
        let machine = FinalSettlement::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        let cancel = machine.find(CANCEL).unwrap();
        assert_eq!(cancel.from_states(), ["draft", "approved"]);
        assert_eq!(cancel.to_state(), Some("cancelled"));
    }
}
