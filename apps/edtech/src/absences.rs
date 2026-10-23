//! `EDU.RUL.02`: a student has been absent N sessions in a row (10 §7.2, §7.1
//! *Session* `submit_attendance`; the alert BL-EDU-14 mentions). When a session is
//! `held` (or its attendance corrected) the rule counts, for each student who was
//! away, the sessions of their enrolment held in a row up to it with an absence,
//! excused or not; at N (default 2) it tells the academic coordinator and the
//! guardian (the student when adult) once for that run of absences.
//!
//! The alert is kept on the row that triggered it (`alert_sent_at`): a correction
//! that turns that absence into attendance tells the guardian that the earlier
//! message was wrong ([`crate::corrections`]), and the run of absences starts again
//! from the next one.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, now};
use renox::prelude::*;

use crate::attendance::Attendance;
use crate::enrolment::Enrolment;
use crate::enrolments::party_name;
use crate::notices;
use crate::permissions;
use crate::session::{Session, SessionStatus};
use crate::texts::english;

/// N: the absences in a row that make the rule fire.
pub const IN_A_ROW: i64 = 2;

/// The marks of `enrolment_id` in sessions that took place, oldest session first,
/// with the session's start.
pub(crate) async fn held_marks(db: &Db, enrolment_id: i64) -> Result<Vec<(Attendance, Session)>> {
    let mut found = Vec::new();
    for row in Attendance::of_enrolment(db, enrolment_id).await? {
        if let Some(session) = Session::find(db, row.session_id).await?
            && session.status == SessionStatus::Held
        {
            found.push((row, session));
        }
    }
    found.sort_by_key(|(_, session)| session.starts_at);
    Ok(found)
}

/// The run of absences of `enrolment_id` that ends at the session starting at
/// `until` (included): the rows, newest first, from that session back to the last
/// time the student was there.
///
/// # Errors
///
/// The database's errors.
pub async fn run_of_absences(
    db: &Db,
    enrolment_id: i64,
    until: DateTime,
) -> Result<Vec<Attendance>> {
    let marks = held_marks(db, enrolment_id).await?;
    let mut run = Vec::new();
    for (row, session) in marks.into_iter().rev() {
        if session.starts_at > until {
            continue;
        }
        if row.mark.is_absence() {
            run.push(row);
        } else {
            break;
        }
    }
    Ok(run)
}

/// The sessions in a row the student has missed up to now: the run that ends at
/// the latest held session. `0` after they attended.
///
/// # Errors
///
/// The database's errors.
pub async fn absence_streak(db: &Db, enrolment_id: i64) -> Result<i64> {
    let latest = held_marks(db, enrolment_id)
        .await?
        .into_iter()
        .map(|(_, session)| session.starts_at)
        .max();
    match latest {
        Some(until) => Ok(
            i64::try_from(run_of_absences(db, enrolment_id, until).await?.len())
                .unwrap_or(i64::MAX),
        ),
        None => Ok(0),
    }
}

/// Tells the coordinator and the guardian of `row`'s run of absences, once.
async fn alert(
    state: &AppState,
    enrolment: &Enrolment,
    row: &Attendance,
    count: i64,
    on: NaiveDate,
) -> Result {
    let student = party_name(state, "en", enrolment.student_party_id).await;
    let title = english(
        "edtech.absences.title",
        &[("student", &student), ("count", &count)],
    );
    let body = english(
        "edtech.absences.body",
        &[
            ("student", &student),
            ("count", &count),
            ("date", &on),
            ("enrolment", &enrolment.number()),
        ],
    );
    let coordinators = notices::holders(state, permissions::COORDINATOR_ROLE).await?;
    notices::tell_users(
        state,
        enrolment.subject(),
        &enrolment.number(),
        &coordinators,
        &title,
        &body,
    )
    .await?;
    notices::tell_family(state, enrolment, &title, &body).await?;
    Attendance::query()
        .where_(Attendance::ID.eq(row.id))
        .update(
            &state.db,
            &[("alert_sent_at", &Some(now())), ("updated_at", &now())],
        )
        .await?;
    Ok(())
}

/// Runs `EDU.RUL.02` for session `session_id`: each student who was away in it
/// whose run of absences reached [`IN_A_ROW`] and has not been told of it.
/// Answers the students told.
///
/// # Errors
///
/// The database's and the notifier's errors.
pub async fn evaluate(state: &AppState, session_id: i64) -> Result<Vec<i64>> {
    let session = Session::find_or_404(&state.db, session_id).await?;
    let mut told = Vec::new();
    for row in Attendance::of_session(&state.db, session.id).await? {
        if !row.mark.is_absence() || row.alert_sent_at.is_some() {
            continue;
        }
        let run = run_of_absences(&state.db, row.enrolment_id, session.starts_at).await?;
        let count = i64::try_from(run.len()).unwrap_or(i64::MAX);
        if count < IN_A_ROW || run.iter().any(|earlier| earlier.alert_sent_at.is_some()) {
            continue;
        }
        let Some(enrolment) = Enrolment::find(&state.db, row.enrolment_id).await? else {
            continue;
        };
        alert(state, &enrolment, &row, count, session.session_date).await?;
        told.push(row.student_party_id);
    }
    Ok(told)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attendance::Mark;

    #[test]
    fn two_absences_in_a_row_make_the_rule_fire() {
        assert_eq!(IN_A_ROW, 2);
        assert!(Mark::Absent.is_absence() && Mark::Excused.is_absence());
        assert!(!Mark::Late.is_absence());
    }
}
