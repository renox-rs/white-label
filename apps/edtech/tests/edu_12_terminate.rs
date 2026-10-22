//! BL-EDU-12 (#447): a running batch is stopped. The coordinator requests
//! `terminate` (`EDU.ACL.15`), the centre manager approves; then the remaining
//! sessions are cancelled and each student is moved to another batch or
//! withdrawn with the unused sessions refunded pro rata, without admin fee or
//! the no-refund tier; the tutor is paid for the sessions taught only; the
//! people are told, and for a regulatory reason the centre manager and the
//! owner as well.

mod support;

use edtech::batches::{self, NewBatch};
use edtech::enrolments::{self, NewEnrolment};
use edtech::exit::{ExitAction, ExitChoice};
use edtech::notices::{FAMILY_NOTICE, NOTICE};
use edtech::{Batch, BatchStatus, Enrolment, EnrolmentStatus, Session, SessionStatus, sessions};
use renox::auth::permissions;
use renox::chrono::TimeDelta;
use renox::prelude::*;
use support::{Centre, Messages, PRICE, boot, day, next_weekday};
use wl_kernel::approvals::{ApprovalPolicy, approve};
use wl_kernel::invoicing::{InvoiceId, InvoicePaid};

/// A batch of 12 sessions (Monday and Wednesday for six weeks) that started
/// five weeks ago, opened, with eight of its sessions in the past: AED 210.00
/// a session at the course fee of AED 2,520.00.
async fn running_batch(t: &Centre, name: &str) -> Batch {
    let tutor = t.tutor(&format!("Tutor {name}")).await;
    let room = t.room(&format!("Room {name}"), 14).await;
    let course = t.course().await;
    let amal = t.coordinator().await;
    let first = next_weekday(day(-35), 1);
    let batch = batches::create(
        t.state(),
        &amal,
        NewBatch::new(
            course.id,
            12,
            (first, first + TimeDelta::days(41)),
            &[1, 3],
            "18:00",
            90,
        )
        .tutor(tutor.id, None)
        .room(room.id, Some(14)),
    )
    .await
    .expect("the batch is planned");
    batches::open(t.state(), &amal, batch.id)
        .await
        .expect("the batch opens")
}

/// A batch of the same course that starts next month, to move students to.
async fn target_batch(t: &Centre) -> Batch {
    let tutor = t.tutor("Tutor B2").await;
    let room = t.room("Room B2", 14).await;
    t.open_batch(&tutor, &room, next_weekday(day(30), 1)).await
}

/// The tutor submits the attendance of the first `count` sessions of `batch`.
async fn hold(t: &Centre, batch: &Batch, count: usize) {
    let tariq = t.teacher().await;
    let planned = Session::of_batch(t.db(), batch.id).await.unwrap();
    for session in planned.iter().filter(|s| s.status == SessionStatus::Scheduled).take(count) {
        sessions::submit_attendance(t.state(), &tariq, session.id)
            .await
            .expect("the attendance is submitted");
    }
}

/// The first invoice of `enrolment` is paid in full, which activates it.
async fn pay_first(t: &Centre, enrolment: &Enrolment) {
    let id = enrolment.first_invoice_id.expect("a first invoice");
    let invoice = t
        .books
        .all()
        .into_iter()
        .find(|i| i.id == id)
        .expect("the invoice");
    t.books.pay(id, invoice.gross());
    t.state()
        .emit(InvoicePaid {
            invoice: InvoiceId(id),
            party: enrolment.payer(),
            source: enrolment.subject(),
        })
        .await
        .unwrap();
}

/// A paid, active enrolment of `student` in `batch`, with an admin fee a
/// termination must not keep.
async fn paid_enrolment(t: &Centre, student: i64, batch: &Batch) -> Enrolment {
    let desk = t.desk().await;
    let e = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(student, batch.id, PRICE)
            .admin_fee(10_500)
            .terms_accepted(),
    )
    .await
    .unwrap();
    pay_first(t, &e).await;
    Enrolment::find_or_404(t.db(), e.id).await.unwrap()
}

async fn reload(t: &Centre, e: &Enrolment) -> Enrolment {
    Enrolment::find_or_404(t.db(), e.id).await.unwrap()
}

/// A user who holds the global role `role` with `grants`.
async fn holder(t: &Centre, name: &str, role: &str, grants: &[&str]) -> User {
    let email = format!("{}@example.com", name.to_lowercase());
    let user = User::register(t.db(), name, &email, "password123")
        .await
        .unwrap();
    permissions::define_role(t.db(), role, grants).await.unwrap();
    user.assign_role(t.db(), role).await.unwrap();
    user
}

/// The sessions of `batch` in `status`.
async fn count_in(t: &Centre, batch: &Batch, status: SessionStatus) -> usize {
    Session::of_batch(t.db(), batch.id)
        .await
        .unwrap()
        .iter()
        .filter(|s| s.status == status)
        .count()
}

/// Running batch B (12 sessions, 5 held) with E1 (paid AED 2,520) and E2: the
/// coordinator requests `terminate` with the reason "venue", E1 -> withdraw,
/// E2 -> transfer to B2, and the centre manager approves: 7 sessions are
/// `cancelled`, E1 is `withdrawn` with a refund of 2,520 x 7/12 = AED 1,470
/// awaiting approval, E2 is `transferred` with a new active enrolment in B2,
/// students and guardians are told, and the tutor's timesheet has 5 sessions.
#[renox::test]
async fn edu_12_the_approved_termination_cancels_the_rest_and_settles_each_student() {
    let t = boot().await;
    let b = running_batch(&t, "B").await;
    let b2 = target_batch(&t).await;
    let (omar, mother, sara) = (
        t.person(1, "Omar Khan"),
        t.person(2, "Huda Khan"),
        t.person(3, "Sara Ali"),
    );
    t.people.guardian(omar, mother);
    let desk = t.desk().await;
    let created = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(omar, b.id, PRICE)
            .guardian(mother)
            .admin_fee(10_500)
            .terms_accepted(),
    )
    .await
    .unwrap();
    pay_first(&t, &created).await;
    let e1 = reload(&t, &created).await;
    let e2 = paid_enrolment(&t, sara, &b).await;
    hold(&t, &b, 5).await;
    assert_eq!(
        Batch::find_or_404(t.db(), b.id).await.unwrap().status,
        BatchStatus::Running
    );
    let amal_user = t.coordinator_user().await;
    let amal = t.auth(&amal_user).await;

    let asked = batches::terminate(
        t.state(),
        &amal,
        b.id,
        "venue",
        None,
        vec![ExitChoice::withdraw(e1.id), ExitChoice::transfer(e2.id, b2.id)],
    )
    .await
    .unwrap();

    // Nothing changes while the centre manager decides.
    let request = asked.approval_request.expect("the centre manager decides");
    assert_eq!(asked.batch.status, BatchStatus::Running);
    assert_eq!(count_in(&t, &b, SessionStatus::Scheduled).await, 7);
    assert_eq!(reload(&t, &e1).await.status, EnrolmentStatus::Active);
    assert!(t.cash.refunds().is_empty());

    let manager = t.manager_user().await;
    approve(t.state(), request, &manager, "").await.unwrap();

    let stopped = Batch::find_or_404(t.db(), b.id).await.unwrap();
    assert_eq!(stopped.status, BatchStatus::Terminated);
    assert_eq!(stopped.termination_reason.as_deref(), Some("The venue is lost"));
    assert_eq!(count_in(&t, &b, SessionStatus::Cancelled).await, 7);
    assert_eq!(count_in(&t, &b, SessionStatus::Held).await, 5);
    let one = reload(&t, &e1).await;
    assert_eq!(one.status, EnrolmentStatus::Withdrawn);
    assert_eq!(one.refund_amount, 147_000, "2,520 x 7/12, no admin fee");
    let notes = t.books.notes();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].gross, 147_000);
    let refunds = t.cash.refunds();
    assert_eq!(refunds.len(), 1, "one refund, waiting for its own approval");
    assert_eq!(refunds[0].0.amount, 147_000);
    assert_eq!(refunds[0].1, Some(amal_user.id));
    let two = reload(&t, &e2).await;
    assert_eq!(two.status, EnrolmentStatus::Transferred);
    let next = Enrolment::find_or_404(t.db(), two.transferred_to_id.unwrap())
        .await
        .unwrap();
    assert_eq!(next.status, EnrolmentStatus::Active);
    assert_eq!(next.batch_id, Some(b2.id));
    assert_eq!(
        next.carried_paid, 147_000,
        "the unused value moves to the new enrolment"
    );
    let family = t.messages.of_type(FAMILY_NOTICE);
    assert!(family.iter().any(|n| Messages::told_party(n, mother)), "the guardian is told");
    assert!(family.iter().any(|n| Messages::told_party(n, sara)), "the student is told");
    assert!(
        t.messages
            .of_type(NOTICE)
            .iter()
            .all(|n| !Messages::text(n).contains("regulatory")),
        "a venue is not a regulatory reason"
    );
    // The tutor is paid for the 5 sessions taught, not the 12 planned.
    let taught = sessions::taught_in(t.db(), b.tutor_resource_id.unwrap(), day(-60), day(60))
        .await
        .unwrap();
    assert_eq!(taught.len(), 5);
}

/// With the reason "approval revoked" (or expired) the centre manager and the
/// owner each get the regulatory notice as well; with "venue" they do not.
#[renox::test]
async fn edu_12_a_regulatory_reason_tells_the_centre_manager_and_the_owner() {
    let t = boot().await;
    let b = running_batch(&t, "B").await;
    let sara = t.person(1, "Sara Ali");
    let e = paid_enrolment(&t, sara, &b).await;
    hold(&t, &b, 5).await;
    let manager = holder(
        &t,
        "Manal",
        edtech::permissions::CENTRE_MANAGER_ROLE,
        &[edtech::permissions::BATCHES_APPROVE, edtech::permissions::BATCHES_VIEW],
    )
    .await;
    let owner = holder(&t, "Omar", edtech::permissions::OWNER_ROLE, &[]).await;
    let amal = t.coordinator().await;

    let asked = batches::terminate(
        t.state(),
        &amal,
        b.id,
        "approval_revoked",
        Some("the regulator withdrew the licence"),
        vec![ExitChoice::withdraw(e.id)],
    )
    .await
    .unwrap();
    approve(t.state(), asked.approval_request.unwrap(), &manager, "")
        .await
        .unwrap();

    let stopped = Batch::find_or_404(t.db(), b.id).await.unwrap();
    assert_eq!(stopped.status, BatchStatus::Terminated);
    assert_eq!(
        stopped.termination_reason.as_deref(),
        Some("The approval was revoked: the regulator withdrew the licence")
    );
    let notices = t.messages.of_type(NOTICE);
    let regulatory = notices
        .iter()
        .find(|n| Messages::text(n).contains("regulatory"))
        .expect("the regulatory notice");
    assert!(Messages::told_user(regulatory, manager.id), "the centre manager");
    assert!(Messages::told_user(regulatory, owner.id), "the owner");
    assert!(Messages::text(regulatory).contains("approval was revoked"));
}

/// The coordinator cannot approve their own request, even holding the
/// permission; a user without it cannot either.
#[renox::test]
async fn edu_12_the_coordinator_cannot_approve_their_own_request() {
    let t = boot().await;
    let b = running_batch(&t, "B").await;
    hold(&t, &b, 5).await;
    let amina_user = t
        .user(
            "Amina",
            &[
                edtech::permissions::BATCHES_VIEW,
                edtech::permissions::BATCHES_MANAGE,
                edtech::permissions::BATCHES_APPROVE,
            ],
        )
        .await;
    let amina = t.auth(&amina_user).await;
    let asked = batches::terminate(t.state(), &amina, b.id, "venue", None, Vec::new())
        .await
        .unwrap();
    let request = asked.approval_request.unwrap();

    let own = approve(t.state(), request, &amina_user, "").await;
    let amal_user = t.coordinator_user().await;
    let other = approve(t.state(), request, &amal_user, "").await;

    assert!(matches!(own, Err(Error::Forbidden)), "the requester never approves");
    assert!(matches!(other, Err(Error::Forbidden)), "nor does a coordinator without the right");
    assert_eq!(
        Batch::find_or_404(t.db(), b.id).await.unwrap().status,
        BatchStatus::Running
    );
}

/// B2 is full at the approval: E2 is withdrawn instead, refunded pro rata, and
/// the coordinator is told.
#[renox::test]
async fn edu_12_a_transfer_whose_seat_is_gone_is_withdrawn_and_the_coordinator_told() {
    let t = boot().await;
    let b = running_batch(&t, "B").await;
    let b2 = target_batch(&t).await;
    let sara = t.person(1, "Sara Ali");
    let e2 = paid_enrolment(&t, sara, &b).await;
    hold(&t, &b, 5).await;
    let amal_user = t.coordinator_user().await;
    let amal = t.auth(&amal_user).await;
    let asked = batches::terminate(
        t.state(),
        &amal,
        b.id,
        "tutor_unavailable",
        None,
        vec![ExitChoice::transfer(e2.id, b2.id)],
    )
    .await
    .unwrap();
    // The last seat of B2 goes between the request and the approval.
    for n in 0..12 {
        t.enrolment_in(&b2, 100 + n, EnrolmentStatus::Active).await;
    }

    let manager = t.manager_user().await;
    approve(t.state(), asked.approval_request.unwrap(), &manager, "")
        .await
        .unwrap();

    let gone = reload(&t, &e2).await;
    assert_eq!(gone.status, EnrolmentStatus::Withdrawn);
    assert_eq!(gone.refund_amount, 147_000);
    let stopped = Batch::find_or_404(t.db(), b.id).await.unwrap();
    assert_eq!(stopped.exit_log.0[0].action, ExitAction::FellBack);
    let told = t
        .messages
        .of_type(NOTICE)
        .into_iter()
        .find(|n| Messages::text(n).contains("could not be moved"))
        .expect("the coordinator is told");
    assert!(Messages::told_user(&told, amal_user.id));
}

/// The sessions held while the request waited count as used; a student past
/// half the course still gets the pro rata refund, with no admin fee; one who
/// has not paid is withdrawn with nothing to refund and the link cancelled.
#[renox::test]
async fn edu_12_sessions_held_before_the_approval_count_and_no_tier_or_fee_applies() {
    let t = boot().await;
    let b = running_batch(&t, "B").await;
    let e1 = paid_enrolment(&t, t.person(1, "Sara Ali"), &b).await;
    let waiting = t.enrol(t.person(2, "Omar Khan"), &b).await;
    hold(&t, &b, 5).await;
    let amal = t.coordinator().await;
    let asked = batches::terminate(t.state(), &amal, b.id, "too_few_students", None, Vec::new())
        .await
        .unwrap();
    // Two more sessions are taught while the centre manager decides: 7 of 12,
    // past the half where a withdrawal would get nothing.
    hold(&t, &b, 2).await;

    let manager = t.manager_user().await;
    approve(t.state(), asked.approval_request.unwrap(), &manager, "")
        .await
        .unwrap();

    let paid = reload(&t, &e1).await;
    assert_eq!(paid.status, EnrolmentStatus::Withdrawn);
    assert_eq!(paid.refund_amount, 105_000, "2,520 x 5/12: 7 sessions were held");
    assert_eq!(count_in(&t, &b, SessionStatus::Held).await, 7);
    assert_eq!(count_in(&t, &b, SessionStatus::Cancelled).await, 5);
    let unpaid = reload(&t, &waiting).await;
    assert_eq!(unpaid.status, EnrolmentStatus::Withdrawn);
    assert_eq!((unpaid.refund_amount, unpaid.amount_owed), (0, 0));
    assert!(t.cash.links().iter().any(|l| l.cancelled), "the payment link is cancelled");
    assert_eq!(t.books.notes().len(), 1, "a credit note for the paid student only");
}

/// Only a running batch is terminated; a termination needs a known reason; a
/// policy taken away cannot let it through unapproved.
#[renox::test]
async fn edu_12_a_termination_needs_a_running_batch_a_reason_and_its_approval_policy() {
    let t = boot().await;
    let b = running_batch(&t, "B").await;
    let amal = t.coordinator().await;
    // Still `open` until the first session is held.
    let early = batches::terminate(t.state(), &amal, b.id, "venue", None, Vec::new())
        .await
        .unwrap_err();
    assert!(early.to_string().contains("running"), "{early}");

    hold(&t, &b, 2).await;
    let unknown = batches::terminate(t.state(), &amal, b.id, "bored", None, Vec::new())
        .await
        .unwrap_err();
    assert!(unknown.to_string().contains("Pick the reason"), "{unknown}");
    let desk = t.desk().await;
    let denied = batches::terminate(t.state(), &desk, b.id, "venue", None, Vec::new()).await;
    assert!(matches!(denied, Err(Error::Forbidden)));

    ApprovalPolicy::query()
        .where_(ApprovalPolicy::ACTION.eq("edtech.terminate"))
        .delete(t.db())
        .await
        .unwrap();
    let unapproved = batches::terminate(t.state(), &amal, b.id, "venue", None, Vec::new())
        .await
        .unwrap_err();
    assert!(unapproved.to_string().contains("approval"), "{unapproved}");
    assert_eq!(
        Batch::find_or_404(t.db(), b.id).await.unwrap().status,
        BatchStatus::Running
    );
}

/// The batch page tells the coordinator the termination waits and shows the
/// approval to the centre manager.
#[renox::test]
async fn edu_12_the_page_offers_the_termination_and_shows_the_request() {
    let t = boot().await;
    let b = running_batch(&t, "B").await;
    hold(&t, &b, 2).await;
    let amal_user = t.coordinator_user().await;
    t.app.acting_as(&amal_user);
    t.app
        .get(&format!("/edtech/batches/{}", b.id))
        .await
        .assert_ok()
        .assert_see("Terminate the batch")
        .assert_dont_see("Cancel the batch");
    let amal = t.auth(&amal_user).await;
    batches::terminate(t.state(), &amal, b.id, "venue", None, Vec::new())
        .await
        .unwrap();

    t.app.acting_as(&amal_user);
    t.app
        .get(&format!("/edtech/batches/{}", b.id))
        .await
        .assert_ok()
        .assert_see("waits for the centre manager");
    let manager = t.manager_user().await;
    let waiting = wl_kernel::approvals::awaiting(t.db(), &manager).await.unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].action, "edtech.terminate");
}
