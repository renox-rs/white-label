//! Escalating an open dispute (wl-invoicing §5.10, `FIN.RUL.15`).
//!
//! An open dispute is told again every N whole days since it was raised
//! (Asia/Dubai, N = `dispute_escalation_days`, default 14). [`step_on`]
//! counts the steps; [`notification_type`] is the notice
//! ([`ESCALATED`]) and [`data`] its variables.

use renox::chrono::NaiveDate;
use renox::serde_json::{Value, json};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel};

use crate::allocate::amount_text;
use crate::dispute::Dispute;
use crate::invoice::Invoice;
use crate::texts::english;

/// The rule, as the specification names it.
pub const RULE: &str = "FIN.RUL.15";

/// The notification type of an escalation.
pub const ESCALATED: &str = "invoicing.dispute_escalation";

/// The module that sends the notice.
const MODULE: &str = "invoicing";

/// The row of the wl-notify §5.4 default table the type belongs to.
const GROUP: &str = "invoices";

/// The step that is current on `today` for a dispute raised on `raised_on`
/// (its Asia/Dubai date): `(today - raised_on) / n` whole days, when that
/// is at least 1, else `None`. The step stays the same until the next
/// whole `n` days have passed, so a day the task skipped is caught up by the
/// next run, which sends the current step only. `n` is never below 1 (0 is
/// read as 1).
pub fn step_on(raised_on: NaiveDate, today: NaiveDate, n: u32) -> Option<u32> {
    let days = (today - raised_on).num_days();
    if days < 0 {
        return None;
    }
    let step = u32::try_from(days / i64::from(n.max(1))).ok()?;
    (step >= 1).then_some(step)
}

/// `date` as `18 Oct 2026`.
fn date_text(date: NaiveDate) -> String {
    date.format("%-d %b %Y").to_string()
}

/// Who a copy of the notice is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience<'a> {
    /// The owner side: the owner, or the receivables holders.
    Owner,
    /// The finance manager, with the owner's name (or "receivables").
    FinanceManager {
        /// The owner's name, or `receivables` when the owner is gone.
        owner: &'a str,
    },
}

/// The template variables of the notice for `step` (the days open are
/// `step * days`). `ticket_number` is the dispute's ticket, when it has one.
pub fn data(
    invoice: &Invoice,
    dispute: &Dispute,
    raised_on: NaiveDate,
    step: u32,
    days: u32,
    ticket_number: Option<&str>,
    audience: Audience<'_>,
) -> Value {
    let party_name = invoice
        .party_snapshot
        .as_ref()
        .map(|snapshot| snapshot.0.legal_name_en.clone())
        .unwrap_or_default();
    let owner = match audience {
        Audience::Owner => "",
        Audience::FinanceManager { owner } => owner,
    };
    json!({
        "number": invoice.number.clone().unwrap_or_default(),
        "party_name": party_name,
        "amount": amount_text("AED", dispute.amount),
        "reason": dispute.reason,
        "raised_on": date_text(raised_on),
        "days": step.saturating_mul(days),
        "step": step,
        "ticket": ticket_number.unwrap_or("-"),
        "owner": owner,
        "link": format!("/invoices/{}", invoice.id),
    })
}

/// The notification type [`ESCALATED`]: staff, transactional, by e-mail.
pub fn notification_type() -> NotificationTypeDef {
    NotificationTypeDef::new(ESCALATED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_invoicing.escalation.subject")),
            english("wl_invoicing.escalation.body"),
        )
}

/// Registers [`ESCALATED`] with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(notification_type());
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::documents::dubai_date;

    fn day(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    #[test]
    fn nothing_before_the_first_whole_period() {
        assert_eq!(step_on(day(11, 1), day(11, 1), 14), None);
        assert_eq!(step_on(day(11, 1), day(11, 14), 14), None);
        assert_eq!(step_on(day(11, 1), day(10, 20), 14), None);
    }

    #[test]
    fn steps_of_fourteen_days() {
        assert_eq!(step_on(day(11, 1), day(11, 15), 14), Some(1));
        assert_eq!(step_on(day(11, 1), day(11, 28), 14), Some(1));
        assert_eq!(step_on(day(11, 1), day(11, 29), 14), Some(2));
        assert_eq!(step_on(day(11, 1), day(12, 12), 14), Some(2));
        assert_eq!(step_on(day(11, 1), day(12, 13), 14), Some(3));
    }

    #[test]
    fn steps_of_seven_days() {
        assert_eq!(step_on(day(11, 1), day(11, 7), 7), None);
        assert_eq!(step_on(day(11, 1), day(11, 8), 7), Some(1));
        assert_eq!(step_on(day(11, 1), day(11, 15), 7), Some(2));
    }

    #[test]
    fn a_skipped_day_is_caught_up_with_the_current_step() {
        // No run on 15 Nov: the run of 16 Nov still sees step 1, and the run
        // of 30 Nov sees step 2 (step 1 was missed and is not sent).
        assert_eq!(step_on(day(11, 1), day(11, 16), 14), Some(1));
        assert_eq!(step_on(day(11, 1), day(11, 30), 14), Some(2));
    }

    #[test]
    fn a_zero_period_counts_as_one_day() {
        assert_eq!(step_on(day(11, 1), day(11, 3), 0), Some(2));
    }

    #[test]
    fn a_late_evening_utc_is_the_next_dubai_day() {
        let raised_at = NaiveDate::from_ymd_opt(2026, 10, 31)
            .unwrap()
            .and_hms_opt(21, 30, 0)
            .unwrap()
            .and_utc();
        let raised_on = dubai_date(raised_at);
        assert_eq!(raised_on, day(11, 1));
        assert_eq!(step_on(raised_on, day(11, 14), 14), None);
        assert_eq!(step_on(raised_on, day(11, 15), 14), Some(1));
    }

    #[test]
    fn the_notice_has_its_texts() {
        assert!(english("wl_invoicing.escalation.body").contains("{{ days }}"));
        assert_eq!(notification_type().key, ESCALATED);
    }
}
