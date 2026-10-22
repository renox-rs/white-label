//! The certificate services (10 §4.4 step 5, §7.1 *Certificate*,
//! `EDU.CRT.01`): a draft when the enrolment completes, `issue`, `reissue`,
//! `revoke`, and what the public verification page shows.
//!
//! Decisions:
//!
//! - The completion criteria of `EDU.ACL.06` are, in this base, that the
//!   enrolment is `completed` (the batch ended, or the credits are used up).
//!   Attendance thresholds and pass marks come with the gradebook (`EDU.ASM.01`
//!   is Later).
//! - Only a course enrolment gets a certificate; a package or a membership
//!   has none.
//! - A reissued draft is issued with the enrolment's completion date as the
//!   date its documents are checked on ([`crate::certificate_machine`]).
//! - The verification page is public. It shows the number, the name, the
//!   course and the dates of an issued certificate, says *superseded* or
//!   *revoked* for one that is, and knows nothing of a draft.
//! - The QR image and the PDF in the portal are made from the verification
//!   path by wl-documents (`DOC.GEN.01`); this module keeps the code and the
//!   path they print.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::documents::today;
use wl_kernel::parties::directory;

use crate::certificate::{Certificate, CertificateStatus, new_code};
use crate::certificate_machine::{CertificateInput, ISSUE, REISSUE, REVOKE};
use crate::course::Course;
use crate::batch::Batch;
use crate::db::refused;
use crate::enrolment::{Enrolment, EnrolmentKind, EnrolmentStatus};
use crate::permissions;
use crate::texts::english;

/// The corrections of a reissue (`reissue`).
#[derive(Debug, Clone, Default)]
pub struct Corrections {
    /// The corrected name.
    pub student_name: Option<String>,
    /// The corrected completion date.
    pub completion_date: Option<NaiveDate>,
    /// Whether a recorded KHDA attestation is copied to the new draft.
    pub copy_attestation: bool,
}

/// Whether the completion criteria of `enrolment` are met (`EDU.ACL.06`).
pub fn criteria_met(enrolment: &Enrolment) -> bool {
    enrolment.status == EnrolmentStatus::Completed
}

/// The draft certificate of a completed enrolment, when it should have one:
/// a course enrolment that is `completed` and has no live certificate.
/// Returns the draft created, or `None`.
///
/// # Errors
///
/// The database's errors.
pub async fn evaluate(state: &AppState, enrolment: &Enrolment) -> Result<Option<Certificate>> {
    if enrolment.kind != EnrolmentKind::Course || !criteria_met(enrolment) {
        return Ok(None);
    }
    let live = Certificate::of_enrolment(&state.db, enrolment.id)
        .await?
        .into_iter()
        .any(|c| matches!(c.status, CertificateStatus::Draft | CertificateStatus::Issued));
    if live {
        return Ok(None);
    }
    let Some(batch) = (match enrolment.batch_id {
        Some(id) => Batch::find(&state.db, id).await?,
        None => None,
    }) else {
        return Ok(None);
    };
    let course = Course::find_or_404(&state.db, batch.course_id).await?;
    let name = directory(state)
        .display_name(&state.db, enrolment.student(), "en")
        .await?
        .unwrap_or_else(|| format!("#{}", enrolment.student_party_id));
    let mut draft = Certificate {
        enrolment_id: enrolment.id,
        student_party_id: enrolment.student_party_id,
        student_name: name,
        course_title: course.name_en.clone(),
        approval_no: course.approval_no.clone(),
        states_approval: course.approval_required,
        completion_date: enrolment.completed_on.unwrap_or_else(today),
        verification_code: new_code(),
        ..Certificate::default()
    };
    draft.save(&state.db).await?;
    Ok(Some(draft))
}

/// The input of a run for `certificate`: whether the criteria are met and the
/// date its documents are checked on.
async fn input_for(
    state: &AppState,
    certificate: &Certificate,
    by: &AuthUser,
) -> Result<CertificateInput> {
    let enrolment = Enrolment::find_or_404(&state.db, certificate.enrolment_id).await?;
    // A reissued draft is checked on the day the course was completed.
    let on_date = if certificate.supersedes_id.is_some() {
        certificate.completion_date
    } else {
        today()
    };
    Ok(CertificateInput {
        criteria_met: Some(criteria_met(&enrolment)),
        on_date: Some(on_date),
        by: Some(by.id),
        ..CertificateInput::default()
    })
}

/// Runs the transition `name` as `by`.
async fn run(
    state: &AppState,
    by: &AuthUser,
    certificate: &Certificate,
    name: &str,
    reason: Option<&str>,
    input: CertificateInput,
) -> Result<Certificate> {
    let builder = certificate.transition(name).by(by);
    let builder = match reason {
        Some(reason) => builder.reason(reason),
        None => builder,
    };
    let _outcome = builder
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Certificate::find_or_404(&state.db, certificate.id).await
}

/// `issue`: the draft is published with its number.
///
/// The centre permit (and the course approval when the certificate states it)
/// must be valid on the date in the input (BL-EDU-07, [`crate::documents`]).
///
/// # Errors
///
/// A 403 without `certificates.issue`; a 422 unless the certificate is a
/// `draft` and the criteria are met.
pub async fn issue(state: &AppState, by: &AuthUser, id: i64) -> Result<Certificate> {
    let certificate = Certificate::find_or_404(&state.db, id).await?;
    let mut input = input_for(state, &certificate, by).await?;
    // The centre permit, and the course approval when the certificate states
    // it, valid on the day it is judged on; the machine's guard
    // `document_valid` refuses what the guard refused, naming the documents.
    let on = input.on_date.unwrap_or_else(today);
    input.documents = Some(
        crate::documents::check_certificate_issue(state, &certificate, on, Some(by.id)).await?,
    );
    run(state, by, &certificate, ISSUE, None, input).await
}

/// `reissue`: the issued certificate is superseded for `reason` and a linked
/// draft carries the corrections (the seam of BL-EDU-16). Returns the new
/// draft.
///
/// # Errors
///
/// A 403 without `certificates.issue`; a 422 without a reason, unless the
/// certificate is `issued` or when the criteria are no longer met (then
/// `revoke`).
pub async fn reissue(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
    corrections: Corrections,
) -> Result<Certificate> {
    let certificate = Certificate::find_or_404(&state.db, id).await?;
    let mut input = input_for(state, &certificate, by).await?;
    input.student_name = corrections.student_name;
    input.completion_date = corrections.completion_date;
    input.copy_attestation = corrections.copy_attestation;
    run(state, by, &certificate, REISSUE, Some(reason), input).await?;
    Certificate::query()
        .where_(Certificate::SUPERSEDES_ID.eq(id))
        .order_by_desc(Certificate::ID)
        .first(&state.db)
        .await?
        .ok_or_else(|| refused(english("edtech.certificates.problem.no_draft", &[])))
}

/// `revoke`: the certificate is withdrawn for `reason` (the company admin).
///
/// # Errors
///
/// A 403 without `certificates.revoke`; a 422 without a reason or unless the
/// certificate is `issued`.
pub async fn revoke(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<Certificate> {
    let certificate = Certificate::find_or_404(&state.db, id).await?;
    let input = CertificateInput {
        by: Some(by.id),
        ..CertificateInput::default()
    };
    run(state, by, &certificate, REVOKE, Some(reason), input).await
}

/// Whether `user` may issue certificates.
pub fn may_issue(user: &AuthUser) -> bool {
    user.has_permission(permissions::CERTIFICATES_ISSUE)
}

/// What the public page says of a verification code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verification {
    /// No published certificate has this code.
    Unknown,
    /// An issued certificate.
    Valid(Shown),
    /// Replaced by `replacement`, once the new one is issued.
    Superseded {
        /// The old certificate.
        shown: Shown,
        /// The replacement's number and code; `None` until it is issued
        /// ("superseded, replacement pending").
        replacement: Option<(String, String)>,
    },
    /// Withdrawn.
    Revoked(Shown),
}

/// The fields the public page shows of a certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// `CRT-0150`.
    pub number: String,
    /// The name on it.
    pub student_name: String,
    /// The course on it.
    pub course_title: String,
    /// The day the course was completed.
    pub completion_date: NaiveDate,
    /// The day it was issued.
    pub issued_on: Option<NaiveDate>,
    /// The day it was revoked, for a revoked one.
    pub revoked_on: Option<NaiveDate>,
}

fn shown(certificate: &Certificate) -> Shown {
    Shown {
        number: certificate.number.clone().unwrap_or_default(),
        student_name: certificate.student_name.clone(),
        course_title: certificate.course_title.clone(),
        completion_date: certificate.completion_date,
        issued_on: certificate.issued_on,
        revoked_on: certificate.revoked_on,
    }
}

/// What the public page says of `code`. A draft is not published, so it is
/// unknown.
///
/// # Errors
///
/// The database's errors.
pub async fn verify(db: &Db, code: &str) -> Result<Verification> {
    let Some(certificate) = Certificate::by_code(db, code).await? else {
        return Ok(Verification::Unknown);
    };
    Ok(match certificate.status {
        CertificateStatus::Draft => Verification::Unknown,
        CertificateStatus::Issued => Verification::Valid(shown(&certificate)),
        CertificateStatus::Revoked => Verification::Revoked(shown(&certificate)),
        CertificateStatus::Superseded => {
            let replacement = match certificate.superseded_by_id {
                Some(id) => Certificate::find(db, id)
                    .await?
                    .filter(|c| c.status != CertificateStatus::Draft)
                    .and_then(|c| c.number.map(|number| (number, c.verification_code))),
                None => None,
            };
            Verification::Superseded {
                shown: shown(&certificate),
                replacement,
            }
        }
    })
}

/// Registers the machine.
pub(crate) fn register() {
    crate::certificate_machine::register();
}
