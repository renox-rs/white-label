//! The 03 deployment's values for the shared mechanisms (03 Logistics §3.3,
//! §4.1, §7.1, §7.2, §8, §9; BL-LOG-25 and BL-LOG-26).
//!
//! The mechanisms live in the shared modules (`wl-documents`, `wl-hr`,
//! `wl-sales`, `wl-delivery`, `wl-payments`, `wl-invoicing`, `wl-portal`) and
//! the vertical reaches them only through `wl-kernel`. These are the tables
//! of values; the host (`host/src/app/logistics_seed.rs`) turns them into the
//! modules' own builder calls and one idempotent seed, and the settings pages
//! read the same values back, so a value shown is a value in force. Nothing
//! here is a rule and no module hard-codes a value: a client changes a seeded
//! value in the module's own settings, except where a value is an open item
//! (`L-19`, `L-20`) and is locked or off.

use crate::permissions::{
    ACCOUNTANT_ROLE, CASHIER_ROLE, CUSTOMER_SERVICE_ROLE, DISPATCHER_ROLE,
    DOCUMENTATION_OFFICER_ROLE, DRIVER_ROLE, FLEET_MANAGER_ROLE, HUB_STAFF_ROLE,
    OPERATIONS_MANAGER_ROLE,
};

// ---------------------------------------------------------------- documents

/// The guarded transition of a delivery put on a run (wl-delivery `assign`).
pub const ASSIGN_GUARDED: &str = "delivery.assign";
/// The guarded transition of a run that leaves (wl-delivery `dispatch`).
pub const DISPATCH_GUARDED: &str = "run.dispatch";

/// The driving licence (a driver, through `HRM.QUA.01`).
pub const DRIVING_LICENCE: &str = "driving_licence";
/// The RTA rider permit and qualification certificate (a motorbike rider).
pub const RIDER_PERMIT: &str = "rider_permit";
/// The vehicle registration, Mulkiya.
pub const MULKIYA: &str = "mulkiya";
/// The vehicle insurance.
pub const VEHICLE_INSURANCE: &str = "vehicle_insurance";
/// Asateel or another vehicle permit.
pub const VEHICLE_PERMIT: &str = "vehicle_permit";
/// The shipping and customs documents of a freight job.
pub const FREIGHT_DOCUMENTS: &str = "freight_documents";
/// A client contract.
pub const CLIENT_CONTRACT: &str = "client_contract";
/// A rate card agreement.
pub const RATE_CARD_AGREEMENT: &str = "rate_card_agreement";

/// The subject type of a driver's employee record.
pub const EMPLOYEE_SUBJECT: &str = "hr.employee";
/// The subject type of a driver when `wl-hr` is off: the user's account.
pub const DRIVER_USER_SUBJECT: &str = "auth.user";
/// The subject type of a vehicle.
pub const VEHICLE_SUBJECT: &str = "inventory.asset";
/// The subject type of a client party.
pub const PARTY_SUBJECT: &str = "parties.party";

/// The days a blocking kind looks ahead for planned work (`DOC.RUL.02`).
pub const LOOK_AHEAD_DAYS: i64 = 30;

/// The days before expiry a blocking kind reminds (`DOC.RUL.01`).
pub const BLOCKING_REMINDERS: [i64; 3] = [90, 30, 7];

/// The days before expiry the other kinds remind.
pub const OTHER_REMINDERS: [i64; 3] = [60, 30, 7];

/// A document kind of 03 (§9 *Document blocks*).
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
    /// The guarded transitions it blocks (mode `block`); none for a kind that
    /// only reminds.
    pub blocks: &'static [&'static str],
    /// The role whose holders in the subject's scope get the expiry
    /// reminders (`DOC.RUL.01`) and the renewal task (`DOC.RUL.03`).
    pub role: &'static str,
    /// A condition on the guarded record: the field and the values that make
    /// the blocks apply (the rider permit is for a motorbike rider only).
    pub when: Option<(&'static str, &'static [&'static str])>,
}

impl KindSpec {
    /// Whether the kind is a legal block: it blocks something, with the
    /// override mode `never` that nobody can change until the lawyer answers
    /// `L-19`.
    pub fn is_legal_block(&self) -> bool {
        !self.blocks.is_empty()
    }
}

/// The kinds 03 seeds. The first five block `run.dispatch` (the licence and
/// the permit also `delivery.assign`) as legal blocks, override `never`
/// (`⚠ open: lawyer (L-19)`); the others only remind.
pub const KINDS: [KindSpec; 8] = [
    KindSpec {
        key: DRIVING_LICENCE,
        name_en: "Driving licence",
        name_ar: "رخصة القيادة",
        subjects: &[EMPLOYEE_SUBJECT, DRIVER_USER_SUBJECT],
        blocks: &[ASSIGN_GUARDED, DISPATCH_GUARDED],
        role: OPERATIONS_MANAGER_ROLE,
        when: None,
    },
    KindSpec {
        key: RIDER_PERMIT,
        name_en: "RTA rider permit and qualification certificate",
        name_ar: "تصريح الدراجة النارية وشهادة التأهيل",
        subjects: &[EMPLOYEE_SUBJECT, DRIVER_USER_SUBJECT],
        blocks: &[ASSIGN_GUARDED, DISPATCH_GUARDED],
        role: OPERATIONS_MANAGER_ROLE,
        when: Some(("vehicle_type", &["motorbike"])),
    },
    KindSpec {
        key: MULKIYA,
        name_en: "Mulkiya",
        name_ar: "ملكية المركبة",
        subjects: &[VEHICLE_SUBJECT],
        blocks: &[DISPATCH_GUARDED],
        role: FLEET_MANAGER_ROLE,
        when: None,
    },
    KindSpec {
        key: VEHICLE_INSURANCE,
        name_en: "Vehicle insurance",
        name_ar: "تأمين المركبة",
        subjects: &[VEHICLE_SUBJECT],
        blocks: &[DISPATCH_GUARDED],
        role: FLEET_MANAGER_ROLE,
        when: None,
    },
    KindSpec {
        key: VEHICLE_PERMIT,
        name_en: "Asateel or other vehicle permit",
        name_ar: "تصريح أساطيل أو تصريح مركبة آخر",
        subjects: &[VEHICLE_SUBJECT],
        blocks: &[DISPATCH_GUARDED],
        role: FLEET_MANAGER_ROLE,
        when: None,
    },
    KindSpec {
        key: FREIGHT_DOCUMENTS,
        name_en: "Freight shipping and customs documents",
        name_ar: "مستندات الشحن والتخليص الجمركي",
        subjects: &["logistics.freight_job"],
        blocks: &[],
        role: DOCUMENTATION_OFFICER_ROLE,
        when: None,
    },
    KindSpec {
        key: CLIENT_CONTRACT,
        name_en: "Client contract",
        name_ar: "عقد العميل",
        subjects: &[PARTY_SUBJECT],
        blocks: &[],
        role: ACCOUNTANT_ROLE,
        when: None,
    },
    KindSpec {
        key: RATE_CARD_AGREEMENT,
        name_en: "Rate card agreement",
        name_ar: "اتفاقية بطاقة الأسعار",
        subjects: &[PARTY_SUBJECT],
        blocks: &[],
        role: ACCOUNTANT_ROLE,
        when: None,
    },
];

/// The rules 03 marks compliance-critical (§7.2): `LOG.RUL.10` (expired
/// vehicle documents ground the vehicle), `DOC.RUL.03` for the blocking kinds
/// above, and `WRK.RUL.15` (the midday break).
pub const CRITICAL_RULES: [&str; 3] = ["LOG.RUL.10", "DOC.RUL.03", "WRK.RUL.15"];

/// The rule that grounds a vehicle whose documents expired.
pub const GROUND_RULE: &str = "LOG.RUL.10";
/// When it runs, Asia/Dubai.
pub const GROUND_RULE_AT: &str = "00:30";

// -------------------------------------------------------------- integrations

/// The key of the SMS gateway connection (tracking SMS to recipients).
pub const SMS_GATEWAY: &str = "sms.gateway";

/// The fallback of the SMS gateway (§8 *Integration down*): tracking SMS wait
/// in the queue, the dispatcher calls the recipients on today's runs, and the
/// clients are told by e-mail.
pub const SMS_FALLBACK_EN: &str = "Tracking SMS are queued and sent when the gateway is back. The dispatcher calls the recipients on today's runs (the task lists them) and the clients are told by e-mail.";
/// See [`SMS_FALLBACK_EN`].
pub const SMS_FALLBACK_AR: &str = "تبقى رسائل التتبع في الطابور وتُرسل عند عودة البوابة. يتصل المنسّق بمستلمي رحلات اليوم (تعرضهم المهمة) ويُبلَّغ العملاء بالبريد الإلكتروني.";

/// The fallback of the e-invoicing ASP (§8): invoices are issued and sent as
/// PDF; the transmissions wait and are sent when it recovers, and
/// `FIN.RUL.16` raises a task after 24 hours.
pub const ASP_FALLBACK_EN: &str = "Issue the invoices and send them to the client as PDF; the transmissions stay queued and are sent when the ASP recovers.";
/// See [`ASP_FALLBACK_EN`].
pub const ASP_FALLBACK_AR: &str = "أصدر الفواتير وأرسلها للعميل بصيغة PDF؛ تبقى عمليات الإرسال في الطابور وتُرسل عند عودة مزود الخدمة.";

/// The months from the entity's go-live during which an invoice for
/// international goods transport with an air waybill is left out of the
/// e-invoicing mandate (§9 *E-invoicing*); sea and road freight are not.
pub const EINVOICE_EXCLUSION_MONTHS: u32 = 24;

// --------------------------------------------------------------- late entry

/// The late-entry entity of a driver's e-POD entered after the stop (§4.1
/// *Driver without signal at a stop*).
pub const EPOD_LATE_ENTRY: &str = "delivery.pod";
/// `M` of `PLT.ACL.10`: minutes of delay above which an e-POD is flagged with
/// a reason.
pub const LATE_ENTRY_FLAG_MINUTES: i64 = 30;
/// `N` of `PLT.ACL.10`: hours of delay above which an e-POD needs an
/// operations manager's approval.
pub const LATE_ENTRY_APPROVAL_HOURS: i64 = 24;

// -------------------------------------------------------------------- sales

/// The lead sources 03 uses (`SAL.CRM.02`), by `wl-sales` code. The module's
/// *walk-in* and *aggregator* sources are turned off by the seed.
pub const LEAD_SOURCES: [&str; 7] = [
    "referral", "website", "social", "paid_ads", "partner", "event", "other",
];

/// The campaign channels of 03 as (03's name, the `wl-sales` marketing channel
/// it is recorded as). The module's list is fixed: LinkedIn and marketplaces
/// are recorded as *other online*, trade events as *event*.
pub const CAMPAIGN_CHANNELS: [(&str, &str); 5] = [
    ("LinkedIn", "other_online"),
    ("Search ads", "google"),
    ("Email", "email"),
    ("Trade events", "event"),
    ("Marketplaces", "other_online"),
];

// --------------------------------------------------------------- onboarding

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
    /// The steps.
    pub steps: &'static [StepSpec],
}

const EMPLOYEE_STEP: StepSpec = ("Upload your personal documents", "employee", 3, "task");
const SOP_STEP: StepSpec = ("SOPs acknowledged", "employee", 7, "sops");
const WELCOME_STEP: StepSpec = ("Welcome and introductions", "manager", 1, "task");

/// The checklists 03 seeds (`HRM.RUL.16`): started when the employee record
/// is created in `onboarding`; the operations manager, the line manager (the
/// dispatcher of the depot) and the employee get tasks; completion blocks
/// nothing (run dispatch stays guarded by the driving licence).
pub const ONBOARDING: [OnboardingSpec; 4] = [
    OnboardingSpec {
        name: "Driver onboarding",
        role: DRIVER_ROLE,
        steps: &[
            (
                "Licence and documents",
                "role:operations_manager",
                2,
                "task",
            ),
            WELCOME_STEP,
            ("Vehicle handover", "manager", 2, "task"),
            ("PWA and e-POD training", "manager", 3, "task"),
            (
                "Cash handover training",
                "role:operations_manager",
                3,
                "task",
            ),
            EMPLOYEE_STEP,
            SOP_STEP,
        ],
    },
    OnboardingSpec {
        name: "Hub staff onboarding",
        role: HUB_STAFF_ROLE,
        steps: &[
            ("Documents and access", "role:operations_manager", 2, "task"),
            WELCOME_STEP,
            ("Scanning and sorting walk-through", "manager", 3, "task"),
            EMPLOYEE_STEP,
            SOP_STEP,
        ],
    },
    OnboardingSpec {
        name: "Dispatcher onboarding",
        role: DISPATCHER_ROLE,
        steps: &[
            ("Documents and access", "role:operations_manager", 2, "task"),
            WELCOME_STEP,
            (
                "Run planning walk-through",
                "role:operations_manager",
                3,
                "task",
            ),
            EMPLOYEE_STEP,
            SOP_STEP,
        ],
    },
    OnboardingSpec {
        name: "Customer service onboarding",
        role: CUSTOMER_SERVICE_ROLE,
        steps: &[
            ("Documents and access", "role:operations_manager", 2, "task"),
            WELCOME_STEP,
            ("Claims and complaints walk-through", "manager", 3, "task"),
            EMPLOYEE_STEP,
            SOP_STEP,
        ],
    },
];

/// The role that is the line manager of the staff above: the dispatcher of
/// the depot.
pub const LINE_MANAGER_ROLE: &str = DISPATCHER_ROLE;

// --------------------------------------------------------------------- SOPs

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
    /// The roles that must read and acknowledge it (per depot).
    pub readers: &'static [&'static str],
}

/// The SOP library 03 seeds: the operations manager owns dispatch, delivery,
/// cash handover and the hub, the fleet manager the daily vehicle check.
/// `DOC.RUL.04` asks each reader to acknowledge within 7 days, reminds every
/// 3 days and tells the reader's manager and the SOP's owner role after 7
/// days overdue (the module's defaults).
pub const SOPS: [SopSpec; 5] = [
    SopSpec {
        code: "SOP-LOG-DISPATCH",
        title: "Dispatch and runs",
        category: "operations",
        owner_role: OPERATIONS_MANAGER_ROLE,
        readers: &[DISPATCHER_ROLE, DRIVER_ROLE],
    },
    SopSpec {
        code: "SOP-LOG-DELIVERY",
        title: "Delivery and proof of delivery",
        category: "operations",
        owner_role: OPERATIONS_MANAGER_ROLE,
        readers: &[DRIVER_ROLE, DISPATCHER_ROLE, CUSTOMER_SERVICE_ROLE],
    },
    SopSpec {
        code: "SOP-LOG-CASH",
        title: "Cash handover",
        category: "finance",
        owner_role: OPERATIONS_MANAGER_ROLE,
        readers: &[DRIVER_ROLE, CASHIER_ROLE],
    },
    SopSpec {
        code: "SOP-LOG-HUB",
        title: "Hub operations",
        category: "operations",
        owner_role: OPERATIONS_MANAGER_ROLE,
        readers: &[HUB_STAFF_ROLE],
    },
    SopSpec {
        code: "SOP-LOG-VEHICLE-CHECK",
        title: "Daily vehicle check",
        category: "fleet",
        owner_role: FLEET_MANAGER_ROLE,
        readers: &[DRIVER_ROLE],
    },
];

// ------------------------------------------------------------ registrations

/// The reminder days of the registration kinds (`DOC.RUL.01`).
pub const TRADEMARK_REMINDERS: [i64; 3] = [180, 90, 30];
/// See [`TRADEMARK_REMINDERS`].
pub const OTHER_CERTIFICATION_REMINDERS: [i64; 3] = [90, 30, 7];

// ----------------------------------------------------- delivery, cash, money

/// The COD difference accepted without approval, in fils (`WRK.ACL.09`).
pub const COD_TOLERANCE: i64 = 0;
/// The minutes the approver has to answer a COD difference before the attempt
/// fails.
pub const COD_APPROVAL_MINUTES: i64 = 10;
/// The days a shipment may stay untraced before it is marked lost
/// (`WRK.RUL.17`, task after 3 days and escalation after 7: see `trace`).
pub const UNTRACED_DAYS: i64 = 7;
/// The abort reasons 03 adds to the module's list.
pub const EXTRA_ABORT_REASONS: [&str; 1] = ["driver_unreachable"];
/// The failure reasons 03 lists besides the module's standard ones; all five
/// are the module's seeded defaults.
pub const FAILURE_REASONS: [&str; 5] = [
    "refused_to_pay",
    "no_cash",
    "cancelled_by_client",
    "run_aborted",
    "wrong_address",
];
/// The cash shortage written off without a second approver, in fils
/// (`FIN.ACL.06`, AED 50).
pub const CASH_WRITE_OFF_LIMIT: i64 = 5_000;
/// Recovering a cash shortage by salary deduction (`FIN.ACL.19`): off until
/// the lawyer answers `L-20`; a setting, not a rule of code.
pub const SALARY_DEDUCTION: bool = false;
/// The write-off limit of an invoice balance, in fils (`FIN.ACL.15`): every
/// write-off is approved.
pub const INVOICE_WRITE_OFF_LIMIT: i64 = 0;
/// The roles whose holders approve an invoice write-off.
pub const WRITE_OFF_APPROVERS: [&str; 2] = [OPERATIONS_MANAGER_ROLE, wl_kernel::COMPANY_ADMIN_ROLE];
/// The days a dispute stays open before it is escalated (`FIN.RUL.15`).
pub const DISPUTE_ESCALATION_DAYS: u32 = 14;
/// The projected balance below which `FIN.RUL.23` alerts, in fils
/// (AED 100,000).
pub const CASH_THRESHOLD: i64 = 10_000_000;
/// The weeks the cash-flow forecast looks ahead.
pub const CASH_FORECAST_WEEKS: u32 = 13;
/// When the cash-flow alert is checked, Asia/Dubai.
pub const CASH_ALERT_TIME: &str = "06:00";
/// The days after the end of a client's contract during which its portal
/// users keep read-only access (`POR.ACL.01`); the portal closes after them.
pub const PORTAL_CLOSES_AFTER_DAYS: i64 = 90;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_five_blocking_kinds_are_legal_blocks_and_the_others_only_remind() {
        let blocking: Vec<&str> = KINDS
            .iter()
            .filter(|kind| kind.is_legal_block())
            .map(|kind| kind.key)
            .collect();
        assert_eq!(
            blocking,
            [
                DRIVING_LICENCE,
                RIDER_PERMIT,
                MULKIYA,
                VEHICLE_INSURANCE,
                VEHICLE_PERMIT
            ]
        );
        for kind in KINDS
            .iter()
            .filter(|kind| kind.subjects == [VEHICLE_SUBJECT])
        {
            assert_eq!(kind.blocks, [DISPATCH_GUARDED], "{}", kind.key);
            assert_eq!(kind.role, FLEET_MANAGER_ROLE);
        }
    }

    #[test]
    fn every_onboarding_step_has_a_valid_owner_and_ends_with_the_sops() {
        for spec in &ONBOARDING {
            assert_eq!(spec.steps.last().map(|step| step.3), Some("sops"));
            for step in spec.steps {
                assert!(
                    matches!(step.1, "hr" | "manager" | "employee") || step.1.starts_with("role:"),
                    "{}",
                    step.1
                );
            }
        }
    }
}
