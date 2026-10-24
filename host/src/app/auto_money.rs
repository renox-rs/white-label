//! The 08 Auto client's values for the shared money and stock mechanisms (08
//! §3.3, §4.1, §4.3 *When it goes wrong*, §5, §7.1 *Negative-flow settings*,
//! §7.2; BL-AUT-18), and the rows of the page `/settings/negative-flow` that
//! show them.
//!
//! The mechanisms live in `wl-invoicing`, `wl-payments`, `wl-inventory` and
//! `wl-sales`; the numbers are `auto::money_defaults` and this file turns them
//! into the builder calls of each module. Nothing here is a rule: the rules
//! (`FIN.RUL.13`-`15`, `FIN.RUL.18`, `FIN.RUL.23`, `FIN.RUL.24`, `STK.RUL.02`,
//! `STK.RUL.08`, `STK.RUL.09`) run in the modules with these values, and the
//! limits that live on a stored approval policy (`FIN.ACL.15`, `FIN.ACL.20`,
//! `FIN.ACL.21`) stay editable by the company administrator.
//!
//! - invoicing ([`invoicing`]): every write-off is an approval (the limit of
//!   AED 0) by the Owner / GM or the company admin, never the requester; the
//!   B2B buyer disputes in the portal, escalated after 14 days to the Owner /
//!   GM; the insurer follow-up after 30 days for the insurance coordinator; the
//!   cash-flow alert below AED 100,000, the budget and the owner drawings;
//! - payments ([`payments`]): a cheque is presented twice at most, a bounce
//!   reminds after 3 days and asks the accountant to decide after 14, no
//!   bounce fee, the cheques drivers and sales reps collect are handed over per
//!   person per day and counted by another person, a cash shortage is always
//!   an approval with salary recovery off;
//! - inventory ([`inventory`]): a stock write-off above AED 1,000 and a Return
//!   to supplier above AED 5,000 need a second person, a return is chased
//!   after 30 days, the supplier scorecard keeps its 3-month window.
//!
//! **Not enforced, kept for the page:** the modules have no setting for the
//! sales discount limit (`SAL.ACL.02`), the credit grace (`SAL.ACL.03`), the
//! cancellation after picking (`SAL.ACL.05`), the credit re-check at dispatch,
//! the bounced-cheque hold of the credit group (`SAL.RUL.05`) and the partial
//! acceptance at proof of delivery (the delivery module is not enabled for 08):
//! the rows say so (`$S/f8/DEFERRED.md`).

use auto::money_defaults as money;
use renox::chrono::Weekday;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_inventory::{ScoreSettings, WlInventory};
use wl_invoicing::{BudgetCategory, WlInvoicing};
use wl_payments::WlPayments;

use super::negative_flow::{Row, aed};

/// The invoicing module with the 08 client's money values: see the module
/// documentation. `entities` are the codes of the client's legal entities,
/// each given the cash-flow threshold of AED 100,000.
pub fn invoicing(module: WlInvoicing, entities: &[String]) -> WlInvoicing {
    let mut module = module
        // The invoice of a delivered job card is issued at once: the customer
        // collects the vehicle against it (08 §4.4 step 7, `FIN.INV.01`).
        .auto_issue_source(auto::job_card::JOB_CARD_SUBJECT_TYPE)
        // `FIN.ACL.15`: every write-off is approved.
        .write_off_approvers(&money::WRITE_OFF_APPROVERS)
        // The B2B buyer disputes an invoice in the portal; `FIN.RUL.15`
        // escalates after 14 days.
        .portal_disputes(money::PORTAL_DISPUTES)
        .dispute_escalation_days(money::DISPUTE_ESCALATION_DAYS)
        .dispute_escalation_role(money::DISPUTE_ESCALATION_ROLE)
        // `FIN.RUL.14`: the proposal after 6 months (the module's floor).
        .bad_debt_relief(true)
        // `FIN.RUL.13`: the insurer's invoice with no remittance is followed
        // up after 30 days; a bill its channel rejected is the coordinator's.
        .payer_follow_up_days(money::INSURER_FOLLOW_UP_DAYS)
        .payer_bill_task_role(money::PAYER_BILL_TASK_ROLE)
        // `FIN.BUD.01`: the accountant owns every category.
        .budget_category_owner(money::BUDGET_CATEGORY_OWNER)
        // `FIN.CFL.01`, `FIN.RUL.23`: 13 weeks from Monday, insurer
        // receivables by their due dates, due dates moved by each customer's
        // days late, disputed amounts not counted, checked daily at 06:00.
        .cash_week_start(Weekday::Mon)
        .cash_forecast_weeks(money::CASH_FORECAST_WEEKS)
        .cash_days_late(true)
        .cash_disputed_counted(false)
        .cash_alert_time(money::CASH_ALERT_TIME)
        .cash_alert_roles(&money::CASH_ALERT_ROLES)
        // `FIN.RUL.24`: warns at 100% of a month's line the category's owner
        // role and the accountant.
        .budget_variance_percent(money::BUDGET_WARNING_PERCENT)
        .budget_variance_role(money::BUDGET_CATEGORY_OWNER)
        // `FIN.ACL.21`: every drawing is approved by someone other than the
        // requester; the Owner / GM is told when none else can approve.
        .drawings_self_approve(false)
        .drawings_notify_role(money::DRAWINGS_APPROVER_ROLE);
    for code in entities {
        module = module.cash_threshold(code, money::CASH_THRESHOLD);
    }
    module
}

/// The payments module with the 08 client's values (`FIN.RUL.18`,
/// `FIN.CSH.01`, `FIN.ACL.06`, `FIN.ACL.19`, `FIN.ACL.20`): no bounce fee is
/// set, so none is invoiced until the client sets a charge type and an amount.
pub fn payments(module: WlPayments) -> WlPayments {
    module
        .cheque_presentations(money::CHEQUE_PRESENTATIONS)
        .cheque_chase(money::BOUNCE_REMIND_DAYS, money::BOUNCE_DECIDE_DAYS)
        .cheque_manager(money::CHEQUE_MANAGER_ROLE, true)
        // Drivers and sales reps hand their cheques over per person per day.
        .field_cheque_handover(money::FIELD_CHEQUE_HANDOVER)
        // A counter shortage is always an approval and is never recovered
        // from the salary (`⚠ open: lawyer L-20`).
        .cash_write_off_limit(money::CASH_WRITE_OFF_LIMIT)
        .cash_salary_deduction(money::SALARY_RECOVERY)
        .cash_manager(money::CASH_MANAGER_ROLE)
}

/// The inventory module with the 08 client's values: the recall page of the
/// app (`BL-AUT-16`), `STK.ACL.02`, `STK.ACL.06`, `STK.RUL.08`, `STK.SUP.01`,
/// `STK.ACL.07`.
pub fn inventory(module: WlInventory) -> WlInventory {
    super::auto_seed::inventory(module)
        .wastage_approval_fils(money::STOCK_ADJUSTMENT_APPROVAL_FILS)
        .return_approval_fils(money::RETURN_APPROVAL_FILS)
        .return_chase_days(money::RETURN_CHASE_DAYS)
        .preferred_change_needs_approval(money::PREFERRED_CHANGE_NEEDS_APPROVAL)
        .supplier_score(ScoreSettings {
            window_months: money::SCORECARD_WINDOW_MONTHS,
            min_receipts: money::SCORECARD_MIN_RECEIPTS,
            grace_days: money::SCORECARD_GRACE_DAYS,
            threshold: money::SCORECARD_THRESHOLD,
            ..ScoreSettings::default()
        })
}

/// Gives every budget category with no owner role the accountant
/// (`FIN.BUD.01`, `FIN.RUL.24`); the owner drawings category never has one. A
/// role an administrator set is kept; the mapping to the client's export
/// accounts is the accountant's. Returns how many categories changed.
///
/// # Errors
///
/// The database's errors.
pub async fn install_budget_categories(db: &Db) -> Result<usize> {
    let mut changed = 0;
    for mut category in BudgetCategory::listed(db, true).await? {
        if category.owner_role.is_none() && !category.is_drawings() {
            category.owner_role = Some(money::BUDGET_CATEGORY_OWNER.to_owned());
            category.save(db).await?;
            changed += 1;
        }
    }
    Ok(changed)
}

fn yes(flag: bool) -> &'static str {
    if flag { "on" } else { "off" }
}

/// The states `transition` of the machine of `M` leaves, in words.
fn void_from<M: StateMachine>() -> String {
    M::machine()
        .find("void")
        .map_or_else(|_| "none".to_owned(), |t| t.from_states().join(", "))
}

/// The rows of the read-only money settings page for the 08 client: what the
/// invoicing, payments and inventory modules have in force for it, read back
/// from the same builder calls (`entities` are the codes of the client's legal
/// entities).
pub fn rows(entities: &[String]) -> Vec<Row> {
    let invoicing = invoicing(WlInvoicing::new(), entities);
    let invoicing = invoicing.config();
    let payments = payments(WlPayments::new());
    let payments = payments.config();
    let inventory = inventory(WlInventory::new());
    let inventory = inventory.config();
    let host = "set by the host for this client";
    let module = "default of the module, set by the host";
    let seed_note = "the 08 values (auto::money_defaults)";
    let missing = "kept in auto::money_defaults; the module has no setting yet, nothing enforces it (listed as deferred)";
    let make = |number: &str, group: &str, setting: &str, value: String, source: &str| Row {
        number: number.to_owned(),
        group: group.to_owned(),
        setting: setting.to_owned(),
        value,
        source: source.to_owned(),
    };
    let voidable = |subject_type: &str| {
        if wl_kernel::voids::get(subject_type).is_some() {
            "allowed"
        } else {
            "never"
        }
    };
    let thresholds = if entities.is_empty() {
        "none".to_owned()
    } else {
        entities
            .iter()
            .map(|code| format!("{code} {}", aed(invoicing.cash_threshold_of(code))))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let bounce_fee = payments.bounce_fee.as_ref().map_or_else(
        || "none until the client sets a charge type and an amount".to_owned(),
        |fee| format!("{} {}", fee.charge_type, aed(fee.amount)),
    );
    vec![
        make(
            "1",
            "Void",
            "Void allowed (PLT.ACL.09)",
            format!(
                "job cards from {}: {}; insurance claims from {}: {}; draft sales orders (no void exists in wl-sales yet)",
                void_from::<auto::JobCard>(),
                voidable(auto::job_card::JOB_CARD_SUBJECT_TYPE),
                void_from::<auto::InsuranceClaim>(),
                voidable(auto::claim::INSURANCE_CLAIM_SUBJECT_TYPE),
            ),
            "the voidable records the app registers",
        ),
        make(
            "2",
            "Void",
            "Void never (SAL.ACL.04)",
            format!(
                "counter sales: {}; invoices: {}; anything sent to an insurer (a sales return, a credit note or a supplementary instead)",
                voidable(wl_kernel::sales::COUNTER_SALE_SUBJECT_TYPE),
                voidable(wl_kernel::invoicing::INVOICE_SUBJECT_TYPE),
            ),
            "the voidable records the modules register",
        ),
        make(
            "3",
            "Sales",
            "Sales order cancel after picking (SAL.ACL.05)",
            format!(
                "outcome {}, approval by the {}, fee {} by default",
                money::CANCEL_AFTER_PICKING_OUTCOME,
                money::CANCEL_AFTER_PICKING_APPROVER,
                aed(money::CANCEL_AFTER_PICKING_FEE)
            ),
            missing,
        ),
        make(
            "4",
            "Sales",
            "Booking cancel after check-in (SCH.ACL.06, SCH.ACL.05)",
            "staff only; no booking deposits in the MVP (a client that takes deposits gets SCH.ACL.05)"
                .to_owned(),
            "wl-scheduling, set by the host",
        ),
        make(
            "5",
            "Sales",
            "Discount limit (SAL.ACL.02)",
            format!(
                "above {}% needs the {}",
                money::DISCOUNT_LIMIT_PERCENT,
                money::DISCOUNT_APPROVER_ROLE
            ),
            missing,
        ),
        make(
            "6",
            "Sales",
            "Credit grace (SAL.ACL.03)",
            format!(
                "{} days past terms; per customer and credit group",
                money::CREDIT_GRACE_DAYS
            ),
            missing,
        ),
        make(
            "7",
            "Sales",
            "Credit re-check",
            format!(
                "at dispatch and at counter collection for orders on account: {}",
                yes(money::CREDIT_RECHECK_AT_DISPATCH)
            ),
            missing,
        ),
        make(
            "8",
            "Sales",
            "Bounced cheque hold (SAL.RUL.05)",
            format!(
                "the whole credit group: {}",
                yes(money::BOUNCED_CHEQUE_HOLDS_GROUP)
            ),
            missing,
        ),
        make(
            "9",
            "Delivery",
            "Partial acceptance at proof of delivery",
            format!(
                "on for parts deliveries: {} (the delivery module is not enabled for 08 yet)",
                yes(money::PARTIAL_ACCEPTANCE_AT_POD)
            ),
            missing,
        ),
        make(
            "10",
            "Invoices",
            "Invoice dispute by the B2B buyer in the portal; insurer short payments through the payer bill (FIN.TPP.01)",
            yes(invoicing.portal_disputes).to_owned(),
            host,
        ),
        make(
            "11",
            "Invoices",
            "Dispute escalation (FIN.RUL.15)",
            format!(
                "after {} days, to {}",
                invoicing.dispute_escalation_days, invoicing.dispute_escalation_role
            ),
            host,
        ),
        make(
            "12",
            "Invoices",
            "Write-off limit (FIN.ACL.15)",
            format!(
                "AED 0: every write-off is approved by {}, never the requester",
                invoicing.write_off_approvers.join(" or ")
            ),
            host,
        ),
        make(
            "13",
            "Invoices",
            "Bad-debt relief proposal (FIN.RUL.14)",
            format!(
                "{} after {} months",
                yes(invoicing.bad_debt_relief),
                invoicing.bad_debt_relief_months.get()
            ),
            module,
        ),
        make(
            "14",
            "Invoices",
            "Insurer follow-up (FIN.RUL.13)",
            format!(
                "{} days after the payer bill; a task for the {}",
                invoicing.payer_follow_up_days, invoicing.payer_bill_task_role
            ),
            host,
        ),
        make(
            "15",
            "Invoices",
            "Invoices drafted and issued at once",
            format!(
                "on for {}; reviewed for the others",
                invoicing.auto_issue_sources.join(", ")
            ),
            host,
        ),
        make(
            "16",
            "Invoices",
            "E-invoicing ASP (from 2027)",
            "invoices queue while the ASP is down and are sent on recovery (FIN.RUL.16); the go-live date of each legal entity is set per client".to_owned(),
            "default of the module",
        ),
        make(
            "17",
            "Cheques",
            "Bounced cheque (FIN.RUL.18)",
            format!(
                "bounce fee: {}; reminders after {} and {} days; at most {} presentations; the {} is told besides the accountant: {}",
                bounce_fee,
                payments.cheque_reminder_days,
                payments.cheque_decision_days,
                payments.cheque_max_presentations,
                payments.cheque_manager_role,
                yes(payments.cheque_notify_manager)
            ),
            host,
        ),
        make(
            "18",
            "Cash",
            "Cash custody (FIN.CSH.01, FIN.ACL.20)",
            format!(
                "counter till sessions: {}; cheques collected by drivers and sales reps handed over per person per day, cheques only, counted by another person: {}",
                yes(money::COUNTER_TILL_SESSIONS),
                yes(payments.field_cheque_handover)
            ),
            host,
        ),
        make(
            "19",
            "Cash",
            "Cash shortage write-off (FIN.ACL.06, FIN.ACL.19)",
            format!(
                "{}: always an approval; told {}; salary recovery {}",
                aed(payments.cash_write_off_limit),
                payments.cash_manager_role,
                yes(payments.cash_salary_deduction)
            ),
            host,
        ),
        make(
            "20",
            "Stock",
            "Stock write-off approval (STK.ACL.02)",
            format!(
                "above {} a second holder of stock.approve",
                aed(inventory.wastage_approval_fils)
            ),
            host,
        ),
        make(
            "21",
            "Stock",
            "Return to supplier approval (STK.ACL.06)",
            format!(
                "above {} a second person; the {} approves",
                aed(inventory.return_approval_fils),
                money::RETURN_APPROVER_ROLE
            ),
            host,
        ),
        make(
            "22",
            "Stock",
            "Return chase (STK.RUL.08) and low stock (STK.RUL.02)",
            format!(
                "a return is chased after {} days; stock below minimum: {}",
                inventory.return_chase_days,
                yes(money::LOW_STOCK_ALERT)
            ),
            host,
        ),
        make(
            "23",
            "Stock",
            "Supplier scorecard (STK.SUP.01, STK.RUL.09, STK.ACL.07)",
            format!(
                "score over {} months, at least {} receipts, {} grace day(s); under {} the purchaser gets a review task and the {} is told; changing a preferred supplier needs a second person: {}",
                inventory.score.window_months,
                inventory.score.min_receipts,
                inventory.score.grace_days,
                inventory.score.threshold,
                money::SCORECARD_TOLD_ROLE,
                yes(inventory.preferred_change_needs_approval)
            ),
            host,
        ),
        make(
            "24",
            "Cash-flow",
            "Forecast alert (FIN.CFL.01, FIN.RUL.23)",
            format!(
                "{} weeks from {:?}; alert below {}; {} at {}; insurer receivables by their due dates; due dates moved by days late: {}; disputed amounts counted: {}",
                invoicing.cash_forecast_weeks,
                invoicing.cash_week_start,
                thresholds,
                invoicing.cash_alert_roles.join(" and "),
                invoicing.cash_alert_time,
                yes(invoicing.cash_days_late),
                yes(invoicing.cash_disputed_counted)
            ),
            host,
        ),
        make(
            "25",
            "Budget",
            "Budget categories and approval (FIN.BUD.01, FIN.RUL.24)",
            format!(
                "categories owned by {}; one budget per legal entity and fiscal year, approved by {}; warns at {}% of a month's line: the category's owner role and {}",
                invoicing
                    .budget_category_owner
                    .as_deref()
                    .unwrap_or("nobody"),
                money::BUDGET_APPROVER_ROLE,
                invoicing.budget_variance_percent,
                invoicing.budget_variance_role
            ),
            host,
        ),
        make(
            "26",
            "Budget",
            "Owner drawings (FIN.BUD.01, FIN.ACL.21)",
            format!(
                "every drawing approved whatever its amount by {}, never the requester; kept out of operating cost and profit; {} told when none else can approve",
                money::DRAWINGS_APPROVER_ROLE,
                invoicing.drawings_notify_role
            ),
            host,
        ),
        make(
            "27",
            "Party blocked",
            "Money out of a blocked party",
            "the module's default; the agreements of 08 (fleet contract, job card, insurance claim) have their block effects registered by the app".to_owned(),
            seed_note,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entities() -> Vec<String> {
        vec!["AWF".to_owned()]
    }

    #[test]
    fn aut_18_the_invoicing_values_are_the_08_ones() {
        let module = invoicing(WlInvoicing::new(), &entities());
        let config = module.config();
        assert_eq!(config.write_off_approvers, ["owner", "admin"]);
        assert!(config.portal_disputes);
        assert_eq!(config.dispute_escalation_days, 14);
        assert_eq!(config.dispute_escalation_role, "owner");
        assert_eq!(config.payer_follow_up_days, 30);
        assert_eq!(config.bad_debt_relief_months.get(), 6);
        assert_eq!(config.cash_forecast_weeks, 13);
        assert_eq!(config.cash_week_start, Weekday::Mon);
        assert_eq!(config.cash_threshold_of("AWF"), 10_000_000);
        assert_eq!(config.cash_alert_roles, ["owner", "accountant"]);
        assert!(config.cash_days_late && !config.cash_disputed_counted);
        assert_eq!(config.cash_alert_time, "06:00");
        assert_eq!(config.budget_variance_percent, 100);
        assert_eq!(config.budget_variance_role, "accountant");
        assert_eq!(config.budget_category_owner.as_deref(), Some("accountant"));
        assert!(!config.drawings_self_approve);
        assert_eq!(config.drawings_notify_role, "owner");
        assert_eq!(config.auto_issue_sources, ["auto.job_card"]);
    }

    #[test]
    fn aut_18_the_payments_values_are_the_08_ones() {
        let module = payments(WlPayments::new());
        let config = module.config();
        assert_eq!(config.cheque_max_presentations, 2);
        assert_eq!(
            (config.cheque_reminder_days, config.cheque_decision_days),
            (3, 14)
        );
        assert!(config.bounce_fee.is_none(), "no bounce fee unless set");
        assert!(config.field_cheque_handover);
        assert_eq!(config.cash_write_off_limit, 0);
        assert!(!config.cash_salary_deduction);
        assert_eq!(config.cash_manager_role, "branch_manager");
    }

    #[test]
    fn aut_18_the_inventory_values_are_the_08_ones() {
        let module = inventory(WlInventory::new());
        let config = module.config();
        assert_eq!(config.wastage_approval_fils, 100_000);
        assert_eq!(config.return_approval_fils, 500_000);
        assert_eq!(config.return_chase_days, 30);
        assert!(config.preferred_change_needs_approval);
        assert_eq!(
            (
                config.score.window_months,
                config.score.min_receipts,
                config.score.grace_days,
                config.score.threshold
            ),
            (3, 3, 1, 70)
        );
        assert_eq!(config.recall_notices_route.as_deref(), Some("auto.recall"));
    }

    #[test]
    fn aut_18_the_page_lists_every_line_once() {
        let rows = rows(&entities());
        let numbers: Vec<String> = rows.iter().map(|r| r.number.clone()).collect();
        let expected: Vec<String> = (1..=27).map(|n| n.to_string()).collect();
        assert_eq!(numbers, expected);
        assert!(
            rows.iter()
                .all(|r| !r.value.is_empty() && !r.source.is_empty())
        );
    }
}
