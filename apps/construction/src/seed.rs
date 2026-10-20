//! The 02 values for the shared money, procurement, labour and compliance
//! mechanisms (02 Construction §3.3, §4.2 to §4.4, §7.1, §7.2; BL-CON-17 and
//! BL-CON-18).
//!
//! The mechanisms live in the shared modules (`wl-invoicing`, `wl-payments`,
//! `wl-inventory`, `wl-attendance`, `wl-hr`, `wl-documents`, `wl-sales`,
//! `wl-ticketing`) and the vertical reaches them only through `wl-kernel`.
//! This file is therefore plain data, the one place where a 02 value is
//! written down: the host (`host/src/app/construction_seed.rs`) turns these
//! tables into the modules' own builder calls and seeds, and its settings
//! page reads them back. What this file does itself is mark the labour rules
//! compliance-critical for the owner alert ([`register_critical_rules`]).
//!
//! # Settings whose owning module does not expose them yet
//!
//! Recorded here, not built (the mechanism belongs to the module):
//!
//! - `FIN.ACL.15` limit of AED 0 is the module's own rule (every write-off
//!   waits); only the approvers are a setting ([`WRITE_OFF_APPROVERS`]).
//! - `FIN.RUL.14` relief, `FIN.RUL.16` e-invoice task and `FIN.ACL.21`
//!   drawings approval have no setting beyond the notified role; their 02
//!   value is the module's default and is read back, not set.
//! - Midday break window and season (`HRM.RUL.01`, `HRM.RUL.13`) have no
//!   builder: 12:30 to 15:00 from 15 June to 15 September is the module's
//!   default, which equals the 02 value.
//! - The HSE officer who is told of a midday flag is `hse_role`; the reviewer
//!   roles per flag (`HRM.ACL.09`), the adjustment origins (`HRM.ACL.08`) and
//!   the absconding approver (`HRM.ACL.07`) are the module's fixed rules.
//! - The campaign channels of `SAL.CRM.02` are a fixed list in wl-sales:
//!   the 02 names are mapped to the closest ([`CAMPAIGN_CHANNELS`]).
//! - The rejection reasons of the module are a superset of the four of 02
//!   (`temperature` and `short_shelf_life` cannot be removed): the four are
//!   set to need a photo, the rest stay unused.
//! - Read-only for clients (caps, break window, season: legal values) has no
//!   flag in the modules yet.
//! - The block effect `continue` for a blocked client is registered by
//!   [`crate::award::register_block_effect`] (BL-CON-03).

use wl_kernel::compliance::CriticalRule;

/// The managing director's role: the company admin.
pub const MD: &str = wl_kernel::COMPANY_ADMIN_ROLE;
/// The company admin (the owner role).
pub const COMPANY_ADMIN: &str = wl_kernel::COMPANY_ADMIN_ROLE;
/// The project manager.
pub const PROJECT_MANAGER: &str = crate::permissions::PROJECT_MANAGER_ROLE;
/// The accountant.
pub const ACCOUNTANT: &str = "accountant";
/// HR.
pub const HR: &str = "hr";
/// The HSE officer.
pub const HSE_OFFICER: &str = "hse";
/// The procurement officer.
pub const PROCUREMENT: &str = "procurement";
/// The supervisor.
pub const SUPERVISOR: &str = "supervisor";
/// The storekeeper.
pub const STOREKEEPER: &str = "storekeeper";
/// The quantity surveyor.
pub const QUANTITY_SURVEYOR: &str = crate::permissions::QS_ROLE;

// ---- BL-CON-17: money and procurement -------------------------------------

/// Item 1: who approves a write-off, never the requester. The limit is AED 0.
pub const WRITE_OFF_APPROVERS: [&str; 1] = [MD];
/// Item 2: the party portal dispute is off (the portal is Later).
pub const PORTAL_DISPUTES: bool = false;
/// Item 3, `FIN.RUL.15`: days before the dispute is escalated.
pub const DISPUTE_ESCALATION_DAYS: u32 = 14;
/// Item 3: the role the dispute is escalated to.
pub const DISPUTE_ESCALATION_ROLE: &str = PROJECT_MANAGER;
/// Item 4, `FIN.RUL.14`: bad-debt relief is on.
pub const BAD_DEBT_RELIEF: bool = true;
/// Item 7: the most presentations of one cheque.
pub const CHEQUE_PRESENTATIONS: i64 = 2;
/// Item 8, `FIN.RUL.18`: days after a bounce of the reminder.
pub const BOUNCE_REMIND_DAYS: i64 = 3;
/// Item 8: days after a bounce of the task to decide.
pub const BOUNCE_DECIDE_DAYS: i64 = 14;
/// Item 9: the role told of a client cheque bounce and given the decision.
pub const CHEQUE_MANAGER_ROLE: &str = PROJECT_MANAGER;
/// Item 10: the rejection reasons at goods receipt; each needs a photo.
pub const REJECTION_REASONS: [&str; 4] = ["damaged", "wrong_item", "quality", "excess"];
/// Item 12, `STK.ACL.06`: a return above AED 10,000 needs approval, in fils.
pub const RETURN_APPROVAL_FILS: i64 = 1_000_000;
/// Item 13, `STK.RUL.08`: days a return stays `dispatched` before the task.
pub const RETURN_CHASE_DAYS: i64 = 30;
/// Item 16, `STK.ACL.03`: AED 10,000 (project manager) and AED 50,000 (MD).
pub const ORDER_LIMITS_FILS: [i64; 2] = [1_000_000, 5_000_000];
/// Item 18, `STK.ACL.02`: a count variance or wastage above AED 1,000.
pub const WASTAGE_APPROVAL_FILS: i64 = 100_000;
/// Item 22: the payment methods, *set-off* among them.
pub const PAYMENT_METHODS: [&str; 5] = ["cash", "transfer", "cheque", "card", "set_off"];
/// Item 23, `FIN.RUL.23`: the projected balance below which the MD and the
/// accountant are alerted, in fils (AED 200,000).
pub const CASH_THRESHOLD: i64 = 20_000_000;
/// Item 23: the weeks of the forecast.
pub const CASH_FORECAST_WEEKS: u32 = 13;
/// Item 23: the time of the daily check, Asia/Dubai.
pub const CASH_ALERT_TIME: &str = "06:00";
/// Item 23: who is alerted.
pub const CASH_ALERT_ROLES: [&str; 2] = [MD, ACCOUNTANT];
/// Item 23: due dates move by each client's average days late.
pub const CASH_DAYS_LATE: bool = true;
/// Item 23: disputed amounts are not counted.
pub const CASH_DISPUTED_COUNTED: bool = false;
/// Item 24, `FIN.RUL.24`: the share of a month's line that warns, in percent.
pub const BUDGET_VARIANCE_PERCENT: u32 = 100;
/// Item 24: the owner role of a budget category without one of its own.
pub const BUDGET_CATEGORY_OWNER: &str = ACCOUNTANT;
/// Item 25, `FIN.ACL.21`: a drawing is never self-approved.
pub const DRAWINGS_SELF_APPROVE: bool = false;
/// Item 26, `STK.ACL.07`: a preferred supplier change needs a second person.
pub const PREFERRED_CHANGE_NEEDS_APPROVAL: bool = true;
/// Item 26: the score threshold, `STK.RUL.09`.
pub const SCORE_THRESHOLD: i64 = 70;
/// Item 26: the months of the score window.
pub const SCORE_WINDOW_MONTHS: u32 = 3;
/// Item 26: the grace days of an on-time receipt.
pub const SCORE_GRACE_DAYS: i64 = 1;
/// Item 26: the receipts a score needs.
pub const SCORE_MIN_RECEIPTS: i64 = 3;
/// The invoice sources 02 issues at once (a project's IPA invoice is
/// reviewed: none).
pub const AUTO_ISSUE_SOURCES: [&str; 0] = [];

/// The key of the category of an absence case ticket (`CON.RUL.08`).
pub const ABSENCE: &str = crate::absence::ABSENCE_CATEGORY;
/// The key of the category of a snag (item 21: never chargeable).
pub const SNAG: &str = "snag";
/// The key of the category of a defect (item 21: never chargeable).
pub const DEFECT: &str = "defect";

/// A ticket category 02 seeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategorySpec {
    /// The key.
    pub key: &'static str,
    /// The English name.
    pub name_en: &'static str,
    /// The Arabic name.
    pub name_ar: &'static str,
    /// Chargeable to the client: never in 02.
    pub chargeable: bool,
}

/// The ticket categories 02 seeds.
pub const CATEGORIES: [CategorySpec; 3] = [
    CategorySpec {
        key: ABSENCE,
        name_en: "Absence case",
        name_ar: "حالة غياب",
        chargeable: false,
    },
    CategorySpec {
        key: SNAG,
        name_en: "Snag",
        name_ar: "ملاحظة تسليم",
        chargeable: false,
    },
    CategorySpec {
        key: DEFECT,
        name_en: "Defect",
        name_ar: "عيب",
        chargeable: false,
    },
];

// ---- BL-CON-18: labour and compliance -------------------------------------

/// Item 3, `HRM.RUL.13`: normal hours and overtime hours a day.
pub const OVERTIME_PER_DAY: i64 = 2;
/// Item 3: overtime hours in three weeks.
pub const OVERTIME_PER_WINDOW: i64 = 144;
/// Item 6, `HRM.RUL.14`: the kinds checked.
pub const PERMIT_KINDS: [&str; 2] = ["work_permit", "residence_visa"];
/// Item 7: the transition the permit kinds guard, in mode `flag`.
pub const ATTENDANCE_ACCEPT: &str = "attendance_record.accept";
/// Item 8, `DOC.RUL.02`: days of look-ahead.
pub const PERMIT_LOOK_AHEAD_DAYS: i64 = 30;
/// Item 14, `HRM.ACL.01`: days back a supervisor may enter attendance.
pub const LATE_ENTRY_DAYS: i64 = 1;
/// Item 13: days of absence before `report_absconding`.
pub const ABSENCE_DAYS: i64 = 7;
/// Item 2: the rules marked compliance-critical.
pub const CRITICAL_RULES: [&str; 3] = ["HRM.RUL.01", "HRM.RUL.13", "HRM.RUL.14"];

/// A document kind 02 seeds (item 7 and 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindSpec {
    /// The key.
    pub key: &'static str,
    /// The English name.
    pub name_en: &'static str,
    /// The Arabic name.
    pub name_ar: &'static str,
    /// The subject types it attaches to.
    pub subjects: &'static [&'static str],
    /// The days before expiry to remind.
    pub reminders: &'static [i64],
    /// The responsible role.
    pub role: &'static str,
    /// The transition it flags (never blocks): `None` for remind-only kinds.
    pub flags: Option<&'static str>,
    /// The look-ahead days of `DOC.RUL.02`.
    pub look_ahead: i64,
}

const EMPLOYEE: &str = wl_kernel::workforce::EMPLOYEE_SUBJECT_TYPE;
const PARTY: &str = wl_kernel::parties::PARTY_SUBJECT_TYPE;
const PROJECT: &str = crate::project::SUBJECT_TYPE;

/// The kinds 02 seeds. Work permit and residence visa flag the attendance
/// record (`HRM.RUL.14`, no override needed); the other kinds only remind
/// (`DOC.RUL.01`) and block nothing.
pub const KINDS: [KindSpec; 4] = [
    KindSpec {
        key: "work_permit",
        name_en: "Work permit",
        name_ar: "تصريح العمل",
        subjects: &[EMPLOYEE],
        reminders: &[90, 30, 7],
        role: HR,
        flags: Some(ATTENDANCE_ACCEPT),
        look_ahead: PERMIT_LOOK_AHEAD_DAYS,
    },
    KindSpec {
        key: "residence_visa",
        name_en: "Residence visa",
        name_ar: "تأشيرة الإقامة",
        subjects: &[EMPLOYEE],
        reminders: &[90, 30, 7],
        role: HR,
        flags: Some(ATTENDANCE_ACCEPT),
        look_ahead: PERMIT_LOOK_AHEAD_DAYS,
    },
    KindSpec {
        key: "client_contractor_registration",
        name_en: "Client's contractor registration",
        name_ar: "تسجيل المقاول لدى العميل",
        subjects: &[PARTY],
        reminders: &[90, 30, 7],
        role: PROJECT_MANAGER,
        flags: None,
        look_ahead: 0,
    },
    KindSpec {
        key: "performance_bond_document",
        name_en: "Performance bond document",
        name_ar: "مستند ضمان حسن التنفيذ",
        subjects: &[PROJECT],
        reminders: &[60, 30, 7],
        role: ACCOUNTANT,
        flags: None,
        look_ahead: 0,
    },
];

/// Item 19: the registration kinds on (wl-documents seeds them), with their
/// responsible roles. *ISO certificate* is the HSE officer's; *Trademark
/// registration* and *Other certification* are the MD's. No `blocks`.
pub const REGISTRATION_KINDS: [(&str, &str); 3] = [
    ("iso_certificate", HSE_OFFICER),
    ("trademark_registration", MD),
    ("other_certification", MD),
];

/// Item 15: the lead sources 02 uses, by `wl-sales` code. The module's other
/// sources are turned off.
pub const LEAD_SOURCES: [&str; 5] = ["referral", "website", "event", "partner", "other"];

/// Item 16: the campaign channels of 02 as (02's name, the `wl-sales` channel
/// it is recorded as).
pub const CAMPAIGN_CHANNELS: [(&str, &str); 5] = [
    ("Tender invitations", "partner"),
    ("Exhibitions", "radio_outdoor"),
    ("LinkedIn", "other_online"),
    ("Email", "email"),
    ("Company profile mailings", "email"),
];

/// Item 16, `SAL.ACL.07`: a cost entry over a campaign's budget needs a
/// manager.
pub const OVER_BUDGET_APPROVAL: bool = true;

/// One step of an onboarding checklist: title, owner (`hr`, `manager`,
/// `employee` or `role:<key>`), due days and kind (`task`, `sops` or
/// `checklist:<template>`).
pub type StepSpec = (&'static str, &'static str, i64, &'static str);

/// An onboarding checklist for one role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnboardingSpec {
    /// The template's name.
    pub name: &'static str,
    /// The job role it applies to.
    pub role: &'static str,
    /// The steps.
    pub steps: &'static [StepSpec],
}

const HR_STEP: StepSpec = ("Contract, visa and ID documents", "hr", 2, "task");
const MANAGER_STEP: StepSpec = ("Welcome and introductions", "manager", 1, "task");
const EMPLOYEE_STEP: StepSpec = ("Upload your personal documents", "employee", 3, "task");
const SOP_STEP: StepSpec = ("SOPs acknowledged", "employee", 7, "sops");

/// The onboarding checklists 02 seeds (`HRM.RUL.16`); completion blocks
/// nothing.
pub const ONBOARDING: [OnboardingSpec; 5] = [
    OnboardingSpec {
        name: "Worker onboarding",
        role: "worker",
        steps: &[
            HR_STEP,
            MANAGER_STEP,
            ("Site induction", "manager", 1, "checklist:site_induction"),
            ("PPE issue", "manager", 1, "checklist:ppe_issue"),
            EMPLOYEE_STEP,
            SOP_STEP,
        ],
    },
    OnboardingSpec {
        name: "Supervisor onboarding",
        role: SUPERVISOR,
        steps: &[
            HR_STEP,
            MANAGER_STEP,
            ("Site induction", "manager", 1, "checklist:site_induction"),
            EMPLOYEE_STEP,
            SOP_STEP,
        ],
    },
    OnboardingSpec {
        name: "Storekeeper onboarding",
        role: STOREKEEPER,
        steps: &[
            HR_STEP,
            MANAGER_STEP,
            (
                "Receiving and issuing walk-through",
                "manager",
                3,
                "checklist:store_induction",
            ),
            EMPLOYEE_STEP,
            SOP_STEP,
        ],
    },
    OnboardingSpec {
        name: "Procurement officer onboarding",
        role: PROCUREMENT,
        steps: &[
            HR_STEP,
            MANAGER_STEP,
            (
                "Purchase orders and suppliers walk-through",
                "manager",
                3,
                "checklist:procurement_induction",
            ),
            EMPLOYEE_STEP,
            SOP_STEP,
        ],
    },
    OnboardingSpec {
        name: "Quantity surveyor onboarding",
        role: QUANTITY_SURVEYOR,
        steps: &[
            HR_STEP,
            MANAGER_STEP,
            (
                "IPAs, variations and retention walk-through",
                "manager",
                3,
                "checklist:qs_induction",
            ),
            EMPLOYEE_STEP,
            SOP_STEP,
        ],
    },
];

/// Item 17, `HRM.RUL.17`: the reminder interval after the first, in days.
pub const ONBOARDING_REMIND_EVERY_DAYS: i64 = 2;
/// Item 17: the days after which HR and the line manager are told.
pub const ONBOARDING_ESCALATE_DAYS: i64 = 5;

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
    /// The roles that must read and acknowledge it.
    pub readers: &'static [&'static str],
}

/// The SOP library 02 seeds: the HSE officer owns the safety SOPs and method
/// statements, the project manager the site SOPs. `DOC.RUL.04` asks for an
/// acknowledgement within 7 days, reminds every 3 days and tells the manager
/// and the owner role after 7 days overdue (the module's defaults).
pub const SOPS: [SopSpec; 4] = [
    SopSpec {
        code: "SOP-CON-SAFETY",
        title: "Site safety",
        category: "safety",
        owner_role: HSE_OFFICER,
        readers: &[SUPERVISOR, "worker"],
    },
    SopSpec {
        code: "SOP-CON-METHOD-STATEMENTS",
        title: "Method statements",
        category: "safety",
        owner_role: HSE_OFFICER,
        readers: &[SUPERVISOR, "worker"],
    },
    SopSpec {
        code: "SOP-CON-SITE-RULES",
        title: "Site rules",
        category: "site",
        owner_role: PROJECT_MANAGER,
        readers: &[SUPERVISOR, "worker"],
    },
    SopSpec {
        code: "SOP-CON-STORES",
        title: "Site stores",
        category: "site",
        owner_role: PROJECT_MANAGER,
        readers: &[SUPERVISOR, STOREKEEPER],
    },
];

/// Marks [`CRITICAL_RULES`] compliance-critical with the company admin (the
/// MD) as owner (item 1 and 2): a failed run alerts the owner role.
pub(crate) fn register_critical_rules() {
    for rule in CRITICAL_RULES {
        CriticalRule::new(rule).owner_role(MD).register();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_money_values_are_those_of_the_spec() {
        assert_eq!(WRITE_OFF_APPROVERS, [MD]);
        assert_eq!(RETURN_APPROVAL_FILS, 1_000_000);
        assert_eq!(CASH_THRESHOLD, 20_000_000);
        assert!(PAYMENT_METHODS.contains(&"set_off"));
        assert!(!DRAWINGS_SELF_APPROVE);
    }

    #[test]
    fn no_category_is_chargeable() {
        assert!(CATEGORIES.iter().all(|c| !c.chargeable));
        assert!(CATEGORIES.iter().any(|c| c.key == "absence"));
    }

    #[test]
    fn the_permit_kinds_only_flag() {
        for key in PERMIT_KINDS {
            let kind = KINDS.iter().find(|k| k.key == key).expect("a kind");
            assert_eq!(kind.flags, Some(ATTENDANCE_ACCEPT));
        }
        assert!(KINDS.iter().filter(|k| k.flags.is_none()).count() == 2);
    }

    #[test]
    fn every_checklist_ends_with_the_sops() {
        for spec in ONBOARDING {
            assert_eq!(spec.steps.last().map(|s| s.3), Some("sops"));
        }
    }
}
