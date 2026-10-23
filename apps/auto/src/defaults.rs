//! The 08 Auto values for the shared negative-flow mechanisms that are not
//! about money (08 §4.1, §7.1 *Negative-flow settings*, *Notification types
//! registered by 08*, §7.2 *Settings for the shared sales, finance, staff and
//! document capabilities*, §8, §9; BL-AUT-18).
//!
//! The mechanisms live in the shared modules (`wl-documents`, `wl-scheduling`,
//! `wl-notify`, `wl-sales`, `wl-hr`, `wl-kernel`); this file holds the numbers
//! and names of the 08 table and the host turns them into the owning modules'
//! own calls (`host/src/app/auto_settings.rs`). The app depends on no crate
//! that owns those tables.
//!
//! What the earlier issues already seeded is not repeated here: the document
//! kinds with their `blocks`, override modes and responsible roles
//! ([`crate::seed`], `host/src/app/auto_seed.rs`), the scheduling numbers, the
//! block effects of the agreements ([`crate::blocked`]) and the compliance
//! critical rules the app owns (`AUT.RUL.15`, `AUT.RUL.16`).

use wl_kernel::notify::{GROUP_INVOICES, GiveUpReaction};

use crate::permissions::{
    ACCOUNTANT_ROLE, BRANCH_MANAGER_ROLE, COUNTER_SALES_ROLE, OWNER_ROLE, SERVICE_ADVISOR_ROLE,
    STOREKEEPER_ROLE, TECHNICIAN_ROLE, WORKSHOP_MANAGER_ROLE,
};

// ---- Notifications (08 §7.1 *Notification types registered by 08*) ----

/// The reactions when an invoice, a statement or a payment reminder gives up:
/// the contact is marked invalid, the other contact is tried, then the
/// accountant gets a task.
pub const INVOICE_GIVE_UP: [GiveUpReaction; 3] = [
    GiveUpReaction::MarkContactInvalid,
    GiveUpReaction::AlternativeRecipient,
    GiveUpReaction::StaffTask,
];

/// The role that gets the task of an invoice message that gave up.
pub const INVOICE_TASK_ROLE: &str = ACCOUNTANT_ROLE;

/// How a notification type of a shared module is configured for 08.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeSpec {
    /// The type's key.
    pub key: &'static str,
    /// The group (the row of the wl-notify §5.4 table).
    pub group: &'static str,
    /// What happens on give-up.
    pub give_up: &'static [GiveUpReaction],
    /// The staff role that gets the task.
    pub role: Option<&'static str>,
}

/// The types of the shared modules whose stored rows the host sets for 08: the
/// invoice reminders and the returned payment go to the accountant when they
/// give up. (The types 08 registers itself carry theirs in [`crate::notices`];
/// none of the shared ones is statutory.)
pub const NOTIFICATION_TYPES: [TypeSpec; 3] = [
    TypeSpec {
        key: "invoice.reminder_before_due",
        group: GROUP_INVOICES,
        give_up: &INVOICE_GIVE_UP,
        role: Some(INVOICE_TASK_ROLE),
    },
    TypeSpec {
        key: "invoice.reminder_after_due",
        group: GROUP_INVOICES,
        give_up: &INVOICE_GIVE_UP,
        role: Some(INVOICE_TASK_ROLE),
    },
    TypeSpec {
        key: "payments.payment_returned",
        group: GROUP_INVOICES,
        give_up: &INVOICE_GIVE_UP,
        role: Some(INVOICE_TASK_ROLE),
    },
];

/// The most marketing messages one contact gets in 7 days (`NTF.RUL.02`).
pub const MARKETING_CAP: i64 = 2;

/// A broadcast to more than this many recipients needs a second approver
/// (`NTF.ACL.03`).
pub const BROADCAST_APPROVAL_ABOVE: i64 = 200;

/// The marketing send window, minutes after midnight Asia/Dubai: 07:00 to
/// 21:00 (`NTF.RUL.02`).
pub const SEND_WINDOW_MINUTES: (i64, i64) = (7 * 60, 21 * 60);

// ---- Late entry (08 §7.1 *Late entry*, `PLT.ACL.10`) ----

/// The hours a test drive, a job card time or a delivery may be entered late
/// before the entry waits for the workshop or branch manager.
pub const LATE_ENTRY_APPROVAL_HOURS: i64 = 24;

// ---- Alerts (08 §7.2 *Compliance-critical rules*, `PLT.RUL.04`) ----

/// The compliance-critical rules (§7.2): a run that fails alerts the Owner /
/// GM and the company admin, and nothing switches that off. Both are marked by
/// the app.
pub const CRITICAL_RULES: [&str; 2] = ["AUT.RUL.15", "AUT.RUL.16"];

/// The owner role alerted when a compliance-critical rule fails.
pub const ALERT_ROLE: &str = OWNER_ROLE;

// ---- Sales and loyalty (08 §5, §7.1 settings) ----

/// The lead sources 08 uses: eight of the module's nine (*aggregator* is not
/// used and is turned off); *partner* is insurers and fleet brokers.
pub const LEAD_SOURCES: &[&str] = &[
    "referral", "walk_in", "website", "social", "paid_ads", "partner", "event", "other",
];

/// The lead sources that require a campaign (`SAL.CRM.02`): none in 08.
pub const CAMPAIGN_REQUIRED_SOURCES: &[&str] = &[];

/// The campaign channels 08 uses, as (name in the spec, the module's marketing
/// channel it is recorded as): the module's list is fixed.
pub const CAMPAIGN_CHANNELS: [(&str, &str); 5] = [
    ("Search ads", "google"),
    ("Instagram", "meta"),
    ("SMS", "sms"),
    ("Email", "email"),
    ("Trade events", "event"),
];

/// The days before a loyalty lot expires that its member is reminded
/// (`SAL.RUL.08`).
pub const LOYALTY_REMINDER_DAYS: i64 = 30;

/// The points above which a manual adjustment needs a manager (`SAL.ACL.07`).
pub const LOYALTY_ADJUSTMENT_LIMIT: i64 = 500;

/// The points `SAL.RUL.07` earns for every [`LOYALTY_EARN_PER_AED`] dirhams of
/// workshop invoices and counter sales to an individual car owner: 1 point per
/// AED 10 (a company, fleet or insurer-paid job has no loyalty account).
pub const LOYALTY_POINTS: i64 = 1;
/// The dirhams that earn [`LOYALTY_POINTS`].
pub const LOYALTY_EARN_PER_AED: i64 = 10;
/// What one point is worth, in fils: 100 points are AED 10.
pub const LOYALTY_POINT_VALUE_FILS: i64 = 10;
/// The fewest points that can be redeemed: 100 points, AED 10.
pub const LOYALTY_MIN_REDEEM: i64 = 100;
/// The months points are valid after they are earned.
pub const LOYALTY_VALIDITY_MONTHS: i64 = 24;
/// The name of the programme.
pub const LOYALTY_NAME: &str = "Rewards";

// ---- Connections (08 §7.1 *Integrations*, §8, `PLT.RUL.05`) ----

/// The screen of the job cards: where the advisor calls the customer.
pub const JOB_CARDS_SCREEN: &str = "auto.job_cards";

/// What the advisors do while the SMS gateway is down.
pub const SMS_FALLBACK_EN: &str = "Call the customer yourself; the messages are queued and sent when the SMS gateway recovers.";
/// The same, in Arabic.
pub const SMS_FALLBACK_AR: &str =
    "اتصل بالعميل مباشرة؛ تبقى الرسائل في الانتظار وتُرسل عند عودة بوابة الرسائل.";

// ---- Onboarding checklists (HRM.ONB.01, HRM.RUL.16, HRM.RUL.17) ----

/// One step of an onboarding checklist: title, owner (`hr`, `manager`,
/// `employee` or `role:<key>`), due days after the start and kind (`task`,
/// `sops` or `checklist:<template>`).
pub type StepSpec = (&'static str, &'static str, i64, &'static str);

/// An onboarding checklist for one job role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnboardingSpec {
    /// The template's name.
    pub name: &'static str,
    /// The job role it applies to.
    pub role: &'static str,
    /// The steps, in order.
    pub steps: &'static [StepSpec],
}

/// The job role of a delivery driver.
pub const DRIVER_ROLE: &str = "driver";

/// The employee uploads what they hold; the employee's own steps are recorded
/// only by them (`HRM.ACL.10`).
const DOCUMENTS_STEP: StepSpec = (
    "Upload your Emirates ID and your documents",
    "employee",
    3,
    "task",
);
/// The employee reads the SOPs and acknowledges them.
const SOPS_STEP: StepSpec = ("SOPs acknowledged", "employee", 7, "sops");
/// The line manager's induction.
const LINE_MANAGER_STEP: StepSpec = ("Induction by your line manager", "manager", 3, "task");

/// The onboarding checklists 08 seeds: technician, service advisor, counter
/// sales, storekeeper and delivery driver. Tasks go to the branch manager (the
/// workshop manager for workshop roles), the line manager and the employee.
/// Each ends with the SOPs; completion blocks nothing (the role is active
/// before the checklist is complete).
pub const ONBOARDING: [OnboardingSpec; 5] = [
    OnboardingSpec {
        name: "Technician onboarding",
        role: TECHNICIAN_ROLE,
        steps: &[
            (
                "System account and workshop access",
                "role:workshop_manager",
                1,
                "task",
            ),
            (
                "Bay, tools and safety induction",
                "role:workshop_manager",
                3,
                "task",
            ),
            LINE_MANAGER_STEP,
            DOCUMENTS_STEP,
            SOPS_STEP,
        ],
    },
    OnboardingSpec {
        name: "Service advisor onboarding",
        role: SERVICE_ADVISOR_ROLE,
        steps: &[
            (
                "System account and access",
                "role:workshop_manager",
                1,
                "task",
            ),
            (
                "Check-in, estimates and delivery induction",
                "role:workshop_manager",
                3,
                "task",
            ),
            LINE_MANAGER_STEP,
            DOCUMENTS_STEP,
            SOPS_STEP,
        ],
    },
    OnboardingSpec {
        name: "Counter sales onboarding",
        role: COUNTER_SALES_ROLE,
        steps: &[
            (
                "System account and till access",
                "role:branch_manager",
                1,
                "task",
            ),
            (
                "Parts counter, cash and returns induction",
                "role:branch_manager",
                3,
                "task",
            ),
            LINE_MANAGER_STEP,
            DOCUMENTS_STEP,
            SOPS_STEP,
        ],
    },
    OnboardingSpec {
        name: "Storekeeper onboarding",
        role: STOREKEEPER_ROLE,
        steps: &[
            (
                "System account and store access",
                "role:branch_manager",
                1,
                "task",
            ),
            (
                "Receiving, put-away and stock counts induction",
                "role:branch_manager",
                3,
                "task",
            ),
            LINE_MANAGER_STEP,
            DOCUMENTS_STEP,
            SOPS_STEP,
        ],
    },
    OnboardingSpec {
        name: "Delivery driver onboarding",
        role: DRIVER_ROLE,
        steps: &[
            (
                "System account and driver app",
                "role:branch_manager",
                1,
                "task",
            ),
            (
                "Parts deliveries, proof of delivery and cheques induction",
                "role:branch_manager",
                3,
                "task",
            ),
            LINE_MANAGER_STEP,
            DOCUMENTS_STEP,
            SOPS_STEP,
        ],
    },
];

/// An SOP of the library with its owner role and required readers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SopSpec {
    /// The code.
    pub code: &'static str,
    /// The English title.
    pub title: &'static str,
    /// The category.
    pub category: &'static str,
    /// The role whose holders accept and review it.
    pub owner_role: &'static str,
    /// The roles that must read and acknowledge it (per branch).
    pub readers: &'static [&'static str],
}

/// The SOP library 08 seeds: the workshop manager owns check-in, quality
/// control and workshop safety; the branch manager owns the counter, cash and
/// returns. `DOC.RUL.04` asks each reader to acknowledge within 7 days,
/// reminds every 3 days and tells the reader's manager and the SOP's owner
/// role after 7 days overdue (the module's defaults); a new version asks for a
/// new acknowledgement.
pub const SOPS: [SopSpec; 6] = [
    SopSpec {
        code: "SOP-AUT-CHECKIN",
        title: "Vehicle check-in and inspection",
        category: "check_in",
        owner_role: WORKSHOP_MANAGER_ROLE,
        readers: &[
            SERVICE_ADVISOR_ROLE,
            TECHNICIAN_ROLE,
            WORKSHOP_MANAGER_ROLE,
        ],
    },
    SopSpec {
        code: "SOP-AUT-QC",
        title: "Quality control before delivery",
        category: "quality_control",
        owner_role: WORKSHOP_MANAGER_ROLE,
        readers: &[TECHNICIAN_ROLE, WORKSHOP_MANAGER_ROLE],
    },
    SopSpec {
        code: "SOP-AUT-SAFETY",
        title: "Workshop safety",
        category: "safety",
        owner_role: WORKSHOP_MANAGER_ROLE,
        readers: &[
            TECHNICIAN_ROLE,
            SERVICE_ADVISOR_ROLE,
            STOREKEEPER_ROLE,
            WORKSHOP_MANAGER_ROLE,
        ],
    },
    SopSpec {
        code: "SOP-AUT-COUNTER",
        title: "Parts counter: quotes, orders and collection",
        category: "counter",
        owner_role: BRANCH_MANAGER_ROLE,
        readers: &[COUNTER_SALES_ROLE, BRANCH_MANAGER_ROLE],
    },
    SopSpec {
        code: "SOP-AUT-CASH",
        title: "Cash, till sessions and cheques",
        category: "cash",
        owner_role: BRANCH_MANAGER_ROLE,
        readers: &[COUNTER_SALES_ROLE, DRIVER_ROLE, BRANCH_MANAGER_ROLE],
    },
    SopSpec {
        code: "SOP-AUT-RETURNS",
        title: "Returns, warranty and supplier claims",
        category: "returns",
        owner_role: BRANCH_MANAGER_ROLE,
        readers: &[COUNTER_SALES_ROLE, STOREKEEPER_ROLE, BRANCH_MANAGER_ROLE],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lead_sources_are_the_modules_without_the_aggregator() {
        assert_eq!(LEAD_SOURCES.len(), 8);
        assert!(!LEAD_SOURCES.contains(&"aggregator"));
        assert!(LEAD_SOURCES.contains(&"partner"));
        assert!(CAMPAIGN_REQUIRED_SOURCES.is_empty());
    }

    #[test]
    fn a_hundred_points_are_ten_dirhams_and_expire_after_two_years() {
        assert_eq!(LOYALTY_MIN_REDEEM * LOYALTY_POINT_VALUE_FILS, 1_000);
        assert_eq!(LOYALTY_VALIDITY_MONTHS, 24);
    }

    #[test]
    fn every_checklist_gives_tasks_to_a_manager_the_line_manager_and_the_employee() {
        assert_eq!(ONBOARDING.len(), 5);
        for spec in &ONBOARDING {
            assert!(spec.steps.iter().any(|s| s.1.starts_with("role:")));
            assert!(spec.steps.iter().any(|s| s.1 == "manager"));
            assert!(spec.steps.iter().any(|s| s.1 == "employee"));
            assert_eq!(spec.steps.last().map(|s| s.3), Some("sops"));
        }
        // The workshop manager gets the tasks of the workshop roles.
        for spec in &ONBOARDING[..2] {
            assert!(spec.steps.iter().any(|s| s.1 == "role:workshop_manager"));
        }
    }

    #[test]
    fn the_sop_owners_are_the_workshop_manager_and_the_branch_manager() {
        for sop in &SOPS {
            assert!(
                sop.owner_role == WORKSHOP_MANAGER_ROLE || sop.owner_role == BRANCH_MANAGER_ROLE,
                "{}",
                sop.code
            );
            assert!(sop.readers.contains(&sop.owner_role));
        }
        assert_eq!(SOPS.iter().filter(|s| s.owner_role == "workshop_manager").count(), 3);
    }
}
