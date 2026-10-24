//! The base of the 10 EdTech enrolment (base for #436, #437, #438, #439, #440,
//! #441): a student is enrolled with a seat held, the fees are raised through
//! the invoicer, the first payment activates it, a freeze keeps its causes
//! within the allowance, and a withdrawal stores the refund the policy gives.

mod support;

use edtech::batches::{self, NewBatch};
use edtech::enrolment::{EnrolmentFreeze, Instalment};
use edtech::enrolment_machine::{ACTIVATE_SYSTEM, FREEZE_ALLOWANCE_DAYS, WITHDRAW_APPROVED};
use edtech::enrolments::{self, NewEnrolment};
use edtech::refund::{RefundOutcome, ratio};
use edtech::{
    Batch, Enrolment, EnrolmentKind, EnrolmentStatus, FreezeCause, InstalmentStatus, seats,
};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use renox::state_machine::history;
use support::{Centre, PRICE, boot, day, next_weekday};
use wl_kernel::PartyId;
use wl_kernel::invoicing::{InvoiceId, InvoicePaid};

/// An open batch of 12 seats on Monday and Wednesday, from next Monday.
async fn open_future(t: &Centre) -> Batch {
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    t.open_batch(&tutor, &room, next_weekday(day(7), 1)).await
}

/// An open batch that started two weeks ago.
async fn open_started(t: &Centre) -> Batch {
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    t.open_batch(&tutor, &room, next_weekday(day(-14), 1)).await
}

/// The first invoice of `enrolment` is paid in full.
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

/// Enrolling holds a seat of the batch for 48 hours, inserts the enrolment
/// `pending_payment` and raises the first invoice and the instalment plan:
/// AED 2,520 in two instalments of AED 1,260, the first with the AED 105
/// registration fee.
#[renox::test]
async fn edu_base_enrolling_holds_a_seat_and_raises_the_first_invoice() {
    let t = boot().await;
    let batch = open_future(&t).await;
    let student = t.person(1, "Sara Ali");
    let desk = t.desk().await;

    let e = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(student, batch.id, PRICE)
            .instalments(2)
            .registration_fee(10_500)
            .terms_accepted(),
    )
    .await
    .unwrap();

    assert_eq!(e.status, EnrolmentStatus::PendingPayment);
    assert_eq!(e.kind, EnrolmentKind::Course);
    assert!(e.seat_held);
    assert_eq!(e.number(), format!("ENR-{:04}", e.id));
    assert_eq!(e.payer_party_id, student);
    assert_eq!(e.sessions_total, 8);
    let hold = e.seat_hold_until.expect("a hold end");
    assert!(hold > renox::db::now() + TimeDelta::hours(47));
    assert!(hold <= renox::db::now() + TimeDelta::hours(48));
    let batch = Batch::find_or_404(t.db(), batch.id).await.unwrap();
    assert_eq!(batch.seats_taken, 1);
    assert_eq!(seats::available(t.db(), batch.id).await.unwrap(), 11);

    let invoices = t.books.all();
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].gross(), 126_000 + 10_500);
    assert_eq!(invoices[0].draft.party, PartyId(student));
    assert!(invoices[0].issued);
    assert_eq!(e.first_invoice_id, Some(invoices[0].id));

    let plan = Instalment::of_enrolment(t.db(), e.id).await.unwrap();
    assert_eq!(plan.len(), 2);
    assert_eq!((plan[0].amount, plan[1].amount), (126_000, 126_000));
    assert_eq!(plan[0].status, InstalmentStatus::Invoiced);
    assert_eq!(plan[1].status, InstalmentStatus::Scheduled);
    assert_eq!(
        plan[1].due_date,
        edtech::fees::due_date(plan[0].due_date, 2)
    );
}

/// The later instalments are invoiced on their due dates, for the active
/// enrolments only.
#[renox::test]
async fn edu_base_a_later_instalment_is_invoiced_when_it_falls_due() {
    let t = boot().await;
    let batch = open_future(&t).await;
    let student = t.person(1, "Sara Ali");
    let desk = t.desk().await;
    let e = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(student, batch.id, PRICE)
            .instalments(2)
            .terms_accepted(),
    )
    .await
    .unwrap();
    let due = edtech::fees::due_date(day(0), 2);
    // Nothing is billed while the enrolment waits for its payment.
    assert_eq!(edtech::fees::invoice_due(t.state(), due).await.unwrap(), 0);
    pay_first(&t, &e).await;
    assert_eq!(edtech::fees::invoice_due(t.state(), due).await.unwrap(), 1);
    assert_eq!(t.books.all().len(), 2);
    assert_eq!(t.books.all()[1].gross(), 126_000);
    let plan = Instalment::of_enrolment(t.db(), e.id).await.unwrap();
    assert_eq!(plan[1].status, InstalmentStatus::Invoiced);
    assert!(plan[1].invoice_id.is_some());
    // A second run bills nothing again.
    assert_eq!(edtech::fees::invoice_due(t.state(), due).await.unwrap(), 0);
}

/// The student, the guardian and the payer are parties: unknown ones are
/// refused, a guardian must be one of the student's, and the guardian pays
/// unless another payer is named.
#[renox::test]
async fn edu_base_the_student_guardian_and_payer_are_parties() {
    let t = boot().await;
    let batch = open_future(&t).await;
    let desk = t.desk().await;
    let child = t.person(1, "Omar Khan");
    let mother = t.person(2, "Huda Khan");
    let stranger = t.person(3, "Nadia Said");
    let company = t.person(4, "Gulf Logistics LLC");
    t.people.guardian(child, mother);

    let unknown =
        enrolments::create(t.state(), &desk, NewEnrolment::course(99, batch.id, PRICE)).await;
    assert!(unknown.is_err());

    let wrong = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(child, batch.id, PRICE).guardian(stranger),
    )
    .await
    .unwrap_err();
    assert!(wrong.to_string().contains("guardian"));
    assert_eq!(seats::available(t.db(), batch.id).await.unwrap(), 12);

    let e = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(child, batch.id, PRICE).guardian(mother),
    )
    .await
    .unwrap();
    assert_eq!(e.guardian_party_id, Some(mother));
    assert_eq!(e.payer_party_id, mother);
    assert_eq!(t.books.all()[0].draft.party, PartyId(mother));

    // A second child of the same mother: the same student has one place only.
    let sister = t.person(5, "Dana Khan");
    t.people.guardian(sister, mother);
    let paid_by_company = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(sister, batch.id, PRICE)
            .guardian(mother)
            .payer(company),
    )
    .await
    .unwrap();
    assert_eq!(paid_by_company.payer_party_id, company);
}

/// Only an open batch takes an enrolment, and a full batch takes no more:
/// the counsellor stops at the capacity, the centre manager at the room's
/// limit.
#[renox::test]
async fn edu_base_the_seats_of_a_batch_are_taken_up_to_the_capacity() {
    let t = boot().await;
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let course = t.course().await;
    let amal = t.coordinator().await;
    let first = next_weekday(day(7), 1);
    let planned = batches::create(
        t.state(),
        &amal,
        NewBatch::new(
            course.id,
            1,
            (first, first + TimeDelta::days(27)),
            &[1, 3],
            "18:00",
            90,
        )
        .tutor(tutor.id, None)
        .room(room.id, Some(2)),
    )
    .await
    .unwrap();
    let desk = t.desk().await;
    let (a, b, c) = (
        t.person(1, "Sara Ali"),
        t.person(2, "Omar Khan"),
        t.person(3, "Huda Khan"),
    );

    let closed = enrolments::create(t.state(), &desk, NewEnrolment::course(a, planned.id, PRICE))
        .await
        .unwrap_err();
    assert!(closed.to_string().contains("not open"));

    let batch = batches::open(t.state(), &amal, planned.id).await.unwrap();
    enrolments::create(t.state(), &desk, NewEnrolment::course(a, batch.id, PRICE))
        .await
        .unwrap();
    let full = enrolments::create(t.state(), &desk, NewEnrolment::course(b, batch.id, PRICE))
        .await
        .unwrap_err();
    assert!(full.to_string().contains("no free seat"));
    assert_eq!(
        Batch::find_or_404(t.db(), batch.id)
            .await
            .unwrap()
            .seats_taken,
        1
    );

    let manager = t.manager().await;
    enrolments::create(
        t.state(),
        &manager,
        NewEnrolment::course(b, batch.id, PRICE),
    )
    .await
    .unwrap();
    let over = enrolments::create(
        t.state(),
        &manager,
        NewEnrolment::course(c, batch.id, PRICE),
    )
    .await
    .unwrap_err();
    assert!(over.to_string().contains("no free seat"));
}

/// An enrolment is activated only with the terms accepted and its discount
/// approved; the activation confirms the seat, which stays taken.
#[renox::test]
async fn edu_base_activation_needs_the_terms_and_an_approved_discount() {
    let t = boot().await;
    let batch = open_future(&t).await;
    let student = t.person(1, "Sara Ali");
    let desk = t.desk().await;
    let manager = t.manager().await;

    let e = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(student, batch.id, PRICE).discount(50_000),
    )
    .await
    .unwrap();
    assert!(!e.discount_approved);
    let refused = enrolments::activate(t.state(), Some(&desk), e.id)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("terms"));

    enrolments::accept_terms(t.state(), &desk, e.id)
        .await
        .unwrap();
    let still = enrolments::activate(t.state(), Some(&desk), e.id)
        .await
        .unwrap_err();
    assert!(still.to_string().contains("discount"));
    assert!(
        enrolments::approve_discount(t.state(), &desk, e.id)
            .await
            .is_err()
    );

    enrolments::approve_discount(t.state(), &manager, e.id)
        .await
        .unwrap();
    let active = enrolments::activate(t.state(), Some(&desk), e.id)
        .await
        .unwrap();
    assert_eq!(active.status, EnrolmentStatus::Active);
    assert!(active.seat_hold_until.is_none());
    assert!(active.seat_held);
    assert_eq!(
        Batch::find_or_404(t.db(), batch.id)
            .await
            .unwrap()
            .seats_taken,
        1
    );
    // The fee is the price less the discount.
    assert_eq!(active.fee(), PRICE - 50_000);
}

/// The first payment of the first invoice activates the enrolment as the
/// system; one whose terms are not accepted waits for staff.
#[renox::test]
async fn edu_base_the_first_payment_activates_the_enrolment() {
    let t = boot().await;
    let batch = open_future(&t).await;
    let desk = t.desk().await;
    let (a, b) = (t.person(1, "Sara Ali"), t.person(2, "Omar Khan"));

    let accepted = t.enrol(a, &batch).await;
    pay_first(&t, &accepted).await;
    let active = Enrolment::find_or_404(t.db(), accepted.id).await.unwrap();
    assert_eq!(active.status, EnrolmentStatus::Active);
    let rows = history(t.db(), "edtech_enrolments", active.id)
        .await
        .unwrap();
    assert_eq!(
        rows.last().map(|r| r.transition.as_str()),
        Some(ACTIVATE_SYSTEM)
    );

    let no_terms = enrolments::create(t.state(), &desk, NewEnrolment::course(b, batch.id, PRICE))
        .await
        .unwrap();
    pay_first(&t, &no_terms).await;
    let waiting = Enrolment::find_or_404(t.db(), no_terms.id).await.unwrap();
    assert_eq!(waiting.status, EnrolmentStatus::PendingPayment);
}

/// A freeze is a cause: the student's own counts against 30 days a year, a
/// block or a mirrored freeze does not, and the enrolment is active again only
/// when no cause is left.
#[renox::test]
async fn edu_base_a_freeze_keeps_its_causes_within_the_yearly_allowance() {
    let t = boot().await;
    let batch = open_future(&t).await;
    let student = t.person(1, "Sara Ali");
    let desk = t.desk().await;
    let e = t
        .enrolment_in(&batch, student, EnrolmentStatus::Active)
        .await;

    // A freeze of 10 days (today to the ninth day after).
    let until = day(9);
    let frozen = enrolments::freeze(
        t.state(),
        Some(&desk),
        e.id,
        FreezeCause::Student,
        Some(until),
    )
    .await
    .unwrap();
    assert_eq!(frozen.status, EnrolmentStatus::Frozen);
    let rows = EnrolmentFreeze::open_of(t.db(), e.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].counted_days, 10);

    // The same cause twice is refused; a block only adds its cause.
    let twice = enrolments::freeze(
        t.state(),
        Some(&desk),
        e.id,
        FreezeCause::Student,
        Some(until),
    )
    .await
    .unwrap_err();
    assert!(twice.to_string().contains("already frozen"));
    enrolments::freeze(t.state(), None, e.id, FreezeCause::Block, None)
        .await
        .unwrap();
    let rows = EnrolmentFreeze::open_of(t.db(), e.id).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].counted_days, 0);

    // Each trigger removes only its own cause.
    let still = enrolments::unfreeze(t.state(), Some(&desk), e.id, FreezeCause::Student)
        .await
        .unwrap();
    assert_eq!(still.status, EnrolmentStatus::Frozen);
    let active = enrolments::unfreeze(t.state(), None, e.id, FreezeCause::Block)
        .await
        .unwrap();
    assert_eq!(active.status, EnrolmentStatus::Active);
    assert!(
        EnrolmentFreeze::open_of(t.db(), e.id)
            .await
            .unwrap()
            .is_empty()
    );

    // 10 days are used: 21 more would pass the 30 of the year.
    let used = EnrolmentFreeze::counted_in_year(t.db(), student, day(0))
        .await
        .unwrap();
    assert_eq!(used, 10);
    let too_long = enrolments::freeze(
        t.state(),
        Some(&desk),
        e.id,
        FreezeCause::Student,
        Some(day(FREEZE_ALLOWANCE_DAYS - used)),
    )
    .await
    .unwrap_err();
    assert!(too_long.to_string().contains("30 days"));
    let ok = enrolments::freeze(
        t.state(),
        Some(&desk),
        e.id,
        FreezeCause::Student,
        Some(day(FREEZE_ALLOWANCE_DAYS - used - 1)),
    )
    .await
    .unwrap();
    assert_eq!(ok.status, EnrolmentStatus::Frozen);
}

/// A frozen enrolment keeps its seat; only staff with `enrolments.manage` act.
#[renox::test]
async fn edu_base_a_frozen_enrolment_keeps_its_seat_and_only_staff_freeze() {
    let t = boot().await;
    let batch = open_future(&t).await;
    let student = t.person(1, "Sara Ali");
    let e = t
        .enrolment_in(&batch, student, EnrolmentStatus::Active)
        .await;
    let tutor = t.teacher().await;
    let denied = enrolments::freeze(
        t.state(),
        Some(&tutor),
        e.id,
        FreezeCause::Student,
        Some(day(3)),
    )
    .await;
    assert!(denied.is_err());
    let desk = t.desk().await;
    enrolments::freeze(
        t.state(),
        Some(&desk),
        e.id,
        FreezeCause::Student,
        Some(day(3)),
    )
    .await
    .unwrap();
    assert_eq!(
        Batch::find_or_404(t.db(), batch.id)
            .await
            .unwrap()
            .seats_taken,
        1
    );
}

/// `withdraw` after the start: the refund is the paid amount less the sessions
/// held and the admin fee (`EDU.REF.01`), stored on the enrolment with the
/// reason; the seat is free and a second withdrawal is refused.
#[renox::test]
async fn edu_base_withdrawing_stores_the_refund_and_frees_the_seat() {
    let t = boot().await;
    let batch = open_started(&t).await;
    let student = t.person(1, "Sara Ali");
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
    pay_first(&t, &e).await;
    for (number, days_ago) in [(101, 3), (102, 2), (103, 1)] {
        t.held_session(&batch, number, days_ago).await;
    }

    // The row as it is after the payment: `active`, not `pending_payment`.
    let paid = Enrolment::find_or_404(t.db(), e.id).await.unwrap();
    assert_eq!(paid.status, EnrolmentStatus::Active);
    let preview = enrolments::refund_for(t.state(), &paid, day(0))
        .await
        .unwrap();
    assert_eq!(preview.outcome, RefundOutcome::AfterStart);

    let blank = enrolments::withdraw(t.state(), Some(&desk), e.id, " ").await;
    assert!(blank.is_err());
    // A refund waits for another user: the enrolment stays as it is until the
    // centre manager approves it (`FIN.ACL.04`).
    let asked = enrolments::request_withdrawal(t.state(), Some(&desk), e.id, "moved abroad")
        .await
        .unwrap();
    assert_eq!(asked.enrolment.status, EnrolmentStatus::Active);
    let request = asked
        .approval_request
        .expect("the refund waits for approval");
    let manager = t.manager_user().await;
    wl_kernel::approvals::approve(t.state(), request, &manager, "")
        .await
        .unwrap();
    let gone = Enrolment::find_or_404(t.db(), e.id).await.unwrap();
    assert_eq!(gone.status, EnrolmentStatus::Withdrawn);
    assert_eq!(gone.refund_amount, PRICE - ratio(PRICE, 3, 8) - 10_500);
    assert_eq!(gone.refund_amount, preview.refund);
    assert_eq!(gone.amount_owed, 0);
    assert_eq!(gone.withdrawn_on, Some(day(0)));
    assert_eq!(gone.withdrawal_reason.as_deref(), Some("moved abroad"));
    assert!(!gone.seat_held);
    assert_eq!(
        Batch::find_or_404(t.db(), batch.id)
            .await
            .unwrap()
            .seats_taken,
        0
    );
    let rows = history(t.db(), "edtech_enrolments", gone.id).await.unwrap();
    assert_eq!(
        rows.last().map(|r| r.transition.as_str()),
        Some(WITHDRAW_APPROVED)
    );
    assert!(
        enrolments::withdraw(t.state(), Some(&desk), e.id, "again")
            .await
            .is_err()
    );
}

/// An instalment plan whose second part is not invoiced owes the difference
/// and the unpaid instalment is cancelled.
#[renox::test]
async fn edu_base_withdrawing_cancels_the_instalments_not_yet_invoiced() {
    let t = boot().await;
    let batch = open_started(&t).await;
    let student = t.person(1, "Sara Ali");
    let desk = t.desk().await;
    let e = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(student, batch.id, PRICE)
            .instalments(2)
            .admin_fee(10_500)
            .terms_accepted(),
    )
    .await
    .unwrap();
    pay_first(&t, &e).await;
    for (number, days_ago) in [(101, 6), (102, 5), (103, 4), (104, 3), (105, 2)] {
        t.held_session(&batch, number, days_ago).await;
    }
    let gone = enrolments::withdraw(t.state(), Some(&desk), e.id, "too hard")
        .await
        .unwrap();
    // Five of eight sessions used (past the half) and AED 1,260 paid: the
    // student owes what is used and the admin fee, less what was paid.
    assert_eq!(gone.refund_amount, 0);
    assert_eq!(gone.amount_owed, ratio(PRICE, 5, 8) + 10_500 - 126_000);
    let plan = Instalment::of_enrolment(t.db(), e.id).await.unwrap();
    assert_eq!(plan[0].status, InstalmentStatus::Invoiced);
    assert_eq!(plan[1].status, InstalmentStatus::Cancelled);
}

/// A seat that was never paid for credits nothing and gives the seat back.
#[renox::test]
async fn edu_base_withdrawing_from_pending_payment_credits_nothing() {
    let t = boot().await;
    let batch = open_future(&t).await;
    let student = t.person(1, "Sara Ali");
    let e = t.enrol(student, &batch).await;
    let desk = t.desk().await;
    let gone = enrolments::withdraw(t.state(), Some(&desk), e.id, "changed my mind")
        .await
        .unwrap();
    assert_eq!(gone.status, EnrolmentStatus::Withdrawn);
    assert_eq!((gone.refund_amount, gone.amount_owed), (0, 0));
    assert_eq!(
        Batch::find_or_404(t.db(), batch.id)
            .await
            .unwrap()
            .seats_taken,
        0
    );
}

/// A package enrolment sells a package of sessions through wl-sales' contract,
/// with its invoice, and holds no seat.
#[renox::test]
async fn edu_base_a_package_enrolment_sells_a_package_and_holds_no_seat() {
    let t = boot().await;
    let student = t.person(1, "Sara Ali");
    let desk = t.desk().await;
    let e = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::package(student, "ielts_10", "IELTS, 10 sessions", 10, 150_000),
    )
    .await
    .unwrap();
    assert_eq!(e.kind, EnrolmentKind::Package);
    assert!(!e.seat_held);
    assert!(e.batch_id.is_none());
    let sold = t.packages.sold();
    assert_eq!(sold.len(), 1);
    assert_eq!(sold[0].credits, 10);
    assert_eq!(sold[0].item, "ielts_10");
    assert_eq!(sold[0].holder, PartyId(student));
    assert_eq!(sold[0].invoice, e.first_invoice_id);
    assert_eq!(e.package_id, Some(1));
    assert_eq!(t.books.all()[0].gross(), 150_000);
}

/// A membership starts its subscription when it activates, and ends it when
/// the student leaves.
#[renox::test]
async fn edu_base_a_membership_follows_its_subscription() {
    let t = boot().await;
    let student = t.person(1, "Sara Ali");
    let desk = t.desk().await;
    let e = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::membership(student, "club", "Conversation club", 31_500).terms_accepted(),
    )
    .await
    .unwrap();
    assert!(t.subs.started().is_empty());
    let active = enrolments::activate(t.state(), Some(&desk), e.id)
        .await
        .unwrap();
    let started = t.subs.started();
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].party, PartyId(student));
    assert_eq!(started[0].source, active.subject());
    assert_eq!(started[0].unit_price, 30_000);
    assert_eq!(active.subscription_id, Some(1));

    enrolments::withdraw(t.state(), Some(&desk), e.id, "moving")
        .await
        .unwrap();
    let ended = t.subs.cancelled();
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].0, active.subject());
}

/// The enrolment pages: the list, the form with the party picker and the batch
/// select, the page with its actions; a tutor sees none of it.
#[renox::test]
async fn edu_base_the_enrolment_pages_show_and_enrol() {
    let t = boot().await;
    let batch = open_future(&t).await;
    let student = t.person(23, "Sara Ali");
    let desk = t.desk_user().await;
    t.app.acting_as(&desk);

    t.app
        .get("/edtech/enrolments")
        .await
        .assert_ok()
        .assert_see("Enrolments")
        .assert_see("Enrol a student");

    t.app
        .get("/edtech/parties/options?q=sara")
        .await
        .assert_ok()
        .assert_see("P-00023");

    t.app
        .htmx()
        .post(
            "/edtech/enrolments",
            &[
                ("kind", "course"),
                ("student", "P-00023"),
                ("batch", batch.id.to_string().as_str()),
                ("price", "2,520"),
                ("registration_fee", "105"),
                ("terms", "on"),
            ],
        )
        .await
        .assert_ok();
    let e = Enrolment::query()
        .where_(Enrolment::STUDENT_PARTY_ID.eq(student))
        .first(t.db())
        .await
        .unwrap()
        .expect("the enrolment");
    assert_eq!(e.price, PRICE);
    assert_eq!(e.registration_fee, 10_500);

    t.app
        .get("/edtech/enrolments")
        .await
        .assert_ok()
        .assert_see("Sara Ali")
        .assert_see("Waiting for payment")
        .assert_see("2,520.00");
    t.app
        .get(&format!("/edtech/enrolments/{}", e.id))
        .await
        .assert_ok()
        .assert_see("Activate")
        .assert_see("Withdraw")
        .assert_see("AED 105.00");

    t.app
        .htmx()
        .post(
            &format!("/edtech/enrolments/{}/activate", e.id),
            &[("x", "y")],
        )
        .await
        .assert_ok();
    let active = Enrolment::find_or_404(t.db(), e.id).await.unwrap();
    assert_eq!(active.status, EnrolmentStatus::Active);

    t.app
        .htmx()
        .post(
            &format!("/edtech/enrolments/{}/withdraw", e.id),
            &[("reason", "moving")],
        )
        .await
        .assert_ok();
    t.app
        .get(&format!("/edtech/enrolments/{}", e.id))
        .await
        .assert_ok()
        .assert_see("Withdrawn");

    let tutor = t.teacher_user().await;
    t.app.acting_as(&tutor);
    t.app.get("/edtech/enrolments").await.assert_forbidden();
}
