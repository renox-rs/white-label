//! Correcting the attendance of a held session (10 §3.3 `EDU.ACL.04` and
//! `EDU.ACL.16`, §4.2 row *Attendance was marked wrong*, §4.5 step 4, §7.1
//! *Session* `correct_attendance`; BL-EDU-14).
//!
//! A `held` session is not terminal: its marks can be corrected. **The tutor** may
//! within 24 hours of the scheduled end ([`crate::session_machine::TUTOR_WINDOW_HOURS`],
//! the clock of the centre, Asia/Dubai); later only **the academic coordinator**
//! (`batches.manage`), and both with a reason. One correction changes any number of
//! marks of the session; what follows it, after the commit:
//!
//! - **The credits.** Each corrected mark is priced again by the policy stored with
//!   the enrolment and only the difference moves ([`crate::credits`]): an unexcused
//!   absence that becomes attendance costs the same (no change with the default
//!   policy); attendance or an unexcused absence that becomes *excused* gives its
//!   credit back (`reverse`, a package `completed` at zero comes back to `active`);
//!   an excused absence that becomes attendance uses one. With no credit left the
//!   correction is saved and the counsellor has one task to sell credits.
//! - **The make-up right** of an excused absence is given or, if unused, taken back
//!   ([`crate::make_up`]).
//! - **The guardian** is told when the correction changes an absence that had
//!   triggered an `EDU.RUL.02` alert ([`crate::absences`]); the run of absences
//!   starts again from the corrected mark.
//! - **The completion** of the enrolment is judged again ([`crate::completion`]):
//!   an enrolment already `completed` stays so; a draft certificate whose criteria
//!   now fail is discarded; an issued one is left alone and the coordinator gets a
//!   task to `reissue` or `revoke` it.
//! - **The tutor's pay** is changed only when a timesheet entry changes. A tutor is
//!   paid per session taught (`EDU.RUL.10`), which a student's mark does not change,
//!   so a correction of marks moves no entry; the adjustment timesheet of a
//!   `locked` one (`HRM.ACL.08`, wl-attendance) is for a correction that does
//!   (a session that did not take place), which the machine does not offer here.

use renox::chrono::Duration;
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::documents::today;

use crate::absences;
use crate::attendance::{Attendance, Mark, MarkInput, credit_due};
use crate::completion;
use crate::credits;
use crate::db::refused;
use crate::enrolment::Enrolment;
use crate::enrolments::party_name;
use crate::notices;
use crate::permissions;
use crate::session::{Session, SessionStatus};
use crate::session_machine::{
    CORRECT_ATTENDANCE, CORRECT_ATTENDANCE_COORDINATOR, SessionInput, TUTOR_WINDOW_HOURS,
};
use crate::texts::english;

/// Whether the tutor can still correct the attendance of `session` at the moment
/// the clock says: until 24 hours after the scheduled end.
pub fn tutor_window_open(session: &Session) -> bool {
    now() <= session.ends_at + Duration::hours(TUTOR_WINDOW_HOURS)
}

/// Whether `user` may correct the attendance of `session` now: the coordinator
/// always, the tutor (`sessions.teach`) within the window.
pub fn may_correct(user: &AuthUser, session: &Session) -> bool {
    session.status == SessionStatus::Held
        && (user.has_permission(permissions::BATCHES_MANAGE)
            || (user.has_permission(permissions::SESSIONS_TEACH) && tutor_window_open(session)))
}

/// Corrects the marks `changes` (the enrolment and its new mark) of the `held`
/// session `id`, for `reason`, as `by`.
///
/// # Errors
///
/// A 403 for a user who is neither a tutor nor the coordinator; a 422 for a
/// session that is not `held`, a tutor after the 24 hours (the message sends them
/// to the coordinator), a missing reason, a student with no mark in the session, a
/// mark that is none, or marks that are the marks already there.
pub async fn correct_attendance(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    changes: &[(i64, Mark)],
    reason: &str,
) -> Result<Session> {
    let session = Session::find_or_404(&state.db, id).await?;
    let coordinator = by.has_permission(permissions::BATCHES_MANAGE);
    if !coordinator && !by.has_permission(permissions::SESSIONS_TEACH) {
        return Err(Error::Forbidden);
    }
    if session.status != SessionStatus::Held {
        return Err(refused(english("edtech.corrections.problem.not_held", &[])));
    }
    if reason.trim().is_empty() {
        return Err(refused(english("edtech.corrections.problem.reason", &[])));
    }
    if !coordinator && !tutor_window_open(&session) {
        return Err(refused(english("edtech.corrections.problem.window_over", &[])));
    }
    let rows = Attendance::of_session(&state.db, session.id).await?;
    let mut marks: Vec<MarkInput> = Vec::new();
    let mut before: Vec<Attendance> = Vec::new();
    for (enrolment_id, mark) in changes {
        let Some(row) = rows.iter().find(|row| row.enrolment_id == *enrolment_id) else {
            return Err(refused(english("edtech.attendance.problem.not_on_roster", &[])));
        };
        if *mark == Mark::Pending {
            return Err(refused(english("edtech.attendance.problem.mark", &[])));
        }
        if row.mark == *mark || marks.iter().any(|m| m.enrolment_id == *enrolment_id) {
            continue;
        }
        let enrolment = Enrolment::find_or_404(&state.db, *enrolment_id).await?;
        marks.push(MarkInput {
            enrolment_id: *enrolment_id,
            student_party_id: row.student_party_id,
            mark: *mark,
            credit_due: credit_due(&enrolment, *mark),
        });
        before.push(row.clone());
    }
    if marks.is_empty() {
        return Err(refused(english("edtech.corrections.problem.nothing", &[])));
    }
    let name = if coordinator {
        CORRECT_ATTENDANCE_COORDINATOR
    } else {
        CORRECT_ATTENDANCE
    };
    let input = SessionInput {
        by: Some(by.id),
        on: Some(today()),
        marks: marks.clone(),
        ..SessionInput::default()
    };
    let _outcome = session
        .transition(name)
        .by(by)
        .reason(reason.trim())
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    follow(state, &session, &before, &marks).await?;
    Session::find_or_404(&state.db, session.id).await
}

/// What follows a correction, after its commit: the credits, the guardian, the
/// run of absences and the completion criteria.
async fn follow(
    state: &AppState,
    session: &Session,
    before: &[Attendance],
    marks: &[MarkInput],
) -> Result {
    for mark in marks {
        let Some(row) = Attendance::find_of(&state.db, session.id, mark.enrolment_id).await? else {
            continue;
        };
        // A credit the ledger could not serve is kept and tried again by the job.
        let _kept_for_the_job = credits::settle(state, row.id).await;
        let was = before.iter().find(|old| old.enrolment_id == mark.enrolment_id);
        if let Some(old) = was
            && old.alert_sent_at.is_some()
            && old.mark.is_absence()
            && !mark.mark.is_absence()
        {
            tell_guardian(state, session, &row, mark.mark).await?;
        }
    }
    absences::evaluate(state, session.id).await?;
    for mark in marks {
        if let Some(enrolment) = Enrolment::find(&state.db, mark.enrolment_id).await? {
            completion::reevaluate(state, &enrolment).await?;
        }
    }
    Ok(())
}

/// Tells the guardian that the absence they were alerted about was corrected, and
/// takes the alert off the row: the next run of absences is a new one.
async fn tell_guardian(
    state: &AppState,
    session: &Session,
    row: &Attendance,
    now_marked: Mark,
) -> Result {
    let Some(enrolment) = Enrolment::find(&state.db, row.enrolment_id).await? else {
        return Ok(());
    };
    let student = party_name(state, "en", enrolment.student_party_id).await;
    let mark = english(&format!("edtech.attendance.mark.{}", now_marked.as_str()), &[]);
    let title = english(
        "edtech.corrections.guardian_title",
        &[("student", &student), ("date", &session.session_date)],
    );
    let body = english(
        "edtech.corrections.guardian_body",
        &[
            ("student", &student),
            ("date", &session.session_date),
            ("mark", &mark),
        ],
    );
    notices::tell_family(state, &enrolment, &title, &body).await?;
    Attendance::query()
        .where_(Attendance::ID.eq(row.id))
        .update(
            &state.db,
            &[
                ("alert_sent_at", &None::<renox::db::DateTime>),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ended(hours_ago: i64) -> Session {
        Session {
            status: SessionStatus::Held,
            ends_at: now() - Duration::hours(hours_ago),
            ..Session::default()
        }
    }

    #[test]
    fn the_tutor_has_twenty_four_hours_after_the_end() {
        assert!(tutor_window_open(&ended(1)));
        assert!(tutor_window_open(&ended(23)));
        assert!(!tutor_window_open(&ended(25)));
        assert_eq!(TUTOR_WINDOW_HOURS, 24);
    }
}
