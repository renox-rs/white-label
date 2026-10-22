//! The completion criteria of an enrolment and their re-evaluation after an
//! attendance correction (10 §7.1 *Session* `correct_attendance`, `EDU.ACL.06`;
//! BL-EDU-14).
//!
//! The criteria of a certificate are that the enrolment is `completed` and the
//! attendance is at least [`ATTENDANCE_PERCENT`] (80). The attendance is the share
//! of the sessions held, among those the student has a mark in, where they were
//! present or late (an excused absence counts as an absence here). An enrolment
//! with no mark at all has no attendance to judge and meets the criteria by being
//! completed; the assessment (`EDU.ASM.01`) is Later.
//!
//! After a correction [`reevaluate`] judges it again:
//!
//! - an enrolment already `completed` **stays completed** (terminal);
//! - a plain **draft** certificate whose criteria now fail is discarded;
//! - an **issued** certificate (or a reissue draft) whose criteria now fail is not
//!   touched: the coordinator gets one task to `reissue` or `revoke` it, never
//!   automatic;
//! - when the criteria are met again and the enrolment is completed with no live
//!   certificate (the draft was discarded), a new draft is made.

use renox::prelude::*;
use wl_kernel::role_tasks::{self, RoleTask};

use crate::absences::held_marks;
use crate::certificate::{Certificate, CertificateStatus};
use crate::certificates;
use crate::enrolment::{Enrolment, EnrolmentKind, EnrolmentStatus};
use crate::enrolments::party_name;
use crate::permissions;
use crate::texts::english;

/// The attendance a certificate asks for, in percent (`EDU.ACL.06`).
pub const ATTENDANCE_PERCENT: i64 = 80;

/// The attendance of `enrolment_id` in percent, whole: the sessions held in which
/// they were present or late over the sessions held in which they have a mark.
/// `None` when they have no mark yet.
///
/// # Errors
///
/// The database's errors.
pub async fn attendance_percent(db: &Db, enrolment_id: i64) -> Result<Option<i64>> {
    let marks = held_marks(db, enrolment_id).await?;
    let counted = i64::try_from(marks.len()).unwrap_or(i64::MAX);
    if counted == 0 {
        return Ok(None);
    }
    let attended = i64::try_from(
        marks
            .iter()
            .filter(|(row, _)| row.mark.is_present())
            .count(),
    )
    .unwrap_or(i64::MAX);
    Ok(Some(attended * 100 / counted))
}

/// Whether the attendance of `enrolment_id` meets the criteria: at least
/// [`ATTENDANCE_PERCENT`], or no mark to judge.
///
/// # Errors
///
/// The database's errors.
pub async fn attendance_met(db: &Db, enrolment_id: i64) -> Result<bool> {
    Ok(attendance_percent(db, enrolment_id)
        .await?
        .is_none_or(|percent| percent >= ATTENDANCE_PERCENT))
}

/// What [`reevaluate`] did for a certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Review {
    /// The criteria are met; nothing changed.
    Met,
    /// A draft whose criteria failed was discarded.
    Discarded,
    /// An issued certificate: the coordinator has a task to reissue or revoke it.
    Task,
    /// The criteria are met again and a new draft was made.
    Redrafted,
}

/// The key of the one open task for the certificate `id`.
pub fn task_key(certificate: i64) -> String {
    format!("edtech.certificate_review:{certificate}")
}

/// Judges the completion criteria of `enrolment` again after its marks changed.
/// Answers what was done, one entry per certificate touched.
///
/// # Errors
///
/// The database's and the task creator's errors.
pub async fn reevaluate(state: &AppState, enrolment: &Enrolment) -> Result<Vec<Review>> {
    if enrolment.kind != EnrolmentKind::Course {
        return Ok(Vec::new());
    }
    let percent = attendance_percent(&state.db, enrolment.id).await?;
    let met = percent.is_none_or(|value| value >= ATTENDANCE_PERCENT);
    let certificates_of = Certificate::of_enrolment(&state.db, enrolment.id).await?;
    let mut done = Vec::new();
    if met {
        let live = certificates_of.iter().any(|c| {
            matches!(
                c.status,
                CertificateStatus::Draft | CertificateStatus::Issued | CertificateStatus::Revoked
            )
        });
        if enrolment.status == EnrolmentStatus::Completed
            && !live
            && certificates::evaluate(state, enrolment).await?.is_some()
        {
            done.push(Review::Redrafted);
        }
        return Ok(done);
    }
    let shown = percent.unwrap_or_default();
    for certificate in certificates_of {
        match certificate.status {
            CertificateStatus::Draft if certificate.supersedes_id.is_none() => {
                Certificate::query()
                    .where_(Certificate::ID.eq(certificate.id))
                    .where_(Certificate::STATUS.eq(CertificateStatus::Draft))
                    .delete(&state.db)
                    .await?;
                done.push(Review::Discarded);
            }
            CertificateStatus::Draft | CertificateStatus::Issued => {
                let student = party_name(state, "en", enrolment.student_party_id).await;
                let title = english(
                    "edtech.completion.task_title",
                    &[("certificate", &certificate.label()), ("percent", &shown)],
                );
                let body = english(
                    "edtech.completion.task_body",
                    &[
                        ("student", &student),
                        ("certificate", &certificate.label()),
                        ("percent", &shown),
                        ("threshold", &ATTENDANCE_PERCENT),
                    ],
                );
                let task = RoleTask::new(
                    certificate.subject(),
                    permissions::COORDINATOR_ROLE,
                    &title,
                    &task_key(certificate.id),
                )
                .description(&body);
                role_tasks::open(state, task).await?;
                done.push(Review::Task);
            }
            CertificateStatus::Revoked | CertificateStatus::Superseded => {}
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_threshold_is_eighty_percent() {
        assert_eq!(ATTENDANCE_PERCENT, 80);
        assert_eq!(task_key(7), "edtech.certificate_review:7");
    }
}
