//! Chasing returns to supplier that stay `dispatched` (wl-inventory §6
//! `STK.RUL.08`, issue #226): after N days procurement and finance are told
//! and procurement gets one task, the notice repeating every N days.

mod support;

use renox::chrono::Days;
use renox::prelude::*;
use wl_inventory::WlInventory;
use wl_inventory::chase;
use wl_inventory::supplier_return::{NewReturnLine, ReturnStatus};
use wl_kernel::documents::today;
use wl_kernel::notify::NotifyRecipient;

use support::{Booted, boot, boot_with};

/// A dispatched return of 4 units, dispatched today.
async fn dispatched(t: &Booted, clerk: &User) -> wl_inventory::SupplierReturn {
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    t.stock_in(&item, &store, None, 10_000, 5_000).await;
    let ret = t
        .stock_return(
            &store,
            vec![NewReturnLine::of(item.id, 4_000, "damaged").credit()],
        )
        .await;
    t.dispatch_as(clerk, &ret, "courier", "C-1")
        .await
        .assert_ok();
    let ret = t.return_now(&ret).await;
    assert_eq!(ret.status, ReturnStatus::Dispatched);
    ret
}

/// Runs the scheduled chase `days` days after today.
async fn chase_on(t: &Booted, days: i64) {
    t.clock_at(days);
    t.app.run_scheduled(chase::TASK).await.unwrap();
}

/// Done when 1 and 2: a return dispatched on day 0 is left alone on day 30,
/// notified on day 31 with one task, not again on day 32, notified again on
/// day 61 with no new task; credited on day 70, it is not notified on day 91.
#[renox::test]
async fn a_return_not_credited_is_chased_after_n_days_and_every_n_days_after() {
    let t = boot().await;
    let procurement = t.procurement("Pia").await;
    let ret = dispatched(&t, &procurement).await;
    let finance = t.finance("Fay").await;

    chase_on(&t, 30).await;
    assert!(t.notices.all().is_empty());
    assert!(t.tickets.opened().is_empty());

    chase_on(&t, 31).await;
    let notices = t.notices.all();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].type_key, chase::NOTICE);
    assert_eq!(notices[0].subject, ret.subject());
    let mut to: Vec<i64> = notices[0]
        .recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect();
    to.sort_unstable();
    let mut expected = vec![procurement.id, finance.id];
    expected.sort_unstable();
    assert_eq!(to, expected);
    let tickets = t.tickets.opened();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].subject, ret.subject());
    assert_eq!(
        tickets[0].source_key.as_deref(),
        Some(chase::task_key(ret.id).as_str())
    );
    let now = t.return_now(&ret).await;
    assert_eq!(now.chase_count, 1);
    assert_eq!(
        now.last_chased_on,
        Some(today().checked_add_days(Days::new(31)).unwrap())
    );

    // Day 32: nothing more, no task per day.
    chase_on(&t, 32).await;
    assert_eq!(t.notices.all().len(), 1);
    assert_eq!(t.tickets.opened().len(), 1);

    // Day 61 = 31 + 30: notified again, still one task.
    chase_on(&t, 61).await;
    assert_eq!(t.notices.all().len(), 2);
    assert_eq!(t.tickets.opened().len(), 1);
    assert_eq!(t.return_now(&ret).await.chase_count, 2);

    // Credited on day 70: not notified on day 91, and the task is done.
    t.clock_at(70);
    t.app
        .at_travelled_time(async {
            t.credit_as(&procurement, &ret, "refund_received", "refund came in")
                .await
                .assert_ok();
        })
        .await;
    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Credited);
    assert_eq!(t.tickets.resolved().len(), 1);
    chase_on(&t, 91).await;
    assert_eq!(t.notices.all().len(), 2);
}

/// A return that is still a draft, or was cancelled, is not chased.
#[renox::test]
async fn only_dispatched_returns_are_chased() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    t.stock_in(&item, &store, None, 10_000, 5_000).await;
    let _draft = t
        .stock_return(&store, vec![NewReturnLine::of(item.id, 1_000, "damaged")])
        .await;
    t.procurement("Pia").await;

    chase_on(&t, 45).await;

    assert!(t.notices.all().is_empty());
    assert!(t.tickets.opened().is_empty());
}

/// N is a setting: with 10 days, day 10 is quiet and day 11 chases.
#[renox::test]
async fn the_number_of_days_is_a_setting() {
    let t = boot_with(WlInventory::new().return_chase_days(10)).await;
    let pia = t.procurement("Pia").await;
    dispatched(&t, &pia).await;

    chase_on(&t, 10).await;
    assert!(t.notices.all().is_empty());
    chase_on(&t, 11).await;
    assert_eq!(t.notices.all().len(), 1);
    // The next notice is 10 days later.
    chase_on(&t, 20).await;
    assert_eq!(t.notices.all().len(), 1);
    chase_on(&t, 21).await;
    assert_eq!(t.notices.all().len(), 2);
}

/// The task runs daily at 07:00 Asia/Dubai.
#[test]
fn the_rule_runs_daily_at_seven_in_dubai() {
    assert_eq!(chase::TASK_AT, "07:00");
    assert_eq!(chase::TASK_ZONE, "Asia/Dubai");
}
