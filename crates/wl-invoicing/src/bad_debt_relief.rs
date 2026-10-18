//! Proposing bad-debt VAT relief (wl-invoicing §5.9, `FIN.RUL.14`).
//!
//! A write-off leaves a `pending` bad-debt relief adjustment (#64).
//! [`run`] proposes each one whose conditions are all met on the given day:
//!
//! - the invoice is `written_off` and has its write-off notice date;
//! - the amount is above zero;
//! - the day is after the date of supply plus the configured months
//!   ([`eligible_from`]; month ends are clamped, so 31 Aug + 6 months is
//!   28 Feb);
//! - the VAT summary of the issuing entity whose period contains the date of
//!   supply exists and is `locked`.
//!
//! The adjustment goes onto the issuing entity's `open` summary of the period
//! that contains the day; without one it stays `pending` and the next run
//! tries again. `propose` runs in its own transaction per adjustment, and the
//! accountants are told after the commit. The rule never touches the
//! invoice, never creates a credit note and never includes an adjustment.
//! The daily task [`TASK`] calls [`run`] for today in Asia/Dubai.

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::chrono::{Days, Months, NaiveDate};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::LegalEntity;
use wl_kernel::documents::today;
use wl_kernel::invoicing::InvoiceStatus;
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::allocate::amount_text;
use crate::config::{InvoicingConfig, ReliefMonths};
use crate::config_of;
use crate::invoice::Invoice;
use crate::texts::english;
use crate::vat_adjustment::{AdjustmentKind, AdjustmentStatus, ProposeInput, VatAdjustment};
use crate::vat_summary::{VatSummary, VatSummaryStatus};

/// The rule, as the specification names it.
pub const RULE: &str = "FIN.RUL.14";

/// The notification type of a proposal.
pub const PROPOSED: &str = "invoicing.bad_debt_relief_proposed";

/// The daily scheduled task that runs [`run`].
pub const TASK: &str = "invoicing:bad-debt-relief";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The module that sends the notice.
const MODULE: &str = "invoicing";

/// The row of the wl-notify §5.4 default table the type belongs to.
const GROUP: &str = "invoices";

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReliefRun {
    /// How many adjustments it proposed.
    pub proposed: usize,
    /// How many were eligible but had no `open` summary to go on.
    pub waiting_for_open_summary: usize,
}

/// The first day on which relief may be proposed for a supply on `supply`:
/// the day after `supply` plus `months`, the end of a month clamped.
/// `None` when the date is out of range.
pub fn eligible_from(supply: NaiveDate, months: ReliefMonths) -> Option<NaiveDate> {
    supply
        .checked_add_months(Months::new(months.get()))?
        .checked_add_days(Days::new(1))
}

/// What the eligibility check reads, besides the adjustment's own fields.
#[derive(Debug, Clone, Copy)]
pub struct Facts {
    /// The status of the summary that contains the date of supply, when
    /// there is one.
    pub supply_summary: Option<VatSummaryStatus>,
}

/// Whether `adjustment` for `invoice` may be proposed on `on` (`FIN.RUL.14`).
/// A pure check; the `open` summary to put it on is looked up apart.
pub fn eligible(
    adjustment: &VatAdjustment,
    invoice: &Invoice,
    months: ReliefMonths,
    facts: Facts,
    on: NaiveDate,
) -> bool {
    if adjustment.kind != AdjustmentKind::BadDebtRelief
        || adjustment.status != AdjustmentStatus::Pending
        || adjustment.amount <= 0
        || invoice.status != InvoiceStatus::WrittenOff
        || invoice.write_off_notice_on.is_none()
        || facts.supply_summary != Some(VatSummaryStatus::Locked)
    {
        return false;
    }
    let Some(supply) = invoice.date_of_supply.or(invoice.issue_date) else {
        return false;
    };
    eligible_from(supply, months).is_some_and(|first| on >= first)
}

/// `date` as `18 Oct 2026`.
fn date_text(date: NaiveDate) -> String {
    date.format("%-d %b %Y").to_string()
}

/// The template variables of the notice.
fn data(
    invoice: &Invoice,
    adjustment: &VatAdjustment,
    summary: &VatSummary,
) -> renox::serde_json::Value {
    let party_name = invoice
        .party_snapshot
        .as_ref()
        .map(|snapshot| snapshot.0.legal_name_en.clone())
        .unwrap_or_default();
    json!({
        "number": invoice.number.clone().unwrap_or_default(),
        "party_name": party_name,
        "amount": amount_text("AED", adjustment.amount),
        "period": format!(
            "{} to {}",
            date_text(summary.period_start),
            date_text(summary.period_end)
        ),
        "link": format!("/vat-summaries/{}", summary.id),
    })
}

/// The notification type [`PROPOSED`]: staff, transactional, by e-mail.
pub fn notification_type() -> NotificationTypeDef {
    NotificationTypeDef::new(PROPOSED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_invoicing.relief.proposed.subject")),
            english("wl_invoicing.relief.proposed.body"),
        )
}

/// Registers [`PROPOSED`] with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(notification_type());
}

/// Proposes the bad-debt relief that is due on `on` (`FIN.RUL.14`).
///
/// It looks at every `pending` bad-debt relief adjustment each run, so a
/// day that was skipped is caught up on the next one. A second run finds
/// nothing pending. An adjustment that changed in the meantime is left as
/// it is; any other failure stops the run with its error.
pub async fn run(state: &AppState, on: NaiveDate) -> Result<ReliefRun> {
    let config = config_of(state);
    let pending = VatAdjustment::query()
        .where_(VatAdjustment::KIND.eq(AdjustmentKind::BadDebtRelief))
        .where_(VatAdjustment::STATUS.eq(AdjustmentStatus::Pending))
        .order_by(VatAdjustment::ID)
        .get(&state.db)
        .await?;
    let mut result = ReliefRun::default();
    for adjustment in pending {
        match propose_one(state, &config, &adjustment, on).await? {
            Outcome::Proposed => result.proposed += 1,
            Outcome::NoOpenSummary => result.waiting_for_open_summary += 1,
            Outcome::Skipped => {}
        }
    }
    Ok(result)
}

enum Outcome {
    Proposed,
    NoOpenSummary,
    Skipped,
}

async fn propose_one(
    state: &AppState,
    config: &InvoicingConfig,
    adjustment: &VatAdjustment,
    on: NaiveDate,
) -> Result<Outcome> {
    let Some(entity_id) = adjustment.legal_entity_id else {
        return Ok(Outcome::Skipped);
    };
    let Some(invoice) = Invoice::find(&state.db, adjustment.source_id).await? else {
        return Ok(Outcome::Skipped);
    };
    let Some(entity) = LegalEntity::find(&state.db, entity_id).await? else {
        return Ok(Outcome::Skipped);
    };
    let Some(supply) = invoice.date_of_supply.or(invoice.issue_date) else {
        return Ok(Outcome::Skipped);
    };
    let supply_summary = VatSummary::containing(&state.db, config, &entity, supply)
        .await?
        .map(|s| s.status);
    let facts = Facts { supply_summary };
    if !eligible(
        adjustment,
        &invoice,
        config.bad_debt_relief_months,
        facts,
        on,
    ) {
        return Ok(Outcome::Skipped);
    }
    let summary = VatSummary::containing(&state.db, config, &entity, on)
        .await?
        .filter(|s| s.status == VatSummaryStatus::Open);
    let Some(summary) = summary else {
        return Ok(Outcome::NoOpenSummary);
    };
    let input = ProposeInput {
        summary_id: summary.id,
    };
    match adjustment
        .transition(crate::vat_adjustment::PROPOSE)
        .by_system()
        .input(&input)
        .run(state)
        .await
    {
        Ok(outcome) => {
            // After the commit.
            outcome.emit(state).await?;
        }
        Err(
            TransitionError::NotFrom(_) | TransitionError::Guard(_) | TransitionError::Conflict,
        ) => return Ok(Outcome::Skipped),
        Err(error) => return Err(error.into_error()),
    }
    notify_accountants(state, config, &invoice, adjustment, &summary).await?;
    Ok(Outcome::Proposed)
}

/// Tells each holder of the notify role, one message per user.
async fn notify_accountants(
    state: &AppState,
    config: &InvoicingConfig,
    invoice: &Invoice,
    adjustment: &VatAdjustment,
    summary: &VatSummary,
) -> Result {
    let mut ids: Vec<i64> =
        users_with_role_in(&state.db, &config.relief_notify_role, &Scope::global())
            .await?
            .iter()
            .map(|user| user.id)
            .collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(());
    }
    let mut request =
        NotifyRequest::new(PROPOSED, invoice.subject()).data(data(invoice, adjustment, summary));
    for id in ids {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Registers the daily [`TASK`] at the configured time, unless relief is off.
pub(crate) fn schedule(app: &mut Registry, config: &InvoicingConfig) {
    if !config.bad_debt_relief {
        return;
    }
    app.schedule()
        .daily_at(
            &config.bad_debt_relief_run_at,
            TASK,
            |state: AppState| async move { run(&state, today()).await.map(|_| ()) },
        )
        .timezone(TASK_ZONE);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("date")
    }

    fn six() -> ReliefMonths {
        ReliefMonths::new(6).expect("six")
    }

    fn facts(status: Option<VatSummaryStatus>) -> Facts {
        Facts {
            supply_summary: status,
        }
    }

    fn written_off(supply: NaiveDate) -> Invoice {
        Invoice {
            status: InvoiceStatus::WrittenOff,
            date_of_supply: Some(supply),
            write_off_notice_on: Some(day(2026, 7, 1)),
            ..Default::default()
        }
    }

    fn pending(amount: i64) -> VatAdjustment {
        VatAdjustment {
            amount,
            ..Default::default()
        }
    }

    #[test]
    fn the_first_day_is_the_one_after_the_months() {
        assert_eq!(
            eligible_from(day(2026, 3, 15), six()),
            Some(day(2026, 9, 16))
        );
        assert_eq!(
            eligible_from(day(2026, 3, 15), ReliefMonths::new(9).unwrap()),
            Some(day(2026, 12, 16))
        );
    }

    #[test]
    fn the_end_of_a_month_is_clamped() {
        assert_eq!(
            eligible_from(day(2026, 8, 31), six()),
            Some(day(2027, 3, 1))
        );
        assert_eq!(
            eligible_from(day(2027, 8, 31), six()),
            Some(day(2028, 3, 1))
        );
        assert_eq!(
            eligible_from(day(2026, 12, 31), six()),
            Some(day(2027, 7, 1))
        );
    }

    #[test]
    fn it_is_eligible_from_the_first_day_only() {
        let invoice = written_off(day(2026, 3, 15));
        let locked = facts(Some(VatSummaryStatus::Locked));
        let check = |on| eligible(&pending(500), &invoice, six(), locked, on);
        assert!(!check(day(2026, 9, 15)));
        assert!(check(day(2026, 9, 16)));
        assert!(check(day(2026, 12, 1)));
    }

    #[test]
    fn the_issue_date_stands_in_for_a_missing_date_of_supply() {
        let invoice = Invoice {
            date_of_supply: None,
            issue_date: Some(day(2026, 3, 15)),
            ..written_off(day(2026, 3, 15))
        };
        let locked = facts(Some(VatSummaryStatus::Locked));
        assert!(eligible(
            &pending(500),
            &invoice,
            six(),
            locked,
            day(2026, 9, 16)
        ));
        let none = Invoice {
            issue_date: None,
            ..invoice
        };
        assert!(!eligible(
            &pending(500),
            &none,
            six(),
            locked,
            day(2026, 9, 16)
        ));
    }

    #[test]
    fn every_condition_is_needed() {
        let on = day(2026, 9, 16);
        let invoice = written_off(day(2026, 3, 15));
        let locked = facts(Some(VatSummaryStatus::Locked));
        assert!(eligible(&pending(500), &invoice, six(), locked, on));

        let no_notice = Invoice {
            write_off_notice_on: None,
            ..invoice.clone()
        };
        assert!(!eligible(&pending(500), &no_notice, six(), locked, on));

        let paid = Invoice {
            status: InvoiceStatus::Paid,
            ..invoice.clone()
        };
        assert!(!eligible(&pending(500), &paid, six(), locked, on));

        assert!(!eligible(&pending(0), &invoice, six(), locked, on));

        for status in [
            None,
            Some(VatSummaryStatus::Open),
            Some(VatSummaryStatus::Prepared),
            Some(VatSummaryStatus::Reviewed),
        ] {
            assert!(!eligible(&pending(500), &invoice, six(), facts(status), on));
        }

        let proposed = VatAdjustment {
            status: AdjustmentStatus::Proposed,
            ..pending(500)
        };
        assert!(!eligible(&proposed, &invoice, six(), locked, on));
        let other = VatAdjustment {
            kind: AdjustmentKind::Manual,
            ..pending(500)
        };
        assert!(!eligible(&other, &invoice, six(), locked, on));
    }

    #[test]
    fn the_notification_type_is_registered_with_english_text() {
        let def = notification_type();
        assert_eq!(def.key, "invoicing.bad_debt_relief_proposed");
        assert_eq!(def.category, Category::Transactional);
        assert_eq!(def.channels, [NotifyChannel::Email]);
        let template = def
            .template_for(NotifyChannel::Email, "en")
            .expect("template");
        assert!(template.body.contains("{{ number }}"));
        assert!(
            template
                .subject
                .as_deref()
                .is_some_and(|s| s.contains("{{ number }}"))
        );
    }
}
