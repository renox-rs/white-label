//! BL-PAY-23 (#116), `FIN.ACL.05`, `FIN.ACL.06`, X-07, 03-N9-a, wl-payments
//! §4 `raise_shortage`: a shortage found after the holder's handover was
//! counted (a 03 delivery reversed days later with liability *driver*) is
//! raised by the operations manager as a new handover, `short` from the start
//! and linked to its source, recovered or written off like any other.

mod support;

use renox::auth::permissions;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::approvals::ApprovalRequest;
use wl_kernel::custody::{HandoverKind, HandoverStatus};
use wl_kernel::notify::NotifyRecipient;
use wl_payments::CashHandover;
use wl_payments::cash_handover::PostingKind;
use wl_payments::cash_notices::CASH_SHORTAGE;

use support::{Booted, boot};

fn shipment(n: i64) -> Subject {
    Subject::new("logistics.shipment", n)
}

/// The driver's manager: a holder of the default manager role.
async fn manager(t: &Booted, name: &str) -> User {
    let user = User::register(
        t.db(),
        name,
        &format!("{}@example.com", name.to_lowercase()),
        "password123",
    )
    .await
    .unwrap();
    permissions::define_role(
        t.db(),
        "finance_manager",
        &[wl_payments::permissions::CASH_VIEW],
    )
    .await
    .unwrap();
    user.assign_role(t.db(), "finance_manager").await.unwrap();
    user
}

fn told(recipients: &[NotifyRecipient], user: &User) -> bool {
    recipients
        .iter()
        .any(|r| matches!(r, NotifyRecipient::User { id } if *id == user.id))
}

/// `by` raises `amount` against `holder` for `source` (a shipment).
async fn raise(
    t: &Booted,
    by: &User,
    holder: &User,
    amount: &str,
    source: i64,
) -> renox::testing::TestResponse {
    t.cash_post(
        by,
        "/cash/shortages",
        &[
            ("holder", &holder.id.to_string()),
            ("amount", amount),
            ("source_type", "logistics.shipment"),
            ("source_id", &source.to_string()),
            ("reason", "Delivery reversed, liability driver"),
        ],
    )
    .await
}

/// The later shortage raised for `source`.
async fn later_of(t: &Booted, source: i64) -> CashHandover {
    CashHandover::query()
        .where_(CashHandover::KIND.eq(HandoverKind::LaterShortage))
        .where_(CashHandover::SOURCE_ID.eq(source))
        .first(t.db())
        .await
        .unwrap()
        .expect("a later shortage")
}

/// A counted handover of driver D with AED 300.00 collected on 2027-05-01.
async fn counted(t: &Booted, driver: &User, counter: &User) -> CashHandover {
    let handover = t.collect(driver, "2027-05-01", 30_000, 1).await;
    t.count_cash(counter, &handover, "300.00").await.assert_ok();
    let after = t.reloaded(&handover).await;
    assert_eq!(after.status, HandoverStatus::Counted);
    after
}

/// AC1: driver D's handover of 2027-05-01 is `counted` and shipment S (COD AED
/// 300.00) is reversed on 2027-05-04 with liability *driver*: operations
/// manager M raises AED 300.00 with S as the source; a new handover of kind
/// *later shortage* is `short` AED 300.00, D and their manager are notified and
/// `CashShortage` carries S. The counted handover is not changed.
#[renox::test]
async fn a_later_shortage_is_a_new_short_handover_linked_to_its_source() {
    let t = boot().await;
    let mira = manager(&t, "Mira").await;
    let d = t.holder("Dani").await;
    let c = t.counter("Chandra").await;
    let m = t.cash_manager("Mona").await;
    let first = counted(&t, &d, &c).await;
    let shortages_before = t.cash_shortage.all().len();

    raise(&t, &m, &d, "300.00", 9).await.assert_ok();

    let later = later_of(&t, 9).await;
    assert_ne!(later.id, first.id);
    assert_eq!(later.kind, HandoverKind::LaterShortage);
    assert_eq!(later.status, HandoverStatus::Short);
    assert_eq!(
        (later.expected, later.counted, later.difference),
        (30_000, 0, 30_000)
    );
    assert_eq!(later.holder_user_id, d.id);
    assert_eq!(later.source(), Some(shipment(9)));
    assert_eq!(later.created_by, Some(m.id));
    assert_eq!(
        later.reason.as_deref(),
        Some("Delivery reversed, liability driver")
    );
    let events = t.cash_shortage.all();
    assert_eq!(events.len(), shortages_before + 1);
    let event = events.last().unwrap();
    assert_eq!(event.source, Some(shipment(9)));
    assert_eq!(event.difference, 30_000);
    assert_eq!(event.handover, later.handover_id());
    let recipients = t.notices.recipients(CASH_SHORTAGE);
    assert!(told(&recipients, &d), "the driver is told");
    assert!(told(&recipients, &mira), "the driver's manager is told");
    // The handover the cash first came in on stays as it was counted.
    let unchanged = t.reloaded(&first).await;
    assert_eq!(unchanged.status, HandoverStatus::Counted);
    assert_eq!((unchanged.expected, unchanged.counted), (30_000, 30_000));
    t.app.acting_as(&m);
    t.app.get(&format!("/cash/{}", later.id)).await.assert_ok();
}

/// AC2: a second `raise_shortage` for S is refused while the first is `short`;
/// once the first is settled another can be raised.
#[renox::test]
async fn a_second_shortage_for_the_same_source_is_refused_while_the_first_is_short() {
    let t = boot().await;
    let d = t.holder("Dani").await;
    let c = t.counter("Chandra").await;
    let m = t.cash_manager("Mona").await;
    counted(&t, &d, &c).await;
    raise(&t, &m, &d, "300.00", 9).await.assert_ok();

    raise(&t, &m, &d, "300.00", 9).await.assert_status(422);
    // Another source is another shortage.
    raise(&t, &m, &d, "50.00", 10).await.assert_ok();
    let count = CashHandover::query()
        .where_(CashHandover::KIND.eq(HandoverKind::LaterShortage))
        .count(t.db())
        .await
        .unwrap();
    assert_eq!(count, 2);

    let first = later_of(&t, 9).await;
    t.cash_post(
        &c,
        &format!("/cash/{}/recover", first.id),
        &[("amount", "300.00")],
    )
    .await
    .assert_ok();
    raise(&t, &m, &d, "300.00", 9).await.assert_ok();
}

/// AC3: D running `raise_shortage` on themselves is refused (`FIN.ACL.05`),
/// even holding `cash.count`; a user without it is refused at the route; a
/// reason, an amount and a source are required.
#[renox::test]
async fn nobody_raises_a_shortage_against_themselves_and_the_request_is_checked() {
    let t = boot().await;
    let d = t.counter("Dani").await;
    let c = t.counter("Chandra").await;
    let plain = t.holder("Pia").await;
    counted(&t, &d, &c).await;

    raise(&t, &d, &d, "300.00", 9).await.assert_status(422);
    raise(&t, &plain, &d, "300.00", 9).await.assert_forbidden();
    raise(&t, &c, &d, "0", 9).await.assert_status(422);
    t.cash_post(
        &c,
        "/cash/shortages",
        &[
            ("holder", &d.id.to_string()),
            ("amount", "10.00"),
            ("source_type", "logistics.shipment"),
            ("source_id", "9"),
            ("reason", "  "),
        ],
    )
    .await
    .assert_status(422);
    t.cash_post(
        &c,
        "/cash/shortages",
        &[
            ("holder", &d.id.to_string()),
            ("amount", "10.00"),
            ("source_type", "no source"),
            ("source_id", "9"),
            ("reason", "Reversed"),
        ],
    )
    .await
    .assert_status(422);
    let count = CashHandover::query()
        .where_(CashHandover::KIND.eq(HandoverKind::LaterShortage))
        .count(t.db())
        .await
        .unwrap();
    assert_eq!(count, 0);
}

/// AC4: AED 300.00 recovered in cash moves the later shortage to `recovered`;
/// one of AED 40.00 is written off at once by the cashier within the AED 50.00
/// limit, while one of AED 60.00 waits for an approval (`FIN.ACL.06`).
#[renox::test]
async fn a_later_shortage_is_recovered_or_written_off_like_any_other() {
    let t = boot().await;
    t.set_cash_write_off_limit(5_000).await;
    let d = t.holder("Dani").await;
    let c = t.counter("Chandra").await;
    let m = t.cash_manager("Mona").await;
    counted(&t, &d, &c).await;

    raise(&t, &m, &d, "300.00", 9).await.assert_ok();
    let big = later_of(&t, 9).await;
    t.cash_post(
        &c,
        &format!("/cash/{}/recover", big.id),
        &[("amount", "300.00")],
    )
    .await
    .assert_ok();
    assert_eq!(t.reloaded(&big).await.status, HandoverStatus::Recovered);

    raise(&t, &m, &d, "40.00", 10).await.assert_ok();
    let small = later_of(&t, 10).await;
    t.write_off_cash(&m, &small).await.assert_ok();
    let written = t.reloaded(&small).await;
    assert_eq!(written.status, HandoverStatus::WrittenOff);
    assert_eq!(written.written_off, 4_000);
    assert!(t.pending_request(&small.subject()).await.is_none());
    let postings = t.postings(&written).await;
    assert_eq!(postings.len(), 1);
    assert_eq!(postings[0].kind, PostingKind::ShortageLoss);
    assert_eq!(postings[0].amount, 4_000);

    raise(&t, &m, &d, "60.00", 11).await.assert_ok();
    let above = later_of(&t, 11).await;
    t.write_off_cash(&m, &above).await.assert_ok();
    let request: ApprovalRequest = t
        .pending_request(&above.subject())
        .await
        .expect("above the limit an approval waits");
    assert_eq!(request.payload.0["amount"], 6_000);
    assert_eq!(t.reloaded(&above).await.status, HandoverStatus::Short);
}

/// While the holder's handover for that cash is still `open`, no later
/// shortage is made: the amount is added to its expected total and shows at its
/// count.
#[renox::test]
async fn while_the_handover_is_open_the_amount_is_added_to_what_is_expected() {
    let t = boot().await;
    let d = t.holder("Dani").await;
    let c = t.counter("Chandra").await;
    let m = t.cash_manager("Mona").await;
    let open = t.collect(&d, "2027-05-01", 30_000, 1).await;

    // The delivery of the cash is the source, as wl-delivery would name it.
    t.cash_post(
        &m,
        "/cash/shortages",
        &[
            ("holder", &d.id.to_string()),
            ("amount", "100.00"),
            ("source_type", "delivery.delivery"),
            ("source_id", "1"),
            ("reason", "Delivery reversed"),
        ],
    )
    .await
    .assert_ok();

    let after = t.reloaded(&open).await;
    assert_eq!(after.status, HandoverStatus::Open);
    assert_eq!(after.expected, 40_000);
    assert!(t.cash_shortage.all().is_empty());
    let count = CashHandover::query()
        .where_(CashHandover::KIND.eq(HandoverKind::LaterShortage))
        .count(t.db())
        .await
        .unwrap();
    assert_eq!(count, 0);

    t.count_cash(&c, &after, "300.00").await.assert_ok();
    let short = t.reloaded(&open).await;
    assert_eq!(short.status, HandoverStatus::Short);
    assert_eq!(short.difference, 10_000);
}

/// A later shortage for a cash whose handover was counted keeps the period of
/// that handover (its day and run), so the reversed delivery's run is found.
#[renox::test]
async fn a_later_shortage_keeps_the_period_of_the_counted_handover() {
    let t = boot().await;
    let d = t.holder("Dani").await;
    let c = t.counter("Chandra").await;
    let m = t.cash_manager("Mona").await;
    let first = counted(&t, &d, &c).await;

    t.cash_post(
        &m,
        "/cash/shortages",
        &[
            ("holder", &d.id.to_string()),
            ("amount", "300.00"),
            ("source_type", "delivery.delivery"),
            ("source_id", "1"),
            ("reason", "Delivery reversed"),
        ],
    )
    .await
    .assert_ok();

    let later = later_of(&t, 1).await;
    assert_ne!(later.id, first.id);
    assert_eq!(later.period_date, first.period_date);
    assert_eq!(later.period_kind, first.period_kind);
    assert_eq!(later.location, first.location);
}
