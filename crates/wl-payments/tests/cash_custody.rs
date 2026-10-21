//! Cash custody (wl-payments §3 *Cash holder*, *Cash handover*, `FIN.CSH.01`;
//! base for BL-PAY-17 (#110), BL-PAY-20 (#113), BL-PAY-21 (#114) and BL-PAY-23
//! (#116)): other modules collect cash into a holder's handover through the
//! kernel contract; the cash of a period adds up in one handover, one source
//! adds its cash once, and a counted handover takes no more.

mod support;

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::custody::{CashCollection, HandoverPeriod, HandoverStatus, PeriodKind, custody};
use wl_payments::{CashHandover, CashHolder};

use support::boot;
use support::cash::{at, date};

fn delivery(n: i64) -> Subject {
    Subject::new("delivery.delivery", n)
}

fn run(n: i64) -> Subject {
    Subject::new("delivery.run", n)
}

/// The cash a holder collects on one day adds up in one handover that is
/// `open`, with an entry per source, the holder's branch and its expected
/// amount; a member of staff sees it on the handover page.
#[renox::test]
async fn cash_collected_in_a_period_adds_up_in_one_open_handover() {
    let t = boot().await;
    let rana = t.holder("Rana").await;
    let first = t.collect(&rana, "2027-04-01", 100_000, 1).await;
    let second = t.collect(&rana, "2027-04-01", 50_000, 2).await;

    assert_eq!(first.id, second.id);
    let handover = t.reloaded(&first).await;
    assert_eq!(handover.status, HandoverStatus::Open);
    assert_eq!(handover.expected, 150_000);
    assert_eq!(handover.holder_user_id, rana.id);
    assert_eq!(handover.responsible_user_id, rana.id);
    assert_eq!(handover.period_kind, PeriodKind::Day);
    assert_eq!(handover.location.as_deref(), Some("Dubai branch"));
    assert_eq!(t.entries(&handover).await.len(), 2);
    assert!(
        CashHolder::find(t.db(), handover.holder_id)
            .await
            .unwrap()
            .is_some()
    );

    let viewer = t.counter("Chandra").await;
    t.app.acting_as(&viewer);
    t.app
        .get(&format!("/cash/{}", handover.id))
        .await
        .assert_ok();
    t.app.get("/cash").await.assert_ok();
}

/// One source adds its cash once: the same delivery collected twice answers
/// the same handover and changes nothing.
#[renox::test]
async fn one_source_adds_its_cash_once() {
    let t = boot().await;
    let rana = t.holder("Rana").await;
    let first = t.collect(&rana, "2027-04-01", 100_000, 7).await;
    let again = t.collect(&rana, "2027-04-01", 100_000, 7).await;

    assert_eq!(first.id, again.id);
    assert_eq!(t.reloaded(&first).await.expected, 100_000);
    assert_eq!(t.entries(&first).await.len(), 1);
}

/// Another day, another holder and a run are other handovers; a run is its
/// own period.
#[renox::test]
async fn each_holder_period_and_run_is_its_own_handover() {
    let t = boot().await;
    let rana = t.holder("Rana").await;
    let omar = t.holder("Omar").await;
    let monday = t.collect(&rana, "2027-04-01", 10_000, 1).await;
    let tuesday = t.collect(&rana, "2027-04-02", 10_000, 2).await;
    let other = t.collect(&omar, "2027-04-01", 10_000, 3).await;
    let by_run = custody(t.state())
        .collect(
            t.state(),
            CashCollection::new(
                rana.id,
                HandoverPeriod::run(run(5), date("2027-04-01"), Some(at("2027-04-01T14:00:00Z"))),
                20_000,
                delivery(4),
            ),
        )
        .await
        .unwrap()
        .unwrap();

    let ids = [monday.id, tuesday.id, other.id, by_run.0];
    for (i, a) in ids.iter().enumerate() {
        for b in &ids[i + 1..] {
            assert_ne!(a, b);
        }
    }
    let run_handover = t.handover(by_run).await;
    assert_eq!(run_handover.period_kind, PeriodKind::Run);
    assert_eq!(run_handover.period_subject(), Some(run(5)));
    let found = custody(t.state())
        .handover_of(t.db(), rana.id, &run(5))
        .await
        .unwrap()
        .expect("the handover of the run");
    assert_eq!(found.id, by_run);
    assert_eq!(found.expected, 20_000);
}

/// Cash collected for a delivery is taken out again while the handover is
/// `open` (a delivery reversed before the count); once it is counted the
/// release is refused, and a counted handover takes no more cash.
#[renox::test]
async fn cash_is_released_while_open_and_a_counted_handover_takes_no_more() {
    let t = boot().await;
    let rana = t.holder("Rana").await;
    let chandra = t.counter("Chandra").await;
    let handover = t.collect(&rana, "2027-04-01", 100_000, 1).await;
    t.collect(&rana, "2027-04-01", 40_000, 2).await;

    assert!(
        custody(t.state())
            .release(t.state(), delivery(2))
            .await
            .unwrap()
    );
    assert_eq!(t.reloaded(&handover).await.expected, 100_000);
    // Released once; nothing was collected for another source.
    assert!(
        !custody(t.state())
            .release(t.state(), delivery(2))
            .await
            .unwrap()
    );
    assert!(
        !custody(t.state())
            .release(t.state(), delivery(99))
            .await
            .unwrap()
    );

    t.count_cash(&chandra, &handover, "1000.00")
        .await
        .assert_ok();
    assert_eq!(t.reloaded(&handover).await.status, HandoverStatus::Counted);
    let refused = custody(t.state()).release(t.state(), delivery(1)).await;
    assert!(matches!(
        refused,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
    let late = custody(t.state())
        .collect(
            t.state(),
            CashCollection::new(
                rana.id,
                HandoverPeriod::day(date("2027-04-01")),
                5_000,
                delivery(3),
            ),
        )
        .await;
    assert!(matches!(
        late,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
    assert_eq!(t.reloaded(&handover).await.expected, 100_000);
}

/// The end of a run's period is the earlier of what is recorded and what the
/// run reports; a handover that is not open keeps its end.
#[renox::test]
async fn the_end_of_a_period_is_the_earlier_of_the_two_ends() {
    let t = boot().await;
    let rana = t.holder("Rana").await;
    let planned = at("2027-04-01T14:00:00Z");
    let id = custody(t.state())
        .collect(
            t.state(),
            CashCollection::new(
                rana.id,
                HandoverPeriod::run(run(5), date("2027-04-01"), Some(planned)),
                20_000,
                delivery(4),
            ),
        )
        .await
        .unwrap()
        .unwrap();

    // A later report does not move the end back.
    custody(t.state())
        .end_period(t.state(), rana.id, run(5), at("2027-04-01T16:00:00Z"))
        .await
        .unwrap();
    assert_eq!(t.handover(id).await.period_end, Some(planned));
    // An earlier one (the run returned) does.
    let returned = at("2027-04-01T11:00:00Z");
    custody(t.state())
        .end_period(t.state(), rana.id, run(5), returned)
        .await
        .unwrap();
    assert_eq!(t.handover(id).await.period_end, Some(returned));
    let handover: CashHandover = t.handover(id).await;
    assert_eq!(handover.ends_at(), returned);
}
