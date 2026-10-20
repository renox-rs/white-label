//! The Subscription (wl-invoicing §3, §4 *Subscription*): a recurring plan
//! billed every cycle, with its pause causes ([`SubscriptionPause`]) and its
//! cycle invoices ([`SubscriptionCycle`]). Its status moves only through the
//! machine in [`crate::subscription_machine`]; the services are in
//! [`crate::subscriptions`].
//!
//! The cycle dates and the pro rata arithmetic are plain functions here
//! ([`advance`], [`cycle_end`], [`unused_days`], [`pro_rata`]), all in
//! Asia/Dubai calendar days.

use renox::chrono::{Datelike, Days, NaiveDate};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::invoicing::VatCategory;
use wl_kernel::subscriptions::{
    BillingCycle, PauseCause, PaymentMode, SUBSCRIPTION_SUBJECT_TYPE, SubscriptionId,
    SubscriptionStatus, SubscriptionSummary,
};
use wl_kernel::{PartyId, Subject};

/// The subject type of a cycle, the origin of its invoice line.
pub const CYCLE_SUBJECT_TYPE: &str = "invoicing.subscription_cycle";

/// A recurring plan billed every cycle. Money is `i64` fils of
/// [`Subscription::currency`].
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_subscriptions",
    state_machine,
    index(status),
    index(party_id),
    index(source_type, source_id)
)]
pub struct Subscription {
    /// The primary key.
    pub id: i64,
    /// Who is billed.
    pub party_id: i64,
    /// The issuing legal entity; `None` is the default one.
    pub legal_entity_id: Option<i64>,
    /// The type of the agreement it bills, e.g. `accounting.engagement`.
    pub source_type: String,
    /// The id of the agreement it bills.
    pub source_id: i64,
    /// The plan, as the cycle line names it.
    pub plan: String,
    /// The price of one unit for one cycle, without VAT.
    pub unit_price: i64,
    /// The VAT treatment of the cycle line.
    pub vat_category: VatCategory,
    /// The quantity in thousandths.
    pub quantity_milli: i64,
    /// The ISO currency code.
    pub currency: String,
    /// How often it is billed.
    pub cycle: BillingCycle,
    /// The day of the month cycles start on (1 to 31).
    pub anchor_day: i64,
    /// The first day of the first cycle.
    pub start_date: NaiveDate,
    /// The last day billed, if any.
    pub end_date: Option<NaiveDate>,
    /// Days of notice of a user or party cancel.
    pub notice_days: i64,
    /// How cycle invoices are paid.
    pub payment_mode: PaymentMode,
    /// The lifecycle state; changed only by a transition.
    pub status: SubscriptionStatus,
    /// The first day of the next cycle to bill.
    pub next_cycle_date: NaiveDate,
    /// The day it went `past_due`; `FIN.RUL.06` counts its retries from it.
    pub past_due_on: Option<NaiveDate>,
    /// The `FIN.RUL.06` retries or reminders done since `past_due_on`.
    pub retries_done: i64,
    /// The first day not used of the current pause.
    pub paused_from: Option<NaiveDate>,
    /// The reason shown for the current pause (`on_hold` for a block).
    pub pause_reason: Option<String>,
    /// Days of user pause taken in `pause_year`.
    pub pause_days_used: i64,
    /// The calendar year `pause_days_used` counts.
    pub pause_year: Option<i64>,
    /// The day a user or party cancel takes effect after its notice.
    pub cancel_on: Option<NaiveDate>,
    /// The cancel day, once `cancelled`.
    pub cancelled_on: Option<NaiveDate>,
    /// Why it was cancelled.
    pub cancel_reason: Option<String>,
    /// The credit note of the unused part made on cancel.
    pub final_credit_note_id: Option<i64>,
    /// The final pro rata invoice made on cancel.
    pub final_invoice_id: Option<i64>,
    /// The pro rata settlement of the cancel is still to be made (set in the
    /// cancel's transaction, cleared once the credit note or final invoice
    /// exists), so a failed one is made again on the next try.
    pub settlement_due: bool,
    /// The first day of the pause the subscription was in when cancelled:
    /// the unused part of its cycle runs from it.
    pub settlement_paused_from: Option<NaiveDate>,
    /// The user who cancelled, for the settlement; `None` for the system.
    pub settlement_by: Option<i64>,
    /// The user who started it; `None` for the system.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Subscription {
    /// The subscription's id in the kernel contract.
    pub fn subscription_id(&self) -> SubscriptionId {
        SubscriptionId(self.id)
    }

    /// The subscription as a [`Subject`]: `invoicing.subscription:42`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBSCRIPTION_SUBJECT_TYPE, self.id)
    }

    /// Who is billed.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// The agreement it bills.
    pub fn source(&self) -> Subject {
        Subject {
            subject_type: self.source_type.clone(),
            subject_id: self.source_id,
        }
    }

    /// The anchor day as a day of the month.
    pub fn anchor(&self) -> u32 {
        u32::try_from(self.anchor_day.clamp(1, 31)).unwrap_or(1)
    }

    /// The first day of the cycle after the one starting on `start`.
    pub fn after(&self, start: NaiveDate) -> NaiveDate {
        advance(self.cycle, self.anchor(), start)
    }

    /// The last day of the cycle starting on `start`.
    pub fn end_of(&self, start: NaiveDate) -> NaiveDate {
        cycle_end(self.cycle, self.anchor(), start)
    }

    /// The first cycle date on or after `on`, stepping the schedule from the
    /// next cycle date (the anchor day keeps the dates on their day).
    pub fn next_on_or_after(&self, on: NaiveDate) -> NaiveDate {
        let mut date = self.next_cycle_date;
        while date < on {
            date = self.after(date);
        }
        date
    }

    /// What other modules see of it, with its open pause causes.
    pub fn summary(&self, causes: Vec<PauseCause>) -> SubscriptionSummary {
        SubscriptionSummary {
            id: self.subscription_id(),
            party: self.party(),
            source: self.source(),
            status: self.status,
            cycle: self.cycle,
            next_cycle_date: self.next_cycle_date,
            pause_causes: causes,
            cancelled_on: self.cancelled_on,
            final_credit_note: self.final_credit_note_id,
            final_invoice: self.final_invoice_id,
        }
    }

    /// The subscriptions of `agreement`, oldest first.
    pub async fn of_source(db: &Db, agreement: &Subject) -> Result<Vec<Subscription>> {
        Subscription::query()
            .where_(Subscription::SOURCE_TYPE.eq(agreement.subject_type.as_str()))
            .where_(Subscription::SOURCE_ID.eq(agreement.subject_id))
            .order_by(Subscription::ID)
            .get(db)
            .await
    }
}

/// One cause of a pause. A cause is open while `ended_on` is empty; at most
/// one cause of each kind is open per subscription.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_invoicing_subscription_pauses", index(subscription_id))]
pub struct SubscriptionPause {
    /// The primary key.
    pub id: i64,
    /// The subscription.
    pub subscription_id: i64,
    /// Why it is paused.
    pub cause: PauseCause,
    /// The reason recorded (`on_hold` for a party block).
    pub reason: String,
    /// The first day of this cause.
    pub started_on: NaiveDate,
    /// The day a user pause ends (it resumes on that day).
    pub until: Option<NaiveDate>,
    /// The day the cause was removed; `None` while open.
    pub ended_on: Option<NaiveDate>,
    /// The user who paused; `None` for the system.
    pub by_user_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl SubscriptionPause {
    /// The open causes of subscription `id`, oldest first.
    pub async fn open_of<'c>(
        db: impl renox::db::Executor<'c>,
        id: i64,
    ) -> Result<Vec<SubscriptionPause>> {
        SubscriptionPause::query()
            .where_(SubscriptionPause::SUBSCRIPTION_ID.eq(id))
            .where_(SubscriptionPause::ENDED_ON.is_null())
            .order_by(SubscriptionPause::ID)
            .get(db)
            .await
    }

    /// Every cause of subscription `id`, open or ended, oldest first.
    pub async fn all_of(db: &Db, id: i64) -> Result<Vec<SubscriptionPause>> {
        SubscriptionPause::query()
            .where_(SubscriptionPause::SUBSCRIPTION_ID.eq(id))
            .order_by(SubscriptionPause::ID)
            .get(db)
            .await
    }
}

/// One billed cycle: its dates and its cycle invoice. One per cycle start.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_subscription_cycles",
    unique(subscription_id, period_start),
    index(invoice_id)
)]
pub struct SubscriptionCycle {
    /// The primary key.
    pub id: i64,
    /// The subscription.
    pub subscription_id: i64,
    /// The cycle's first day.
    pub period_start: NaiveDate,
    /// The cycle's last day.
    pub period_end: NaiveDate,
    /// The cycle invoice; `None` only while it is being drafted.
    pub invoice_id: Option<i64>,
    /// Whether a final pro rata invoice of a cancel bills it.
    pub is_final: bool,
    /// Whether the invoice was reopened by a reversed payment
    /// (`InvoiceReopened`): it is outstanding until paid again, due or not.
    pub reopened: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl SubscriptionCycle {
    /// The cycle as a [`Subject`]: `invoicing.subscription_cycle:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(CYCLE_SUBJECT_TYPE, self.id)
    }

    /// The cycles of subscription `id`, oldest first.
    pub async fn of(db: &Db, id: i64) -> Result<Vec<SubscriptionCycle>> {
        SubscriptionCycle::query()
            .where_(SubscriptionCycle::SUBSCRIPTION_ID.eq(id))
            .order_by(SubscriptionCycle::PERIOD_START)
            .get(db)
            .await
    }

    /// The cycle of subscription `id` whose days include `day`.
    pub async fn containing(db: &Db, id: i64, day: NaiveDate) -> Result<Option<SubscriptionCycle>> {
        SubscriptionCycle::query()
            .where_(SubscriptionCycle::SUBSCRIPTION_ID.eq(id))
            .where_(SubscriptionCycle::PERIOD_START.lte(day))
            .where_(SubscriptionCycle::PERIOD_END.gte(day))
            .order_by(SubscriptionCycle::PERIOD_START)
            .first(db)
            .await
    }
}

/// The last day of `month` in `year`.
fn last_day(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .and_then(|d| d.pred_opt())
        .map_or(28, |d| d.day())
}

/// `date` moved by `months` months, on the anchor day clamped to the
/// month's last day.
fn add_months(date: NaiveDate, months: u32, anchor: u32) -> NaiveDate {
    let total = date.year() * 12
        + i32::try_from(date.month0()).unwrap_or(0)
        + i32::try_from(months).unwrap_or(0);
    let year = total.div_euclid(12);
    let month = u32::try_from(total.rem_euclid(12)).unwrap_or(0) + 1;
    let day = anchor.clamp(1, 31).min(last_day(year, month));
    NaiveDate::from_ymd_opt(year, month, day).unwrap_or(date)
}

/// The first day of the cycle after the one starting on `start`.
pub fn advance(cycle: BillingCycle, anchor: u32, start: NaiveDate) -> NaiveDate {
    match cycle {
        BillingCycle::Week => start.checked_add_days(Days::new(7)).unwrap_or(start),
        BillingCycle::Month => add_months(start, 1, anchor),
        BillingCycle::Quarter => add_months(start, 3, anchor),
        BillingCycle::Year => add_months(start, 12, anchor),
    }
}

/// The last day of the cycle starting on `start`.
pub fn cycle_end(cycle: BillingCycle, anchor: u32, start: NaiveDate) -> NaiveDate {
    advance(cycle, anchor, start).pred_opt().unwrap_or(start)
}

/// The days of the cycle `start..=end` that are not used, and the cycle's
/// days: from the day after `on` (the cancel day counts as used), or, when
/// paused, from the pause's first day `paused_from` (the first day not
/// used), whatever the cancel date (wl-invoicing §4 `cancel`).
pub fn unused_days(
    start: NaiveDate,
    end: NaiveDate,
    on: NaiveDate,
    paused_from: Option<NaiveDate>,
) -> (i64, i64) {
    let total = (end - start).num_days() + 1;
    let first_unused = match paused_from {
        Some(from) => from,
        None => on.succ_opt().unwrap_or(on),
    };
    let first_unused = first_unused.max(start);
    let unused = ((end - first_unused).num_days() + 1).clamp(0, total);
    (unused, total)
}

/// `amount × part ÷ whole`, rounded half up (away from zero) to the fils.
pub fn pro_rata(amount: i64, part: i64, whole: i64) -> i64 {
    if whole <= 0 {
        return 0;
    }
    let num = i128::from(amount) * i128::from(part);
    let den = i128::from(whole);
    let magnitude = (num.abs() * 2 + den) / (den * 2);
    let rounded = if num < 0 { -magnitude } else { magnitude };
    i64::try_from(rounded).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn monthly_cycles_keep_the_anchor_day() {
        assert_eq!(
            advance(BillingCycle::Month, 1, d(2027, 3, 1)),
            d(2027, 4, 1)
        );
        assert_eq!(
            cycle_end(BillingCycle::Month, 1, d(2027, 3, 1)),
            d(2027, 3, 31)
        );
        // The 31st falls back to the month's last day, then comes back.
        assert_eq!(
            advance(BillingCycle::Month, 31, d(2027, 1, 31)),
            d(2027, 2, 28)
        );
        assert_eq!(
            advance(BillingCycle::Month, 31, d(2027, 2, 28)),
            d(2027, 3, 31)
        );
        assert_eq!(
            advance(BillingCycle::Month, 15, d(2027, 12, 15)),
            d(2028, 1, 15)
        );
        assert_eq!(
            advance(BillingCycle::Quarter, 1, d(2027, 11, 1)),
            d(2028, 2, 1)
        );
        assert_eq!(
            advance(BillingCycle::Year, 29, d(2028, 2, 29)),
            d(2029, 2, 28)
        );
        assert_eq!(advance(BillingCycle::Week, 1, d(2027, 3, 1)), d(2027, 3, 8));
    }

    #[test]
    fn the_cancel_day_counts_as_used() {
        // Withdrawn 10 March: 11 to 31 March unused, 21 of 31 days.
        assert_eq!(
            unused_days(d(2027, 3, 1), d(2027, 3, 31), d(2027, 3, 10), None),
            (21, 31)
        );
        // Paused from 11 March, cancelled 15 April: the same 21 days.
        assert_eq!(
            unused_days(
                d(2027, 3, 1),
                d(2027, 3, 31),
                d(2027, 4, 15),
                Some(d(2027, 3, 11))
            ),
            (21, 31)
        );
        // Cancelled on the last day: nothing unused.
        assert_eq!(
            unused_days(d(2027, 3, 1), d(2027, 3, 31), d(2027, 3, 31), None),
            (0, 31)
        );
    }

    #[test]
    fn pro_rata_rounds_half_up_to_the_fils() {
        assert_eq!(pro_rata(310_000, 21, 31), 210_000);
        assert_eq!(pro_rata(295_238, 21, 31), 200_000);
        assert_eq!(pro_rata(1, 1, 2), 1);
        assert_eq!(pro_rata(100, 0, 31), 0);
        assert_eq!(pro_rata(100, 5, 0), 0);
    }
}
