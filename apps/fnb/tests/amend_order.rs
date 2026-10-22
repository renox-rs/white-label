//! BL-FNB-05 (#369), 06 §4.1 and §4.2 rows *An item runs out after the order
//! was accepted*, §7.1 *Order* rows `amend` and `cancel_line`, `SAL.ACL.05`:
//! when an item runs out after acceptance the cashier calls the customer and
//! substitutes it (`amend`: the price difference refunded, or charged by a
//! payment link), drops the line (`cancel_line`: the line refunded) or
//! cancels the order. The kitchen tickets follow at once. Aggregator orders
//! are changed on the aggregator's tablet: only a cancelled line is recorded,
//! with no refund, flagged for the payout reconciliation. The modules are the
//! real wl-sales, wl-payments (with a fake gateway for the link),
//! wl-inventory and wl-delivery.

mod support;

use std::sync::atomic::{AtomicI64, Ordering};

use fnb::{ChangeNote, ExpectedRefund, FnbOrder, FnbOrderStatus, KitchenTicket, TicketStatus};
use renox::prelude::*;
use wl_inventory::MovementType;
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::sales::SALES_ORDER_SUBJECT_TYPE;
use wl_payments::refund::Refund;
use wl_sales::order_change::{OrderSettlement, SETTLE_LINK};
use wl_sales::{OrderRevision, SalesOrder};

use support::{World, boot_gateway, boot_own_riders};

/// Salmon (a shawarma plate here) at AED 55.00, direct ORD-701.
const SALMON: [(&str, &str, &str); 1] = [("shawarma-plate", "1", "55.00")];

/// A main at AED 30.00 and fries (a falafel wrap here) at AED 12.00.
const MAIN_AND_FRIES: [(&str, &str, &str); 2] = [
    ("shawarma-plate", "1", "30.00"),
    ("falafel-wrap", "1", "12.00"),
];

/// Two lines at AED 25.00 and AED 18.00, an aggregator's ORD-702.
const TWO_LINES: [(&str, &str, &str); 2] = [
    ("shawarma-plate", "1", "25.00"),
    ("hummus-bowl", "1", "18.00"),
];

static CUSTOMERS: AtomicI64 = AtomicI64::new(1);

async fn customer(w: &World) -> wl_kernel::PartyId {
    let n = CUSTOMERS.fetch_add(1, Ordering::SeqCst);
    w.customer(&format!("Mariam Al Hosani {n}")).await
}

/// The refunds asked for `order`'s sales order, oldest first.
async fn refunds_of(w: &World, order: &FnbOrder) -> Vec<Refund> {
    Refund::query()
        .where_(Refund::SOURCE_ID.eq(order.sales_order_id))
        .where_(Refund::SOURCE_KIND.eq("order"))
        .order_by(Refund::ID)
        .get(w.db())
        .await
        .unwrap()
}

/// The wastage movements of `item`.
async fn wastage_of(w: &World, item: &wl_inventory::Item) -> Vec<wl_inventory::StockMovement> {
    w.movements_of(item)
        .await
        .into_iter()
        .filter(|m| m.movement_type == MovementType::Wastage)
        .collect()
}

/// The tickets of `order`, oldest first.
async fn tickets_of(w: &World, order: &FnbOrder) -> Vec<KitchenTicket> {
    KitchenTicket::of_order(w.db(), order.id).await.unwrap()
}

/// The transitions of `order` as the history lists them.
async fn history_of(w: &World, order: &FnbOrder) -> Vec<String> {
    renox::state_machine::history(w.db(), FnbOrder::TABLE, order.id)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.transition)
        .collect()
}

/// The amend form of the cashier: `line` is changed or replaced by `item`.
fn amend_form<'a>(line: &'a str, item: &'a str, price: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        ("line", line),
        ("new_item", item),
        ("quantity", ""),
        ("price", price),
        ("reason", "The salmon ran out"),
        ("customer_agreed", "on"),
        ("call_time", "18:45"),
    ]
}

/// The cancel-line form of the cashier.
fn line_form<'a>(line: &'a str, outcome: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        ("line", line),
        ("reason", "item_unavailable"),
        ("note", ""),
        ("outcome", outcome),
    ]
}

/// The cook starts the ticket `ticket`.
async fn start(w: &World, ticket: &KitchenTicket) {
    w.post(
        &w.cook,
        &format!("/tickets/{}/start", ticket.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);
}

/// AC1: given direct ORD-701 `accepted` with Salmon (AED 55.00, queued), when
/// the cashier amends it to Chicken (AED 45.00) with "customer agreed",
/// revision 2 exists, AED 10.00 is refunded, and the ticket shows Chicken.
/// (A substitute is taxed as the line it replaces, so the lines are
/// zero-rated here as everywhere in the stories.)
#[renox::test]
async fn substituting_an_item_refunds_the_difference_and_the_ticket_shows_the_substitute() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let accepted = w.accept(&w.place(c, "online", &SALMON).await).await;
    let salmon = tickets_of(&w, &accepted).await.remove(0);
    assert_eq!(salmon.status, TicketStatus::Queued);

    let line = salmon.sale_line_id.to_string();
    w.post(
        &w.cashier,
        &format!("/orders/{}/amend", accepted.id),
        &amend_form(&line, "falafel-wrap", "45.00"),
    )
    .await
    .assert_status(303);

    // Revision 2 is in force, revision 1 is kept.
    let sales = SalesOrder::find_or_404(w.db(), accepted.sales_order_id).await.unwrap();
    assert_eq!(sales.revision, 2);
    assert_eq!(sales.gross_total, 4_500);
    let kept = OrderRevision::query()
        .where_(OrderRevision::ORDER_ID.eq(sales.id))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].revision, 1);
    // AED 10.00 is asked of wl-payments.
    let refunds = refunds_of(&w, &accepted).await;
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].amount, 1_000);
    // The ticket of the salmon is gone; the substitute is on the screen.
    let tickets = tickets_of(&w, &accepted).await;
    assert_eq!(tickets.len(), 2);
    assert_eq!(tickets[0].status, TicketStatus::Voided);
    assert_eq!(tickets[1].status, TicketStatus::Queued);
    assert_eq!(tickets[1].description, "Falafel wrap");
    assert_eq!(tickets[1].quantity_milli, 1_000);
    let board = w.get(&w.cook, "/kitchen").await;
    board.assert_ok();
    assert!(board.text().contains("Falafel wrap"));
    assert!(!board.text().contains(&salmon.description));
    // Who, why and when of the call are kept; the order is still accepted.
    let notes = ChangeNote::of_order(w.db(), accepted.id).await.unwrap();
    assert_eq!(notes.len(), 1);
    assert!(notes[0].customer_agreed);
    assert_eq!(notes[0].call_time.as_deref(), Some("18:45"));
    assert_eq!(notes[0].requested_by, Some(w.cashier.id));
    assert_eq!(notes[0].status, "done");
    assert_eq!(w.reload(&accepted).await.status, FnbOrderStatus::Accepted);
    assert!(history_of(&w, &accepted).await.contains(&"amend".to_owned()));
}

/// AC2: amending to Steak (AED 70.00) sends a AED 15.00 payment link; the
/// ticket updates at once, without waiting for the link to be paid.
#[renox::test]
async fn a_dearer_substitute_is_charged_through_a_payment_link_and_the_kitchen_does_not_wait() {
    let w = boot_gateway().await;
    let c = customer(&w).await;
    let accepted = w.accept(&w.place(c, "online", &SALMON).await).await;
    let salmon = tickets_of(&w, &accepted).await.remove(0);

    let line = salmon.sale_line_id.to_string();
    w.post(
        &w.cashier,
        &format!("/orders/{}/amend", accepted.id),
        &amend_form(&line, "kunafa-tray", "70.00"),
    )
    .await
    .assert_status(303);

    let links = OrderSettlement::query()
        .where_(OrderSettlement::ORDER_ID.eq(accepted.sales_order_id))
        .where_(OrderSettlement::KIND.eq(SETTLE_LINK))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].amount, 1_500);
    assert_eq!(links[0].status, "open");
    // Nothing is refunded; the ticket of the dearer item is queued now.
    assert!(refunds_of(&w, &accepted).await.is_empty());
    let tickets = tickets_of(&w, &accepted).await;
    assert_eq!(tickets[0].status, TicketStatus::Voided);
    assert_eq!(tickets[1].status, TicketStatus::Queued);
    assert_eq!(tickets[1].description, "Kunafa tray");
}

/// AC3: `cancel_line` on Fries (AED 12.00, prepaid) refunds AED 12.00 and
/// removes it from the ticket.
#[renox::test]
async fn cancelling_a_paid_line_refunds_it_and_removes_its_ticket() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let accepted = w.accept(&w.place(c, "online", &MAIN_AND_FRIES).await).await;
    let tickets = tickets_of(&w, &accepted).await;
    let fries = &tickets[1];

    let line = fries.sale_line_id.to_string();
    w.post(
        &w.cashier,
        &format!("/orders/{}/line-cancel", accepted.id),
        &line_form(&line, ""),
    )
    .await
    .assert_status(303);

    let refunds = refunds_of(&w, &accepted).await;
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].amount, 1_200);
    let after = tickets_of(&w, &accepted).await;
    assert_eq!(after[0].status, TicketStatus::Queued);
    assert_eq!(after[1].status, TicketStatus::Voided);
    let notes = ChangeNote::of_order(w.db(), accepted.id).await.unwrap();
    assert_eq!(notes[0].kind, "cancel_line");
    assert_eq!(notes[0].sale_line_id, Some(fries.sale_line_id));
    assert!(history_of(&w, &accepted).await.contains(&"cancel_line".to_owned()));
    // The order is still accepted and the other line is cooked as before.
    assert_eq!(w.reload(&accepted).await.status, FnbOrderStatus::Accepted);
}

/// AC4: `amend` on aggregator order ORD-702 is refused; `cancel_line` with
/// "item unavailable" is recorded with no refund and the line is flagged for
/// payout reconciliation.
#[renox::test]
async fn an_aggregator_order_is_not_amended_here_and_a_cancelled_line_is_flagged_for_the_payout() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let order = w.place_via("aggregator_a", c, "on_account", &TWO_LINES).await;
    assert_eq!(order.status, FnbOrderStatus::Accepted);
    let tickets = tickets_of(&w, &order).await;
    let line = tickets[0].sale_line_id.to_string();

    // Amend is refused, and the page does not offer it.
    let refused = w
        .post(
            &w.cashier,
            &format!("/orders/{}/amend", order.id),
            &amend_form(&line, "falafel-wrap", "45.00"),
        )
        .await;
    refused.assert_status(422);
    assert!(refused.text().contains("tablet"));
    let page = w.get(&w.manager, &format!("/orders/{}", order.id)).await;
    page.assert_ok();
    assert!(!page.text().contains("Replace it with"));
    assert_eq!(
        SalesOrder::find_or_404(w.db(), order.sales_order_id).await.unwrap().revision,
        1
    );

    // The cancelled line is recorded: no refund, an expected refund of the
    // aggregator.
    w.post(
        &w.cashier,
        &format!("/orders/{}/line-cancel", order.id),
        &line_form(&line, ""),
    )
    .await
    .assert_status(303);
    assert!(refunds_of(&w, &order).await.is_empty());
    let expected = ExpectedRefund::of_order(w.db(), order.id).await.unwrap();
    assert_eq!(expected.len(), 1);
    assert_eq!(expected[0].sale_line_id, Some(tickets[0].sale_line_id));
    assert_eq!(expected[0].amount, 2_500);
    assert!(expected[0].reason.contains("not available"));
    assert_eq!(tickets_of(&w, &order).await[0].status, TicketStatus::Voided);
    let page = w.get(&w.manager, &format!("/orders/{}", order.id)).await;
    assert!(page.text().contains("Refunds expected from the aggregator"));
}

/// AC5: cancelling the only open line is refused (cancel the order instead).
#[renox::test]
async fn the_only_open_line_cannot_be_cancelled() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let accepted = w.accept(&w.place(c, "online", &SALMON).await).await;
    let ticket = tickets_of(&w, &accepted).await.remove(0);
    let line = ticket.sale_line_id.to_string();

    let answer = w
        .post(
            &w.cashier,
            &format!("/orders/{}/line-cancel", accepted.id),
            &line_form(&line, ""),
        )
        .await;
    answer.assert_status(422);
    assert!(answer.text().contains("last open line"));
    assert_eq!(tickets_of(&w, &accepted).await[0].status, TicketStatus::Queued);
    assert!(refunds_of(&w, &accepted).await.is_empty());
    // The page says so instead of offering the button.
    let page = w.get(&w.cashier, &format!("/orders/{}", accepted.id)).await;
    assert!(page.text().contains("last open line"));
}

/// A placed order is amended before the kitchen has it; its tickets are made
/// from the new lines when it is accepted.
#[renox::test]
async fn a_placed_order_is_amended_before_it_is_accepted() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let placed = w.place(c, "online", &SALMON).await;
    let line = wl_sales::SaleLine::of(
        w.db(),
        &Subject::new(SALES_ORDER_SUBJECT_TYPE, placed.sales_order_id),
    )
    .await
    .unwrap()
    .remove(0);

    w.post(
        &w.cashier,
        &format!("/orders/{}/amend", placed.id),
        &amend_form(&line.id.to_string(), "falafel-wrap", "45.00"),
    )
    .await
    .assert_status(303);
    assert!(tickets_of(&w, &placed).await.is_empty());
    assert!(history_of(&w, &placed).await.contains(&"amend_placed".to_owned()));

    let accepted = w.accept(&placed).await;
    let tickets = tickets_of(&w, &accepted).await;
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].description, "Falafel wrap");
}

/// The form is checked: the customer must have agreed, the call time is a
/// time, a reason is given; a cook cannot amend or cancel a line.
#[renox::test]
async fn an_amendment_needs_the_agreement_the_time_of_the_call_and_the_permission() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let accepted = w.accept(&w.place(c, "online", &MAIN_AND_FRIES).await).await;
    let tickets = tickets_of(&w, &accepted).await;
    let line = tickets[0].sale_line_id.to_string();
    let path = format!("/orders/{}/amend", accepted.id);

    let mut not_agreed = amend_form(&line, "", "20.00");
    not_agreed.retain(|(name, _)| *name != "customer_agreed");
    w.post(&w.cashier, &path, &not_agreed).await.assert_status(422);
    let mut bad_time = amend_form(&line, "", "20.00");
    bad_time.retain(|(name, _)| *name != "call_time");
    bad_time.push(("call_time", "evening"));
    w.post(&w.cashier, &path, &bad_time).await.assert_status(422);
    let mut no_reason = amend_form(&line, "", "20.00");
    no_reason.retain(|(name, _)| *name != "reason");
    no_reason.push(("reason", " "));
    w.post(&w.cashier, &path, &no_reason).await.assert_status(422);
    w.post(&w.cook, &path, &amend_form(&line, "", "20.00"))
        .await
        .assert_forbidden();
    w.post(
        &w.cook,
        &format!("/orders/{}/line-cancel", accepted.id),
        &line_form(&line, ""),
    )
    .await
    .assert_forbidden();
    assert_eq!(
        SalesOrder::find_or_404(w.db(), accepted.sales_order_id).await.unwrap().revision,
        1
    );

    // A price change of the same line is an amendment too (AED 30 to 28).
    w.post(&w.cashier, &path, &amend_form(&line, "", "28.00"))
        .await
        .assert_status(303);
    assert_eq!(
        SalesOrder::find_or_404(w.db(), accepted.sales_order_id).await.unwrap().gross_total,
        2_800 + 1_200
    );
}

/// Once the kitchen started, the order is `preparing`: it is no longer
/// amended (only `placed` and `accepted` orders are), and a line whose ticket
/// was bumped is not cancelled.
#[renox::test]
async fn a_started_order_is_not_amended_and_a_bumped_line_is_not_cancelled() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let accepted = w.accept(&w.place(c, "online", &MAIN_AND_FRIES).await).await;
    let tickets = tickets_of(&w, &accepted).await;
    let (main, fries) = (&tickets[0], &tickets[1]);
    start(&w, main).await;
    let main_line = main.sale_line_id.to_string();
    let fries_line = fries.sale_line_id.to_string();

    // The kitchen has it: no amendment, whichever line.
    for line in [&main_line, &fries_line] {
        let refused = w
            .post(
                &w.cashier,
                &format!("/orders/{}/amend", accepted.id),
                &amend_form(line, "falafel-wrap", "40.00"),
            )
            .await;
        refused.assert_status(422);
        assert!(refused.text().contains("placed or accepted"));
    }
    assert_eq!(
        SalesOrder::find_or_404(w.db(), accepted.sales_order_id).await.unwrap().revision,
        1
    );

    // A bumped line is not cancelled.
    w.post(
        &w.cook,
        &format!("/tickets/{}/bump", main.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);
    let bumped = w
        .post(
            &w.cashier,
            &format!("/orders/{}/line-cancel", accepted.id),
            &line_form(&main_line, "scrap"),
        )
        .await;
    bumped.assert_status(422);
    assert!(bumped.text().contains("bumped"));
    assert_eq!(tickets_of(&w, &accepted).await[0].status, TicketStatus::Done);
}

/// `SAL.ACL.05`: a started line needs an outcome and a manager; with *scrap*
/// the food is posted as wastage and its ticket is voided; the order whose
/// other tickets are all bumped is `ready` then.
#[renox::test]
async fn cancelling_a_started_line_needs_an_outcome_and_a_manager_and_scraps_the_food() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let accepted = w.accept(&w.place(c, "online", &MAIN_AND_FRIES).await).await;
    let tickets = tickets_of(&w, &accepted).await;
    let (main, fries) = (&tickets[0], &tickets[1]);
    // The fries are cooked and bumped; the main is cooking.
    start(&w, fries).await;
    w.post(&w.cook, &format!("/tickets/{}/bump", fries.id), &[("x", "")])
        .await
        .assert_status(303);
    start(&w, main).await;
    assert_eq!(w.reload(&accepted).await.status, FnbOrderStatus::Preparing);
    let line = main.sale_line_id.to_string();
    let path = format!("/orders/{}/line-cancel", accepted.id);

    // No outcome: refused.
    w.post(&w.cashier, &path, &line_form(&line, ""))
        .await
        .assert_status(422);
    // With scrap it waits for a manager.
    w.post(&w.cashier, &path, &line_form(&line, "scrap"))
        .await
        .assert_status(303);
    let request = ApprovalRequest::pending_about(
        w.db(),
        &Subject::new(SALES_ORDER_SUBJECT_TYPE, accepted.sales_order_id),
    )
    .await
    .unwrap()
    .remove(0);
    assert_eq!(request.action, "sales.order_cancel_line");
    assert!(approvals::approve(w.state(), request.id, &w.cashier, "").await.is_err());
    assert!(wastage_of(&w, &w.chicken).await.is_empty());

    approvals::approve(w.state(), request.id, &w.manager, "")
        .await
        .unwrap();

    // The main's food is wastage, its ticket is voided, the line is refunded.
    assert_eq!(wastage_of(&w, &w.chicken).await.len(), 1);
    let after = tickets_of(&w, &accepted).await;
    assert_eq!(after[0].status, TicketStatus::Voided);
    assert_eq!(after[1].status, TicketStatus::Done);
    assert_eq!(refunds_of(&w, &accepted).await[0].amount, 3_000);
    // Nothing is left to cook, so the order is ready.
    assert_eq!(w.reload(&accepted).await.status, FnbOrderStatus::Ready);
}

/// A line that is cancelled is not paid for on the doorstep: the delivery of a
/// cash order collects what is still open, and the order's total follows.
#[renox::test]
async fn the_delivery_of_a_cash_order_collects_only_what_is_still_open() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let accepted = w
        .accept(&w.place(c, "cash_on_delivery", &MAIN_AND_FRIES).await)
        .await;
    let fries = tickets_of(&w, &accepted).await.remove(1);

    w.post(
        &w.cashier,
        &format!("/orders/{}/line-cancel", accepted.id),
        &line_form(&fries.sale_line_id.to_string(), ""),
    )
    .await
    .assert_status(303);
    assert_eq!(w.reload(&accepted).await.total, 3_000);

    // The cook works the tickets still on the screen (the voided one is gone).
    for ticket in tickets_of(&w, &accepted).await {
        if ticket.status != TicketStatus::Queued {
            continue;
        }
        start(&w, &ticket).await;
        w.post(&w.cook, &format!("/tickets/{}/bump", ticket.id), &[("x", "")])
            .await
            .assert_status(303);
    }
    let ready = w.reload(&accepted).await;
    assert_eq!(ready.status, FnbOrderStatus::Ready);
    let delivery = w.delivery_of(&ready).await;
    assert_eq!(delivery.amount_to_collect, 3_000);
}
