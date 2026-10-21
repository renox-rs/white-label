//! BL-DLV-03 (#159) and BL-DLV-11 (#167), `FIN.CSH.01`, `X-11`, with the real
//! `wl-payments` as the cash custody: the run closes when the driver's
//! handover of the run is counted in the cash pages (counted or short), and
//! the record of this module is refused while a custody is provided.

mod support;

use renox::prelude::Model;
use wl_delivery::{Run, RunStatus, close};
use wl_kernel::custody::HandoverStatus;
use wl_payments::permissions as pay;

use support::{World, boot_with_real_payments, user_with};

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

/// The count of the real handover closes the run and `RunClosed` carries the
/// cash; before it neither the close nor the module's own record works.
#[renox::test]
async fn the_run_closes_when_the_real_handover_is_counted() {
    let w = boot_with_real_payments().await;
    let run = back_with_cash(&w).await;
    let handover = close::handover_of(w.state(), &run)
        .await
        .unwrap()
        .expect("the driver's handover of the run");
    assert_eq!(handover.expected, 10_000);
    assert_eq!(handover.status, HandoverStatus::Open);

    w.post(&w.dispatcher, &format!("/runs/{}/close", run.id), &[])
        .await
        .assert_status(422);
    w.post(
        &w.dispatcher,
        &format!("/runs/{}/cash", run.id),
        &[("amount", "100.00")],
    )
    .await
    .assert_status(422);
    assert_eq!(w.reload_run(&run).await.status, RunStatus::ReturnedToDepot);

    let counter = user_with(&w, "Chandra", &[pay::CASH_VIEW, pay::CASH_COUNT]).await;
    w.post(
        &counter,
        &format!("/cash/{}/count", handover.id),
        &[("counted", "100.00")],
    )
    .await
    .assert_ok();

    let closed = w.reload_run(&run).await;
    assert_eq!(closed.status, RunStatus::Closed);
    assert_eq!(closed.cash_counted_at, None, "the module's record is unused");
    let events = w.run_closed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].cash_collected, 10_000);
    let counted = close::handover_of(w.state(), &closed)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(counted.status, HandoverStatus::Counted);
}

/// A short count does not block the close: the handover is `short` by AED 10
/// and the run is closed all the same.
#[renox::test]
async fn a_short_real_count_still_closes_the_run_and_leaves_the_shortage() {
    let w = boot_with_real_payments().await;
    let run = back_with_cash(&w).await;
    let handover = close::handover_of(w.state(), &run).await.unwrap().unwrap();
    let counter = user_with(&w, "Chandra", &[pay::CASH_VIEW, pay::CASH_COUNT]).await;

    w.post(
        &counter,
        &format!("/cash/{}/count", handover.id),
        &[("counted", "90.00")],
    )
    .await
    .assert_ok();

    assert_eq!(w.reload_run(&run).await.status, RunStatus::Closed);
    let after = close::handover_of(w.state(), &run).await.unwrap().unwrap();
    assert_eq!(after.status, HandoverStatus::Short);
    assert_eq!(after.difference, 1_000);
}
