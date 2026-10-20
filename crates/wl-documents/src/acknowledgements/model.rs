//! The acknowledgement record (wl-documents §3 *Acknowledgement*, §4.3).

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::workforce::{EMPLOYEE_SUBJECT_TYPE, EmployeeId};

/// Where an acknowledgement stands (§4.3).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AckStatus {
    /// The reader must read and acknowledge this version.
    #[default]
    Pending,
    /// Read and confirmed, with evidence.
    Acknowledged,
    /// A newer version was accepted before this one was acknowledged.
    Replaced,
    /// The reader is no longer required.
    Cancelled,
}

/// The method of an acknowledgement in the reader's own session.
pub const METHOD_CLICK: &str = "click-to-accept";

/// The method of an acknowledgement recorded in person with a drawn
/// signature.
pub const METHOD_DRAWN: &str = "drawn";

/// One reader's confirmation that they read one SOP version.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[allow(clippy::duplicated_attributes)] // each `index(..)` is a separate index; `status` repeats by design
#[model(
    table = "wl_documents_sop_acknowledgements",
    state_machine,
    index(sop_id, document_id, status),
    index(employee_id, status)
)]
pub struct SopAcknowledgement {
    /// The primary key.
    pub id: i64,
    /// The SOP.
    #[model(references = "wl_documents_sops")]
    pub sop_id: i64,
    /// The version document it asks for.
    #[model(references = "wl_documents_documents")]
    pub document_id: i64,
    /// The running number of that version.
    pub version_no: i64,
    /// The reader, an `hr.employee`.
    pub employee_id: i64,
    /// The day it was requested (Asia/Dubai).
    pub requested_on: NaiveDate,
    /// The day it is due.
    pub due_date: NaiveDate,
    /// Where it stands; changed only by the machine.
    pub status: AckStatus,
    /// The version or translation the reader opened in their session.
    pub opened_document_id: Option<i64>,
    /// Its checksum when it was opened.
    pub opened_checksum: Option<String>,
    /// When it was opened.
    pub opened_at: Option<DateTime>,
    /// When it was acknowledged.
    pub acknowledged_at: Option<DateTime>,
    /// The user who acknowledged (the reader, or the reader's account in
    /// their own session); empty for a reader without an account.
    pub acknowledged_by: Option<i64>,
    /// The file the acknowledgement covers: the version or a translation.
    pub ack_document_id: Option<i64>,
    /// The language of that file.
    pub language: String,
    /// The checksum of that file.
    pub checksum: String,
    /// `click-to-accept` or `drawn`.
    pub method: String,
    /// The reader's IP address (own session).
    pub ip_address: Option<String>,
    /// The reader's device (own session).
    pub user_agent: Option<String>,
    /// The drawn signature (in person); class `personal`.
    pub signature: Option<String>,
    /// The staff user who witnessed an in-person record.
    pub witness_id: Option<i64>,
    /// When the reader was told, on their own channel, of an in-person
    /// record.
    pub in_person_notified_at: Option<DateTime>,
    /// How many reminders the reader (or their manager) got.
    pub reminders_sent: i64,
    /// The day of the last reminder.
    pub last_reminded_on: Option<NaiveDate>,
    /// The day the manager and the owner role were told; once.
    pub escalated_on: Option<NaiveDate>,
    /// Why it was replaced or cancelled.
    pub ended_reason: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl SopAcknowledgement {
    /// The reader as an id.
    pub fn employee(&self) -> EmployeeId {
        EmployeeId(self.employee_id)
    }

    /// The reader as a kernel subject.
    pub fn reader(&self) -> Subject {
        Subject::new(EMPLOYEE_SUBJECT_TYPE, self.employee_id)
    }

    /// Whether it is pending and past its due date on `day`: *overdue*.
    pub fn overdue_on(&self, day: NaiveDate) -> bool {
        self.status == AckStatus::Pending && day > self.due_date
    }

    /// Whether the record was made in person.
    pub fn in_person(&self) -> bool {
        self.method == METHOD_DRAWN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    #[test]
    fn a_pending_acknowledgement_is_overdue_from_the_day_after_its_due_date() {
        let ack = SopAcknowledgement {
            due_date: day(11, 9),
            ..SopAcknowledgement::default()
        };
        assert!(!ack.overdue_on(day(11, 9)));
        assert!(ack.overdue_on(day(11, 10)));
        let done = SopAcknowledgement {
            status: AckStatus::Acknowledged,
            ..ack
        };
        assert!(!done.overdue_on(day(12, 1)));
    }

    #[test]
    fn the_reader_is_an_employee_subject() {
        let ack = SopAcknowledgement {
            employee_id: 7,
            ..SopAcknowledgement::default()
        };
        assert_eq!(ack.reader(), Subject::new("hr.employee", 7));
        assert_eq!(ack.employee(), EmployeeId(7));
        assert!(!ack.in_person());
    }
}
