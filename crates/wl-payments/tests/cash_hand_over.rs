//! BL-PAY-20 (#113), `FIN.CSH.01`, X-12, X-07: the cash a receiving driver
//! counts on a run hand-over (`RunHandedOver` of wl-delivery) leaves the
//! giver's expected amount and joins the receiver's handover; any difference
//! stays on the giver's handover and shows as a shortage at its count; an
//! excess is an overage line of the receiver's handover. The event is
//! emitted as wl-delivery emits it; `hand_over_real.rs` of wl-delivery runs
//! the same with the real delivery module.

mod support;

use renox::prelude::*;
use wl_kernel::custody::{
    CashCollection, HandoverPeriod, HandoverStatus, PeriodKind, custody,
};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::{DeliveryId, RunHandedOver, RunId, Subject};
use wl_payments::cash_handover::PostingKind;
use wl_payments::cash_hand_over;
use wl_payments::cash_notices::CASH_HANDED_OVER;
use wl_payments::{CashHandover, CashHolder};

use support::cash::{at, date};
use support::{Booted, boot};

const GIVING_RUN: i64 = 1;
const RECEIVING_RUN: i64 = 2;

fn run(n: i64) -> Subject {
    RunId(n).subject()
}

fn period(n: i64) -> HandoverPeriod {
    HandoverPeriod::run(run(n), date("2027-04-01"), Some(at("2027-04-01T14:00:00Z")))
}

/// `driver` collected `amount` fils on the giving run for the delivery `n`.
async fn collected(t: &Booted, driver: &User, amount: i64, n: i64) -> CashHandover {
    let id = custody(t.state())
        .collect(
            t.state(),
            CashCollection::new(
                driver.id,
                period(GIVING_RUN),
                amount,
                DeliveryId(n).subject(),
            )
            .location("Dubai depot"),
        )
        .await
        .expect("the cash is collected")
        .expect("payments is on");
    t.handover(id).await
}

/// The hand-over `id` from the giver to the receiver with `counted` fils.
fn event(id: i64, giver: &User, receiver: &User, dispatcher: &User, counted: i64) -> RunHandedOver {
    RunHandedOver {
        hand_over: id,
        run: RunId(GIVING_RUN),
        to_run: RunId(RECEIVING_RUN),
        from_driver: giver.id,
        to_driver: receiver.id,
        requested_by: dispatcher.id,
        deliveries: vec![DeliveryId(10), DeliveryId(11)],
        cash_counted: counted,
        cheques: Vec::new(),
        reason: "breakdown".to_owned(),
        to_period: period(RECEIVING_RUN),
    }
}

/// The receiver's handover of the receiving run.
async fn receivers(t: &Booted, receiver: &User) -> Option<CashHandover> {
    CashHandover::of_period_record(t.db(), receiver.id, &run(RECEIVING_RUN))
        .await
        .unwrap()
}

fn notified(recipients: &[NotifyRecipient], user: &User) -> bool {
    recipients
        .iter()
        .any(|r| matches!(r, NotifyRecipient::User { id } if *id == user.id))
}

struct Cast {
    giver: User,
    receiver: User,
    dispatcher: User,
    counter: User,
}

async fn cast(t: &Booted) -> Cast {
    Cast {
        giver: t.holder("Gina").await,
        receiver: t.holder("Hadi").await,
        dispatcher: t.holder("Dina").await,
        counter: t.counter("Chandra").await,
    }
}

/// AC1: driver G with AED 1,200.00 expected and receiver H confirming AED
/// 1,150.00: H's handover (opened for the receiving run) expects AED
/// 1,150.00, G's keeps AED 50.00 expected, both drivers and the dispatcher
/// are notified, and each payment keeps its collector.
#[renox::test]
async fn the_counted_cash_joins_the_receivers_handover_and_the_rest_stays_with_the_giver() {
    let t = boot().await;
    let c = cast(&t).await;
    let giver = collected(&t, &c.giver, 70_000, 1).await;
    collected(&t, &c.giver, 50_000, 2).await;
    assert!(receivers(&t, &c.receiver).await.is_none());

    t.app
        .state()
        .emit(event(1, &c.giver, &c.receiver, &c.dispatcher, 115_000))
        .await
        .unwrap();

    let after = t.reloaded(&giver).await;
    assert_eq!(after.expected, 5_000);
    assert_eq!(after.status, HandoverStatus::Open);
    let theirs = receivers(&t, &c.receiver).await.expect("opened for H");
    assert_eq!(theirs.expected, 115_000);
    assert_eq!(theirs.status, HandoverStatus::Open);
    assert_eq!(theirs.holder_user_id, c.receiver.id);
    assert_eq!(theirs.period_kind, PeriodKind::Run);
    assert_eq!(theirs.period_date, date("2027-04-01"));
    // The giver's payments are still the giver's: two entries stay, one
    // takes the moved part out; the receiver's entry is the hand-over's.
    let given = t.entries(&after).await;
    assert_eq!(given.iter().filter(|e| e.amount > 0).count(), 2);
    assert_eq!(given.iter().map(|e| e.amount).sum::<i64>(), 5_000);
    let taken: Vec<_> = given.iter().filter(|e| e.amount < 0).collect();
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].source_type, cash_hand_over::SOURCE_TYPE);
    assert_eq!(taken[0].source_id, 1);
    let got = t.entries(&theirs).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].amount, 115_000);
    assert_eq!(got[0].source_type, cash_hand_over::SOURCE_TYPE);
    assert!(
        CashHolder::find(t.db(), theirs.holder_id)
            .await
            .unwrap()
            .is_some()
    );
    // Both drivers and the dispatcher are told.
    let recipients = t.notices.recipients(CASH_HANDED_OVER);
    assert!(notified(&recipients, &c.giver));
    assert!(notified(&recipients, &c.receiver));
    assert!(notified(&recipients, &c.dispatcher));
    let request = &t.notices.of_type(CASH_HANDED_OVER)[0];
    assert_eq!(request.data["amount"], "AED 1,150.00");
    assert_eq!(request.data["rest"], "AED 50.00");
}

/// AC2: at G's `count` with AED 0 counted, G is `short` AED 50.00; H's
/// handover is counted at what H holds.
#[renox::test]
async fn the_difference_shows_as_a_shortage_at_the_givers_count() {
    let t = boot().await;
    let c = cast(&t).await;
    let giver = collected(&t, &c.giver, 120_000, 1).await;
    t.app
        .state()
        .emit(event(1, &c.giver, &c.receiver, &c.dispatcher, 115_000))
        .await
        .unwrap();

    t.count_cash(&c.counter, &giver, "0").await.assert_ok();

    let after = t.reloaded(&giver).await;
    assert_eq!(after.status, HandoverStatus::Short);
    assert_eq!(after.difference, 5_000);
    let events = t.cash_shortage.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].difference, 5_000);
    assert_eq!(events[0].holder_user, c.giver.id);
    // The receiver's cash is counted on its own.
    let theirs = receivers(&t, &c.receiver).await.unwrap();
    t.count_cash(&c.counter, &theirs, "1150.00")
        .await
        .assert_ok();
    assert_eq!(t.reloaded(&theirs).await.status, HandoverStatus::Counted);
}

/// AC3: G with AED 1,200.00 expected and H confirming AED 1,230.00: H's
/// handover expects AED 1,230.00 including an overage line of AED 30.00 from
/// the hand-over, held as a liability; G's handover expects AED 0.00.
#[renox::test]
async fn an_excess_is_an_overage_line_of_the_receivers_handover() {
    let t = boot().await;
    let c = cast(&t).await;
    let giver = collected(&t, &c.giver, 120_000, 1).await;

    t.app
        .state()
        .emit(event(1, &c.giver, &c.receiver, &c.dispatcher, 123_000))
        .await
        .unwrap();

    assert_eq!(t.reloaded(&giver).await.expected, 0);
    let theirs = receivers(&t, &c.receiver).await.unwrap();
    assert_eq!(theirs.expected, 123_000);
    let entries = t.entries(&theirs).await;
    assert_eq!(entries.len(), 2);
    let line = entries
        .iter()
        .find(|e| e.source_type == cash_hand_over::OVERAGE_SOURCE_TYPE)
        .expect("the overage line");
    assert_eq!(line.amount, 3_000);
    assert_eq!(line.source_id, 1);
    let postings = t.postings(&theirs).await;
    assert_eq!(postings.len(), 1);
    assert_eq!(postings[0].kind, PostingKind::OverageLiability);
    assert_eq!(postings[0].amount, 3_000);
    // The giver's handover keeps nothing for it: counted at 0 it is counted.
    t.count_cash(&c.counter, &t.reloaded(&giver).await, "0")
        .await
        .assert_ok();
    assert_eq!(t.reloaded(&giver).await.status, HandoverStatus::Counted);
}

/// Edge: the event twice moves the cash once.
#[renox::test]
async fn the_event_twice_moves_the_cash_once() {
    let t = boot().await;
    let c = cast(&t).await;
    let giver = collected(&t, &c.giver, 120_000, 1).await;
    let first = event(7, &c.giver, &c.receiver, &c.dispatcher, 123_000);

    t.app.state().emit(first.clone()).await.unwrap();
    t.app.state().emit(first.clone()).await.unwrap();

    assert_eq!(t.reloaded(&giver).await.expected, 0);
    let theirs = receivers(&t, &c.receiver).await.unwrap();
    assert_eq!(theirs.expected, 123_000);
    assert_eq!(t.entries(&theirs).await.len(), 2);
    assert_eq!(t.postings(&theirs).await.len(), 1);
    assert_eq!(t.entries(&t.reloaded(&giver).await).await.len(), 2);
    let again = cash_hand_over::hand_over(t.state(), &first).await.unwrap();
    assert!(again.already);
    assert_eq!(again.moved, 120_000);
    assert_eq!(again.overage, 3_000);
    // A second hand-over is another key and moves what is left.
    let second = event(8, &c.giver, &c.receiver, &c.dispatcher, 1_000);
    t.app.state().emit(second).await.unwrap();
    assert_eq!(
        receivers(&t, &c.receiver).await.unwrap().expected,
        124_000
    );
}

/// Edge: a receiver that has an open handover for the run already gets the
/// cash added to it.
#[renox::test]
async fn the_cash_joins_a_handover_the_receiver_has_already() {
    let t = boot().await;
    let c = cast(&t).await;
    collected(&t, &c.giver, 40_000, 1).await;
    let own = custody(t.state())
        .collect(
            t.state(),
            CashCollection::new(
                c.receiver.id,
                period(RECEIVING_RUN),
                10_000,
                DeliveryId(50).subject(),
            ),
        )
        .await
        .unwrap()
        .unwrap();

    t.app
        .state()
        .emit(event(1, &c.giver, &c.receiver, &c.dispatcher, 40_000))
        .await
        .unwrap();

    let theirs = t.handover(own).await;
    assert_eq!(theirs.expected, 50_000);
    assert_eq!(t.entries(&theirs).await.len(), 2);
}

/// Edge: a giver with no cash on the run: everything the receiver counts is
/// an overage line.
#[renox::test]
async fn a_giver_without_cash_leaves_everything_as_an_overage_line() {
    let t = boot().await;
    let c = cast(&t).await;

    t.app
        .state()
        .emit(event(1, &c.giver, &c.receiver, &c.dispatcher, 10_000))
        .await
        .unwrap();

    let theirs = receivers(&t, &c.receiver).await.unwrap();
    assert_eq!(theirs.expected, 10_000);
    let postings = t.postings(&theirs).await;
    assert_eq!(postings.len(), 1);
    assert_eq!(postings[0].amount, 10_000);
    assert!(
        CashHandover::of_period_record(t.db(), c.giver.id, &run(GIVING_RUN))
            .await
            .unwrap()
            .is_none()
    );
}

/// Must not: a driver hands cash over to themselves, the counted cash is
/// negative, or the giver's handover was counted already; nothing moves.
#[renox::test]
async fn nothing_moves_to_oneself_or_from_a_counted_handover() {
    let t = boot().await;
    let c = cast(&t).await;
    let giver = collected(&t, &c.giver, 40_000, 1).await;

    let to_self = event(1, &c.giver, &c.giver, &c.dispatcher, 40_000);
    let refused = cash_hand_over::hand_over(t.state(), &to_self).await;
    assert!(matches!(
        refused,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
    let negative = event(2, &c.giver, &c.receiver, &c.dispatcher, -1);
    assert!(cash_hand_over::hand_over(t.state(), &negative).await.is_err());
    assert_eq!(t.reloaded(&giver).await.expected, 40_000);

    t.count_cash(&c.counter, &giver, "400.00").await.assert_ok();
    let late = event(3, &c.giver, &c.receiver, &c.dispatcher, 40_000);
    assert!(matches!(
        cash_hand_over::hand_over(t.state(), &late).await,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
    assert!(receivers(&t, &c.receiver).await.is_none());
    assert!(t.notices.of_type(CASH_HANDED_OVER).is_empty());
}
