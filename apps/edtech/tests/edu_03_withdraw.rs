//! BL-EDU-03 (#438): a student leaves early and the money is settled with
//! proper tax documents. `withdraw` calculates the refund by the policy the
//! student accepted (`EDU.REF.01`); a refund waits for another user's approval
//! (`FIN.ACL.04`) and the enrolment is `withdrawn` only then; the credit note,
//! the cancelled instalments, the invoice for an amount owed and the refund
//! request follow. A refund that fails later leaves the enrolment `withdrawn`.
//!
//! The invoice book, the payments and the notifier are stand-ins
//! (`support::money`); the approvals are the kernel's own.

mod support;

use edtech::enrolments::{self, NewEnrolment};
use edtech::refund::vat_share;
use edtech::{Batch, Enrolment, EnrolmentKind, EnrolmentStatus, InstalmentStatus, seats};
use edtech::batches::{self, NewBatch};
use edtech::enrolment::Instalment;
use renox::chrono::TimeDelta;
use renox::prelude::*;
use support::{Centre, PRICE, boot, day, next_weekday};
use wl_kernel::approvals::{self, approve};
use wl_kernel::invoicing::{CREDIT_NOTE_SUBJECT_TYPE, InvoiceId, InvoicePaid};
use wl_kernel::payments::{
    PaymentMethod, RefundAttemptRef, RefundFailed, RefundFailedFrom, RefundFailureKind, RefundId,
    RefundSource,
};
use wl_kernel::PartyId;

/// A batch of 12 sessions that started two weeks ago (Monday and Wednesday for
/// six weeks), 12 seats, opened: AED 210.00 a session at the course fee of
/// AED 2,520.00.
async fn started_batch(t: &Centre) -> Batch {
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let course = t.course().await;
    let amal = t.coordinator().await;
    let first = next_weekday(day(-14), 1);
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

/// `count` sessions of `batch` held on earlier days.
async fn hold_sessions(t: &Centre, batch: &Batch, count: i64) {
    for n in 0..count {
        t.held_session(batch, 101 + n, count - n).await;
    }
}

/// An enrolment of Sara in `batch` with the AED 105.00 admin fee, upfront,
/// paid and active.
async fn paid_enrolment(t: &Centre, batch: &Batch, instalments: i64) -> Enrolment {
    let sara = t.person(1, "Sara Ali");
    let desk = t.desk().await;
    let e = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(sara, batch.id, PRICE)
            .instalments(instalments)
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

/// Counsellor A withdraws a student with policy "after start", 3 of 12
/// sessions held and AED 2,520.00 paid upfront: a refund of
/// 2,520 - 3 x 210 - 105 = AED 1,785.00 waits for approval; A cannot approve;
/// user B approves; the enrolment is `withdrawn`, a credit note of AED 1,785.00
/// (VAT AED 85.00) exists, the seat is free and a refund of AED 1,785.00 is
/// requested, recorded by A so that A cannot approve it either.
#[renox::test]
async fn edu_03_a_refund_waits_for_another_user_and_the_documents_follow() {
    let t = boot().await;
    let batch = started_batch(&t).await;
    let e = paid_enrolment(&t, &batch, 1).await;
    hold_sessions(&t, &batch, 3).await;
    let desk_user = t.desk_user().await;
    let desk = t.auth(&desk_user).await;

    let asked = enrolments::request_withdrawal(t.state(), Some(&desk), e.id, "moved abroad")
        .await
        .unwrap();

    let request = asked.approval_request.expect("the refund waits for approval");
    assert_eq!(asked.enrolment.status, EnrolmentStatus::Active, "not before the approval");
    assert_eq!(asked.enrolment.refund_amount, 0);
    assert_eq!(seats::count_in(t.db(), batch.id).await.unwrap(), 1);
    assert!(t.books.notes().is_empty());
    assert!(t.cash.refunds().is_empty());
    let denied = approve(t.state(), request, &desk_user, "").await;
    assert!(matches!(denied, Err(Error::Forbidden)), "A cannot approve");

    let manager = t.manager_user().await;
    approve(t.state(), request, &manager, "").await.unwrap();

    let gone = reload(&t, &e).await;
    assert_eq!(gone.status, EnrolmentStatus::Withdrawn);
    assert_eq!(gone.refund_amount, 178_500);
    assert_eq!(gone.withdrawal_reason.as_deref(), Some("moved abroad"));
    assert!(!gone.seat_held);
    assert_eq!(seats::available(t.db(), batch.id).await.unwrap(), 12);
    let notes = t.books.notes();
    assert_eq!(notes.len(), 1, "one credit note");
    assert_eq!(notes[0].gross, 178_500);
    assert_eq!(vat_share(notes[0].gross), 8_500, "VAT AED 85.00");
    assert_eq!(notes[0].lines, [(1, 170_000)]);
    assert_eq!(Some(notes[0].invoice), e.first_invoice_id);
    let refunds = t.cash.refunds();
    assert_eq!(refunds.len(), 1);
    let (refund, recorder) = &refunds[0];
    assert_eq!(refund.amount, 178_500);
    assert_eq!(refund.party, PartyId(1));
    assert_eq!(refund.currency, "AED");
    assert!(matches!(&refund.source,
        RefundSource::CreditNote(s) if s.subject_type == CREDIT_NOTE_SUBJECT_TYPE));
    assert_eq!(*recorder, Some(desk_user.id), "A recorded the refund");
    assert!(gone.settlement.0.done);
    assert_eq!(gone.settlement.0.refunds.len(), 1);
}

/// The requester never approves, even holding the permission.
#[renox::test]
async fn edu_03_the_user_who_asks_cannot_approve_their_own_withdrawal() {
    let t = boot().await;
    let batch = started_batch(&t).await;
    let e = paid_enrolment(&t, &batch, 1).await;
    hold_sessions(&t, &batch, 3).await;
    let dana_user = t
        .user(
            "Dana",
            &[
                edtech::permissions::ENROLMENTS_VIEW,
                edtech::permissions::ENROLMENTS_MANAGE,
                edtech::permissions::ENROLMENTS_REFUND_APPROVE,
            ],
        )
        .await;
    let dana = t.auth(&dana_user).await;

    let asked = enrolments::request_withdrawal(t.state(), Some(&dana), e.id, "moved abroad")
        .await
        .unwrap();
    let request = asked.approval_request.expect("waits");
    let denied = approve(t.state(), request, &dana_user, "").await;

    assert!(matches!(denied, Err(Error::Forbidden)));
    assert_eq!(reload(&t, &e).await.status, EnrolmentStatus::Active);
}

/// The same course in two instalments of AED 1,260.00 (the second not yet
/// invoiced) and 8 sessions held: the second instalment is cancelled and one
/// invoice for 8 x 210 + 105 - 1,260 = AED 525.00 is issued; no credit note,
/// and no approval since nothing is refunded.
#[renox::test]
async fn edu_03_the_amount_owed_is_invoiced_once_and_the_instalment_cancelled() {
    let t = boot().await;
    let batch = started_batch(&t).await;
    let e = paid_enrolment(&t, &batch, 2).await;
    hold_sessions(&t, &batch, 8).await;
    let desk = t.desk().await;

    let asked = enrolments::request_withdrawal(t.state(), Some(&desk), e.id, "too hard")
        .await
        .unwrap();

    assert!(asked.approval_request.is_none(), "nothing is refunded");
    let gone = asked.enrolment;
    assert_eq!(gone.status, EnrolmentStatus::Withdrawn);
    assert_eq!((gone.refund_amount, gone.amount_owed), (0, 52_500));
    let plan = Instalment::of_enrolment(t.db(), e.id).await.unwrap();
    assert_eq!(plan[0].status, InstalmentStatus::Invoiced);
    assert_eq!(plan[1].status, InstalmentStatus::Cancelled);
    assert!(t.books.notes().is_empty(), "no credit note");
    assert!(t.cash.refunds().is_empty());
    let invoices = t.books.all();
    assert_eq!(invoices.len(), 2, "the first invoice and the amount owed");
    assert_eq!(invoices[1].gross(), 52_500, "AED 525.00");
    assert_eq!(gone.settlement.0.owed_invoice, Some(invoices[1].id));
    assert_eq!(invoices[1].draft.party, PartyId(1));
}

/// 7 of 12 sessions held (after 50%) and paid upfront: no refund and no
/// invoice.
#[renox::test]
async fn edu_03_after_half_the_course_nothing_is_refunded_or_billed() {
    let t = boot().await;
    let batch = started_batch(&t).await;
    let e = paid_enrolment(&t, &batch, 1).await;
    hold_sessions(&t, &batch, 7).await;
    let desk = t.desk().await;

    let asked = enrolments::request_withdrawal(t.state(), Some(&desk), e.id, "no time")
        .await
        .unwrap();

    assert!(asked.approval_request.is_none());
    let gone = asked.enrolment;
    assert_eq!(gone.status, EnrolmentStatus::Withdrawn);
    assert_eq!((gone.refund_amount, gone.amount_owed), (0, 0));
    assert!(t.books.notes().is_empty());
    assert!(t.cash.refunds().is_empty());
    assert_eq!(t.books.all().len(), 1, "no invoice for an amount owed");
}

/// A membership enrolment: the subscription is cancelled at once, and no other
/// credit note is issued.
#[renox::test]
async fn edu_03_a_membership_is_cancelled_at_once_without_another_credit_note() {
    let t = boot().await;
    let sara = t.person(1, "Sara Ali");
    let desk = t.desk().await;
    let e = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::membership(sara, "gold", "Gold membership", 50_000).terms_accepted(),
    )
    .await
    .unwrap();
    enrolments::activate(t.state(), Some(&desk), e.id).await.unwrap();
    assert_eq!(t.subs.started().len(), 1);

    let asked = enrolments::request_withdrawal(t.state(), Some(&desk), e.id, "moving")
        .await
        .unwrap();

    assert!(asked.approval_request.is_none());
    assert_eq!(asked.enrolment.status, EnrolmentStatus::Withdrawn);
    let cancelled = t.subs.cancelled();
    assert_eq!(cancelled.len(), 1, "the subscription is cancelled at once");
    assert_eq!(cancelled[0].0, e.subject());
    assert_eq!(cancelled[0].1, day(0));
    assert!(t.books.notes().is_empty(), "its own pro rata credit is wl-invoicing's");
    assert!(t.cash.refunds().is_empty());
}

/// A package with 4 of 10 credits unused at AED 150.00 a session: the refund
/// is AED 600.00 and waits for approval. The package `refund` of wl-sales
/// (BL-SAL, one credit note of its own) is not built, so this vertical issues
/// no credit note for it (DEFERRED #438).
#[renox::test]
async fn edu_03_a_package_refund_is_valued_at_the_single_session_price() {
    let t = boot().await;
    let sara = t.person(1, "Sara Ali");
    let desk = t.desk().await;
    let e = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::package(sara, "pack10", "Ten sessions", 10, 150_000).terms_accepted(),
    )
    .await
    .unwrap();
    assert_eq!(e.kind, EnrolmentKind::Package);
    pay_first(&t, &e).await;
    t.packages.give(PartyId(sara), "pack10", 4);

    let asked = enrolments::request_withdrawal(t.state(), Some(&desk), e.id, "enough")
        .await
        .unwrap();
    let request = asked.approval_request.expect("a refund waits for approval");
    let manager = t.manager_user().await;
    approve(t.state(), request, &manager, "").await.unwrap();

    let gone = reload(&t, &e).await;
    assert_eq!(gone.status, EnrolmentStatus::Withdrawn);
    assert_eq!(gone.refund_amount, 60_000, "AED 600.00");
    assert!(
        gone.settlement.0.credit_notes.is_empty(),
        "the package credit note is wl-sales' own"
    );
}

/// From `pending_payment` nothing was paid: no credit note, the payment link
/// is cancelled, the seat released, and no approval is asked.
#[renox::test]
async fn edu_03_from_pending_payment_nothing_is_credited_and_the_link_is_cancelled() {
    let t = boot().await;
    let batch = started_batch(&t).await;
    let sara = t.person(1, "Sara Ali");
    let e = t.enrol(sara, &batch).await;
    assert_eq!(e.status, EnrolmentStatus::PendingPayment);
    let desk = t.desk().await;

    let asked = enrolments::request_withdrawal(t.state(), Some(&desk), e.id, "changed my mind")
        .await
        .unwrap();

    assert!(asked.approval_request.is_none());
    assert_eq!(asked.enrolment.status, EnrolmentStatus::Withdrawn);
    assert!(t.books.notes().is_empty(), "no credit note");
    assert!(t.cash.refunds().is_empty());
    assert!(t.cash.links()[0].cancelled, "the payment link is cancelled");
    assert_eq!(seats::available(t.db(), batch.id).await.unwrap(), 12);
}

/// An issued instalment invoice nothing was paid on is credited in full, the
/// paid one gets the credit note of the refund.
#[renox::test]
async fn edu_03_an_unpaid_instalment_invoice_is_credited_in_full() {
    let t = boot().await;
    let batch = started_batch(&t).await;
    let e = paid_enrolment(&t, &batch, 2).await;
    hold_sessions(&t, &batch, 3).await;
    let plan = Instalment::of_enrolment(t.db(), e.id).await.unwrap();
    assert_eq!(edtech::fees::invoice_due(t.state(), plan[1].due_date).await.unwrap(), 1);
    let second = Instalment::of_enrolment(t.db(), e.id).await.unwrap()[1]
        .invoice_id
        .expect("the second instalment is invoiced");
    let desk = t.desk().await;

    let asked = enrolments::request_withdrawal(t.state(), Some(&desk), e.id, "leaving")
        .await
        .unwrap();
    let manager = t.manager_user().await;
    approve(t.state(), asked.approval_request.unwrap(), &manager, "")
        .await
        .unwrap();

    // Paid 1,260.00; used 3 x 210 + 105 = 735.00; refund 525.00.
    let gone = reload(&t, &e).await;
    assert_eq!(gone.refund_amount, 52_500);
    assert!(t.books.credited_in_full(second), "the unpaid invoice is credited in full");
    let notes = t.books.notes();
    assert_eq!(notes.len(), 2);
    let refund_note = notes
        .iter()
        .find(|n| Some(n.invoice) == e.first_invoice_id)
        .expect("the refund is credited on the paid invoice");
    assert_eq!(refund_note.gross, 52_500);
    assert_eq!(t.cash.refunds().len(), 1, "only the paid money is asked back");
}

/// If the refund fails afterwards the enrolment stays `withdrawn` and nothing in
/// this vertical changes (the credit note is reopened by wl-invoicing, BL-INV-08,
/// and the retry is wl-payments', `FIN.RUL.21`).
#[renox::test]
async fn edu_03_a_failed_refund_leaves_the_enrolment_withdrawn() {
    let t = boot().await;
    let batch = started_batch(&t).await;
    let e = paid_enrolment(&t, &batch, 1).await;
    hold_sessions(&t, &batch, 3).await;
    let desk = t.desk().await;
    let asked = enrolments::request_withdrawal(t.state(), Some(&desk), e.id, "moved abroad")
        .await
        .unwrap();
    let manager = t.manager_user().await;
    approve(t.state(), asked.approval_request.unwrap(), &manager, "")
        .await
        .unwrap();
    let (refund, _by) = t.cash.refunds()[0].clone();

    t.state()
        .emit(RefundFailed {
            refund: RefundId(1),
            party: refund.party,
            amount: refund.amount,
            currency: "AED".to_owned(),
            source: refund.source.clone(),
            method: PaymentMethod::Gateway,
            kind: RefundFailureKind::GatewayDeclined,
            reason: "card closed".to_owned(),
            attempt: RefundAttemptRef { id: 1, number: 1 },
            from: RefundFailedFrom::Approved,
            outgoing_payment: None,
        })
        .await
        .unwrap();

    let gone = reload(&t, &e).await;
    assert_eq!(gone.status, EnrolmentStatus::Withdrawn);
    assert_eq!(gone.refund_amount, 178_500);
    assert!(gone.settlement.0.done);
    assert_eq!(t.books.notes().len(), 1, "the credit note is not made again");
    assert_eq!(t.cash.refunds().len(), 1);
}

/// A money step that fails (payments refuse the refund) is kept on the
/// enrolment and shown on its page; finishing it later does not repeat the
/// credit note.
#[renox::test]
async fn edu_03_a_step_that_failed_is_repeated_without_the_others() {
    let t = boot().await;
    let batch = started_batch(&t).await;
    let e = paid_enrolment(&t, &batch, 1).await;
    hold_sessions(&t, &batch, 3).await;
    let desk = t.desk().await;
    let asked = enrolments::request_withdrawal(t.state(), Some(&desk), e.id, "moved abroad")
        .await
        .unwrap();
    let manager = t.manager_user().await;
    t.cash.refuse_refunds(true);
    approve(t.state(), asked.approval_request.unwrap(), &manager, "")
        .await
        .unwrap();

    let stuck = reload(&t, &e).await;
    assert_eq!(stuck.status, EnrolmentStatus::Withdrawn, "the state stays");
    assert!(!stuck.settlement.0.done);
    assert!(stuck.settlement.0.problem.is_some());
    assert_eq!(t.books.notes().len(), 1);
    assert!(t.cash.refunds().is_empty());
    let desk_user = t.desk_user().await;
    t.app.acting_as(&desk_user);
    t.app
        .get(&format!("/edtech/enrolments/{}", e.id))
        .await
        .assert_ok()
        .assert_see("Not finished")
        .assert_see("Finish the money steps");

    t.cash.refuse_refunds(false);
    let done = edtech::settlement::settle(t.state(), e.id).await.unwrap();

    assert!(done.settlement.0.done);
    assert_eq!(t.books.notes().len(), 1, "the credit note is not made twice");
    assert_eq!(t.cash.refunds().len(), 1);
}

/// The enrolment page tells the counsellor that the withdrawal waits, and shows
/// the settled money afterwards.
#[renox::test]
async fn edu_03_the_page_shows_the_waiting_withdrawal_and_the_settled_money() {
    let t = boot().await;
    let batch = started_batch(&t).await;
    let e = paid_enrolment(&t, &batch, 1).await;
    hold_sessions(&t, &batch, 3).await;
    let desk_user = t.desk_user().await;
    let desk = t.auth(&desk_user).await;
    let asked = enrolments::request_withdrawal(t.state(), Some(&desk), e.id, "moved abroad")
        .await
        .unwrap();

    t.app.acting_as(&desk_user);
    t.app
        .get(&format!("/edtech/enrolments/{}", e.id))
        .await
        .assert_ok()
        .assert_see("waits for approval");
    let manager = t.manager_user().await;
    approve(t.state(), asked.approval_request.unwrap(), &manager, "")
        .await
        .unwrap();
    t.app
        .get(&format!("/edtech/enrolments/{}", e.id))
        .await
        .assert_ok()
        .assert_see("Withdrawn")
        .assert_see("Settled")
        .assert_see("AED 1,785.00");
}

/// The batch row and the pending approval are about the same record: the
/// request lists under "Needs your action" for the approvers only.
#[renox::test]
async fn edu_03_the_request_is_for_the_approvers_not_the_counsellor() {
    let t = boot().await;
    let batch = started_batch(&t).await;
    let e = paid_enrolment(&t, &batch, 1).await;
    hold_sessions(&t, &batch, 3).await;
    let desk_user = t.desk_user().await;
    let desk = t.auth(&desk_user).await;
    enrolments::request_withdrawal(t.state(), Some(&desk), e.id, "moved abroad")
        .await
        .unwrap();
    let manager = t.manager_user().await;

    let mine = approvals::awaiting(t.db(), &manager).await.unwrap();
    let theirs = approvals::awaiting(t.db(), &desk_user).await.unwrap();

    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].action, "edtech.withdraw");
    assert_eq!(mine[0].subject(), e.subject());
    assert!(theirs.is_empty());
    let _ = Batch::find_or_404(t.db(), batch.id).await.unwrap();
}
