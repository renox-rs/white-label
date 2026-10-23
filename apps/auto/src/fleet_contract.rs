//! The fleet service contract: an agreement with a fleet for a term (08 §4.5,
//! §6 *Fleet contract*, §7.1 *Fleet contract*, `AUT.FLT.01`).
//!
//! A contract names the fleet (a party) and its contact, the labour rate and
//! the part price tier, what is included, the notice period, the early
//! termination fee and the signed contract. The vehicles under contract are
//! the vehicles whose `fleet_contract_id` it is ([`crate::vehicle::Vehicle`]);
//! their service schedule is [`crate::fleet_service::FleetService`]; each visit
//! is a job card under the contract ([`crate::fleet_job::FleetJob`]) billed in
//! the consolidated monthly invoice ([`crate::fleet_billing`]). The status
//! moves only through the machine in [`crate::fleet_contract_machine`].

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// A fleet contract was terminated early (08 §7.1 *Fleet contract*
/// `terminate`): its last day of service passed, the future bookings were
/// cancelled, the final invoice drafted and the fee invoiced. It is emitted
/// once, after all that was done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetContractTerminated {
    /// The contract.
    pub contract: Subject,
    /// Its number, `FC-0001`.
    pub number: String,
    /// The fleet (a party).
    pub fleet_party_id: i64,
    /// Why: fleet sold, non-payment, breach or agreement.
    pub reason: Option<String>,
    /// The last day of service.
    pub effective_date: Option<NaiveDate>,
    /// The final consolidated invoice, when one was drafted.
    pub final_invoice: Option<i64>,
    /// The invoice of the early termination fee, when there is one.
    pub fee_invoice: Option<i64>,
}

impl Event for FleetContractTerminated {}

pub use crate::seed::FLEET_CONTRACT_SUBJECT_TYPE;

/// Where a fleet contract stands (08 §7.1 *Fleet contract*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FleetStatus {
    /// Being negotiated.
    #[default]
    Draft,
    /// In force; services scheduled.
    Active,
    /// Paused (unpaid invoices, fleet put on hold).
    Suspended,
    /// Inside the renewal window.
    RenewalDue,
    /// Expired or terminated (terminal).
    Ended,
}

impl FleetStatus {
    /// Whether the contract is in force in some way: its vehicles are served
    /// and its job cards billed under it.
    pub fn is_in_force(self) -> bool {
        matches!(
            self,
            FleetStatus::Active | FleetStatus::Suspended | FleetStatus::RenewalDue
        )
    }
}

/// A fleet service contract.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "auto_fleet_contracts",
    state_machine,
    index(fleet_party_id),
    index(status)
)]
pub struct FleetContract {
    /// The primary key.
    pub id: i64,
    /// The fleet (a party): billed and the owner of the vehicles.
    pub fleet_party_id: i64,
    /// The fleet contact (a party), told of services and invoices.
    pub contact_party_id: Option<i64>,
    /// What the contract is called.
    pub title: String,
    /// The first day of the term.
    pub start_date: NaiveDate,
    /// The last day of service: the contract is valid through it.
    pub end_date: NaiveDate,
    /// The hourly labour rate agreed, in fils.
    pub labour_rate: i64,
    /// The part price tier (a wl-sales tier name).
    pub price_tier: Option<String>,
    /// What the contract includes.
    pub inclusions: Option<String>,
    /// The notice period in days.
    pub notice_period_days: i64,
    /// The early termination fee in fils; 0 for none.
    pub early_termination_fee: i64,
    /// Days before the end date at which the renewal is due.
    pub renewal_window_days: i64,
    /// The signed contract (a document reference).
    pub signed_document: Option<String>,
    /// When it was activated.
    pub activated_at: Option<DateTime>,
    /// Why it is suspended.
    pub suspended_reason: Option<String>,
    /// When it was suspended.
    pub suspended_at: Option<DateTime>,
    /// Why it was terminated (fleet sold, non-payment, breach, agreement).
    pub termination_reason: Option<String>,
    /// When notice of the termination was given.
    pub notice_date: Option<NaiveDate>,
    /// The last day of service of a terminated contract.
    pub effective_date: Option<NaiveDate>,
    /// A termination was approved and its effective date has not passed: the
    /// contract keeps its state, marked *termination scheduled*.
    pub termination_scheduled: bool,
    /// The user who asked for the termination.
    pub termination_requested_by: Option<i64>,
    /// The fleet asked for the early termination fee to be waived; the owner's
    /// approval of the termination waives it.
    pub termination_fee_waived: bool,
    /// The owner who approved the termination.
    pub termination_approved_by: Option<i64>,
    /// When the termination was approved.
    pub termination_approved_at: Option<DateTime>,
    /// The invoice of the early termination fee (wl-invoicing).
    pub termination_fee_invoice_id: Option<i64>,
    /// When the fleet contact was told that the contract ended.
    pub contact_notified_at: Option<DateTime>,
    /// When everything that follows the end was done (bookings cancelled,
    /// the last invoice, the fee, the notice).
    pub wound_down_at: Option<DateTime>,
    /// When the contract ended.
    pub ended_at: Option<DateTime>,
    /// The last day of the periods invoiced so far (`FIN.USG.01`).
    pub invoiced_through_date: Option<NaiveDate>,
    /// The lifecycle state; changed only by a transition.
    pub status: FleetStatus,
    /// The user who made it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl FleetContract {
    /// The number people read: `FC-0007`.
    pub fn number(&self) -> String {
        format!("FC-{:04}", self.id)
    }

    /// The contract as a [`Subject`]: `auto.fleet_contract:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(FLEET_CONTRACT_SUBJECT_TYPE, self.id)
    }

    /// Whether the contract is valid on `day`: from its start through its end
    /// date (and through the effective date of a termination).
    pub fn covers(&self, day: NaiveDate) -> bool {
        let last = match (self.termination_scheduled, self.effective_date) {
            (true, Some(effective)) => effective.min(self.end_date),
            _ => self.end_date,
        };
        self.start_date <= day && day <= last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(month: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, d).unwrap()
    }

    #[test]
    fn a_contract_is_numbered_and_is_a_subject() {
        let contract = FleetContract {
            id: 7,
            ..FleetContract::default()
        };
        assert_eq!(contract.number(), "FC-0007");
        assert_eq!(contract.subject().to_string(), "auto.fleet_contract:7");
    }

    #[test]
    fn a_contract_is_valid_through_its_end_date() {
        let contract = FleetContract {
            start_date: day(1, 1),
            end_date: day(12, 31),
            ..FleetContract::default()
        };
        assert!(!contract.covers(day(1, 1).pred_opt().unwrap()));
        assert!(contract.covers(day(1, 1)));
        assert!(contract.covers(day(12, 31)));
        assert!(!contract.covers(day(12, 31).succ_opt().unwrap()));
    }

    #[test]
    fn a_scheduled_termination_ends_the_term_at_its_effective_date() {
        let contract = FleetContract {
            start_date: day(1, 1),
            end_date: day(12, 31),
            termination_scheduled: true,
            effective_date: Some(day(11, 20)),
            ..FleetContract::default()
        };
        assert!(contract.covers(day(11, 20)), "the last day is served");
        assert!(!contract.covers(day(11, 21)));
    }

    #[test]
    fn only_a_contract_in_force_serves_vehicles() {
        assert!(FleetStatus::Active.is_in_force());
        assert!(FleetStatus::Suspended.is_in_force());
        assert!(FleetStatus::RenewalDue.is_in_force());
        assert!(!FleetStatus::Draft.is_in_force());
        assert!(!FleetStatus::Ended.is_in_force());
    }
}
