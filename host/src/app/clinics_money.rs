//! The 05 Clinics client's values for the shared money and stock mechanisms
//! (05 section 3.3, 4.1, 4.2, 4.6, 5 and 7.2 *Settings for the shared
//! negative flows*; BL-CLN-16).
//!
//! The mechanisms live in `wl-invoicing`, `wl-payments`, `wl-sales` and
//! `wl-inventory`; the numbers are in `clinics::seed` (the 05 table) and this
//! file turns them into the builder calls of each module and into the one
//! seed the modules have no builder for. Nothing here is a rule: the rules
//! (`FIN.RUL.15`, `FIN.RUL.19`, `FIN.RUL.21`, `FIN.RUL.23`, `FIN.RUL.24`,
//! `STK.RUL.06`, `STK.RUL.07`) run in the modules with these values, and the
//! limits that live on a stored approval policy (`FIN.ACL.15`, `FIN.ACL.07`,
//! `FIN.ACL.18`, `FIN.ACL.21`) stay editable by the company administrator.
//!
//! - invoicing ([`invoicing`]): a patient invoice write-off is always an
//!   approval (the module's limit of AED 0), by the owner / medical director
//!   or the company administrator; a patient disputes in the portal and the
//!   clinic manager gets the ticket; escalation after 14 days; the claim
//!   write-off limit of a new payer agreement (AED 1,000, above: the owner)
//!   and the payer follow-up after 45 days; the cash-flow alert, the budget
//!   and the owner drawings;
//! - payments ([`payments`]): a pending online payment is checked after 30
//!   minutes, a late confirmation is refunded automatically, a failed refund
//!   is retried once after 24 hours, a chargeback's evidence reminder keeps
//!   the module's default (05 gives no number of days);
//! - inventory ([`inventory`]): pending issues alert after 24 hours, a recall
//!   hold is released by a second holder of `lots.release`, a hold is
//!   reminded every 24 hours. No branch store allows consumption below zero:
//!   a location blocks negative stock unless someone changes it, and nothing
//!   here does;
//! - the budget categories ([`install_budget_categories`]): owned by the
//!   accountant.
//!
//! Decisions:
//! - The write-off approvers of a patient balance are the owner / medical
//!   director and the company administrator: 05 gives the limit (AED 0) and
//!   no approver, and the owner is who approves the clinic's other write-offs
//!   (`FIN.ACL.14`). The requester is never the approver (the module's
//!   separation of duties).
//! - The dispute escalation tells the invoice's owner after 14 days and, from
//!   the second step, the owner / medical director (05 has no finance
//!   manager).
//! - The cash holder, handover and shortage values (`FIN.ACL.05`,
//!   `FIN.ACL.06`, `FIN.ACL.19`) are `clinics::seed` constants only:
//!   wl-payments' handover is `BL-PAY-17` (#110), not built.

use clinics::permissions::{CLINIC_MANAGER_ROLE, OWNER_ROLE};
use clinics::seed;
use renox::chrono::Weekday;
use renox::prelude::*;
use wl_inventory::WlInventory;
use wl_invoicing::{BudgetCategory, WlInvoicing};
use wl_kernel::COMPANY_ADMIN_ROLE;
use wl_payments::WlPayments;

use super::clinics_seed::CLINICS;
use super::negative_flow::{Row, aed};
use super::roles::ACCOUNTANT;

/// The invoicing module with the 05 client's money values: see the module
/// documentation. `entities` are the codes of the client's legal entities,
/// each given the cash-flow threshold of AED 50,000.
pub fn invoicing(module: WlInvoicing, entities: &[String]) -> WlInvoicing {
    let mut module = module
        // The patient's share of a visit is issued at once, so that the
        // deposit held for the booking can be allocated to it (BL-CLN-05).
        .auto_issue_source("clinics.visit")
        // `FIN.ACL.15`: a patient balance write-off is always approved.
        .write_off_approvers(&[OWNER_ROLE, COMPANY_ADMIN_ROLE])
        // `FIN.ACL.14`: a payer-bill write-off above the agreement's limit
        // is the owner's.
        .payer_write_off_approvers(&[OWNER_ROLE, COMPANY_ADMIN_ROLE])
        .payer_write_off_limit(seed::CLAIM_WRITE_OFF_LIMIT)
        .payer_follow_up_days(seed::PAYER_FOLLOW_UP_DAYS)
        // `FIN.BUD.01`: the accountant owns every category, marketing too.
        .budget_category_owner(ACCOUNTANT)
        // The patient's billing dispute (portal complaint form, `FIN.RUL.15`).
        .portal_disputes(true)
        .dispute_ticket_role(CLINIC_MANAGER_ROLE)
        .dispute_escalation_days(seed::DISPUTE_ESCALATION_DAYS)
        .dispute_escalation_role(OWNER_ROLE)
        // `FIN.CFL.01`, `FIN.RUL.23`: 13 weeks from Monday, insurer
        // remittances moved by each payer's days late, disputed amounts not
        // counted, checked daily at 06:00.
        .cash_week_start(Weekday::Mon)
        .cash_forecast_weeks(seed::CASH_FORECAST_WEEKS)
        .cash_days_late(true)
        .cash_disputed_counted(false)
        .cash_alert_time(seed::CASH_ALERT_TIME)
        .cash_alert_roles(&[OWNER_ROLE, ACCOUNTANT])
        // `FIN.RUL.24`: warns at 100% of a month's line, the category's owner
        // role (the accountant) and the accountant.
        .budget_variance_percent(seed::BUDGET_WARNING_PERCENT)
        .budget_variance_role(ACCOUNTANT)
        // `FIN.ACL.21`: every drawing is approved by someone other than the
        // requester.
        .drawings_self_approve(false)
        .drawings_notify_role(OWNER_ROLE);
    for code in entities {
        module = module.cash_threshold(code, seed::CASH_THRESHOLD);
    }
    module
}

/// The payments module with the 05 client's values (`FIN.RUL.19`,
/// `FIN.RUL.20`, `FIN.RUL.21`, wl-payments section 7). 05 gives no number of
/// days for the chargeback evidence reminder: the module's own default stays.
pub fn payments(module: WlPayments) -> WlPayments {
    module
        .pending_timeout(seed::PENDING_TIMEOUT_MINUTES)
        .late_refunds_auto(true)
        .refund_retry(seed::REFUND_RETRY_HOURS, seed::REFUND_MAX_RETRIES)
}

/// The inventory module with the 05 client's values (`STK.RUL.06`,
/// `STK.RUL.07`, `STK.ACL.05`). Any other client keeps the module's
/// defaults.
pub fn inventory(client_app: &str, module: WlInventory) -> WlInventory {
    if client_app != CLINICS {
        return module;
    }
    module
        .pending_alert_hours(seed::PENDING_ISSUE_ALERT_HOURS)
        .hold_second_person_reasons(seed::SECOND_PERSON_HOLD_REASONS)
        .hold_reminder_hours(seed::HOLD_REMINDER_HOURS)
        .hold_reminder_hours_for_reason(seed::SUSPECT_HOLD_REASON, seed::HOLD_REMINDER_HOURS)
}

/// The budget categories are owned by the accountant (`FIN.BUD.01`,
/// `FIN.RUL.24`): every category with no owner role gets it, the owner
/// drawings category excepted (it is never operating cost). A role an
/// administrator set is kept; the mapping to the client's export accounts is
/// the accountant's. Returns how many categories changed.
///
/// # Errors
///
/// The database's errors.
pub async fn install_budget_categories(db: &Db) -> Result<usize> {
    let mut changed = 0;
    for mut category in BudgetCategory::listed(db, true).await? {
        if category.owner_role.is_none() && !category.is_drawings() {
            category.owner_role = Some(ACCOUNTANT.to_owned());
            category.save(db).await?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// The rows of the read-only money settings page for the 05 client: what the
/// invoicing, payments and inventory modules have in force for it, read back
/// from the same builder calls (`entities` are the codes of the client's
/// legal entities).
pub fn rows(entities: &[String]) -> Vec<Row> {
    let invoicing = invoicing(WlInvoicing::new(), entities);
    let invoicing = invoicing.config();
    let payments = payments(WlPayments::new());
    let payments = payments.config();
    let stock = inventory(CLINICS, WlInventory::new());
    let stock = stock.config();
    let host = "set by the host for this client";
    let module = "default of the module";
    let seed_note = "the 05 values (clinics::seed)";
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
    vec![
        make(
            "1",
            "Invoices",
            "Patient write-off (FIN.ACL.15)",
            format!(
                "limit AED 0: always an approval by {}, never the requester",
                invoicing.write_off_approvers.join(" or ")
            ),
            host,
        ),
        make(
            "2",
            "Invoices",
            "Billing dispute by the patient (FIN.RUL.15)",
            format!(
                "{}; escalated after {} days to {}; the ticket goes to {}",
                if invoicing.portal_disputes { "on" } else { "off" },
                invoicing.dispute_escalation_days,
                invoicing.dispute_escalation_role,
                invoicing.dispute_ticket_role.as_deref().unwrap_or("nobody")
            ),
            host,
        ),
        make(
            "3",
            "Claims",
            "Claim write-off limit (FIN.ACL.14)",
            format!(
                "{} for a new payer agreement; above it {} approve",
                aed(invoicing.payer_write_off_limit),
                invoicing.payer_write_off_approvers.join(" or ")
            ),
            seed_note,
        ),
        make(
            "4",
            "Claims",
            "Follow-up of a payer bill with no remittance (FIN.RUL.13)",
            format!("{} days", invoicing.payer_follow_up_days),
            seed_note,
        ),
        make(
            "5",
            "Payments",
            "Online payment pending (FIN.RUL.19)",
            format!(
                "checked with the gateway after {} minutes; a late confirmation is refunded: {}",
                payments.pending_timeout_minutes,
                if payments.late_refund_auto { "yes" } else { "no" }
            ),
            seed_note,
        ),
        make(
            "6",
            "Payments",
            "Failed refund (FIN.RUL.21)",
            format!(
                "retried {} time(s) after {} hours",
                payments.refund_max_retries, payments.refund_retry_hours
            ),
            seed_note,
        ),
        make(
            "7",
            "Payments",
            "Chargeback evidence reminder (FIN.RUL.20)",
            format!("{} days before the deadline", payments.chargeback_reminder_days),
            module,
        ),
        make(
            "8",
            "Cash",
            "Cash holders and handover (FIN.CSH.01)",
            format!(
                "one holder per {} of the receptionist's {}; shortage write-off limit {}; salary recovery {} (not built: BL-PAY-17)",
                seed::CASH_HANDOVER_PERIOD,
                seed::CASH_HOLDER_MODE,
                aed(seed::CASH_SHORTAGE_WRITE_OFF_LIMIT),
                if seed::SALARY_RECOVERY { "on" } else { "off" }
            ),
            seed_note,
        ),
        make(
            "9",
            "Cash-flow",
            "Forecast alert (FIN.RUL.23)",
            format!(
                "{} weeks from {:?}; alert below {}; {} at {}",
                invoicing.cash_forecast_weeks,
                invoicing.cash_week_start,
                thresholds,
                invoicing.cash_alert_roles.join(" and "),
                invoicing.cash_alert_time
            ),
            host,
        ),
        make(
            "10",
            "Budget",
            "Budget categories (FIN.RUL.24)",
            format!(
                "owned by {}; warns at {}% of a month's line: the category's owner role and {}",
                invoicing
                    .budget_category_owner
                    .as_deref()
                    .unwrap_or("nobody"),
                invoicing.budget_variance_percent,
                invoicing.budget_variance_role
            ),
            host,
        ),
        make(
            "11",
            "Budget",
            "Owner drawings (FIN.ACL.21)",
            format!(
                "every drawing approved by someone other than the requester; {} told when none else can approve",
                invoicing.drawings_notify_role
            ),
            host,
        ),
        make(
            "12",
            "Stock",
            "Negative stock (STK.RUL.06)",
            "block: the short quantity of a visit's consumables becomes a pending issue; no branch store allows consumption below zero".to_owned(),
            module,
        ),
        make(
            "13",
            "Stock",
            "Pending issue alert (STK.RUL.06)",
            format!(
                "the storekeeper is told at once; after {} hours they have the task and the clinic manager is told",
                stock.pending_alert_hours
            ),
            seed_note,
        ),
        make(
            "14",
            "Stock",
            "Stock hold (STK.RUL.07, STK.ACL.05)",
            format!(
                "reminded every {} hours; a second holder of lots.release for {}",
                stock.hold_reminder_hours,
                stock.hold_second_person_reasons.join(", ")
            ),
            seed_note,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entities() -> Vec<String> {
        vec!["PGC".to_owned()]
    }

    #[test]
    fn the_invoicing_values_are_the_05_ones() {
        let module = invoicing(WlInvoicing::new(), &entities());
        let config = module.config();
        assert_eq!(config.write_off_approvers, ["owner", "admin"]);
        assert_eq!(config.payer_write_off_approvers, ["owner", "admin"]);
        assert_eq!(config.payer_write_off_limit, 100_000);
        assert_eq!(config.payer_follow_up_days, 45);
        assert!(config.portal_disputes);
        assert_eq!(config.dispute_escalation_days, 14);
        assert_eq!(config.dispute_escalation_role, "owner");
        assert_eq!(
            config.dispute_ticket_role.as_deref(),
            Some("clinic_manager")
        );
        assert_eq!(config.cash_forecast_weeks, 13);
        assert_eq!(config.cash_week_start, Weekday::Mon);
        assert_eq!(config.cash_threshold_of("PGC"), 5_000_000);
        assert_eq!(config.cash_alert_roles, ["owner", "accountant"]);
        assert!(config.cash_days_late && !config.cash_disputed_counted);
        assert_eq!(config.cash_alert_time, "06:00");
        assert_eq!(config.budget_variance_percent, 100);
        assert_eq!(config.budget_variance_role, "accountant");
        assert!(!config.drawings_self_approve);
        assert_eq!(config.auto_issue_sources, ["clinics.visit"]);
    }

    #[test]
    fn the_payments_values_are_the_05_ones() {
        let module = payments(WlPayments::new());
        let config = module.config();
        assert_eq!(config.pending_timeout_minutes, 30);
        assert!(config.late_refund_auto);
        assert_eq!(config.refund_retry_hours, 24);
        assert_eq!(config.refund_max_retries, 1);
        // 05 gives no number: the module's own default applies.
        assert_eq!(
            config.chargeback_reminder_days,
            wl_payments::config::DEFAULT_CHARGEBACK_REMINDER_DAYS
        );
    }

    #[test]
    fn the_inventory_values_are_the_05_ones_and_another_client_keeps_the_defaults() {
        let module = inventory(CLINICS, WlInventory::new());
        let config = module.config();
        assert_eq!(config.pending_alert_hours, 24);
        assert!(config.release_needs_second_person("recall"));
        assert!(!config.release_needs_second_person("quality"));
        assert_eq!(config.reminder_hours("quality", None), 24);
        let other = inventory("property", WlInventory::new().pending_alert_hours(6));
        assert_eq!(other.config().pending_alert_hours, 6);
    }
}
