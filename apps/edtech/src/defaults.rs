//! The 10 EdTech values for the shared negative-flow mechanisms that are not
//! about fees (10 §4.1, §7.1 *Booking*, §7.2 *Notification types* and
//! *Settings for the shared sales, finance, staff and document capabilities*,
//! §8, §9; BL-EDU-17).
//!
//! The mechanisms live in the shared modules (`wl-documents`, `wl-scheduling`,
//! `wl-parties`, `wl-notify`, `wl-sales`, `wl-hr`, `wl-kernel`); this file holds
//! the numbers and names of the 10 table and the host turns them into the
//! owning modules' own calls (`host/src/app/edtech_settings.rs`). The app
//! depends on no crate that owns those tables.
//!
//! What the earlier issues already seeded is not repeated here: the document
//! kinds with their `blocks`, override modes, look-ahead windows and
//! responsible roles ([`crate::seed`], `host/src/app/edtech_seed.rs`), the
//! scheduling numbers (D = 30, H = 2), the block effect of an enrolment
//! ([`crate::blocked`]) and the two compliance-critical rules the app owns
//! (`EDU.RUL.12`, `EDU.RUL.13`).

use wl_kernel::COMPANY_ADMIN_ROLE;
use wl_kernel::approvals::PolicyDefault;
use wl_kernel::notify::{
    GROUP_GUARDIAN, GROUP_INVOICES, GROUP_SIGNATURES, GiveUpReaction, NotifyChannel,
};

use crate::permissions::{
    ACCOUNTANT_ROLE, CENTRE_MANAGER_ROLE, COORDINATOR_ROLE, COUNSELLOR_ROLE, OWNER_ROLE,
    TUTOR_ROLE,
};

// ---- Documents (10 §9) ----

/// The approval action of a one-use override of a document block
/// (`DOC.ACL.02`, wl-documents).
pub const OVERRIDE_ACTION: &str = "documents.block_override";

/// Hours an override request may wait before it can expire (`DOC.ACL.02`).
pub const OVERRIDE_REQUEST_HOURS: i64 = 72;

/// The approval of an override in 10: the company owner decides, never the
/// requester, with the company admin as the fallback. Only the police
/// clearance can be switched to `with_approval`: the centre permit, the course
/// approval and the tutor permit or freelance visa are fixed to `never`.
pub fn override_policy() -> PolicyDefault {
    PolicyDefault::new(OVERRIDE_ACTION)
        .step([OWNER_ROLE, COMPANY_ADMIN_ROLE])
        .expiry_hours(Some(OVERRIDE_REQUEST_HOURS))
}

// ---- Notifications (10 §7.2 *Notification types*, `NTF.RUL.01`) ----

/// The group of the messages to students and guardians (*Guardian messages*).
pub const GUARDIAN_GROUP: &str = GROUP_GUARDIAN;

/// The reactions when a message to a guardian or a student gives up: the other
/// guardian first, then a task to the counsellor. An adult student without a
/// guardian has no other contact, so the task comes at once.
pub const GUARDIAN_GIVE_UP: [GiveUpReaction; 2] = [
    GiveUpReaction::AlternativeRecipient,
    GiveUpReaction::StaffTask,
];

/// The role that gets the task of a guardian message that gave up.
pub const GUARDIAN_TASK_ROLE: &str = COUNSELLOR_ROLE;

/// The channels of a message to a student or a guardian: WhatsApp, then e-mail
/// (§8: e-mail instead of WhatsApp while WhatsApp is down).
pub const FAMILY_CHANNELS: [NotifyChannel; 2] = [NotifyChannel::Whatsapp, NotifyChannel::Email];

/// The group of terms, refund policy and guardian consent requests (*Portal
/// invitations, document requests, signature requests*).
pub const REQUEST_GROUP: &str = GROUP_SIGNATURES;

/// The reactions when such a request gives up: the contact is marked invalid
/// and the counsellor gets a task.
pub const REQUEST_GIVE_UP: [GiveUpReaction; 2] =
    [GiveUpReaction::MarkContactInvalid, GiveUpReaction::StaffTask];

/// The role that gets the task of a request that gave up.
pub const REQUEST_TASK_ROLE: &str = COUNSELLOR_ROLE;

/// The reactions when an invoice, a membership charge or a reminder gives up:
/// the contact is marked invalid, the other guardian or payer contact is tried,
/// then the accountant gets a task.
pub const INVOICE_GIVE_UP: [GiveUpReaction; 3] = [
    GiveUpReaction::MarkContactInvalid,
    GiveUpReaction::AlternativeRecipient,
    GiveUpReaction::StaffTask,
];

/// The role that gets the task of an invoice message that gave up.
pub const INVOICE_TASK_ROLE: &str = ACCOUNTANT_ROLE;

/// How a notification type of a shared module is configured for 10.
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

/// The types of the shared modules whose stored rows the host sets for 10: the
/// invoice reminders and the membership notices go to the accountant when they
/// give up. (The types 10 registers itself carry theirs in [`crate::notices`];
/// the booking types keep the wl-scheduling group; none is statutory.)
pub const NOTIFICATION_TYPES: [TypeSpec; 4] = [
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
        key: "invoicing.subscription_past_due",
        group: GROUP_INVOICES,
        give_up: &INVOICE_GIVE_UP,
        role: Some(INVOICE_TASK_ROLE),
    },
    TypeSpec {
        key: "invoicing.subscription_retry",
        group: GROUP_INVOICES,
        give_up: &INVOICE_GIVE_UP,
        role: Some(INVOICE_TASK_ROLE),
    },
];

/// The most marketing messages one recipient gets in 7 days (`NTF.RUL.02`).
pub const MARKETING_CAP: i64 = 2;

/// A broadcast to more than this many recipients needs a second approver
/// (`NTF.ACL.03`).
pub const BROADCAST_APPROVAL_ABOVE: i64 = 200;

/// The marketing send window, minutes after midnight Asia/Dubai: 07:00 to
/// 21:00 (`NTF.RUL.02`).
pub const SEND_WINDOW_MINUTES: (i64, i64) = (7 * 60, 21 * 60);

// ---- Alerts (10 §7.2 *Compliance-critical*, `PLT.RUL.04`) ----

/// The compliance-critical rules (§7.2): a run that fails alerts the company
/// owner and the company admin, and nothing switches that off. `EDU.RUL.12`
/// and `EDU.RUL.13` are marked by the app, `DOC.RUL.03` by the host.
pub const CRITICAL_RULES: [&str; 3] = ["DOC.RUL.03", "EDU.RUL.12", "EDU.RUL.13"];

/// The owner role alerted when a compliance-critical rule fails: in 10 the
/// company owner.
pub const ALERT_ROLE: &str = OWNER_ROLE;

// ---- Sales and loyalty (10 §5, §7.2 settings) ----

/// The lead sources 10 uses: eight of the module's nine (*aggregator* is not
/// used and is turned off); *event* is school fairs, *partner* schools and
/// corporates.
pub const LEAD_SOURCES: &[&str] = &[
    "referral", "walk_in", "website", "social", "paid_ads", "event", "partner", "other",
];

/// The lead sources that require a campaign (`SAL.CRM.02`): none in 10.
pub const CAMPAIGN_REQUIRED_SOURCES: &[&str] = &[];

/// The campaign channels 10 uses, as (name in the spec, the module's marketing
/// channel it is recorded as): the module's list is fixed.
pub const CAMPAIGN_CHANNELS: [(&str, &str); 6] = [
    ("Instagram", "meta"),
    ("TikTok", "tiktok"),
    ("Search ads", "google"),
    ("WhatsApp", "whatsapp"),
    ("Email", "email"),
    ("School and community events", "event"),
];

/// The days before a loyalty lot expires that its member is reminded
/// (`SAL.RUL.08`).
pub const LOYALTY_REMINDER_DAYS: i64 = 30;

/// The points above which a manual adjustment needs a manager (`SAL.ACL.07`).
pub const LOYALTY_ADJUSTMENT_LIMIT: i64 = 500;

/// The points `SAL.RUL.07` earns for every [`LOYALTY_EARN_PER_AED`] dirhams of
/// fees paid by the student or guardian: 1 point per AED 10 (corporate-paid
/// fees earn none: a company has no loyalty account).
pub const LOYALTY_POINTS: i64 = 1;
/// The dirhams that earn [`LOYALTY_POINTS`].
pub const LOYALTY_EARN_PER_AED: i64 = 10;
/// What one point is worth, in fils: 100 points are AED 10.
pub const LOYALTY_POINT_VALUE_FILS: i64 = 10;
/// The fewest points that can be redeemed: 100 points, AED 10.
pub const LOYALTY_MIN_REDEEM: i64 = 100;
/// The largest part of a fee that points may pay, in basis points: 10%.
pub const LOYALTY_MAX_SHARE_BPS: i64 = 1_000;
/// The months points are valid after they are earned.
pub const LOYALTY_VALIDITY_MONTHS: i64 = 12;
/// The name of the programme.
pub const LOYALTY_NAME: &str = "Rewards";

// ---- Connections (10 §8 *When an integration is down*, `PLT.RUL.05`) ----

/// The key of the attendance connection: the tutor's attendance entry. The MVP
/// has no integration behind it; an administrator can take it out of service
/// by hand, and the desk and the session page then show the paper fallback.
pub const ATTENDANCE_CONNECTION: &str = "edtech.attendance";

/// The screen that lists the enrolments: where the desk collects a fee.
pub const DESK_SCREEN: &str = "edtech.enrolments";
/// The screen of the money to collect after a chargeback.
pub const COLLECT_SCREEN: &str = "edtech.collect";
/// The screen of one session: where the attendance is entered.
pub const SESSION_SCREEN: &str = "edtech.session";

/// What the desk does while the payment gateway is down.
pub const GATEWAY_FALLBACK_EN: &str = "Take the fee on the desk card terminal or in cash with a receipt and record it; the payment links are sent again when the gateway recovers.";
/// The same, in Arabic.
pub const GATEWAY_FALLBACK_AR: &str = "استلم الرسوم عبر جهاز البطاقات في المكتب أو نقدًا مع إيصال وسجّلها؛ تُرسل روابط الدفع مرة أخرى عند عودة البوابة.";
/// What staff do while WhatsApp is down.
pub const WHATSAPP_FALLBACK_EN: &str =
    "Messages to students and guardians go by e-mail instead of WhatsApp.";
/// The same, in Arabic.
pub const WHATSAPP_FALLBACK_AR: &str =
    "تُرسل الرسائل إلى الطلاب وأولياء الأمور بالبريد الإلكتروني بدلًا من واتساب.";
/// What staff do while e-mail is down.
pub const EMAIL_FALLBACK_EN: &str = "Receipts and invoices stay visible in the portal; give the student or guardian a printed copy at the desk.";
/// The same, in Arabic.
pub const EMAIL_FALLBACK_AR: &str =
    "تبقى الإيصالات والفواتير ظاهرة في البوابة؛ سلّم الطالب أو ولي الأمر نسخة مطبوعة عند الاستقبال.";
/// What the tutor and the coordinator do while attendance entry is down.
pub const ATTENDANCE_FALLBACK_EN: &str = "Take the attendance on paper; the tutor or the coordinator enters it later (a reason is asked once the tutor's 24 hours have passed).";
/// The same, in Arabic.
pub const ATTENDANCE_FALLBACK_AR: &str = "سجّل الحضور على الورق؛ يدخله المدرّس أو المنسّق لاحقًا (يُطلب سبب بعد انقضاء 24 ساعة على المدرّس).";

/// Hours after the end of a session during which the tutor enters or corrects
/// the attendance without a reason (`EDU.ACL.16`); after it only the
/// coordinator may, with a reason.
pub const TUTOR_ATTENDANCE_HOURS: i64 = crate::session_machine::TUTOR_WINDOW_HOURS;

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

/// The first step of every checklist: the centre manager sets up the account
/// and the access.
const ACCOUNT_STEP: StepSpec = (
    "System account and access",
    "role:centre_manager",
    1,
    "task",
);
/// The academic coordinator's induction.
const COORDINATOR_STEP: StepSpec = (
    "Courses, batches and timetable induction",
    "role:academic_coordinator",
    3,
    "task",
);
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

/// The job role of a tutor or trainer.
pub const TUTOR_JOB_ROLE: &str = TUTOR_ROLE;

/// The onboarding checklists 10 seeds: tutor / trainer, admissions counsellor
/// and academic coordinator. Tasks go to the centre manager, the academic
/// coordinator and the employee. Each ends with the SOPs; completion blocks
/// nothing (the role is active before the checklist is complete, and sessions
/// stay guarded by the tutor permits and the police clearance).
pub const ONBOARDING: [OnboardingSpec; 3] = [
    OnboardingSpec {
        name: "Tutor and trainer onboarding",
        role: TUTOR_JOB_ROLE,
        steps: &[
            ACCOUNT_STEP,
            COORDINATOR_STEP,
            (
                "Police clearance and tutor permit recorded",
                "role:academic_coordinator",
                1,
                "task",
            ),
            DOCUMENTS_STEP,
            SOPS_STEP,
        ],
    },
    OnboardingSpec {
        name: "Admissions counsellor onboarding",
        role: COUNSELLOR_ROLE,
        steps: &[
            ACCOUNT_STEP,
            (
                "Front desk, enrolments and fees",
                "role:centre_manager",
                3,
                "checklist:counsellor_induction",
            ),
            DOCUMENTS_STEP,
            SOPS_STEP,
        ],
    },
    OnboardingSpec {
        name: "Academic coordinator onboarding",
        role: COORDINATOR_ROLE,
        steps: &[
            ACCOUNT_STEP,
            (
                "Centre, rooms and tutors",
                "role:centre_manager",
                3,
                "checklist:coordinator_induction",
            ),
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
    /// The roles that must read and acknowledge it (per campus).
    pub readers: &'static [&'static str],
}

/// The SOP library 10 seeds: the academic coordinator owns teaching,
/// assessment and safeguarding; the centre manager owns the front desk and
/// refunds. `DOC.RUL.04` asks each reader to acknowledge within 7 days,
/// reminds every 3 days and tells the reader's manager and the SOP's owner
/// role after 7 days overdue (the module's defaults); a new version asks for
/// a new acknowledgement.
pub const SOPS: [SopSpec; 5] = [
    SopSpec {
        code: "SOP-EDU-TEACHING",
        title: "Teaching: planning, delivery and attendance",
        category: "teaching",
        owner_role: COORDINATOR_ROLE,
        readers: &[TUTOR_ROLE, COORDINATOR_ROLE],
    },
    SopSpec {
        code: "SOP-EDU-ASSESSMENT",
        title: "Assessment and certificates",
        category: "assessment",
        owner_role: COORDINATOR_ROLE,
        readers: &[TUTOR_ROLE, COORDINATOR_ROLE],
    },
    SopSpec {
        code: "SOP-EDU-SAFEGUARDING",
        title: "Safeguarding of minors",
        category: "safeguarding",
        owner_role: COORDINATOR_ROLE,
        readers: &[
            TUTOR_ROLE,
            COORDINATOR_ROLE,
            COUNSELLOR_ROLE,
            CENTRE_MANAGER_ROLE,
        ],
    },
    SopSpec {
        code: "SOP-EDU-FRONT-DESK",
        title: "Front desk: enquiries, trials and enrolments",
        category: "front_desk",
        owner_role: CENTRE_MANAGER_ROLE,
        readers: &[COUNSELLOR_ROLE, CENTRE_MANAGER_ROLE],
    },
    SopSpec {
        code: "SOP-EDU-REFUNDS",
        title: "Fees, refunds and withdrawals",
        category: "refunds",
        owner_role: CENTRE_MANAGER_ROLE,
        readers: &[COUNSELLOR_ROLE, CENTRE_MANAGER_ROLE],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lead_sources_are_the_modules_without_the_aggregator() {
        assert_eq!(LEAD_SOURCES.len(), 8);
        assert!(!LEAD_SOURCES.contains(&"aggregator"));
        assert!(CAMPAIGN_REQUIRED_SOURCES.is_empty());
    }

    #[test]
    fn a_hundred_points_are_ten_dirhams() {
        assert_eq!(LOYALTY_MIN_REDEEM * LOYALTY_POINT_VALUE_FILS, 1_000);
        assert_eq!(LOYALTY_MAX_SHARE_BPS, 1_000);
    }

    #[test]
    fn every_checklist_gives_tasks_to_the_manager_and_the_employee_and_ends_with_the_sops() {
        for spec in &ONBOARDING {
            assert!(spec.steps.iter().any(|s| s.1 == "role:centre_manager"));
            assert!(spec.steps.iter().any(|s| s.1 == "employee"));
            assert_eq!(spec.steps.last().map(|s| s.3), Some("sops"));
        }
        assert!(
            ONBOARDING[0]
                .steps
                .iter()
                .any(|s| s.1 == "role:academic_coordinator")
        );
    }

    #[test]
    fn the_sop_owners_are_the_coordinator_and_the_centre_manager() {
        for sop in &SOPS {
            assert!(
                sop.owner_role == COORDINATOR_ROLE || sop.owner_role == CENTRE_MANAGER_ROLE,
                "{}",
                sop.code
            );
            assert!(sop.readers.contains(&sop.owner_role) || sop.owner_role == CENTRE_MANAGER_ROLE);
        }
    }

    #[test]
    fn the_guardian_reactions_try_the_other_guardian_then_the_counsellor() {
        assert_eq!(
            GUARDIAN_GIVE_UP,
            [
                GiveUpReaction::AlternativeRecipient,
                GiveUpReaction::StaffTask
            ]
        );
        assert_eq!(GUARDIAN_TASK_ROLE, "counsellor");
        assert_eq!(INVOICE_TASK_ROLE, "accountant");
    }
}
