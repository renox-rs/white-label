//! BL-EDU-06 (#441): a card payment for a course fee is charged back. The
//! invoice reopens, the enrolment does not change by itself, and the
//! counsellor (or, when that user left, the counsellors of the campus) gets one
//! task to collect the money again or withdraw the student, with the amount,
//! the payer and both links; the task closes when the invoice is paid again.
//!
//! The invoice book, the payment links, the notifier and the tickets are
//! stand-ins (`support`); `InvoiceReopened` and `InvoicePaid` are emitted as
//! wl-invoicing does after a reversed and a new payment.

mod support;

use edtech::batches::{self, NewBatch};
use edtech::enrolments::{self, NewEnrolment};
use edtech::notices::NOTICE;
use edtech::{Batch, Chargeback, Enrolment, EnrolmentFreeze, EnrolmentStatus};
use renox::auth::permissions::{self, Scope};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use support::{Centre, Messages, PRICE, boot, day, next_weekday};
use wl_kernel::Subject;
use wl_kernel::invoicing::{InvoiceId, InvoiceReopened, InvoiceStatus, ReopenReason};

/// An open batch of 12 seats from next Monday.
async fn open_future(t: &Centre) -> Batch {
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    t.open_batch(&tutor, &room, next_weekday(day(7), 1)).await
}

/// Sara's enrolment in `batch`, made by `by`, paid by card in full: active.
async fn paid_by_card(t: &Centre, batch: &Batch, by: &renox::AuthUser) -> Enrolment {
    let sara = t.person(1, "Sara Ali");
    let e = enrolments::create(
        t.state(),
        by,
        NewEnrolment::course(sara, batch.id, PRICE).terms_accepted(),
    )
    .await
    .expect("the student is enrolled");
    let active = t.pay_first(&e).await;
    assert_eq!(active.status, EnrolmentStatus::Active);
    active
}

/// What the first invoice of `e` came to, in fils.
fn gross_of(t: &Centre, e: &Enrolment) -> i64 {
    let id = e.first_invoice_id.expect("a first invoice");
    t.books
        .all()
        .into_iter()
        .find(|invoice| invoice.id == id)
        .expect("the invoice")
        .gross()
}

async fn reload(t: &Centre, e: &Enrolment) -> Enrolment {
    Enrolment::find_or_404(t.db(), e.id).await.unwrap()
}

/// Given E with its course fee invoice paid by card (AED 2,520.00), when the
/// payment is reversed for a chargeback, the invoice reopens, E stays `active`
/// and unfrozen, and the counsellor has one task with the amount, the payer and
/// both links.
#[renox::test]
async fn edu_06_a_charged_back_fee_gives_the_counsellor_one_task_and_changes_nothing() {
    let t = boot().await;
    let batch = open_future(&t).await;
    let desk_user = t.desk_user().await;
    let desk = t.auth(&desk_user).await;
    let e = paid_by_card(&t, &batch, &desk).await;
    let invoice = e.first_invoice_id.unwrap();
    let gross = gross_of(&t, &e);
    assert_eq!(gross, PRICE);

    t.charge_back(&e, invoice, gross).await;

    let after = reload(&t, &e).await;
    assert_eq!(
        after.status,
        EnrolmentStatus::Active,
        "the enrolment is unchanged"
    );
    assert!(
        EnrolmentFreeze::open_of(t.db(), e.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(after.seat_held);
    let tasks = t.tasks.about(&e.subject());
    assert_eq!(tasks.len(), 1, "one task");
    let task = &tasks[0];
    assert!(task.is_open());
    assert_eq!(
        task.assignee_user(),
        Some(desk_user.id),
        "to the enrolment's counsellor"
    );
    assert!(task.ticket.due_at.is_some(), "due in two working days");
    let text = &task.ticket.description;
    assert!(text.contains("AED 2,520.00"), "{text}");
    assert!(text.contains("Sara Ali"), "the payer: {text}");
    assert!(
        text.contains(&format!("/edtech/enrolments/{}/collect", e.id)),
        "{text}"
    );
    assert!(
        text.contains(&format!("/edtech/enrolments/{}", e.id)),
        "{text}"
    );
    assert!(task.ticket.title.contains(&e.number()));
    let rows = Chargeback::outstanding_of(t.db(), e.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].amount, gross);
    assert_eq!(rows[0].reversal_reason.as_deref(), Some("chargeback"));
    // The same event twice makes no second task.
    t.announce_reopened(&e, invoice, gross).await;
    assert_eq!(t.tasks.about(&e.subject()).len(), 1);
    assert!(t.tasks.about(&e.subject())[0].comments.is_empty());
}

/// When the chargeback is won and the invoice is paid again, the task closes.
#[renox::test]
async fn edu_06_the_task_closes_when_the_invoice_is_paid_again() {
    let t = boot().await;
    let batch = open_future(&t).await;
    let desk = t.desk().await;
    let e = paid_by_card(&t, &batch, &desk).await;
    let invoice = e.first_invoice_id.unwrap();
    let gross = gross_of(&t, &e);
    t.charge_back(&e, invoice, gross).await;
    assert_eq!(t.tasks.open().len(), 1);

    t.pay_again(&e, invoice, gross).await;

    assert!(t.tasks.open().is_empty(), "no task is left open");
    let closed = &t.tasks.about(&e.subject())[0];
    assert!(
        closed
            .completed
            .as_deref()
            .is_some_and(|note| note.contains("Paid again"))
    );
    assert!(
        Chargeback::outstanding_of(t.db(), e.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(reload(&t, &e).await.status, EnrolmentStatus::Active);
}

/// A membership cycle invoice reopened (its source is the subscription) makes
/// no task for the counsellor, and neither does an allocation moved.
#[renox::test]
async fn edu_06_a_membership_cycle_invoice_reopened_makes_no_task() {
    let t = boot().await;
    let batch = open_future(&t).await;
    let desk = t.desk().await;
    let e = paid_by_card(&t, &batch, &desk).await;

    let reopen = |source: Subject, reason: ReopenReason| InvoiceReopened {
        invoice: InvoiceId(900),
        party: e.payer(),
        source,
        money_source: Subject::new("payments.payment", 5),
        amount: 31_500,
        balance: 31_500,
        new_status: InvoiceStatus::Issued,
        reason,
        reversal_reason: Some("chargeback".to_owned()),
    };
    t.state()
        .emit(reopen(
            Subject::new("invoicing.subscription", 1),
            ReopenReason::PaymentReversed,
        ))
        .await
        .unwrap();
    t.state()
        .emit(reopen(e.subject(), ReopenReason::AllocationMoved))
        .await
        .unwrap();

    assert!(t.tasks.all().is_empty());
    assert!(
        Chargeback::outstanding_of(t.db(), e.id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// An open batch of campus 3 (the scope of the campus counsellors below).
async fn campus_batch(t: &Centre) -> Batch {
    let campus = Subject::new("test.campus", 3);
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
        .campus(campus),
    )
    .await
    .unwrap();
    batches::open(t.state(), &amal, batch.id).await.unwrap()
}

/// Two counsellors who work at campus 3: their user numbers.
async fn campus_counsellors(t: &Centre) -> Vec<i64> {
    let scope = Scope::new("test.campus", 3);
    permissions::define_role(
        t.db(),
        edtech::permissions::COUNSELLOR_ROLE,
        &[
            edtech::permissions::ENROLMENTS_VIEW,
            edtech::permissions::ENROLMENTS_MANAGE,
        ],
    )
    .await
    .unwrap();
    let mut found = Vec::new();
    for (name, email) in [
        ("Dina D", "dina.d@example.com"),
        ("Eman E", "eman.e@example.com"),
    ] {
        let user = User::register(t.db(), name, email, "password123")
            .await
            .unwrap();
        user.assign_role_in(t.db(), edtech::permissions::COUNSELLOR_ROLE, &scope)
            .await
            .unwrap();
        found.push(user.id);
    }
    found
}

/// The task of the chargeback goes to the first of `campus` and all of them are
/// told, `leaver` is not.
fn assert_campus_task(t: &Centre, e: &Enrolment, campus: &[i64], leaver: i64) {
    let tasks = t.tasks.about(&e.subject());
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].assignee_user(),
        Some(campus[0]),
        "the first counsellor of the campus holds it, not the one who left"
    );
    let told: Vec<_> = t
        .messages
        .of_type(NOTICE)
        .into_iter()
        .filter(|m| Messages::told_user(m, campus[0]))
        .collect();
    assert_eq!(told.len(), 1);
    assert!(
        campus.iter().all(|id| Messages::told_user(&told[0], *id)),
        "all the campus counsellors are told"
    );
    assert!(!Messages::told_user(&told[0], leaver));
}

/// Given E's counsellor C1 who no longer holds the role, the task goes to the
/// counsellors of E's campus (the first holds it, all are told).
#[renox::test]
async fn edu_06_the_counsellors_of_the_campus_get_the_task_when_the_counsellor_left() {
    let t = boot().await;
    let batch = campus_batch(&t).await;
    // C1 enrols the student, then leaves: their role is taken away.
    let c1_user = t
        .user(
            "Cara",
            &[
                edtech::permissions::ENROLMENTS_VIEW,
                edtech::permissions::ENROLMENTS_MANAGE,
            ],
        )
        .await;
    let c1 = t.auth(&c1_user).await;
    let e = paid_by_card(&t, &batch, &c1).await;
    assert_eq!(e.campus_id, Some(3));
    c1_user.remove_role(t.db(), "role_cara").await.unwrap();
    let campus = campus_counsellors(&t).await;

    t.charge_back(&e, e.first_invoice_id.unwrap(), PRICE).await;

    assert_campus_task(&t, &e, &campus, c1_user.id);
}

/// Given E's counsellor C1 deactivated: the employee behind the account has
/// exited (wl-hr no longer lists them), while the user still holds the role
/// with the permission. The task goes to the counsellors of the campus all the
/// same; the same chargeback with C1 still employed goes to C1.
#[renox::test]
async fn edu_06_a_deactivated_counsellor_who_still_holds_the_role_gets_no_task() {
    let t = boot().await;
    let batch = campus_batch(&t).await;
    let c1_user = t
        .user(
            "Cara",
            &[
                edtech::permissions::ENROLMENTS_VIEW,
                edtech::permissions::ENROLMENTS_MANAGE,
            ],
        )
        .await;
    let c1 = t.auth(&c1_user).await;
    let e = paid_by_card(&t, &batch, &c1).await;
    // C1 is an employee of the centre, in post.
    t.people.add(900, "Cara Counsellor");
    let hired = wl_hr::employee::create(
        t.state(),
        wl_hr::employee::NewEmployee::new(wl_kernel::PartyId(900), "counsellor", day(-400))
            .status(wl_hr::employee::EmployeeStatus::Active)
            .user(c1_user.id),
    )
    .await
    .unwrap();
    let campus = campus_counsellors(&t).await;
    let scope = Scope::new("test.campus", 3);
    c1_user
        .assign_role_in(t.db(), edtech::permissions::COUNSELLOR_ROLE, &scope)
        .await
        .unwrap();
    let recipients = edtech::chargebacks::recipients(t.state(), &e)
        .await
        .unwrap();
    assert_eq!(recipients, vec![c1_user.id], "C1 is in post: C1 gets it");

    // C1 is deactivated: the exit settled, the role and the permission remain.
    wl_hr::Employee::query()
        .where_(wl_hr::Employee::ID.eq(hired.id))
        .update(
            t.db(),
            &[("status", &wl_hr::employee::EmployeeStatus::Exited)],
        )
        .await
        .unwrap();
    assert!(
        permissions::users_with_permission_in(
            t.db(),
            edtech::permissions::ENROLMENTS_MANAGE,
            &Scope::global()
        )
        .await
        .unwrap()
        .iter()
        .any(|u| u.id == c1_user.id),
        "the user still holds the permission"
    );

    t.charge_back(&e, e.first_invoice_id.unwrap(), PRICE).await;

    assert_campus_task(&t, &e, &campus, c1_user.id);
}

/// Several reopened invoices of one enrolment: one open task, updated.
#[renox::test]
async fn edu_06_several_reopened_invoices_of_one_enrolment_update_the_one_task() {
    let t = boot().await;
    let batch = open_future(&t).await;
    let desk = t.desk().await;
    let e = paid_by_card(&t, &batch, &desk).await;
    let first = e.first_invoice_id.unwrap();
    let gross = gross_of(&t, &e);
    t.charge_back(&e, first, gross).await;

    // An instalment invoice reopens too.
    t.charge_back(&e, 4_000, 63_000).await;

    let tasks = t.tasks.about(&e.subject());
    assert_eq!(tasks.len(), 1, "one task");
    assert_eq!(tasks[0].comments.len(), 1, "updated, not opened again");
    assert!(
        tasks[0].comments[0].contains("AED 3,150.00"),
        "{:?}",
        tasks[0].comments
    );
    assert_eq!(
        Chargeback::outstanding_of(t.db(), e.id)
            .await
            .unwrap()
            .len(),
        2
    );

    // Paying one leaves the task open, updated; paying the other closes it.
    t.pay_again(&e, first, gross).await;
    assert_eq!(t.tasks.open().len(), 1);
    assert_eq!(t.tasks.about(&e.subject())[0].comments.len(), 2);
    t.pay_again(&e, 4_000, 63_000).await;
    assert!(t.tasks.open().is_empty());
}

/// Withdrawing from the task is the normal `withdraw` with what is actually
/// paid after the reversal: nothing is refunded for what was charged back, and
/// the task closes.
#[renox::test]
async fn edu_06_withdrawing_after_the_chargeback_uses_what_is_actually_paid() {
    let t = boot().await;
    let batch = open_future(&t).await;
    let desk = t.desk().await;
    let e = paid_by_card(&t, &batch, &desk).await;
    let invoice = e.first_invoice_id.unwrap();
    t.charge_back(&e, invoice, gross_of(&t, &e)).await;
    assert_eq!(enrolments::paid_of(t.state(), &e).await.unwrap(), 0);

    let asked = enrolments::request_withdrawal(t.state(), Some(&desk), e.id, "cannot pay")
        .await
        .unwrap();

    assert!(
        asked.approval_request.is_none(),
        "nothing is refunded: no approval"
    );
    assert_eq!(asked.enrolment.status, EnrolmentStatus::Withdrawn);
    assert_eq!(asked.enrolment.refund_amount, 0);
    assert!(t.cash.refunds().is_empty());
    assert!(
        t.tasks.open().is_empty(),
        "the task ends with the withdrawal"
    );
}

/// *Send payment link* makes a link for the money to collect, replaces the
/// first one and sends the path to the payer; the page shows the amount and
/// both ways out; the enrolment is then an active prepaid subject, so the money
/// paid on the link is not refunded as a late payment.
#[renox::test]
async fn edu_06_the_page_sends_a_payment_link_for_the_money_to_collect() {
    let t = boot().await;
    let batch = open_future(&t).await;
    let desk_user = t.desk_user().await;
    let desk = t.auth(&desk_user).await;
    let e = paid_by_card(&t, &batch, &desk).await;
    let gross = gross_of(&t, &e);
    assert!(
        !edtech::lapse::enrolment_active(t.state(), &e.subject())
            .await
            .unwrap(),
        "an active enrolment with nothing to collect takes no prepaid money"
    );
    t.charge_back(&e, e.first_invoice_id.unwrap(), gross).await;
    t.app.acting_as(&desk_user);

    t.app
        .get(&format!("/edtech/enrolments/{}/collect", e.id))
        .await
        .assert_ok()
        .assert_see("AED 2,520.00")
        .assert_see("Sara Ali")
        .assert_see("Send payment link")
        .assert_see("Withdraw the student");
    t.app
        .get(&format!("/edtech/enrolments/{}", e.id))
        .await
        .assert_ok()
        .assert_see("charged back")
        .assert_see(&format!("/edtech/enrolments/{}/collect", e.id));

    let before = t.messages.sent().len();
    t.app
        .htmx()
        .post(
            &format!("/edtech/enrolments/{}/payment-link", e.id),
            &[("x", "y")],
        )
        .await
        .assert_ok();

    let links = t.cash.links();
    let made = links.last().expect("a link was made");
    assert_eq!(made.request.amount, gross);
    assert_eq!(made.request.party, e.payer());
    assert_eq!(made.request.prepaid, e.subject());
    assert_eq!(made.request.created_by, Some(desk_user.id));
    let stored = reload(&t, &e).await;
    assert_eq!(
        stored.payment_link_path.as_deref(),
        Some(format!("/pay/link-{}", made.id).as_str())
    );
    assert!(t.messages.sent().len() > before, "the payer is told");
    assert!(
        edtech::lapse::enrolment_active(t.state(), &e.subject())
            .await
            .unwrap()
    );

    // Without a gateway there is no link and the counsellor is told so.
    t.cash.without_gateway();
    let refused = t
        .app
        .htmx()
        .post(
            &format!("/edtech/enrolments/{}/payment-link", e.id),
            &[("x", "y")],
        )
        .await;
    refused.assert_status(422);
    assert!(refused.text().contains("Online payment is not available"));
}
