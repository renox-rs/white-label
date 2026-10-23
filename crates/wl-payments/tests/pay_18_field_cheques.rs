//! BL-PAY-18 (#111), `FIN.ACL.20`, X-07, 08 MEDIUM "cheques in the field
//! handover": cheques collected in the field are expected in the holder's
//! handover. The counter ticks each present or missing; present cheques get
//! their custody confirmed and only then can be deposited; a missing cheque is
//! never written off as cash: it is found by a recount, or returned with the
//! reason "lost" and the party asked for a replacement. The tests act through
//! the real cash and cheque pages.

mod support;

use renox::prelude::*;
use wl_kernel::cheques::{ChequeStatus, RegisterCheque};
use wl_kernel::custody::HandoverStatus;
use wl_kernel::documents::today;
use wl_kernel::notify::NotifyRecipient;
use wl_payments::cash_cheques::{CashCheque, ChequeTick};
use wl_payments::cash_handover::PostingKind;
use wl_payments::cash_notices::CHEQUE_REPLACEMENT;
use wl_payments::{CashHandover, Cheque};

use support::{BLUE_DUNES, Booted, boot};

/// Sales rep S collected cheques #101 (AED 5,000.00) and #102 (AED 3,000.00)
/// in the field; the handover of the day (cash expected 0) and the cheques.
async fn rep_with_cheques(t: &Booted, rep: &User) -> (CashHandover, Cheque, Cheque) {
    let entity = t.entity("PGP").await;
    let first = t
        .register_cheque(
            RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "101", today(), 500_000)
                .collected_by(rep.id, true),
        )
        .await;
    let second = t
        .register_cheque(
            RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "102", today(), 300_000)
                .collected_by(rep.id, true),
        )
        .await;
    let handover = CashHandover::query()
        .where_(CashHandover::HOLDER_USER_ID.eq(rep.id))
        .first(t.db())
        .await
        .unwrap()
        .expect("registering a field cheque made the holder's handover");
    (handover, first, second)
}

/// `by` counts `handover` at `counted` dirhams with the cheques of `shown`
/// listed and the ones of `present` in hand.
async fn count_with(
    t: &Booted,
    by: &User,
    handover: &CashHandover,
    action: &str,
    counted: &str,
    shown: &[&Cheque],
    present: &[&Cheque],
) -> renox::testing::TestResponse {
    let shown_ids = shown
        .iter()
        .map(|c| c.id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let mut pairs: Vec<(String, String)> = vec![
        ("counted".to_owned(), counted.to_owned()),
        ("holder_present".to_owned(), "yes".to_owned()),
    ];
    if !shown.is_empty() {
        pairs.push(("shown".to_owned(), shown_ids));
    }
    for cheque in present {
        pairs.push(("present".to_owned(), cheque.id.to_string()));
    }
    let borrowed: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    t.cash_post(by, &format!("/cash/{}/{action}", handover.id), &borrowed)
        .await
}

/// AC1: rep S collected cheques #101 and #102; counter C ticks #101 present
/// and #102 missing: `short`; #101 can be deposited, #102 cannot.
#[renox::test]
async fn pay_18_a_missing_cheque_makes_the_handover_short_and_only_a_confirmed_one_is_deposited() {
    let t = boot().await;
    let rep = t.holder("Sami").await;
    let c = t.counter("Chandra").await;
    let staff = t.cheque_staff("Amal").await;
    let (handover, first, second) = rep_with_cheques(&t, &rep).await;
    assert_eq!(handover.expected, 0);
    // Not deposited before the handover confirms them.
    t.cheque_action(&staff, &first, "deposit", &[])
        .await
        .assert_status(422);

    count_with(
        &t,
        &c,
        &handover,
        "count",
        "0",
        &[&first, &second],
        &[&first],
    )
    .await
    .assert_ok();

    let after = t.reloaded(&handover).await;
    assert_eq!(after.status, HandoverStatus::Short);
    assert_eq!((after.counted, after.difference), (0, 0));
    assert_eq!(t.cheque(first.id).await.custody_handover_id, Some(handover.id));
    assert_eq!(t.cheque(second.id).await.custody_handover_id, None);
    let rows = CashCheque::of_handover(t.db(), handover.id).await.unwrap();
    let states: Vec<_> = rows.iter().map(|r| (r.cheque_id, r.state)).collect();
    assert_eq!(
        states,
        vec![(first.id, ChequeTick::Present), (second.id, ChequeTick::Missing)]
    );
    let events = t.cash_shortage.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].missing_cheques, vec![second.id]);
    assert_eq!(events[0].difference, 0);
    // No cash posting: a missing cheque is not cash.
    assert!(t.postings(&after).await.is_empty());
    assert_eq!(
        t.deposit_cheque(&staff, &first).await.status,
        ChequeStatus::Deposited
    );
    t.cheque_action(&staff, &second, "deposit", &[])
        .await
        .assert_status(422);
    assert_eq!(t.cheque(second.id).await.status, ChequeStatus::Received);
}

/// AC2 and AC3: `write_off` (and `recover`) is refused until #102 is found or
/// returned as lost; returned with the reason "lost", the party is asked for a
/// replacement; then `recover` is allowed and closes the handover with
/// nothing to recover.
#[renox::test]
async fn pay_18_a_missing_cheque_blocks_the_settlement_until_it_is_returned_as_lost() {
    let t = boot().await;
    let rep = t.holder("Sami").await;
    let c = t.counter("Chandra").await;
    let boss = t.cash_manager("Mia").await;
    let staff = t.cheque_staff("Amal").await;
    let (handover, first, second) = rep_with_cheques(&t, &rep).await;
    count_with(&t, &c, &handover, "count", "0", &[&first, &second], &[&first])
        .await
        .assert_ok();

    t.write_off_cash(&boss, &handover).await.assert_status(422);
    t.cash_post(&c, &format!("/cash/{}/recover", handover.id), &[])
        .await
        .assert_status(422);
    assert_eq!(t.reloaded(&handover).await.status, HandoverStatus::Short);

    // Returned with the reason "lost" from the handover's page.
    t.cash_post(
        &staff,
        &format!("/cash/{}/cheques/{}/lost", handover.id, second.id),
        &[("x", "")],
    )
    .await
    .assert_ok();

    assert_eq!(t.cheque(second.id).await.status, ChequeStatus::Returned);
    let rows = CashCheque::of_handover(t.db(), handover.id).await.unwrap();
    assert_eq!(rows[1].state, ChequeTick::ReturnedLost);
    let asked = t.notices.of_type(CHEQUE_REPLACEMENT);
    assert_eq!(asked.len(), 1);
    assert!(matches!(
        &asked[0].recipients[0],
        NotifyRecipient::Party { id } if *id == BLUE_DUNES
    ));
    assert_eq!(asked[0].data["number"], "102");
    assert_eq!(asked[0].data["amount"], "AED 3,000.00");

    t.cash_post(&c, &format!("/cash/{}/recover", handover.id), &[])
        .await
        .assert_ok();
    let done = t.reloaded(&handover).await;
    assert_eq!(done.status, HandoverStatus::Recovered);
    assert!(t.postings(&done).await.is_empty());
}

/// AC3, the other way: after the cheque is returned as lost, `write_off` is
/// allowed and writes off no cash.
#[renox::test]
async fn pay_18_after_a_cheque_is_returned_as_lost_the_handover_can_be_written_off_with_no_loss() {
    let t = boot().await;
    let rep = t.holder("Sami").await;
    let c = t.counter("Chandra").await;
    let boss = t.cash_manager("Mia").await;
    let staff = t.cheque_staff("Amal").await;
    let (handover, first, second) = rep_with_cheques(&t, &rep).await;
    count_with(&t, &c, &handover, "count", "0", &[&first, &second], &[&first])
        .await
        .assert_ok();
    t.cash_post(
        &staff,
        &format!("/cash/{}/cheques/{}/lost", handover.id, second.id),
        &[("x", "")],
    )
    .await
    .assert_ok();

    t.write_off_cash(&boss, &handover).await.assert_ok();

    let done = t.reloaded(&handover).await;
    assert_eq!(done.status, HandoverStatus::WrittenOff);
    assert_eq!(done.written_off, 0);
    assert!(
        t.postings(&done)
            .await
            .iter()
            .all(|p| p.kind != PostingKind::ShortageLoss),
        "a missing cheque is never a cash loss"
    );
}

/// A recount that finds the cheque confirms it; it can then be deposited and
/// the handover is `counted`.
#[renox::test]
async fn pay_18_a_recount_that_finds_the_cheque_confirms_it() {
    let t = boot().await;
    let rep = t.holder("Sami").await;
    let c = t.counter("Chandra").await;
    let d = t.counter("Dev").await;
    let staff = t.cheque_staff("Amal").await;
    let (handover, first, second) = rep_with_cheques(&t, &rep).await;
    count_with(&t, &c, &handover, "count", "0", &[&first, &second], &[&first])
        .await
        .assert_ok();
    // A recount that still does not find it keeps it missing.
    count_with(&t, &d, &handover, "recount", "0", &[&second], &[])
        .await
        .assert_ok();
    assert_eq!(t.reloaded(&handover).await.status, HandoverStatus::Short);
    assert_eq!(t.cheque(second.id).await.custody_handover_id, None);

    let e = t.counter("Eli").await;
    count_with(&t, &e, &handover, "recount", "0", &[&second], &[&second])
        .await
        .assert_ok();

    let after = t.reloaded(&handover).await;
    assert_eq!(after.status, HandoverStatus::Counted);
    assert_eq!(t.cheque(second.id).await.custody_handover_id, Some(handover.id));
    let rows = CashCheque::of_handover(t.db(), handover.id).await.unwrap();
    assert_eq!(rows[1].state, ChequeTick::Found);
    assert_eq!(
        t.deposit_cheque(&staff, &t.cheque(second.id).await).await.status,
        ChequeStatus::Deposited
    );
}

/// A cheque-only holder (08): expected cash 0, every expected cheque must be
/// ticked, a stranger cannot be ticked, and the handover is `counted` when
/// every cheque is present.
#[renox::test]
async fn pay_18_every_expected_cheque_is_ticked_and_all_present_is_counted() {
    let t = boot().await;
    let rep = t.holder("Sami").await;
    let c = t.counter("Chandra").await;
    let (handover, first, second) = rep_with_cheques(&t, &rep).await;

    // Nothing ticked: refused.
    t.cash_post(
        &c,
        &format!("/cash/{}/count", handover.id),
        &[("counted", "0")],
    )
    .await
    .assert_status(422);
    // One cheque left out: refused.
    count_with(&t, &c, &handover, "count", "0", &[&first], &[&first])
        .await
        .assert_status(422);
    assert_eq!(t.reloaded(&handover).await.status, HandoverStatus::Open);
    assert_eq!(t.cheque(first.id).await.custody_handover_id, None);

    count_with(
        &t,
        &c,
        &handover,
        "count",
        "0",
        &[&first, &second],
        &[&first, &second],
    )
    .await
    .assert_ok();

    let after = t.reloaded(&handover).await;
    assert_eq!(after.status, HandoverStatus::Counted);
    assert!(t.cash_shortage.all().is_empty());
    assert_eq!(t.cheque(first.id).await.custody_handover_id, Some(handover.id));
    assert_eq!(t.cheque(second.id).await.custody_handover_id, Some(handover.id));
}

/// The holder cannot count their own cheques, and cheques entered at the
/// office are not expected.
#[renox::test]
async fn pay_18_an_office_cheque_is_not_expected_and_the_holder_cannot_count() {
    let t = boot().await;
    let rep = t.counter("Sami").await;
    let entity = t.entity("PGP").await;
    let office = t
        .register_cheque(
            RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "300", today(), 100_000)
                .collected_by(rep.id, false),
        )
        .await;
    let (handover, first, second) = rep_with_cheques(&t, &rep).await;

    count_with(
        &t,
        &rep,
        &handover,
        "count",
        "0",
        &[&first, &second],
        &[&first, &second],
    )
    .await
    .assert_status(422);
    let c = t.counter("Chandra").await;
    // The office cheque is a stranger to the handover.
    count_with(
        &t,
        &c,
        &handover,
        "count",
        "0",
        &[&first, &second, &office],
        &[&first, &second, &office],
    )
    .await
    .assert_status(422);
    count_with(
        &t,
        &c,
        &handover,
        "count",
        "0",
        &[&first, &second],
        &[&first, &second],
    )
    .await
    .assert_ok();
    assert_eq!(t.cheque(office.id).await.custody_handover_id, None);
}
