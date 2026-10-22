//! The 10 EdTech client's values for the shared money mechanisms (10 §3.3,
//! §4.3 *When it goes wrong*, §5, §6, §7.1 *Subscription*, §7.2, §9, §13
//! question 5; BL-EDU-18).
//!
//! The mechanisms live in `wl-invoicing`, `wl-payments` and `wl-sales`; the
//! numbers are `edtech::money_defaults` and this file turns them into the
//! builder calls of each module and into the one seed the modules have no
//! builder for. Nothing here is a rule: the rules (`FIN.RUL.06`, `FIN.RUL.15`,
//! `FIN.RUL.19`-`21`, `FIN.RUL.23`, `FIN.RUL.24`) run in the modules with these
//! values, and the limits that live on a stored approval policy (`FIN.ACL.04`,
//! `FIN.ACL.06`, `FIN.ACL.15`, `FIN.ACL.21`) stay editable by the company
//! administrator.
//!
//! - invoicing ([`invoicing`]): every write-off is an approval (the module's
//!   limit of AED 0) by the centre manager or the company admin, never the
//!   requester; a corporate contact does not dispute in the portal (off until
//!   the corporate portal ships) and a dispute is escalated after 14 days;
//!   the membership is retried on days 1, 3 and 7 and then `suspended`, and
//!   the pause allowance is 30 days a year (shared with the enrolment freeze);
//!   the cash-flow alert, the budget and the owner drawings;
//! - payments ([`payments`]): a pending online payment is checked after 30
//!   minutes, a late confirmation after the seat lapsed is refunded
//!   automatically, a failed refund is retried once after 24 hours and then
//!   the accountant gets a task to pay by another method, a desk cash shortage
//!   is always an approval (AED 0) with salary recovery off;
//! - the budget categories ([`install_budget_categories`]): *Tutor costs* is
//!   added to the module's own and every category is owned by the accountant.
//!
//! Decisions:
//! - The write-off approvers are the centre manager and the company admin: the
//!   spec gives the limit (AED 0) and no approver; the centre manager is who
//!   approves the refunds and discounts (`FIN.ACL.04`, `SAL.ACL.02`). The
//!   requester is never the approver (the module's separation of duties).
//! - The centre manager holds `cash.write_off` and `cash.view` in an EdTech
//!   deployment ([`super::edtech_settings::grant_roles`]): the approval of a
//!   desk shortage is that permission, and the accountant counts the cash.
//! - **Not built here:** the package `refund` valuation and the catalogue
//!   defaults (wl-sales has no `refund` of a package and no catalogue item
//!   yet), listed in `$S/f10/DEFERRED.md`.

use edtech::money_defaults as money;
use renox::chrono::Weekday;
use renox::prelude::*;
use wl_invoicing::budget_category::{self, CategoryInput};
use wl_invoicing::{BudgetCategory, BudgetKind, MappingKind, WlInvoicing};
use wl_payments::WlPayments;

use super::negative_flow::{Row, aed};

/// The invoicing module with the 10 client's money values: see the module
/// documentation. `entities` are the codes of the client's legal entities,
/// each given the cash-flow threshold of AED 25,000.
pub fn invoicing(module: WlInvoicing, entities: &[String]) -> WlInvoicing {
    let mut module = module
        // The fee invoice of an enrolment is issued at once, so that the first
        // payment, made on the payment link, is allocated to it and activates
        // the enrolment (`EDU.RUL.11`); the membership's cycle invoices bill
        // themselves (`FIN.RUL.06`).
        .auto_issue_source(edtech::enrolment::ENROLMENT_SUBJECT_TYPE)
        .auto_issue_source("invoicing.subscription")
        // `FIN.ACL.15`: every write-off is approved.
        .write_off_approvers(&money::WRITE_OFF_APPROVERS)
        // A corporate contact does not dispute in the portal until the
        // corporate portal ships; `FIN.RUL.15` escalates after 14 days.
        .portal_disputes(money::PORTAL_DISPUTES)
        .dispute_escalation_days(money::DISPUTE_ESCALATION_DAYS)
        // `FIN.RUL.06`: retries on days 1, 3 and 7, then `suspend`; the
        // allowance of 30 days a year is shared with the enrolment freeze.
        .subscription_suspension(money::MEMBERSHIP_SUSPENDS)
        .subscription_retry_days(&money::MEMBERSHIP_RETRY_DAYS)
        .subscription_pause_allowance(money::PAUSE_ALLOWANCE_DAYS)
        // `FIN.BUD.01`: the accountant owns every category.
        .budget_category_owner(money::BUDGET_CATEGORY_OWNER)
        // `FIN.CFL.01`, `FIN.RUL.23`: 13 weeks from Monday, instalments by
        // due date moved by each payer's days late, disputed amounts not
        // counted, checked daily at 06:00.
        .cash_week_start(Weekday::Mon)
        .cash_forecast_weeks(money::CASH_FORECAST_WEEKS)
        .cash_days_late(true)
        .cash_disputed_counted(false)
        .cash_alert_time(money::CASH_ALERT_TIME)
        .cash_alert_roles(&money::CASH_ALERT_ROLES)
        // `FIN.RUL.24`: warns at 100% of a month's line the category's owner
        // role (the accountant) and the accountant.
        .budget_variance_percent(money::BUDGET_WARNING_PERCENT)
        .budget_variance_role(money::BUDGET_CATEGORY_OWNER)
        // `FIN.ACL.21`: every drawing is approved by someone other than the
        // requester; the owner is told when none else can approve.
        .drawings_self_approve(false)
        .drawings_notify_role(money::DRAWINGS_APPROVER_ROLE);
    for code in entities {
        module = module.cash_threshold(code, money::CASH_THRESHOLD);
    }
    module
}

/// The payments module with the 10 client's values (`FIN.RUL.19`, `FIN.RUL.21`,
/// `FIN.ACL.06`, `FIN.ACL.19`, wl-payments section 7). The chargeback evidence
/// reminder and the chargeback tasks keep the module's defaults.
pub fn payments(module: WlPayments) -> WlPayments {
    module
        .pending_timeout(money::PENDING_TIMEOUT_MINUTES)
        .late_refunds_auto(true)
        .refund_retry(money::REFUND_RETRY_HOURS, money::REFUND_MAX_RETRIES)
        // A desk shortage is always an approval and is never recovered from
        // the salary (`⚠ open: lawyer L-20`).
        .cash_write_off_limit(money::CASH_WRITE_OFF_LIMIT)
        .cash_salary_deduction(money::SALARY_RECOVERY)
        .cash_manager(money::CASH_MANAGER_ROLE)
}

/// Adds the budget categories 10 has beyond the module's own (*Tutor costs*)
/// and gives every category with no owner role the accountant (`FIN.BUD.01`,
/// `FIN.RUL.24`); the owner drawings category never has one. A role an
/// administrator set is kept; the mapping to the client's export accounts is
/// the accountant's. Returns how many categories changed.
///
/// # Errors
///
/// The database's errors.
pub async fn install_budget_categories(db: &Db) -> Result<usize> {
    let mut changed = 0;
    for spec in &money::BUDGET_CATEGORIES {
        if BudgetCategory::by_code(db, spec.code).await?.is_some() {
            continue;
        }
        budget_category::save(
            db,
            None,
            CategoryInput {
                code: spec.code.to_owned(),
                name_en: spec.name_en.to_owned(),
                name_ar: Some(spec.name_ar.to_owned()),
                kind: BudgetKind::Expense,
                parent_code: None,
                owner_role: Some(money::BUDGET_CATEGORY_OWNER.to_owned()),
                owner_party_id: None,
                mappings: spec
                    .cost_codes
                    .iter()
                    .map(|code| (MappingKind::CostCode, (*code).to_owned()))
                    .collect(),
                active: true,
            },
        )
        .await?;
        changed += 1;
    }
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

/// The rows of the read-only money settings page for the 10 client: what the
/// invoicing and payments modules have in force for it, read back from the same
/// builder calls (`entities` are the codes of the client's legal entities).
pub fn rows(entities: &[String]) -> Vec<Row> {
    let invoicing = invoicing(WlInvoicing::new(), entities);
    let invoicing = invoicing.config();
    let payments = payments(WlPayments::new());
    let payments = payments.config();
    let host = "set by the host for this client";
    let module = "default of the module, set by the host";
    let seed_note = "the 10 values (edtech::money_defaults)";
    let make = |number: &str, group: &str, setting: &str, value: String, source: &str| Row {
        number: number.to_owned(),
        group: group.to_owned(),
        setting: setting.to_owned(),
        value,
        source: source.to_owned(),
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
    let retry_days = invoicing
        .subscription_retry_days
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    vec![
        make(
            "1",
            "Refunds",
            "Refund approval (FIN.ACL.04)",
            "the user who records a refund cannot approve it; always".to_owned(),
            module,
        ),
        make(
            "2",
            "Refunds",
            "Failed refund (FIN.RUL.21)",
            format!(
                "retried {} time(s) after {} hours, then a task to the accountant to pay by another method; the credit note balance is restored by wl-invoicing",
                payments.refund_max_retries, payments.refund_retry_hours
            ),
            module,
        ),
        make(
            "3",
            "Payments",
            "Pending online payment (FIN.RUL.19)",
            format!(
                "checked with the gateway after {} minutes",
                payments.pending_timeout_minutes
            ),
            module,
        ),
        make(
            "4",
            "Payments",
            "Late payment after the seat lapsed",
            format!(
                "refunded automatically (late_confirm): {}",
                yes(payments.late_refund_auto)
            ),
            module,
        ),
        make(
            "5",
            "Payments",
            "Chargebacks (FIN.RUL.20, FIN.ACL.18)",
            format!(
                "module defaults (evidence reminder {} days before the deadline); a course fee gives the counsellor a task, a membership goes past_due",
                payments.chargeback_reminder_days
            ),
            module,
        ),
        make(
            "6",
            "Memberships",
            "Membership retries (FIN.RUL.06)",
            format!(
                "days {retry_days}; suspend after the last retry: {}",
                yes(invoicing.subscription_suspension)
            ),
            host,
        ),
        make(
            "7",
            "Memberships",
            "Pause and freeze allowance (EDU.RUL.14)",
            format!(
                "{} days a year, shared by the membership pause and the enrolment freeze (overrides the wl-invoicing default of 0)",
                invoicing.subscription_pause_allowance_days
            ),
            host,
        ),
        make(
            "8",
            "Packages",
            "Package refund valuation",
            "unused sessions at the single-session price, without the package discount (the refund is calculated by the app; the package refund of wl-sales is not built)".to_owned(),
            seed_note,
        ),
        make(
            "9",
            "Packages",
            "Package after a reopened invoice",
            format!(
                "a task to the {} to collect; consumption is not suspended ({})",
                money::PACKAGE_COLLECT_ROLE,
                yes(money::SUSPEND_PACKAGE_WHILE_UNPAID)
            ),
            host,
        ),
        make(
            "10",
            "Cash",
            "Desk cash (FIN.CSH.01)",
            format!(
                "one holder per {} per {}, counted by another user",
                money::CASH_HOLDER_ROLE,
                money::CASH_HANDOVER_PERIOD
            ),
            seed_note,
        ),
        make(
            "11",
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
            "12",
            "Corporate",
            "Corporate invoice dispute from the portal",
            format!(
                "{} until the corporate portal ships",
                yes(invoicing.portal_disputes)
            ),
            host,
        ),
        make(
            "13",
            "Corporate",
            "Dispute escalation (FIN.RUL.15)",
            format!(
                "after {} days, to {}",
                invoicing.dispute_escalation_days, invoicing.dispute_escalation_role
            ),
            module,
        ),
        make(
            "14",
            "Corporate",
            "E-invoice rejection (FIN.RUL.16)",
            "corporate (B2B) invoices only; with the ASP down the invoice is still issued and sent as PDF".to_owned(),
            "default of the module",
        ),
        make(
            "15",
            "Invoices",
            "Bad-debt write-off (FIN.ACL.15)",
            format!(
                "AED 0: every write-off is approved by {}, never the requester",
                invoicing.write_off_approvers.join(" or ")
            ),
            host,
        ),
        make(
            "16",
            "Catalogue",
            "Refund policy template (catalogue default)",
            format!(
                "before the start 100% minus the registration fee; after the start the unused sessions pro rata minus an admin fee; after {}% of the sessions no refund",
                money::REFUND_NO_REFUND_AFTER_PERCENT
            ),
            seed_note,
        ),
        make(
            "17",
            "Catalogue",
            "Absence policy (catalogue default)",
            format!(
                "an unexcused absence consumes a credit: {}; notice for an excused absence: {} hours",
                if money::ABSENCE_CONSUMES_CREDIT { "yes" } else { "no" },
                money::EXCUSE_NOTICE_HOURS
            ),
            seed_note,
        ),
        make(
            "18",
            "Cash-flow",
            "Forecast alert (FIN.CFL.01, FIN.RUL.23)",
            format!(
                "{} weeks from {:?}; alert below {}; {} at {}; instalments by due date; due dates moved by days late: {}; disputed amounts counted: {}",
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
            "19",
            "Budget",
            "Budget categories and approval (FIN.BUD.01, FIN.RUL.24)",
            format!(
                "categories owned by {} (tutor costs, rent, marketing, materials, staff); one budget per legal entity and fiscal year, approved by {}; warns at {}% of a month's line: the category's owner role and {}",
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
            "20",
            "Budget",
            "Owner drawings (FIN.BUD.01, FIN.ACL.21)",
            format!(
                "every drawing approved whatever its amount by {}, never the requester; kept out of operating cost and profit; {} told when none else can approve",
                money::DRAWINGS_APPROVER_ROLE,
                invoicing.drawings_notify_role
            ),
            host,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::Weekday;

    fn entities() -> Vec<String> {
        vec!["GSTC".to_owned()]
    }

    #[test]
    fn the_invoicing_values_are_the_10_ones() {
        let module = invoicing(WlInvoicing::new(), &entities());
        let config = module.config();
        assert_eq!(config.write_off_approvers, ["centre_manager", "admin"]);
        assert!(!config.portal_disputes);
        assert_eq!(config.dispute_escalation_days, 14);
        assert!(config.subscription_suspension);
        assert_eq!(config.subscription_retry_days, [1, 3, 7]);
        assert_eq!(config.subscription_pause_allowance_days, 30);
        assert_eq!(config.cash_forecast_weeks, 13);
        assert_eq!(config.cash_week_start, Weekday::Mon);
        assert_eq!(config.cash_threshold_of("GSTC"), 2_500_000);
        assert_eq!(config.cash_alert_roles, ["owner", "accountant"]);
        assert!(config.cash_days_late && !config.cash_disputed_counted);
        assert_eq!(config.cash_alert_time, "06:00");
        assert_eq!(config.budget_variance_percent, 100);
        assert_eq!(config.budget_variance_role, "accountant");
        assert_eq!(config.budget_category_owner.as_deref(), Some("accountant"));
        assert!(!config.drawings_self_approve);
        assert_eq!(config.drawings_notify_role, "owner");
    }

    #[test]
    fn the_payments_values_are_the_10_ones() {
        let module = payments(WlPayments::new());
        let config = module.config();
        assert_eq!(config.pending_timeout_minutes, 30);
        assert!(config.late_refund_auto);
        assert_eq!(config.refund_retry_hours, 24);
        assert_eq!(config.refund_max_retries, 1);
        assert_eq!(config.cash_write_off_limit, 0);
        assert!(!config.cash_salary_deduction);
        assert_eq!(config.cash_manager_role, "centre_manager");
    }

    #[test]
    fn the_page_lists_every_line_of_the_issue_table_once() {
        let rows = rows(&entities());
        let numbers: Vec<String> = rows.iter().map(|r| r.number.clone()).collect();
        let expected: Vec<String> = (1..=20).map(|n| n.to_string()).collect();
        assert_eq!(numbers, expected);
        assert!(
            rows.iter()
                .all(|r| !r.value.is_empty() && !r.source.is_empty())
        );
    }
}
