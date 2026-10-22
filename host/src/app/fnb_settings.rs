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
use wl_kernel::integrations;
use wl_kernel::late_entries;

use super::compliance_settings::{Row, kind_text, type_text};
use super::fnb_compliance as comp;

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

// ---------------------------------------------------------------- compliance

/// The row's value is this client's seed.
const SEED: &str = "F&B seed (BL-FNB-17)";

fn row(
    number: &str,
    group: &str,
    setting: &str,
    value: String,
    source: &str,
    reference: &str,
) -> Row {
    Row {
        number: number.to_owned(),
        group: group.to_owned(),
        setting: setting.to_owned(),
        value,
        source: source.to_owned(),
        reference: reference.to_owned(),
        open: String::new(),
    }
}

fn open(mut row: Row, item: &str) -> Row {
    row.open = item.to_owned();
    row
}

fn days(list: &[i64]) -> String {
    list.iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join("/")
}

/// The connection `key` as registered: its owner role and fallback.
fn connection_text(key: &str) -> String {
    match integrations::get(key) {
        None => format!("{key}: not registered"),
        Some(def) => format!(
            "{key}: owner {}; screens {}; fallback: {}",
            def.owner_role_name(),
            def.screen_keys().join(", "),
            def.fallback_en()
        ),
    }
}

/// The late-entry limits of `subject_type` as the kernel reads them: the seed's
/// row, else the defaults of `PLT.ACL.10`.
fn late_entry_text(subject_type: &str) -> String {
    let registered = if late_entries::get(subject_type).is_some() {
        "registered"
    } else {
        "type not offered by its module yet"
    };
    format!("{subject_type} ({registered})")
}

/// The 06 client's stock, document, rule, connection, notice, sales, staff and
/// registration settings (BL-FNB-17), one row per line of the issue's table.
/// `owner_role` is the owner role of `client.toml`.
pub fn compliance_rows(owner_role: &str) -> Vec<Row> {
    let stock = fnb_seed::inventory(wl_inventory::WlInventory::new());
    let stock = stock.config();
    let sales = super::property_seed::sales_for(FNB);
    let sales = sales.config();
    let hr = comp::hr(wl_hr::WlHr::new());
    let hr = hr.config();
    let critical: Vec<String> = comp::CRITICAL_RULES
        .iter()
        .map(|rule| {
            let state = if wl_kernel::compliance::is_critical(rule) {
                "marked"
            } else {
                "not registered"
            };
            format!("{rule} ({state})")
        })
        .collect();
    let temperature = stock
        .rejection_reasons
        .iter()
        .find(|r| r.code == "temperature")
        .is_some_and(|r| r.evidence_required);
    let onboarding: Vec<String> = comp::ONBOARDING
        .iter()
        .map(|o| format!("{} ({} steps)", o.role, o.steps.len()))
        .collect();
    let sops: Vec<String> = comp::SOPS
        .iter()
        .map(|s| {
            format!(
                "{}: owner {}, readers {}",
                s.title,
                s.owner_role,
                s.readers.join(", ")
            )
        })
        .collect();
    let types: Vec<String> = [
        fnb::notices::CUSTOMER_NOTICE,
        fnb::notices::LOGIN_OTP,
        fnb::recall::NOTICE_TYPE,
    ]
    .iter()
    .map(|key| type_text(key))
    .collect();
    let kinds: Vec<String> = comp::KINDS
        .iter()
        .map(|k| format!("{}: {}", k.1, kind_text(k.0)))
        .collect();
    let late: Vec<String> = comp::LATE_ENTRY_TYPES
        .iter()
        .map(|t| late_entry_text(t))
        .collect();
    vec![
        row(
            "1",
            "Stock",
            "Negative stock policy",
            format!(
                "an outlet's kitchen allows automatic consumption below zero for items that are not lot-tracked (allow_consumption); the central kitchen ({}) and lot-tracked items block",
                comp::CENTRAL_OUTLET
            ),
            "stock locations: F&B seed (`fnb:install`)",
            "§7.2 settings",
        ),
        row(
            "2",
            "Stock",
            "Pending issue and negative balance alert (STK.RUL.06)",
            format!(
                "after {} hours the holders of stock.adjust at the outlet (the storekeeper) get a task and the manager is told",
                stock.pending_alert_hours
            ),
            "WlInventory::pending_alert_hours",
            "§4.4",
        ),
        row(
            "3",
            "Stock",
            "Goods receipt rejection evidence",
            format!(
                "reason temperature needs the reading from the receiving checklist: {}",
                on(temperature)
            ),
            "WlInventory::rejection_reason",
            "§7.2 settings",
        ),
        row(
            "4",
            "Stock",
            "Returns to supplier not credited (STK.RUL.08)",
            format!("chased after {} days", stock.return_chase_days),
            "WlInventory::return_chase_days",
            "§4.4",
        ),
        row(
            "5",
            "Stock",
            "Stock hold deciders (lots.release)",
            format!(
                "the head chef of the outlet ({HEAD}: the role is assigned in the scope of the outlet, and the stock location of the outlet carries that scope); the outlet's {KITCHEN_MANAGER} as deputy; a chef of another outlet cannot decide",
                HEAD = super::roles::HEAD_CHEF
            ),
            "role grants in the outlet's scope; locations: F&B seed",
            "§7.2 settings",
        ),
        row(
            "6",
            "Stock",
            "Hold reminder (STK.RUL.07)",
            format!(
                "H = {} hours for a temperature hold, then {} (escalation after twice H)",
                stock.reminder_hours("temperature", None),
                stock
                    .escalation_role("temperature", None)
                    .unwrap_or("nobody")
            ),
            "WlInventory::hold_reminder_hours_for_reason",
            "§7.2",
        ),
        row(
            "7",
            "Stock",
            "Hold reasons needing a second person (STK.ACL.05)",
            stock.hold_second_person_reasons.join(", ")
                + ": another holder of lots.release releases; a temperature line is released by the chef alone",
            "WlInventory::hold_second_person_reasons",
            "§3.3",
        ),
        row(
            "8",
            "Stock",
            "Wastage approval (STK.ACL.02)",
            format!(
                "above {} by the {KITCHEN_MANAGER}, never the person who recorded it",
                aed(stock.wastage_approval_fils)
            ),
            "WlInventory::wastage_approval_fils",
            "§3.3",
        ),
        row(
            "9",
            "Stock",
            "Purchase order approval (STK.ACL.03)",
            format!(
                "above {} the area manager approves",
                stock
                    .order_limits_fils
                    .first()
                    .map_or_else(|| "no limit".to_owned(), |l| aed(*l))
            ),
            "WlInventory::order_limits_fils",
            "§3.3",
        ),
        row(
            "10",
            "Stock",
            "Recall owner",
            format!(
                "the owner role ({owner_role}) with the head chef; the customers are told on the F&B page of the recall ({}); a notice that gives up gives the owner a task",
                stock.recall_notices_route.clone().unwrap_or_default()
            ),
            "WlInventory::recall_notices_route",
            "§7.2 settings",
        ),
        open(
            row(
                "11",
                "Compliance",
                "Compliance-critical rules (owner alert, PLT.RUL.04)",
                critical.join("; "),
                "marked by the host for this client; a missed or failed run alerts the owner",
                "§7.2",
            ),
            "FNB.RUL.03 and FNB.RUL.04 are marked; the scheduled runs themselves are not built (DEFERRED-fnb-settings)",
        ),
        row(
            "12",
            "Compliance",
            "Owner alert recipients",
            format!(
                "the owner role ({owner_role}), and the company admin ({})",
                super::roles::ADMIN
            ),
            "client.toml [roles] owner",
            "§7.2",
        ),
        open(
            row(
                "13",
                "Documents",
                "Health card and PIC certificate expiry (DOC.EXP.01, DOC.RUL.01, DOC.RUL.03)",
                format!(
                    "reminders {} days before; the owner is told on expiry; no transition is blocked in the MVP; the reminders go to the {KITCHEN_MANAGER} of the employee's outlet. {}",
                    days(&comp::CARD_REMINDERS),
                    kinds[2..].join(" | ")
                ),
                SEED,
                "§4.6",
            ),
            "AU-15 (the validity is the expiry printed on the card)",
        ),
        row(
            "14",
            "Documents",
            "Responsible role per document kind",
            format!(
                "trade licence: {OPERATIONS_DIRECTOR_ROLE}; food permit, health card, PIC certificate: {KITCHEN_MANAGER} of the outlet; Halal certificate: {}; Trademark registration and Other certification: {OPERATIONS_DIRECTOR_ROLE}",
                super::roles::STOREKEEPER
            ),
            SEED,
            "§7.2 settings",
        ),
        row(
            "15",
            "Documents",
            "Trade licence and food permit",
            format!("{} | {}", kinds[0], kinds[1]),
            SEED,
            "§7.2 settings",
        ),
        row(
            "16",
            "Notices",
            "Notification types and groups",
            types.join(" | ")
                + " | the order status message that gives up creates no task; a recall notice that gives up gives the recall owner a task",
            SEED,
            "§7.2; wl-notify §5.4",
        ),
        row(
            "17",
            "Kernel",
            "Records that support void",
            "payout statement; orders are cancelled, receipts reversed by a sales return"
                .to_owned(),
            "registered by the F&B app",
            "§7.2 settings",
        ),
        row(
            "18",
            "Late entry",
            "Late entry after an outage (PLT.ACL.10)",
            format!(
                "business time = when it happened; flagged with a reason after M = {} minutes; the {AREA} approves after N = {} hours or into a submitted daily closing; types: {}",
                comp::LATE_ENTRY_FLAG_MINUTES,
                comp::LATE_ENTRY_APPROVAL_HOURS,
                late.join(", "),
                AREA = super::roles::AREA_MANAGER
            ),
            "limits seeded by `fnb:install`, editable per entity",
            "§4.6, §7.2 settings",
        ),
        row(
            "19",
            "Integrations",
            "Device connections",
            format!(
                "kitchen devices per outlet, active in the opening hours, down after {} minutes: {}",
                fnb_seed::OUTAGE_DOWN_MINUTES,
                ["marina", "sharjah"]
                    .iter()
                    .map(|o| format!("fnb.devices.{o}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            HOST,
            "wl-kernel §7",
        ),
        row(
            "20",
            "Integrations",
            "Fallbacks (wl-kernel §5.14)",
            [
                comp::DELIVERECT,
                integrations::PAYMENT_GATEWAY,
                integrations::SMS,
            ]
            .iter()
            .map(|k| connection_text(k))
            .collect::<Vec<_>>()
            .join(" | "),
            SEED,
            "§7.2 settings",
        ),
        row(
            "21",
            "Integrations",
            "Business-day cut-off per outlet",
            "04:00 (a change applies from the next business day)".to_owned(),
            "Outlet cut-off (BL-FNB-15)",
            "§7.2 settings",
        ),
        row(
            "22",
            "Sales",
            "Lead sources used (SAL.CRM.02)",
            format!(
                "the nine of the module: walk-in, website, social, paid ads, aggregator, referral, event, partner (corporate catering), other; a direct customer takes the source of their first order; a source requires a campaign: {}",
                if sales.campaign_required_sources.is_empty() {
                    "none".to_owned()
                } else {
                    sales.campaign_required_sources.join(", ")
                }
            ),
            "sources: the module's list; none turned off",
            "§5; §7.2 settings",
        ),
        row(
            "23",
            "Sales",
            "Campaign channels (SAL.CRM.02, SAL.ACL.07)",
            format!(
                "Instagram, TikTok, search ads, aggregator promotions, SMS, email (the module's fixed list: TikTok and aggregator promotions are recorded as other online); a cost entry over the budget needs a manager: {}",
                on(sales.over_budget_approval)
            ),
            "approval: F&B seed",
            "§5; §7.2 settings",
        ),
        row(
            "24",
            "Stock",
            "Supplier scorecard (STK.SUP.01, STK.RUL.09, STK.ACL.07)",
            format!(
                "score 0-100, weights {:?}; a monthly snapshot below {} over a {}-month window opens a review task for the storekeeper / purchaser, the area manager told; making another supplier preferred needs a second person: {}",
                stock.score.weights,
                stock.score.threshold,
                stock.score.window_months,
                on(stock.preferred_change_needs_approval)
            ),
            "WlInventory::supplier_score",
            "§5; §7.2 settings",
        ),
        row(
            "25",
            "Sales",
            "Loyalty earn and redeem (SAL.LOY.01, SAL.RUL.07)",
            format!(
                "{} earns 1 point per AED 1 on direct orders (none on aggregator orders: no member is attached); {} points = {} discount, at most {}% of the order, confirmed by a one-time code: {}",
                comp::LOYALTY_NAME,
                comp::LOYALTY_MIN_REDEEM,
                aed(comp::LOYALTY_MIN_REDEEM * comp::LOYALTY_POINT_VALUE_FILS),
                comp::LOYALTY_MAX_SHARE_BPS / 100,
                on(sales.confirm_redemption_by_code)
            ),
            "programme: F&B seed",
            "§5; §7.2 settings",
        ),
        open(
            row(
                "26",
                "Sales",
                "Loyalty expiry, adjustments, segments and campaigns (SAL.RUL.08, SAL.ACL.07, NTF.RUL.02, NTF.ACL.03)",
                format!(
                    "points expire {} months after they are earned, the member reminded {} days before; a manual adjustment above {} points needs a manager; segments: regulars (3 or more orders in 30 days), lapsed (no order in 45 days); campaigns only to opted-in customers, sent {:02}:00-{:02}:00, at most {} marketing messages per customer in 7 days; a broadcast to more than {} recipients needs a second person",
                    comp::LOYALTY_VALIDITY_MONTHS,
                    sales.expiry_reminder_days,
                    sales.adjustment_limit,
                    comp::SEND_WINDOW_MINUTES.0 / 60,
                    comp::SEND_WINDOW_MINUTES.1 / 60,
                    comp::MARKETING_CAP,
                    comp::BROADCAST_APPROVAL_ABOVE
                ),
                "WlSales, WlNotify and the F&B seed",
                "§5; §7.2 settings",
            ),
            "A-15 (VAT on the discounted supply of a redemption)",
        ),
        row(
            "27",
            "Staff",
            "Onboarding checklists (HRM.ONB.01, HRM.RUL.16, HRM.RUL.17, HRM.ACL.10)",
            format!(
                "{}; started when the employee record is created in onboarding; tasks to the {} of the outlet, the head chef and the employee; the employee's own steps only by them; completion blocks nothing; HRM.RUL.17 reminds on the first run after the due date, then every {} days, and tells HR and the line manager after {} days",
                onboarding.join("; "),
                hr.manager_role,
                wl_hr::reminders::REMIND_EVERY,
                wl_hr::reminders::ESCALATE_AFTER
            ),
            "checklists: F&B seed; reminder days: default of the module",
            "§5; §7.2 settings",
        ),
        row(
            "28",
            "Staff",
            "SOP library (HRM.ONB.01, DOC.RUL.04, HRM.ACL.10)",
            format!(
                "{}; DOC.RUL.04 asks each reader to acknowledge within {} days, reminds every {} days and tells the reader's manager and the SOP owner role after {} days overdue; only the reader acknowledges; a new version asks again",
                sops.join("; "),
                wl_documents::sops::service::DEFAULT_ACK_DUE_DAYS,
                wl_documents::acknowledgements::run::REMIND_EVERY,
                wl_documents::acknowledgements::run::ESCALATE_AFTER
            ),
            "SOPs and readers: F&B seed; days: default of the module",
            "§5; §7.2 settings",
        ),
        row(
            "29",
            "Registrations",
            "Registration and certification kinds (DOC.EXP.01, DOC.RUL.01)",
            format!(
                "Halal certificate (item for central kitchen products, supplier party for supplier ingredients; responsible role {}), Trademark registration ({} days; the brand is the mark; subject the legal entity) and Other certification ({} days; legal entity), the last two with {OPERATIONS_DIRECTOR_ROLE}; no blocks",
                super::roles::STOREKEEPER,
                days(&wl_documents::registrations::REGISTRATIONS[0].reminders),
                days(&wl_documents::registrations::REGISTRATIONS[4].reminders)
            ),
            "default of the module, on for 06 (roles: F&B seed)",
            "§5; §7.2 settings",
        ),
    ]
}

/// The role told as the owner of the documents and the registrations.
const OPERATIONS_DIRECTOR_ROLE: &str = super::roles::OPERATIONS_DIRECTOR;
