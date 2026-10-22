//! The 06 F&B client's values for the shared sales, delivery and access
//! mechanisms (06 §3.3, §4.2, §7.1, §7.2; BL-FNB-01 to BL-FNB-03 and the
//! module issues #112, #126, #163, #166, #238 that name them).
//!
//! The mechanisms live in the shared modules; the vertical reaches them only
//! through `wl-kernel`. This file turns the 06 values into:
//!
//! - the sales module's settings ([`sales`]): the outcomes of a cancelled
//!   started line are *return to stock* and *scrap*, the limits of the
//!   cancellation and the refund (`SAL.ACL.01`, `SAL.ACL.05`, `SAL.ACL.06`,
//!   AED 50), the direct site waits for the outlet to accept
//!   (`pending_acceptance`), the aggregators' external numbers are unique;
//! - the delivery module's settings ([`delivery`]): the perishable policy for
//!   own-rider deliveries is *return* after a failed visit and *discard* after
//!   an abort (`WRK.RUL.16`), a wastage above AED 200 needs a second person
//!   (`STK.ACL.02`), the delivery promise is 45 minutes (BL-DLV-10), the
//!   customer may refuse one line at the door (partial acceptance on), a cash
//!   difference is absorbed by the
//!   customer, the business or the rider (`WRK.ACL.09`), and the kitchen
//!   manager gets the dispatcher's tasks;
//! - the invoicing and payments settings ([`invoicing`], [`payments`]): the
//!   cash-flow alert, the budget and the owner drawings (`FIN.RUL.23`,
//!   `FIN.RUL.24`, `FIN.ACL.21`), the online payment check, late payments,
//!   the refund retry, the cash write-off limit and the salary deduction that
//!   stays off (`FIN.RUL.19`, `SAL.RUL.06`, `FIN.RUL.21`, `FIN.ACL.06`,
//!   `FIN.ACL.19`, BL-FNB-16);
//! - the failure reasons 06 adds to the rider's list ([`install`]): *rider
//!   issue*, *refused: the order was late* and *refused: wrong item*;
//! - the roles' extra grants of this client ([`grant_roles`]): the cashier,
//!   the storekeeper and the accountant of the shared roles table get their
//!   06 permissions here only;
//! - the seed ([`FnbSeed`], command `fnb:install`), which adds what the
//!   database lacks and keeps what the client changed.
//!
//! Nothing here is a rule: the rules run in the modules with these values.

use fnb::permissions as fnb_perm;
use renox::chrono::Weekday;
use renox::prelude::*;
use wl_delivery::reasons::FailureReason;
use wl_delivery::{PerishableAction, WlDelivery};
use wl_inventory::{ScoreSettings, WlInventory};
use wl_invoicing::budget_category::{self, CategoryInput};
use wl_invoicing::{BudgetCategory, BudgetKind, MappingKind, WlInvoicing};
use wl_kernel::approvals::PolicyDefault;
use wl_kernel::delivery::CodAbsorber;
use wl_payments::WlPayments;
use wl_sales::WlSales;

use super::roles::{
    self, ACCOUNTANT, ADMIN, AREA_MANAGER, CASHIER, KITCHEN_MANAGER, OPERATIONS_DIRECTOR,
    STOREKEEPER,
};

/// The client app these values are for.
pub const FNB: &str = "fnb";

/// The source type of an F&B order: the key of the perishable policy and of the
/// delivery promise.
pub const ORDER_SOURCE: &str = fnb::SUBJECT_TYPE;

/// The delivery promise of every brand until a brand sets its own, in minutes.
pub const PROMISE_MINUTES: i64 = 45;

/// A wastage above this value needs a second person (`STK.ACL.02`, AED 200).
pub const WASTAGE_APPROVAL_FILS: i64 = 20_000;

/// A cash difference up to this is accepted without an approver (AED 5,
/// `WRK.ACL.09`).
pub const COD_TOLERANCE: i64 = 500;

/// A cancellation or a refund above this needs a manager (`SAL.ACL.01`,
/// `SAL.ACL.06`, AED 50).
pub const APPROVAL_LIMIT: i64 = 5_000;

/// The days after which a delivery nobody traced is marked lost (`WRK.RUL.17`).
pub const LOST_AFTER_DAYS: i64 = 7;

/// The minutes after which an online payment still `pending` is asked of the
/// gateway (`FIN.RUL.19`; the module's default is 30, lowered for food).
pub const PENDING_PAYMENT_MINUTES: i64 = 10;

/// The minutes an online-paid order waits for the gateway before it is
/// cancelled as *payment failed* (the wl-sales default, kept on purpose).
pub const ONLINE_PAYMENT_WINDOW_MINUTES: i64 = 15;

/// The hours before a refund that failed at the gateway is retried, once
/// (`FIN.RUL.21`).
pub const REFUND_RETRY_HOURS: i64 = 24;

/// A shortage of a rider's handover or a till session up to this is written
/// off without an approver (`FIN.ACL.06`, AED 20); above it the area manager
/// approves. The till variance that needs approval is the same figure.
pub const SHORTAGE_WRITE_OFF_LIMIT: i64 = 2_000;

/// Recovering a shortage by a salary deduction (`FIN.ACL.19`): off until the
/// lawyer answers `L-20`. A setting, never decided in code.
pub const SALARY_DEDUCTION: bool = false;

/// The projected balance below which `FIN.RUL.23` alerts, in fils (AED
/// 50,000), for every legal entity.
pub const CASH_THRESHOLD: i64 = 5_000_000;

/// The weeks the cash-flow forecast looks ahead.
pub const CASH_FORECAST_WEEKS: u32 = 13;

/// When the cash-flow alert is checked, Asia/Dubai.
pub const CASH_ALERT_TIME: &str = "06:00";

/// The share of a month's budget line at which `FIN.RUL.24` warns, in percent.
pub const BUDGET_WARN_PERCENT: u32 = 100;

/// The channels the aggregators' tablets and reports enter orders through
/// (never their real names on a screen, DEMO-DATA.md §06).
pub const AGGREGATOR_CHANNELS: [&str; 3] = ["aggregator_a", "aggregator_b", "aggregator_c"];

/// The F&B module with the 06 values (the defaults of the spec; open item L-33
/// keeps a customer-side failure unrefunded until the lawyer answers).
pub fn module() -> fnb::Fnb {
    // A completed order consumes its recipe from the outlet's stock, so a
    // recall of an ingredient lot finds the orders that may have used it.
    // The kitchen devices of each outlet are a device connection active in its
    // opening hours and `down` after 5 minutes (`FNB.RUL.15`, BL-FNB-14); the
    // outlets named here also get their own paper-order late-entry type.
    fnb::Fnb::new()
        .recipe_consumption(true)
        // A refused line up to AED 50 is refunded at once, above it a kitchen
        // manager files the refund (`SAL.ACL.06`); a wastage above AED 200
        // needs the kitchen manager (`STK.ACL.02`).
        .refused_refund_limit(APPROVAL_LIMIT)
        .wastage_approval(WASTAGE_APPROVAL_FILS)
        .kitchen_devices(
            "marina",
            MARINA_OPENS,
            MARINA_CLOSES,
            &["kds-marina-1", "kds-marina-2", "till-marina"],
        )
        .kitchen_devices(
            "sharjah",
            "10:00",
            "23:00",
            &["kds-sharjah-1", "till-sharjah"],
        )
        .outage_down_minutes(OUTAGE_DOWN_MINUTES)
        .outage_prompt_minutes(OUTAGE_PROMPT_MINUTES)
}

/// Marina Kitchen opens at 11:00 and closes at 02:00 the next night.
pub const MARINA_OPENS: &str = "11:00";

/// See [`MARINA_OPENS`].
pub const MARINA_CLOSES: &str = "02:00";

/// N: minutes of silence before an outlet's kitchen devices are `down`
/// (`FNB.RUL.15`).
pub const OUTAGE_DOWN_MINUTES: i64 = 5;

/// R: minutes between the prompts to resume the channels (`FNB.RUL.15`).
pub const OUTAGE_PROMPT_MINUTES: i64 = 15;

/// The sales module with the 06 client's settings (06 §7.1, §3.3).
pub fn sales(module: WlSales) -> WlSales {
    module
        .over_budget_approval(true)
        .campaign_required_for(&[])
        .cancel_outcomes(&["return_to_stock", "scrap"])
        .cancel_approval_limit(APPROVAL_LIMIT)
        .refund_approval_limit(APPROVAL_LIMIT)
        .cancellation_fee_vat("standard")
        .order_acceptance_channels(&["website"])
        .aggregator_channels(&AGGREGATOR_CHANNELS)
        .unique_external_order_number(true)
        .order_manager_role(KITCHEN_MANAGER)
        // The online payment window stays the module's 15 minutes; the
        // gateway is asked at 10 (`FIN.RUL.19`, see [`payments`]). Written
        // here so that a change of the module default never moves 06. A
        // cancellation after the kitchen started has no fee.
        .online_payment_window_minutes(ONLINE_PAYMENT_WINDOW_MINUTES)
        .cancellation_fee_percent(0)
        .cancellation_fee_amount(0)
        // `SAL.CRM.02`, `SAL.ACL.07`: no lead source requires a campaign (set
        // above) and a cost entry that takes a campaign over its budget needs
        // a manager (`over_budget_approval`). `SAL.RUL.08`, `SAL.ACL.07`: a
        // member is reminded 30 days before points expire, a manual
        // adjustment above 500 points needs a manager, a redemption is
        // confirmed by a one-time code (`SAL.LOY.01`).
        .expiry_reminder_days(super::fnb_compliance::LOYALTY_REMINDER_DAYS)
        .adjustment_limit(super::fnb_compliance::LOYALTY_ADJUSTMENT_LIMIT)
        .confirm_redemption_by_code(true)
}

/// The delivery module with the 06 client's settings (06 §4.2, §7.1;
/// `WRK.RUL.16`, `WRK.ACL.09`, BL-DLV-10).
pub fn delivery(module: WlDelivery) -> WlDelivery {
    module
        .perishable_policy(
            ORDER_SOURCE,
            PerishableAction::Return,
            PerishableAction::Discard,
        )
        .discard_approval_limit(WASTAGE_APPROVAL_FILS)
        .promise_minutes(ORDER_SOURCE, PROMISE_MINUTES)
        // The customer may refuse one line at the door (wrong item); the
        // refused line is refunded by a *no goods* return and, on cash on
        // delivery, only the accepted lines are collected (06 §4.2, §7.2).
        .partial_acceptance(ORDER_SOURCE, true)
        .cod_tolerance(COD_TOLERANCE)
        .cod_approval(true)
        // A delivery nobody traced is marked lost after 7 days (`WRK.RUL.17`);
        // a failed cooked-food delivery is never queued for reattempt (the
        // module's reattempt rule stays off and the perishable policy above
        // returns or discards it).
        .untraced_days(LOST_AFTER_DAYS)
        .attempt_rule(false)
        .cod_absorbers(&[
            CodAbsorber::Customer,
            CodAbsorber::Business,
            CodAbsorber::Driver,
        ])
        .dispatcher_role(KITCHEN_MANAGER)
}

/// The invoicing module with the 06 client's cash-flow, budget and drawings
/// values (`FIN.CFL.01`, `FIN.RUL.23`, `FIN.BUD.01`, `FIN.RUL.24`,
/// `FIN.ACL.21`, 06 §5, §7.2): the forecast has 13 weekly buckets from
/// Monday; `FIN.RUL.23` alerts the operations director and the accountant at
/// 06:00 when the projected balance of a legal entity falls below AED 50,000
/// in any week; due dates are not moved by the average days late (customers
/// pay at order) and disputed amounts are not counted; the categories of the
/// budget belong to the accountant and the owner (the operations director)
/// approves the budget; `FIN.RUL.24` warns the category's owner role and the
/// accountant at 100% of a month's line; no owner drawing is approved by the
/// person who recorded it, whatever its amount. `entities` are the codes of
/// the legal entities.
pub fn invoicing(module: WlInvoicing, entities: &[String]) -> WlInvoicing {
    let mut module = module
        .cash_week_start(Weekday::Mon)
        .cash_forecast_weeks(CASH_FORECAST_WEEKS)
        .cash_days_late(false)
        .cash_disputed_counted(false)
        .cash_alert_time(CASH_ALERT_TIME)
        .cash_alert_roles(&[OPERATIONS_DIRECTOR, ACCOUNTANT])
        .budget_category_owner(ACCOUNTANT)
        .budget_variance_percent(BUDGET_WARN_PERCENT)
        .budget_variance_role(ACCOUNTANT)
        .drawings_self_approve(false)
        .drawings_notify_role(OPERATIONS_DIRECTOR);
    for code in entities {
        module = module.cash_threshold(code, CASH_THRESHOLD);
    }
    module
}

/// The payments module with the 06 client's values (06 §3.3, §4.2, §7.2): an
/// online payment still pending after 10 minutes is asked of the gateway
/// (`FIN.RUL.19`); a payment confirmed after the order was rejected or
/// cancelled is refunded in full without staff action (`SAL.RUL.06`); a
/// refund that failed at the gateway is retried once after 24 hours
/// (`FIN.RUL.21`); a shortage of a rider's handover or a till session up to
/// AED 20 is written off without an approver and the kitchen manager is the
/// manager of the cash (`FIN.ACL.06`); recovering a shortage by a salary
/// deduction stays off while the lawyer has not answered (`FIN.ACL.19`, open
/// item L-20); a bounced cheque adds no fee. Accepting a chargeback needs a
/// second person always (`FIN.ACL.18`: the module's default policy, step
/// `payments.chargeback`, separation of duties on).
pub fn payments(module: WlPayments) -> WlPayments {
    module
        .pending_timeout(PENDING_PAYMENT_MINUTES)
        .late_refunds_auto(true)
        .refund_retry(REFUND_RETRY_HOURS, 1)
        .cash_write_off_limit(SHORTAGE_WRITE_OFF_LIMIT)
        .cash_manager(KITCHEN_MANAGER)
        .cash_salary_deduction(SALARY_DEDUCTION)
}

/// The approval policies 06 changes, registered by [`FnbSeed`] after the
/// shared modules' own (the later registration of an action replaces the
/// earlier): a delivery reversed after it was marked delivered, and a delivery
/// marked lost by hand, are approved by the kitchen manager (or the company
/// admin), never by the requester and never by the rider (`WRK.ACL.10`); a
/// cash shortage above the write-off limit is approved by the area manager (or
/// the company admin), never by the requester (`FIN.ACL.06`).
pub fn register_approvals() {
    PolicyDefault::new(wl_delivery::reverse::ACTION)
        .step([KITCHEN_MANAGER, ADMIN])
        .expiry_hours(Some(wl_delivery::reverse::EXPIRY_HOURS))
        .register();
    PolicyDefault::new(wl_delivery::lost::ACTION)
        .step([KITCHEN_MANAGER, ADMIN])
        .expiry_hours(Some(wl_delivery::lost::EXPIRY_HOURS))
        .register();
    PolicyDefault::new(wl_payments::cash_settle::ACTION)
        .amount_above(SHORTAGE_WRITE_OFF_LIMIT)
        .step([AREA_MANAGER, ADMIN])
        .register();
}

/// The hours a pending issue or a negative balance may stay open before the
/// storekeeper gets a task (`STK.RUL.06`, the module's default, set on purpose).
pub const PENDING_ALERT_HOURS: i64 = 24;

/// The days a return to a supplier may stay uncredited before it is chased
/// (`STK.RUL.08`).
pub const RETURN_CHASE_DAYS: i64 = 30;

/// A purchase order above this needs the area manager (`STK.ACL.03`, AED
/// 5,000), in fils.
pub const PURCHASE_ORDER_LIMIT_FILS: i64 = 500_000;

/// The calendar months a supplier score snapshot covers (06: 1; the module's
/// default is 3).
pub const SUPPLIER_SCORE_WINDOW_MONTHS: u32 = 1;

/// H, the hours before an undecided line of a temperature hold is reminded to
/// the chef, and half the hours before the kitchen manager is told
/// (`STK.RUL.07`, lowered from 24 for food).
pub const TEMPERATURE_HOLD_REMINDER_HOURS: i64 = 2;

/// The stock module with the 06 client's values: a wastage above AED 200 needs
/// a second person (`STK.ACL.02`), a temperature hold is
/// reminded after 2 hours and escalated to the kitchen manager after 4
/// (`STK.RUL.07`), only a recall hold needs a second holder of `lots.release`
/// to release (`STK.ACL.05`), a goods receipt rejection for `temperature`
/// needs the reading (BL-FNB-12), and the customers of a recall are told on
/// the F&B page of the recall (BL-FNB-13).
pub fn inventory(module: WlInventory) -> WlInventory {
    module
        .wastage_approval_fils(WASTAGE_APPROVAL_FILS)
        .hold_reminder_hours_for_reason("temperature", TEMPERATURE_HOLD_REMINDER_HOURS)
        .hold_escalation_role_for_reason("temperature", KITCHEN_MANAGER)
        .hold_second_person_reasons(&["recall"])
        .rejection_reason("temperature", true)
        // The customers of a recall are listed and told on the F&B page of
        // the recall (BL-FNB-13).
        .recall_notices_route("fnb.recall_trace")
        // `STK.RUL.06`: a pending issue or a negative balance open for 24
        // hours gives the storekeeper a task and tells the manager.
        .pending_alert_hours(PENDING_ALERT_HOURS)
        // `STK.RUL.08`: a return to a supplier not credited is chased after
        // 30 days.
        .return_chase_days(RETURN_CHASE_DAYS)
        // `STK.ACL.03`: a purchase order above AED 5,000 needs the area
        // manager.
        .order_limits_fils(&[PURCHASE_ORDER_LIMIT_FILS])
        // `STK.SUP.01`, `STK.RUL.09`, `STK.ACL.07`: the monthly snapshot of
        // the supplier score is over a 1-month window (the module's default is
        // 3), below 70 a review task opens; making another supplier the
        // preferred one needs a second person.
        .supplier_score(ScoreSettings {
            window_months: SUPPLIER_SCORE_WINDOW_MONTHS,
            ..ScoreSettings::default()
        })
        .preferred_change_needs_approval(true)
}

/// The stock module for a client: the 06 values for F&B, the given module for
/// any other.
pub fn inventory_for(client_app: &str, module: WlInventory) -> WlInventory {
    if client_app == FNB {
        inventory(module)
    } else {
        module
    }
}

/// The delivery module for a client: the 06 values for F&B, the module's own
/// for any other.
pub fn delivery_for(client_app: &str, module: WlDelivery) -> WlDelivery {
    if client_app == FNB {
        delivery(module)
    } else {
        module
    }
}

/// What the extra grants of the F&B client are: the permissions of the roles
/// the shared table gives other duties in other verticals.
pub fn grant_roles(client_app: &str) {
    if client_app != FNB {
        return;
    }
    use wl_delivery::permissions as delivery;
    use wl_payments::permissions as payments;
    use wl_sales::permissions as sales;
    // The cashier enters aggregator and phone orders, rings up counter sales
    // and asks for a refund the kitchen manager approves (`FNB.ACL.17`); the
    // cashier of 03 counts cash, the cashier of 06 keeps the till.
    roles::grant_extra(
        CASHIER,
        &[
            fnb_perm::ORDERS_VIEW,
            fnb_perm::ORDERS_REFUND_REQUEST,
            // Cancels an order, or a line, and amends a direct order after
            // calling the customer (`SAL.ACL.01`, `SAL.ACL.05`: a manager
            // approves above AED 50 or for a started line).
            fnb_perm::ORDERS_CANCEL,
            fnb_perm::ORDERS_AMEND,
            // Enters an order a payout statement names and the kitchen never
            // recorded, as a late entry (BL-FNB-10).
            fnb_perm::LATE_ORDERS_ENTER,
            fnb_perm::RECONCILIATION_VIEW,
            // Enters the paper orders of an outage as late entries, sees the
            // channels and the closings, and keeps the till: opens it with a
            // float and counts it (BL-FNB-14, BL-FNB-15).
            fnb_perm::PAPER_ORDERS_ENTER,
            fnb_perm::OUTLET_CHANNELS_VIEW,
            fnb_perm::CLOSING_VIEW,
            sales::POS_OPEN_TILL,
            sales::POS_CLOSE_TILL,
            sales::SALES_ORDERS_VIEW,
            sales::SALES_ORDERS_CREATE,
            sales::SALES_ORDERS_CANCEL,
            sales::SALES_ORDERS_AMEND,
            sales::RETURNS_REQUEST,
            sales::RETURNS_HANDLE,
            sales::POS_SELL,
            // Counts a rider's cash and another cashier's till, never their
            // own (`FIN.ACL.05`, BL-FNB-16).
            payments::CASH_VIEW,
            payments::CASH_COUNT,
        ],
    );
    // The accountant reconciles and reads the orders.
    roles::grant_extra(
        ACCOUNTANT,
        &[
            fnb_perm::ORDERS_VIEW,
            // Imports, matches and voids the aggregators' payout statements
            // and reads the reports.
            fnb_perm::RECONCILIATION_VIEW,
            fnb_perm::RECONCILIATION_MANAGE,
            fnb_perm::LATE_ORDERS_ENTER,
            fnb_perm::REPORTS_VIEW,
            // Reviews the daily closings and the till sessions.
            fnb_perm::CLOSING_VIEW,
            fnb_perm::CLOSING_APPROVE,
            sales::POS_REVIEW,
            sales::SALES_ORDERS_VIEW,
            payments::VIEW,
            delivery::VIEW,
        ],
    );
    // HR sees the rider fault entries on the riders' records (BL-FNB-08).
    roles::grant_extra(roles::HR, &[fnb_perm::RIDER_FAULTS_VIEW]);
    // The storekeeper sees the orders whose food they stock.
    roles::grant_extra(STOREKEEPER, &[fnb_perm::ORDERS_VIEW]);
}

/// Adds what the 06 client's modules lack, for the modules the manifest
/// enables, and keeps what the client changed: the failure reasons 06 adds to
/// wl-delivery's list, the aggregators' payout presets and the budget
/// categories. Returns how many rows it added or changed.
///
/// # Errors
///
/// The database's errors.
pub async fn install(state: &AppState) -> Result<usize> {
    let db = &state.db;
    let manifest = state.provided::<std::sync::Arc<wl_kernel::Manifest>>();
    let on = |name: &str| manifest.as_ref().is_some_and(|m| m.is_enabled(name));
    let mut added = 0;
    for (code, name_en, name_ar, counts, allows_reattempt) in fnb::failed::FAILURE_REASONS {
        if FailureReason::query()
            .where_(FailureReason::CODE.eq(code))
            .first(db)
            .await?
            .is_some()
        {
            continue;
        }
        let mut reason = FailureReason {
            code: code.to_owned(),
            name_en: name_en.to_owned(),
            name_ar: name_ar.to_owned(),
            counts,
            allows_reattempt,
            active: true,
            ..FailureReason::default()
        };
        reason.insert(db).await?;
        added += 1;
    }
    // The aggregators' payout column presets and refund reason mappings.
    added += fnb::seed::install_payouts(db).await?;
    if on("invoicing") {
        added += install_budget_categories(db).await?;
    }
    // The late-entry limits, checklists, SOPs, send window, loyalty
    // programme, segments and the stock locations of the outlets.
    added += super::fnb_compliance::install(state).await?.total();
    Ok(added)
}

/// The budget categories 06 keeps besides the module's (06 §5): *packaging*
/// and *aggregator commission*. Food cost is the module's *Cost of sales*,
/// staff its *Salaries and staff costs*, rent its *Rent and utilities*.
pub const BUDGET_CATEGORIES: [(&str, &str, &str, &str); 2] = [
    ("packaging", "Packaging", "التعبئة والتغليف", "packaging"),
    (
        "aggregator_commission",
        "Aggregator commission",
        "عمولة المنصات",
        "aggregator_commission",
    ),
];

/// The accountant owns every budget category (06 has no marketing manager: the
/// module's *Marketing* category belongs to the accountant too) and the two
/// categories of [`BUDGET_CATEGORIES`] exist, mapped to the cost code and the
/// expense category of the same key. A category the client changed is kept.
/// Returns how many rows it created or changed.
///
/// # Errors
///
/// The database's errors.
pub async fn install_budget_categories(db: &Db) -> Result<usize> {
    let mut changed = 0;
    for (code, name_en, name_ar, key) in BUDGET_CATEGORIES {
        if BudgetCategory::by_code(db, code).await?.is_some() {
            continue;
        }
        let input = CategoryInput {
            code: code.to_owned(),
            name_en: name_en.to_owned(),
            name_ar: Some(name_ar.to_owned()),
            kind: BudgetKind::Expense,
            owner_role: Some(ACCOUNTANT.to_owned()),
            mappings: vec![
                (MappingKind::CostCode, key.to_owned()),
                (MappingKind::ExpenseCategory, key.to_owned()),
            ],
            active: true,
            ..CategoryInput::default()
        };
        // A key another category already collects is the client's choice: the
        // category is then left out and the page of categories shows why.
        if budget_category::save(db, None, input).await.is_ok() {
            changed += 1;
        }
    }
    for mut category in BudgetCategory::listed(db, true).await? {
        let ownerless = category
            .owner_role
            .as_deref()
            .is_none_or(|role| role == roles::MARKETING_MANAGER);
        if ownerless && !category.is_drawings() {
            category.owner_role = Some(ACCOUNTANT.to_owned());
            category.save(db).await?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// The module that seeds the 06 client's values: the seeder runs after the
/// shared modules' own (it is mounted after them), and the command
/// `fnb:install` runs the same install.
pub struct FnbSeed;

impl Module for FnbSeed {
    fn name(&self) -> &'static str {
        "fnb_seed"
    }

    fn register(&self, app: &mut Registry) {
        // After the shared modules' own registrations (this module is mounted
        // after them), so that these replace the defaults.
        register_approvals();
        app.command(
            "fnb:install",
            "Add the failure reasons, payout presets and budget categories the F&B client adds, where the database lacks them",
            |_args, state: AppState| async move {
                let added = install(&state).await?;
                println!("{added} 06 defaults added.");
                Ok(())
            },
        );
        app.seeder(|state: AppState| async move { install(&state).await.map(|_| ()) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_failure_reasons_are_not_reattempted_and_the_rider_issue_is_not_counted() {
        for (code, _, _, counts, allows) in fnb::failed::FAILURE_REASONS {
            assert!(!allows, "{code} is never reattempted (cooked food)");
            assert_eq!(counts, code != "rider_issue", "{code}");
        }
    }
}
