//! Helpers of the outage and daily closing tests (BL-FNB-14, BL-FNB-15): an app
//! whose Marina and Sharjah kitchens have device connections, their channels,
//! the managers who resume and approve, the clock moved to a wall-clock time of
//! Dubai, heartbeats and the scheduled check, and paper orders entered through
//! the real route.

#![allow(dead_code)]

use fnb::permissions as perm;
use fnb::{Fnb, Outlet};
use renox::chrono::TimeDelta;
use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::integrations::{self, ConnectionStatus, IntegrationConnection, devices};
use wl_sales::permissions as sperm;

use super::{World, boot_with, make_user, own_riders};

/// The people of the outlets.
pub struct Ops {
    /// The kitchen manager of Marina: resumes the channels, enters paper
    /// orders, submits the closing.
    pub kitchen: User,
    /// The area manager: resumes, approves late entries, approves and reopens
    /// a closing, changes the cut-off.
    pub area: User,
    /// The cashier: enters paper orders, opens and counts the till.
    pub till: User,
    /// The accountant: approves a closing and reviews the till sessions.
    pub accountant: User,
    /// Marina Kitchen: open 11:00 to 02:00.
    pub marina: Outlet,
    /// Sharjah Kitchen: open 10:00 to 23:00.
    pub sharjah: Outlet,
}

/// The device connection of Marina's kitchen.
pub const MARINA: &str = "fnb.devices.marina";

/// The device connection of Sharjah's kitchen.
pub const SHARJAH: &str = "fnb.devices.sharjah";

/// The app with the 06 delivery settings, the kitchen devices of the two
/// kitchens and their channels (Habibi Bowls is cooked by both).
pub async fn boot_ops() -> (World, Ops) {
    let fnb = Fnb::new()
        .kitchen_devices("marina", "11:00", "02:00", &["kds-1", "kds-2"])
        .kitchen_devices("sharjah", "10:00", "23:00", &["kds-s1"]);
    let w = boot_with(own_riders(), fnb).await;
    fnb::seed::demo_channels(w.db()).await.expect("the channels seed");
    wl_kernel::approvals::install(w.db()).await.expect("the approval policies");
    let kitchen = make_user(
        w.db(),
        "Mounir",
        &[
            perm::OUTLET_CHANNELS_VIEW,
            perm::OUTLET_CHANNELS_MANAGE,
            perm::PAPER_ORDERS_ENTER,
            perm::CLOSING_VIEW,
            perm::CLOSING_SUBMIT,
        ],
    )
    .await;
    let area = make_user(
        w.db(),
        "Huda",
        &[
            perm::OUTLET_CHANNELS_VIEW,
            perm::OUTLET_CHANNELS_MANAGE,
            perm::CLOSING_VIEW,
            perm::CLOSING_APPROVE,
            perm::CLOSING_REOPEN,
            perm::CLOSING_CUTOFF,
            wl_kernel::permissions::LATE_ENTRIES_APPROVE,
            wl_kernel::permissions::APPROVALS_VIEW,
            sperm::POS_REOPEN_TILL,
            sperm::POS_REVIEW,
        ],
    )
    .await;
    let till = make_user(
        w.db(),
        "Tala",
        &[
            perm::PAPER_ORDERS_ENTER,
            perm::OUTLET_CHANNELS_VIEW,
            perm::CLOSING_VIEW,
            sperm::POS_SELL,
            sperm::POS_OPEN_TILL,
            sperm::POS_CLOSE_TILL,
        ],
    )
    .await;
    let accountant = make_user(
        w.db(),
        "Nour",
        &[perm::CLOSING_VIEW, perm::CLOSING_APPROVE, sperm::POS_REVIEW],
    )
    .await;
    let marina = Outlet::by_code(w.db(), "marina").await.unwrap().expect("marina");
    let sharjah = Outlet::by_code(w.db(), "sharjah").await.unwrap().expect("sharjah");
    (
        w,
        Ops {
            kitchen,
            area,
            till,
            accountant,
            marina,
            sharjah,
        },
    )
}

/// The instant of the Dubai wall-clock `clock` (`HH:MM`) on the day `days`
/// after today (the real day: call it outside the travelled clock).
pub fn dubai_at(days: i64, clock: &str) -> DateTime {
    let day = wl_kernel::documents::today() + TimeDelta::days(days);
    fnb::business_day::at_local(day, clock).expect("a time of day")
}

/// The date `days` after today (Dubai).
pub fn day_after(days: i64) -> renox::chrono::NaiveDate {
    wl_kernel::documents::today() + TimeDelta::days(days)
}

/// The clock of the app now (travelled).
pub async fn now_of(w: &World) -> DateTime {
    w.app.at_travelled_time(async { renox::db::now() }).await
}

/// Moves the clock of the app to `target`.
pub async fn go_to(w: &World, target: DateTime) {
    let delta = target - now_of(w).await;
    assert!(delta >= TimeDelta::zero(), "the clock only moves forward");
    w.travel(delta);
}

/// A heartbeat of `device` of the connection `key`.
pub async fn beat(w: &World, key: &str, device: &str) {
    w.app
        .at_travelled_time(devices::heartbeat(w.state(), key, Some(device)))
        .await
        .expect("the heartbeat is recorded");
}

/// The scheduled check: missed heartbeats degrade a connection and a degraded
/// one goes down after N minutes (`integrations:run`).
pub async fn tick(w: &World) {
    let state = w.state().clone();
    w.app
        .at_travelled_time(async move {
            devices::check(&state).await.expect("the check runs");
            integrations::run_due(&state).await.expect("the run runs");
        })
        .await;
}

/// The status of the connection `key`.
pub async fn status_of(w: &World, key: &str) -> ConnectionStatus {
    IntegrationConnection::by_key(w.db(), key)
        .await
        .unwrap()
        .expect("the connection exists")
        .status
}
