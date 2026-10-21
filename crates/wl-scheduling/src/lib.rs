#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod config;
pub mod resource;
mod texts;

use renox::db::Migration;
use renox::prelude::*;
use wl_kernel::scheduling::Initiator;

pub use config::{SchedulingConfig, config_of};
pub use resource::{
    AvailabilityBlock, AvailabilityRule, Resource, ResourceType, ServiceResource, ServiceType,
};

/// The module's permissions (wl-scheduling §6).
pub mod permissions {
    /// See bookings in scope; other resources' calendars show busy or free
    /// only unless `bookings.view_details`.
    pub const VIEW: &str = "bookings.view";
    /// See the details of bookings on all resources in scope.
    pub const VIEW_DETAILS: &str = "bookings.view_details";
    /// Book, accept, decline, reschedule, cancel, mark no-show and manage the
    /// waiting list.
    pub const MANAGE: &str = "bookings.manage";
    /// Check parties in and start services.
    pub const CHECKIN: &str = "bookings.checkin";
    /// Waive a deposit forfeiture for a party-initiated late cancellation,
    /// walk-out or no-show.
    pub const WAIVE_DEPOSIT: &str = "bookings.waive_deposit";
    /// Manage resources, service types, availability rules and manual blocks.
    pub const RESOURCES_MANAGE: &str = "resources.manage";
}

/// The scheduling module. The host adds it with
/// `.module(wl_scheduling::WlScheduling::new())` when `client.toml` enables
/// `scheduling`.
#[derive(Default)]
#[non_exhaustive]
pub struct WlScheduling {
    config: SchedulingConfig,
}

impl WlScheduling {
    /// The module with the defaults of wl-scheduling §7.
    pub fn new() -> Self {
        Self::default()
    }

    /// Minutes a `requested` booking holds its slot.
    pub fn request_hold_minutes(mut self, minutes: i64) -> Self {
        assert!(minutes >= 0, "the request hold cannot be negative");
        self.config.request_hold_minutes = minutes;
        self
    }

    /// Minutes an unpaid self-booking holds its slot.
    pub fn prepayment_hold_minutes(mut self, minutes: i64) -> Self {
        assert!(minutes >= 0, "the prepayment hold cannot be negative");
        self.config.prepayment_hold_minutes = minutes;
        self
    }

    /// Minutes a waiting-list offer holds the slot.
    pub fn offer_hold_minutes(mut self, minutes: i64) -> Self {
        assert!(minutes >= 0, "the offer hold cannot be negative");
        self.config.offer_hold_minutes = minutes;
        self
    }

    /// Minutes after the start before a booking may be marked no-show.
    pub fn no_show_grace_minutes(mut self, minutes: i64) -> Self {
        assert!(minutes >= 0, "the grace cannot be negative");
        self.config.no_show_grace_minutes = minutes;
        self
    }

    /// How many days ahead a party can book.
    pub fn horizon_days(mut self, days: i64) -> Self {
        assert!(days >= 1, "the booking horizon is at least a day");
        self.config.horizon_days = days;
        self
    }

    /// Hours of notice a self-booking needs.
    pub fn min_notice_hours(mut self, hours: i64) -> Self {
        assert!(hours >= 0, "the notice cannot be negative");
        self.config.min_notice_hours = hours;
        self
    }

    /// The step of the slot grid, in minutes.
    pub fn slot_step_minutes(mut self, minutes: i64) -> Self {
        assert!(minutes >= 1, "the slot step is at least a minute");
        self.config.slot_step_minutes = minutes;
        self
    }

    /// Adds a cancellation reason code with its default *initiated by*. The
    /// codes of the module stay (wl-scheduling §4.1).
    pub fn reason_code(mut self, code: &str, initiated_by: Initiator) -> Self {
        self.config
            .extra_reason_codes
            .push((code.to_owned(), initiated_by));
        self
    }
}

impl Module for WlScheduling {
    fn name(&self) -> &'static str {
        "wl-scheduling"
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Resource>()
            .model::<ServiceType>()
            .model::<ServiceResource>()
            .model::<AvailabilityRule>()
            .model::<AvailabilityBlock>();
        app.provide(self.config.clone());
    }
}
