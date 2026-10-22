//! The tutor eligibility guard (10 §3.3 `EDU.ACL.14`, §7.1 *Batch* `open` and
//! `change_tutor`, *Session* `reschedule` and `change_tutor`, §9, BL-EDU-08).
//!
//! A tutor is assigned to a batch or a session only if their documents are
//! valid on **every** session date they would teach, not only on the first:
//!
//! | Document | Needed when |
//! |---|---|
//! | Police clearance ([`POLICE_CLEARANCE_KIND`]) | minors are enrolled: the course is marked *for minors*, or the batch has a `pending_payment`, `active` or `frozen` enrolment of a student under 18 (an enrolment with a guardian) |
//! | Private tutor permit or freelance visa ([`TUTOR_PERMIT_KIND`]) | the tutor's employment type is `freelance` (`Workforce::is_freelance`) |
//!
//! Each date is asked of the kernel's standard guard,
//! `qualification_valid(employee, kind, date)` (wl-hr `HRM.QUA.01`), which asks
//! wl-documents `document_valid` and answers *no* when there is none. The permit
//! and the freelance visa are one document kind: a tutor holding either one, valid
//! on the date, passes; one who holds an expired permit and a valid visa passes.
//!
//! Decisions:
//!
//! - **Override: never.** Police clearance is *best practice, not a legal block*
//!   (`⚠ open: lawyer (L-19)`); it is built with the default of the spec, override
//!   `never`. A client that switches the kind to *approval by the owner*
//!   (BL-EDU-17) changes how wl-documents answers a check of the transition (the
//!   providers in [`crate::documents`]); this guard, which judges the *new* tutor
//!   before they are on the record, stays a refusal.
//! - **All dates.** The guard returns every failing date; the refusal names the
//!   first and lists the others so that the coordinator sees how many sessions
//!   the document leaves out.
//! - **No minors' data.** The refusal names the tutor and the document, never a
//!   student or a guardian.
//! - **No employee.** A tutor that is only a resource has no documents to ask: the
//!   police clearance cannot be checked, so a batch with minors refuses it ("has
//!   no employee record"); an adult batch needs no document and passes.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::parties::directory;
use wl_kernel::qualifications::qualifications;
use wl_kernel::workforce::{EmployeeId, workforce};

use crate::batch::Batch;
use crate::course::Course;
use crate::db::refused;
use crate::enrolment::{Enrolment, EnrolmentStatus};
use crate::seed::{POLICE_CLEARANCE_KIND, TUTOR_PERMIT_KIND};
use crate::texts::english;
use crate::tutors::Tutor;

/// How many failing dates a refusal lists before it says how many more.
const LISTED_DATES: usize = 8;

/// One date a tutor's document does not cover.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Failure {
    /// The session date.
    pub date: NaiveDate,
    /// The document kind that is missing or expired on it.
    pub kind: &'static str,
}

/// Whether minors are enrolled in `batch` now: its course is marked *for
/// minors*, or one of its enrolments that holds a seat (`pending_payment`,
/// `active`, `frozen`) is a student under 18, which is an enrolment with a
/// guardian.
///
/// # Errors
///
/// The database's errors.
pub async fn minors_enrolled(db: &Db, batch: &Batch) -> Result<bool> {
    if let Some(course) = Course::find(db, batch.course_id).await?
        && course.for_minors
    {
        return Ok(true);
    }
    let minors = Enrolment::query()
        .where_(Enrolment::BATCH_ID.eq(batch.id))
        .where_(Enrolment::STATUS.is_in([
            EnrolmentStatus::PendingPayment,
            EnrolmentStatus::Active,
            EnrolmentStatus::Frozen,
        ]))
        .where_(Enrolment::GUARDIAN_PARTY_ID.is_not_null())
        .count(db)
        .await?;
    Ok(minors > 0)
}

/// Whether the employee behind the tutor is a freelancer.
///
/// # Errors
///
/// The workforce's errors.
pub async fn is_freelance(state: &AppState, employee: Option<i64>) -> Result<bool> {
    match employee {
        Some(employee) => {
            workforce(state)
                .is_freelance(state, EmployeeId(employee))
                .await
        }
        None => Ok(false),
    }
}

/// The document kinds a tutor must hold to teach `batch`.
///
/// # Errors
///
/// The database's and the workforce's errors.
pub async fn required_kinds(
    state: &AppState,
    employee: Option<i64>,
    batch: &Batch,
) -> Result<Vec<&'static str>> {
    let mut kinds = Vec::new();
    if minors_enrolled(&state.db, batch).await? {
        kinds.push(POLICE_CLEARANCE_KIND);
    }
    if is_freelance(state, employee).await? {
        kinds.push(TUTOR_PERMIT_KIND);
    }
    Ok(kinds)
}

/// The dates of `dates`, sorted, each once, on which `employee` lacks a valid
/// document of one of `kinds`.
///
/// # Errors
///
/// The qualification guard's errors.
pub async fn failures(
    state: &AppState,
    employee: Option<i64>,
    kinds: &[&'static str],
    dates: &[NaiveDate],
) -> Result<Vec<Failure>> {
    let mut days: Vec<NaiveDate> = dates.to_vec();
    days.sort_unstable();
    days.dedup();
    let mut found = Vec::new();
    for kind in kinds {
        for date in &days {
            let valid = match employee {
                Some(employee) => {
                    qualifications(state)
                        .qualification_valid(state, EmployeeId(employee), kind, *date, None, None)
                        .await?
                        .valid
                }
                None => false,
            };
            if !valid {
                found.push(Failure { date: *date, kind });
            }
        }
    }
    found.sort();
    Ok(found)
}

/// The name of a document kind in a refusal.
pub fn kind_text(kind: &str) -> String {
    english(
        if kind == POLICE_CLEARANCE_KIND {
            "edtech.tutors.kind.police_clearance"
        } else {
            "edtech.tutors.kind.tutor_permit"
        },
        &[],
    )
}

/// The tutor as a refusal names them: the name of the employee's person, else
/// `The tutor`.
pub async fn tutor_text(state: &AppState, employee: Option<i64>) -> String {
    let fallback = english("edtech.tutors.the_tutor", &[]);
    let Some(employee) = employee else {
        return fallback;
    };
    let Ok(Some(profile)) = workforce(state).staff(state, EmployeeId(employee)).await else {
        return fallback;
    };
    directory(state)
        .display_name(&state.db, profile.party, "en")
        .await
        .ok()
        .flatten()
        .unwrap_or(fallback)
}

/// The refusal for `failures`: the tutor, each document that fails with the
/// first date it fails on, and the dates (the first eight, then how many more).
pub fn refusal(tutor: &str, failures: &[Failure], no_employee: bool) -> Error {
    let mut kinds: Vec<&'static str> = failures.iter().map(|f| f.kind).collect();
    kinds.sort_unstable();
    kinds.dedup();
    let mut lines = Vec::new();
    for kind in kinds {
        let dates: Vec<NaiveDate> = failures
            .iter()
            .filter(|f| f.kind == kind)
            .map(|f| f.date)
            .collect();
        let first = dates.first().copied().unwrap_or(NaiveDate::MIN);
        let mut listed = dates
            .iter()
            .take(LISTED_DATES)
            .map(NaiveDate::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        if dates.len() > LISTED_DATES {
            listed.push_str(&english(
                "edtech.tutors.problem.more_dates",
                &[("count", &(dates.len() - LISTED_DATES))],
            ));
        }
        let key = if no_employee {
            "edtech.tutors.problem.no_employee"
        } else {
            "edtech.tutors.problem.not_eligible"
        };
        lines.push(english(
            key,
            &[
                ("tutor", &tutor),
                ("kind", &kind_text(kind)),
                ("first", &first),
                ("dates", &listed),
            ],
        ));
    }
    refused(lines.join("\n"))
}

/// Refuses unless `tutor` holds the documents `batch` needs on every one of
/// `dates` (`EDU.ACL.14`).
///
/// # Errors
///
/// A 422 naming the missing kind and the first failing date; the guards' errors.
pub async fn check(state: &AppState, tutor: &Tutor, batch: &Batch, dates: &[NaiveDate]) -> Result {
    let kinds = required_kinds(state, tutor.employee_id, batch).await?;
    if kinds.is_empty() || dates.is_empty() {
        return Ok(());
    }
    let missing = failures(state, tutor.employee_id, &kinds, dates).await?;
    if missing.is_empty() {
        return Ok(());
    }
    let name = tutor_text(state, tutor.employee_id).await;
    Err(refusal(&name, &missing, tutor.employee_id.is_none()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    #[test]
    fn the_refusal_names_the_kind_the_first_date_and_the_others() {
        let failures: Vec<Failure> = (16..=18)
            .map(|d| Failure {
                date: day(12, d),
                kind: POLICE_CLEARANCE_KIND,
            })
            .collect();
        let Error::Status(_, message) = refusal("Layla Hassan", &failures, false) else {
            panic!("a refusal");
        };
        assert!(message.contains("Layla Hassan"), "{message}");
        assert!(message.contains("police clearance"), "{message}");
        assert!(message.contains("2026-12-16"), "{message}");
        assert!(message.contains("2026-12-17"), "{message}");
    }

    #[test]
    fn failures_sort_by_date_first() {
        let mut found = [
            Failure {
                date: day(12, 20),
                kind: TUTOR_PERMIT_KIND,
            },
            Failure {
                date: day(12, 16),
                kind: POLICE_CLEARANCE_KIND,
            },
        ];
        found.sort();
        assert_eq!(found[0].date, day(12, 16));
    }
}
