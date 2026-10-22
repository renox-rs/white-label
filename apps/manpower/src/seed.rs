//! What the Manpower client keeps in the shared modules (09 Manpower §3, §4,
//! §7.2, §8, §9; BL-MPW-14, BL-MPW-17, BL-MPW-20, BL-MPW-21).
//!
//! The mechanisms live in the shared modules (`wl-documents`, `wl-parties`,
//! `wl-notify`, `wl-ticketing`, `wl-invoicing`, `wl-payments`, `wl-attendance`,
//! `wl-hr`, `wl-sales`) and the vertical reaches them only through `wl-kernel`.
//! This file is therefore plain data, the one place where a 09 value is
//! written down: the host (`host/src/app/manpower_seed.rs`) turns these
//! tables into the modules' own builder calls and seeds. What this file does
//! itself is mark the compliance-critical rules for the owner alert
//! ([`register_critical_rules`]).
//!
//! # Settings whose owning module does not expose them yet
//!
//! Recorded here, not built (the mechanism belongs to the module; the same
//! list, with the issue and the provider, is in the batch's deferred table):
//!
//! - **Visa step templates** per emirate with the *cancellation* variants
//!   (`VisaStepTemplate`): wl-hr has no template model, only steps with a
//!   target ([`crate::visa_steps`]).
//! - **The cut-off day per legal entity** (`HRM.RUL.02`): `HrConfig` holds one
//!   day for the company, 25 here ([`CUT_OFF_DAY`]); there is no per-entity
//!   `PayrollCalendar` for it (the kernel's calendar gives the WPS due date).
//! - **WPS deadline alerts** (`HRM.RUL.03`: 3 days before, day 2 and day 5) and
//!   **the salary change approval above 10 %** (`MPW.ACL.06`): not built in
//!   wl-hr.
//! - **Recovery caps** (`HRM.ACL.06`, 20 % and 50 %) and the **flag review**
//!   (`HRM.ACL.09`), **payroll and settlement SoD** (`HRM.ACL.04`), the
//!   **non-standard exits approver** (`HRM.ACL.07`) and the **payroll basis**
//!   `attendance` are the module's own fixed rules, equal to the 09 values.
//! - **The cash-flow `CashForecastSource` of 09** (payroll by the WPS due date,
//!   visa and insurance costs by their schedule) and the **export account
//!   mapping** of the budget categories: needs a wl-hr provider and the client's
//!   chart of accounts.
//! - **The e-invoicing go-live date and the ASP adapter** of each legal entity
//!   are the client's own (`WlInvoicing::einvoicing`).
//! - **Worker SMS types** (payslip published, document expiry, leave decision,
//!   shift change) and the **rate card / renewal signature requests** and
//!   **timesheet approval request and reminder** types have no sender in the
//!   shared modules yet; the 09 reactions are set on the types that exist
//!   ([`NOTIFICATION_TYPES`], [`crate::notices`]).
//! - **The campaign channels** of `SAL.CRM.02` are a fixed list in wl-sales:
//!   the 09 names are mapped to the closest ([`CAMPAIGN_CHANNELS`]).

use wl_kernel::compliance::CriticalRule;
use wl_kernel::notify::GiveUpReaction;

use crate::permissions::{ACCOUNT_MANAGER_ROLE, HR_ROLE, PRO_ROLE, SUPERVISOR_ROLE};

/// The MD: the company admin.
pub const MD: &str = wl_kernel::COMPANY_ADMIN_ROLE;
/// The company admin.
pub const COMPANY_ADMIN: &str = wl_kernel::COMPANY_ADMIN_ROLE;
/// The accountant.
pub const ACCOUNTANT: &str = "accountant";
/// HR / the payroll officer.
pub const HR: &str = HR_ROLE;
/// The PRO / visa officer.
pub const PRO: &str = PRO_ROLE;
/// The account manager.
pub const ACCOUNT_MANAGER: &str = ACCOUNT_MANAGER_ROLE;
/// The site supervisor.
pub const SUPERVISOR: &str = SUPERVISOR_ROLE;

/// The category of a complaint about the conduct of a worker (BL-MPW-14,
/// `WRK.TKT.01`).
pub const CONDUCT_CATEGORY: &str = "conduct";

/// The category of an absence case, opened by `MPW.RUL.09` for HR
/// (BL-MPW-17).
pub const ABSENCE_CATEGORY: &str = "absence";

/// The category of a worker's grievance, confidential (09 §3.2).
pub const GRIEVANCE_CATEGORY: &str = "grievance";

/// The role whose holders look after absence cases.
pub const ABSENCE_ROLE: &str = crate::permissions::HR_ROLE;

/// A ticket category the vertical seeds.
#[derive(Debug, Clone, Copy)]
pub struct CategorySpec {
    /// The fixed key the vertical opens tickets with.
    pub key: &'static str,
    /// The English name.
    pub name_en: &'static str,
    /// The Arabic name.
    pub name_ar: &'static str,
    /// The role whose holders look after its tickets.
    pub assignee_role: Option<&'static str>,
    /// Whether only the holders of `tickets.confidential.view` and the
    /// requester see its tickets (`WRK.ACL.03`).
    pub confidential: bool,
}

/// The ticket categories of 09.
pub const CATEGORIES: [CategorySpec; 3] = [
    CategorySpec {
        key: CONDUCT_CATEGORY,
        name_en: "Conduct complaint",
        name_ar: "شكوى سلوك",
        assignee_role: None,
        confidential: false,
    },
    CategorySpec {
        key: ABSENCE_CATEGORY,
        name_en: "Absence case",
        name_ar: "حالة غياب",
        assignee_role: Some(ABSENCE_ROLE),
        confidential: false,
    },
    // A worker's grievance: the requester is the worker, and only HR, the
    // accountant and the MD see it (09 §3.2, wl-ticketing §3).
    CategorySpec {
        key: GRIEVANCE_CATEGORY,
        name_en: "Worker grievance",
        name_ar: "شكوى عامل",
        assignee_role: Some(ABSENCE_ROLE),
        confidential: true,
    },
];

// ---- BL-MPW-20: documents, parties, notifications, alerts, integrations ----

/// The look-ahead window of the kinds that block work, in days (`MPW.ACL.07`
/// N = 30, `DOC.RUL.02`).
pub const LOOK_AHEAD_DAYS: i64 = 30;

/// The deployment transitions the blocking kinds guard, in mode `block`.
pub const DEPLOYMENT_GUARDS: [&str; 3] = [
    "deployment.activate",
    "deployment.extend",
    "deployment.resume",
];

/// The transition the blocking kinds only flag (`HRM.RUL.14`).
pub const ATTENDANCE_ACCEPT: &str = "attendance_record.accept";

/// The job roles the SIRA licence is required for.
pub const SECURITY_ROLES: [&str; 2] = ["security_guard", "security_supervisor"];

/// The job roles of a worker on site.
pub const WORKER_ROLES: &str = "worker,security_guard,security_supervisor,cleaner,driver,labourer";

/// How a kind is overridden when a required document is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Override {
    /// Never: a legal block (`⚠ open: lawyer (L-19)`), locked.
    Never,
    /// With the MD's approval (`DOC.ACL.02`).
    WithApproval,
}

/// What a kind guards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guards {
    /// Nothing: the kind only expires and reminds.
    Nothing,
    /// The deployment transitions (block) and the attendance accept (flag), for
    /// every record.
    Everyone,
    /// As [`Guards::Everyone`], for the job roles of [`SECURITY_ROLES`] only.
    SecurityRoles,
}

/// A document kind 09 seeds (§9 table).
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
    /// The days before expiry to remind; none: the kind has no expiry.
    pub reminders: &'static [i64],
    /// The responsible role.
    pub role: &'static str,
    /// The roles told on expiry besides the responsible role: the account
    /// manager of each affected client for a worker's document (§9).
    pub also_notify: &'static [&'static str],
    /// What it guards.
    pub guards: Guards,
    /// How it is overridden.
    pub override_mode: Override,
    /// The look-ahead days of `DOC.RUL.02`; 0: the module's.
    pub look_ahead: i64,
}

const EMPLOYEE: &str = wl_kernel::workforce::EMPLOYEE_SUBJECT_TYPE;
const CONTRACT: &str = crate::contract::SUBJECT_TYPE;
const PAYROLL_RUN: &str = wl_kernel::workforce::PAYROLL_RUN_SUBJECT_TYPE;
const TIMESHEET: &str = wl_kernel::timesheets::TIMESHEET_SUBJECT_TYPE;

const fn guarding(
    key: &'static str,
    name_en: &'static str,
    name_ar: &'static str,
    role: &'static str,
    also_notify: &'static [&'static str],
    guards: Guards,
    override_mode: Override,
) -> KindSpec {
    KindSpec {
        key,
        name_en,
        name_ar,
        subjects: &[EMPLOYEE],
        reminders: &[90, 30, 7],
        role,
        also_notify,
        guards,
        override_mode,
        look_ahead: LOOK_AHEAD_DAYS,
    }
}

/// The kinds 09 seeds. The residence visa, the work permit and the SIRA licence
/// are legal blocks (override `never`, locked); the Emirates ID, the health
/// insurance, the health card and the site pass can be overridden with the
/// MD's approval; the others block nothing.
pub const KINDS: [KindSpec; 12] = [
    guarding(
        "residence_visa",
        "Residence visa",
        "تأشيرة الإقامة",
        PRO,
        &[ACCOUNT_MANAGER],
        Guards::Everyone,
        Override::Never,
    ),
    guarding(
        "work_permit",
        "Work permit",
        "تصريح العمل",
        PRO,
        &[ACCOUNT_MANAGER],
        Guards::Everyone,
        Override::Never,
    ),
    guarding(
        "sira_licence",
        "SIRA licence",
        "رخصة هيئة تنظيم الأمن الخاص",
        PRO,
        &[ACCOUNT_MANAGER],
        Guards::SecurityRoles,
        Override::Never,
    ),
    guarding(
        "emirates_id",
        "Emirates ID",
        "الهوية الإماراتية",
        PRO,
        &[ACCOUNT_MANAGER],
        Guards::Everyone,
        Override::WithApproval,
    ),
    guarding(
        "health_insurance",
        "Health insurance",
        "التأمين الصحي",
        PRO,
        &[ACCOUNT_MANAGER],
        Guards::Everyone,
        Override::WithApproval,
    ),
    guarding(
        "health_card",
        "Health card",
        "البطاقة الصحية",
        PRO,
        &[ACCOUNT_MANAGER],
        Guards::Everyone,
        Override::WithApproval,
    ),
    guarding(
        "site_pass",
        "Site pass",
        "تصريح الموقع",
        ACCOUNT_MANAGER,
        &[],
        Guards::Everyone,
        Override::WithApproval,
    ),
    KindSpec {
        key: "passport",
        name_en: "Passport",
        name_ar: "جواز السفر",
        subjects: &[EMPLOYEE],
        reminders: &[180, 90, 30],
        role: PRO,
        also_notify: &[],
        guards: Guards::Nothing,
        override_mode: Override::WithApproval,
        look_ahead: 0,
    },
    KindSpec {
        key: "iloe",
        name_en: "ILOE insurance",
        name_ar: "التأمين ضد التعطل عن العمل",
        subjects: &[EMPLOYEE],
        reminders: &[90, 30, 7],
        role: PRO,
        also_notify: &[],
        guards: Guards::Nothing,
        override_mode: Override::WithApproval,
        look_ahead: 0,
    },
    KindSpec {
        key: "labour_contract",
        name_en: "Labour contract",
        name_ar: "عقد العمل",
        subjects: &[EMPLOYEE],
        reminders: &[90, 30, 7],
        role: HR,
        also_notify: &[],
        guards: Guards::Nothing,
        override_mode: Override::WithApproval,
        look_ahead: 0,
    },
    KindSpec {
        key: "client_contract",
        name_en: "Client contract",
        name_ar: "عقد العميل",
        subjects: &[CONTRACT],
        reminders: &[60, 30],
        role: ACCOUNT_MANAGER,
        also_notify: &[],
        guards: Guards::Nothing,
        override_mode: Override::WithApproval,
        look_ahead: 0,
    },
    KindSpec {
        key: "rate_card_acceptance",
        name_en: "Rate card acceptance",
        name_ar: "قبول جدول الأسعار",
        subjects: &[CONTRACT],
        reminders: &[],
        role: ACCOUNT_MANAGER,
        also_notify: &[],
        guards: Guards::Nothing,
        override_mode: Override::WithApproval,
        look_ahead: 0,
    },
];

/// The signed timesheet, a kind of the account manager's that blocks nothing.
pub const SIGNED_TIMESHEET: KindSpec = KindSpec {
    key: "signed_timesheet",
    name_en: "Signed timesheet",
    name_ar: "كشف الدوام الموقع",
    subjects: &[TIMESHEET],
    reminders: &[],
    role: ACCOUNT_MANAGER,
    also_notify: &[],
    guards: Guards::Nothing,
    override_mode: Override::WithApproval,
    look_ahead: 0,
};

/// The bank's result file of a payroll run (the manual WPS fallback, #510):
/// uploaded on the run's documents page, its number is recorded with the
/// partial result. The accountant answers for it; it blocks nothing.
pub const BANK_RESULT: KindSpec = KindSpec {
    key: "bank_result",
    name_en: "Bank result file",
    name_ar: "ملف نتيجة البنك",
    subjects: &[PAYROLL_RUN],
    reminders: &[],
    role: ACCOUNTANT,
    also_notify: &[],
    guards: Guards::Nothing,
    override_mode: Override::WithApproval,
    look_ahead: 0,
};

/// The registration kinds on for 09 with their responsible role (the MD):
/// *Trademark registration* and *Other certification*. No `blocks`.
pub const REGISTRATION_KINDS: [(&str, &str); 2] =
    [("trademark_registration", MD), ("other_certification", MD)];

/// The kind of party a blocked client is (`PartyBlocked`, block effect
/// `continue`, decider the MD): see [`crate::blocked`].
pub const BLOCK_EFFECT_KIND: &str = crate::blocked::KIND;

/// How a notification type is configured for 09 (§7.2 *Notification types*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeSpec {
    /// The type's key.
    pub key: &'static str,
    /// The group (the row of wl-notify §5.4); empty: none.
    pub group: &'static str,
    /// Whether it is a statutory notice.
    pub statutory: bool,
    /// What happens on give-up.
    pub give_up: &'static [GiveUpReaction],
    /// The staff role told.
    pub role: Option<&'static str>,
}

/// The give-up of a statutory notice: the contact is marked invalid and the
/// statutory-notice task (deliver by another means, record the proof) goes to
/// the account manager.
pub const STATUTORY_GIVE_UP: [GiveUpReaction; 2] = [
    GiveUpReaction::MarkContactInvalid,
    GiveUpReaction::StatutoryNotice,
];

/// The give-up of a portal / document / signature message and of a deployment
/// notice to the client: the contact is marked invalid and the account manager
/// gets a task.
pub const CLIENT_GIVE_UP: [GiveUpReaction; 2] = [
    GiveUpReaction::MarkContactInvalid,
    GiveUpReaction::StaffTask,
];

/// The give-up of an invoice, statement or reminder: the contact is marked
/// invalid, the client's other finance contact is tried, then the accountant
/// gets a task.
pub const INVOICE_GIVE_UP: [GiveUpReaction; 3] = [
    GiveUpReaction::MarkContactInvalid,
    GiveUpReaction::AlternativeRecipient,
    GiveUpReaction::StaffTask,
];

/// The group of the statutory notices.
pub const GROUP_STATUTORY: &str = "statutory";

/// The types of the shared modules whose stored rows the seed sets for 09
/// (the types 09 registers itself carry theirs in [`crate::notices`]):
/// timesheet escalations and the deemed-approval notice go with the portal
/// invitations, the invoice reminders to the accountant.
pub const NOTIFICATION_TYPES: [TypeSpec; 4] = [
    TypeSpec {
        key: "attendance.timesheet_escalated",
        group: wl_kernel::notify::GROUP_PORTAL,
        statutory: false,
        give_up: &CLIENT_GIVE_UP,
        role: Some(ACCOUNT_MANAGER),
    },
    TypeSpec {
        key: "attendance.timesheet_deemed",
        group: wl_kernel::notify::GROUP_PORTAL,
        statutory: false,
        give_up: &CLIENT_GIVE_UP,
        role: Some(ACCOUNT_MANAGER),
    },
    TypeSpec {
        key: "invoice.reminder_before_due",
        group: wl_kernel::notify::GROUP_INVOICES,
        statutory: false,
        give_up: &INVOICE_GIVE_UP,
        role: Some(ACCOUNTANT),
    },
    TypeSpec {
        key: "invoice.reminder_after_due",
        group: wl_kernel::notify::GROUP_INVOICES,
        statutory: false,
        give_up: &INVOICE_GIVE_UP,
        role: Some(ACCOUNTANT),
    },
];

/// Item "Compliance-critical rules": a failed run alerts the owner (the MD and
/// the company admin, `PLT.RUL.04`). Set by us; not editable by the client.
pub const CRITICAL_RULES: [&str; 9] = [
    "HRM.RUL.03",
    "HRM.RUL.04",
    "HRM.RUL.09",
    "HRM.RUL.10",
    "HRM.RUL.11",
    "HRM.RUL.14",
    "MPW.RUL.09",
    "MPW.RUL.12",
    "DOC.RUL.03",
];

/// The role of the owner alert (`PLT.RUL.04`): the holders of `md` and the
/// company admins are alerted by a failed run.
pub const MD_ROLE: &str = crate::permissions::MD_ROLE;

/// Marks [`CRITICAL_RULES`] compliance-critical with the MD as owner: a failed
/// run alerts the holders of the MD role and the company admin.
pub(crate) fn register_critical_rules() {
    for rule in CRITICAL_RULES {
        CriticalRule::new(rule).owner_role(MD_ROLE).register();
    }
}

/// The connection keys 09 gives a manual fallback (§8).
pub const SMS_CONNECTION: &str = "sms";
/// The bank or exchange house the SIF is uploaded to by hand.
pub const WPS_CONNECTION: &str = "wps.bank";
/// Attendance imports and devices.
pub const ATTENDANCE_CONNECTION: &str = "attendance.import";

/// A connection and its manual fallback (§8): key, owning module, owner role,
/// the screens it affects, the English and Arabic fallback.
pub type ConnectionSpec = (
    &'static str,
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static str,
);

/// The integration fallbacks of 09.
pub const CONNECTIONS: [ConnectionSpec; 3] = [
    (
        SMS_CONNECTION,
        "notify",
        HR,
        &["notify.messages"],
        "A message to a worker that cannot be sent becomes a task for HR: reach the worker through the site supervisor.",
        "تتحول الرسالة التي تعذر إرسالها إلى العامل إلى مهمة للموارد البشرية: تواصل مع العامل عبر مشرف الموقع.",
    ),
    (
        WPS_CONNECTION,
        "hr",
        ACCOUNTANT,
        &["hr.payroll"],
        "Download the SIF of the approved run, upload it in the bank's or exchange house's portal by hand and attach the bank's result file to the run.",
        "نزّل ملف SIF للمسيرة المعتمدة وارفعه يدويًا في بوابة البنك أو شركة الصرافة وأرفق ملف النتيجة بالمسيرة.",
    ),
    (
        ATTENDANCE_CONNECTION,
        "attendance",
        HR,
        &["attendance.records"],
        "Keep the attendance on paper; the site supervisor enters it late with the late-entry flag (one day back).",
        "احتفظ بالحضور على الورق؛ يدخله مشرف الموقع لاحقًا مع علامة الإدخال المتأخر (يوم واحد للخلف).",
    ),
];

/// Item "Lead sources used" (`SAL.CRM.02`): the sources 09 uses, by `wl-sales`
/// code. The module's other sources are turned off. Paid ads is the 09 name for
/// the module's code `paid_ads`.
pub const LEAD_SOURCES: [&str; 6] = [
    "referral", "website", "paid_ads", "partner", "event", "other",
];

/// Item "Campaign channels" (`SAL.CRM.02`) as (09's name, the `wl-sales`
/// channel it is recorded as).
pub const CAMPAIGN_CHANNELS: [(&str, &str); 4] = [
    ("LinkedIn", "other_online"),
    ("Search ads", "google"),
    ("Email", "email"),
    ("Industry events", "event"),
];

/// `SAL.ACL.07`: a cost entry over a campaign's budget needs a manager.
pub const OVER_BUDGET_APPROVAL: bool = true;

/// `HRM.RUL.17`: the reminder interval after the first, in days.
pub const ONBOARDING_REMIND_EVERY_DAYS: i64 = 2;
/// `HRM.RUL.17`: the days after which HR and the line manager are told.
pub const ONBOARDING_ESCALATE_DAYS: i64 = 5;

/// One step of an onboarding checklist: title, owner (`hr`, `manager`,
/// `employee` or `role:<key>`), due days and kind (`task`, `sops` or
/// `checklist:<template>`).
pub type StepSpec = (&'static str, &'static str, i64, &'static str);

/// An onboarding checklist for the job roles it applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnboardingSpec {
    /// The template's name.
    pub name: &'static str,
    /// The job roles it applies to, comma separated.
    pub role: &'static str,
    /// The steps.
    pub steps: &'static [StepSpec],
}

const HR_STEP: StepSpec = ("Contract, visa and ID documents", "hr", 2, "task");
const MANAGER_STEP: StepSpec = ("Welcome and introductions", "manager", 1, "task");
const AM_STEP: StepSpec = (
    "Client-site rules and site pass",
    "role:account_manager",
    2,
    "task",
);
const EMPLOYEE_STEP: StepSpec = ("Upload your personal documents", "employee", 3, "task");
const SOP_STEP: StepSpec = ("SOPs acknowledged", "employee", 7, "sops");

/// The checklists 09 seeds (`HRM.RUL.16`): the worker's (deployment induction,
/// PPE with the worker's signature, tasks for HR, the account manager and the
/// employee), the recruiter's, the site supervisor's and the account
/// manager's. Visa steps stay in visa cases; completion blocks nothing.
pub const ONBOARDING: [OnboardingSpec; 4] = [
    OnboardingSpec {
        name: "Worker onboarding",
        role: WORKER_ROLES,
        steps: &[
            HR_STEP,
            AM_STEP,
            (
                "Deployment induction",
                "manager",
                1,
                "checklist:workplace_induction",
            ),
            ("PPE issue", "manager", 1, "checklist:ppe_handover"),
            EMPLOYEE_STEP,
            SOP_STEP,
        ],
    },
    OnboardingSpec {
        name: "Recruiter onboarding",
        role: "recruiter",
        steps: &[HR_STEP, MANAGER_STEP, EMPLOYEE_STEP, SOP_STEP],
    },
    OnboardingSpec {
        name: "Site supervisor onboarding",
        role: "site_supervisor",
        steps: &[
            HR_STEP,
            MANAGER_STEP,
            (
                "Deployment induction",
                "manager",
                1,
                "checklist:workplace_induction",
            ),
            EMPLOYEE_STEP,
            SOP_STEP,
        ],
    },
    OnboardingSpec {
        name: "Account manager onboarding",
        role: "account_manager",
        steps: &[HR_STEP, MANAGER_STEP, EMPLOYEE_STEP, SOP_STEP],
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
    /// The roles that must read and acknowledge it.
    pub readers: &'static [&'static str],
}

/// The SOP library 09 seeds: HR / the payroll officer owns the HR and payroll
/// SOPs, the account manager the client-site rules per contract.
/// `DOC.RUL.04` asks each reader to acknowledge within 7 days, reminds every 3
/// days and tells the reader's manager and the owner role after 7 days overdue
/// (the module's defaults).
pub const SOPS: [SopSpec; 3] = [
    SopSpec {
        code: "SOP-MPW-HR-POLICIES",
        title: "Worker conduct, leave and pay rules",
        category: "hr",
        owner_role: HR,
        readers: &["worker", SUPERVISOR],
    },
    SopSpec {
        code: "SOP-MPW-PAYROLL",
        title: "Payroll, WPS and deductions",
        category: "payroll",
        owner_role: HR,
        readers: &[HR, ACCOUNTANT],
    },
    SopSpec {
        code: "SOP-MPW-CLIENT-SITE-RULES",
        title: "Client-site rules",
        category: "site",
        owner_role: ACCOUNT_MANAGER,
        readers: &["worker", SUPERVISOR, ACCOUNT_MANAGER],
    },
];

// ---- BL-MPW-21: money, attendance, payroll and HR --------------------------

/// Item "Bad-debt write-off limit" (`FIN.ACL.15`): AED 0, every write-off
/// approved, by the MD.
pub const WRITE_OFF_APPROVERS: [&str; 1] = [MD];
/// The client approver may raise an invoice dispute in the portal.
pub const PORTAL_DISPUTES: bool = true;
/// `FIN.RUL.15`: days before an open dispute is escalated.
pub const DISPUTE_ESCALATION_DAYS: u32 = 14;
/// `FIN.RUL.14`: bad-debt relief is on.
pub const BAD_DEBT_RELIEF: bool = true;
/// The most presentations of one cheque.
pub const CHEQUE_PRESENTATIONS: i64 = 2;
/// `FIN.RUL.18`: days after a bounce of the reminder.
pub const BOUNCE_REMIND_DAYS: i64 = 3;
/// `FIN.RUL.18`: days after a bounce of the task to decide.
pub const BOUNCE_DECIDE_DAYS: i64 = 14;
/// The role told of a client cheque bounce and given the decision.
pub const CHEQUE_MANAGER_ROLE: &str = ACCOUNTANT;
/// `FIN.RUL.23`: the projected balance below which the MD and the accountant
/// are alerted, in fils (AED 250,000).
pub const CASH_THRESHOLD: i64 = 25_000_000;
/// `FIN.CFL.01`: the weeks of the forecast.
pub const CASH_FORECAST_WEEKS: u32 = 13;
/// `FIN.RUL.23`: the time of the daily check, Asia/Dubai.
pub const CASH_ALERT_TIME: &str = "06:00";
/// `FIN.RUL.23`: who is alerted.
pub const CASH_ALERT_ROLES: [&str; 2] = [MD, ACCOUNTANT];
/// Due dates move by each client's average days late.
pub const CASH_DAYS_LATE: bool = true;
/// Disputed amounts are not counted.
pub const CASH_DISPUTED_COUNTED: bool = false;
/// `FIN.RUL.24`: the share of a month's line that warns, in percent.
pub const BUDGET_VARIANCE_PERCENT: u32 = 100;
/// `FIN.RUL.24`: the owner role of a budget category without one of its own.
pub const BUDGET_CATEGORY_OWNER: &str = ACCOUNTANT;
/// `FIN.ACL.21`: a drawing is never approved by its requester.
pub const DRAWINGS_SELF_APPROVE: bool = false;

/// A budget category 09 adds to the module's (the module has *Salaries and
/// staff costs* for payroll and *Transport and fuel*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetSpec {
    /// The code.
    pub code: &'static str,
    /// The English name.
    pub name_en: &'static str,
    /// The Arabic name.
    pub name_ar: &'static str,
    /// The cost codes it collects.
    pub cost_codes: &'static [&'static str],
}

/// The expense categories 09 adds: visa and insurance costs, accommodation and
/// office. Payroll and transport are the module's *Salaries and staff costs*
/// and *Transport and fuel*.
pub const BUDGET_CATEGORIES: [BudgetSpec; 3] = [
    BudgetSpec {
        code: "visa_insurance",
        name_en: "Visa and insurance costs",
        name_ar: "تكاليف التأشيرات والتأمين",
        cost_codes: &["visa", "insurance"],
    },
    BudgetSpec {
        code: "accommodation",
        name_en: "Accommodation",
        name_ar: "السكن",
        cost_codes: &["accommodation"],
    },
    BudgetSpec {
        code: "office",
        name_en: "Office",
        name_ar: "المكتب",
        cost_codes: &["office"],
    },
];

/// `HRM.ACL.01`: days back a supervisor may enter attendance.
pub const LATE_ENTRY_DAYS: i64 = 1;
/// `HRM.RUL.13`: overtime hours a day.
pub const OVERTIME_PER_DAY: i64 = 2;
/// `HRM.RUL.13`: overtime hours in three weeks.
pub const OVERTIME_PER_WINDOW: i64 = 144;
/// `HRM.RUL.14`: the kinds checked.
pub const PERMIT_KINDS: [&str; 2] = ["work_permit", "residence_visa"];
/// `HRM.RUL.15`: days after the submit that the client's escalation contact and
/// the account manager's manager are told.
pub const ESCALATION_DAYS: i64 = 6;
/// `HRM.RUL.02`: the payroll cut-off day of each month.
pub const CUT_OFF_DAY: u32 = 25;
/// `HRM.RUL.11`: days before the WPS due date a rejection still open is
/// escalated.
pub const WPS_ESCALATION_DAYS: i64 = 1;
/// `HRM.ACL.05`: the cooling period of a new pay account, in days.
pub const COOLING_DAYS: i64 = 5;
/// `HRM.RUL.09`: days after the last working day HR and the accountant are
/// told (the MD on the 14th, the legal deadline).
pub const SETTLEMENT_ALERT_DAYS: i64 = 10;
/// Days of consecutive absence before `report_absconding` (more than 7).
pub const ABSENCE_DAYS: i64 = 7;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_categories_have_distinct_keys() {
        assert_ne!(CATEGORIES[0].key, CATEGORIES[1].key);
        assert!(CATEGORIES.iter().any(|c| c.key == ABSENCE_CATEGORY));
    }
}
