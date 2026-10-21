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
//!   is retried once after 24 hours, a chargeback's evidence is reminded 2
//!   days ahead;
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
/// `FIN.RUL.20`, `FIN.RUL.21`, wl-payments section 7).
pub fn payments(module: WlPayments) -> WlPayments {
    module
        .pending_timeout(seed::PENDING_TIMEOUT_MINUTES)
        .late_refunds_auto(true)
        .refund_retry(seed::REFUND_RETRY_HOURS, seed::REFUND_MAX_RETRIES)
        .chargeback_reminder(seed::CHARGEBACK_REMINDER_DAYS)
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
        assert_eq!(config.chargeback_reminder_days, 2);
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
