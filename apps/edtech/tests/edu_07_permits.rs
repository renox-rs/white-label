//! BL-EDU-07 (#442): a centre must not open a batch or issue a certificate under
//! an expired centre permit or course approval (`EDU.RUL.13`). Both transitions
//! are blocked with no override; sixty days ahead and on the expiry the
//! coordinator, the centre manager and the owner are told in one message which
//! batches are affected, the responsible role gets a renewal task due on the
//! expiry date, running batches are not stopped, and a failed run alerts the
//! owner and the admin.
//!
//! The documents module is a stand-in guard that answers from a table
//! (`support::documents`); the app's own `BlockSubjects` providers, the kernel's
//! compliance alert and the rule runs are the real ones.

mod support;

use std::time::Duration;

use edtech::batches::{self, NewBatch};
use edtech::certificate::{Certificate, CertificateStatus};
use edtech::certificates;
use edtech::courses::{self, NewCourse};
use edtech::enrolments;
use edtech::notices::NOTICE;
use edtech::permissions as perm;
use edtech::permits;
use edtech::{Batch, BatchStatus, Course};
use renox::auth::permissions::Scope;
use renox::chrono::TimeDelta;
use renox::prelude::*;
use support::{Centre, Messages, boot, day, next_weekday};
use wl_kernel::compliance::ComplianceAlert;
use wl_kernel::documents::{DocumentExpired, DocumentExpiring, DocumentId};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::{COMPANY_ADMIN_ROLE, LegalEntity, Subject};

/// The course the tests teach, which the regulator must approve.
async fn approved_course(t: &Centre) -> Course {
    let amal = t.coordinator().await;
    let mut new = NewCourse::new("IELTS-B2", "IELTS preparation", "English language", 40).level("B2");
    new.approval_required = true;
    new.approval_no = Some("KHDA-2026-77".to_owned());
    courses::create(t.state(), &amal, new).await.unwrap()
}

/// The legal entity that issues for the centre: what the centre permit is
/// attached to.
async fn permit_of_centre(t: &Centre) -> Subject {
    let entity = LegalEntity::default_entity(t.db()).await.unwrap();
    Subject::new("kernel.legal_entity", entity.id)
}

/// The guard knows the kinds' `blocks`, the centre permit is valid for a long
/// time and the course's approval ends on `approval_until`.
async fn documents_until(t: &Centre, course: &Course, approval_until: renox::chrono::NaiveDate) {
    t.documents.edtech_blocks();
    t.documents
        .set(&permit_of_centre(t).await, "centre_permit", Some(day(400)));
    t.documents
        .set(&course.subject(), "course_approval", Some(approval_until));
}

/// A batch of the course planned from next Monday on Monday and Wednesday.
async fn planned(t: &Centre) -> Batch {
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    t.batch_of(&tutor, &room, next_weekday(day(7), 1), &[1, 3])
        .await
}

fn refusal(error: &Error) -> String {
    match error {
        Error::Status(status, message) => {
            assert_eq!(*status, StatusCode::UNPROCESSABLE_ENTITY, "{message}");
            message.clone()
        }
        other => panic!("not a refusal: {other:?}"),
    }
}

/// Given course C's approval expiring before the last session of planned batch
/// B, `open` is refused naming the approval and the day it expired; B stays
/// planned.
#[renox::test]
async fn edu_07_open_is_refused_when_the_approval_ends_before_the_last_session() {
    let t = boot().await;
    let course = approved_course(&t).await;
    documents_until(&t, &course, day(20)).await;
    let batch = planned(&t).await;
    let amal = t.coordinator().await;

    let error = batches::open(t.state(), &amal, batch.id).await.unwrap_err();

    let message = refusal(&error);
    assert!(message.contains("Course approval"), "{message}");
    assert!(message.contains(&day(20).to_string()), "{message}");
    assert!(message.contains("override: not allowed"), "no override: {message}");
    assert_eq!(
        Batch::find_or_404(t.db(), batch.id).await.unwrap().status,
        BatchStatus::Planned
    );
    // The page shows the refusal too.
    let manager_user = t.coordinator_user().await;
    t.app.acting_as(&manager_user);
    let page = t
        .app
        .htmx()
        .post(&format!("/edtech/batches/{}/open", batch.id), &[("x", "y")])
        .await;
    page.assert_status(422);
    assert!(page.text().contains("Course approval"));
}

/// The same batch ending before the approval does (also on the very day, which
/// is the last day it is valid), `open` succeeds.
#[renox::test]
async fn edu_07_open_succeeds_when_the_approval_covers_the_last_session() {
    let t = boot().await;
    let course = approved_course(&t).await;
    let batch = planned(&t).await;
    let last = batches::plan_for(t.state(), &batch)
        .unwrap()
        .last()
        .expect("sessions")
        .date;
    documents_until(&t, &course, last).await;
    let amal = t.coordinator().await;

    let opened = batches::open(t.state(), &amal, batch.id).await.unwrap();

    assert_eq!(opened.status, BatchStatus::Open);
}

/// A course that needs no approval has no approval check, but the centre permit
/// is always needed.
#[renox::test]
async fn edu_07_a_course_without_an_approval_needs_only_the_centre_permit() {
    let t = boot().await;
    t.documents.edtech_blocks();
    let batch = planned(&t).await;
    let amal = t.coordinator().await;

    let missing = batches::open(t.state(), &amal, batch.id).await.unwrap_err();
    let message = refusal(&missing);
    assert!(message.contains("Centre permit missing"), "{message}");
    assert!(!message.contains("Course approval"), "{message}");

    t.documents
        .set(&permit_of_centre(&t).await, "centre_permit", Some(day(400)));
    let opened = batches::open(t.state(), &amal, batch.id).await.unwrap();
    assert_eq!(opened.status, BatchStatus::Open);
}

/// A draft certificate of a student who completed the course in `batch`.
async fn draft_in(t: &Centre, batch: &Batch) -> Certificate {
    let student = t.person(1, "Sara Ali");
    let desk = t.desk().await;
    let e = t.enrol(student, batch).await;
    enrolments::activate(t.state(), Some(&desk), e.id).await.unwrap();
    enrolments::complete(t.state(), e.id).await.unwrap();
    Certificate::of_enrolment(t.db(), e.id)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("a draft certificate")
}

/// Given the centre permit expired, issuing a draft is refused even though the
/// certificate states no approval; a renewal makes it pass.
#[renox::test]
async fn edu_07_issue_is_refused_on_an_expired_permit_even_if_no_approval_is_printed() {
    let t = boot().await;
    t.documents.edtech_blocks();
    let permit = permit_of_centre(&t).await;
    t.documents.set(&permit, "centre_permit", Some(day(400)));
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let batch = t.open_batch(&tutor, &room, next_weekday(day(7), 1)).await;
    let draft = draft_in(&t, &batch).await;
    assert!(!draft.states_approval);
    t.documents.set(&permit, "centre_permit", Some(day(-5)));
    let registrar = t.registrar().await;

    let error = certificates::issue(t.state(), &registrar, draft.id)
        .await
        .unwrap_err();

    let message = refusal(&error);
    assert!(message.contains("Centre permit expired on"), "{message}");
    assert!(message.contains(&day(-5).to_string()), "{message}");
    let still = Certificate::find_or_404(t.db(), draft.id).await.unwrap();
    assert_eq!(still.status, CertificateStatus::Draft);
    assert!(still.number.is_none(), "no number is spent");

    // The renewal is accepted: the guard answers yes again.
    t.documents.set(&permit, "centre_permit", Some(day(400)));
    let issued = certificates::issue(t.state(), &registrar, draft.id)
        .await
        .unwrap();
    assert_eq!(issued.status, CertificateStatus::Issued);
    assert_eq!(issued.number.as_deref(), Some("CRT-0001"));
}

/// The course approval blocks `issue` only for a certificate that states it:
/// an expired approval stops one that prints it and not one that does not.
#[renox::test]
async fn edu_07_the_approval_blocks_issue_only_when_the_certificate_states_it() {
    let t = boot().await;
    let course = approved_course(&t).await;
    documents_until(&t, &course, day(400)).await;
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let batch = t.open_batch(&tutor, &room, next_weekday(day(7), 1)).await;
    let stated = draft_in(&t, &batch).await;
    assert!(stated.states_approval);
    t.documents.set(&course.subject(), "course_approval", Some(day(-1)));
    let registrar = t.registrar().await;

    let error = certificates::issue(t.state(), &registrar, stated.id)
        .await
        .unwrap_err();
    let message = refusal(&error);
    assert!(message.contains("Course approval expired on"), "{message}");
    assert!(!message.contains("Centre permit"), "{message}");

    // The same certificate not printing the approval is not stopped by it.
    Certificate::query()
        .where_(Certificate::ID.eq(stated.id))
        .update(t.db(), &[("states_approval", &false)])
        .await
        .unwrap();
    let issued = certificates::issue(t.state(), &registrar, stated.id)
        .await
        .unwrap();
    assert_eq!(issued.status, CertificateStatus::Issued);
}

/// A `DocumentExpiring` as wl-documents emits it, `left` days before `expires_on`.
fn expiring(subject: &Subject, kind: &str, expires_on: renox::chrono::NaiveDate, left: i64) -> DocumentExpiring {
    DocumentExpiring {
        document_id: DocumentId(1),
        subject: subject.clone(),
        kind: kind.to_owned(),
        expires_on,
        days_left: left,
        offset: Some(u32::try_from(left).unwrap_or(0)),
        affected: Vec::new(),
    }
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

/// Sixty days before the approval of course C expires, the coordinator, the
/// centre manager and the owner get one message listing C's open and running
/// batches, the academic coordinator has a renewal task due on the expiry date,
/// nothing is stopped, and later reminders say nothing again.
#[renox::test]
async fn edu_07_sixty_days_ahead_one_message_and_the_renewal_task() {
    let t = boot().await;
    let course = approved_course(&t).await;
    documents_until(&t, &course, day(60)).await;
    let coordinator = t.holder("Cora Coordinator", perm::COORDINATOR_ROLE, None).await;
    let manager = t.holder("Mona Manager", perm::CENTRE_MANAGER_ROLE, None).await;
    let owner = t.holder("Omar Owner", perm::OWNER_ROLE, None).await;
    let outsider = t.holder("Tariq Tutor", perm::TUTOR_ROLE, None).await;
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let upcoming = t.open_batch(&tutor, &room, next_weekday(day(7), 1)).await;
    let tutor2 = t.tutor("Hana").await;
    let room2 = t.room("Room 2", 14).await;
    let started = t.open_batch(&tutor2, &room2, next_weekday(day(-14), 1)).await;
    batches::start_due(t.state(), day(0)).await.unwrap();
    assert_eq!(
        Batch::find_or_404(t.db(), started.id).await.unwrap().status,
        BatchStatus::Running
    );
    let subject = course.subject();

    // Ninety days ahead is before the look-ahead: nothing yet.
    t.state()
        .emit(expiring(&subject, "course_approval", day(60), 90))
        .await
        .unwrap();
    assert!(t.messages.of_type(NOTICE).is_empty());
    assert!(t.tasks.all().is_empty());

    t.state()
        .emit(expiring(&subject, "course_approval", day(60), 60))
        .await
        .unwrap();

    let told = t.messages.of_type(NOTICE);
    assert_eq!(told.len(), 1, "one message");
    for user in [&coordinator, &manager, &owner] {
        assert!(Messages::told_user(&told[0], user.id), "{}", user.name);
    }
    assert!(!Messages::told_user(&told[0], outsider.id));
    let text = Messages::text(&told[0]);
    assert!(text.contains(&upcoming.number()), "{text}");
    assert!(text.contains(&started.number()), "{text}");
    assert!(text.contains("open") && text.contains("running"), "{text}");
    assert!(text.contains("not stopped"), "{text}");
    assert!(text.contains(&day(60).to_string()), "{text}");
    let tasks = t.tasks.with_key("edtech.permit_renewal");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee_user(), Some(coordinator.id), "the academic coordinator");
    let due = wl_kernel::calendar::BusinessCalendar::default()
        .from_local(day(60).and_hms_opt(9, 0, 0).unwrap());
    assert_eq!(tasks[0].ticket.due_at, Some(due), "due on the expiry date");
    assert_eq!(
        Batch::find_or_404(t.db(), started.id).await.unwrap().status,
        BatchStatus::Running,
        "a running batch is not stopped"
    );

    // The reminder at 30 days says nothing again.
    t.state()
        .emit(expiring(&subject, "course_approval", day(60), 30))
        .await
        .unwrap();
    assert_eq!(t.messages.of_type(NOTICE).len(), 1);
    assert_eq!(t.tasks.with_key("edtech.permit_renewal").len(), 1);

    // On the expiry the same people are told again; the open task is not doubled.
    t.documents.set(&subject, "course_approval", Some(day(-1)));
    t.state()
        .emit(DocumentExpired {
            document: DocumentId(1),
            subject: subject.clone(),
            kind: "course_approval".to_owned(),
            expires_on: day(60),
            blocks: Vec::new(),
            blocked: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(t.messages.of_type(NOTICE).len(), 2);
    assert!(Messages::text(&t.messages.of_type(NOTICE)[1]).contains("expired"));
    assert_eq!(
        t.tasks.with_key("edtech.permit_renewal").len(),
        1,
        "the open renewal task is not doubled"
    );
}

/// For a campus permit expiring, the campus's centre manager has the renewal
/// task, not another campus's.
#[renox::test]
async fn edu_07_the_campus_centre_manager_renews_the_campus_permit() {
    let t = boot().await;
    t.documents.edtech_blocks();
    let mut entity = LegalEntity {
        code: "GSN".to_owned(),
        name_en: "Gulf Skills North LLC".to_owned(),
        status: "active".to_owned(),
        ..LegalEntity::default()
    };
    entity.save(t.db()).await.unwrap();
    entity.link_scope(t.db(), "test.campus", 3).await.unwrap();
    let permit = Subject::new("kernel.legal_entity", entity.id);
    t.documents.set(&permit, "centre_permit", Some(day(80)));
    let north = t
        .holder("Mona North", perm::CENTRE_MANAGER_ROLE, Some(&Scope::new("test.campus", 3)))
        .await;
    let south = t
        .holder("Mira South", perm::CENTRE_MANAGER_ROLE, Some(&Scope::new("test.campus", 4)))
        .await;
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let course = t.course().await;
    let amal = t.coordinator().await;
    let first = next_weekday(day(7), 1);
    let batch = batches::create(
        t.state(),
        &amal,
        NewBatch::new(
            course.id,
            12,
            (first, first + TimeDelta::days(27)),
            &[1, 3],
            "18:00",
            90,
        )
        .tutor(tutor.id, None)
        .room(room.id, Some(14))
        .campus(Subject::new("test.campus", 3)),
    )
    .await
    .unwrap();
    batches::open(t.state(), &amal, batch.id)
        .await
        .expect("the campus permit covers the batch");

    t.state()
        .emit(expiring(&permit, "centre_permit", day(80), 58))
        .await
        .unwrap();

    let tasks = t.tasks.with_key("edtech.permit_renewal");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee_user(), Some(north.id));
    let told = t.messages.of_type(NOTICE);
    assert_eq!(told.len(), 1);
    assert!(Messages::told_user(&told[0], north.id));
    assert!(!Messages::told_user(&told[0], south.id));
    assert!(Messages::text(&told[0]).contains(&batch.number()));
}

/// A run whose message fails is tried again by the daily run and, after the
/// fifth attempt, alerts the owner and the admin once.
#[renox::test]
async fn edu_07_a_failed_run_alerts_the_owner_and_the_admin() {
    let t = boot().await;
    let course = approved_course(&t).await;
    documents_until(&t, &course, day(60)).await;
    let owner = t.holder("Omar Owner", perm::OWNER_ROLE, None).await;
    let admin = t.holder("Aisha Admin", COMPANY_ADMIN_ROLE, None).await;
    t.messages.fail(true);
    t.messages.always_works(wl_kernel::compliance::service::NOTIFICATION_TYPE);

    t.state()
        .emit(expiring(&course.subject(), "course_approval", day(60), 60))
        .await
        .unwrap();
    assert!(t.messages.of_type(NOTICE).is_empty());
    assert!(
        ComplianceAlert::query()
            .where_(ComplianceAlert::RULE.eq(permits::RULE))
            .get(t.db())
            .await
            .unwrap()
            .is_empty(),
        "not before the last attempt"
    );

    for attempt in 1..=5 {
        t.app.travel(Duration::from_secs(2 * 60 * 60));
        t.app
            .at_travelled_time(permits::retry(t.state()))
            .await
            .unwrap_or_else(|error| panic!("attempt {attempt} ended in an error: {error}"));
    }

    let alerts = ComplianceAlert::query()
        .where_(ComplianceAlert::RULE.eq(permits::RULE))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(alerts.len(), 1, "the owner is alerted once");
    assert_eq!(alerts[0].owner_role, perm::OWNER_ROLE);
    let told = t
        .messages
        .of_type(wl_kernel::compliance::service::NOTIFICATION_TYPE);
    assert_eq!(told.len(), 1);
    let mut recipients = users_of(&told[0]);
    recipients.sort_unstable();
    let mut wanted = vec![owner.id, admin.id];
    wanted.sort_unstable();
    assert_eq!(recipients, wanted, "the owner and the company admin");

    // The notifier works again: a run that was given up on is not tried again.
    t.messages.fail(false);
    t.app
        .at_travelled_time(permits::retry(t.state()))
        .await
        .unwrap();
    assert!(t.messages.of_type(NOTICE).is_empty());
}
