//! The job card: the workshop work order for one visit of a vehicle (08 §6
//! *Job card*, §7.1 *Job card*, `AUT.JOB.01`).
//!
//! A job card is opened at check-in, from the booking that was checked in or
//! at the desk for a walk-in. It records the complaint, the odometer, the fuel
//! level and the vehicle's condition at intake (consumer protection, 08 §2),
//! the technician, the estimate rounds the customer answered
//! ([`crate::approval`]), the labour and part lines ([`crate::labour_line`],
//! [`crate::part_line`]), the QC checklist and the delivery. The status moves
//! only through the machine in [`crate::job_card_machine`]; [`crate::job_cards`]
//! holds the services, [`crate::parts`] the parts issued from stock and
//! [`crate::billing`] the invoice on delivery.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Json};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

pub use crate::seed::JOB_CARD_SUBJECT_TYPE;

/// Where a job card stands (08 §7.1 *Job card*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum JobCardStatus {
    /// Vehicle received; check-in data recorded.
    #[default]
    CheckedIn,
    /// Technician diagnosing.
    Inspecting,
    /// Estimate or added work sent to the customer.
    AwaitingApproval,
    /// Work approved and under way.
    InProgress,
    /// Waiting for parts from stock, another branch or a supplier.
    AwaitingParts,
    /// Work done; QC checklist in progress.
    QualityCheck,
    /// Ready for collection; customer notified.
    Ready,
    /// Collected and invoiced (terminal).
    Delivered,
    /// The customer declined the estimate or left, or it expired (terminal).
    Declined,
    /// Stopped after work started (terminal).
    Cancelled,
    /// Opened in error (terminal).
    Voided,
}

impl JobCardStatus {
    /// Whether the job card is over: delivered, declined, cancelled or voided.
    pub fn is_closed(self) -> bool {
        matches!(
            self,
            JobCardStatus::Delivered
                | JobCardStatus::Declined
                | JobCardStatus::Cancelled
                | JobCardStatus::Voided
        )
    }

    /// Whether the vehicle is in the workshop's care: the job card is open or
    /// ready, not closed.
    pub fn is_open(self) -> bool {
        !self.is_closed()
    }

    /// Whether labour and parts may still be added to the estimate.
    pub fn takes_lines(self) -> bool {
        matches!(
            self,
            JobCardStatus::CheckedIn | JobCardStatus::Inspecting | JobCardStatus::InProgress
        )
    }
}

/// The fuel levels the check-in records, from empty to full.
pub const FUEL_LEVELS: [&str; 5] = ["empty", "quarter", "half", "three_quarters", "full"];

/// The checks of the QC checklist (`WRK.CHK.01`, `AUT.ACL.09`), in order. The
/// workshop's own checklist is a later setting; these are the standard ones.
pub const QC_KEYS: [&str; 5] = ["fluids", "wheels", "lights", "road_test", "clean"];

/// One check of the QC checklist.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct QcCheck {
    /// The check, one of [`QC_KEYS`].
    pub key: String,
    /// Whether it was done.
    pub done: bool,
}

impl QcCheck {
    /// The standard checklist, nothing done yet.
    pub fn standard() -> Vec<QcCheck> {
        QC_KEYS
            .iter()
            .map(|key| QcCheck {
                key: (*key).to_owned(),
                done: false,
            })
            .collect()
    }
}

/// A job card.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "auto_job_cards",
    state_machine,
    unique(booking_id),
    index(vehicle_id),
    index(customer_party_id),
    index(technician_user_id),
    index(status)
)]
pub struct JobCard {
    /// The primary key.
    pub id: i64,
    /// The vehicle.
    #[model(references = "auto_vehicles")]
    pub vehicle_id: i64,
    /// The customer billed: the vehicle's owner, or the fleet.
    pub customer_party_id: i64,
    /// The booking that was checked in, when there is one.
    pub booking_id: Option<i64>,
    /// The type of the workshop (a wl-scheduling location), when it has one.
    pub workshop_type: Option<String>,
    /// The id of the workshop.
    pub workshop_id: Option<i64>,
    /// The service advisor who checked the vehicle in.
    pub advisor_user_id: Option<i64>,
    /// The technician assigned.
    pub technician_user_id: Option<i64>,
    /// The customer's complaint.
    pub complaint: String,
    /// The odometer reading at check-in, in kilometres.
    pub odometer_in: i64,
    /// The fuel level at check-in, one of [`FUEL_LEVELS`].
    pub fuel_level: Option<String>,
    /// The condition of the vehicle at intake: body, interior, valuables.
    pub intake_condition: Option<String>,
    /// The user who recorded the intake.
    pub intake_by: Option<i64>,
    /// When the intake was recorded; the estimate waits for it.
    pub intake_recorded_at: Option<DateTime>,
    /// The customer who acknowledged the recorded condition.
    pub intake_acknowledged_by: Option<String>,
    /// The customer's signature on the condition, as the form sent it.
    pub intake_signature: Option<String>,
    /// What the technician found.
    pub findings: Option<String>,
    /// Whether the damage is from an accident: a police report is needed
    /// (`AUT.ACL.20`).
    pub accident_damage: bool,
    /// The number of the police accident report.
    pub police_report_no: Option<String>,
    /// Why the advisor asks to clear the accident flag (`AUT.ACL.20`); set
    /// while the request waits for the workshop manager.
    pub accident_clear_reason: Option<String>,
    /// The user who asked to clear the accident flag.
    pub accident_clear_by: Option<i64>,
    /// When the request was made.
    pub accident_clear_at: Option<DateTime>,
    /// An internal job card: the repair of damage the workshop caused, paid by
    /// the workshop; nothing on it is invoiced (`AUT.ACL.22`).
    pub internal: bool,
    /// The vehicle incident an internal job card repairs.
    pub incident_id: Option<i64>,
    /// The inspection fee agreed at check-in, in fils; 0 for none.
    pub inspection_fee: i64,
    /// The daily storage fee agreed at check-in, in fils; 0 for none
    /// (`AUT.RUL.18`).
    pub storage_fee_daily: i64,
    /// How many estimates were sent.
    pub estimate_revision: i64,
    /// When the job card entered `awaiting_approval` (`AUT.RUL.03`,
    /// `AUT.RUL.17`).
    pub awaiting_since: Option<DateTime>,
    /// When the customer first approved something.
    pub first_approved_at: Option<DateTime>,
    /// When the wait for parts began (`AUT.RUL.16`).
    pub parts_wait_started_at: Option<DateTime>,
    /// The legal supply date of the parts (`AUT.RUL.16`).
    pub legal_supply_date: Option<NaiveDate>,
    /// The QC checklist.
    pub qc_checks: Json<Vec<QcCheck>>,
    /// What the last failed QC found.
    pub qc_findings: Option<String>,
    /// The user who passed the QC.
    pub qc_by: Option<i64>,
    /// When the vehicle became ready.
    pub ready_at: Option<DateTime>,
    /// When it was collected.
    pub delivered_at: Option<DateTime>,
    /// Who collected it.
    pub delivered_to: Option<String>,
    /// The odometer reading at delivery.
    pub odometer_out: Option<i64>,
    /// The date the next service falls due.
    pub next_service_date: Option<NaiveDate>,
    /// The odometer reading the next service falls due at.
    pub next_service_odometer: Option<i64>,
    /// Why the job card was declined, cancelled or voided.
    pub closed_reason: Option<String>,
    /// When it was declined, cancelled or voided.
    pub closed_at: Option<DateTime>,
    /// The invoice raised on delivery (wl-invoicing).
    pub invoice_id: Option<i64>,
    /// When the invoice was raised.
    pub invoiced_at: Option<DateTime>,
    /// Why the job card ended, as a code: `customer_stopped`,
    /// `claim_withdrawn`, `total_loss`, `no_response`, `customer_left`.
    pub closed_code: Option<String>,
    /// The user who asked for the cancel (the requester of its approval).
    pub stopped_by: Option<i64>,
    /// How the customer acknowledged the work done: `signed`,
    /// `refused_to_sign`, `insurer_notice` or `written_withdrawal`.
    pub ack_method: Option<String>,
    /// The customer who acknowledged the summary of the work done.
    pub ack_name: Option<String>,
    /// The customer's signature, as the form sent it.
    pub ack_signature: Option<String>,
    /// The member of staff who witnessed a refusal to sign.
    pub ack_witness: Option<String>,
    /// The insurer's notice or the customer's written withdrawal (a document
    /// reference).
    pub ack_document: Option<String>,
    /// When the acknowledgement was recorded.
    pub ack_at: Option<DateTime>,
    /// The value of the work done on a stopped job before VAT, in fils.
    pub work_done_value: i64,
    /// What the stopped or declined job card was billed before VAT, in fils.
    pub billed_net: i64,
    /// The advisor billed less than the work done (the branch manager
    /// approved it).
    pub bill_lowered: bool,
    /// The customer is on account: the vehicle may be released unpaid
    /// (`AUT.ACL.17`).
    pub on_account: bool,
    /// The estimate waiting for the customer is the private estimate sent
    /// after the insurance claim was withdrawn (`AUT.RUL.17`, BL-AUT-08): no
    /// labour or part is recorded until it is signed.
    pub private_estimate: bool,
    /// The insurer's written instruction a vehicle was released on after a
    /// total loss (BL-AUT-09).
    pub release_instruction: Option<String>,
    /// When everything that follows the end of the job card was done.
    pub closing_done_at: Option<DateTime>,
    /// When the message asking the customer to collect the vehicle gave up
    /// (`AUT.RUL.17`): the advisor sees "message not delivered".
    pub collect_message_failed_at: Option<DateTime>,
    /// The day the first reminder to collect the vehicle fell due; the storage
    /// fee runs from it (`AUT.RUL.18`).
    pub collect_first_on: Option<NaiveDate>,
    /// The day the second reminder fell due.
    pub collect_second_on: Option<NaiveDate>,
    /// The day the branch manager's task fell due (day 30).
    pub collect_manager_on: Option<NaiveDate>,
    /// The days of storage invoiced at release.
    pub storage_days: i64,
    /// The invoice of the storage fee, raised at release.
    pub storage_invoice_id: Option<i64>,
    /// When the storage fee was invoiced.
    pub storage_invoiced_at: Option<DateTime>,
    /// When the booking was cancelled with the job card.
    pub booking_cancelled_at: Option<DateTime>,
    /// When the vehicle was released.
    pub released_at: Option<DateTime>,
    /// Who took the vehicle.
    pub released_to: Option<String>,
    /// `safe` or `as_is`.
    pub release_condition: Option<String>,
    /// The customer's signature on a release as is.
    pub release_signature: Option<String>,
    /// The user who recorded the release.
    pub released_by: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: JobCardStatus,
    /// The user who opened it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl JobCard {
    /// The number people read: `JOB-00007`.
    pub fn number(&self) -> String {
        format!("JOB-{:05}", self.id)
    }

    /// The job card as a [`Subject`]: `auto.job_card:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(JOB_CARD_SUBJECT_TYPE, self.id)
    }

    /// The workshop as a [`Subject`], when the job card has one.
    pub fn workshop(&self) -> Option<Subject> {
        match (&self.workshop_type, self.workshop_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }

    /// Whether the intake (the vehicle's condition) was recorded.
    pub fn intake_done(&self) -> bool {
        self.intake_recorded_at.is_some()
    }

    /// Whether anything was approved by the customer yet.
    pub fn work_approved(&self) -> bool {
        self.first_approved_at.is_some()
    }

    /// Whether every check of the QC checklist was done.
    pub fn qc_complete(&self) -> bool {
        !self.qc_checks.0.is_empty() && self.qc_checks.0.iter().all(|check| check.done)
    }

    /// The job cards of `vehicle_id`, newest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_vehicle(db: &Db, vehicle_id: i64) -> Result<Vec<JobCard>> {
        JobCard::query()
            .where_(JobCard::VEHICLE_ID.eq(vehicle_id))
            .order_by_desc(JobCard::ID)
            .get(db)
            .await
    }

    /// The job card opened from `booking_id`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_booking(db: &Db, booking_id: i64) -> Result<Option<JobCard>> {
        JobCard::query()
            .where_(JobCard::BOOKING_ID.eq(Some(booking_id)))
            .first(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_card_is_numbered_and_is_a_subject() {
        let card = JobCard {
            id: 7,
            workshop_type: Some("scheduling.location".to_owned()),
            workshop_id: Some(2),
            ..JobCard::default()
        };
        assert_eq!(card.number(), "JOB-00007");
        assert_eq!(card.subject().to_string(), "auto.job_card:7");
        assert_eq!(
            card.workshop().map(|w| w.to_string()),
            Some("scheduling.location:2".to_owned())
        );
        assert_eq!(JobCard::default().workshop(), None);
    }

    #[test]
    fn only_the_four_last_states_are_closed() {
        for open in [
            JobCardStatus::CheckedIn,
            JobCardStatus::Inspecting,
            JobCardStatus::AwaitingApproval,
            JobCardStatus::InProgress,
            JobCardStatus::AwaitingParts,
            JobCardStatus::QualityCheck,
            JobCardStatus::Ready,
        ] {
            assert!(open.is_open(), "{open:?}");
        }
        for closed in [
            JobCardStatus::Delivered,
            JobCardStatus::Declined,
            JobCardStatus::Cancelled,
            JobCardStatus::Voided,
        ] {
            assert!(closed.is_closed(), "{closed:?}");
        }
    }

    #[test]
    fn lines_are_added_before_and_during_the_work_only() {
        assert!(JobCardStatus::CheckedIn.takes_lines());
        assert!(JobCardStatus::Inspecting.takes_lines());
        assert!(JobCardStatus::InProgress.takes_lines());
        assert!(!JobCardStatus::AwaitingApproval.takes_lines());
        assert!(!JobCardStatus::Ready.takes_lines());
        assert!(!JobCardStatus::Cancelled.takes_lines());
    }

    #[test]
    fn the_qc_checklist_is_complete_when_every_check_is_done() {
        let mut card = JobCard::default();
        assert!(!card.qc_complete(), "an empty checklist is not complete");
        card.qc_checks = Json(QcCheck::standard());
        assert!(!card.qc_complete());
        for check in &mut card.qc_checks.0 {
            check.done = true;
        }
        assert!(card.qc_complete());
    }
}
