//! The certificate: proof that a student completed a course (10 §6
//! *Certificate*, §7.1 *Certificate*, `EDU.CRT.01`).
//!
//! A certificate is a `draft` when the enrolment completes
//! ([`crate::certificates::evaluate`]), gets its number and becomes `issued`
//! when it is issued, and is never edited after that: a wrong name or date is
//! a `reissue` (the old one is `superseded`, a new draft is linked to it), a
//! certificate given in error is `revoked`. The verification code is printed
//! on the certificate with its QR and opens the public page
//! ([`verification_path`]); the number is assigned at `issue` and never reused.
//! The status moves only through the machine in
//! [`crate::certificate_machine`].

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// The subject type of a certificate.
pub const CERTIFICATE_SUBJECT_TYPE: &str = "edtech.certificate";

/// Where a certificate stands (10 §7.1 *Certificate*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CertificateStatus {
    /// Generated for review.
    #[default]
    Draft,
    /// Published; verifiable on the public page.
    Issued,
    /// Replaced by a reissued certificate (terminal).
    Superseded,
    /// Withdrawn for an error or misconduct (terminal).
    Revoked,
}

impl CertificateStatus {
}

/// A certificate.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "edtech_certificates",
    state_machine,
    unique(number),
    unique(verification_code),
    index(enrolment_id),
    index(status)
)]
pub struct Certificate {
    /// The primary key.
    pub id: i64,
    /// The enrolment it proves.
    pub enrolment_id: i64,
    /// The student, a party.
    pub student_party_id: i64,
    /// The name printed on it.
    pub student_name: String,
    /// The course printed on it.
    pub course_title: String,
    /// The regulator's approval number of the course, when the certificate
    /// states it.
    pub approval_no: Option<String>,
    /// Whether the certificate states the course approval, so that its
    /// validity is checked at `issue` (10 §9).
    pub states_approval: bool,
    /// The day the course was completed.
    pub completion_date: NaiveDate,
    /// `CRT-0150`, assigned when it is issued.
    pub number: Option<String>,
    /// The code on the certificate and its QR.
    pub verification_code: String,
    /// The day it was issued.
    pub issued_on: Option<NaiveDate>,
    /// The user who issued it.
    pub issued_by: Option<i64>,
    /// The certificate this one replaces.
    pub supersedes_id: Option<i64>,
    /// The certificate that replaced this one, once issued.
    pub superseded_by_id: Option<i64>,
    /// Why it was reissued.
    pub reissue_reason: Option<String>,
    /// Why it was revoked.
    pub revoke_reason: Option<String>,
    /// The day it was revoked.
    pub revoked_on: Option<NaiveDate>,
    /// A recorded KHDA attestation, copied to a reissue only when the
    /// coordinator confirms.
    pub attestation_ref: Option<String>,
    /// How many times a lost copy was printed again (`print_copy`).
    pub copies_printed: i64,
    /// The day the last copy was printed.
    pub last_copy_on: Option<NaiveDate>,
    /// The user who printed it.
    pub last_copy_by: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: CertificateStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Certificate {
    /// The certificate as a [`Subject`]: `edtech.certificate:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(CERTIFICATE_SUBJECT_TYPE, self.id)
    }

    /// What staff read: the number once issued, else `Draft 7`.
    pub fn label(&self) -> String {
        self.number
            .clone()
            .unwrap_or_else(|| format!("CRT-draft-{}", self.id))
    }

    /// The path of the public verification page.
    pub fn verification_path(&self) -> String {
        verification_path(&self.verification_code)
    }

    /// The certificate with this verification code.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn by_code(db: &Db, code: &str) -> Result<Option<Certificate>> {
        Certificate::query()
            .where_(Certificate::VERIFICATION_CODE.eq(code.trim().to_uppercase()))
            .first(db)
            .await
    }

    /// The certificates of `enrolment_id`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_enrolment(db: &Db, enrolment_id: i64) -> Result<Vec<Certificate>> {
        Certificate::query()
            .where_(Certificate::ENROLMENT_ID.eq(enrolment_id))
            .order_by(Certificate::ID)
            .get(db)
            .await
    }
}

/// The path of the public page that verifies the certificate with `code`.
pub fn verification_path(code: &str) -> String {
    format!("/verify/certificate/{code}")
}

/// A new verification code: ten capital letters and digits.
pub fn new_code() -> String {
    renox::random_token()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(10)
        .collect::<String>()
        .to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_certificate_is_a_subject_and_has_a_verification_path() {
        let c = Certificate {
            id: 7,
            verification_code: "AB12CD34EF".to_owned(),
            ..Certificate::default()
        };
        assert_eq!(c.subject().to_string(), "edtech.certificate:7");
        assert_eq!(c.verification_path(), "/verify/certificate/AB12CD34EF");
        assert_eq!(c.label(), "CRT-draft-7");
    }

    #[test]
    fn a_verification_code_is_ten_capitals_and_digits_and_new_each_time() {
        let a = new_code();
        let b = new_code();
        assert_eq!(a.len(), 10);
        assert!(a.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()));
        assert_ne!(a, b);
    }
}
