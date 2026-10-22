//! The attendance of a student in a session (10 §4.2 row *A student is absent*,
//! §7.1 *Session* `submit_attendance` and `correct_attendance`, `EDU.ATT.01`;
//! BL-EDU-15, BL-EDU-14).
//!
//! A row ([`Attendance`]) is the mark of one enrolment in one session. It exists
//! once the student is **on the roster** of the session with something to keep: a
//! notice of absence, a guest added to the session (a package student), or the
//! marks submitted. The roster ([`crate::roster`]) is the `active` course
//! enrolments of the batch and the rows the session has.
//!
//! The mark ([`Mark`]):
//!
//! | Mark | Meaning | Credit of a package |
//! |---|---|---|
//! | `pending` | on the roster, no mark yet (a notice was given too late to excuse, or a guest) | none |
//! | `present` | attended | one |
//! | `late` | arrived late: counts as present | one |
//! | `absent` | absent without a valid notice (*unexcused*) | one, unless the absence policy of the enrolment says no |
//! | `excused` | notified at least N hours before the start (default 24), or excused by the coordinator with a reason | none, and a make-up right |
//!
//! Memberships consume nothing and a course-fee enrolment has no credits, so only
//! a package enrolment's mark ever costs one ([`credit_due`]); an excused absence
//! gives a make-up right whatever the enrolment is ([`crate::make_up`]). The policy
//! used is the one stored with the enrolment at enrolment time
//! (`absence_consumes`, `excuse_notice_hours`).
//!
//! `credit_due` says what the mark costs *now*; `consumed` says what the ledger of
//! wl-sales holds for it. [`crate::credits::settle`] moves the ledger to match,
//! after the commit, so a correction only moves the difference.

use std::collections::BTreeMap;

use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::enrolment::{Enrolment, EnrolmentKind};
use crate::make_up;
use crate::session::Session;

/// The subject type of an attendance row.
pub const ATTENDANCE_SUBJECT_TYPE: &str = "edtech.attendance";

/// What a student's mark in a session is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Mark {
    /// On the roster, not marked yet.
    #[default]
    Pending,
    /// Attended.
    Present,
    /// Arrived late; counts as present.
    Late,
    /// Absent without a valid notice.
    Absent,
    /// Absent with a notice in time, or excused by the coordinator.
    Excused,
}

impl Mark {
    /// The mark stored under `name`.
    pub fn parse(name: &str) -> Option<Mark> {
        [
            Mark::Pending,
            Mark::Present,
            Mark::Late,
            Mark::Absent,
            Mark::Excused,
        ]
        .into_iter()
        .find(|mark| mark.as_str() == name.trim())
    }

    /// Whether the student was there: a late arrival counts as present.
    pub fn is_present(self) -> bool {
        matches!(self, Mark::Present | Mark::Late)
    }

    /// Whether the student was away, excused or not (`EDU.RUL.02` counts both).
    pub fn is_absence(self) -> bool {
        matches!(self, Mark::Absent | Mark::Excused)
    }
}

/// Whether the mark `mark` of `enrolment` costs a credit of its package now: a
/// student who was there always does, an absence without a valid notice does when
/// the policy of the enrolment says so (default yes), an excused absence and a
/// mark still pending never; and only a package has credits.
pub fn credit_due(enrolment: &Enrolment, mark: Mark) -> bool {
    if enrolment.kind != EnrolmentKind::Package {
        return false;
    }
    match mark {
        Mark::Present | Mark::Late => true,
        Mark::Absent => enrolment.absence_consumes,
        Mark::Excused | Mark::Pending => false,
    }
}

/// The mark of one enrolment in one session.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "edtech_attendance",
    unique(session_id, enrolment_id),
    index(enrolment_id)
)]
pub struct Attendance {
    /// The primary key.
    pub id: i64,
    /// The session.
    #[model(references = "edtech_sessions")]
    pub session_id: i64,
    /// The enrolment the student attends under.
    #[model(references = "edtech_enrolments")]
    pub enrolment_id: i64,
    /// The student, a party.
    pub student_party_id: i64,
    /// The mark.
    pub mark: Mark,
    /// When the student or the guardian gave notice of the absence.
    pub notice_at: Option<DateTime>,
    /// The user who recorded the notice.
    pub notice_by: Option<i64>,
    /// Why the coordinator excused a late notice.
    pub excuse_reason: Option<String>,
    /// The coordinator who excused it.
    pub excused_by: Option<i64>,
    /// Whether the mark costs a credit of the package now.
    pub credit_due: bool,
    /// Whether the ledger holds a credit for this row.
    pub consumed: bool,
    /// Why no credit could be taken: `no_credit` or `unpaid`.
    pub credit_note: Option<String>,
    /// When the guardian was told of the absences in a row (`EDU.RUL.02`).
    pub alert_sent_at: Option<DateTime>,
    /// The user who submitted the mark.
    pub marked_by: Option<i64>,
    /// When it was submitted.
    pub marked_at: Option<DateTime>,
    /// When it was last corrected.
    pub corrected_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Attendance {
    /// The row as a [`Subject`]: `edtech.attendance:7`. It is the source a
    /// package credit is consumed and given back under.
    pub fn subject(&self) -> Subject {
        Subject::new(ATTENDANCE_SUBJECT_TYPE, self.id)
    }

    /// The rows of session `session_id`, by enrolment.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_session(db: &Db, session_id: i64) -> Result<Vec<Attendance>> {
        Attendance::query()
            .where_(Attendance::SESSION_ID.eq(session_id))
            .order_by(Attendance::ENROLMENT_ID)
            .get(db)
            .await
    }

    /// The row of `enrolment_id` in `session_id`, if any.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn find_of(
        db: &Db,
        session_id: i64,
        enrolment_id: i64,
    ) -> Result<Option<Attendance>> {
        Attendance::query()
            .where_(Attendance::SESSION_ID.eq(session_id))
            .where_(Attendance::ENROLMENT_ID.eq(enrolment_id))
            .first(db)
            .await
    }

    /// The rows of `enrolment_id`, oldest session first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_enrolment(db: &Db, enrolment_id: i64) -> Result<Vec<Attendance>> {
        Attendance::query()
            .where_(Attendance::ENROLMENT_ID.eq(enrolment_id))
            .order_by(Attendance::ID)
            .get(db)
            .await
    }
}

/// One mark of a run of `submit_attendance`, as the service worked it out: the
/// machine's effect stores it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkInput {
    /// The enrolment.
    pub enrolment_id: i64,
    /// The student.
    pub student_party_id: i64,
    /// The mark.
    pub mark: Mark,
    /// Whether it costs a credit of the package ([`credit_due`]).
    pub credit_due: bool,
}

/// Stores `marks` for `session` in the transaction of the machine's run: the row
/// of each student is made or changed, an excused absence gets its make-up right,
/// a mark that is no longer excused loses it. `corrected` says the marks change
/// an earlier submission.
///
/// # Errors
///
/// The database's errors.
pub(crate) async fn store_marks(
    tx: &mut Transaction,
    session: &Session,
    marks: &[MarkInput],
    by: Option<i64>,
    corrected: bool,
) -> Result {
    let at = now();
    let existing: BTreeMap<i64, Attendance> = Attendance::query()
        .where_(Attendance::SESSION_ID.eq(session.id))
        .get(&mut *tx)
        .await?
        .into_iter()
        .map(|row| (row.enrolment_id, row))
        .collect();
    for mark in marks {
        let row = match existing.get(&mark.enrolment_id) {
            Some(found) => {
                Attendance::query()
                    .where_(Attendance::ID.eq(found.id))
                    .update(
                        &mut *tx,
                        &[
                            ("mark", &mark.mark),
                            ("credit_due", &mark.credit_due),
                            ("marked_by", &by),
                            ("marked_at", &Some(found.marked_at.unwrap_or(at))),
                            (
                                "corrected_at",
                                &if corrected { Some(at) } else { found.corrected_at },
                            ),
                            ("updated_at", &at),
                        ],
                    )
                    .await?;
                let mut updated = found.clone();
                updated.mark = mark.mark;
                updated
            }
            None => {
                let mut fresh = Attendance {
                    session_id: session.id,
                    enrolment_id: mark.enrolment_id,
                    student_party_id: mark.student_party_id,
                    mark: mark.mark,
                    credit_due: mark.credit_due,
                    marked_by: by,
                    marked_at: Some(at),
                    corrected_at: corrected.then_some(at),
                    ..Attendance::default()
                };
                fresh.insert(&mut *tx).await?;
                fresh
            }
        };
        if mark.mark == Mark::Excused {
            make_up::grant(tx, &row, session).await?;
        } else {
            make_up::void(tx, row.id).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(consumes: bool) -> Enrolment {
        Enrolment {
            kind: EnrolmentKind::Package,
            absence_consumes: consumes,
            ..Enrolment::default()
        }
    }

    #[test]
    fn a_student_who_was_there_always_costs_a_credit() {
        for mark in [Mark::Present, Mark::Late] {
            assert!(credit_due(&package(true), mark), "{mark:?}");
            assert!(credit_due(&package(false), mark), "{mark:?}");
        }
    }

    #[test]
    fn an_unexcused_absence_costs_one_by_default_and_an_excused_one_never() {
        assert!(credit_due(&package(true), Mark::Absent));
        assert!(!credit_due(&package(false), Mark::Absent));
        assert!(!credit_due(&package(true), Mark::Excused));
        assert!(!credit_due(&package(true), Mark::Pending));
    }

    #[test]
    fn a_membership_or_a_course_fee_has_no_credit() {
        for kind in [EnrolmentKind::Membership, EnrolmentKind::Course] {
            let enrolment = Enrolment {
                kind,
                absence_consumes: true,
                ..Enrolment::default()
            };
            for mark in [Mark::Present, Mark::Late, Mark::Absent, Mark::Excused] {
                assert!(!credit_due(&enrolment, mark), "{kind:?} {mark:?}");
            }
        }
    }

    #[test]
    fn marks_are_read_back_by_name_and_late_counts_as_present() {
        assert_eq!(Mark::parse("late"), Some(Mark::Late));
        assert_eq!(Mark::parse(" excused "), Some(Mark::Excused));
        assert_eq!(Mark::parse("sick"), None);
        assert!(Mark::Late.is_present() && Mark::Present.is_present());
        assert!(Mark::Absent.is_absence() && Mark::Excused.is_absence());
        assert!(!Mark::Pending.is_absence() && !Mark::Pending.is_present());
    }
}
