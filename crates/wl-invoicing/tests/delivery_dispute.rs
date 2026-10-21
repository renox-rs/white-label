//! BL-INV-06 (#69), `X-11`, 03-N11-a, wl-invoicing §5.10 step 5: a delivery
//! put under review flags the issued invoices that bill its lines, by the
//! system and with the delivery's ticket; its review closing as upheld
//! resolves what it raised; a reversal or a loss leaves the flag and tasks
//! the owner; a draft is not flagged, not issued by the system, and tasks
//! the receivables role. The delivery events are emitted as `wl-delivery`
//! emits them (the real module is in `delivery_real.rs`).

mod support;

use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_invoicing::dispute::{self, Dispute, DisputeVia};
use wl_invoicing::dispute_resolve::{self, ResolveInput};
use wl_invoicing::task::{self, InvoicingTask, TaskKind, TaskStatus};
use wl_invoicing::{Invoice, WlInvoicing, issue};
use wl_kernel::Subject;
use wl_kernel::delivery::{
    DeliveryDisputeClosed, DeliveryDisputed, DeliveryId, DeliveryLost, DeliveryReversed,
    ReviewOutcome,
};
use wl_kernel::invoicing::{
    DisputeBy, DisputeOutcome, DisputeRequest, DraftLine, InvoiceDraft, InvoiceId, InvoiceStatus,
    VatCategory,
};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::tickets::TicketId;

use support::{BLUE_DUNES, Booted, boot, boot_with};

const DELIVERY: DeliveryId = DeliveryId(42);

fn order() -> Subject {
    Subject::new("sales.order", 9)
}

fn line(n: i64) -> Subject {
    Subject::new("sales.order_line", n)
}

/// A draft to Blue Dunes with one zero rated line per `(origin line, price in
/// fils)`.
async fn draft_of(t: &Booted, lines: &[(i64, i64)]) -> Invoice {
    let mut draft = InvoiceDraft::new(BLUE_DUNES, order());
    for (origin, price) in lines {
        draft = draft.line(
            DraftLine::new(
                &format!("Order line {origin}"),
                *price,
                VatCategory::ZeroRated,
            )
            .origin(line(*origin)),
        );
    }
    t.draft_of(draft).await
}

/// The issued invoice of `lines`, issued by `by`.
async fn issued_of(t: &Booted, lines: &[(i64, i64)], by: &User) -> Invoice {
    let draft = draft_of(t, lines).await;
    t.issue(&draft, by).await
}

fn disputed(lines: &[i64], ticket: Option<i64>) -> DeliveryDisputed {
    DeliveryDisputed {
        delivery: DELIVERY,
        source: order(),
        reason: "recipient denies".into(),
        ticket: ticket.map(TicketId),
        lines: lines.iter().map(|n| line(*n)).collect(),
    }
}

fn closed(lines: &[i64], ticket: Option<i64>) -> DeliveryDisputeClosed {
    DeliveryDisputeClosed {
        delivery: DELIVERY,
        source: order(),
        outcome: ReviewOutcome::Upheld,
        ticket: ticket.map(TicketId),
        lines: lines.iter().map(|n| line(*n)).collect(),
    }
}

fn reversed(lines: &[i64]) -> DeliveryReversed {
    DeliveryReversed {
        delivery: DELIVERY,
        source: order(),
        reason: "recipient denies".into(),
        liability: "none".into(),
        proof: None,
        cash_treatment: None,
        approver: None,
        lines: lines.iter().map(|n| line(*n)).collect(),
    }
}

fn lost(lines: &[i64], ticket: Option<i64>) -> DeliveryLost {
    DeliveryLost {
        delivery: DELIVERY,
        source: order(),
        was_under_review: true,
        ticket: ticket.map(TicketId),
        liability: "none".to_owned(),
        run: None,
        lines: lines.iter().map(|n| line(*n)).collect(),
    }
}

async fn disputes_of(t: &Booted, invoice: &Invoice) -> Vec<Dispute> {
    Dispute::query()
        .where_(Dispute::INVOICE_ID.eq(invoice.id))
        .order_by(Dispute::ID)
        .get(t.db())
        .await
        .unwrap()
}

async fn tasks_of(t: &Booted, invoice: &Invoice) -> Vec<InvoicingTask> {
    InvoicingTask::query()
        .where_(InvoicingTask::INVOICE_ID.eq(invoice.id))
        .order_by(InvoicingTask::ID)
        .get(t.db())
        .await
        .unwrap()
}

/// AC1: an `issued` invoice of AED 3,000.00 with delivery lines worth AED
/// 800.00; `DeliveryDisputed` arrives: the invoice is flagged with the
/// reason *delivery disputed*, amount AED 800.00 and the delivery's ticket
/// (no second ticket is opened).
#[renox::test]
async fn a_disputed_delivery_flags_the_invoice_for_its_lines_with_the_deliverys_ticket() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = issued_of(&t, &[(11, 80_000), (12, 220_000)], &a).await;
    assert_eq!(invoice.gross_total, 300_000);

    t.state().emit(disputed(&[11], Some(7))).await.unwrap();

    let after = t.reload(&invoice).await;
    assert!(after.disputed);
    assert_eq!(after.status, InvoiceStatus::Issued);
    let all = disputes_of(&t, &invoice).await;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].reason, "delivery disputed");
    assert_eq!(all[0].amount, 80_000);
    assert_eq!(all[0].positions(), [1]);
    assert_eq!(all[0].ticket_id, Some(7));
    assert_eq!(all[0].raised_via, DisputeVia::System);
    assert_eq!(all[0].source(), Some(DELIVERY.subject()));
    assert!(
        t.tickets.opened().is_empty(),
        "the delivery's ticket is used"
    );
    let events = t.disputed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].amount, 80_000);
    assert_eq!(events[0].ticket, Some(TicketId(7)));
    assert_eq!(events[0].raised_by, DisputeBy::System);
}

/// Without a ticket from the delivery (ticketing was off there) the dispute
/// opens its own billing dispute ticket as any dispute does.
#[renox::test]
async fn a_delivery_without_a_ticket_leaves_the_dispute_its_own_ticket() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = issued_of(&t, &[(11, 80_000)], &a).await;

    t.state().emit(disputed(&[11], None)).await.unwrap();

    let all = disputes_of(&t, &invoice).await;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].ticket_id, Some(1));
    assert_eq!(t.tickets.opened().len(), 1);
}

/// AC2: `DeliveryDisputeClosed` (*upheld*) resolves the dispute `upheld`; the
/// flag is cleared and the delivery's ticket is resolved.
#[renox::test]
async fn closing_the_review_as_upheld_resolves_the_dispute_the_delivery_raised() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = issued_of(&t, &[(11, 80_000), (12, 220_000)], &a).await;
    t.state().emit(disputed(&[11], Some(7))).await.unwrap();

    t.state().emit(closed(&[11], Some(7))).await.unwrap();

    let after = t.reload(&invoice).await;
    assert!(!after.disputed);
    let all = disputes_of(&t, &invoice).await;
    assert_eq!(all[0].outcome, Some(DisputeOutcome::Upheld));
    assert!(all[0].resolved_at.is_some());
    assert_eq!(t.resolved.all().len(), 1);
    assert_eq!(t.resolved.all()[0].outcome, DisputeOutcome::Upheld);
    assert_eq!(t.tickets.resolved().len(), 1);
    assert_eq!(t.tickets.resolved()[0].0, TicketId(7));
}

/// AC3: `DeliveryReversed` arrives instead: the flag stays and the owner gets
/// a task (and a notice); nothing is resolved.
#[renox::test]
async fn a_reversed_delivery_leaves_the_flag_and_gives_the_owner_a_task() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = issued_of(&t, &[(11, 80_000), (12, 220_000)], &a).await;
    t.state().emit(disputed(&[11], Some(7))).await.unwrap();

    t.state().emit(reversed(&[11])).await.unwrap();

    assert!(t.reload(&invoice).await.disputed);
    assert!(disputes_of(&t, &invoice).await[0].is_open());
    assert!(t.resolved.all().is_empty());
    let tasks = tasks_of(&t, &invoice).await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].kind, TaskKind::DeliveryReversed);
    assert_eq!(tasks[0].status, TaskStatus::Open);
    assert_eq!(tasks[0].role, "owner");
    assert_eq!(tasks[0].amount, 80_000);
    assert_eq!(tasks[0].source(), DELIVERY.subject());
    assert!(
        tasks[0].message.contains("credited"),
        "{}",
        tasks[0].message
    );
    let notices = t.notices.of_type(task::REVERSAL_TASK);
    assert_eq!(notices.len(), 1);
    assert!(matches!(
        notices[0].recipients[..],
        [NotifyRecipient::User { id }] if id == a.id
    ));
    // The same event again opens no second task.
    t.state().emit(reversed(&[11])).await.unwrap();
    assert_eq!(tasks_of(&t, &invoice).await.len(), 1);
}

/// AC3 (lost): `DeliveryLost` leaves the flag the same way and tasks the owner.
#[renox::test]
async fn a_lost_delivery_leaves_the_flag_and_gives_the_owner_a_task() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = issued_of(&t, &[(11, 80_000), (12, 220_000)], &a).await;
    t.state().emit(disputed(&[11], Some(7))).await.unwrap();

    t.state().emit(lost(&[11], Some(7))).await.unwrap();

    assert!(t.reload(&invoice).await.disputed);
    let tasks = tasks_of(&t, &invoice).await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].kind, TaskKind::DeliveryLost);
    assert_eq!(tasks[0].role, "owner");
}

/// AC4: the invoice is still a draft: no flag is set, the system does not
/// issue it while the review is open, and the receivables role gets the task
/// *delivery disputed before issue*; closing the review lets the system
/// issue it and closes the task.
#[renox::test]
async fn a_draft_is_not_flagged_not_auto_issued_and_tasks_the_receivables_role() {
    let t = boot_with(WlInvoicing::new().auto_issue_source("sales.order")).await;
    let a = t.user_a().await;
    let draft = draft_of(&t, &[(11, 80_000), (12, 220_000)]).await;

    t.state().emit(disputed(&[11], Some(7))).await.unwrap();

    let after = t.reload(&draft).await;
    assert!(!after.disputed);
    assert_eq!(after.status, InvoiceStatus::Draft);
    assert!(disputes_of(&t, &draft).await.is_empty());
    let tasks = tasks_of(&t, &draft).await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].kind, TaskKind::DeliveryDisputedBeforeIssue);
    assert_eq!(tasks[0].role, "receivables");
    let notices = t.notices.of_type(task::REVERSAL_TASK);
    assert_eq!(notices.len(), 1);
    assert!(matches!(
        notices[0].recipients[..],
        [NotifyRecipient::User { id }] if id == a.id
    ));

    let refused = issue::issue_by_system(t.state(), &draft).await;
    assert!(
        matches!(&refused, Err(TransitionError::Guard(m)) if m.contains("under review")),
        "{refused:?}"
    );
    assert_eq!(t.reload(&draft).await.status, InvoiceStatus::Draft);

    t.state().emit(closed(&[11], Some(7))).await.unwrap();
    assert_eq!(
        tasks_of(&t, &draft).await[0].status,
        TaskStatus::Done,
        "the review is over"
    );
    issue::issue_by_system(t.state(), &draft).await.unwrap();
    assert_eq!(t.reload(&draft).await.status, InvoiceStatus::Issued);
}

/// A draft that bills lines of another delivery is not held.
#[renox::test]
async fn a_draft_of_another_delivery_is_still_issued_by_the_system() {
    let t = boot_with(WlInvoicing::new().auto_issue_source("sales.order")).await;
    let _a = t.user_a().await;
    let other = draft_of(&t, &[(21, 50_000)]).await;

    t.state().emit(disputed(&[11], Some(7))).await.unwrap();

    issue::issue_by_system(t.state(), &other).await.unwrap();
    assert_eq!(t.reload(&other).await.status, InvoiceStatus::Issued);
}

/// AC5: the same invoice is `partially_paid` with a balance of AED 500.00:
/// the flag is set with the disputed amount AED 500.00, capped at the balance.
#[renox::test]
async fn the_disputed_amount_is_capped_at_the_balance_of_a_partly_paid_invoice() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = issued_of(&t, &[(11, 80_000), (12, 220_000)], &a).await;
    t.pay(&invoice, 1, 250_000).await;
    let paid = t.reload(&invoice).await;
    assert_eq!(paid.status, InvoiceStatus::PartiallyPaid);
    assert_eq!(paid.balance, 50_000);

    t.state().emit(disputed(&[11], Some(7))).await.unwrap();

    let all = disputes_of(&t, &invoice).await;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].amount, 50_000);
    assert!(t.reload(&invoice).await.disputed);
}

/// AC6: delivery lines of AED 800.00 billed AED 300.00 on invoice A and AED
/// 500.00 on invoice B: both are flagged, A for AED 300.00 and B for AED
/// 500.00.
#[renox::test]
async fn every_invoice_with_the_deliverys_lines_is_flagged_for_its_own_amount() {
    let t = boot().await;
    let a = t.user_a().await;
    let first = issued_of(&t, &[(11, 30_000), (15, 100_000)], &a).await;
    let second = issued_of(&t, &[(12, 50_000)], &a).await;

    t.state().emit(disputed(&[11, 12], Some(7))).await.unwrap();

    let on_first = disputes_of(&t, &first).await;
    let on_second = disputes_of(&t, &second).await;
    assert_eq!((on_first.len(), on_first[0].amount), (1, 30_000));
    assert_eq!((on_second.len(), on_second[0].amount), (1, 50_000));
    assert!(t.reload(&first).await.disputed && t.reload(&second).await.disputed);
}

/// AC7: an invoice already disputed by a user: `DeliveryDisputed` sets no
/// second flag and the delivery's ticket is linked with a note;
/// `DeliveryDisputeClosed` (*upheld*) leaves that dispute open.
#[renox::test]
async fn an_invoice_already_disputed_by_a_user_is_not_flagged_again() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = issued_of(&t, &[(11, 80_000), (12, 220_000)], &a).await;
    let own = dispute::raise(
        t.state(),
        DisputeRequest::new(
            InvoiceId(invoice.id),
            "wrong price",
            DisputeBy::User { id: a.id },
        )
        .amount(10_000),
    )
    .await
    .unwrap();
    let user_ticket = TicketId(1);

    t.state().emit(disputed(&[11], Some(7))).await.unwrap();

    let all = disputes_of(&t, &invoice).await;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, own);
    assert_eq!(all[0].amount, 10_000, "the disputed amount is unchanged");
    let comments = t.tickets.comments();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].0, user_ticket);
    assert!(comments[0].1.text.contains("delivery.delivery:42"));
    assert!(comments[0].1.text.contains('7'));

    t.state().emit(closed(&[11], Some(7))).await.unwrap();

    assert!(t.reload(&invoice).await.disputed);
    assert!(disputes_of(&t, &invoice).await[0].is_open());
    assert!(t.resolved.all().is_empty());
}

/// Received twice, the event leaves the flag it set as it is: one dispute,
/// and no note on the ticket of a dispute the delivery itself raised.
#[renox::test]
async fn the_same_event_twice_flags_the_invoice_once() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = issued_of(&t, &[(11, 80_000)], &a).await;

    t.state().emit(disputed(&[11], Some(7))).await.unwrap();
    t.state().emit(disputed(&[11], Some(7))).await.unwrap();

    assert_eq!(disputes_of(&t, &invoice).await.len(), 1);
    assert!(t.tickets.comments().is_empty());
}

/// `DeliveryDisputeClosed` for an invoice whose dispute a user already
/// resolved: nothing happens.
#[renox::test]
async fn a_dispute_a_user_already_resolved_is_left_alone() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = issued_of(&t, &[(11, 80_000)], &a).await;
    t.state().emit(disputed(&[11], Some(7))).await.unwrap();
    dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::upheld("Checked by the owner"),
        DisputeBy::User { id: a.id },
    )
    .await
    .unwrap();
    assert_eq!(t.resolved.all().len(), 1);

    t.state().emit(closed(&[11], Some(7))).await.unwrap();

    assert_eq!(t.resolved.all().len(), 1);
    let all = disputes_of(&t, &invoice).await;
    assert_eq!(all[0].resolved_by_user_id, Some(a.id));
}

/// A paid invoice has no dispute to raise: the event leaves it alone and
/// does not fail.
#[renox::test]
async fn a_paid_invoice_is_not_flagged() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = issued_of(&t, &[(11, 80_000)], &a).await;
    t.pay(&invoice, 1, 80_000).await;
    assert_eq!(t.reload(&invoice).await.status, InvoiceStatus::Paid);

    t.state().emit(disputed(&[11], Some(7))).await.unwrap();

    assert!(disputes_of(&t, &invoice).await.is_empty());
    assert!(!t.reload(&invoice).await.disputed);
}
