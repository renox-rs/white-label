//! Alerting on pending issues (wl-inventory §6 `STK.RUL.06`, issue #239):
//! the storekeeper is told when a pending issue is recorded, at most once an
//! hour per item and location; still open after H hours it becomes a task for
//! the storekeeper and a notice to the manager; one that posted before H
//! hours creates nothing.

mod support;

use std::time::Duration;

use renox::prelude::*;
use wl_inventory::goods_receipt::lines_of;
use wl_inventory::shortage_alert::{self, OVERDUE_NOTICE, SHORT_NOTICE, TASK};
use wl_inventory::{DraftLine, Item, Location, PendingStatus, ShortageAlert, WlInventory};
use wl_kernel::Subject;
use wl_kernel::notify::NotifyRecipient;

use support::{Booted, boot, boot_with};

fn visit(n: i64) -> Subject {
    Subject::new("clinics.visit", n)
}

/// Waits `hours` hours and `minutes` minutes.
fn wait(t: &Booted, hours: u64, minutes: u64) {
    t.app
        .travel(Duration::from_secs(hours * 3600 + minutes * 60));
}

/// The users a notice of `kind` was sent to, one entry per notice.
fn told(t: &Booted, kind: &str) -> Vec<Vec<i64>> {
    t.notices
        .all()
        .iter()
        .filter(|request| request.type_key == kind)
        .map(|request| {
            request
                .recipients
                .iter()
                .filter_map(|recipient| match recipient {
                    NotifyRecipient::User { id } => Some(*id),
                    _ => None,
                })
                .collect()
        })
        .collect()
}

/// Consumes in the travelled time, as the visit would.
async fn consume_later(t: &Booted, item: &Item, branch: &Location, visit_no: i64, qty: i64) {
    t.app
        .at_travelled_time(async {
            t.consume(item, branch, qty, &visit(visit_no)).await;
        })
        .await;
}

/// A receipt of `quantity_milli` of `item` at `branch`, posted in the
/// travelled time.
async fn receive_later(t: &Booted, item: &Item, branch: &Location, quantity_milli: i64) {
    let (order, po_line) = t.order(branch, item, 100_000, 500).await;
    let receipt = t
        .receipt(
            &order,
            branch,
            vec![DraftLine::on(po_line.id, quantity_milli, quantity_milli)],
        )
        .await;
    assert_eq!(lines_of(t.db(), receipt.id).await.unwrap().len(), 1);
    let keeper = t.storekeeper("Amal").await;
    t.app
        .at_travelled_time(async {
            t.post_as(&keeper, &receipt).await.assert_ok();
        })
        .await;
}

/// Done when 1: a pending issue at 09:00 tells the storekeeper; at 09:00 the
/// next day, still open, it is a task for the storekeeper and a notice to the
/// manager, and only once.
#[renox::test]
async fn a_pending_issue_still_open_after_a_day_becomes_a_task_and_tells_the_manager() {
    let t = boot().await;
    let clerk = t.clerk("Cass").await;
    let manager = t.user_with("Mia", "stock_manager").await;
    let item = t.item("GAUZE").await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;

    t.consume(&item, &branch, 3_000, &visit(1)).await;

    let first = told(&t, SHORT_NOTICE);
    assert_eq!(first, vec![vec![clerk.id]]);
    assert!(told(&t, OVERDUE_NOTICE).is_empty());
    assert!(t.tickets.opened().is_empty());

    // 23 hours later: not yet.
    wait(&t, 23, 0);
    t.app.run_scheduled(TASK).await.unwrap();
    assert!(t.tickets.opened().is_empty());
    assert!(told(&t, OVERDUE_NOTICE).is_empty());

    // 24 hours after the pending issue: the task and the manager's notice.
    wait(&t, 1, 0);
    t.app.run_scheduled(TASK).await.unwrap();
    let tasks = t.tickets.opened();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].source_key.as_deref(),
        Some(shortage_alert::task_key(branch.id, item.id).as_str())
    );
    assert_eq!(tasks[0].subject, branch.subject());
    assert_eq!(
        tasks[0].assignee,
        Some(wl_kernel::tickets::Assignee::user(clerk.id))
    );
    assert!(tasks[0].title.contains("GAUZE"), "{}", tasks[0].title);
    assert_eq!(told(&t, OVERDUE_NOTICE), vec![vec![manager.id]]);

    // The next run does not repeat either.
    wait(&t, 1, 0);
    t.app.run_scheduled(TASK).await.unwrap();
    assert_eq!(t.tickets.opened().len(), 1);
    assert_eq!(told(&t, OVERDUE_NOTICE).len(), 1);
}

/// Done when 2: a pending issue that posted at 15:00 creates nothing the next
/// day, and the shortage is forgotten.
#[renox::test]
async fn a_pending_issue_that_posted_in_time_creates_no_task() {
    let t = boot().await;
    let _clerk = t.clerk("Cass").await;
    let _manager = t.user_with("Mia", "stock_manager").await;
    let item = t.item("GAUZE").await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.consume(&item, &branch, 3_000, &visit(1)).await;
    assert_eq!(ShortageAlert::query().count(t.db()).await.unwrap(), 1);

    // Six hours later stock arrives and the pending issue posts.
    wait(&t, 6, 0);
    receive_later(&t, &item, &branch, 10_000).await;
    let posted = wl_inventory::PendingIssue::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(posted.status, PendingStatus::Posted);

    // The next day nothing is created, and the shortage is forgotten.
    wait(&t, 18, 0);
    t.app.run_scheduled(TASK).await.unwrap();
    assert!(t.tickets.opened().is_empty());
    assert!(told(&t, OVERDUE_NOTICE).is_empty());
    assert_eq!(ShortageAlert::query().count(t.db()).await.unwrap(), 0);
}

/// The storekeeper is told once an hour per item and location, not once per
/// consumption; another item at the location is told on its own.
#[renox::test]
async fn the_storekeeper_is_told_at_most_once_an_hour_per_item_and_location() {
    let t = boot().await;
    let clerk = t.clerk("Cass").await;
    let gauze = t.item("GAUZE").await;
    let tape = t.item("TAPE").await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;

    t.consume(&gauze, &branch, 1_000, &visit(1)).await;
    wait(&t, 0, 10);
    consume_later(&t, &gauze, &branch, 2, 1_000).await;
    consume_later(&t, &tape, &branch, 3, 1_000).await;
    // Two items, three consumptions: one notice for GAUZE, one for TAPE.
    assert_eq!(told(&t, SHORT_NOTICE), vec![vec![clerk.id], vec![clerk.id]]);

    // An hour after the first notice GAUZE is told again.
    wait(&t, 0, 55);
    consume_later(&t, &gauze, &branch, 4, 1_000).await;
    assert_eq!(told(&t, SHORT_NOTICE).len(), 3);
    assert_eq!(t.open_pending().await.len(), 4);
}

/// One task and one notice per item and location, however many pending
/// issues are open.
#[renox::test]
async fn several_pending_issues_of_one_item_make_one_task() {
    let t = boot().await;
    let _clerk = t.clerk("Cass").await;
    let _manager = t.user_with("Mia", "stock_manager").await;
    let item = t.item("GAUZE").await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.consume(&item, &branch, 1_000, &visit(1)).await;
    wait(&t, 5, 0);
    consume_later(&t, &item, &branch, 2, 1_000).await;

    wait(&t, 19, 0);
    t.app.run_scheduled(TASK).await.unwrap();

    assert_eq!(t.tickets.opened().len(), 1);
    assert_eq!(told(&t, OVERDUE_NOTICE).len(), 1);
}

/// The hours are a setting: with H = 2 the task comes after two hours.
#[renox::test]
async fn the_hours_before_the_task_are_a_setting() {
    let t = boot_with(WlInventory::new().pending_alert_hours(2)).await;
    let _clerk = t.clerk("Cass").await;
    let item = t.item("GAUZE").await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.consume(&item, &branch, 1_000, &visit(1)).await;

    wait(&t, 1, 30);
    t.app.run_scheduled(TASK).await.unwrap();
    assert!(t.tickets.opened().is_empty());

    wait(&t, 0, 31);
    t.app.run_scheduled(TASK).await.unwrap();
    assert_eq!(t.tickets.opened().len(), 1);
}
