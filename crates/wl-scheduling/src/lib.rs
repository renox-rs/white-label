#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod booking;
pub mod booking_machine;
pub mod config;
pub mod conflict;
pub mod conflict_pages;
pub mod deposit;
pub mod eligibility;
pub mod events;
pub mod links;
pub mod notices;
pub mod pages;
pub mod prepaid;
pub mod prepayment;
pub mod refusal;
pub mod resource;
pub mod rules;
pub mod service;
pub mod slots;
mod texts;
pub mod waiting;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;
use wl_kernel::scheduling::{Initiator, SlotReleased};
use wl_kernel::workforce::{LeaveApproved, LeaveCancelled};

pub use booking::{Booking, BookingResource, BookingStatus};
pub use booking_machine::{ActorKind, BookingInput};
pub use config::{SchedulingConfig, config_of};
pub use conflict::{BulkAction, BulkResult, Conflict};
pub use refusal::Refusal;
pub use resource::{
    AvailabilityBlock, AvailabilityRule, Resource, ResourceType, ServiceResource, ServiceType,
};
pub use rules::Reminder;
pub use service::{Actor, CancelRequest, NewBooking};
pub use waiting::{WaitingEntry, WaitingStatus};

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
const VIEWS: &[(&str, &str)] = &[
    (
        "wl-scheduling/bookings.html",
        include_str!("../views/wl-scheduling/bookings.html"),
    ),
    (
        "wl-scheduling/booking.html",
        include_str!("../views/wl-scheduling/booking.html"),
    ),
    (
        "wl-scheduling/conflicts.html",
        include_str!("../views/wl-scheduling/conflicts.html"),
    ),
    (
        "wl-scheduling/link.html",
        include_str!("../views/wl-scheduling/link.html"),
    ),
    (
        "wl-scheduling/link_slots.html",
        include_str!("../views/wl-scheduling/link_slots.html"),
    ),
];

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

    /// Hours before the start at which an open conflict is escalated to the
    /// branch manager (`SCH.RUL.06`).
    pub fn escalation_hours(mut self, hours: i64) -> Self {
        assert!(hours >= 0, "the escalation cannot be negative");
        self.config.escalation_hours = hours;
        self
    }

    /// Days ahead `SCH.RUL.05` checks bookings against the resource's
    /// qualifications and documents at their start (30 by default).
    pub fn look_ahead_days(mut self, days: i64) -> Self {
        assert!(days >= 1, "the look-ahead is at least a day");
        self.config.look_ahead_days = days;
        self
    }

    /// Hours before a booking at which a reminder goes out (`SCH.RUL.01`).
    pub fn reminder_hours(mut self, hours: &[i64]) -> Self {
        assert!(
            hours.iter().all(|h| *h >= 1),
            "a reminder is at least an hour before"
        );
        self.config.reminder_hours = hours.to_vec();
        self
    }

    /// The vertical's manager role: its holders in the booking's location are
    /// told of a conflict that is still open (`SCH.RUL.06`).
    pub fn manager_role(mut self, role: &str) -> Self {
        self.config.manager_role = Some(role.to_owned());
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

    /// The module's settings, as the builder calls left them.
    pub fn config(&self) -> &SchedulingConfig {
        &self.config
    }
}

impl Module for WlScheduling {
    fn name(&self) -> &'static str {
        "wl-scheduling"
    }

    fn routes(&self) -> Routes {
        pages::routes()
            .merge(conflict_pages::routes())
            .merge(links::routes())
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Resource>()
            .model::<ServiceType>()
            .model::<ServiceResource>()
            .model::<AvailabilityRule>()
            .model::<AvailabilityBlock>()
            .model::<Booking>()
            .model::<BookingResource>()
            .model::<WaitingEntry>()
            .model::<Conflict>()
            .model::<Reminder>();
        renox::state_machine::register::<Booking>();
        renox::state_machine::register::<WaitingEntry>();
        app.view_data::<pages::IndexPage>()
            .view_data::<pages::ShowPage>()
            .view_data::<conflict_pages::ConflictsPage>()
            .view_data::<links::LinkPage>()
            .view_data::<links::SlotsPage>();
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        notices::register_types();
        rules::schedule(app);
        // `SCH.RUL.05`: lapsing licences and documents flag the bookings made
        // before they lapsed; the providers wl-documents asks for the work.
        eligibility::register(app);
        // `SCH.RUL.04`: the prepayment hold of a self-booking, the link's
        // expiry and the payments received for it.
        prepayment::register(app);
        // `SCH.RUL.03`: a released slot is offered to the waiting list.
        app.listen(|event: SlotReleased, state: AppState| async move {
            waiting::offer_released(&state, &event, renox::db::now())
                .await
                .map(|_| ())
        });
        // Sick leave and other leave block the employee's resources and flag
        // the bookings they overlap; a cancelled leave takes its blocks away.
        app.listen(|event: LeaveApproved, state: AppState| async move {
            conflict::leave_approved(&state, &event, renox::db::now())
                .await
                .map(|_| ())
        });
        app.listen(|event: LeaveCancelled, state: AppState| async move {
            conflict::leave_cancelled(&state, &event, renox::db::now()).await
        });
        app.provide(self.config.clone());
        // A booking lies in the scope of its location (its branch), so that a
        // vertical can tell which branch a booking named by an event is at.
        wl_kernel::scopes::register(wl_kernel::scheduling::BOOKING_SUBJECT_TYPE, |db, id| {
            Box::pin(async move {
                use renox::auth::permissions::Scope;
                let Some(booking) = Booking::find(db, id).await? else {
                    return Ok(Scope::global());
                };
                // The branch's own scope; a branch type with no resolver
                // is a scope of its own type and id.
                Ok(match rules::location_of(db, &booking).await? {
                    Some(location) => {
                        let registered = wl_kernel::scopes::scope_of(db, &location).await?;
                        if registered.is_global() {
                            Scope::new(&location.subject_type, location.subject_id)
                        } else {
                            registered
                        }
                    }
                    None => Scope::global(),
                })
            })
        });
        // wl-payments asks whether a booking still waits for its money.
        wl_kernel::payments::provide_prepaid_subjects(app, prepaid::PrepaidBookings);
        // In the app group of the staff sidebar; the label is the app's
        // `nav.bookings` text.
        app.nav(
            "app",
            NavLink::route("wl_scheduling.bookings", "nav.bookings")
                .icon("calendar-check")
                .group("nav.app")
                .order(62)
                .permission(permissions::VIEW),
        );
    }
}
