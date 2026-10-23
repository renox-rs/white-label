//! The 08 Auto values for the shared money and stock mechanisms (08 §3.3, §4.1,
//! §4.3 *When it goes wrong*, §5, §7.1 *Negative-flow settings*, §7.2;
//! BL-AUT-18).
//!
//! The mechanisms live in `wl-invoicing`, `wl-payments`, `wl-inventory` and
//! `wl-sales`; the numbers are here and the host turns them into the builder
//! calls of each module (`host/src/app/auto_money.rs`). Nothing here is a
//! rule: the rules (`FIN.RUL.13`, `FIN.RUL.14`, `FIN.RUL.15`, `FIN.RUL.18`,
//! `FIN.RUL.23`, `FIN.RUL.24`, `STK.RUL.02`, `STK.RUL.08`) run in the modules
//! with these values, and the limits that live on a stored approval policy
//! (`FIN.ACL.15`, `FIN.ACL.20`, `FIN.ACL.21`) stay editable by the company
//! administrator.
//!
//! The values the modules have no setting for (the sales discount limit
//! `SAL.ACL.02`, the credit grace `SAL.ACL.03`, the cancellation after picking
//! `SAL.ACL.05`, the bounced-cheque hold `SAL.RUL.05`, the partial acceptance at
//! proof of delivery) are kept here for the settings page and listed as
//! deferred; nothing enforces them yet.

use crate::permissions::{
    ACCOUNTANT_ROLE, BRANCH_MANAGER_ROLE, OWNER_ROLE, PURCHASER_ROLE, WORKSHOP_MANAGER_ROLE,
};

// ---- Invoices and write-offs (08 §3.3, §4.1) ----

/// The roles that approve a write-off, whatever its amount (`FIN.ACL.15`, the
/// limit is AED 0): the Owner / GM and the company admin, never the user who
/// asked (the accountant, who requests it).
pub const WRITE_OFF_APPROVERS: [&str; 2] = [OWNER_ROLE, "admin"];

/// The write-off a user may make without an approval, in fils: AED 0.
pub const WRITE_OFF_LIMIT: i64 = 0;

/// Whether the B2B buyer may flag an invoice as disputed in the portal.
pub const PORTAL_DISPUTES: bool = true;

/// Days a dispute may stay open before `FIN.RUL.15` escalates it.
pub const DISPUTE_ESCALATION_DAYS: u32 = 14;

/// The role a dispute escalates to: the Owner / GM.
pub const DISPUTE_ESCALATION_ROLE: &str = OWNER_ROLE;

/// The months after the date of supply before the bad-debt relief is proposed
/// (`FIN.RUL.14`).
pub const BAD_DEBT_RELIEF_MONTHS: u32 = 6;

/// The days after an insurer's invoice when `FIN.RUL.13` follows up an unpaid
/// one (the task is the insurance coordinator's).
pub const INSURER_FOLLOW_UP_DAYS: i64 = 30;

/// The role told when a payer bill is rejected by the insurer's channel.
pub const PAYER_BILL_TASK_ROLE: &str = crate::permissions::INSURANCE_COORDINATOR_ROLE;

// ---- Cheques and cash (08 §4.1, §3.3) ----

/// The most presentations of one cheque (`FIN.RUL.18`): a third is refused.
pub const CHEQUE_PRESENTATIONS: i64 = 2;

/// Days after a bounce when the customer is reminded (`FIN.RUL.18`).
pub const BOUNCE_REMIND_DAYS: i64 = 3;

/// Days after a bounce when the accountant decides (`FIN.RUL.18`).
pub const BOUNCE_DECIDE_DAYS: i64 = 14;

/// The role told of a bounced cheque besides the accountant: the branch
/// manager.
pub const CHEQUE_MANAGER_ROLE: &str = BRANCH_MANAGER_ROLE;

/// Whether a bounced cheque adds a fee: off, unless the client sets a charge
/// type and an amount.
pub const BOUNCE_FEE: bool = false;

/// Whether the cheques collected by drivers and sales reps are handed over per
/// person per day, cheques only, and reach the deposit list only after another
/// person counts them (`FIN.CSH.01`, `FIN.ACL.20`).
pub const FIELD_CHEQUE_HANDOVER: bool = true;

/// The cash shortage a user may write off without an approval, in fils: AED 0.
pub const CASH_WRITE_OFF_LIMIT: i64 = 0;

/// Whether a shortage may be recovered from the salary (`FIN.ACL.19`): off.
pub const SALARY_RECOVERY: bool = false;

/// The role told of a cash shortage and who decides it.
pub const CASH_MANAGER_ROLE: &str = BRANCH_MANAGER_ROLE;

/// The counter keeps till sessions (`FIN.CSH.01`).
pub const COUNTER_TILL_SESSIONS: bool = true;

// ---- Sales (08 §3.3, §7.1 settings): no module setting yet ----

/// The discount above which an order or sale needs the branch manager
/// (`SAL.ACL.02`), in percent.
pub const DISCOUNT_LIMIT_PERCENT: i64 = 5;

/// The role that approves a discount above the limit.
pub const DISCOUNT_APPROVER_ROLE: &str = BRANCH_MANAGER_ROLE;

/// The days past terms after which an order on account is held (`SAL.ACL.03`),
/// per customer and credit group.
pub const CREDIT_GRACE_DAYS: i64 = 30;

/// Whether the bounced cheque of a company holds the open orders of its whole
/// credit group (`SAL.RUL.05`).
pub const BOUNCED_CHEQUE_HOLDS_GROUP: bool = true;

/// The outcome proposed for a line cancelled after picking (`SAL.ACL.05`).
pub const CANCEL_AFTER_PICKING_OUTCOME: &str = "return to stock";

/// The role that approves a cancellation after picking.
pub const CANCEL_AFTER_PICKING_APPROVER: &str = BRANCH_MANAGER_ROLE;

/// The fee for a cancellation after picking, in fils: none by default.
pub const CANCEL_AFTER_PICKING_FEE: i64 = 0;

/// Whether the credit is checked again at dispatch and at counter collection
/// for orders on account.
pub const CREDIT_RECHECK_AT_DISPATCH: bool = true;

/// Whether the driver may accept a parts delivery in part at proof of
/// delivery.
pub const PARTIAL_ACCEPTANCE_AT_POD: bool = true;

// ---- Stock (08 §3.3, §4.3, §7.2) ----

/// The value of a stock adjustment above which a second person approves, in
/// fils (`STK.ACL.02`): AED 1,000.
pub const STOCK_ADJUSTMENT_APPROVAL_FILS: i64 = 100_000;

/// The value of a Return to supplier above which a second person approves, in
/// fils (`STK.ACL.06`): AED 5,000.
pub const RETURN_APPROVAL_FILS: i64 = 500_000;

/// The role that approves a return to supplier above the limit.
pub const RETURN_APPROVER_ROLE: &str = PURCHASER_ROLE;

/// The days a Return to supplier waits before it is chased (`STK.RUL.08`).
pub const RETURN_CHASE_DAYS: i64 = 30;

/// Whether `STK.RUL.02` (stock below minimum) is on.
pub const LOW_STOCK_ALERT: bool = true;

/// The months a supplier scorecard covers (`STK.SUP.01`).
pub const SCORECARD_WINDOW_MONTHS: u32 = 3;

/// The fewest receipts in the window that give a score.
pub const SCORECARD_MIN_RECEIPTS: i64 = 3;

/// The grace days of a receipt.
pub const SCORECARD_GRACE_DAYS: i64 = 1;

/// The score under which `STK.RUL.09` gives the purchaser a review task.
pub const SCORECARD_THRESHOLD: i64 = 70;

/// Whether changing a part's preferred supplier needs a second person
/// (`STK.ACL.07`).
pub const PREFERRED_CHANGE_NEEDS_APPROVAL: bool = true;

/// The role told when a score falls under the threshold, besides the
/// purchaser.
pub const SCORECARD_TOLD_ROLE: &str = BRANCH_MANAGER_ROLE;

// ---- Cash-flow forecast and budget (08 §5, §7.1 settings) ----

/// The projected balance below which `FIN.RUL.23` alerts, in fils: AED
/// 100,000, in any of the next 13 weeks.
pub const CASH_THRESHOLD: i64 = 10_000_000;

/// The weeks the cash-flow forecast looks ahead (`FIN.CFL.01`).
pub const CASH_FORECAST_WEEKS: u32 = 13;

/// When the daily cash-flow check runs (`FIN.RUL.23`), Asia/Dubai.
pub const CASH_ALERT_TIME: &str = "06:00";

/// The roles `FIN.RUL.23` alerts: the Owner / GM and the accountant.
pub const CASH_ALERT_ROLES: [&str; 2] = [OWNER_ROLE, ACCOUNTANT_ROLE];

/// The share of a month's budget line at which `FIN.RUL.24` warns, in percent.
pub const BUDGET_WARNING_PERCENT: u32 = 100;

/// The owner role of every budget category (`FIN.BUD.01`): the accountant.
pub const BUDGET_CATEGORY_OWNER: &str = ACCOUNTANT_ROLE;

/// Who approves a budget: the Owner / GM.
pub const BUDGET_APPROVER_ROLE: &str = OWNER_ROLE;

/// Who approves an owner drawing, whatever its amount (`FIN.ACL.21`, limit AED
/// 0): the Owner / GM, never the requester.
pub const DRAWINGS_APPROVER_ROLE: &str = OWNER_ROLE;

/// Who is told, besides, when a late entry above 24 hours waits (`PLT.ACL.10`).
pub const LATE_ENTRY_APPROVER_ROLES: [&str; 2] = [WORKSHOP_MANAGER_ROLE, BRANCH_MANAGER_ROLE];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_write_off_and_drawing_is_an_approval() {
        assert_eq!(WRITE_OFF_LIMIT, 0);
        assert_eq!(CASH_WRITE_OFF_LIMIT, 0);
        assert!(!WRITE_OFF_APPROVERS.contains(&ACCOUNTANT_ROLE));
        assert_eq!(DRAWINGS_APPROVER_ROLE, "owner");
    }

    #[test]
    fn the_stock_limits_are_in_fils() {
        assert_eq!(STOCK_ADJUSTMENT_APPROVAL_FILS, 100_000);
        assert_eq!(RETURN_APPROVAL_FILS, 500_000);
        assert_eq!(CASH_THRESHOLD, 10_000_000);
    }
}
