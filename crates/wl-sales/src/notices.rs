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

/// A member is told that a reward is available (`SAL.RUL.07`).
pub const LOYALTY_REWARD: &str = "sales.loyalty_reward";

/// A member is told that earned points were taken back (`SAL.RUL.09`).
pub const LOYALTY_REVERSED: &str = "sales.loyalty_reversed";

/// A member is told that redeemed points were given back.
pub const LOYALTY_GIVEN_BACK: &str = "sales.loyalty_given_back";

/// A member is told that points are about to expire (`SAL.RUL.08`).
pub const LOYALTY_EXPIRY: &str = "sales.loyalty_expiry";

/// The requester of a points adjustment is told that it was rejected.
pub const ADJUSTMENT_REJECTED: &str = "sales.loyalty_adjustment_rejected";

/// The requester of a points adjustment is told that it was returned.
pub const ADJUSTMENT_RETURNED: &str = "sales.loyalty_adjustment_returned";

/// A member is told that the terms of the programme change on a date
/// (`transactional`: wl-sales §4.7, a notice, not marketing).
pub const LOYALTY_TERMS_CHANGED: &str = "sales.loyalty_terms_changed";

/// A member is told that the programme was turned off and until when points
/// can be redeemed.
pub const LOYALTY_PROGRAMME_OFF: &str = "sales.loyalty_programme_off";

/// A member is told that the account was closed.
pub const LOYALTY_CLOSED: &str = "sales.loyalty_closed";

/// The customer's sales rep and the accountants are told that an order went
/// on credit hold (`SAL.RUL.01`).
pub const ORDER_CREDIT_HOLD: &str = "sales.order_credit_hold";

/// The customer's sales rep and the accountants are told that the account
/// went on hold after a bounced cheque (`SAL.RUL.05`).
pub const ACCOUNT_ON_HOLD: &str = "sales.account_on_hold";

/// The customer is told that the online payment of an order failed (BL-SAL-03).
pub const ORDER_PAYMENT_FAILED: &str = "sales.order_payment_failed";

/// The customer is told that a payment for an order that cannot take it is
/// refunded (`SAL.RUL.06`).
pub const ORDER_LATE_PAYMENT: &str = "sales.order_late_payment_refunded";

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
    notify::register_type(english_type(LOYALTY_REWARD, "loyalty_reward"));
    notify::register_type(english_type(LOYALTY_REVERSED, "loyalty_reversed"));
    notify::register_type(english_type(LOYALTY_GIVEN_BACK, "loyalty_given_back"));
    // A reminder: the member's opt-out is respected, and when it gives up
    // nothing happens but the account showing it (`SAL.RUL.08`).
    notify::register_type(
        english_type(LOYALTY_EXPIRY, "loyalty_expiry")
            .category(notify::Category::Reminder)
            .give_up(&[notify::GiveUpReaction::None]),
    );
    notify::register_type(english_type(LOYALTY_TERMS_CHANGED, "loyalty_terms_changed"));
    notify::register_type(english_type(LOYALTY_PROGRAMME_OFF, "loyalty_programme_off"));
    notify::register_type(english_type(LOYALTY_CLOSED, "loyalty_closed"));
    notify::register_type(english_type(ADJUSTMENT_REJECTED, "adjustment_rejected"));
    notify::register_type(english_type(ADJUSTMENT_RETURNED, "adjustment_returned"));
    notify::register_type(english_type(ORDER_PAYMENT_FAILED, "order_payment_failed"));
    notify::register_type(english_type(ORDER_LATE_PAYMENT, "order_late_payment"));
    notify::register_type(english_type(ORDER_CREDIT_HOLD, "order_credit_hold"));
    notify::register_type(english_type(ACCOUNT_ON_HOLD, "account_on_hold"));
    notify::register_type(english_type(
        crate::deliveries::DELIVERY_RETURNED,
        "delivery_returned",
    ));
}

/// The users told of a credit hold of `party`'s order: the sales rep of its
/// account, then the holders of the accountant role.
async fn credit_audience(state: &AppState, party: i64) -> Result<Vec<i64>> {
    let mut users = Vec::new();
    if let Some(account) =
        crate::customer_account::CustomerAccount::of_party(&state.db, party).await?
        && let Some(rep) = account.sales_rep_id
    {
        users.push(rep);
    }
    let role = crate::config::config_of(state).accountant_role;
    for user in renox::auth::permissions::users_with_role(&state.db, &role).await? {
        if !users.contains(&user.id) {
            users.push(user.id);
        }
    }
    Ok(users)
}

/// Tells the sales rep and the accountants that `account` is on hold after
/// the bounced `source` (`SAL.RUL.05`); `orders` of its customer were held.
pub(crate) async fn account_on_hold(
    state: &AppState,
    account: &crate::customer_account::CustomerAccount,
    source: &str,
    orders: i64,
) -> Result {
    let name = wl_kernel::parties::directory(state)
        .display_name(&state.db, wl_kernel::PartyId(account.party_id), "en")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{}", account.party_id));
    let audience = credit_audience(state, account.party_id).await?;
    if audience.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(ACCOUNT_ON_HOLD, account.subject())
        .subject_label(&name)
        .data(json!({
            "customer": name,
            "source": source,
            "orders": orders,
        }));
    for user in audience {
        request = request.to_user(user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the sales rep and the accountants that `order` is on credit hold
/// (`SAL.RUL.01`), with the reason and the figures kept on the order.
pub(crate) async fn order_credit_held(
    state: &AppState,
    order: &crate::sales_order::SalesOrder,
) -> Result {
    let name = wl_kernel::parties::directory(state)
        .display_name(&state.db, wl_kernel::PartyId(order.party_id), "en")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{}", order.party_id));
    let reason = english(
        &format!(
            "wl_sales.credit.reason.{}",
            order.hold_reason.as_deref().unwrap_or("limit")
        ),
        &[],
    );
    let limit = order
        .hold_limit
        .map_or_else(|| "-".to_owned(), |fils| format!("AED {}", aed(fils)));
    let mut audience = credit_audience(state, order.party_id).await?;
    // A customer without an account rep: the user who entered the order.
    if let Some(entered) = order.created_by
        && !audience.contains(&entered)
        && crate::customer_account::CustomerAccount::of_party(&state.db, order.party_id)
            .await?
            .is_none_or(|account| account.sales_rep_id.is_none())
    {
        audience.insert(0, entered);
    }
    open_credit_task(state, order, &reason).await?;
    if audience.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(ORDER_CREDIT_HOLD, order.subject())
        .subject_label(&order.number)
        .data(json!({
            "number": order.number,
            "customer": name,
            "reason": reason,
            "open_balance": format!("AED {}", aed(order.hold_open_balance.unwrap_or_default())),
            "overdue": format!("AED {}", aed(order.hold_overdue.unwrap_or_default())),
            "limit": limit,
        }));
    for user in audience {
        request = request.to_user(user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// The task of the accountant role for an order on credit hold: review the
/// account and release the order or ask for payment. One per order and
/// revision.
async fn open_credit_task(
    state: &AppState,
    order: &crate::sales_order::SalesOrder,
    reason: &str,
) -> Result {
    let config = crate::config::config_of(state);
    let role = if config.accountant_role.is_empty() {
        wl_kernel::COMPANY_ADMIN_ROLE
    } else {
        config.accountant_role.as_str()
    };
    let (title, body) = {
        let total = format!("AED {}", aed(order.gross_total));
        let params: [(&str, &dyn std::fmt::Display); 3] = [
            ("number", &order.number),
            ("total", &total),
            ("reason", &reason),
        ];
        (
            english("wl_sales.credit.task_title", &params),
            english("wl_sales.credit.task_body", &params),
        )
    };
    let key = format!("sales.credit_hold:{}:{}", order.id, order.revision);
    let task = wl_kernel::role_tasks::RoleTask::new(order.subject(), role, &title, &key)
        .description(&body);
    wl_kernel::role_tasks::open(state, task).await?;
    Ok(())
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

/// Tells the member of `account` that a reward is available until
/// `valid_until` (the guardian for a minor).
pub(crate) async fn loyalty_reward(
    state: &AppState,
    account: &crate::loyalty_account::LoyaltyAccount,
    reward: &str,
    valid_until: NaiveDate,
) -> Result {
    // A paused account (the party is on hold) gets no loyalty message.
    if account.paused {
        return Ok(());
    }
    let request = NotifyRequest::new(LOYALTY_REWARD, account.subject())
        .to_party(account.notify_party())
        .subject_label(&account.code)
        .data(json!({
            "code": account.code,
            "reward": reward,
            "valid_until": date_text(Some(valid_until)),
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the member of `account` that `points` were taken back for `why`
/// (the guardian for a minor).
pub(crate) async fn loyalty_reversed(
    state: &AppState,
    account: &crate::loyalty_account::LoyaltyAccount,
    points: i64,
    why: &str,
) -> Result {
    // A paused account (the party is on hold) gets no loyalty message.
    if account.paused {
        return Ok(());
    }
    let request = NotifyRequest::new(LOYALTY_REVERSED, account.subject())
        .to_party(account.notify_party())
        .subject_label(&account.code)
        .data(json!({
            "code": account.code,
            "points": points,
            "why": why,
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the member of `account` that `points` they redeemed were given back
/// for `why` (the guardian for a minor).
pub(crate) async fn loyalty_given_back(
    state: &AppState,
    account: &crate::loyalty_account::LoyaltyAccount,
    points: i64,
    why: &str,
) -> Result {
    // A paused account (the party is on hold) gets no loyalty message.
    if account.paused {
        return Ok(());
    }
    let request = NotifyRequest::new(LOYALTY_GIVEN_BACK, account.subject())
        .to_party(account.notify_party())
        .subject_label(&account.code)
        .data(json!({
            "code": account.code,
            "points": points,
            "why": why,
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the member of `account` that `points` expire on `expires_on` unless
/// they are used (the guardian for a minor).
pub(crate) async fn loyalty_expiry(
    state: &AppState,
    account: &crate::loyalty_account::LoyaltyAccount,
    points: i64,
    value: i64,
    expires_on: NaiveDate,
) -> Result {
    // A paused account (the party is on hold) gets no loyalty message.
    if account.paused {
        return Ok(());
    }
    let request = NotifyRequest::new(LOYALTY_EXPIRY, account.subject())
        .to_party(account.notify_party())
        .subject_label(&account.code)
        .data(json!({
            "code": account.code,
            "points": points,
            "value": aed(value),
            "expires_on": date_text(Some(expires_on)),
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the requester of `adjustment` that it was rejected or returned
/// (`reason` is the manager's comment).
pub(crate) async fn adjustment_ended(
    state: &AppState,
    adjustment: &crate::adjustment::PointsAdjustment,
    account_code: &str,
    key: &str,
    reason: &str,
) -> Result {
    let request = NotifyRequest::new(key, adjustment.subject())
        .to_user(adjustment.requested_by)
        .subject_label(account_code)
        .data(json!({
            "code": account_code,
            "points": adjustment.points,
            "reason": reason,
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the member of `account` that the terms change on `effective_from`:
/// `summary` says what changes. Points already earned keep their value and
/// expiry. A paused account is told nothing.
pub(crate) async fn loyalty_terms_changed(
    state: &AppState,
    account: &crate::loyalty_account::LoyaltyAccount,
    summary: &str,
    effective_from: NaiveDate,
) -> Result {
    if account.paused {
        return Ok(());
    }
    let request = NotifyRequest::new(LOYALTY_TERMS_CHANGED, account.subject())
        .to_party(account.notify_party())
        .subject_label(&account.code)
        .data(json!({
            "code": account.code,
            "summary": summary,
            "effective_from": date_text(Some(effective_from)),
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the member of `account` that the programme was turned off and that
/// points can be redeemed until `closing`.
pub(crate) async fn loyalty_programme_off(
    state: &AppState,
    account: &crate::loyalty_account::LoyaltyAccount,
    closing: NaiveDate,
) -> Result {
    if account.paused {
        return Ok(());
    }
    let request = NotifyRequest::new(LOYALTY_PROGRAMME_OFF, account.subject())
        .to_party(account.notify_party())
        .subject_label(&account.code)
        .data(json!({
            "code": account.code,
            "points": account.points_balance.max(0),
            "closing_date": date_text(Some(closing)),
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the member of `account` that it was closed and how many points went
/// with it (the guardian for a minor).
pub(crate) async fn loyalty_closed(
    state: &AppState,
    account: &crate::loyalty_account::LoyaltyAccount,
    points: i64,
) -> Result {
    let request = NotifyRequest::new(LOYALTY_CLOSED, account.subject())
        .to_party(account.notify_party())
        .subject_label(&account.code)
        .data(json!({
            "code": account.code,
            "points": points,
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the customer of `order` that the online payment failed and that a new
/// order can be placed (the failed order cannot be retried).
pub(crate) async fn order_payment_failed(
    state: &AppState,
    order: &crate::sales_order::SalesOrder,
) -> Result {
    let request = NotifyRequest::new(ORDER_PAYMENT_FAILED, order.subject())
        .to_party(order.party())
        .subject_label(&order.number)
        .data(json!({
            "number": order.number,
            "total": aed(order.gross_total),
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the customer of `order` that the payment of `amount` fils, which the
/// order could not take, is on its way back.
pub(crate) async fn order_late_payment(
    state: &AppState,
    order: &crate::sales_order::SalesOrder,
    amount: i64,
) -> Result {
    let request = NotifyRequest::new(ORDER_LATE_PAYMENT, order.subject())
        .to_party(order.party())
        .subject_label(&order.number)
        .data(json!({
            "number": order.number,
            "amount": aed(amount),
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}
