//! The shared-mechanism settings of the 06 F&B client on the two settings
//! pages: `/settings/negative-flow` (delivery, sales, payments, cash, the
//! cash-flow forecast and the budget; BL-FNB-16) and `/settings/compliance`
//! (stock, documents, rules, connections, notices, sales, staff and
//! registrations; BL-FNB-17), next to the rows of the other clients in
//! [`super::negative_flow`] and [`super::compliance_settings`].
//!
//! One row per line of the issues' settings tables. A value is read back from
//! where it is in force: the numbers from the module builders
//! ([`super::fnb_seed`]) called the way the host calls them, the document
//! kinds, notification types, rules and connections from the registries the
//! modules read, the approval policies from the kernel's registered defaults.
//! The last column says where a value is set; the two answers still open with
//! the lawyer (`L-20`, `L-33`) and the other open items are flagged.

use wl_delivery::PerishableAction;
use wl_kernel::approvals::policy::defaults as approval_defaults;

use super::fnb_seed::{self, FNB, ORDER_SOURCE};
use super::negative_flow::{Row as MoneyRow, aed, delivery_for, invoicing_for, payments_for};
use super::roles::{ADMIN, AREA_MANAGER, KITCHEN_MANAGER};

/// The row's value is set by the host for this client.
const HOST: &str = "set by the host for this client (fnb_seed)";
/// The row's value is the module's own default.
const MODULE: &str = "default of the module";
/// The row's value comes from the roles of the host.
const ROLES: &str = "role permissions of the host";

fn money(number: &str, group: &str, setting: &str, value: String, source: &str) -> MoneyRow {
    MoneyRow {
        number: number.to_owned(),
        group: group.to_owned(),
        setting: setting.to_owned(),
        value,
        source: source.to_owned(),
    }
}

fn on(flag: bool) -> &'static str {
    if flag { "on" } else { "off" }
}

fn action(policy: PerishableAction) -> &'static str {
    match policy {
        PerishableAction::Return => "return to the outlet",
        PerishableAction::Discard => "discard, with a photo",
    }
}

/// Whether the kernel has a default approval policy for `key`.
fn has_policy(key: &str) -> &'static str {
    if approval_defaults().iter().any(|d| d.action() == key) {
        "registered"
    } else {
        "not registered"
    }
}

/// The 06 client's delivery, sales, payment, cash, cash-flow and budget
/// settings (BL-FNB-16), one row per line of the issue's table. `entities` are
/// the codes of the legal entities.
pub fn money_rows(entities: &[String]) -> Vec<MoneyRow> {
    let invoicing = invoicing_for(FNB, entities);
    let invoicing = invoicing.config();
    let payments = payments_for(FNB);
    let payments = payments.config();
    let delivery = delivery_for(FNB);
    let delivery = delivery.settings();
    let sales = super::property_seed::sales_for(FNB);
    let sales = sales.config();
    let policy = delivery.perishable_policy(ORDER_SOURCE);
    let absorbers: Vec<&str> = delivery
        .cod_absorbers()
        .iter()
        .map(|absorber| absorber.as_str())
        .collect();
    let thresholds = if entities.is_empty() {
        "none".to_owned()
    } else {
        entities
            .iter()
            .map(|code| format!("{code} {}", aed(invoicing.cash_threshold_of(code))))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let promise = delivery
        .promise_minutes(ORDER_SOURCE)
        .map_or_else(|| "none".to_owned(), |m| format!("{m} minutes"));
    vec![
        money(
            "1",
            "Delivery",
            "Perishable policy (WRK.RUL.16), F&B orders",
            format!(
                "after a failed visit: {}; after a run abort: {}; never queued for reattempt (the reattempt rule is {})",
                action(policy.on_failed_visit),
                action(policy.on_abort),
                on(delivery.attempt_rule())
            ),
            HOST,
        ),
        money(
            "2",
            "Delivery",
            "Perishable hand-over window",
            format!("the brand's delivery promise, {promise} until a brand sets its own"),
            HOST,
        ),
        money(
            "3",
            "Delivery",
            "Partial acceptance at proof of delivery",
            format!(
                "{}: the customer may refuse a line; a prepaid order refunds it by a no goods return (wrong item, business) up to {}, above it a kitchen manager files the refund; cash on delivery collects the accepted lines only",
                on(delivery.partial_acceptance(ORDER_SOURCE)),
                aed(fnb_seed::APPROVAL_LIMIT)
            ),
            HOST,
        ),
        money(
            "4",
            "Delivery",
            "COD difference tolerance and approver (WRK.ACL.09)",
            format!(
                "{} accepted without approval; a larger difference waits {} minutes for the {KITCHEN_MANAGER} (deliveries.approve_cod_difference), who picks who absorbs it: {}; approval {}",
                aed(delivery.cod_tolerance()),
                delivery.cod_approval_minutes(),
                absorbers.join(", "),
                on(delivery.cod_approval())
            ),
            HOST,
        ),
        money(
            "5",
            "Delivery",
            "Delivery reversal and manual lost (WRK.ACL.10)",
            format!(
                "approved by the {KITCHEN_MANAGER} or the company admin, never the requester; the rider holds neither deliveries.reverse nor the approval (policies {} / {})",
                has_policy(wl_delivery::reverse::ACTION),
                has_policy(wl_delivery::lost::ACTION)
            ),
            "approval policies of the host (FnbSeed)",
        ),
        money(
            "6",
            "Delivery",
            "Delivery lost after (WRK.RUL.17)",
            format!("{} days untraced", delivery.untraced_days()),
            HOST,
        ),
        money(
            "7",
            "Delivery",
            "Midday-break warning on rider runs (WRK.RUL.15)",
            "not offered by wl-delivery: see DEFERRED-fnb-settings".to_owned(),
            "module",
        ),
        money(
            "8",
            "Delivery",
            "Delivery failure reasons",
            format!(
                "the module's list plus {}; refused asks the cause (late, wrong item, other)",
                fnb::failed::FAILURE_REASONS
                    .iter()
                    .map(|(code, ..)| *code)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            "seeded by `fnb:install`",
        ),
        money(
            "9",
            "Sales",
            "Refund after completion with nothing returned (SAL.ACL.06)",
            format!(
                "above {} a manager approves, never the person who recorded it; categories as wl-sales: {}",
                aed(sales.refund_approval_limit),
                sales.complaint_categories.join(", ")
            ),
            HOST,
        ),
        money(
            "10",
            "Sales",
            "Cancel or refund of a paid order (SAL.ACL.01)",
            format!(
                "above {}, or after preparation started, needs a manager ({})",
                aed(sales.cancel_approval_limit),
                sales.order_manager_role
            ),
            HOST,
        ),
        money(
            "11",
            "Sales",
            "Cancel after the kitchen started (SAL.ACL.05)",
            format!(
                "outcomes offered: {}; remake = a new order, outcome scrap; fee {} (VAT {})",
                sales.cancel_outcomes.join(", "),
                if sales.cancellation_fee_bps == 0 && sales.cancellation_fee_amount == 0 {
                    "0".to_owned()
                } else {
                    format!(
                        "{}% + {}",
                        sales.cancellation_fee_bps / 100,
                        aed(sales.cancellation_fee_amount)
                    )
                },
                sales.cancellation_fee_vat
            ),
            HOST,
        ),
        money(
            "12",
            "Sales",
            "Order amendments on aggregator channels",
            format!(
                "{} (changed on the aggregator); channels: {}",
                if sales.aggregator_channels.iter().any(|c| sales.amendments_allowed(c)) {
                    "on"
                } else {
                    "off"
                },
                sales.aggregator_channels.join(", ")
            ),
            HOST,
        ),
        money(
            "13",
            "Sales",
            "Unique external order number per channel",
            on(sales.unique_external_order_number).to_owned(),
            HOST,
        ),
        money(
            "14",
            "Sales",
            "Online payment window before payment_failed",
            format!("{} minutes", sales.online_payment_window_minutes),
            HOST,
        ),
        money(
            "15",
            "Sales",
            "Till variance needing approval",
            format!(
                "{}, the same as the write-off limit: the shortage of a closed till session goes to the till's cash handover (see the cash rows); the variance approval of the till session itself is not built (DEFERRED-fnb-settings)",
                aed(fnb_seed::SHORTAGE_WRITE_OFF_LIMIT)
            ),
            HOST,
        ),
        money(
            "16",
            "Payments",
            "Pending online payment check (FIN.RUL.19)",
            format!("{} minutes", payments.pending_timeout_minutes),
            HOST,
        ),
        money(
            "17",
            "Payments",
            "Late payment after reject or cancel (SAL.RUL.06)",
            format!(
                "refunded in full without staff action: {}; the customer and the kitchen manager are told",
                on(payments.late_refund_auto)
            ),
            HOST,
        ),
        money(
            "18",
            "Payments",
            "Chargeback acceptance (FIN.ACL.18)",
            format!(
                "a second person, always (AED 0); the accountant asks, another holder of payments.chargeback approves (policy {})",
                has_policy(wl_payments::chargeback_close::ACCEPT_ACTION)
            ),
            MODULE,
        ),
        money(
            "19",
            "Payments",
            "Refund retry (FIN.RUL.21)",
            format!(
                "{} retry after {} hours, then another method or customer credit",
                payments.refund_max_retries, payments.refund_retry_hours
            ),
            HOST,
        ),
        money(
            "20",
            "Cash",
            "Cash holders",
            "own riders per person for each run (the module's period is the run); tills in till session mode: the closed session creates the till's handover".to_owned(),
            "wl-delivery, wl-sales",
        ),
        money(
            "21",
            "Cash",
            "Count of rider and till cash (FIN.ACL.05)",
            format!("the {KITCHEN_MANAGER} or the cashier (cash.count), never the holder"),
            ROLES,
        ),
        money(
            "22",
            "Cash",
            "Shortage write-off without approval (FIN.ACL.06)",
            format!(
                "{} per handover; above it the {AREA_MANAGER} or the company admin ({ADMIN}) approves, never the requester (policy {}); the cash is managed by the {}",
                aed(payments.cash_write_off_limit),
                has_policy(wl_payments::cash_settle::ACTION),
                payments.cash_manager_role
            ),
            HOST,
        ),
        money(
            "23",
            "Cash",
            "Shortage recovery by salary deduction (FIN.ACL.19)",
            format!(
                "{}: not offered while the lawyer has not answered",
                on(payments.cash_salary_deduction)
            ),
            "open item L-20: setting WlPayments::cash_salary_deduction, off",
        ),
        money(
            "24",
            "Cash-flow",
            "Forecast and alert (FIN.CFL.01, FIN.RUL.23)",
            format!(
                "{} weekly buckets from {}; alert below {} per entity ({}); checked daily at {}; told: {}; due dates moved by days late: {}; disputed amounts counted: {}; aggregator payouts by their payout schedule: not built (DEFERRED-fnb-settings)",
                invoicing.cash_forecast_weeks,
                invoicing.cash_week_start,
                aed(fnb_seed::CASH_THRESHOLD),
                thresholds,
                invoicing.cash_alert_time,
                invoicing.cash_alert_roles.join(", "),
                on(invoicing.cash_days_late),
                on(invoicing.cash_disputed_counted)
            ),
            HOST,
        ),
        money(
            "25",
            "Budget",
            "Categories and approval (FIN.BUD.01, FIN.RUL.24)",
            format!(
                "categories owned by the {} (new categories too), mapped to the export accounts; approved by the operations director (budgets.approve); FIN.RUL.24 warns the category's owner role and the {} at {}% of a month's line; categories besides the module's: {}",
                invoicing.budget_category_owner.clone().unwrap_or_default(),
                invoicing.budget_variance_role,
                invoicing.budget_variance_percent,
                fnb_seed::BUDGET_CATEGORIES
                    .iter()
                    .map(|(_, name, ..)| *name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            HOST,
        ),
        money(
            "26",
            "Budget",
            "Owner drawings (FIN.BUD.01, FIN.ACL.21)",
            format!(
                "expense category and payment purpose owner drawings, kept out of food cost, operating cost and profit; every drawing approved whatever its amount by the operations director (drawings.approve), never by the requester (self-approval {}); {} is told when a drawing has no second approver",
                on(invoicing.drawings_self_approve),
                invoicing.drawings_notify_role
            ),
            HOST,
        ),
    ]
}
