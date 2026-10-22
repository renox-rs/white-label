//! The 10 EdTech values for the shared money mechanisms (10 §3.3, §4.3 *When it
//! goes wrong*, §5, §6, §7.1 *Subscription*, §7.2 *Settings for the shared
//! sales, finance, staff and document capabilities*, §9, §13 question 5;
//! BL-EDU-18).
//!
//! The mechanisms live in `wl-invoicing`, `wl-payments` and `wl-sales`; the
//! numbers are here and the host turns them into the builder calls of each
//! module (`host/src/app/edtech_money.rs`). Nothing here is a rule: the rules
//! (`FIN.RUL.06`, `FIN.RUL.15`, `FIN.RUL.19`-`21`, `FIN.RUL.23`, `FIN.RUL.24`)
//! run in the modules with these values, and the limits that live on a stored
//! approval policy (`FIN.ACL.04`, `FIN.ACL.06`, `FIN.ACL.15`, `FIN.ACL.21`)
//! stay editable by the company administrator.
//!
//! Where the value is the module's own default (refund approval by another
//! user, a pending payment of 30 minutes, a dispute escalated after 14 days,
//! e-invoicing for corporate invoices only), the host sets it all the same,
//! so that a change of a default never moves the centre.

use crate::enrolments::DEFAULT_EXCUSE_NOTICE_HOURS;
use crate::permissions::{ACCOUNTANT_ROLE, CENTRE_MANAGER_ROLE, COUNSELLOR_ROLE, OWNER_ROLE};
use crate::refund::RefundPolicy;

// ---- Refunds and payments (10 §4.3, §7.2) ----

/// Hours after a gateway refund failed before it is retried (`FIN.RUL.21`).
pub const REFUND_RETRY_HOURS: i64 = 24;

/// The retries of one failed refund: once; then the accountant gets a task to
/// pay by another method.
pub const REFUND_MAX_RETRIES: i64 = 1;

/// Minutes after an online payment became `pending` when the gateway is asked
/// for its final state (`FIN.RUL.19`): the module's default.
pub const PENDING_TIMEOUT_MINUTES: i64 = 30;

/// The roles that approve a write-off, whatever its amount (`FIN.ACL.15`, the
/// limit is AED 0): the centre manager and the company admin, never the user
/// who asked.
pub const WRITE_OFF_APPROVERS: [&str; 2] = [CENTRE_MANAGER_ROLE, "admin"];

/// The role that decides a cash shortage at the desk and is told of one
/// (`FIN.ACL.06`).
pub const CASH_MANAGER_ROLE: &str = CENTRE_MANAGER_ROLE;

/// The cash shortage a user may write off without an approval, in fils: AED 0,
/// every shortage is approved (`FIN.ACL.06`).
pub const CASH_WRITE_OFF_LIMIT: i64 = 0;

/// Whether a shortage may be recovered by salary deduction (`FIN.ACL.19`): off
/// `⚠ open: lawyer (L-20)`; recovery is in cash.
pub const SALARY_RECOVERY: bool = false;

/// Who holds desk cash (`FIN.CSH.01`): each counsellor, per person.
pub const CASH_HOLDER_ROLE: &str = COUNSELLOR_ROLE;

/// The period of a desk handover: a day (Asia/Dubai), counted by another
/// user.
pub const CASH_HANDOVER_PERIOD: &str = "day";

/// Whether a corporate contact may dispute an invoice in the portal: off until
/// the corporate portal ships (it is not in the MVP).
pub const PORTAL_DISPUTES: bool = false;

/// Days a dispute may stay open before `FIN.RUL.15` escalates it: the module's
/// default.
pub const DISPUTE_ESCALATION_DAYS: u32 = 14;

// ---- Memberships and packages (10 §7.1 *Subscription*, §13 question 5) ----

/// The days after a failed membership charge when the payment is tried again
/// (`FIN.RUL.06`): 1, 3 and 7.
pub const MEMBERSHIP_RETRY_DAYS: [u32; 3] = [1, 3, 7];

/// Whether the membership is `suspended` after the last retry.
pub const MEMBERSHIP_SUSPENDS: bool = true;

/// The days a year a student may pause the membership or freeze the
/// enrolment, one allowance shared by both (`EDU.RUL.14`): overrides the
/// wl-invoicing default of 0.
pub const PAUSE_ALLOWANCE_DAYS: u32 = 30;

/// Whether a package stops being consumed when its invoice is reopened:
/// never, the counsellor is given a task to collect (wl-sales §7).
pub const SUSPEND_PACKAGE_WHILE_UNPAID: bool = false;

/// The role that gets the task to collect after a reopened package invoice.
pub const PACKAGE_COLLECT_ROLE: &str = COUNSELLOR_ROLE;

// ---- The catalogue defaults (10 §6, §9) ----

/// The refund policy template of a new catalogue item: before the start 100%
/// minus the registration fee; after the start the unused sessions pro rata
/// minus the admin fee; after this share of the sessions no refund.
pub fn refund_template() -> RefundPolicy {
    RefundPolicy::default()
}

/// The key of the tiered refund template.
pub const REFUND_TEMPLATE_KEY: &str = "standard";

/// The share of the sessions after which nothing is refunded, in percent.
pub const REFUND_NO_REFUND_AFTER_PERCENT: i64 = 50;

/// Whether an unexcused absence consumes a credit of a package (the absence
/// policy of a new catalogue item).
pub const ABSENCE_CONSUMES_CREDIT: bool = true;

/// The hours before a session that a notice excuses an absence.
pub const EXCUSE_NOTICE_HOURS: i64 = DEFAULT_EXCUSE_NOTICE_HOURS;

// ---- Cash-flow forecast and budget (10 §5, §7.2 settings) ----

/// The projected balance below which `FIN.RUL.23` alerts, in fils: AED 25,000,
/// in any of the next 13 weeks.
pub const CASH_THRESHOLD: i64 = 2_500_000;

/// The weeks the cash-flow forecast looks ahead (`FIN.CFL.01`).
pub const CASH_FORECAST_WEEKS: u32 = 13;

/// When the daily cash-flow check runs (`FIN.RUL.23`), Asia/Dubai.
pub const CASH_ALERT_TIME: &str = "06:00";

/// The roles `FIN.RUL.23` alerts: the company owner and the accountant.
pub const CASH_ALERT_ROLES: [&str; 2] = [OWNER_ROLE, ACCOUNTANT_ROLE];

/// The share of a month's budget line at which `FIN.RUL.24` warns, in percent.
pub const BUDGET_WARNING_PERCENT: u32 = 100;

/// The owner role of every budget category (`FIN.BUD.01`): the accountant.
pub const BUDGET_CATEGORY_OWNER: &str = ACCOUNTANT_ROLE;

/// The budget category 10 adds to the module's own (*Cost of sales* covers the
/// materials, *Salaries and staff costs* the staff, *Rent and utilities* the
/// rent and *Marketing* the marketing): the tutors' pay.
pub struct BudgetSpec {
    /// The category code.
    pub code: &'static str,
    /// The English name.
    pub name_en: &'static str,
    /// The Arabic name.
    pub name_ar: &'static str,
    /// The cost codes of the export accounts it maps from.
    pub cost_codes: &'static [&'static str],
}

/// The categories 10 adds.
pub const BUDGET_CATEGORIES: [BudgetSpec; 1] = [BudgetSpec {
    code: "tutor_costs",
    name_en: "Tutor costs",
    name_ar: "تكاليف المدرّسين",
    cost_codes: &["tutor_costs", "tutor_pay"],
}];

/// Who approves a budget: the company owner (`FIN.BUD.01`).
pub const BUDGET_APPROVER_ROLE: &str = OWNER_ROLE;

/// Who approves an owner drawing, whatever its amount (`FIN.ACL.21`, the limit
/// is AED 0): the company owner, never the requester.
pub const DRAWINGS_APPROVER_ROLE: &str = OWNER_ROLE;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refund_template_is_the_tiered_one() {
        let template = refund_template();
        assert_eq!(template.key, REFUND_TEMPLATE_KEY);
        assert_eq!(
            template.no_refund_after_percent,
            REFUND_NO_REFUND_AFTER_PERCENT
        );
    }

    #[test]
    fn the_allowance_is_the_one_the_freeze_counts_against() {
        assert_eq!(
            i64::from(PAUSE_ALLOWANCE_DAYS),
            crate::enrolment_machine::FREEZE_ALLOWANCE_DAYS
        );
    }

    #[test]
    fn the_cash_values_never_allow_a_salary_recovery_or_a_free_write_off() {
        assert_eq!(CASH_WRITE_OFF_LIMIT, 0);
        const { assert!(!SALARY_RECOVERY) };
    }
}
