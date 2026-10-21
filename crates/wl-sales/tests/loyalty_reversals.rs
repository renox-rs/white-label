//! Loyalty points taken back and earned again (BL-SAL-22, #141,
//! `SAL.RUL.07` returns and credit notes, `SAL.RUL.09` reversed payments,
//! write-offs and fulfilments): in proportion, rounded up, once per cause, the
//! balance may go below zero, and the member is told. Each check drives the
//! real pages, transitions and listeners.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::invoicing::{
    CreditNoteId, CreditNoteIssued, InvoiceId, InvoicePaid, InvoiceReopened, InvoiceStatus,
    InvoiceWrittenOff, ReopenReason,
};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::payments::{
    PaymentId, PaymentMethod, PaymentReceived, PaymentReversed, PaymentSource, ReversalReason,
};
use wl_kernel::sales::{
    FulfilOrder, FulfilledLine, ReverseFulfilment, SalesOrderId, order_fulfilment,
};
use wl_kernel::{PartyId, Subject};
use wl_sales::notices::{LOYALTY_REVERSED, LOYALTY_REWARD};
use wl_sales::{
    CounterSale, LoyaltyAccount, LoyaltyEarning, PointsKind, ReturnStatus, SaleLine, SalesOrder,
    SalesReturn,
};

use support::floor::{Floor, account, movements};
use support::shop::Shop;
use support::{Booted, boot_shop};

/// The party of the member M in every test.
const M: i64 = 7;

async fn start() -> (Booted, Shop, Floor) {
    let (t, shop) = boot_shop().await;
    let floor = Floor::new(&t).await;
    (t, shop, floor)
}

/// Returns `quantity` of the first line of `sale` through the pages: request,
/// receive, inspect and refund. Returns the return.
async fn give_back(t: &Booted, sale: &CounterSale, quantity: &str) -> SalesReturn {
    let line = SaleLine::of(t.db(), &sale.subject()).await.unwrap().remove(0);
    t.app
        .post(
            "/sales-returns",
            &[
                ("sale_type", "counter_sale"),
                ("sale_id", &sale.id.to_string()),
                ("lines", &format!("{}:{quantity}", line.id)),
            ],
        )
        .await
        .assert_status(303);
    let r = SalesReturn::query()
        .order_by_desc(SalesReturn::ID)
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    t.app
        .post(&format!("/sales-returns/{}/receive", r.id), &[("x", "")])
        .await
        .assert_status(303);
    t.app
        .post(
            &format!("/sales-returns/{}/resolve", r.id),
            &[("resolution", "refund"), ("inspected", "1")],
        )
        .await
        .assert_status(303);
    let r = SalesReturn::find_or_404(t.db(), r.id).await.unwrap();
    assert_eq!(r.status, ReturnStatus::Resolved);
    r
}

#[renox::test]
async fn returning_half_of_the_eligible_amount_reverses_half_the_points_and_the_balance_may_go_negative()
 {
    let (t, shop, floor) = start().await;
    floor.turn_on(&t, &[]).await;
    let m = floor.enrol(&t, M, "0501234567").await;
    // Two tees at AED 50: AED 100 before VAT earns 100 points.
    let sale = floor
        .ring_up(&t, &[("tee", "50.00", "2")], Some(&m.code), &[("cash", "105.00")])
        .await;
    assert_eq!(account(&t, &m).await.points_balance, 100);
    // 80 of the points were spent elsewhere: the balance is 20.
    LoyaltyAccount::query()
        .where_(LoyaltyAccount::ID.eq(m.id))
        .update(t.db(), &[("points_balance", &20_i64)])
        .await
        .unwrap();

    let r = give_back(&t, &sale, "1").await;
    assert_eq!(r.eligible_net, 5_000);
    // Half of 100 points are taken back: 20 − 50 = −30.
    let m = account(&t, &m).await;
    assert_eq!(m.points_balance, -30);
    let taken = movements(&t, &m, PointsKind::ReverseEarn).await;
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].points, -50);
    assert_eq!(taken[0].cause, "return");
    assert_eq!(taken[0].source(), r.subject());
    let events = shop.points.reversed.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!((events[0].party, events[0].points), (PartyId(M), 50));
    let told = shop.mail.of_type(LOYALTY_REVERSED);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].recipients, vec![NotifyRecipient::Party { id: PartyId(M) }]);

    // The other half: the remaining 50 points, never more than earned.
    give_back(&t, &sale, "1").await;
    assert_eq!(account(&t, &m).await.points_balance, -80);
    let earning = LoyaltyEarning::of_source(t.db(), &sale.subject())
        .await
        .unwrap()
        .unwrap();
    assert_eq!((earning.points, earning.reversed_points), (100, 100));
}

#[renox::test]
async fn a_chargeback_reverses_the_points_once_and_winning_it_earns_them_again_in_a_new_lot() {
    let (t, shop, floor) = start().await;
    floor.turn_on(&t, &[]).await;
    let m = floor.enrol(&t, M, "0501234567").await;
    // A card sale of AED 105, paid with the card payment 77.
    let sale = floor
        .ring_up(
            &t,
            &[("tee", "100.00", "1")],
            Some(&m.code),
            &[("card", "105.00"), ("card_payment", "77")],
        )
        .await;
    assert_eq!(account(&t, &m).await.points_balance, 100);

    let reversed = PaymentReversed {
        payment: PaymentId(77),
        party: PartyId(M),
        legal_entity_id: 1,
        reason: ReversalReason::Chargeback,
        source: PaymentSource::Gateway,
        statement_line: None,
        removed: Vec::new(),
        prepaid: None,
        reversed_on: NaiveDate::from_ymd_opt(2026, 11, 20).unwrap(),
    };
    t.state().emit(reversed.clone()).await.unwrap();
    assert_eq!(account(&t, &m).await.points_balance, 0);
    let told = shop.mail.of_type(LOYALTY_REVERSED);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].recipients, vec![NotifyRecipient::Party { id: PartyId(M) }]);

    // The same reversal delivered twice takes nothing more.
    t.state().emit(reversed).await.unwrap();
    assert_eq!(account(&t, &m).await.points_balance, 0);
    assert_eq!(shop.mail.of_type(LOYALTY_REVERSED).len(), 1);

    // The business wins the chargeback: the payment is received again.
    t.state()
        .emit(PaymentReceived {
            payment: PaymentId(77),
            party: PartyId(M),
            amount: 10_500,
            method: PaymentMethod::Card,
            allocations: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(account(&t, &m).await.points_balance, 100);
    let lots = movements(&t, &m, PointsKind::Earn).await;
    assert_eq!(lots.len(), 2);
    assert_eq!((lots[1].points, lots[1].cause.as_str()), (100, "reearn"));
    assert!(lots[1].lot_expires_on.is_some());
    assert_ne!(lots[0].id, lots[1].id);
    let earning = LoyaltyEarning::of_source(t.db(), &sale.subject())
        .await
        .unwrap()
        .unwrap();
    assert_eq!((earning.reversed_points, earning.movement_id), (0, Some(lots[1].id)));
    // Received again once more: nothing left to earn.
    t.state()
        .emit(PaymentReceived {
            payment: PaymentId(77),
            party: PartyId(M),
            amount: 10_500,
            method: PaymentMethod::Card,
            allocations: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(account(&t, &m).await.points_balance, 100);
}

#[renox::test]
async fn an_invoice_reopened_after_a_bounced_cheque_takes_its_points_back_and_paying_it_earns_a_new_lot()
 {
    let (t, shop, floor) = start().await;
    floor.turn_on(&t, &[]).await;
    let m = floor.enrol(&t, M, "0501234567").await;
    // AED 1,000 without VAT (zero rated), paid by cheque.
    let claim = Subject::new("clinics.visit", 9);
    shop.books.add(8, PartyId(M), 100_000, 100_000);
    let paid = InvoicePaid {
        invoice: InvoiceId(8),
        party: PartyId(M),
        source: claim.clone(),
    };
    t.state().emit(paid.clone()).await.unwrap();
    assert_eq!(account(&t, &m).await.points_balance, 1_000);

    // The cheque bounces: the invoice is open again with its whole amount.
    let reopened = InvoiceReopened {
        invoice: InvoiceId(8),
        party: PartyId(M),
        source: claim,
        money_source: PaymentId(5).subject(),
        amount: 100_000,
        balance: 100_000,
        new_status: InvoiceStatus::Issued,
        reason: ReopenReason::PaymentReversed,
        reversal_reason: Some("bank_return".to_owned()),
    };
    t.state().emit(reopened.clone()).await.unwrap();
    assert_eq!(account(&t, &m).await.points_balance, 0);
    assert_eq!(shop.mail.of_type(LOYALTY_REVERSED).len(), 1);
    // The reversal of the same payment arrives from the payments module too:
    // once per cause, so nothing more is taken.
    t.state().emit(reopened).await.unwrap();
    assert_eq!(account(&t, &m).await.points_balance, 0);

    // Paid again: 1,000 points in a new lot.
    t.state().emit(paid).await.unwrap();
    assert_eq!(account(&t, &m).await.points_balance, 1_000);
    let lots = movements(&t, &m, PointsKind::Earn).await;
    assert_eq!(lots.len(), 2);
    assert_eq!(lots[1].points, 1_000);
    // A reopening for another reason is not a payment reversal.
    t.state()
        .emit(InvoiceReopened {
            invoice: InvoiceId(8),
            party: PartyId(M),
            source: Subject::new("clinics.visit", 9),
            money_source: PaymentId(6).subject(),
            amount: 100_000,
            balance: 100_000,
            new_status: InvoiceStatus::Issued,
            reason: ReopenReason::AllocationMoved,
            reversal_reason: None,
        })
        .await
        .unwrap();
    assert_eq!(account(&t, &m).await.points_balance, 1_000);
}

/// An on-account order of AED 300 before VAT, member attached, confirmed and
/// handed over from the dispatch note 1.
async fn fulfilled_order(t: &Booted, member: &str) -> (SalesOrder, SaleLine) {
    t.app
        .post("/sales-orders", &[("party_id", &M.to_string())])
        .await
        .assert_status(303);
    let order = SalesOrder::query()
        .order_by_desc(SalesOrder::ID)
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    t.app
        .post(
            &format!("/sales-orders/{}/lines", order.id),
            &[("item_code", "desk"), ("unit_price", "300.00")],
        )
        .await
        .assert_status(303);
    t.app
        .post(
            &format!("/sales-orders/{}/member", order.id),
            &[("code", member)],
        )
        .await
        .assert_status(303);
    t.app
        .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
        .await
        .assert_status(303);
    let line = SaleLine::of(t.db(), &order.subject()).await.unwrap().remove(0);
    order_fulfilment(t.state())
        .fulfil(
            t.state(),
            FulfilOrder {
                order: SalesOrderId(order.id),
                lines: vec![FulfilledLine {
                    line: line.id,
                    quantity_milli: 1000,
                }],
                source: Subject::new("inventory.dispatch", 1),
            },
        )
        .await
        .unwrap();
    (SalesOrder::find_or_404(t.db(), order.id).await.unwrap(), line)
}

#[renox::test]
async fn writing_off_the_invoice_of_an_order_that_earned_takes_its_points_back() {
    let (t, shop, floor) = start().await;
    floor.turn_on(&t, &[]).await;
    let m = floor.enrol(&t, M, "0501234567").await;
    let (order, _) = fulfilled_order(&t, &m.code).await;
    assert_eq!(account(&t, &m).await.points_balance, 300);
    let invoice = order.invoice_id.expect("the invoice was drafted on fulfilment");

    // AED 315 with VAT is written off: the 300 points go.
    t.state()
        .emit(InvoiceWrittenOff {
            invoice: InvoiceId(invoice),
            party: PartyId(M),
            source: order.subject(),
            amount: 31_500,
            reason_code: "insolvent".to_owned(),
            approver: None,
        })
        .await
        .unwrap();
    assert_eq!(account(&t, &m).await.points_balance, 0);
    let taken = movements(&t, &m, PointsKind::ReverseEarn).await;
    assert_eq!((taken[0].points, taken[0].cause.as_str()), (-300, "written_off"));
    assert_eq!(shop.mail.of_type(LOYALTY_REVERSED).len(), 1);
}

#[renox::test]
async fn a_reversed_fulfilment_takes_points_back_and_a_new_hand_over_earns_them_again() {
    let (t, _, floor) = start().await;
    floor.turn_on(&t, &[]).await;
    let m = floor.enrol(&t, M, "0501234567").await;
    let (order, line) = fulfilled_order(&t, &m.code).await;
    assert_eq!(account(&t, &m).await.points_balance, 300);

    // The delivery is lost: everything handed over is taken off.
    order_fulfilment(t.state())
        .reverse(
            t.state(),
            ReverseFulfilment {
                order: SalesOrderId(order.id),
                lines: vec![FulfilledLine {
                    line: line.id,
                    quantity_milli: 1000,
                }],
                source: Subject::new("inventory.dispatch", 1),
                cause: "lost".to_owned(),
            },
        )
        .await
        .unwrap();
    assert_eq!(account(&t, &m).await.points_balance, 0);

    // A second dispatch delivers it: earned again, in a new lot.
    order_fulfilment(t.state())
        .fulfil(
            t.state(),
            FulfilOrder {
                order: SalesOrderId(order.id),
                lines: vec![FulfilledLine {
                    line: line.id,
                    quantity_milli: 1000,
                }],
                source: Subject::new("inventory.dispatch", 2),
            },
        )
        .await
        .unwrap();
    assert_eq!(account(&t, &m).await.points_balance, 300);
    assert_eq!(movements(&t, &m, PointsKind::Earn).await.len(), 2);
}

#[renox::test]
async fn a_credit_note_takes_points_back_in_proportion_once() {
    let (t, shop, floor) = start().await;
    floor.turn_on(&t, &[]).await;
    let m = floor.enrol(&t, M, "0501234567").await;
    // The member's share is AED 200 before VAT (200 points).
    let claim = Subject::new("clinics.visit", 3);
    shop.books.add(5, PartyId(M), 100_000, 105_000);
    shop.insurer.claim(claim.clone(), 80_000);
    t.state()
        .emit(InvoicePaid {
            invoice: InvoiceId(5),
            party: PartyId(M),
            source: claim,
        })
        .await
        .unwrap();
    assert_eq!(account(&t, &m).await.points_balance, 200);

    // A credit note of AED 50 before VAT: a quarter of the AED 200.
    let note = CreditNoteIssued {
        credit_note: CreditNoteId(4),
        invoice: InvoiceId(5),
        party: PartyId(M),
        amount: 5_250,
        net_amount: 5_000,
        reason: "price adjustment".to_owned(),
    };
    t.state().emit(note.clone()).await.unwrap();
    assert_eq!(account(&t, &m).await.points_balance, 150);
    t.state().emit(note).await.unwrap();
    assert_eq!(account(&t, &m).await.points_balance, 150);
    let taken = movements(&t, &m, PointsKind::ReverseEarn).await;
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].cause, "credit_note");
}

#[renox::test]
async fn returning_the_tenth_sale_before_the_reward_is_used_withdraws_it_and_leaves_nine_visits() {
    let (t, shop, floor) = start().await;
    floor
        .turn_on(
            &t,
            &[
                ("earn_kind", "per_visit"),
                ("earn_rate", "10"),
                ("visit_reward", "a free coffee"),
            ],
        )
        .await;
    let m = floor.enrol(&t, M, "0501234567").await;
    let mut tenth = None;
    for _ in 0..10 {
        tenth = Some(
            floor
                .ring_up(&t, &[("coffee", "10.00", "1")], Some(&m.code), &[("cash", "10.50")])
                .await,
        );
    }
    let before = account(&t, &m).await;
    assert_eq!(before.visits, 0);
    assert!(before.reward_available_until.is_some());

    give_back(&t, &tenth.unwrap(), "1").await;
    let after = account(&t, &m).await;
    assert_eq!(after.visits, 9);
    assert_eq!(after.reward_available_until, None);
    let taken = movements(&t, &m, PointsKind::ReverseEarn).await;
    assert_eq!((taken.len(), taken[0].points), (1, -1));

    // The reward notice went out once, when the tenth sale made it; the
    // return told the member that the visit was taken back.
    assert_eq!(shop.mail.of_type(LOYALTY_REWARD).len(), 1);
    assert_eq!(shop.mail.of_type(LOYALTY_REVERSED).len(), 1);
}

#[renox::test]
async fn nothing_is_reversed_on_a_closed_account_or_for_a_sale_that_never_earned() {
    let (t, shop, floor) = start().await;
    floor.turn_on(&t, &[]).await;
    let m = floor.enrol(&t, M, "0501234567").await;
    let earned = floor
        .ring_up(&t, &[("tee", "50.00", "2")], Some(&m.code), &[("cash", "105.00")])
        .await;
    let never = floor
        .ring_up(&t, &[("tee", "50.00", "2")], None, &[("cash", "105.00")])
        .await;
    // A return of the sale that never earned reverses nothing.
    give_back(&t, &never, "1").await;
    assert_eq!(account(&t, &m).await.points_balance, 100);

    // The account is closed (BL-SAL-26 does it with a reason): a return on a
    // sale earned before the closing reverses nothing.
    LoyaltyAccount::query()
        .where_(LoyaltyAccount::ID.eq(m.id))
        .update(t.db(), &[("status", &wl_sales::AccountStatus::Closed)])
        .await
        .unwrap();
    give_back(&t, &earned, "1").await;
    assert_eq!(account(&t, &m).await.points_balance, 100);
    assert!(shop.mail.of_type(LOYALTY_REVERSED).is_empty());
}
