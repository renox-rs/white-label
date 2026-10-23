//! BL-EDU-10 (#445): `EDU.RUL.12` flags the sessions of a tutor who will not be
//! eligible or available (a document expiring or expired, a notice, a manual
//! availability block, leave), gives the academic coordinator one task per tutor
//! and trigger, tells the centre manager of a session still flagged 24 hours
//! before it starts, changes no tutor by itself and, when a run fails five times,
//! alerts the owner and the company admin.
//!
//! The days are relative to today: the clearance of the tutor ends on `expiry()`
//! (day 40) and the sessions of the scene are on the next day, two days after it
//! and 36 days after it, as 2026-12-16, 2026-12-18 and 2027-01-20 are in the
//! spec's example of a clearance that ends on 2026-12-15.

mod support;

use std::time::Duration;

use edtech::notices::NOTICE;
use edtech::permissions as perm;
use edtech::sessions;
use edtech::tutor_alerts;
use edtech::{Batch, Session, SessionStatus, TutorAlert};
use renox::chrono::{NaiveDate, TimeDelta};
use renox::prelude::*;
use support::{Centre, Messages, at, boot, day, next_weekday};
use wl_kernel::compliance::ComplianceAlert;
use wl_kernel::documents::{DocumentAccepted, DocumentExpired, DocumentExpiring, DocumentId};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::workforce::{
    EmployeeId, EmployeeNoticeGiven, EmployeeNoticeWithdrawn, LeaveApproved, QualificationExpired,
};
use wl_kernel::{COMPANY_ADMIN_ROLE, Subject};
use wl_scheduling::{AvailabilityBlock, Resource};

/// The task of a tutor alert is found by this key prefix.
const TASK_KEY: &str = "edtech.tutor_alert";

/// The last day the clearance of the tutor is valid.
fn expiry() -> NaiveDate {
    day(40)
}

/// The tutor, the batch she teaches (a batch for minors) and her three sessions.
struct Scene {
    t: Centre,
    tutor: (Resource, i64),
    batch: Batch,
    /// The next day after the expiry, 10:00.
    first: Session,
    /// Two days after it.
    second: Session,
    /// 36 days after it: outside the window of the first run.
    third: Session,
}

/// A `scheduled` session of `batch` on `date` at 10:00 taught by `tutor`.
async fn session_on(
    t: &Centre,
    batch: &Batch,
    number: i64,
    date: NaiveDate,
    tutor: &(Resource, i64),
) -> Session {
    let mut session = Session {
        batch_id: batch.id,
        number,
        session_date: date,
        starts_at: at(date, 10, 0),
        ends_at: at(date, 11, 0),
        tutor_resource_id: Some(tutor.0.id),
        tutor_employee_id: Some(tutor.1),
        room_resource_id: batch.room_resource_id,
        ..Session::default()
    };
    session.save(t.db()).await.unwrap();
    session
}

/// The police clearance of the tutor is valid through `until`.
fn clearance_until(t: &Centre, employee: i64, until: NaiveDate) {
    t.documents.set(
        &Centre::employee_subject(employee),
        "police_clearance",
        Some(until),
    );
}

/// Layla, whose clearance is valid through the expiry, and her three sessions
/// in a batch for minors.
async fn scene() -> Scene {
    let t = boot().await;
    let tutor = t.employed_tutor("Layla Hassan", false).await;
    clearance_until(&t, tutor.1, expiry());
    let room = t.room("Room 1", 14).await;
    let course = t.minors_course().await;
    let batch = t
        .batch_with(
            &course,
            (&tutor.0, Some(tutor.1)),
            &room,
            next_weekday(day(7), 1),
            &[1, 3],
        )
        .await;
    let first = session_on(&t, &batch, 1, expiry() + TimeDelta::days(1), &tutor).await;
    let second = session_on(&t, &batch, 2, expiry() + TimeDelta::days(3), &tutor).await;
    let third = session_on(&t, &batch, 3, expiry() + TimeDelta::days(36), &tutor).await;
    Scene {
        t,
        tutor,
        batch,
        first,
        second,
        third,
    }
}

impl Scene {
    /// `DocumentExpiring` for the clearance, as wl-documents emits it.
    fn expiring(&self) -> DocumentExpiring {
        DocumentExpiring {
            document_id: DocumentId(1),
            subject: Centre::employee_subject(self.tutor.1),
            kind: "police_clearance".to_owned(),
            expires_on: expiry(),
            days_left: 40,
            offset: Some(30),
            affected: Vec::new(),
        }
    }

    async fn session(&self, session: &Session) -> Session {
        Session::find_or_404(self.t.db(), session.id).await.unwrap()
    }

    /// The tasks of tutor alerts opened so far.
    fn tasks(&self) -> Vec<support::tasks::Opened> {
        self.t.tasks.with_key(TASK_KEY)
    }
}

/// Given T's police clearance expiring and T teaching batch B's sessions on the
/// next day, two days later and 36 days later, on `DocumentExpiring` the first
/// two are flagged (they are within 30 days of the first) and the coordinator
/// has one task; the third is not flagged; nothing is changed by itself; the
/// expired event and the qualification's expiry add neither a task nor a flag.
#[renox::test]
async fn edu_10_a_clearance_expiring_flags_the_sessions_in_the_window_and_gives_one_task() {
    let s = scene().await;
    let coordinator =
        s.t.holder("Cora Coordinator", perm::COORDINATOR_ROLE, None)
            .await;

    s.t.state().emit(s.expiring()).await.unwrap();

    let (first, second, third) = (
        s.session(&s.first).await,
        s.session(&s.second).await,
        s.session(&s.third).await,
    );
    assert!(first.is_flagged() && second.is_flagged());
    assert_eq!(
        first.flag_reason.as_deref(),
        Some("document:police_clearance")
    );
    assert!(!third.is_flagged(), "36 days is beyond the window");
    for session in [&first, &second, &third] {
        assert_eq!(
            session.tutor_resource_id,
            Some(s.tutor.0.id),
            "no tutor is changed by itself"
        );
        assert_eq!(session.status, SessionStatus::Scheduled);
    }
    let tasks = s.tasks();
    assert_eq!(tasks.len(), 1, "one task per tutor and trigger");
    assert_eq!(tasks[0].assignee_user(), Some(coordinator.id));
    let body = &tasks[0].ticket.description;
    assert!(body.contains(&first.session_date.to_string()), "{body}");
    assert!(body.contains(&second.session_date.to_string()), "{body}");
    assert!(!body.contains(&third.session_date.to_string()), "{body}");
    assert!(body.contains(&s.batch.number()), "{body}");

    // The same trigger again, by the other events: nothing is doubled.
    s.t.state()
        .emit(DocumentExpired {
            document: DocumentId(1),
            subject: Centre::employee_subject(s.tutor.1),
            kind: "police_clearance".to_owned(),
            expires_on: expiry(),
            blocks: Vec::new(),
            blocked: Vec::new(),
        })
        .await
        .unwrap();
    s.t.state()
        .emit(QualificationExpired {
            employee: EmployeeId(s.tutor.1),
            qualification: None,
            kind: "police_clearance".to_owned(),
            scope: Vec::new(),
            roles: Vec::new(),
            expires_on: expiry(),
            affected: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(s.tasks().len(), 1);
    assert_eq!(
        TutorAlert::of_employee(s.t.db(), s.tutor.1)
            .await
            .unwrap()
            .len(),
        1
    );
    // A document of another kind or another subject is none of its business.
    s.t.state()
        .emit(DocumentExpiring {
            kind: "centre_permit".to_owned(),
            ..s.expiring()
        })
        .await
        .unwrap();
    assert_eq!(s.tasks().len(), 1);
}

/// The daily run five days after the expiry does not flag the session 36 days
/// after it (the window ends 35 days after); the run on the sixth day flags it
/// and adds it to the task.
#[renox::test]
async fn edu_10_the_daily_run_moves_the_window_one_day_at_a_time() {
    let s = scene().await;
    s.t.state().emit(s.expiring()).await.unwrap();
    assert!(!s.session(&s.third).await.is_flagged());

    tutor_alerts::run_daily(s.t.state(), expiry() + TimeDelta::days(5))
        .await
        .unwrap();
    assert!(
        !s.session(&s.third).await.is_flagged(),
        "the window ends a day short"
    );
    assert!(s.tasks()[0].comments.is_empty());

    tutor_alerts::run_daily(s.t.state(), expiry() + TimeDelta::days(6))
        .await
        .unwrap();

    let third = s.session(&s.third).await;
    assert!(third.is_flagged(), "the window now reaches it");
    let tasks = s.tasks();
    assert_eq!(tasks.len(), 1, "the task is updated, not doubled");
    assert_eq!(tasks[0].comments.len(), 1);
    assert!(tasks[0].comments[0].contains(&third.session_date.to_string()));
}

/// With a session still flagged 24 hours before it starts, the centre manager
/// is told, once; not before.
#[renox::test]
async fn edu_10_the_centre_manager_is_told_a_day_before_a_flagged_session() {
    let s = scene().await;
    let manager =
        s.t.holder("Mona Manager", perm::CENTRE_MANAGER_ROLE, None)
            .await;
    s.t.state().emit(s.expiring()).await.unwrap();
    let day_before = s.first.starts_at - TimeDelta::hours(24);

    let early = tutor_alerts::escalate(s.t.state(), day_before - TimeDelta::minutes(1))
        .await
        .unwrap();
    assert_eq!(early, 0);
    assert!(
        s.t.messages.of_type(NOTICE).is_empty(),
        "not before the 24 hours"
    );

    let told = tutor_alerts::escalate(s.t.state(), day_before)
        .await
        .unwrap();

    assert_eq!(told, 1);
    let sent = s.t.messages.of_type(NOTICE);
    assert_eq!(sent.len(), 1);
    assert!(Messages::told_user(&sent[0], manager.id));
    let text = Messages::text(&sent[0]);
    assert!(text.contains(&s.first.session_date.to_string()), "{text}");
    assert!(text.contains("police clearance"), "{text}");
    assert!(s.session(&s.first).await.flag_escalated_at.is_some());

    // Once: asking again tells nobody again.
    tutor_alerts::escalate(s.t.state(), day_before + TimeDelta::hours(1))
        .await
        .unwrap();
    assert_eq!(s.t.messages.of_type(NOTICE).len(), 1);
    // The second session is two days later: not yet.
    assert!(s.session(&s.second).await.flag_escalated_at.is_none());
}

/// A session that is no longer flagged is not escalated: the tutor was changed
/// before the day.
#[renox::test]
async fn edu_10_a_resolved_session_is_not_escalated() {
    let s = scene().await;
    let _manager =
        s.t.holder("Mona Manager", perm::CENTRE_MANAGER_ROLE, None)
            .await;
    s.t.state().emit(s.expiring()).await.unwrap();
    let uma = s.t.employed_tutor("Uma Nasser", false).await;
    clearance_until(&s.t, uma.1, day(400));
    let amal = s.t.coordinator().await;

    let moved = sessions::change_tutor(s.t.state(), &amal, s.first.id, uma.0.id, Some(uma.1))
        .await
        .unwrap();

    assert!(!moved.is_flagged(), "the flag clears with the new tutor");
    assert!(
        s.session(&s.second).await.is_flagged(),
        "the other one stays flagged"
    );
    let told = tutor_alerts::escalate(s.t.state(), s.first.starts_at - TimeDelta::hours(24))
        .await
        .unwrap();
    assert_eq!(told, 0);
    assert!(s.t.messages.of_type(NOTICE).is_empty());

    // Cancelling the other one resolves it too.
    sessions::cancel(s.t.state(), &amal, s.second.id, "Room closed", None)
        .await
        .unwrap();
    assert!(!s.session(&s.second).await.is_flagged());
}

/// A renewal accepted: the flags come off and the task is completed, at once on
/// `DocumentAccepted`.
#[renox::test]
async fn edu_10_a_renewal_clears_the_flags_and_completes_the_task() {
    let s = scene().await;
    s.t.state().emit(s.expiring()).await.unwrap();
    assert!(s.session(&s.first).await.is_flagged());
    assert!(s.tasks()[0].is_open());

    clearance_until(&s.t, s.tutor.1, day(400));
    s.t.state()
        .emit(DocumentAccepted {
            document: DocumentId(2),
            subject: Centre::employee_subject(s.tutor.1),
            kind: "police_clearance".to_owned(),
            expires_on: Some(day(400)),
        })
        .await
        .unwrap();

    assert!(!s.session(&s.first).await.is_flagged());
    assert!(!s.session(&s.second).await.is_flagged());
    let tasks = s.tasks();
    assert!(!tasks[0].is_open(), "the task is completed");
    assert!(
        tasks[0]
            .completed
            .as_deref()
            .unwrap_or("")
            .contains("Nothing is left")
    );
    let alert = &TutorAlert::of_employee(s.t.db(), s.tutor.1).await.unwrap()[0];
    assert!(alert.closed_at.is_some());
}

/// A tutor whose sessions are in a batch with adults only does not need the
/// clearance: nothing is flagged and nobody gets a task.
#[renox::test]
async fn edu_10_a_batch_without_minors_is_not_flagged_for_the_clearance() {
    let t = boot().await;
    let tutor = t.employed_tutor("Layla Hassan", false).await;
    clearance_until(&t, tutor.1, expiry());
    let room = t.room("Room 1", 14).await;
    let course = t.course().await;
    let batch = t
        .batch_with(
            &course,
            (&tutor.0, Some(tutor.1)),
            &room,
            next_weekday(day(7), 1),
            &[1, 3],
        )
        .await;
    let session = session_on(&t, &batch, 1, expiry() + TimeDelta::days(1), &tutor).await;

    t.state()
        .emit(DocumentExpiring {
            document_id: DocumentId(1),
            subject: Centre::employee_subject(tutor.1),
            kind: "police_clearance".to_owned(),
            expires_on: expiry(),
            days_left: 40,
            offset: Some(30),
            affected: Vec::new(),
        })
        .await
        .unwrap();

    assert!(
        !Session::find_or_404(t.db(), session.id)
            .await
            .unwrap()
            .is_flagged()
    );
    assert!(t.tasks.with_key(TASK_KEY).is_empty());
}

/// A manual block on T for three days flags T's sessions on those days only and
/// gives the coordinator one task; a block of another source is no trigger.
#[renox::test]
async fn edu_10_a_manual_block_flags_only_the_sessions_inside_it() {
    let s = scene().await;
    let coordinator =
        s.t.holder("Cora Coordinator", perm::COORDINATOR_ROLE, None)
            .await;
    let start = day(10);
    let mut sessions = Vec::new();
    for offset in 0..5 {
        sessions.push(
            session_on(
                &s.t,
                &s.batch,
                10 + offset,
                start + TimeDelta::days(offset - 1),
                &s.tutor,
            )
            .await,
        );
    }
    // The block runs from the start of day 10 to the end of day 12.
    let block = AvailabilityBlock::manual(
        s.tutor.0.id,
        at(start, 0, 0),
        at(start + TimeDelta::days(3), 0, 0),
        "sick",
    );

    wl_scheduling::conflict::add_block(s.t.state(), block, renox::db::now())
        .await
        .expect("the block is added");

    let flagged: Vec<bool> = {
        let mut found = Vec::new();
        for session in &sessions {
            found.push(s.session(session).await.is_flagged());
        }
        found
    };
    // Days 9 and 13 are outside the block; 10, 11 and 12 are inside.
    assert_eq!(flagged, [false, true, true, true, false]);
    let tasks = s.tasks();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee_user(), Some(coordinator.id));
    assert_eq!(
        s.session(&sessions[1]).await.flag_reason.as_deref(),
        Some("block")
    );

    // A roster block is wl-scheduling's own business.
    let mut roster = AvailabilityBlock::manual(
        s.tutor.0.id,
        at(start + TimeDelta::days(20), 0, 0),
        at(start + TimeDelta::days(21), 0, 0),
        "roster",
    );
    roster.source = "roster".to_owned();
    wl_scheduling::conflict::add_block(s.t.state(), roster, renox::db::now())
        .await
        .unwrap();
    assert_eq!(
        TutorAlert::of_employee(s.t.db(), s.tutor.1)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// Notice given: the sessions from the day after the last working day are
/// flagged and the coordinator has a task; withdrawing the notice takes the
/// flags off and completes the task.
#[renox::test]
async fn edu_10_a_notice_flags_the_sessions_after_the_last_working_day() {
    let s = scene().await;
    let last_day = expiry() + TimeDelta::days(1);

    s.t.state()
        .emit(EmployeeNoticeGiven {
            employee: EmployeeId(s.tutor.1),
            last_working_day: last_day,
        })
        .await
        .unwrap();

    // The session on the last working day is taught; the one two days later is not.
    assert!(!s.session(&s.first).await.is_flagged());
    let second = s.session(&s.second).await;
    assert!(second.is_flagged());
    assert_eq!(second.flag_reason.as_deref(), Some("notice"));
    assert_eq!(s.tasks().len(), 1);

    s.t.state()
        .emit(EmployeeNoticeWithdrawn {
            employee: EmployeeId(s.tutor.1),
        })
        .await
        .unwrap();

    assert!(!s.session(&s.second).await.is_flagged());
    assert!(!s.tasks()[0].is_open());
}

/// `LeaveApproved` is a trigger that stays off until tutor leave is enabled
/// (`HRM.LEV.01`, Later for 10); built, it flags only the sessions inside the
/// leave.
#[renox::test]
async fn edu_10_leave_is_built_but_inactive() {
    let s = scene().await;
    let leave = LeaveApproved {
        leave: Subject::new("hr.leave_request", 5),
        employee: EmployeeId(s.tutor.1),
        leave_type: "annual".to_owned(),
        from: expiry() + TimeDelta::days(3),
        to: expiry() + TimeDelta::days(10),
        unpaid: false,
    };

    s.t.state().emit(leave.clone()).await.unwrap();
    assert!(
        !s.session(&s.second).await.is_flagged(),
        "inactive until tutor leave is enabled"
    );
    assert!(s.tasks().is_empty());

    tutor_alerts::on_leave(s.t.state(), &leave, true)
        .await
        .unwrap();

    assert!(s.session(&s.second).await.is_flagged(), "inside the leave");
    assert!(!s.session(&s.first).await.is_flagged(), "before the leave");
    assert!(!s.session(&s.third).await.is_flagged(), "after the leave");
    assert_eq!(s.tasks().len(), 1);
}

/// The users a request is for.
fn users_of(request: &wl_kernel::notify::NotifyRequest) -> Vec<i64> {
    request
        .recipients
        .iter()
        .filter_map(|recipient| match recipient {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect()
}

/// A run that cannot open the coordinator's task is tried again by the job and,
/// after the fifth attempt, alerts the owner and the company admin once.
#[renox::test]
async fn edu_10_a_failed_run_alerts_the_owner_and_the_admin() {
    let s = scene().await;
    let owner = s.t.holder("Omar Owner", perm::OWNER_ROLE, None).await;
    let admin = s.t.holder("Aisha Admin", COMPANY_ADMIN_ROLE, None).await;
    s.t.tasks.fail_for(Some(TASK_KEY));
    s.t.messages
        .always_works(wl_kernel::compliance::service::NOTIFICATION_TYPE);

    s.t.state().emit(s.expiring()).await.unwrap();
    assert!(s.tasks().is_empty(), "the task could not be opened");
    assert!(
        ComplianceAlert::query()
            .where_(ComplianceAlert::RULE.eq(tutor_alerts::RULE))
            .get(s.t.db())
            .await
            .unwrap()
            .is_empty(),
        "not before the last attempt"
    );

    for attempt in 1..=5 {
        s.t.app.travel(Duration::from_secs(2 * 60 * 60));
        s.t.app
            .at_travelled_time(tutor_alerts::retry(s.t.state()))
            .await
            .unwrap_or_else(|error| panic!("attempt {attempt} ended in an error: {error}"));
    }

    let alerts = ComplianceAlert::query()
        .where_(ComplianceAlert::RULE.eq(tutor_alerts::RULE))
        .get(s.t.db())
        .await
        .unwrap();
    assert_eq!(alerts.len(), 1, "the owner is alerted once");
    assert_eq!(alerts[0].owner_role, perm::OWNER_ROLE);
    let told =
        s.t.messages
            .of_type(wl_kernel::compliance::service::NOTIFICATION_TYPE);
    assert_eq!(told.len(), 1);
    let mut recipients = users_of(&told[0]);
    recipients.sort_unstable();
    let mut wanted = vec![owner.id, admin.id];
    wanted.sort_unstable();
    assert_eq!(recipients, wanted, "the owner and the company admin");
}

/// The flag shows on the session page and the batch page for staff, in words.
#[renox::test]
async fn edu_10_the_flag_shows_on_the_session_and_batch_pages() {
    let s = scene().await;
    let amal = s.t.coordinator_user().await;
    s.t.app.acting_as(&amal);
    s.t.app
        .get(&format!("/edtech/sessions/{}", s.first.id))
        .await
        .assert_ok()
        .assert_dont_see("Tutor not available or not eligible");

    s.t.state().emit(s.expiring()).await.unwrap();

    s.t.app
        .get(&format!("/edtech/sessions/{}", s.first.id))
        .await
        .assert_ok()
        .assert_see("Tutor not available or not eligible")
        .assert_see("police clearance");
    s.t.app
        .get(&format!("/edtech/batches/{}", s.batch.id))
        .await
        .assert_ok()
        .assert_see("Tutor not available or not eligible");
}
