//! The module's settings (wl-scheduling §7). They are client defaults kept in
//! code, set on [`WlScheduling`](crate::WlScheduling) with builder calls; a
//! settings page comes later.

use wl_kernel::scheduling::Initiator;

/// How long a `requested` booking holds its slot while staff decide (§7).
pub const DEFAULT_REQUEST_HOLD_MINUTES: i64 = 15;
/// How long an unpaid self-booking holds its slot: the expiry of its payment
/// link (§7).
pub const DEFAULT_PREPAYMENT_HOLD_MINUTES: i64 = 15;
/// How long a waiting-list offer holds the slot (§7).
pub const DEFAULT_OFFER_HOLD_MINUTES: i64 = 30;
/// Minutes after the start before a booking may be marked no-show (§7).
pub const DEFAULT_NO_SHOW_GRACE_MINUTES: i64 = 15;
/// How far ahead a party can book (§7).
pub const DEFAULT_HORIZON_DAYS: i64 = 60;
/// Minimum notice for a self-booking (§7).
pub const DEFAULT_MIN_NOTICE_HOURS: i64 = 2;
/// The step of the slot grid (§7).
pub const DEFAULT_SLOT_STEP_MINUTES: i64 = 15;
/// Hours before the start at which an open conflict is escalated (`SCH.RUL.06`).
pub const DEFAULT_ESCALATION_HOURS: i64 = 2;
/// Hours before a booking at which a reminder goes out (`SCH.RUL.01`; 05, 10).
pub const DEFAULT_REMINDER_HOURS: [i64; 2] = [24, 2];
/// How many nearest free slots a refused booker is shown (§3).
pub const NEAREST_SLOTS: usize = 3;

/// The settings of the scheduling module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulingConfig {
    /// Hold of a `requested` booking, in minutes.
    pub request_hold_minutes: i64,
    /// Hold of an unpaid self-booking, in minutes.
    pub prepayment_hold_minutes: i64,
    /// Hold of a waiting-list offer, in minutes.
    pub offer_hold_minutes: i64,
    /// Grace before a no-show, in minutes.
    pub no_show_grace_minutes: i64,
    /// Booking horizon, in days.
    pub horizon_days: i64,
    /// Minimum notice for a self-booking, in hours.
    pub min_notice_hours: i64,
    /// Slot step, in minutes.
    pub slot_step_minutes: i64,
    /// Hours before the start at which an open conflict is escalated.
    pub escalation_hours: i64,
    /// Hours before a booking at which a reminder goes out.
    pub reminder_hours: Vec<i64>,
    /// The vertical's manager role, whose holders in the booking's location
    /// are told of an open conflict (`SCH.RUL.06`); the users with
    /// `bookings.manage` there when nobody holds it or none is set.
    pub manager_role: Option<String>,
    /// Cancellation reason codes a vertical added, with their default
    /// *initiated by*.
    pub extra_reason_codes: Vec<(String, Initiator)>,
}

impl Default for SchedulingConfig {
    fn default() -> Self {
        Self {
            request_hold_minutes: DEFAULT_REQUEST_HOLD_MINUTES,
            prepayment_hold_minutes: DEFAULT_PREPAYMENT_HOLD_MINUTES,
            offer_hold_minutes: DEFAULT_OFFER_HOLD_MINUTES,
            no_show_grace_minutes: DEFAULT_NO_SHOW_GRACE_MINUTES,
            horizon_days: DEFAULT_HORIZON_DAYS,
            min_notice_hours: DEFAULT_MIN_NOTICE_HOURS,
            slot_step_minutes: DEFAULT_SLOT_STEP_MINUTES,
            escalation_hours: DEFAULT_ESCALATION_HOURS,
            reminder_hours: DEFAULT_REMINDER_HOURS.to_vec(),
            manager_role: None,
            extra_reason_codes: Vec::new(),
        }
    }
}

/// The settings the module provided, else the defaults.
pub fn config_of(state: &renox::prelude::AppState) -> SchedulingConfig {
    state
        .provided::<SchedulingConfig>()
        .map(|config| (*config).clone())
        .unwrap_or_default()
}
