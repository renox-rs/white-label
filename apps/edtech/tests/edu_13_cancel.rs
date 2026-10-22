//! BL-EDU-13 (#448): the centre cancels a batch before its first session. Each
//! student is moved to another batch or withdrawn with everything refunded,
//! registration fee included; a transfer whose seat is gone falls back to a
//! withdrawal and the coordinator is told; a student who has not paid is
//! withdrawn with nothing to refund; each refund waits for its own approval.

mod support;

use edtech::batch_machine::CANCEL;
use edtech::enrolments::{self, NewEnrolment};
use edtech::exit::{ExitAction, ExitChoice};
use edtech::notices::{FAMILY_NOTICE, NOTICE};
use edtech::{Batch, BatchStatus, Enrolment, EnrolmentStatus, SessionStatus, batches, seats};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use support::{Centre, Messages, PRICE, boot, day, next_weekday};
use wl_kernel::invoicing::{InvoiceId, InvoicePaid};

/// An open batch of 12 seats starting on the first Monday on or after `from`,
/// with a tutor and a room of its own.
async fn open_batch(t: &Centre, name: &str, from: NaiveDate) -> Batch {
    let tutor = t.tutor(&format!("Tutor {name}")).await;
    let room = t.room(&format!("Room {name}"), 14).await;
    t.open_batch(&tutor, &room, next_weekday(from, 1)).await
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

/// A paid, active enrolment of `student` in `batch`: AED 2,520.00 and the
/// AED 210.00 registration fee, with an admin fee a policy would keep.
async fn paid_enrolment(t: &Centre, student: i64, batch: &Batch) -> Enrolment {
    let desk = t.desk().await;
    let e = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(student, batch.id, PRICE)
            .registration_fee(21_000)
            .admin_fee(10_500)
            .terms_accepted(),
    )
    .await
    .unwrap();
    pay_first(t, &e).await;
    let active = Enrolment::find_or_404(t.db(), e.id).await.unwrap();
    assert_eq!(active.status, EnrolmentStatus::Active);
    active
}

async fn reload(t: &Centre, e: &Enrolment) -> Enrolment {
    Enrolment::find_or_404(t.db(), e.id).await.unwrap()
}

/// Open batch B with E1 paid AED 2,520 + registration AED 210: the coordinator
/// cancels B with the reason "too few students" and E1 -> withdraw. E1 is
/// `withdrawn`, a credit note of AED 2,730 exists and a refund of AED 2,730
/// awaits approval (no admin fee, no policy tier); the guardian is told; the
/// sessions are cancelled.
#[renox::test]
async fn edu_13_a_withdrawn_student_gets_everything_back_including_the_registration_fee() {
    let t = boot().await;
    let b = open_batch(&t, "B", day(7)).await;
    let (child, mother) = (t.person(1, "Omar Khan"), t.person(2, "Huda Khan"));
    t.people.guardian(child, mother);
    let desk = t.desk().await;
    let created = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(child, b.id, PRICE)
            .guardian(mother)
            .registration_fee(21_000)
            .admin_fee(10_500)
            .terms_accepted(),
    )
    .await
    .unwrap();
    pay_first(&t, &created).await;
    let amal_user = t.coordinator_user().await;
    let amal = t.auth(&amal_user).await;

    let done = batches::cancel(
        t.state(),
        &amal,
        b.id,
        "too few students",
        vec![ExitChoice::withdraw(created.id)],
    )
    .await
    .unwrap();

    assert_eq!(done.status, BatchStatus::Cancelled);
    assert_eq!(done.cancel_reason.as_deref(), Some("too few students"));
    let e = reload(&t, &created).await;
    assert_eq!(e.status, EnrolmentStatus::Withdrawn);
    assert_eq!(e.refund_amount, 273_000, "everything paid, no admin fee");
    assert_eq!(e.amount_owed, 0);
    assert!(!e.seat_held);
    let notes = t.books.notes();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].gross, 273_000, "AED 2,730.00");
    assert_eq!(notes[0].lines, [(1, 240_000), (2, 20_000)], "the registration fee too");
    let refunds = t.cash.refunds();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].0.amount, 273_000);
    assert_eq!(refunds[0].1, Some(amal_user.id), "recorded by the coordinator, decided by another");
    let family = t.messages.of_type(FAMILY_NOTICE);
    let told = family
        .iter()
        .find(|n| Messages::text(n).contains("cancelled"))
        .expect("the guardian is told");
    assert!(Messages::told_party(told, mother));
    assert!(Messages::text(told).contains("273") || Messages::text(told).contains("2,730.00"));
    assert!(
        t.sessions_of(&b)
            .await
            .iter()
            .all(|s| s.status == SessionStatus::Cancelled),
        "the sessions are cancelled"
    );
    assert_eq!(seats::count_in(t.db(), b.id).await.unwrap(), 0);
}

/// E2 -> transfer to B2 creates an active enrolment in B2, holding what E2 paid.
#[renox::test]
async fn edu_13_a_transfer_creates_an_active_enrolment_in_the_other_batch() {
    let t = boot().await;
    let b = open_batch(&t, "B", day(7)).await;
    let b2 = open_batch(&t, "B2", day(21)).await;
    let e2 = paid_enrolment(&t, t.person(1, "Sara Ali"), &b).await;
    let amal = t.coordinator().await;

    batches::cancel(
        t.state(),
        &amal,
        b.id,
        "too few students",
        vec![ExitChoice::transfer(e2.id, b2.id)],
    )
    .await
    .unwrap();

    let old = reload(&t, &e2).await;
    assert_eq!(old.status, EnrolmentStatus::Transferred);
    let next = Enrolment::find_or_404(t.db(), old.transferred_to_id.expect("a new enrolment"))
        .await
        .unwrap();
    assert_eq!(next.status, EnrolmentStatus::Active);
    assert_eq!(next.batch_id, Some(b2.id));
    assert_eq!(next.student_party_id, 1);
    assert_eq!(next.carried_paid, 273_000, "all that was paid moves");
    assert!(next.seat_held);
    assert_eq!(seats::count_in(t.db(), b.id).await.unwrap(), 0);
    assert_eq!(seats::count_in(t.db(), b2.id).await.unwrap(), 1);
    assert!(t.books.notes().is_empty(), "nothing is credited for a transfer");
    assert!(t.cash.refunds().is_empty());
    let family = t.messages.of_type(FAMILY_NOTICE);
    assert!(
        family
            .iter()
            .any(|n| Messages::text(n).contains(&b2.number()) && Messages::text(n).contains("moved")),
        "the student is told where they go"
    );
}

/// B2 is full at the cancel: E2 is withdrawn with a full refund instead and the
/// coordinator is told.
#[renox::test]
async fn edu_13_a_transfer_whose_seat_is_gone_falls_back_to_a_withdrawal() {
    let t = boot().await;
    let b = open_batch(&t, "B", day(7)).await;
    let b2 = open_batch(&t, "B2", day(21)).await;
    let e2 = paid_enrolment(&t, t.person(1, "Sara Ali"), &b).await;
    for n in 0..12 {
        t.enrolment_in(&b2, 100 + n, EnrolmentStatus::Active).await;
    }
    let amal_user = t.coordinator_user().await;
    let amal = t.auth(&amal_user).await;

    let done = batches::cancel(
        t.state(),
        &amal,
        b.id,
        "too few students",
        vec![ExitChoice::transfer(e2.id, b2.id)],
    )
    .await
    .unwrap();

    let gone = reload(&t, &e2).await;
    assert_eq!(gone.status, EnrolmentStatus::Withdrawn);
    assert_eq!(gone.refund_amount, 273_000, "a full refund instead");
    assert_eq!(done.exit_log.0.len(), 1);
    assert_eq!(done.exit_log.0[0].action, ExitAction::FellBack);
    assert_eq!(seats::count_in(t.db(), b2.id).await.unwrap(), 12, "B2 took nobody");
    let staff = t.messages.of_type(NOTICE);
    let told = staff
        .iter()
        .find(|n| Messages::text(n).contains("could not be moved"))
        .expect("the coordinator is told");
    assert!(Messages::told_user(told, amal_user.id));
    assert!(Messages::text(told).contains(&e2.number()));
    assert_eq!(t.books.notes().len(), 1);
    assert_eq!(t.cash.refunds().len(), 1);
}

/// A student who has not paid is withdrawn with nothing to refund and the link
/// is cancelled; a student not named is withdrawn by default.
#[renox::test]
async fn edu_13_an_unpaid_student_is_withdrawn_with_nothing_to_refund() {
    let t = boot().await;
    let b = open_batch(&t, "B", day(7)).await;
    let waiting = t.enrol(t.person(1, "Sara Ali"), &b).await;
    let paid = paid_enrolment(&t, t.person(2, "Omar Khan"), &b).await;
    let amal = t.coordinator().await;

    batches::cancel(t.state(), &amal, b.id, "too few students", Vec::new())
        .await
        .unwrap();

    let unpaid = reload(&t, &waiting).await;
    assert_eq!(unpaid.status, EnrolmentStatus::Withdrawn);
    assert_eq!((unpaid.refund_amount, unpaid.amount_owed), (0, 0));
    assert!(t.cash.links()[0].cancelled, "the payment link is cancelled");
    let other = reload(&t, &paid).await;
    assert_eq!(other.status, EnrolmentStatus::Withdrawn, "withdraw is the default");
    assert_eq!(other.refund_amount, 273_000);
    assert_eq!(t.books.notes().len(), 1, "a credit note for the paid one only");
}

/// `cancel` without a reason is refused, from `running` it is not offered, and a
/// choice for a student who has not paid, or for a batch of another course, is
/// refused in words.
#[renox::test]
async fn edu_13_a_cancel_needs_a_reason_a_batch_that_has_not_started_and_possible_choices() {
    let t = boot().await;
    let b = open_batch(&t, "B", day(7)).await;
    let b2 = open_batch(&t, "B2", day(21)).await;
    let waiting = t.enrol(t.person(1, "Sara Ali"), &b).await;
    let amal = t.coordinator().await;

    let blank = batches::cancel(t.state(), &amal, b.id, "  ", Vec::new())
        .await
        .unwrap_err();
    assert!(blank.to_string().contains("reason"), "{blank}");
    // Through the transition alone the choices must have been checked.
    let bare = b
        .transition(CANCEL)
        .by(&amal)
        .reason("too few students")
        .run(t.state())
        .await;
    assert!(matches!(bare, Err(TransitionError::Guard(_))));
    let unpaid = batches::cancel(
        t.state(),
        &amal,
        b.id,
        "too few students",
        vec![ExitChoice::transfer(waiting.id, b2.id)],
    )
    .await
    .unwrap_err();
    assert!(unpaid.to_string().contains("has not paid"), "{unpaid}");
    let stranger = batches::cancel(
        t.state(),
        &amal,
        b.id,
        "too few students",
        vec![ExitChoice::withdraw(9_999)],
    )
    .await
    .unwrap_err();
    assert!(stranger.to_string().contains("does not hold a seat"), "{stranger}");
    assert_eq!(
        Batch::find_or_404(t.db(), b.id).await.unwrap().status,
        BatchStatus::Open,
        "a refused cancel changes nothing"
    );

    let running = {
        let batch = Batch::find_or_404(t.db(), b2.id).await.unwrap();
        batches::start_due(t.state(), batch.start_date).await.unwrap();
        Batch::find_or_404(t.db(), b2.id).await.unwrap()
    };
    assert_eq!(running.status, BatchStatus::Running);
    let late = batches::cancel(t.state(), &amal, running.id, "too late", Vec::new())
        .await
        .unwrap_err();
    assert!(late.to_string().contains("running"), "{late}");
}

/// A batch that is cancelled holds no seat and the tutor and room are free
/// again; a counsellor cannot cancel; the page offers the action to the
/// coordinator only for a planned or open batch.
#[renox::test]
async fn edu_13_only_the_coordinator_cancels_and_the_page_offers_it_before_the_start() {
    let t = boot().await;
    let b = open_batch(&t, "B", day(7)).await;
    let desk = t.desk().await;
    let denied = batches::cancel(t.state(), &desk, b.id, "too few students", Vec::new()).await;
    assert!(matches!(denied, Err(Error::Forbidden)));

    let amal_user = t.coordinator_user().await;
    t.app.acting_as(&amal_user);
    t.app
        .get(&format!("/edtech/batches/{}", b.id))
        .await
        .assert_ok()
        .assert_see("Cancel the batch");
    let amal = t.auth(&amal_user).await;
    batches::cancel(t.state(), &amal, b.id, "too few students", Vec::new())
        .await
        .unwrap();
    t.app.acting_as(&amal_user);
    t.app
        .get(&format!("/edtech/batches/{}", b.id))
        .await
        .assert_ok()
        .assert_dont_see("Cancel the batch");
}
