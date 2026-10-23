//! The subscription contract (ARCHITECTURE §5, wl-invoicing §4 *Subscription*,
//! §6 `Subscriptions`): how a vertical starts, pauses, resumes and cancels the
//! subscription that bills its agreement, without depending on the
//! `wl-invoicing` crate.
//!
//! The vertical names its agreement (`accounting.engagement:7`,
//! `edtech.enrolment:12`) as the subscription's source and calls the
//! [`Subscriptions`] of [`subscriptions`] with it. Every call acts on the
//! subscriptions of that agreement that are not `cancelled`, and answers what
//! they look like afterwards. Without the `invoicing` module the calls do
//! nothing ([`NoSubscriptions`]):
//!
//! ```ignore
//! use wl_kernel::subscriptions::{self, PauseCause, PauseSubscription};
//!
//! // 04 engagement `suspend`:
//! subscriptions::subscriptions(&state)
//!     .pause(&state, &engagement.subject(), PauseSubscription::new(PauseCause::Agreement, today))
//!     .await?;
//! // 04 engagement `resume`:
//! subscriptions::subscriptions(&state)
//!     .resume(&state, &engagement.subject(), PauseCause::Agreement, today)
//!     .await?;
//! // 04 engagement `end`, the final settlement already made by the vertical:
//! subscriptions::subscriptions(&state)
//!     .cancel(&state, &engagement.subject(), CancelSubscription::new(today).settled())
//!     .await?;
//! ```
//!
//! `wl-invoicing` emits [`SubscriptionPastDue`], [`SubscriptionSuspended`],
//! [`SubscriptionSettled`] and [`SubscriptionCancelled`] after the commit of
//! the transition. A party block pauses the subscriptions itself (it listens
//! to `PartyBlocked` and `PartyUnblocked`): the vertical does not pass
//! [`PauseCause::PartyBlock`] for it.
//!
//! Money is `i64` in fils of the subscription's currency; quantities are in
//! thousandths.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::invoicing::{DEFAULT_CURRENCY, VatCategory};
use crate::parties::PartyId;
use crate::subject::Subject;

/// The subject type of a subscription, for [`Subject`] references.
pub const SUBSCRIPTION_SUBJECT_TYPE: &str = "invoicing.subscription";

/// The id of a subscription.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct SubscriptionId(pub i64);

impl SubscriptionId {
    /// The subscription as a [`Subject`]: `invoicing.subscription:42`.
    pub fn subject(self) -> Subject {
        Subject::new(SUBSCRIPTION_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for SubscriptionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for SubscriptionId {
    fn from(id: i64) -> Self {
        SubscriptionId(id)
    }
}

/// The lifecycle state of a subscription (wl-invoicing §4); the state of the
/// `wl-invoicing` machine.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SubscriptionStatus {
    /// Billed every cycle.
    #[default]
    Active,
    /// The latest cycle is unpaid: a cycle invoice is overdue, or a paid one
    /// was reopened by a reversed payment.
    PastDue,
    /// Billing stopped for a period; at least one pause cause is open.
    Paused,
    /// Unpaid after the last reminder or retry; the vertical blocks the
    /// service.
    Suspended,
    /// Ended (terminal).
    Cancelled,
}

/// How often a subscription is billed.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BillingCycle {
    /// Every 7 days from the start.
    Week,
    /// Every month, on the anchor day.
    #[default]
    Month,
    /// Every three months, on the anchor day.
    Quarter,
    /// Every year, on the anchor day.
    Year,
}

/// How a subscription's cycle invoices are paid.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PaymentMode {
    /// The party pays each cycle invoice (04 retainer).
    #[default]
    Manual,
    /// The party's saved card is charged (10 membership).
    Automatic,
}

/// Why a subscription is paused. Each open cause is stored; the
/// subscription resumes only when none is left (wl-invoicing §4 `resume`).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PauseCause {
    /// A pause asked by a user (staff, or the party through the vertical),
    /// with its end date; it counts against the pause allowance.
    #[default]
    User,
    /// The vertical's agreement is paused (10 enrolment `freeze`, 04
    /// engagement `suspend`).
    Agreement,
    /// The party is blocked and the block effect for subscriptions is
    /// *pause* (wl-parties §5.6). Recorded with the neutral reason `on_hold`.
    PartyBlock,
}

/// A subscription to start ([`Subscriptions::start`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct StartSubscription {
    /// Who is billed.
    pub party: PartyId,
    /// The vertical's agreement it bills (the source).
    pub source: Subject,
    /// The plan, as the cycle invoice's line names it ("Bookkeeping retainer").
    pub plan: String,
    /// The price of one unit for one cycle, without VAT, in fils.
    pub unit_price: i64,
    /// The VAT treatment of the cycle line.
    pub vat_category: VatCategory,
    /// The quantity in thousandths (1000 = 1).
    pub quantity_milli: i64,
    /// The ISO currency code.
    pub currency: String,
    /// How often it is billed.
    pub cycle: BillingCycle,
    /// The day of the month cycles start on (1 to 31, clamped to the month's
    /// last day); `None` is the start date's day. Not used for weekly cycles.
    pub anchor_day: Option<u32>,
    /// The first day of the first cycle (Asia/Dubai).
    pub start: NaiveDate,
    /// The last day billed, if the agreement has an end; no cycle starting
    /// after it is billed.
    pub end: Option<NaiveDate>,
    /// Days of notice a user or party cancel needs (not a system cancel).
    pub notice_days: u32,
    /// How cycle invoices are paid.
    pub payment_mode: PaymentMode,
    /// The issuing legal entity; `None` is the default one.
    pub legal_entity_id: Option<i64>,
}

impl StartSubscription {
    /// A monthly subscription of `party` for `source`, billing `plan` at
    /// `unit_price` fils (standard VAT, quantity 1, AED, manual payment, no
    /// notice) from `start`.
    pub fn new(
        party: PartyId,
        source: Subject,
        plan: impl Into<String>,
        unit_price: i64,
        start: NaiveDate,
    ) -> Self {
        Self {
            party,
            source,
            plan: plan.into(),
            unit_price,
            vat_category: VatCategory::Standard,
            quantity_milli: 1000,
            currency: DEFAULT_CURRENCY.to_owned(),
            cycle: BillingCycle::Month,
            anchor_day: None,
            start,
            end: None,
            notice_days: 0,
            payment_mode: PaymentMode::Manual,
            legal_entity_id: None,
        }
    }

    /// Sets the VAT category.
    pub fn vat_category(mut self, category: VatCategory) -> Self {
        self.vat_category = category;
        self
    }

    /// Sets the quantity, in thousandths.
    pub fn quantity_milli(mut self, quantity_milli: i64) -> Self {
        self.quantity_milli = quantity_milli;
        self
    }

    /// Sets the currency.
    pub fn currency(mut self, currency: &str) -> Self {
        self.currency = currency.to_owned();
        self
    }

    /// Sets the billing cycle.
    pub fn cycle(mut self, cycle: BillingCycle) -> Self {
        self.cycle = cycle;
        self
    }

    /// Sets the anchor day (1 to 31).
    pub fn anchor_day(mut self, day: u32) -> Self {
        self.anchor_day = Some(day);
        self
    }

    /// Sets the last day billed.
    pub fn end(mut self, end: NaiveDate) -> Self {
        self.end = Some(end);
        self
    }

    /// Sets the notice period of a user or party cancel, in days.
    pub fn notice_days(mut self, days: u32) -> Self {
        self.notice_days = days;
        self
    }

    /// Sets the payment mode.
    pub fn payment_mode(mut self, mode: PaymentMode) -> Self {
        self.payment_mode = mode;
        self
    }

    /// Sets the issuing legal entity.
    pub fn legal_entity(mut self, id: i64) -> Self {
        self.legal_entity_id = Some(id);
        self
    }
}

/// A pause to run ([`Subscriptions::pause`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PauseSubscription {
    /// Why it is paused.
    pub cause: PauseCause,
    /// The pause's first day (the first day not used), Asia/Dubai.
    pub on: NaiveDate,
    /// The reason recorded; `None` gives the cause's default text. A
    /// [`PauseCause::PartyBlock`] always records `on_hold`.
    pub reason: Option<String>,
    /// The last day of a [`PauseCause::User`] pause: it ends (and the
    /// subscription resumes, when no other cause is left) the day after.
    /// Needed for a user pause, ignored otherwise.
    pub until: Option<NaiveDate>,
}

impl PauseSubscription {
    /// A pause for `cause` from `on`.
    pub fn new(cause: PauseCause, on: NaiveDate) -> Self {
        Self {
            cause,
            on,
            reason: None,
            until: None,
        }
    }

    /// Sets the reason recorded.
    pub fn reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Sets the end of a user pause: the pause covers the days up to the
    /// day before `until`, and ends on `until`.
    pub fn until(mut self, until: NaiveDate) -> Self {
        self.until = Some(until);
        self
    }
}

/// A system cancel to run ([`Subscriptions::cancel`]): the vertical's
/// agreement ended or was withdrawn (10 enrolment `withdraw`, 04 engagement
/// `end`). No notice period applies; the subscription ends at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CancelSubscription {
    /// The cancel day (counted as used), Asia/Dubai.
    pub on: NaiveDate,
    /// Whether the vertical's own settlement already credited or billed the
    /// unused part: then no credit note or final invoice is made.
    pub settled: bool,
    /// The reason recorded; `None` gives "agreement ended".
    pub reason: Option<String>,
}

impl CancelSubscription {
    /// A system cancel on `on`, prorated by the invoicing module.
    pub fn new(on: NaiveDate) -> Self {
        Self {
            on,
            settled: false,
            reason: None,
        }
    }

    /// The vertical's settlement already made the credit or final invoice.
    pub fn settled(mut self) -> Self {
        self.settled = true;
        self
    }

    /// Sets the reason recorded.
    pub fn reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

/// What other modules see of a subscription.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SubscriptionSummary {
    /// The id.
    pub id: SubscriptionId,
    /// Who is billed.
    pub party: PartyId,
    /// The agreement it bills.
    pub source: Subject,
    /// The lifecycle state.
    pub status: SubscriptionStatus,
    /// How often it is billed.
    pub cycle: BillingCycle,
    /// The first day of the next cycle to bill.
    pub next_cycle_date: NaiveDate,
    /// The open pause causes (empty unless `paused`).
    pub pause_causes: Vec<PauseCause>,
    /// The cancel day, once `cancelled`.
    pub cancelled_on: Option<NaiveDate>,
    /// The credit note made for the unused part on cancel, if any.
    pub final_credit_note: Option<i64>,
    /// The final pro rata invoice made on cancel, if any.
    pub final_invoice: Option<i64>,
}

/// The future a [`Subscriptions`] method returns.
pub type SubscriptionFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Starts, pauses, resumes and cancels subscriptions for the verticals.
/// `wl-invoicing` provides it; without that module, [`subscriptions`] gives
/// [`NoSubscriptions`], whose every method does nothing.
pub trait Subscriptions: Send + Sync + 'static {
    /// Starts an `active` subscription and returns its id; `None` when
    /// invoicing is off. Refused (422) for a blank plan, a price below 0, a
    /// quantity of 0 or less, an anchor day outside 1 to 31 or an end before
    /// the start.
    fn start<'a>(
        &'a self,
        _state: &'a AppState,
        _start: StartSubscription,
    ) -> SubscriptionFuture<'a, Option<SubscriptionId>> {
        Box::pin(async { Ok(None) })
    }

    /// The subscription with this id; the default knows none.
    fn find<'a>(
        &'a self,
        _db: &'a Db,
        _id: SubscriptionId,
    ) -> SubscriptionFuture<'a, Option<SubscriptionSummary>> {
        Box::pin(async { Ok(None) })
    }

    /// The subscriptions of `agreement`, oldest first, cancelled ones
    /// included; the default has none.
    fn of_agreement<'a>(
        &'a self,
        _db: &'a Db,
        _agreement: &'a Subject,
    ) -> SubscriptionFuture<'a, Vec<SubscriptionSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Pauses, as the system, every `active`, `past_due` or `paused`
    /// subscription of `agreement` for `pause.cause`. On one already
    /// `paused` only the cause is added; a cause already open is not added
    /// twice. A system pause never counts against the pause allowance.
    /// Answers the subscriptions afterwards; the default has none.
    fn pause<'a>(
        &'a self,
        _state: &'a AppState,
        _agreement: &'a Subject,
        _pause: PauseSubscription,
    ) -> SubscriptionFuture<'a, Vec<SubscriptionSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Removes, as the system, the open `cause` of every subscription of
    /// `agreement`, and resumes those with no cause left on `on`: to
    /// `past_due` when a cycle invoice from before the pause is still
    /// overdue, else `active`. The next cycle date is recalculated from `on`
    /// and the anchor day. Answers the subscriptions afterwards; the default
    /// has none.
    fn resume<'a>(
        &'a self,
        _state: &'a AppState,
        _agreement: &'a Subject,
        _cause: PauseCause,
        _on: NaiveDate,
    ) -> SubscriptionFuture<'a, Vec<SubscriptionSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Cancels, as the system and at once, every subscription of `agreement`
    /// that is not `cancelled`: no notice period, a pro rata credit note for
    /// the unused part of a paid (or still unpaid) cycle, or a final pro rata
    /// invoice for an unbilled one, unless `cancel.settled`. No refund is
    /// paid here; refunds to a blocked party wait (`PTY.ACL.03`). Answers the
    /// subscriptions afterwards; the default has none.
    fn cancel<'a>(
        &'a self,
        _state: &'a AppState,
        _agreement: &'a Subject,
        _cancel: CancelSubscription,
    ) -> SubscriptionFuture<'a, Vec<SubscriptionSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The subscriptions when the `invoicing` module is off: nothing is started
/// or found, and pause, resume and cancel do nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSubscriptions;

impl Subscriptions for NoSubscriptions {}

/// Makes `subscriptions` the app's [`Subscriptions`]; `wl-invoicing` calls it
/// in `register`.
pub fn provide_subscriptions(app: &mut Registry, subscriptions: impl Subscriptions) {
    let subscriptions: Arc<dyn Subscriptions> = Arc::new(subscriptions);
    app.provide(subscriptions);
}

/// The app's [`Subscriptions`], or [`NoSubscriptions`] when none is provided.
pub fn subscriptions(state: &AppState) -> Arc<dyn Subscriptions> {
    match state.provided::<Arc<dyn Subscriptions>>() {
        Some(subscriptions) => Arc::clone(&subscriptions),
        None => Arc::new(NoSubscriptions),
    }
}

/// A subscription went past due (`mark_past_due`): a cycle invoice is overdue
/// and not disputed, or a paid one was reopened by a reversed payment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionPastDue {
    /// The subscription.
    pub subscription: SubscriptionId,
    /// Who is billed.
    pub party: PartyId,
    /// The agreement it bills.
    pub source: Subject,
}

impl Event for SubscriptionPastDue {}

/// A pause cause was added to a subscription (`pause`): a user's pause (staff,
/// or the party through the vertical), the vertical's own agreement pause or a
/// party block. Emitted once per cause added, whether the subscription was
/// `active`, `past_due` or already `paused`. 10 `EDU.RUL.14` freezes the
/// enrolment of a membership a user paused; the vertical ignores the cause it
/// asked for itself ([`PauseCause::Agreement`]) so that the two sides do not
/// loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionPaused {
    /// The subscription.
    pub subscription: SubscriptionId,
    /// Who is billed.
    pub party: PartyId,
    /// The agreement it bills.
    pub source: Subject,
    /// The cause that was added.
    pub cause: PauseCause,
    /// The last day of a [`PauseCause::User`] pause.
    pub until: Option<NaiveDate>,
    /// Whether this cause paused the subscription (it was not `paused` before).
    pub first: bool,
}

impl Event for SubscriptionPaused {}

/// A pause cause was removed from a subscription (`resume`): a user's pause
/// ended or was lifted, the vertical's agreement resumed, a party was
/// unblocked. Emitted once per cause removed, also when other causes keep the
/// subscription `paused`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionResumed {
    /// The subscription.
    pub subscription: SubscriptionId,
    /// Who is billed.
    pub party: PartyId,
    /// The agreement it bills.
    pub source: Subject,
    /// The cause that was removed.
    pub cause: PauseCause,
    /// Whether no cause is left and the subscription is no longer `paused`.
    pub resumed: bool,
    /// Whether it went back to `past_due` (a cycle invoice from before the
    /// pause is still overdue) rather than `active`.
    pub to_past_due: bool,
}

impl Event for SubscriptionResumed {}

/// A subscription was suspended (`suspend`, `FIN.RUL.06`): the vertical
/// blocks the service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionSuspended {
    /// The subscription.
    pub subscription: SubscriptionId,
    /// Who is billed.
    pub party: PartyId,
    /// The agreement it bills.
    pub source: Subject,
}

impl Event for SubscriptionSuspended {}

/// A `past_due` or `suspended` subscription is paid up again (`settle`): the
/// vertical restores the service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionSettled {
    /// The subscription.
    pub subscription: SubscriptionId,
    /// Who is billed.
    pub party: PartyId,
    /// The agreement it bills.
    pub source: Subject,
}

impl Event for SubscriptionSettled {}

/// A subscription was cancelled (`cancel`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionCancelled {
    /// The subscription.
    pub subscription: SubscriptionId,
    /// Who is billed.
    pub party: PartyId,
    /// The agreement it bills.
    pub source: Subject,
}

impl Event for SubscriptionCancelled {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_and_causes_are_stored_in_snake_case() {
        let states: Vec<_> = SubscriptionStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            states,
            ["active", "past_due", "paused", "suspended", "cancelled"]
        );
        let causes: Vec<_> = PauseCause::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(causes, ["user", "agreement", "party_block"]);
    }

    #[test]
    fn the_builders_keep_their_defaults() {
        let day = NaiveDate::from_ymd_opt(2027, 3, 1).unwrap();
        let agreement = Subject::new("accounting.engagement", 7);
        let start = StartSubscription::new(PartyId(1), agreement.clone(), "Retainer", 100, day);
        assert_eq!(start.quantity_milli, 1000);
        assert_eq!(start.currency, "AED");
        let pause = PauseSubscription::new(PauseCause::Agreement, day).reason("frozen");
        assert_eq!(pause.reason.as_deref(), Some("frozen"));
        assert!(CancelSubscription::new(day).settled().settled);
        assert_eq!(
            SubscriptionId(4).subject().to_string(),
            "invoicing.subscription:4"
        );
    }
}
