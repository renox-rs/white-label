//! BL-DLV-03 (#159) and BL-DLV-11 (#167), `FIN.CSH.01`, `X-11`: the cash of a
//! run is the driver's handover in the cash custody. The run's period ends
//! when the driver is back, and the run closes when the handover is counted
//! (counted or short), not by a record of this module.

mod support;

use renox::prelude::Model;
use wl_delivery::{Run, RunStatus};

use support::{World, boot_with_custody};

/// A COD delivery of AED 100 delivered in full on a run that is back at the
/// depot.
async fn back_with_cash(w: &World) -> Run {
    let delivery = w.delivered_cod(10_000).await;
    let run = Run::find_or_404(w.db(), delivery.run_id.unwrap())
        .await
        .unwrap();
    w.post(&w.driver, &format!("/driver/runs/{}/return", run.id), &[])
        .await
        .assert_ok();
    let back = w.reload_run(&run).await;
    assert_eq!(back.status, RunStatus::ReturnedToDepot);
    back
}

/// The driver's return ends the cash period of the run in the custody.
#[renox::test]
async fn bringing_the_run_back_ends_the_drivers_cash_period() {
    let w = boot_with_custody().await;
    let run = back_with_cash(&w).await;

    let ended = w.custody.ended();
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].0, w.driver.id);
    assert_eq!(ended[0].1, run.subject());
}

/// The run does not close until the handover is counted; the record of the
/// module is refused; the count in the custody closes the run by itself and
/// `RunClosed` carries the cash.
#[renox::test]
async fn the_run_closes_when_the_handover_is_counted_in_the_custody() {
    let w = boot_with_custody().await;
    let run = back_with_cash(&w).await;

    w.post(&w.dispatcher, &format!("/runs/{}/close", run.id), &[])
        .await
        .assert_status(422)
        .assert_see("not counted");
    w.post(
        &w.dispatcher,
        &format!("/runs/{}/cash", run.id),
        &[("amount", "100.00")],
    )
    .await
    .assert_status(422)
    .assert_see("cash handovers");
    assert_eq!(w.reload_run(&run).await.status, RunStatus::ReturnedToDepot);

    w.custody
        .count(
            w.state(),
            w.driver.id,
            &run.subject(),
            10_000,
            w.dispatcher.id,
        )
        .await;

    let closed = w.reload_run(&run).await;
    assert_eq!(closed.status, RunStatus::Closed);
    assert!(closed.closed_at.is_some());
    assert_eq!(closed.cash_counted_at, None);
    let events = w.run_closed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].cash_collected, 10_000);
}

/// A short count does not block the close: only "counted" does.
#[renox::test]
async fn a_short_count_still_closes_the_run() {
    let w = boot_with_custody().await;
    let run = back_with_cash(&w).await;

    w.custody
        .count(
            w.state(),
            w.driver.id,
            &run.subject(),
            9_000,
            w.dispatcher.id,
        )
        .await;

    assert_eq!(w.reload_run(&run).await.status, RunStatus::Closed);
}

/// The run page says the cash is counted in the cash handovers and offers no
/// form to count it here.
#[renox::test]
async fn the_run_page_sends_the_count_to_the_cash_handovers() {
    let w = boot_with_custody().await;
    let run = back_with_cash(&w).await;

    w.app.acting_as(&w.dispatcher);
    w.app
        .get(&format!("/runs/{}", run.id))
        .await
        .assert_ok()
        .assert_see("counted in the cash handovers")
        .assert_dont_see("name=\"amount\"");
}

/// A run that held no cash closes without a count.
#[renox::test]
async fn a_run_without_cash_closes_without_a_count() {
    let w = boot_with_custody().await;
    let delivery = w.simple_delivery().await;
    let out = w.out_for_delivery(&delivery).await;
    let line = w.lines(&out).await.remove(0);
    w.deliver(
        &w.driver,
        &out,
        &support::stop_fields(&[(line.id, "10", "0", "", "")], "0"),
    )
    .await
    .assert_status(303);
    let run = Run::find_or_404(w.db(), out.run_id.unwrap()).await.unwrap();

    w.post(&w.driver, &format!("/driver/runs/{}/return", run.id), &[])
        .await
        .assert_ok();

    assert_eq!(w.reload_run(&run).await.status, RunStatus::Closed);
}
