//! BL-FNB-10 (#374), 06 §4.3 rows *Aggregator refunds a customer and deducts
//! it from the payout* and *An order on the statement is missing here*, §7.1
//! *Payout statement* (`match`, `dispute`, `resolve`), `FNB.RUL.10`: on `match`
//! each refund line becomes a sales return of kind *no goods* resolved
//! *refunded by the channel* (no refund payment, no credit note, no approval),
//! with the aggregator's reason as category and a responsible party; an
//! unmapped reason blocks `match` until the accountant sets the party; a
//! refund the aggregator takes back runs `reverse`; a missing order is entered
//! as a late entry and `match` passes; one return per refund line, however
//! often the statement is matched. The modules are the real wl-sales,
//! wl-payments, wl-inventory and wl-delivery.

mod support;

use fnb::order::party;
use fnb::payouts::differences;
use fnb::{PayoutLine, PayoutStatus, ReasonMap};
use renox::prelude::*;
use renox::testing::TestResponse;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::documents::dubai_text;
use wl_payments::refund::Refund;
use wl_sales::{ReturnKind, ReturnResolution, ReturnStatus, SalesReturn};

use support::payouts::{AGGREGATOR, completed, file, import, reload, sales_of, staff, statement, void};
use support::{World, boot_own_riders};

/// The returns of the sales order of `order`.
async fn returns_of(w: &World, order: &fnb::FnbOrder) -> Vec<SalesReturn> {
    SalesReturn::query()
        .where_(SalesReturn::SALE_ID.eq(order.sales_order_id))
        .order_by(SalesReturn::ID)
        .get(w.db())
        .await
        .unwrap()
}

/// The refunds wl-payments was asked for the sales order of `order`.
async fn payments_of(w: &World, order: &fnb::FnbOrder) -> Vec<Refund> {
    Refund::query()
        .where_(Refund::SOURCE_ID.eq(order.sales_order_id))
        .where_(Refund::SOURCE_KIND.eq("order"))
        .get(w.db())
        .await
        .unwrap()
}

/// The line of `statement` for the aggregator's `number`.
async fn line_of(w: &World, statement: &fnb::PayoutStatement, number: &str) -> PayoutLine {
    PayoutLine::of_statement(w.db(), statement.id)
        .await
        .unwrap()
        .into_iter()
        .find(|l| l.order_number == number)
        .unwrap_or_else(|| panic!("no line for {number}"))
}

/// The accountant matches the statement.
async fn match_it(w: &World, by: &User, statement: &fnb::PayoutStatement) -> TestResponse {
    w.post(by, &format!("/payouts/{}/match", statement.id), &[("x", "")])
        .await
}

/// AC1: given a statement line of AED 25.00 on ORD-1001, reason "Food was
/// cold" mapped to *cold* and *business*, on `match` a *no goods* return of
/// AED 25.00 resolved *refunded by the channel* exists, ORD-1001's amount
/// refunded is AED 25.00, and no payment left the outlet.
#[renox::test]
async fn a_refund_line_becomes_a_return_refunded_by_the_channel_and_no_money_leaves() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    let order = completed(&w, "ORD-1001", "60.00").await;
    import(
        &w,
        &s.accountant,
        "AGG-W41",
        &file(&["ORD-1001,60.00,12.00,0,25.00,23.00,Food was cold"]),
    )
    .await
    .assert_status(303);
    let st = statement(&w, "AGG-W41").await;
    let line = line_of(&w, &st, "ORD-1001").await;
    // The mapping proposes the category and the party before the match.
    assert_eq!(line.category.as_deref(), Some("cold"));
    assert_eq!(line.party.as_deref(), Some(party::BUSINESS));

    match_it(&w, &s.accountant, &st).await.assert_status(303);

    assert_eq!(reload(&w, &st).await.status, PayoutStatus::Matched);
    let returns = returns_of(&w, &order).await;
    assert_eq!(returns.len(), 1);
    let made = &returns[0];
    assert_eq!(made.kind, ReturnKind::NoGoods);
    assert_eq!(made.resolution, Some(ReturnResolution::RefundedByChannel));
    assert_eq!(made.status, ReturnStatus::Resolved);
    assert_eq!(made.gross_value, 2_500);
    assert_eq!(made.complaint_category.as_deref(), Some("cold"));
    assert_eq!(made.responsible_party.as_deref(), Some("business"));
    assert!(made.credit_note_id.is_none(), "no credit note");
    assert!(payments_of(&w, &order).await.is_empty(), "no refund payment");
    assert!(
        ApprovalRequest::pending_about(w.db(), &made.subject())
            .await
            .unwrap()
            .is_empty(),
        "no approval: no money leaves the outlet"
    );
    assert_eq!(sales_of(&w, &order).await.refunded_amount, 2_500);
    assert_eq!(line_of(&w, &st, "ORD-1001").await.return_id, Some(made.id));
    // The page shows the return and the receivable.
    w.get(&s.accountant, &format!("/payouts/{}", st.id))
        .await
        .assert_ok()
        .assert_see("Refunded by the channel");
}

/// AC2: a line with reason "Customer changed mind" (unmapped) blocks `match`
/// until a party is set.
#[renox::test]
async fn an_unmapped_refund_reason_blocks_match_until_the_party_is_set() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    let order = completed(&w, "ORD-1002", "60.00").await;
    import(
        &w,
        &s.accountant,
        "AGG-W42",
        &file(&["ORD-1002,60.00,12.00,0,15.00,33.00,Customer changed mind"]),
    )
    .await
    .assert_status(303);
    let st = statement(&w, "AGG-W42").await;
    let line = line_of(&w, &st, "ORD-1002").await;
    assert!(line.party.is_none(), "nothing proposes a party");

    // Match is refused and nothing is created.
    match_it(&w, &s.accountant, &st).await.assert_status(422);
    assert_eq!(reload(&w, &st).await.status, PayoutStatus::Imported);
    assert!(returns_of(&w, &order).await.is_empty());
    // The page says why Match is not offered.
    w.get(&s.accountant, &format!("/payouts/{}", st.id))
        .await
        .assert_ok()
        .assert_see("no responsible party");

    // A party outside the two is refused; the accountant sets one.
    let path = format!("/payouts/{}/lines/{}/party", st.id, line.id);
    w.post(&s.accountant, &path, &[("party", "rider"), ("category", "other")])
        .await
        .assert_status(422);
    w.post(&s.reader, &path, &[("party", "aggregator"), ("category", "other")])
        .await
        .assert_status(403);
    w.post(&s.accountant, &path, &[("party", "aggregator"), ("category", "other")])
        .await
        .assert_status(303);
    match_it(&w, &s.accountant, &st).await.assert_status(303);

    let returns = returns_of(&w, &order).await;
    assert_eq!(returns.len(), 1);
    assert_eq!(returns[0].responsible_party.as_deref(), Some("aggregator"));
    assert_eq!(returns[0].complaint_category.as_deref(), Some("other"));
    assert_eq!(returns[0].gross_value, 1_500);
    // The reason is remembered for the next statement.
    let map = ReasonMap::find_for(w.db(), AGGREGATOR, "customer changed mind")
        .await
        .unwrap()
        .expect("a mapping was learnt");
    assert_eq!((map.category.as_str(), map.party.as_str()), ("other", "aggregator"));
}

/// AC3: disputing that line and resolving it with "aggregator reverses the
/// refund" marks the return `reversed` with reason *aggregator took the refund
/// back* and ORD-1001's amount refunded back to 0.
#[renox::test]
async fn a_refund_the_aggregator_takes_back_reverses_its_return() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    let order = completed(&w, "ORD-1001", "60.00").await;
    import(
        &w,
        &s.accountant,
        "AGG-W41",
        &file(&["ORD-1001,60.00,12.00,0,25.00,23.00,Food was cold"]),
    )
    .await
    .assert_status(303);
    let st = statement(&w, "AGG-W41").await;
    match_it(&w, &s.accountant, &st).await.assert_status(303);
    let line = line_of(&w, &st, "ORD-1001").await;
    assert_eq!(reload(&w, &st).await.receivable_amount, 2_300);

    // The kitchen did not cause it: dispute with the aggregator.
    let dispute = format!("/payouts/{}/dispute", st.id);
    let field = format!("line_{}", line.id);
    w.post(&s.accountant, &dispute, &[("reason", "")])
        .await
        .assert_status(422);
    w.post(&s.accountant, &dispute, &[("reason", "Rider was at fault"), (field.as_str(), "on")])
        .await
        .assert_status(303);
    let disputed = reload(&w, &st).await;
    assert_eq!(disputed.status, PayoutStatus::Disputed);
    assert!(line_of(&w, &st, "ORD-1001").await.disputed);
    let key = format!("fnb.payout_dispute:{}", st.id);
    assert_eq!(w.tickets.opened_with(&key).len(), 1, "a follow-up task");

    // The aggregator answers: it reverses the refund.
    let reverse = format!("reverse_{}", line.id);
    let resolve = format!("/payouts/{}/resolve", st.id);
    w.post(&s.accountant, &resolve, &[("answer", "")])
        .await
        .assert_status(422);
    w.post(
        &s.accountant,
        &resolve,
        &[
            ("answer", "Aggregator reverses the refund"),
            (reverse.as_str(), "on"),
        ],
    )
    .await
    .assert_status(303);

    let resolved = reload(&w, &st).await;
    assert_eq!(resolved.status, PayoutStatus::Matched);
    assert_eq!(resolved.resolve_note.as_deref(), Some("Aggregator reverses the refund"));
    assert_eq!(resolved.receivable_amount, 4_800, "the refund is paid back");
    let returns = returns_of(&w, &order).await;
    assert_eq!(returns.len(), 1);
    assert_eq!(returns[0].status, ReturnStatus::Reversed);
    assert_eq!(
        returns[0].closing_reason.as_deref(),
        Some("aggregator took the refund back")
    );
    assert_eq!(sales_of(&w, &order).await.refunded_amount, 0);
    assert!(payments_of(&w, &order).await.is_empty());
    assert!(!w.tickets.closed().is_empty(), "the follow-up task closed");
}

/// Re-matching after a dispute never makes a second return: one per refund
/// line.
#[renox::test]
async fn a_statement_disputed_before_it_is_matched_makes_its_return_once() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    let order = completed(&w, "ORD-1001", "60.00").await;
    import(
        &w,
        &s.accountant,
        "AGG-W41",
        &file(&["ORD-1001,60.00,12.00,0,25.00,23.00,Food was cold"]),
    )
    .await
    .assert_status(303);
    let st = statement(&w, "AGG-W41").await;
    let line = line_of(&w, &st, "ORD-1001").await;
    let field = format!("line_{}", line.id);
    w.post(
        &s.accountant,
        &format!("/payouts/{}/dispute", st.id),
        &[("reason", "Commission is wrong"), (field.as_str(), "on")],
    )
    .await
    .assert_status(303);
    assert!(returns_of(&w, &order).await.is_empty(), "nothing is matched yet");

    // The answer matches it: the receivable is booked and the return made.
    w.post(
        &s.accountant,
        &format!("/payouts/{}/resolve", st.id),
        &[("answer", "Commission corrected")],
    )
    .await
    .assert_status(303);
    let after = reload(&w, &st).await;
    assert_eq!(after.status, PayoutStatus::Matched);
    assert_eq!(after.receivable_amount, 2_300);
    assert_eq!(returns_of(&w, &order).await.len(), 1);
    assert_eq!(sales_of(&w, &order).await.refunded_amount, 2_500);

    // Disputing and resolving again changes nothing about the return.
    w.post(
        &s.accountant,
        &format!("/payouts/{}/dispute", st.id),
        &[("reason", "Again"), (field.as_str(), "on")],
    )
    .await
    .assert_status(303);
    w.post(
        &s.accountant,
        &format!("/payouts/{}/resolve", st.id),
        &[("answer", "Same")],
    )
    .await
    .assert_status(303);
    assert_eq!(returns_of(&w, &order).await.len(), 1);
    assert_eq!(sales_of(&w, &order).await.refunded_amount, 2_500);
}

/// AC4: given a statement line for ORD-1099 not recorded, the cashier enters
/// it as a late entry with that number and `match` passes.
#[renox::test]
async fn a_missing_order_is_entered_as_a_late_entry_and_match_passes() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    import(
        &w,
        &s.accountant,
        "AGG-W43",
        &file(&["ORD-1099,42.00,8.40,0,0,33.60,"]),
    )
    .await
    .assert_status(303);
    let st = statement(&w, "AGG-W43").await;
    assert!(line_of(&w, &st, "ORD-1099").await.is_unmatched());
    match_it(&w, &s.accountant, &st).await.assert_status(422);

    // The accountant cannot enter orders; the cashier can, once.
    let now = dubai_text(renox::db::now());
    let (day, time) = now.split_once(' ').unwrap();
    let form = [
        ("aggregator", AGGREGATOR),
        ("order_number", "ORD-1099"),
        ("occurred_on", day),
        ("occurred_time", time),
        ("gross", "42.00"),
        ("reason", "The tablet was offline"),
    ];
    w.post(&s.accountant, "/payouts/late-order", &form)
        .await
        .assert_status(403);
    w.post(&s.late_cashier, "/payouts/late-order", &form)
        .await
        .assert_status(303);
    w.post(&s.late_cashier, "/payouts/late-order", &form)
        .await
        .assert_status(422);

    let line = line_of(&w, &st, "ORD-1099").await;
    assert_eq!(line.match_state, "matched");
    assert!(line.late_order_id.is_some());
    match_it(&w, &s.accountant, &st).await.assert_status(303);
    assert_eq!(reload(&w, &st).await.status, PayoutStatus::Matched);
    let late = fnb::LateOrder::find_number(w.db(), AGGREGATOR, "ord-1099")
        .await
        .unwrap()
        .expect("the late order");
    assert_eq!(late.gross, 4_200);
    assert_eq!(late.entered_by, Some(s.late_cashier.id));
}

/// A line with no order can also be explained, once, by the accountant.
#[renox::test]
async fn a_line_with_no_order_can_be_explained_instead() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    import(
        &w,
        &s.accountant,
        "AGG-W44",
        &file(&["FEE-OCT,0,0,0,0,-20.00,"]),
    )
    .await
    .assert_status(303);
    let st = statement(&w, "AGG-W44").await;
    let line = line_of(&w, &st, "FEE-OCT").await;
    let path = format!("/payouts/{}/lines/{}/explain", st.id, line.id);
    w.post(&s.accountant, &path, &[("note", "  ")]).await.assert_status(422);
    w.post(&s.accountant, &path, &[("note", "Monthly platform fee")])
        .await
        .assert_status(303);
    assert_eq!(line_of(&w, &st, "FEE-OCT").await.match_state, "explained");
    match_it(&w, &s.accountant, &st).await.assert_status(303);
}

/// `FNB.RUL.10`: unmatched lines above AED 100 give the accountant a
/// reconciliation task; below it none; the task closes when the statement is
/// matched.
#[renox::test]
async fn unmatched_lines_above_the_limit_open_a_task_for_the_accountant() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    import(
        &w,
        &s.accountant,
        "AGG-SMALL",
        &file(&["ORD-7001,60.00,12.00,0,0,48.00,"]),
    )
    .await
    .assert_status(303);
    assert!(w.tickets.opened().is_empty(), "AED 60.00 is under the limit");

    import(
        &w,
        &s.accountant,
        "AGG-BIG",
        &file(&["ORD-7002,60.00,12.00,0,0,48.00,", "ORD-7003,95.00,19.00,0,0,76.00,"]),
    )
    .await
    .assert_status(303);
    let big = statement(&w, "AGG-BIG").await;
    let key = format!("fnb.payout_unmatched:{}", big.id);
    let opened = w.tickets.opened_with(&key);
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].subject, big.subject());
    assert!(reload(&w, &big).await.task_id.is_some());
}

/// The orders flagged for the payout (an order the aggregator cancelled, whose
/// compensation is expected) are listed as differences until the statement
/// pays them; voiding the statement frees them again.
#[renox::test]
async fn an_aggregator_cancelled_order_is_a_difference_until_a_statement_pays_it() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    let customer = w.customer("Cancelled customer").await;
    let placed = w
        .place_via_external(AGGREGATOR, Some("ORD-8001"), customer, "on_account", &[("shawarma-plate", "1", "60.00")])
        .await;
    let ready = w.cook(&placed).await;
    w.post(
        &w.manager,
        &format!("/orders/{}/pickup/cancel", ready.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);
    let cancelled = w.reload(&ready).await;
    assert!(cancelled.compensation_expected);

    // A statement without the order's compensation lists it as a difference.
    let other = completed(&w, "ORD-8002", "30.00").await;
    import(
        &w,
        &s.accountant,
        "AGG-W45",
        &file(&["ORD-8002,30.00,6.00,0,0,24.00,"]),
    )
    .await
    .assert_status(303);
    let st = statement(&w, "AGG-W45").await;
    let found = differences(w.db(), &st).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].kind, "compensation");
    assert_eq!(found[0].order_number, cancelled.number);
    w.get(&s.accountant, &format!("/payouts/{}", st.id))
        .await
        .assert_ok()
        .assert_see(&cancelled.number);
    let _ = other;

    // A statement that pays it matches it.
    import(
        &w,
        &s.accountant,
        "AGG-W46",
        &file(&["ORD-8001,60.00,12.00,0,0,48.00,"]),
    )
    .await
    .assert_status(303);
    let paying = statement(&w, "AGG-W46").await;
    assert!(differences(w.db(), &paying).await.unwrap().is_empty());
    match_it(&w, &s.accountant, &paying).await.assert_status(303);
    let paid = w.reload(&cancelled).await;
    assert_eq!(paid.compensation_statement_id, Some(paying.id));
    assert!(paid.compensation_matched_at.is_some());

    // Voided, the matching is reversed and the order is expected again.
    void(&w, &s.accountant, paying.id, "wrong file").await;
    let request = ApprovalRequest::pending_about(w.db(), &paying.subject())
        .await
        .unwrap()
        .remove(0);
    approvals::approve(w.state(), request.id, &s.approver, "")
        .await
        .unwrap();
    let again = w.reload(&cancelled).await;
    assert!(again.compensation_matched_at.is_none());
    assert!(again.compensation_statement_id.is_none());
}
