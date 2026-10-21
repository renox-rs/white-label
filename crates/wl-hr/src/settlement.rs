//! The final settlement (wl-hr §3 *Final settlement*, §4, §5.4, §7, issues
//! #197 and #212 to #215).
//!
//! ```text
//! [*] -> draft --approve--> approved --pay--> paid --close--> closed
//!        draft | approved --cancel--> cancelled
//!        approved | paid --dispute--> disputed --resolve--> approved
//!                                               --resolve_closed--> closed
//! ```
//!
//! - The **draft** is made by an exit ([`draft_or_relabel_in`]). The preparer
//!   (`settlements.prepare`) enters the amounts ([`crate::settlement_amounts`]):
//!   the dues (unpaid salary, leave encashment, gratuity, notice pay, other
//!   dues) and the recoveries by type. Computing them from the exit data (the
//!   gratuity, the encashment) is not part of this module's issues: the
//!   preparer types the figures.
//! - **`approve`** and **`pay`** ([`crate::settlement_pay`]): another user
//!   approves (`HRM.ACL.04`), a `settlements.pay` user pays; payments are
//!   recorded against the settlement ([`crate::settlement_payment`]).
//! - **Recoveries** are offset only within the Art. 25 caps; the rest is the
//!   amount due from the employee and becomes a receivable at `pay`
//!   ([`crate::settlement_offset`], BL-HRM-23).
//! - **`dispute`**, **`resolve`**, **`resolve_closed`**
//!   ([`crate::settlement_dispute`], BL-HRM-22): the dispute opens a labour-case
//!   ticket; the undisputed amount can still be paid.
//! - **`close`** and **`cancel`** ([`crate::settlement_close`], BL-HRM-24).
//! - The 14-day deadline alert `HRM.RUL.09` is [`crate::settlement_alerts`]
//!   (BL-HRM-25).
//!
//! The listeners after the commit (ticket, notices, events, tasks) are in
//! [`crate::settlement_events`].
//!
//! [`draft_or_relabel_in`] is what `report_absconding` uses: it drafts the
//! settlement with reason *absconding*, or, when the employee already has one
//! open (`draft` or `approved`, e.g. the one `give_notice` drafted), gives
//! that one the reason and drafts no second.

use renox::chrono::{Duration, NaiveDate};
use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine};
use serde::{Deserialize, Serialize};
use wl_kernel::workforce::SETTLEMENT_SUBJECT_TYPE;

use crate::texts::english;
use crate::{settlement_close, settlement_dispute, settlement_pay};

/// A 422 with the text `key`.
pub(crate) fn refuse(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// The reason of the settlement of an absconded employee.
pub const REASON_ABSCONDING: &str = "absconding";

/// Payee of a settlement: the employee.
pub const PAYEE_EMPLOYEE: &str = "employee";

/// Payee of a settlement: the estate of a deceased employee.
pub const PAYEE_ESTATE: &str = "estate";

/// `approve` (draft -> approved, `settlements.approve`).
pub const APPROVE: &str = "approve";
/// `pay` (approved -> paid, `settlements.pay`).
pub const PAY: &str = "pay";
/// `dispute` (approved, paid -> disputed, `settlements.prepare`).
pub const DISPUTE: &str = "dispute";
/// `resolve` (disputed -> approved, `settlements.approve`).
pub const RESOLVE: &str = "resolve";
/// `resolve` that closes the settlement (disputed -> closed): `upheld` when
/// the net is paid in full or nothing is payable.
pub const RESOLVE_CLOSED: &str = "resolve_closed";
/// `close` (paid -> closed, system).
pub const CLOSE: &str = "close";
/// Not needed any more (notice withdrawn, worker reinstated).
pub const CANCEL: &str = "cancel";
/// `cancel` run by the system for `withdraw_notice` and `reinstate`.
pub const CANCEL_BY_SYSTEM: &str = "cancel_by_system";

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
    /// The last working day: the 14-day deadline counts from it.
    pub last_working_day: Option<NaiveDate>,
    /// Unpaid salary, AED fils.
    pub unpaid_salary: i64,
    /// Leave encashment, AED fils.
    pub leave_encashment: i64,
    /// Gratuity, AED fils.
    pub gratuity: i64,
    /// Notice pay, AED fils.
    pub notice_pay: i64,
    /// Other dues, AED fils.
    pub other_dues: i64,
    /// The overpayment recoveries added while it is not paid, AED fils: a
    /// negative difference of a supplementary run for the employee who left
    /// ([`crate::supplementary`]).
    pub recoveries: i64,
    /// Advances and loans to recover, AED fils (only the 50% total caps them).
    pub advance_recoveries: i64,
    /// Fines to recover, AED fils.
    pub fine_recoveries: i64,
    /// Court debts to recover, AED fils.
    pub court_debt_recoveries: i64,
    /// Damage to recover, AED fils.
    pub damage_recoveries: i64,
    /// The recoveries offset against the dues within the Art. 25 caps.
    pub recoveries_offset: i64,
    /// The dues minus the offset: what the employee is paid, never below 0.
    pub net: i64,
    /// The recoveries above the caps: what the employee owes.
    pub owed_by_employee: i64,
    /// The part of a revised net below what was paid, owed by the employee.
    pub overpaid: i64,
    /// The user who entered the amounts (`HRM.ACL.04`).
    pub prepared_by: Option<i64>,
    /// The user who approved.
    pub approved_by: Option<i64>,
    /// The user who paid.
    pub paid_by: Option<i64>,
    /// The version of the settlement statement (`DOC.GEN.01`); a revision
    /// makes a new one.
    pub statement_version: i64,
    /// The day it was paid.
    pub paid_on: Option<NaiveDate>,
    /// For the payee *estate*: the estate's representative.
    pub representative: Option<String>,
    /// For the payee *estate*: the document giving the right to receive.
    pub right_document: Option<String>,
    /// The day the acknowledgement was requested (not for the estate).
    pub acknowledgement_requested_on: Option<NaiveDate>,
    /// The day the acknowledgement was signed.
    pub acknowledgement_signed_on: Option<NaiveDate>,
    /// From this day a paid settlement closes without a dispute.
    pub closes_on: Option<NaiveDate>,
    /// The day it closed.
    pub closed_on: Option<NaiveDate>,
    /// Who disputes: `worker`, `mohre_complaint` or `court`.
    pub dispute_source: Option<String>,
    /// The complaint or case reference.
    pub dispute_reference: Option<String>,
    /// The part both sides accept, AED fils.
    pub undisputed_amount: i64,
    /// The hearing date.
    pub hearing_on: Option<NaiveDate>,
    /// The date a reply is due.
    pub reply_due_on: Option<NaiveDate>,
    /// The labour-case ticket.
    pub dispute_ticket: Option<i64>,
    /// `approved` or `paid`: where the dispute came from.
    pub disputed_from: Option<String>,
    /// The user who ran `dispute`.
    pub disputed_by: Option<i64>,
    /// The user who entered the revised amounts while `disputed`.
    pub revised_by: Option<i64>,
    /// `upheld` or `revised`.
    pub dispute_outcome: Option<String>,
    /// The decision, agreement or judgment attached to `resolve`.
    pub resolution_document: Option<String>,
    /// The user who resolved.
    pub resolved_by: Option<i64>,
    /// The day `HRM.RUL.09` told HR and the accountant.
    pub alerted_on: Option<NaiveDate>,
    /// The day `HRM.RUL.09` told the MD it is overdue.
    pub overdue_alerted_on: Option<NaiveDate>,
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
            .transition(settlement_pay::approve())
            .transition(settlement_pay::pay())
            .transition(settlement_dispute::dispute())
            .transition(settlement_dispute::resolve(false))
            .transition(settlement_dispute::resolve(true))
            .transition(settlement_close::close())
            .transition(settlement_close::cancel(false))
            .transition(settlement_close::cancel(true))
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

    /// The settlement as a subject (`hr.settlement:7`) for tickets, notices
    /// and the audit log.
    pub fn subject(&self) -> wl_kernel::Subject {
        wl_kernel::Subject::new(SETTLEMENT_SUBJECT_TYPE, self.id)
    }

    /// The dues: unpaid salary, leave encashment, gratuity, notice pay and
    /// other dues.
    pub fn dues(&self) -> i64 {
        self.unpaid_salary
            + self.leave_encashment
            + self.gratuity
            + self.notice_pay
            + self.other_dues
    }

    /// All recoveries, whatever their type.
    pub fn recoveries_total(&self) -> i64 {
        self.recoveries
            + self.advance_recoveries
            + self.fine_recoveries
            + self.court_debt_recoveries
            + self.damage_recoveries
    }

    /// The legal deadline: `days` after the last working day (Art. 53).
    pub fn deadline(&self, days: i64) -> Option<NaiveDate> {
        self.last_working_day.map(|day| day + Duration::days(days))
    }

    /// Whether the settlement is still open for the 14-day rule: `draft`,
    /// `approved` or `disputed`.
    pub fn is_unpaid_state(&self) -> bool {
        matches!(
            self.status,
            SettlementStatus::Draft | SettlementStatus::Approved | SettlementStatus::Disputed
        )
    }

    /// The settlement `id` inside `tx`, or a 404.
    pub async fn load_in(tx: &mut Transaction, id: i64) -> Result<FinalSettlement> {
        FinalSettlement::find(&mut *tx, id)
            .await?
            .ok_or(Error::NotFound)
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
    fn a_settlement_starts_as_a_draft_and_has_the_spec_transitions() {
        let machine = FinalSettlement::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        for (name, from, to) in [
            (APPROVE, "draft", "approved"),
            (PAY, "approved", "paid"),
            (DISPUTE, "approved,paid", "disputed"),
            (RESOLVE, "disputed", "approved"),
            (RESOLVE_CLOSED, "disputed", "closed"),
            (CLOSE, "paid", "closed"),
            (CANCEL, "draft,approved", "cancelled"),
            (CANCEL_BY_SYSTEM, "draft,approved", "cancelled"),
        ] {
            let transition = machine.find(name).unwrap();
            assert_eq!(transition.from_states().join(","), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
        }
    }

    #[test]
    fn the_dues_and_recoveries_are_the_sum_of_their_parts() {
        let settlement = FinalSettlement {
            unpaid_salary: 100,
            leave_encashment: 20,
            gratuity: 3,
            notice_pay: 4,
            other_dues: 5,
            recoveries: 1,
            advance_recoveries: 2,
            fine_recoveries: 3,
            court_debt_recoveries: 4,
            damage_recoveries: 5,
            ..FinalSettlement::default()
        };
        assert_eq!(settlement.dues(), 132);
        assert_eq!(settlement.recoveries_total(), 15);
    }
}
