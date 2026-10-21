//! The module's notifications (#138 A17), sent through the kernel
//! [`Notifier`](wl_kernel::notify::Notifier): transactional, by e-mail, with
//! English texts first, to the campaign's owner. Without wl-notify the
//! kernel's fallback puts them in the bell.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::campaign::Campaign;
use crate::cost_entry::CostEntry;
use crate::texts::english;

/// The owner is told that the campaign ended.
pub const CAMPAIGN_ENDED: &str = "sales.campaign_ended";

/// The owner is told that a campaign reached a share of its budget.
pub const BUDGET_ALERT: &str = "sales.campaign_budget_alert";
/// The owner is told that a cost entry was rejected.
pub const COST_REJECTED: &str = "sales.campaign_cost_rejected";
/// The owner is told that a cost entry was voided with its supplier bill.
pub const COST_BILL_VOIDED: &str = "sales.campaign_cost_bill_voided";

/// The holder (and payer) of a package is told that a balance is due again.
pub const PACKAGE_BALANCE_DUE: &str = "sales.package_balance_due";

/// The owner of a lead is told that its win was reverted.
pub const LEAD_WIN_REVERTED: &str = "sales.lead_win_reverted";

/// `date` as `18 Oct 2026`, empty when unset.
fn date_text(date: Option<NaiveDate>) -> String {
    date.map(|d| d.format("%-d %b %Y").to_string())
        .unwrap_or_default()
}

/// The notification type `sales.campaign_ended`.
pub fn campaign_ended_type() -> NotificationTypeDef {
    let subject = english("wl_sales.notices.ended_subject", &[]);
    let body = english("wl_sales.notices.ended_body", &[]);
    NotificationTypeDef::new(CAMPAIGN_ENDED, "sales")
        .group("sales")
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// A transactional e-mail type of the sales module with English texts.
fn english_type(key: &str, texts: &str) -> NotificationTypeDef {
    let subject = english(&format!("wl_sales.notices.{texts}_subject"), &[]);
    let body = english(&format!("wl_sales.notices.{texts}_body"), &[]);
    NotificationTypeDef::new(key, "sales")
        .group("sales")
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// Registers the module's notification types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(campaign_ended_type());
    notify::register_type(english_type(BUDGET_ALERT, "budget_alert"));
    notify::register_type(english_type(COST_REJECTED, "cost_rejected"));
    notify::register_type(english_type(COST_BILL_VOIDED, "cost_bill_voided"));
    notify::register_type(english_type(LEAD_WIN_REVERTED, "lead_win_reverted"));
    notify::register_type(english_type(PACKAGE_BALANCE_DUE, "package_balance_due"));
}

/// `fils` as `4,000.00`.
pub(crate) fn aed(fils: i64) -> String {
    let sign = if fils < 0 { "-" } else { "" };
    let fils = fils.abs();
    let whole = (fils / 100).to_string();
    let mut grouped = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{sign}{grouped}.{:02}", fils % 100)
}

/// Tells the owner of `campaign` that its approved costs reached `bps` basis
/// points of the budget.
pub(crate) async fn budget_alert(
    state: &AppState,
    campaign: &Campaign,
    bps: u32,
    total: i64,
) -> Result {
    let Some(owner) = campaign.owner_id else {
        return Ok(());
    };
    let request = NotifyRequest::new(BUDGET_ALERT, campaign.campaign_id().subject())
        .to_user(owner)
        .subject_label(&campaign.code)
        .data(json!({
            "name": campaign.name,
            "code": campaign.code,
            "percent": bps / 100,
            "approved": aed(total),
            "budget": aed(campaign.budget.unwrap_or_default()),
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the owner of the entry's campaign that the entry was rejected.
pub(crate) async fn cost_rejected(state: &AppState, entry: &CostEntry) -> Result {
    let Some(campaign) = Campaign::find(&state.db, entry.campaign_id).await? else {
        return Ok(());
    };
    let Some(owner) = campaign.owner_id else {
        return Ok(());
    };
    let request = NotifyRequest::new(COST_REJECTED, campaign.campaign_id().subject())
        .to_user(owner)
        .subject_label(&campaign.code)
        .data(json!({
            "name": campaign.name,
            "code": campaign.code,
            "amount": aed(entry.amount),
            "description": entry.description,
            "reason": entry.reason.clone().unwrap_or_default(),
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the owner of the entry's campaign that the entry was voided with
/// its supplier bill.
pub(crate) async fn cost_bill_voided(state: &AppState, entry: &CostEntry) -> Result {
    let Some(campaign) = Campaign::find(&state.db, entry.campaign_id).await? else {
        return Ok(());
    };
    let Some(owner) = campaign.owner_id else {
        return Ok(());
    };
    let request = NotifyRequest::new(COST_BILL_VOIDED, campaign.campaign_id().subject())
        .to_user(owner)
        .subject_label(&campaign.code)
        .data(json!({
            "name": campaign.name,
            "code": campaign.code,
            "amount": aed(entry.amount),
            "description": entry.description,
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the campaign's owner that it ended. A campaign without an owner
/// tells nobody.
pub(crate) async fn campaign_ended(state: &AppState, campaign: &Campaign) -> Result {
    let Some(owner) = campaign.owner_id else {
        return Ok(());
    };
    let request = NotifyRequest::new(CAMPAIGN_ENDED, campaign.campaign_id().subject())
        .to_user(owner)
        .subject_label(&campaign.code)
        .data(json!({
            "name": campaign.name,
            "code": campaign.code,
            "end_date": date_text(campaign.end_date),
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the owner of `lead` that it is `qualified` again because its
/// conversion record `voided` was voided as entered in error. A lead without
/// an owner tells nobody.
pub(crate) async fn lead_win_reverted(
    state: &AppState,
    lead: &crate::lead::Lead,
    voided: &wl_kernel::Subject,
) -> Result {
    let Some(owner) = lead.owner_id else {
        return Ok(());
    };
    let request = NotifyRequest::new(LEAD_WIN_REVERTED, lead.subject())
        .to_user(owner)
        .subject_label(&lead.contact_name)
        .data(json!({
            "name": lead.contact_name,
            "voided": voided.to_string(),
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}
