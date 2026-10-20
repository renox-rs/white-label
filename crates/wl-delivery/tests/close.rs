//! BL-DLV-11 (#167), `X-11`: a run closes only when its cash is counted and
//! every undelivered delivery and refused line is received back at the depot
//! or reported missing. The system records the quantities on the lines and
//! emits `DeliveryRefusedReceived` and `RunClosed`; a missing item gives the
//! dispatcher a task and its delivery stays where it is.

mod support;

use renox::prelude::*;
use wl_delivery::{Delivery, DeliveryLine, DeliveryStatus, NewRun, Run, RunStatus, Trace, VehicleType};
use wl_kernel::documents::today;
use wl_kernel::tickets::TASK_CATEGORY;

use support::{VAN, World, boot, stop_fields, user_with};

/// A run of `driver` with `deliveries` on it, dispatched today.
async fn dispatched_run(w: &World, driver: &renox::prelude::User, deliveries: &[&Delivery]) -> Run {
    w.valid_documents_for(driver.id, "2099-12-31");
    let run = wl_delivery::plan::create_run(
        w.state(),
        NewRun::new(today(), driver.id).vehicle(VehicleType::Van, Some(VAN)),
    )
    .await
    .unwrap();
    for delivery in deliveries {
        w.assign(&w.dispatcher, delivery, &run).await.assert_ok();
    }
    w.dispatch(&w.dispatcher, &run).await.assert_ok();
    w.reload_run(&run).await
}

/// The driver brings the run back.
async fn bring_back(w: &World, run: &Run) -> renox::testing::TestResponse {
    w.post(&w.driver, &format!("/driver/runs/{}/return", run.id), &[]).await
}

struct Scene {
    run: Run,
    /// Delivered with 3 of 10 refused (damaged).
    refused: Delivery,
    refused_line: DeliveryLine,
    /// Two deliveries whose visit failed.
    first: Delivery,
    second: Delivery,
}

/// A run with one delivery handed over with a refused line (qty 3 of 10,
/// `collect` fils to collect) and two whose visit failed; the driver is back
/// at the depot.
async fn back_at_the_depot(w: &World, collect: i64) -> Scene {
    let refused = w.request(&[("Tiles", 10_000, None)], collect).await;
    let first = w.simple_delivery().await;
    let second = w.simple_delivery().await;
    let run = dispatched_run(w, &w.driver, &[&refused, &first, &second]).await;
    let line = w.lines(&refused).await.remove(0);
    let cash = format!("{}.{:02}", collect / 100, collect % 100);
    let fields = stop_fields(&[(line.id, "7", "3", "damaged", "damage-photo-1")], &cash);
    w.deliver(&w.driver, &refused, &fields).await.assert_ok();
    w.fail(&w.driver, &first, "not_home").await.assert_ok();
    w.fail(&w.driver, &second, "not_home").await.assert_ok();
    bring_back(w, &run).await.assert_ok();
    let run = w.reload_run(&run).await;
    assert_eq!(run.status, RunStatus::ReturnedToDepot);
    Scene {
        run,
        refused: w.reload(&refused).await,
        refused_line: w.lines(&refused).await.remove(0),
        first: w.reload(&first).await,
        second: w.reload(&second).await,
    }
}

fn receipts_path(run: &Run) -> String {
    format!("/runs/{}/receipts", run.id)
}

/// AC1: a run back at the depot with 2 undelivered deliveries and 1 refused
/// line (qty 3); the depot user ticks all received and the cash handover is
/// counted (none was collected): the run closes, `DeliveryRefusedReceived`
/// (3 received, 0 missing) and `RunClosed` are emitted.
#[renox::test]
async fn ticking_everything_received_closes_the_run_and_emits_the_events() {
    let w = boot().await;
    let s = back_at_the_depot(&w, 0).await;
    let (a, b) = (format!("item_{}_0", s.first.id), format!("item_{}_0", s.second.id));
    let line = format!("received_{}_{}", s.refused.id, s.refused_line.id);

    w.post(
        &w.dispatcher,
        &receipts_path(&s.run),
        &[(a.as_str(), "received"), (b.as_str(), "received"), (line.as_str(), "3")],
    )
    .await
    .assert_ok();

    let run = w.reload_run(&s.run).await;
    assert_eq!(run.status, RunStatus::Closed);
    assert!(run.closed_at.is_some());
    let events = w.refused_received.all();
    assert_eq!(events.len(), 1, "only the delivery with refused lines");
    assert_eq!(events[0].delivery, s.refused.delivery_id());
    assert_eq!(events[0].source, s.refused.source());
    assert_eq!(events[0].run, run.run_id());
    assert_eq!(events[0].lines.len(), 1);
    assert_eq!(events[0].lines[0].description, "Tiles");
    assert_eq!(events[0].lines[0].refused_milli, 3_000);
    assert_eq!(events[0].lines[0].received_milli, 3_000);
    assert_eq!(events[0].lines[0].missing_milli, 0);
    assert_eq!(events[0].lines[0].refusal_reason.as_deref(), Some("damaged"));
    let closed = w.run_closed.all();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].run, run.run_id());
    assert_eq!(closed[0].driver, w.driver.id);
    assert_eq!(closed[0].vehicle, Some(VAN));
    assert_eq!((closed[0].delivered, closed[0].undelivered, closed[0].missing), (1, 2, 0));
    // The quantities are on the line, with who and when.
    let line = w.lines(&s.refused).await.remove(0);
    assert_eq!((line.refused_received_milli, line.refused_missing_milli), (3_000, 0));
    assert!(line.accounted_at.is_some());
    assert_eq!(line.accounted_by, Some(w.dispatcher.id));
    // The deliveries did not move; the received ones left a depot receipt.
    assert_eq!(w.reload(&s.refused).await.status, DeliveryStatus::Delivered);
    assert_eq!(w.reload(&s.first).await.status, DeliveryStatus::Attempted);
    let receipts = Trace::query()
        .where_(Trace::KIND.eq("depot_receipt"))
        .count(w.db())
        .await
        .unwrap();
    assert_eq!(receipts, 2);
    assert!(w.tickets.opened().is_empty(), "nothing is missing");
}

/// AC2: the refused line ticked 2 received and 1 missing: the event carries 2
/// and 1; the delivery stays `delivered`; the missing goods give the
/// dispatcher a task.
#[renox::test]
async fn a_part_received_refused_line_reports_the_missing_part() {
    let w = boot().await;
    let s = back_at_the_depot(&w, 0).await;
    let (a, b) = (format!("item_{}_0", s.first.id), format!("item_{}_0", s.second.id));
    let received = format!("received_{}_{}", s.refused.id, s.refused_line.id);
    let missing = format!("missing_{}_{}", s.refused.id, s.refused_line.id);

    w.post(
        &w.dispatcher,
        &receipts_path(&s.run),
        &[
            (a.as_str(), "received"),
            (b.as_str(), "received"),
            (received.as_str(), "2"),
            (missing.as_str(), "1"),
        ],
    )
    .await
    .assert_ok();

    assert_eq!(w.reload_run(&s.run).await.status, RunStatus::Closed);
    let events = w.refused_received.all();
    assert_eq!(events[0].lines[0].received_milli, 2_000);
    assert_eq!(events[0].lines[0].missing_milli, 1_000);
    assert_eq!(w.reload(&s.refused).await.status, DeliveryStatus::Delivered);
    let line = w.lines(&s.refused).await.remove(0);
    assert_eq!((line.refused_received_milli, line.refused_missing_milli), (2_000, 1_000));
    assert_eq!(w.run_closed.all()[0].missing, 1);
    let tasks = w.tickets.opened();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].category_key, TASK_CATEGORY);
    assert!(tasks[0].title.contains(&s.refused.number));
}

/// AC3: one of the undelivered deliveries is reported missing: the run
/// closes, the dispatcher gets a task, and the delivery stays `attempted`
/// (no depot receipt: its untraced clock does not restart).
#[renox::test]
async fn an_undelivered_delivery_reported_missing_gets_a_task_and_stays_attempted() {
    let w = boot().await;
    let s = back_at_the_depot(&w, 0).await;
    let (a, b) = (format!("item_{}_0", s.first.id), format!("item_{}_0", s.second.id));
    let line = format!("received_{}_{}", s.refused.id, s.refused_line.id);

    w.post(
        &w.dispatcher,
        &receipts_path(&s.run),
        &[(a.as_str(), "missing"), (b.as_str(), "received"), (line.as_str(), "3")],
    )
    .await
    .assert_ok();

    assert_eq!(w.reload_run(&s.run).await.status, RunStatus::Closed);
    assert_eq!(w.reload(&s.first).await.status, DeliveryStatus::Attempted);
    let tasks = w.tickets.opened();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].category_key, TASK_CATEGORY);
    assert!(tasks[0].title.contains(&s.first.number));
    assert_eq!(tasks[0].subject, s.first.delivery_id().subject());
    let first_receipts = Trace::query()
        .where_(Trace::DELIVERY_ID.eq(s.first.id))
        .where_(Trace::KIND.eq("depot_receipt"))
        .count(w.db())
        .await
        .unwrap();
    assert_eq!(first_receipts, 0);
    let second_receipts = Trace::query()
        .where_(Trace::DELIVERY_ID.eq(s.second.id))
        .where_(Trace::KIND.eq("depot_receipt"))
        .count(w.db())
        .await
        .unwrap();
    assert_eq!(second_receipts, 1);
    assert_eq!(w.run_closed.all()[0].missing, 1);
}

/// AC4: the cash handover is not counted yet: the run does not close, even
/// with every item ticked; counting it (by someone else than the driver)
/// closes it.
#[renox::test]
async fn the_run_does_not_close_until_the_cash_is_counted() {
    let w = boot().await;
    let s = back_at_the_depot(&w, 10_000).await;
    let (a, b) = (format!("item_{}_0", s.first.id), format!("item_{}_0", s.second.id));
    let line = format!("received_{}_{}", s.refused.id, s.refused_line.id);

    w.post(
        &w.dispatcher,
        &receipts_path(&s.run),
        &[(a.as_str(), "received"), (b.as_str(), "received"), (line.as_str(), "3")],
    )
    .await
    .assert_ok();
    assert_eq!(w.reload_run(&s.run).await.status, RunStatus::ReturnedToDepot);
    w.post(&w.dispatcher, &format!("/runs/{}/close", s.run.id), &[])
        .await
        .assert_status(422)
        .assert_see("not counted");
    assert!(w.run_closed.all().is_empty());
    assert!(w.refused_received.all().is_empty());

    // The driver does not count their own cash and has no right to.
    w.post(&w.driver, &format!("/runs/{}/cash", s.run.id), &[("amount", "100.00")])
        .await
        .assert_forbidden();
    w.post(&w.dispatcher, &format!("/runs/{}/cash", s.run.id), &[("amount", "x")])
        .await
        .assert_status(422);
    w.post(&w.dispatcher, &format!("/runs/{}/cash", s.run.id), &[("amount", "100.00")])
        .await
        .assert_ok();

    let run = w.reload_run(&s.run).await;
    assert_eq!(run.status, RunStatus::Closed);
    assert_eq!(run.cash_counted_amount, Some(10_000));
    assert_eq!(run.cash_counted_by, Some(w.dispatcher.id));
    assert_eq!(w.run_closed.all()[0].cash_collected, 10_000);
}

/// The driver who also holds `runs.dispatch` cannot count the cash of their
/// own run.
#[renox::test]
async fn the_runs_driver_cannot_count_their_own_cash() {
    let w = boot().await;
    let dora = user_with(
        &w,
        "Dora",
        &[
            wl_delivery::permissions::RUNS_DRIVE,
            wl_delivery::permissions::COMPLETE,
            wl_delivery::permissions::RUNS_DISPATCH,
        ],
    )
    .await;
    let delivery = w.request(&[("Tiles", 10_000, None)], 10_000).await;
    let run = dispatched_run(&w, &dora, &[&delivery]).await;
    let line = w.lines(&delivery).await.remove(0);
    let fields = stop_fields(&[(line.id, "10", "0", "", "")], "100.00");
    w.deliver(&dora, &delivery, &fields).await.assert_ok();
    w.post(&dora, &format!("/driver/runs/{}/return", run.id), &[]).await.assert_ok();
    assert_eq!(w.reload_run(&run).await.status, RunStatus::ReturnedToDepot);

    w.post(&dora, &format!("/runs/{}/cash", run.id), &[("amount", "100.00")])
        .await
        .assert_status(422);

    assert_eq!(w.reload_run(&run).await.cash_counted_at, None);
}

/// A run with every stop delivered in full and no cash closes as it comes
/// back: there is nothing to account for.
#[renox::test]
async fn a_run_with_nothing_to_account_for_closes_when_it_is_back() {
    let w = boot().await;
    let delivery = w.simple_delivery().await;
    let run = dispatched_run(&w, &w.driver, &[&delivery]).await;
    let line = w.lines(&delivery).await.remove(0);
    let fields = stop_fields(&[(line.id, "10", "0", "", "")], "0");
    w.deliver(&w.driver, &delivery, &fields).await.assert_ok();

    bring_back(&w, &run).await.assert_ok();

    assert_eq!(w.reload_run(&run).await.status, RunStatus::Closed);
    assert_eq!(w.run_closed.all().len(), 1);
    assert!(w.refused_received.all().is_empty());
}

/// The run cannot come back while a stop has no outcome, only its driver
/// brings it back, and it does not close before it is back.
#[renox::test]
async fn the_run_returns_only_with_every_stop_decided_and_by_its_driver() {
    let w = boot().await;
    let open = w.simple_delivery().await;
    let done = w.simple_delivery().await;
    let run = dispatched_run(&w, &w.driver, &[&open, &done]).await;
    w.fail(&w.driver, &done, "not_home").await.assert_ok();

    bring_back(&w, &run).await.assert_status(422).assert_see("no outcome");
    w.post(&w.other_driver, &format!("/driver/runs/{}/return", run.id), &[])
        .await
        .assert_status(422);
    w.post(&w.dispatcher, &format!("/driver/runs/{}/return", run.id), &[])
        .await
        .assert_forbidden();
    w.post(&w.dispatcher, &format!("/runs/{}/close", run.id), &[])
        .await
        .assert_status(422);
    assert_eq!(w.reload_run(&run).await.status, RunStatus::Dispatched);

    w.fail(&w.driver, &open, "not_home").await.assert_ok();
    bring_back(&w, &run).await.assert_ok();
    let back = w.reload_run(&run).await;
    assert_eq!(back.status, RunStatus::ReturnedToDepot);
    assert!(back.returned_at.is_some());
}

/// The ticks are checked: only items of the run, whole undelivered
/// deliveries, quantities that add up to the refused quantity; only a holder
/// of `runs.dispatch` ticks; nothing is saved by a refused request.
#[renox::test]
async fn the_depot_ticks_are_checked() {
    let w = boot().await;
    let s = back_at_the_depot(&w, 0).await;
    let received = format!("received_{}_{}", s.refused.id, s.refused_line.id);
    let missing = format!("missing_{}_{}", s.refused.id, s.refused_line.id);
    let first = format!("item_{}_0", s.first.id);

    w.post(&w.driver, &receipts_path(&s.run), &[(first.as_str(), "received")])
        .await
        .assert_forbidden();
    // 2 + 2 is not the refused quantity of 3.
    w.post(
        &w.dispatcher,
        &receipts_path(&s.run),
        &[(received.as_str(), "2"), (missing.as_str(), "2")],
    )
    .await
    .assert_status(422);
    // More than was refused.
    w.post(&w.dispatcher, &receipts_path(&s.run), &[(received.as_str(), "4")])
        .await
        .assert_status(422);
    w.post(&w.dispatcher, &receipts_path(&s.run), &[(first.as_str(), "maybe")])
        .await
        .assert_status(422);

    // Partly received is only for refused lines: the missing part is what is left.
    w.post(&w.dispatcher, &receipts_path(&s.run), &[(missing.as_str(), "1")])
        .await
        .assert_ok();
    let line = w.lines(&s.refused).await.remove(0);
    assert_eq!(line.refused_received_milli, 0, "the lines get their quantities at close");
    assert_eq!(w.reload_run(&s.run).await.status, RunStatus::ReturnedToDepot);
}

/// The run page lists the items for the depot user once the run is back.
#[renox::test]
async fn the_run_page_lists_the_items_to_account_for() {
    let w = boot().await;
    let s = back_at_the_depot(&w, 0).await;

    w.app.acting_as(&w.dispatcher);
    w.app
        .get(&format!("/runs/{}", s.run.id))
        .await
        .assert_ok()
        .assert_see("At the depot")
        .assert_see(&s.first.number)
        .assert_see("Tiles");
}
