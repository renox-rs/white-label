//! BL-EDU-01 (#436): an enrolment waiting for its first payment holds its
//! seat for 48 hours and then lapses (`EDU.RUL.11`): the seat is released, the
//! payment link is cancelled, the family and the counsellor are told. A pending
//! gateway payment, a part payment and a failed payment before the end do not
//! lapse it; a payment after the lapse is refunded by wl-payments (a `lapsed`
//! enrolment is no active prepaid subject).
//!
//! The payments module is a stand-in here (`Cash`): the refund of the late
//! payment itself is wl-payments' `late_confirm`, tested there; what this app
//! owns, the answer of `PrepaidSubjects` and the notice to the counsellor, is
//! tested here.

mod support;

use std::time::Duration;

use edtech::enrolment_machine::{LapseFacts, lapse_problem};
use edtech::enrolments::{self, NewEnrolment};
use edtech::lapse;
use edtech::notices::{FAMILY_NOTICE, NOTICE};
use edtech::{Batch, Enrolment, EnrolmentStatus, seats};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use support::{Centre, Messages, PRICE, boot, day, next_weekday};
use wl_kernel::PartyId;
use wl_kernel::payments::{
    PaymentFailed, PaymentFailureKind, PaymentId, PaymentStatus, RefundApproved, RefundId,
    RefundSource, prepaid_subjects,
};

/// An open batch of 12 seats starting next Monday week.
async fn open_batch(t: &Centre) -> Batch {
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    t.open_batch(&tutor, &room, next_weekday(day(7), 1)).await
}

/// The student `1`, Sara, enrolled in `batch` by the desk counsellor.
async fn sara_in(t: &Centre, batch: &Batch) -> Enrolment {
    let sara = t.person(1, "Sara Ali");
    t.enrol(sara, batch).await
}

/// The moment the hold of `e` ends.
fn hold_end(e: &Enrolment) -> renox::db::DateTime {
    e.seat_hold_until.expect("a seat hold")
}

/// The enrolment as stored now.
async fn reload(t: &Centre, e: &Enrolment) -> Enrolment {
    Enrolment::find_or_404(t.db(), e.id).await.unwrap()
}

/// A failed gateway payment of `e`.
fn failed(t: &Centre, e: &Enrolment, link: Option<wl_kernel::Subject>) -> PaymentFailed {
    let _ = t;
    PaymentFailed {
        payment: PaymentId(700),
        party: PartyId(e.payer_party_id),
        amount: PRICE,
        kind: PaymentFailureKind::Declined,
        reason: "card declined".to_owned(),
        subscription: None,
        payment_link: link,
        prepaid: Some(e.subject()),
    }
}

/// Enrolment E created in `pending_payment` with no payment: when its 48 hours
/// are up it is `lapsed`, the batch has one more free seat, the payment link
/// is cancelled, and the guardian and the counsellor are told. Before the end
/// of the hold nothing happens.
#[renox::test]
async fn edu_01_an_unpaid_hold_lapses_after_48_hours() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let (child, mother) = (t.person(1, "Omar Khan"), t.person(2, "Huda Khan"));
    t.people.guardian(child, mother);
    let desk_user = t.desk_user().await;
    let desk = t.auth(&desk_user).await;
    let e = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(child, batch.id, PRICE)
            .guardian(mother)
            .terms_accepted(),
    )
    .await
    .unwrap();
    assert_eq!(seats::available(t.db(), batch.id).await.unwrap(), 11);
    let links = t.cash.links();
    assert_eq!(links.len(), 1, "a payment link for the first invoice");
    assert_eq!(
        links[0].request.expires_at,
        hold_end(&e),
        "it expires with the hold"
    );
    assert_eq!(links[0].request.amount, PRICE);
    assert_eq!(
        reload(&t, &e).await.payment_link_path.as_deref(),
        Some("/pay/link-900")
    );

    let early = hold_end(&e) - TimeDelta::minutes(1);
    assert!(lapse::lapse_due(t.state(), early).await.unwrap().is_empty());
    assert_eq!(reload(&t, &e).await.status, EnrolmentStatus::PendingPayment);

    let lapsed = lapse::lapse_due(t.state(), hold_end(&e)).await.unwrap();

    assert_eq!(lapsed, [e.id]);
    assert_eq!(reload(&t, &e).await.status, EnrolmentStatus::Lapsed);
    assert_eq!(seats::available(t.db(), batch.id).await.unwrap(), 12);
    assert!(!reload(&t, &e).await.seat_held);
    assert!(t.cash.links()[0].cancelled, "the old link takes no payment");
    let family = t.messages.of_type(FAMILY_NOTICE);
    assert_eq!(family.len(), 1);
    assert!(
        Messages::told_party(&family[0], mother),
        "the guardian is told"
    );
    let text = Messages::text(&family[0]);
    assert!(text.contains("seat was released"), "{text}");
    assert!(text.contains(&e.number()), "{text}");
    let staff = t.messages.of_type(NOTICE);
    assert_eq!(staff.len(), 1);
    assert!(
        Messages::told_user(&staff[0], desk_user.id),
        "the counsellor is told"
    );
    assert!(Messages::text(&staff[0]).contains("lapsed"));
    // A second run finds nothing to do.
    assert!(
        lapse::lapse_due(t.state(), hold_end(&e))
            .await
            .unwrap()
            .is_empty()
    );
}

/// A gateway payment `pending` at the end of the hold keeps the enrolment
/// `pending_payment`; when it times out after that, the enrolment lapses at once.
#[renox::test]
async fn edu_01_a_pending_gateway_payment_stops_the_lapse_until_it_fails() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let e = sara_in(&t, &batch).await;
    let payment = t.cash.add(e.subject(), PaymentStatus::Pending, PRICE);

    let at_the_mark = lapse::lapse_due(t.state(), hold_end(&e)).await.unwrap();
    assert!(at_the_mark.is_empty(), "a payment is pending");
    assert_eq!(reload(&t, &e).await.status, EnrolmentStatus::PendingPayment);

    // Thirty minutes later the gateway gives up: `PaymentFailed` (timeout).
    t.cash.set_status(payment, PaymentStatus::Failed);
    let event = PaymentFailed {
        kind: PaymentFailureKind::Timeout,
        ..failed(&t, &e, None)
    };
    t.app.travel(Duration::from_secs(48 * 3600 + 30 * 60));
    t.app
        .at_travelled_time(lapse::on_payment_failed(t.state(), &event))
        .await
        .unwrap();
    t.app.travel_back();

    assert_eq!(reload(&t, &e).await.status, EnrolmentStatus::Lapsed);
    assert_eq!(seats::available(t.db(), batch.id).await.unwrap(), 12);
}

/// A failed card payment on the first day leaves the enrolment
/// `pending_payment`: a new link is sent, the old one is replaced, the hold is
/// not extended, and the enrolment still lapses when it ends.
#[renox::test]
async fn edu_01_a_failed_payment_gives_a_new_link_and_does_not_extend_the_hold() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let e = sara_in(&t, &batch).await;
    let old = reload(&t, &e).await;
    let old_link = old.payment_link().expect("a link");
    let event = failed(&t, &e, Some(old_link));

    lapse::on_payment_failed(t.state(), &event).await.unwrap();

    let now = reload(&t, &e).await;
    assert_eq!(now.status, EnrolmentStatus::PendingPayment);
    assert_eq!(
        now.seat_hold_until, e.seat_hold_until,
        "the hold is not extended"
    );
    let links = t.cash.links();
    assert_eq!(links.len(), 2);
    assert!(links[0].cancelled, "the failed link is replaced");
    assert!(!links[1].cancelled);
    assert_eq!(
        links[1].request.expires_at,
        hold_end(&e),
        "it ends with the hold"
    );
    assert_eq!(now.payment_link_path.as_deref(), Some("/pay/link-901"));
    let family = t.messages.of_type(FAMILY_NOTICE);
    assert_eq!(family.len(), 1);
    let text = Messages::text(&family[0]);
    assert!(text.contains("payment did not go through"), "{text}");
    assert!(
        text.contains("/pay/link-901"),
        "the new link is in the message: {text}"
    );

    let lapsed = lapse::lapse_due(t.state(), hold_end(&e)).await.unwrap();
    assert_eq!(lapsed, [e.id], "it still lapses when the hold ends");
}

/// Once lapsed an enrolment is no active prepaid subject, so wl-payments
/// refunds a late payment; a pending one is. The counsellor is told of the
/// refund of AED 1,050.00, and the re-enrolled student's enrolment stays
/// `pending_payment` with its own link.
#[renox::test]
async fn edu_01_a_late_payment_is_refunded_and_the_counsellor_told() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let sara = t.person(1, "Sara Ali");
    let desk_user = t.desk_user().await;
    let desk = t.auth(&desk_user).await;
    let e1 = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(sara, batch.id, PRICE).terms_accepted(),
    )
    .await
    .unwrap();
    let subjects = prepaid_subjects(t.state());
    assert!(subjects.is_active(t.state(), &e1.subject()).await.unwrap());
    t.lapse(&e1).await;
    assert!(
        !subjects.is_active(t.state(), &e1.subject()).await.unwrap(),
        "a lapsed enrolment is never an active subject"
    );
    // Another counsellor enrols the student again before the payment comes.
    let dalia_user = t
        .user(
            "Dalia",
            &[
                edtech::permissions::ENROLMENTS_VIEW,
                edtech::permissions::ENROLMENTS_MANAGE,
            ],
        )
        .await;
    let dalia = t.auth(&dalia_user).await;
    let e2 = enrolments::create(
        t.state(),
        &dalia,
        NewEnrolment::course(sara, batch.id, PRICE).terms_accepted(),
    )
    .await
    .expect("a lapsed enrolment never reopens: this is a new one");
    assert_ne!(e1.id, e2.id);
    assert!(subjects.is_active(t.state(), &e2.subject()).await.unwrap());

    // AED 1,050 arrives late for E1 and wl-payments approves its refund itself.
    let payment = t.cash.add(e1.subject(), PaymentStatus::Received, 105_000);
    let approved = RefundApproved {
        refund: RefundId(1),
        party: PartyId(sara),
        legal_entity_id: 1,
        amount: 105_000,
        currency: "AED".to_owned(),
        source: RefundSource::Payment(PaymentId(payment)),
        approved_by: None,
    };
    lapse::on_refund_approved(t.state(), &approved)
        .await
        .unwrap();

    let notices = t.messages.of_type(NOTICE);
    let refund = notices
        .iter()
        .find(|n| Messages::text(n).contains("late payment was refunded"))
        .expect("the counsellor is told of the refund");
    assert!(Messages::text(refund).contains("AED 1,050.00"));
    assert!(Messages::text(refund).contains(&e1.number()));
    assert!(Messages::told_user(refund, desk_user.id));
    assert!(
        Messages::told_user(refund, dalia_user.id),
        "the counsellor of the new enrolment is told too"
    );
    let second = reload(&t, &e2).await;
    assert_eq!(second.status, EnrolmentStatus::PendingPayment);
    assert!(
        second.payment_link_path.is_some()
            && second.payment_link_path != reload(&t, &e1).await.payment_link_path,
        "E2 is paid through its own link"
    );
}

/// A refund a person approved is not a late payment's: nobody is told.
#[renox::test]
async fn edu_01_a_refund_approved_by_a_person_is_left_alone() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let e = sara_in(&t, &batch).await;
    t.lapse(&e).await;
    let payment = t.cash.add(e.subject(), PaymentStatus::Received, 105_000);
    let approved = RefundApproved {
        refund: RefundId(1),
        party: PartyId(e.payer_party_id),
        legal_entity_id: 1,
        amount: 105_000,
        currency: "AED".to_owned(),
        source: RefundSource::Payment(PaymentId(payment)),
        approved_by: Some(9),
    };
    let before = t.messages.sent().len();

    lapse::on_refund_approved(t.state(), &approved)
        .await
        .unwrap();

    assert_eq!(t.messages.sent().len(), before);
}

/// Batch B full when the counsellor tries to re-enrol the student: no E2 is
/// created.
#[renox::test]
async fn edu_01_a_lapsed_student_cannot_take_a_seat_of_a_full_batch() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    for n in 0..11 {
        t.enrolment_in(&batch, 100 + n, EnrolmentStatus::Active)
            .await;
    }
    let e1 = sara_in(&t, &batch).await;
    t.lapse(&e1).await;
    // Somebody else takes the freed seat.
    t.enrolment_in(&batch, 200, EnrolmentStatus::Active).await;
    let desk = t.desk().await;

    let again = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(1, batch.id, PRICE).terms_accepted(),
    )
    .await;

    assert!(again.is_err(), "EDU.ACL.17 applies to a re-enrolment");
    let mine = Enrolment::query()
        .where_(Enrolment::STUDENT_PARTY_ID.eq(1))
        .count(t.db())
        .await
        .unwrap();
    assert_eq!(mine, 1, "no second enrolment was created");
}

/// A partial first payment is a payment: the enrolment does not lapse and the
/// counsellor sees the balance on its page.
#[renox::test]
async fn edu_01_a_part_payment_stops_the_lapse_and_the_page_shows_the_balance() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let e = sara_in(&t, &batch).await;
    t.books.pay(e.first_invoice_id.unwrap(), 50_000);

    let lapsed = lapse::lapse_due(t.state(), hold_end(&e)).await.unwrap();

    assert!(lapsed.is_empty());
    assert_eq!(reload(&t, &e).await.status, EnrolmentStatus::PendingPayment);
    let desk_user = t.desk_user().await;
    t.app.acting_as(&desk_user);
    t.app
        .get(&format!("/edtech/enrolments/{}", e.id))
        .await
        .assert_ok()
        .assert_see("Paid so far")
        .assert_see("AED 500.00")
        .assert_see("Balance to pay")
        .assert_see("AED 2,020.00");
}

/// A discount still waiting for the centre manager does not stop the lapse.
#[renox::test]
async fn edu_01_a_discount_waiting_for_approval_does_not_stop_the_lapse() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let desk = t.desk().await;
    let sara = t.person(1, "Sara Ali");
    let e = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(sara, batch.id, PRICE)
            .discount(50_000)
            .terms_accepted(),
    )
    .await
    .unwrap();
    assert!(!e.discount_approved);

    let lapsed = lapse::lapse_due(t.state(), hold_end(&e)).await.unwrap();

    assert_eq!(lapsed, [e.id]);
}

/// A lapsed enrolment never reopens: no transition leaves `lapsed`, and the
/// page and the batch show the freed seat.
#[renox::test]
async fn edu_01_a_lapsed_enrolment_never_reopens() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let e = sara_in(&t, &batch).await;
    let lapsed = t.lapse(&e).await;
    assert_eq!(lapsed.status, EnrolmentStatus::Lapsed);

    let activate = enrolments::activate(t.state(), None, e.id).await;
    let withdraw = enrolments::withdraw(t.state(), None, e.id, "changed my mind").await;

    assert!(activate.is_err());
    assert!(withdraw.is_err());
    assert_eq!(reload(&t, &e).await.status, EnrolmentStatus::Lapsed);
    let desk_user = t.desk_user().await;
    t.app.acting_as(&desk_user);
    t.app
        .get(&format!("/edtech/enrolments/{}", e.id))
        .await
        .assert_ok()
        .assert_see("Lapsed");
    t.app
        .get(&format!("/edtech/batches/{}", batch.id))
        .await
        .assert_ok();
}

/// The guard says in words why a hold does not lapse.
#[test]
fn edu_01_the_guard_names_why_a_hold_does_not_lapse() {
    let until = renox::db::now();
    let e = Enrolment {
        seat_hold_until: Some(until),
        ..Enrolment::default()
    };
    let facts = |paid, pending, after: i64| LapseFacts {
        at: until + TimeDelta::minutes(after),
        paid,
        pending,
    };
    assert!(lapse_problem(&e, None).is_some());
    assert!(lapse_problem(&e, Some(&facts(0, false, 0))).is_none());
    assert!(
        lapse_problem(&e, Some(&facts(1, false, 5)))
            .unwrap()
            .contains("payment arrived")
    );
    assert!(
        lapse_problem(&e, Some(&facts(0, true, 5)))
            .unwrap()
            .contains("pending at the gateway")
    );
    assert!(
        lapse_problem(&e, Some(&facts(0, false, -5)))
            .unwrap()
            .contains("still held")
    );
}
