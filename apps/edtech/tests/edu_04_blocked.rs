//! BL-EDU-04 (#439): a blocked payer. The `active` enrolments they pay for
//! pause through `freeze` without the freeze-day limit, the seat is kept and
//! nothing is sent to them; only the centre manager withdraws one, and the refund
//! waits in wl-payments (`PTY.ACL.03`); `PartyUnblocked` removes only the block's
//! cause; the package is given back the days it stood frozen; a
//! `pending_payment` enrolment is left to lapse.
//!
//! The party directory, the invoice book and the package ledger are stand-ins
//! (`support::money`); the kernel's approvals are the real ones. The extension of
//! a real package by the frozen days is wl-sales' own test
//! (`crates/wl-sales/tests/packages.rs`).

mod support;

use edtech::enrolments::{self, NewEnrolment};
use edtech::{Batch, Enrolment, EnrolmentFreeze, EnrolmentStatus, FreezeCause, seats};
use renox::prelude::*;
use support::{Centre, Messages, PRICE, boot, day, next_weekday};
use wl_kernel::approvals::approve;

const GUARDIAN: i64 = 2;

/// An open batch of 12 seats from next Monday.
async fn open_future(t: &Centre) -> Batch {
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    t.open_batch(&tutor, &room, next_weekday(day(7), 1)).await
}

/// Student `student`, a child of guardian G, enrolled in `batch` and paid: active.
async fn paid_course(t: &Centre, batch: &Batch, student: i64, name: &str) -> Enrolment {
    t.person(student, name);
    t.people.guardian(student, GUARDIAN);
    let desk = t.desk().await;
    let e = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(student, batch.id, PRICE)
            .guardian(GUARDIAN)
            .terms_accepted(),
    )
    .await
    .expect("the student is enrolled");
    let active = t.pay_first(&e).await;
    assert_eq!(active.status, EnrolmentStatus::Active);
    active
}

/// Guardian G pays for E1 (a course), E2 (a package), E3 (a course the student
/// froze until `day(14)`) and E4 (a course not paid yet).
struct Family {
    batch: Batch,
    e1: Enrolment,
    e2: Enrolment,
    e3: Enrolment,
    e4: Enrolment,
}

async fn family(t: &Centre) -> Family {
    t.person(GUARDIAN, "Hassan Ali");
    let batch = open_future(t).await;
    let e1 = paid_course(t, &batch, 1, "Sara Ali").await;
    t.person(3, "Omar Ali");
    t.people.guardian(3, GUARDIAN);
    let desk = t.desk().await;
    let pack = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::package(3, "ielts_10", "IELTS 10 sessions", 10, 150_000)
            .guardian(GUARDIAN)
            .terms_accepted(),
    )
    .await
    .expect("the package is sold");
    let e2 = t.pay_first(&pack).await;
    assert_eq!(e2.status, EnrolmentStatus::Active);
    let e3 = paid_course(t, &batch, 4, "Noor Ali").await;
    enrolments::freeze(
        t.state(),
        Some(&desk),
        e3.id,
        FreezeCause::Student,
        Some(day(14)),
    )
    .await
    .expect("the student freezes");
    t.person(5, "Yusuf Ali");
    t.people.guardian(5, GUARDIAN);
    let e4 = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(5, batch.id, PRICE)
            .guardian(GUARDIAN)
            .terms_accepted(),
    )
    .await
    .expect("the student is enrolled");
    assert_eq!(e4.status, EnrolmentStatus::PendingPayment);
    Family {
        batch,
        e1,
        e2,
        e3,
        e4,
    }
}

async fn reload(t: &Centre, e: &Enrolment) -> Enrolment {
    Enrolment::find_or_404(t.db(), e.id).await.unwrap()
}

async fn open_causes(t: &Centre, e: &Enrolment) -> Vec<FreezeCause> {
    EnrolmentFreeze::open_of(t.db(), e.id)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.cause)
        .collect()
}

/// Given guardian G paying for active E1 and E2, when G is blocked both are
/// `frozen` for the block, the seats are kept, the block counts no day against
/// the allowance, and no message goes to G.
#[renox::test]
async fn edu_04_blocking_the_payer_freezes_the_active_enrolments_and_tells_nobody() {
    let t = boot().await;
    let f = family(&t).await;
    let seated = seats::count_in(t.db(), f.batch.id).await.unwrap();
    let told = t.messages.sent().len();

    t.block(GUARDIAN).await;

    for e in [&f.e1, &f.e2] {
        let frozen = reload(&t, e).await;
        assert_eq!(frozen.status, EnrolmentStatus::Frozen, "{}", e.number());
        let rows = EnrolmentFreeze::open_of(t.db(), e.id).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].cause, FreezeCause::Block);
        assert_eq!(
            rows[0].counted_days, 0,
            "a block uses none of the allowance"
        );
        assert!(frozen.seat_held, "the seat is kept");
    }
    assert_eq!(seats::count_in(t.db(), f.batch.id).await.unwrap(), seated);
    assert_eq!(t.messages.sent().len(), told, "nothing is sent to G");
    assert!(
        t.messages
            .sent()
            .iter()
            .all(|m| !Messages::told_party(m, GUARDIAN))
    );
    // Blocking again adds no second cause.
    t.block(GUARDIAN).await;
    assert_eq!(open_causes(&t, &f.e1).await, [FreezeCause::Block]);
}

/// A counsellor cannot withdraw E1; the centre manager can; the credit note is
/// issued and the refund is asked, to be held by wl-payments for the blocked
/// party.
#[renox::test]
async fn edu_04_only_the_centre_manager_withdraws_and_the_refund_is_asked_for_the_payer() {
    let t = boot().await;
    let f = family(&t).await;
    t.block(GUARDIAN).await;

    let desk = t.desk().await;
    let refused = enrolments::request_withdrawal(t.state(), Some(&desk), f.e1.id, "moved abroad")
        .await
        .unwrap_err();
    assert!(
        matches!(&refused, Error::Status(StatusCode::UNPROCESSABLE_ENTITY, m)
            if m.contains("on hold") && m.contains("centre manager")),
        "{refused:?}"
    );
    assert_eq!(reload(&t, &f.e1).await.status, EnrolmentStatus::Frozen);
    assert!(t.books.notes().is_empty());

    let manager = t.manager().await;
    let asked = enrolments::request_withdrawal(t.state(), Some(&manager), f.e1.id, "moved abroad")
        .await
        .unwrap();
    let request = asked
        .approval_request
        .expect("the refund waits for another user");
    let accountant = t
        .user(
            "Aziz",
            &[
                edtech::permissions::ENROLMENTS_VIEW,
                edtech::permissions::ENROLMENTS_REFUND_APPROVE,
            ],
        )
        .await;
    approve(t.state(), request, &accountant, "").await.unwrap();

    let gone = reload(&t, &f.e1).await;
    assert_eq!(gone.status, EnrolmentStatus::Withdrawn);
    assert_eq!(t.books.notes().len(), 1, "the credit note is issued");
    let refunds = t.cash.refunds();
    assert_eq!(refunds.len(), 1, "the refund is asked of wl-payments");
    assert_eq!(
        refunds[0].0.party.0, GUARDIAN,
        "to the payer, who is on hold"
    );
}

/// Given E3 frozen by the student until later, after `PartyUnblocked` E3 stays
/// frozen with its own cause, and E1 and E2 are `active`.
#[renox::test]
async fn edu_04_unblocking_removes_only_the_cause_of_the_block() {
    let t = boot().await;
    let f = family(&t).await;
    t.block(GUARDIAN).await;
    assert_eq!(
        open_causes(&t, &f.e3).await,
        [FreezeCause::Student, FreezeCause::Block],
        "the student's freeze is kept and the block is a second cause"
    );

    t.unblock(GUARDIAN).await;

    assert_eq!(reload(&t, &f.e1).await.status, EnrolmentStatus::Active);
    assert_eq!(reload(&t, &f.e2).await.status, EnrolmentStatus::Active);
    assert_eq!(reload(&t, &f.e3).await.status, EnrolmentStatus::Frozen);
    assert_eq!(open_causes(&t, &f.e3).await, [FreezeCause::Student]);
    assert!(open_causes(&t, &f.e1).await.is_empty());
}

/// Both the payer and the student are blocked: unblocking one leaves the cause
/// until the other is unblocked too.
#[renox::test]
async fn edu_04_the_cause_stays_while_the_other_party_is_still_blocked() {
    let t = boot().await;
    let f = family(&t).await;
    t.block(GUARDIAN).await;
    t.block(1).await;
    assert_eq!(open_causes(&t, &f.e1).await, [FreezeCause::Block]);

    t.unblock(GUARDIAN).await;
    assert_eq!(reload(&t, &f.e1).await.status, EnrolmentStatus::Frozen);
    t.unblock(1).await;
    assert_eq!(reload(&t, &f.e1).await.status, EnrolmentStatus::Active);
}

/// A package of E2 valid through day 50, frozen for 10 days by the block, is
/// valid through day 60 once G is unblocked.
#[renox::test]
async fn edu_04_the_package_is_given_back_the_days_it_stood_frozen() {
    let t = boot().await;
    let f = family(&t).await;
    let package = f.e2.package_id.expect("the package of E2");
    t.packages.set_end(package, day(50));
    t.block(GUARDIAN).await;
    // The block began ten days ago.
    Enrolment::query()
        .where_(Enrolment::ID.eq(f.e2.id))
        .update(t.db(), &[("frozen_on", &Some(day(-10)))])
        .await
        .unwrap();

    t.unblock(GUARDIAN).await;

    assert_eq!(reload(&t, &f.e2).await.status, EnrolmentStatus::Active);
    assert_eq!(t.packages.end_of(package), Some(day(60)));
    let given = t.packages.extended();
    assert_eq!(given.len(), 1);
    assert_eq!((given[0].0, given[0].1), (package, 10));
}

/// Given E4 of G in `pending_payment`, the block leaves it `pending_payment`
/// and it lapses at its seat hold.
#[renox::test]
async fn edu_04_a_pending_payment_enrolment_is_left_to_lapse() {
    let t = boot().await;
    let f = family(&t).await;

    t.block(GUARDIAN).await;
    let waiting = reload(&t, &f.e4).await;
    assert_eq!(waiting.status, EnrolmentStatus::PendingPayment);
    assert!(open_causes(&t, &f.e4).await.is_empty());

    let lapsed = t.lapse(&waiting).await;
    assert_eq!(lapsed.status, EnrolmentStatus::Lapsed);
}

/// Staff see the enrolment of a blocked payer as frozen *On hold*, and a
/// counsellor is not offered the withdrawal.
#[renox::test]
async fn edu_04_the_page_says_on_hold_and_offers_the_withdrawal_to_the_manager_only() {
    let t = boot().await;
    let f = family(&t).await;
    t.block(GUARDIAN).await;
    let path = format!("/edtech/enrolments/{}", f.e1.id);

    let desk = t.desk_user().await;
    t.app.acting_as(&desk);
    let page = t.app.get(&path).await;
    page.assert_ok().assert_see("Frozen").assert_see("On hold");
    assert!(
        !page.text().contains("The student leaves."),
        "no withdrawal offered"
    );

    let manager = t.manager_user().await;
    t.app.acting_as(&manager);
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("On hold")
        .assert_see("The student leaves.");
}
