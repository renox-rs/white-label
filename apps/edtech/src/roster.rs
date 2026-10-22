//! Who is in a session and how an absence is told (10 §4.2 row *A student is
//! absent*, §7.1 *Session* `submit_attendance`; BL-EDU-15).
//!
//! **The roster** of a session: the `active` course enrolments of its batch (a
//! student whose seat is only held, or who is frozen, is not expected), and the
//! students with a row in the session: a package student the counsellor added
//! ([`add_student`]), or one whose notice was recorded. A session of a batch has
//! no row for its course students until something is kept for them.
//!
//! **The notice of absence.** The guardian or the student notifies through the
//! portal (a screen not specified yet) or the counsellor records it with its time
//! ([`record_notice`]). A notice given at least N hours before the session starts
//! (N is the enrolment's `excuse_notice_hours`, default 24) pre-marks the student
//! *excused*: the tutor sees it, the credit is not used and a make-up right is
//! earned. A later notice leaves the student unexcused unless the academic
//! coordinator marks it excused with a reason ([`excuse_late`]). A notice given
//! after the session started is an absence, not a notice.
//!
//! **The marks** a tutor submits ([`prepare`]): every student of the roster is
//! marked. A student pre-marked excused stays excused unless the tutor says they
//! came; a tutor cannot excuse an absence by themself.

use std::collections::BTreeSet;

use renox::chrono::Duration;
use renox::db::{DateTime, now};
use renox::prelude::*;

use crate::attendance::{Attendance, Mark, MarkInput, credit_due};
use crate::db::refused;
use crate::enrolment::{Enrolment, EnrolmentStatus};
use crate::enrolments::party_name;
use crate::permissions;
use crate::session::{Session, SessionStatus};
use crate::texts::english;

/// One student of a session: the enrolment and the row the session keeps for it,
/// if any.
#[derive(Debug, Clone)]
pub struct Student {
    /// The enrolment the student attends under.
    pub enrolment: Enrolment,
    /// The row of the session for it.
    pub row: Option<Attendance>,
}

impl Student {
    /// The mark now: the row's, else `pending`.
    pub fn mark(&self) -> Mark {
        self.row.as_ref().map_or(Mark::Pending, |row| row.mark)
    }
}

/// `Forbidden` unless `user` holds one of `permissions`.
fn require_any(user: &AuthUser, wanted: &[&str]) -> Result {
    if wanted.iter().any(|permission| user.has_permission(permission)) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// The students of `session`, by enrolment.
///
/// # Errors
///
/// The database's errors.
pub async fn roster(db: &Db, session: &Session) -> Result<Vec<Student>> {
    let rows = Attendance::of_session(db, session.id).await?;
    let mut students: Vec<Student> = Vec::new();
    let seated = Enrolment::query()
        .where_(Enrolment::BATCH_ID.eq(session.batch_id))
        .where_(Enrolment::STATUS.eq(EnrolmentStatus::Active))
        .order_by(Enrolment::ID)
        .get(db)
        .await?;
    for enrolment in seated {
        let row = rows.iter().find(|row| row.enrolment_id == enrolment.id).cloned();
        students.push(Student { enrolment, row });
    }
    for row in &rows {
        if students.iter().any(|s| s.enrolment.id == row.enrolment_id) {
            continue;
        }
        if let Some(enrolment) = Enrolment::find(db, row.enrolment_id).await? {
            students.push(Student {
                enrolment,
                row: Some(row.clone()),
            });
        }
    }
    students.sort_by_key(|student| student.enrolment.id);
    Ok(students)
}

/// The `scheduled` session `id` the staff member changes the roster of.
async fn open_session(state: &AppState, id: i64) -> Result<Session> {
    let session = Session::find_or_404(&state.db, id).await?;
    if session.status != SessionStatus::Scheduled {
        return Err(refused(english("edtech.attendance.problem.session_over", &[])));
    }
    Ok(session)
}

/// Adds the student of `enrolment_id` to the session `session_id` as a guest: a
/// package student who takes this session. A student of the batch is on the
/// roster already.
///
/// # Errors
///
/// A 403 unless `by` holds `enrolments.manage` or `batches.manage`; a 422 for a
/// session that is not `scheduled`, an enrolment that is not `active` or a
/// student already on the roster.
pub async fn add_student(
    state: &AppState,
    by: &AuthUser,
    session_id: i64,
    enrolment_id: i64,
) -> Result<Attendance> {
    require_any(
        by,
        &[permissions::ENROLMENTS_MANAGE, permissions::BATCHES_MANAGE],
    )?;
    let session = open_session(state, session_id).await?;
    let enrolment = Enrolment::find_or_404(&state.db, enrolment_id).await?;
    if enrolment.status != EnrolmentStatus::Active {
        return Err(refused(english("edtech.attendance.problem.not_active", &[])));
    }
    let present = roster(&state.db, &session).await?;
    if present.iter().any(|s| s.enrolment.id == enrolment.id) {
        return Err(refused(english("edtech.attendance.problem.already_on", &[])));
    }
    let mut row = Attendance {
        session_id: session.id,
        enrolment_id: enrolment.id,
        student_party_id: enrolment.student_party_id,
        mark: Mark::Pending,
        ..Attendance::default()
    };
    row.save(&state.db).await?;
    Ok(row)
}

/// Records the notice of absence of the student of `enrolment_id` for session
/// `session_id`, given at `given_at`. At least N hours before the start the
/// student is pre-marked *excused*; later the student stays unexcused until the
/// coordinator excuses it ([`excuse_late`]). The first notice counts.
///
/// # Errors
///
/// A 403 unless `by` holds `enrolments.manage` or `batches.manage`; a 422 for a
/// session that is not `scheduled`, a notice given after the start or a student
/// who is not on the roster.
pub async fn record_notice(
    state: &AppState,
    by: &AuthUser,
    session_id: i64,
    enrolment_id: i64,
    given_at: DateTime,
) -> Result<Attendance> {
    require_any(
        by,
        &[permissions::ENROLMENTS_MANAGE, permissions::BATCHES_MANAGE],
    )?;
    let session = open_session(state, session_id).await?;
    if given_at >= session.starts_at {
        return Err(refused(english(
            "edtech.attendance.problem.notice_after_start",
            &[],
        )));
    }
    let student = roster(&state.db, &session)
        .await?
        .into_iter()
        .find(|s| s.enrolment.id == enrolment_id)
        .ok_or_else(|| refused(english("edtech.attendance.problem.not_on_roster", &[])))?;
    let hours = student.enrolment.excuse_notice_hours;
    let excused = session.starts_at - given_at >= Duration::hours(hours);
    match student.row {
        Some(row) => {
            let first = row.notice_at.map_or(given_at, |old| old.min(given_at));
            let mark = if excused || row.mark == Mark::Excused {
                Mark::Excused
            } else {
                row.mark
            };
            Attendance::query()
                .where_(Attendance::ID.eq(row.id))
                .update(
                    &state.db,
                    &[
                        ("notice_at", &Some(first)),
                        ("notice_by", &Some(by.id)),
                        ("mark", &mark),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
            Attendance::find_or_404(&state.db, row.id).await
        }
        None => {
            let mut row = Attendance {
                session_id: session.id,
                enrolment_id: student.enrolment.id,
                student_party_id: student.enrolment.student_party_id,
                mark: if excused { Mark::Excused } else { Mark::Pending },
                notice_at: Some(given_at),
                notice_by: Some(by.id),
                ..Attendance::default()
            };
            row.save(&state.db).await?;
            Ok(row)
        }
    }
}

/// The academic coordinator marks a late notice *excused*, with a reason: the
/// credit is not used and a make-up right is earned.
///
/// # Errors
///
/// A 403 without `batches.manage`; a 422 without a reason, for a session that is
/// not `scheduled`, or for a student with no notice to excuse.
pub async fn excuse_late(
    state: &AppState,
    by: &AuthUser,
    session_id: i64,
    enrolment_id: i64,
    reason: &str,
) -> Result<Attendance> {
    require_any(by, &[permissions::BATCHES_MANAGE])?;
    let session = open_session(state, session_id).await?;
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(refused(english("edtech.attendance.problem.reason", &[])));
    }
    let row = Attendance::find_of(&state.db, session.id, enrolment_id).await?;
    let Some(row) = row.filter(|row| row.notice_at.is_some() && row.mark != Mark::Excused) else {
        return Err(refused(english("edtech.attendance.problem.no_notice", &[])));
    };
    Attendance::query()
        .where_(Attendance::ID.eq(row.id))
        .update(
            &state.db,
            &[
                ("mark", &Mark::Excused),
                ("excuse_reason", &Some(reason.to_owned())),
                ("excused_by", &Some(by.id)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Attendance::find_or_404(&state.db, row.id).await
}

/// The marks of a submission, worked out: every student of the roster has one.
/// A student the tutor does not mention stays *excused* if the notice pre-marked
/// them, else is marked `present` when `default_present` (the page's "all
/// present" start) or the submission is refused naming who is missing. A tutor
/// cannot excuse an absence: *excused* is kept only for a student a notice or the
/// coordinator excused.
///
/// # Errors
///
/// A 422 naming the students not marked, a student who is not on the roster, or
/// an excuse no notice supports.
pub async fn prepare(
    state: &AppState,
    session: &Session,
    given: &[(i64, Mark)],
    default_present: bool,
) -> Result<Vec<MarkInput>> {
    let students = roster(&state.db, session).await?;
    let known: BTreeSet<i64> = students.iter().map(|s| s.enrolment.id).collect();
    if given.iter().any(|(id, _)| !known.contains(id)) {
        return Err(refused(english("edtech.attendance.problem.not_on_roster", &[])));
    }
    let mut marks = Vec::new();
    let mut missing = Vec::new();
    for student in &students {
        let asked = given
            .iter()
            .find(|(id, _)| *id == student.enrolment.id)
            .map(|(_, mark)| *mark)
            .filter(|mark| *mark != Mark::Pending);
        let pre_marked = student.mark() == Mark::Excused;
        let mark = match asked {
            Some(Mark::Excused) if !pre_marked => {
                return Err(refused(english(
                    "edtech.attendance.problem.excuse_needs_notice",
                    &[("hours", &student.enrolment.excuse_notice_hours)],
                )));
            }
            Some(mark) => mark,
            None if pre_marked => Mark::Excused,
            None if default_present => Mark::Present,
            None => {
                missing.push(party_name(state, "en", student.enrolment.student_party_id).await);
                continue;
            }
        };
        marks.push(MarkInput {
            enrolment_id: student.enrolment.id,
            student_party_id: student.enrolment.student_party_id,
            mark,
            credit_due: credit_due(&student.enrolment, mark),
        });
    }
    if !missing.is_empty() {
        return Err(refused(english(
            "edtech.attendance.problem.unmarked",
            &[("students", &missing.join(", "))],
        )));
    }
    Ok(marks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_student_without_a_row_is_not_marked_yet() {
        let student = Student {
            enrolment: Enrolment::default(),
            row: None,
        };
        assert_eq!(student.mark(), Mark::Pending);
        let marked = Student {
            enrolment: Enrolment::default(),
            row: Some(Attendance {
                mark: Mark::Excused,
                ..Attendance::default()
            }),
        };
        assert_eq!(marked.mark(), Mark::Excused);
    }
}
