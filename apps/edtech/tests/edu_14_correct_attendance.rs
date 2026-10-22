//! BL-EDU-14 (#449): the attendance of a held session can be corrected. The
//! tutor corrects within 24 hours of the scheduled end, later only the
//! coordinator, with a reason (`EDU.ACL.04`, `EDU.ACL.16`). The credits are priced
//! again by the stored absence policy and only the difference moves; the
//! guardian is told when an absence that had triggered an alert is corrected; the
//! completion is judged again (a draft certificate is discarded, an issued one
//! gets a task to reissue or revoke, never a change).
//!
//! The tutor's timesheet is not touched by a mark (a tutor is paid per session
//! taught); the adjustment of a locked timesheet is deferred, see
//! `docs/backlog` notes in the commit.

mod support;

use std::time::Duration;

use edtech::attendance::{Attendance, Mark};
use edtech::certificate::{Certificate, CertificateStatus};
use edtech::certificates;
use edtech::completion;
use edtech::corrections;
use edtech::enrolments;
use edtech::make_up;
use edtech::notices::FAMILY_NOTICE;
use edtech::roster;
use edtech::sessions;
use edtech::{Batch, Enrolment, EnrolmentStatus, Session, SessionStatus};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use support::{Centre, Messages, boot, day, next_weekday};
use wl_kernel::PartyId;
use wl_kernel::documents::dubai_date;

const ITEM: &str = "pack10";

/// A planned batch whose sessions are fixtures.
async fn batch(t: &Centre) -> Batch {
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    t.batch_of(&tutor, &room, next_weekday(day(7), 1), &[1, 3]).await
}

/// A `scheduled` session of `batch` that ended `hours` hours ago, after an hour.
async fn ended_ago(t: &Centre, batch: &Batch, number: i64, hours: i64) -> Session {
    let ends_at = renox::db::now() - TimeDelta::hours(hours);
    let starts_at = ends_at - TimeDelta::hours(1);
    let mut session = Session {
        batch_id: batch.id,
        number,
        session_date: dubai_date(starts_at),
        starts_at,
        ends_at,
        tutor_resource_id: batch.tutor_resource_id,
        room_resource_id: batch.room_resource_id,
        ..Session::default()
    };
    session.save(t.db()).await.unwrap();
    session
}

/// The row of `enrolment` in `session`.
async fn row(t: &Centre, session: &Session, enrolment: &Enrolment) -> Attendance {
    Attendance::find_of(t.db(), session.id, enrolment.id)
        .await
        .unwrap()
        .expect("a row")
}

/// The refusal message.
fn refusal(error: &Error) -> String {
    match error {
        Error::Status(status, message) => {
            assert_eq!(*status, StatusCode::UNPROCESSABLE_ENTITY, "{message}");
            message.clone()
        }
        other => panic!("not a refusal: {other:?}"),
    }
}

/// Session S ended 16 hours ago with P marked absent (unexcused, credit
/// consumed) after an earlier absence: the guardian got the alert of two in a row.
struct Scene {
    t: Centre,
    batch: Batch,
    earlier: Session,
    session: Session,
    p: Enrolment,
    mother: i64,
}

async fn scene() -> Scene {
    let t = boot().await;
    let batch = batch(&t).await;
    let child = t.person(1, "Omar Child");
    let mother = t.person(2, "Mother");
    t.people.guardian(child, mother);
    let p = t.package_in(child, ITEM, 10, EnrolmentStatus::Active).await;
    Enrolment::query()
        .where_(Enrolment::ID.eq(p.id))
        .update(t.db(), &[("guardian_party_id", &Some(mother))])
        .await
        .unwrap();
    let earlier = ended_ago(&t, &batch, 1, 40).await;
    let session = ended_ago(&t, &batch, 2, 16).await;
    let desk = t.desk().await;
    let tariq = t.teacher().await;
    for held in [&earlier, &session] {
        roster::add_student(t.state(), &desk, held.id, p.id).await.unwrap();
        sessions::submit_attendance_marked(t.state(), &tariq, held.id, &[(p.id, Mark::Absent)], false)
            .await
            .unwrap();
    }
    Scene {
        t,
        batch,
        earlier,
        session,
        p,
        mother,
    }
}

/// Given S held with P absent (credit consumed) and the guardian alerted, tutor T
/// marks P present with the reason "marked wrong row": no credit change, P's
/// absence streak resets, and the guardian is told.
#[renox::test]
async fn edu_14_the_tutor_corrects_within_the_window_without_a_credit_change() {
    let s = scene().await;
    let tariq = s.t.teacher().await;
    assert_eq!(edtech::absences::absence_streak(s.t.db(), s.p.id).await.unwrap(), 2);
    assert_eq!(s.t.packages.left(PartyId(s.p.student_party_id), ITEM), 8);
    assert_eq!(s.t.messages.of_type(FAMILY_NOTICE).len(), 1, "the alert of two in a row");
    let asked = s.t.packages.asked().len();

    corrections::correct_attendance(
        s.t.state(),
        &tariq,
        s.session.id,
        &[(s.p.id, Mark::Present)],
        "marked wrong row",
    )
    .await
    .unwrap();

    let corrected = row(&s.t, &s.session, &s.p).await;
    assert_eq!(corrected.mark, Mark::Present);
    assert!(corrected.corrected_at.is_some());
    assert_eq!(s.t.packages.left(PartyId(s.p.student_party_id), ITEM), 8, "no credit change");
    assert_eq!(s.t.packages.asked().len(), asked, "the ledger is not asked again");
    assert!(corrected.consumed);
    assert_eq!(edtech::absences::absence_streak(s.t.db(), s.p.id).await.unwrap(), 0);
    let told = s.t.messages.of_type(FAMILY_NOTICE);
    assert_eq!(told.len(), 2, "the guardian is told of the correction");
    assert!(Messages::told_party(&told[1], s.mother));
    assert!(Messages::text(&told[1]).contains("corrected"));
    // The held session stays held, with the history of the correction.
    let held = Session::find_or_404(s.t.db(), s.session.id).await.unwrap();
    assert_eq!(held.status, SessionStatus::Held);
    let history = renox::state_machine::history(s.t.db(), "edtech_sessions", held.id)
        .await
        .unwrap();
    let last = history.last().unwrap();
    assert_eq!(last.transition, "correct_attendance");
    assert_eq!(last.reason.as_deref(), Some("marked wrong row"));
    let _ = (&s.batch, &s.earlier);
}

/// 24 hours after the end T cannot correct; the coordinator can, with a reason;
/// without a reason it is refused for both; a user who neither teaches nor
/// coordinates may not.
#[renox::test]
async fn edu_14_after_the_window_only_the_coordinator_corrects_and_with_a_reason() {
    let s = scene().await;
    let tariq = s.t.teacher().await;
    let amal = s.t.coordinator().await;
    let cara = s.t.counsellor().await;
    assert!(
        corrections::may_correct(&tariq, &Session::find_or_404(s.t.db(), s.session.id).await.unwrap())
    );

    // 8.5 hours later: 24.5 hours after the end.
    s.t.app.travel(Duration::from_secs(8 * 3600 + 30 * 60));
    let late = s
        .t
        .app
        .at_travelled_time(corrections::correct_attendance(
            s.t.state(),
            &tariq,
            s.session.id,
            &[(s.p.id, Mark::Present)],
            "marked wrong row",
        ))
        .await
        .unwrap_err();
    assert!(refusal(&late).contains("24 hours"), "{late}");
    assert!(refusal(&late).contains("coordinator"), "{late}");

    let bare = s
        .t
        .app
        .at_travelled_time(corrections::correct_attendance(
            s.t.state(),
            &amal,
            s.session.id,
            &[(s.p.id, Mark::Present)],
            "  ",
        ))
        .await
        .unwrap_err();
    assert!(refusal(&bare).contains("reason"), "{bare}");
    assert_eq!(row(&s.t, &s.session, &s.p).await.mark, Mark::Absent, "nothing changed");

    let forbidden = s
        .t
        .app
        .at_travelled_time(corrections::correct_attendance(
            s.t.state(),
            &cara,
            s.session.id,
            &[(s.p.id, Mark::Present)],
            "marked wrong row",
        ))
        .await;
    assert!(matches!(forbidden, Err(Error::Forbidden)));

    s.t.app
        .at_travelled_time(corrections::correct_attendance(
            s.t.state(),
            &amal,
            s.session.id,
            &[(s.p.id, Mark::Present)],
            "marked wrong row",
        ))
        .await
        .expect("the coordinator corrects after the window");
    assert_eq!(row(&s.t, &s.session, &s.p).await.mark, Mark::Present);
    let history = renox::state_machine::history(s.t.db(), "edtech_sessions", s.session.id)
        .await
        .unwrap();
    assert_eq!(
        history.last().map(|h| h.transition.as_str()),
        Some("correct_attendance_coordinator")
    );
}

/// Marking P as excused gives the credit back (balance +1) and earns a make-up
/// right; marking P present again takes the credit and the right is gone.
#[renox::test]
async fn edu_14_excusing_gives_the_credit_back_and_attending_takes_it_again() {
    let s = scene().await;
    let amal = s.t.coordinator().await;
    let party = PartyId(s.p.student_party_id);
    assert_eq!(s.t.packages.left(party, ITEM), 8);

    corrections::correct_attendance(s.t.state(), &amal, s.session.id, &[(s.p.id, Mark::Excused)], "Hospital note")
        .await
        .unwrap();

    assert_eq!(s.t.packages.left(party, ITEM), 9, "the credit is given back");
    let excused = row(&s.t, &s.session, &s.p).await;
    assert!(!excused.consumed && !excused.credit_due);
    let fresh = Enrolment::find_or_404(s.t.db(), s.p.id).await.unwrap();
    assert_eq!(make_up::open_rights(s.t.db(), &fresh).await.unwrap().len(), 1);

    corrections::correct_attendance(s.t.state(), &amal, s.session.id, &[(s.p.id, Mark::Present)], "He came after all")
        .await
        .unwrap();

    assert_eq!(s.t.packages.left(party, ITEM), 8, "one credit is used again");
    assert!(row(&s.t, &s.session, &s.p).await.consumed);
    assert!(
        make_up::open_rights(s.t.db(), &fresh).await.unwrap().is_empty(),
        "no excused absence, no right"
    );
}

/// With "unexcused absence consumes" = no, an absence corrected to attendance
/// consumes one and attendance corrected to an absence gives it back.
#[renox::test]
async fn edu_14_a_policy_that_does_not_charge_absences_moves_the_credit_with_the_mark() {
    let t = boot().await;
    let batch = batch(&t).await;
    let student = t.person(1, "Sara Ali");
    let p = t.package_in(student, ITEM, 10, EnrolmentStatus::Active).await;
    t.set_policy(&p, false, 24).await;
    let session = ended_ago(&t, &batch, 1, 5).await;
    let desk = t.desk().await;
    let tariq = t.teacher().await;
    roster::add_student(t.state(), &desk, session.id, p.id).await.unwrap();
    sessions::submit_attendance_marked(t.state(), &tariq, session.id, &[(p.id, Mark::Absent)], false)
        .await
        .unwrap();
    assert_eq!(t.packages.left(PartyId(student), ITEM), 10);

    corrections::correct_attendance(t.state(), &tariq, session.id, &[(p.id, Mark::Present)], "came late")
        .await
        .unwrap();
    assert_eq!(t.packages.left(PartyId(student), ITEM), 9);
    corrections::correct_attendance(t.state(), &tariq, session.id, &[(p.id, Mark::Absent)], "left at once")
        .await
        .unwrap();
    assert_eq!(t.packages.left(PartyId(student), ITEM), 10);
}

/// With no credit left a correction that needs one is saved and the counsellor
/// gets a task to sell credits.
#[renox::test]
async fn edu_14_with_no_credit_left_the_correction_is_saved_and_the_counsellor_has_a_task() {
    let t = boot().await;
    let batch = batch(&t).await;
    let student = t.person(1, "Sara Ali");
    let p = t.package_in(student, ITEM, 0, EnrolmentStatus::Active).await;
    let session = ended_ago(&t, &batch, 1, 5).await;
    let desk = t.desk().await;
    let tariq = t.teacher().await;
    roster::add_student(t.state(), &desk, session.id, p.id).await.unwrap();
    roster::record_notice(
        t.state(),
        &desk,
        session.id,
        p.id,
        session.starts_at - TimeDelta::hours(30),
    )
    .await
    .unwrap();
    sessions::submit_attendance_marked(t.state(), &tariq, session.id, &[], false)
        .await
        .unwrap();
    assert!(t.tasks.with_key("edtech.credits").is_empty(), "an excused absence needs none");

    corrections::correct_attendance(t.state(), &tariq, session.id, &[(p.id, Mark::Present)], "he came")
        .await
        .expect("the correction is saved");

    let corrected = row(&t, &session, &p).await;
    assert_eq!(corrected.mark, Mark::Present);
    assert!(corrected.credit_due && !corrected.consumed);
    let tasks = t.tasks.with_key("edtech.credits");
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].ticket.title.contains("Sara Ali"));
}

/// Four sessions attended: the certificate is issued. A correction that makes the
/// attendance 75% gives the coordinator a task to reissue or revoke it; the
/// certificate and the completed enrolment are unchanged.
#[renox::test]
async fn edu_14_an_issued_certificate_whose_criteria_fail_gets_a_task_and_no_change() {
    let t = boot().await;
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let open = t.open_batch(&tutor, &room, next_weekday(day(7), 1)).await;
    let student = t.person(1, "Ahmad Yusuf");
    let desk = t.desk().await;
    let e = t.enrol(student, &open).await;
    enrolments::activate(t.state(), Some(&desk), e.id).await.unwrap();
    let tariq = t.teacher().await;
    let amal = t.coordinator().await;
    let coordinator = t.holder("Cora Coordinator", edtech::permissions::COORDINATOR_ROLE, None).await;
    let mut held = Vec::new();
    for number in 0..4 {
        let session = ended_ago(&t, &open, 100 + number, 100 - number * 10).await;
        sessions::submit_attendance(t.state(), &tariq, session.id).await.unwrap();
        held.push(session);
    }
    assert_eq!(completion::attendance_percent(t.db(), e.id).await.unwrap(), Some(100));
    enrolments::complete(t.state(), e.id).await.unwrap();
    let draft = Certificate::of_enrolment(t.db(), e.id)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("a draft certificate");
    let registrar = t.registrar().await;
    let issued = certificates::issue(t.state(), &registrar, draft.id).await.unwrap();
    assert_eq!(issued.status, CertificateStatus::Issued);

    corrections::correct_attendance(
        t.state(),
        &amal,
        held[0].id,
        &[(e.id, Mark::Absent)],
        "marked present by mistake",
    )
    .await
    .unwrap();

    assert_eq!(completion::attendance_percent(t.db(), e.id).await.unwrap(), Some(75));
    let tasks = t.tasks.with_key("edtech.certificate_review");
    assert_eq!(tasks.len(), 1, "one task to reissue or revoke");
    assert_eq!(tasks[0].assignee_user(), Some(coordinator.id));
    assert!(tasks[0].ticket.title.contains(&issued.label()), "{}", tasks[0].ticket.title);
    assert!(tasks[0].ticket.description.contains("75%"));
    let same = Certificate::find_or_404(t.db(), issued.id).await.unwrap();
    assert_eq!(same.status, CertificateStatus::Issued, "never changed by itself");
    assert_eq!(same.number, issued.number);
    assert_eq!(
        Enrolment::find_or_404(t.db(), e.id).await.unwrap().status,
        EnrolmentStatus::Completed,
        "a completed enrolment stays completed"
    );
}

/// A draft certificate whose criteria now fail is discarded; when the criteria are
/// met again a new draft is made.
#[renox::test]
async fn edu_14_a_draft_certificate_is_discarded_and_made_again() {
    let t = boot().await;
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let open = t.open_batch(&tutor, &room, next_weekday(day(7), 1)).await;
    let student = t.person(1, "Ahmad Yusuf");
    let desk = t.desk().await;
    let e = t.enrol(student, &open).await;
    enrolments::activate(t.state(), Some(&desk), e.id).await.unwrap();
    let tariq = t.teacher().await;
    let amal = t.coordinator().await;
    let mut held = Vec::new();
    for number in 0..4 {
        let session = ended_ago(&t, &open, 100 + number, 100 - number * 10).await;
        sessions::submit_attendance(t.state(), &tariq, session.id).await.unwrap();
        held.push(session);
    }
    enrolments::complete(t.state(), e.id).await.unwrap();
    assert_eq!(Certificate::of_enrolment(t.db(), e.id).await.unwrap().len(), 1);

    corrections::correct_attendance(t.state(), &amal, held[0].id, &[(e.id, Mark::Absent)], "wrong row")
        .await
        .unwrap();
    assert!(
        Certificate::of_enrolment(t.db(), e.id).await.unwrap().is_empty(),
        "the draft whose criteria fail is discarded"
    );
    assert!(t.tasks.with_key("edtech.certificate_review").is_empty());

    corrections::correct_attendance(t.state(), &amal, held[0].id, &[(e.id, Mark::Late)], "he did come")
        .await
        .unwrap();
    let again = Certificate::of_enrolment(t.db(), e.id).await.unwrap();
    assert_eq!(again.len(), 1, "a new draft is made");
    assert_eq!(again[0].status, CertificateStatus::Draft);
}

/// Only a held session is corrected, a correction changes something, and the
/// session page offers it to those who may.
#[renox::test]
async fn edu_14_the_page_corrects_and_offers_it_only_to_those_who_may() {
    let s = scene().await;
    let tariq_user = s.t.teacher_user().await;
    s.t.app.acting_as(&tariq_user);
    s.t.app
        .get(&format!("/edtech/sessions/{}", s.session.id))
        .await
        .assert_ok()
        .assert_see("Correct attendance");

    // Nothing to correct, and a session still scheduled is not held.
    let amal = s.t.coordinator().await;
    let nothing = corrections::correct_attendance(
        s.t.state(),
        &amal,
        s.session.id,
        &[(s.p.id, Mark::Absent)],
        "no change",
    )
    .await
    .unwrap_err();
    assert!(refusal(&nothing).contains("nothing to correct"), "{nothing}");
    let scheduled = ended_ago(&s.t, &s.batch, 3, 2).await;
    assert!(
        corrections::correct_attendance(s.t.state(), &amal, scheduled.id, &[(s.p.id, Mark::Present)], "x")
            .await
            .is_err()
    );

    // From the page, as the coordinator.
    let coordinator = s.t.coordinator_user().await;
    s.t.app.acting_as(&coordinator);
    s.t.app
        .htmx()
        .post(
            &format!("/edtech/sessions/{}/correction", s.session.id),
            &[
                (format!("mark_{}", s.p.id).as_str(), "excused"),
                ("reason", "Doctor's note"),
            ],
        )
        .await
        .assert_ok();
    assert_eq!(row(&s.t, &s.session, &s.p).await.mark, Mark::Excused);
    // Without a reason the page says why.
    s.t.app
        .htmx()
        .post(
            &format!("/edtech/sessions/{}/correction", s.session.id),
            &[(format!("mark_{}", s.p.id).as_str(), "present"), ("reason", " ")],
        )
        .await
        .assert_status(422);
}
