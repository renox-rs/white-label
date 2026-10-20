//! The 01 values for the shared compliance, document, notice, sales and staff
//! mechanisms (01 Property §4.5, §7.2, §9; BL-PRP-27).
//!
//! The mechanisms live in the shared modules (`wl-documents`, `wl-ticketing`,
//! `wl-notify`, `wl-sales`, `wl-hr`) and the vertical reaches them only through
//! `wl-kernel`. This file is therefore plain data: which document kinds 01
//! seeds and what they block, which ticket categories, which onboarding and
//! SOP lists, which lead sources. The host (`host/src/app/seed.rs` of the
//! client app) turns these tables into the modules' own seeds and builder
//! settings, and `/settings/negative-flow` shows each value with its spec
//! reference. What this file does itself is register the compliance-critical
//! rules the shared modules do not mark ([`register_critical_rules`]) and the
//! guarded transition of the compliance certificates.

use wl_kernel::compliance::CriticalRule;

/// The key of the category *fire and life safety* (compliance-critical).
pub const FIRE_LIFE_SAFETY: &str = "fire_life_safety";
/// The key of the category of a repair charged to the tenant (chargeable,
/// invoiced when the ticket closes).
pub const TENANT_REPAIR: &str = "tenant_repair";
/// The key of the category *lifts* (compliance-critical).
pub const LIFTS: &str = "lifts";
/// The compliance-critical ticket categories (`WRK.RUL.14`, `WRK.RUL.18`).
pub const COMPLIANCE_CATEGORIES: [&str; 2] = [FIRE_LIFE_SAFETY, LIFTS];
/// The categories the Civil Defence approval guards: fire and life safety
/// only (lifts have no Civil Defence approval).
pub const CIVIL_DEFENCE_CATEGORIES: &[&str] = &[FIRE_LIFE_SAFETY];

/// The role of the maintenance coordinator.
pub const COORDINATOR_ROLE: &str = "coordinator";
/// The role of the property manager.
pub const PROPERTY_MANAGER: &str = crate::permissions::PROPERTY_MANAGER_ROLE;
/// The role of the company administrators (the owner role by default).
pub const COMPANY_ADMIN: &str = wl_kernel::COMPANY_ADMIN_ROLE;

/// The key of the vendor's Civil Defence approval.
pub const CIVIL_DEFENCE_APPROVAL: &str = "civil_defence_approval";
/// The key of the building safety certificate.
pub const BUILDING_SAFETY_CERTIFICATE: &str = "building_safety_certificate";
/// The key of the lift certificate.
pub const LIFT_CERTIFICATE: &str = "lift_certificate";

/// The transition of a building that nothing runs: the compliance
/// certificates carry one `flag`-mode entry on it so that `DOC.RUL.03` takes
/// them up on expiry (it handles only kinds with `blocks`) while nothing is
/// ever blocked. Property registers the `BlockSubjects` provider that maps a
/// building to itself ([`crate::compliance::register`]).
pub const CERTIFY: &str = "building.certify";

/// Whether an entry stops the transition or only marks the record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The transition is refused.
    Block,
    /// The transition runs and the record is flagged.
    Flag,
}

/// One `blocks` entry of a kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockSpec {
    /// The guarded transition, `<entity>.<transition>`.
    pub guarded: &'static str,
    /// Block or flag.
    pub mode: Mode,
    /// The field and values the guarded record must have; `None` for every
    /// record.
    pub condition: Option<(&'static str, &'static [&'static str])>,
}

/// A document kind 01 seeds (§7.2 *Document kinds and responsible roles*).
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
    /// Whether it has an expiry date.
    pub expires: bool,
    /// Whether it has a number.
    pub numbered: bool,
    /// The days before expiry to remind (`DOC.RUL.01`).
    pub reminders: &'static [i64],
    /// Whether the party the document is about is reminded as well as the
    /// staff (a tenant's identity documents).
    pub notify_party: bool,
    /// The responsible role (6a).
    pub role: &'static str,
    /// The roles told on expiry as well as the responsible role.
    pub also_notify: &'static [&'static str],
    /// Identity documents (sensitivity *personal*, `PRP.ACL.10`).
    pub personal: bool,
    /// A legal block: override `never`, compliance-critical, entries locked.
    pub legal_block: bool,
    /// Compliance-critical (the owner role is told on expiry).
    pub critical: bool,
    /// The look-ahead days of `DOC.RUL.02`.
    pub look_ahead: i64,
    /// The transitions it guards.
    pub blocks: &'static [BlockSpec],
}

const PARTY: &str = wl_kernel::parties::PARTY_SUBJECT_TYPE;
const LEASE: &str = crate::lease::LEASE_SUBJECT_TYPE;
const AGREEMENT: &str = crate::agreement::AGREEMENT_SUBJECT_TYPE;
const BUILDING: &str = crate::building::BUILDING_SUBJECT_TYPE;
const SERVICE_CONTRACT: &str = wl_kernel::tickets::SERVICE_CONTRACT_SUBJECT_TYPE;
const TICKET: &str = wl_kernel::tickets::TICKET_SUBJECT_TYPE;

/// The condition of the Civil Defence entries: the category is fire and life
/// safety.
const FIRE_CONDITION: Option<(&str, &[&str])> = Some(("category", CIVIL_DEFENCE_CATEGORIES));

/// The condition of the identity entries on `lease.submit`.
const PERSON: Option<(&str, &[&str])> = Some((crate::documents::CONDITION_FIELD, PERSON_VALUES));
const PERSON_VALUES: &[&str] = &["person"];
const COMPANY_VALUES: &[&str] = &["company"];
/// The condition of the company entries on `lease.submit`.
const COMPANY: Option<(&str, &[&str])> = Some((crate::documents::CONDITION_FIELD, COMPANY_VALUES));

const KYC_REMINDERS: &[i64] = &[90, 30, 7];

/// The kinds 01 seeds. The registration kinds (*Trademark registration*,
/// *Other certification*) are wl-documents' own and are listed in
/// [`REGISTRATION_KINDS`].
pub const KINDS: &[KindSpec] = &[
    KindSpec {
        key: "emirates_id",
        name_en: "Emirates ID",
        name_ar: "الهوية الإماراتية",
        subjects: &[PARTY],
        expires: true,
        numbered: true,
        reminders: KYC_REMINDERS,
        notify_party: true,
        role: PROPERTY_MANAGER,
        also_notify: &[],
        personal: true,
        legal_block: false,
        critical: false,
        look_ahead: 0,
        blocks: &[BlockSpec {
            guarded: "lease.submit",
            mode: Mode::Block,
            condition: PERSON,
        }],
    },
    KindSpec {
        key: "passport",
        name_en: "Passport",
        name_ar: "جواز السفر",
        subjects: &[PARTY],
        expires: true,
        numbered: true,
        reminders: KYC_REMINDERS,
        notify_party: true,
        role: PROPERTY_MANAGER,
        also_notify: &[],
        personal: true,
        legal_block: false,
        critical: false,
        look_ahead: 0,
        blocks: &[BlockSpec {
            guarded: "lease.submit",
            mode: Mode::Block,
            condition: PERSON,
        }],
    },
    KindSpec {
        key: "residence_visa",
        name_en: "Residence visa",
        name_ar: "تأشيرة الإقامة",
        subjects: &[PARTY],
        expires: true,
        numbered: true,
        reminders: KYC_REMINDERS,
        notify_party: true,
        role: PROPERTY_MANAGER,
        also_notify: &[],
        personal: true,
        legal_block: false,
        critical: false,
        look_ahead: 0,
        blocks: &[BlockSpec {
            guarded: "lease.submit",
            mode: Mode::Block,
            condition: PERSON,
        }],
    },
    KindSpec {
        key: "trade_licence",
        name_en: "Trade licence",
        name_ar: "الرخصة التجارية",
        subjects: &[PARTY],
        expires: true,
        numbered: true,
        reminders: KYC_REMINDERS,
        notify_party: true,
        role: PROPERTY_MANAGER,
        also_notify: &[],
        personal: false,
        legal_block: false,
        critical: false,
        look_ahead: 0,
        blocks: &[BlockSpec {
            guarded: "lease.submit",
            mode: Mode::Block,
            condition: COMPANY,
        }],
    },
    KindSpec {
        key: "trn_certificate",
        name_en: "TRN certificate",
        name_ar: "شهادة التسجيل الضريبي",
        subjects: &[PARTY],
        expires: false,
        numbered: true,
        reminders: &[],
        notify_party: true,
        role: PROPERTY_MANAGER,
        also_notify: &[],
        personal: false,
        legal_block: false,
        critical: false,
        look_ahead: 0,
        blocks: &[BlockSpec {
            guarded: "lease.submit",
            mode: Mode::Block,
            condition: COMPANY,
        }],
    },
    KindSpec {
        key: "signed_tenancy_contract",
        name_en: "Signed tenancy contract",
        name_ar: "عقد الإيجار الموقع",
        subjects: &[LEASE],
        expires: false,
        numbered: false,
        reminders: &[],
        notify_party: false,
        role: PROPERTY_MANAGER,
        also_notify: &[],
        personal: false,
        legal_block: false,
        critical: false,
        look_ahead: 0,
        blocks: &[BlockSpec {
            guarded: "lease.activate",
            mode: Mode::Block,
            condition: None,
        }],
    },
    KindSpec {
        key: "ejari_certificate",
        name_en: "Ejari / Tawtheeq certificate",
        name_ar: "شهادة إيجاري / توثيق",
        subjects: &[LEASE],
        expires: true,
        numbered: true,
        reminders: KYC_REMINDERS,
        notify_party: false,
        role: PROPERTY_MANAGER,
        also_notify: &[],
        personal: false,
        legal_block: false,
        critical: false,
        look_ahead: 0,
        blocks: &[BlockSpec {
            guarded: "lease.record_ejari",
            mode: Mode::Block,
            condition: None,
        }],
    },
    KindSpec {
        key: crate::terms::TENANT_ACCEPTANCE_KIND,
        name_en: "Tenant acceptance of changed terms",
        name_ar: "موافقة المستأجر على الشروط المعدلة",
        subjects: &[LEASE],
        expires: false,
        numbered: false,
        reminders: &[],
        notify_party: false,
        role: PROPERTY_MANAGER,
        also_notify: &[],
        personal: false,
        legal_block: false,
        critical: false,
        look_ahead: 0,
        blocks: &[],
    },
    KindSpec {
        key: crate::force::INSPECTION_KIND,
        name_en: "Abandonment inspection (with photos)",
        name_ar: "معاينة الهجر (مع الصور)",
        subjects: &[LEASE],
        expires: false,
        numbered: false,
        reminders: &[],
        notify_party: false,
        role: PROPERTY_MANAGER,
        also_notify: &[],
        personal: false,
        legal_block: false,
        critical: false,
        look_ahead: 0,
        blocks: &[],
    },
    KindSpec {
        key: crate::force::REPOSSESSION_BASIS_KIND,
        name_en: "Repossession basis",
        name_ar: "أساس استرداد الحيازة",
        subjects: &[LEASE],
        expires: false,
        numbered: false,
        reminders: &[],
        notify_party: false,
        role: PROPERTY_MANAGER,
        also_notify: &[],
        personal: false,
        legal_block: false,
        critical: false,
        look_ahead: 0,
        blocks: &[],
    },
    KindSpec {
        key: crate::force::JUDGMENT_KIND,
        name_en: "Judgment",
        name_ar: "الحكم",
        subjects: &[LEASE],
        expires: false,
        numbered: false,
        reminders: &[],
        notify_party: false,
        role: PROPERTY_MANAGER,
        also_notify: &[],
        personal: false,
        legal_block: false,
        critical: false,
        look_ahead: 0,
        blocks: &[],
    },
    KindSpec {
        key: crate::force::EXECUTION_RECORD_KIND,
        name_en: "Execution record",
        name_ar: "محضر التنفيذ",
        subjects: &[LEASE],
        expires: false,
        numbered: false,
        reminders: &[],
        notify_party: false,
        role: PROPERTY_MANAGER,
        also_notify: &[],
        personal: false,
        legal_block: false,
        critical: false,
        look_ahead: 0,
        blocks: &[],
    },
    KindSpec {
        key: crate::agreement_lifecycle::SIGNED_AGREEMENT_KIND,
        name_en: "Signed management agreement",
        name_ar: "اتفاقية الإدارة الموقعة",
        subjects: &[AGREEMENT],
        expires: false,
        numbered: false,
        reminders: &[],
        notify_party: false,
        role: PROPERTY_MANAGER,
        also_notify: &[],
        personal: false,
        legal_block: false,
        critical: false,
        look_ahead: 0,
        blocks: &[BlockSpec {
            guarded: "agreement.activate",
            mode: Mode::Block,
            condition: None,
        }],
    },
    // The vendor's Civil Defence approval: a legal block (override `never`),
    // compliance-critical, 30 days look-ahead (row 3). Everything it guards
    // is conditioned on the fire and life safety category.
    KindSpec {
        key: CIVIL_DEFENCE_APPROVAL,
        name_en: "Civil Defence approval",
        name_ar: "موافقة الدفاع المدني",
        subjects: &[PARTY],
        expires: true,
        numbered: true,
        reminders: &[90, 30, 7],
        notify_party: false,
        role: COORDINATOR_ROLE,
        also_notify: &[PROPERTY_MANAGER],
        personal: false,
        legal_block: true,
        critical: true,
        look_ahead: 30,
        blocks: &[
            BlockSpec {
                guarded: "service_contract.activate",
                mode: Mode::Block,
                condition: FIRE_CONDITION,
            },
            BlockSpec {
                guarded: "ticket.resolve",
                mode: Mode::Block,
                condition: FIRE_CONDITION,
            },
        ],
    },
    // The compliance certificates of a building (row 6): compliance-critical,
    // 90, 30 and 7 days to the coordinator and the property manager; the one
    // `flag` entry on [`CERTIFY`] lets `DOC.RUL.03` take them up on expiry
    // and blocks nothing.
    KindSpec {
        key: BUILDING_SAFETY_CERTIFICATE,
        name_en: "Building safety certificate",
        name_ar: "شهادة سلامة المبنى",
        subjects: &[BUILDING],
        expires: true,
        numbered: true,
        reminders: &[90, 30, 7],
        notify_party: false,
        role: COORDINATOR_ROLE,
        also_notify: &[PROPERTY_MANAGER],
        personal: false,
        legal_block: false,
        critical: true,
        look_ahead: 0,
        blocks: &[BlockSpec {
            guarded: CERTIFY,
            mode: Mode::Flag,
            condition: None,
        }],
    },
    KindSpec {
        key: LIFT_CERTIFICATE,
        name_en: "Lift certificate",
        name_ar: "شهادة المصاعد",
        subjects: &[BUILDING],
        expires: true,
        numbered: true,
        reminders: &[90, 30, 7],
        notify_party: false,
        role: COORDINATOR_ROLE,
        also_notify: &[PROPERTY_MANAGER],
        personal: false,
        legal_block: false,
        critical: true,
        look_ahead: 0,
        blocks: &[BlockSpec {
            guarded: CERTIFY,
            mode: Mode::Flag,
            condition: None,
        }],
    },
    KindSpec {
        key: "service_contract",
        name_en: "Service contract",
        name_ar: "عقد الخدمة",
        subjects: &[SERVICE_CONTRACT],
        expires: true,
        numbered: false,
        reminders: &[60, 30, 7],
        notify_party: false,
        role: COORDINATOR_ROLE,
        also_notify: &[],
        personal: false,
        legal_block: false,
        critical: false,
        look_ahead: 0,
        blocks: &[],
    },
    KindSpec {
        key: "service_report",
        name_en: "Service report",
        name_ar: "تقرير الخدمة",
        subjects: &[TICKET],
        expires: false,
        numbered: false,
        reminders: &[],
        notify_party: false,
        role: COORDINATOR_ROLE,
        also_notify: &[],
        personal: false,
        legal_block: false,
        critical: false,
        look_ahead: 0,
        blocks: &[],
    },
];

/// The registration and certification kinds on in 01 (row 22): wl-documents
/// seeds them; the responsible role is the company admin (the owner role).
pub const REGISTRATION_KINDS: [&str; 2] = ["trademark_registration", "other_certification"];

/// The reminder days of the registration kinds (`DOC.RUL.01`, row 22).
pub const TRADEMARK_REMINDERS: [i64; 3] = [180, 90, 30];
/// See [`TRADEMARK_REMINDERS`].
pub const OTHER_CERTIFICATION_REMINDERS: [i64; 3] = [90, 30, 7];

/// A ticket category 01 seeds (rows 7 and 14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategorySpec {
    /// The key.
    pub key: &'static str,
    /// The English name.
    pub name_en: &'static str,
    /// The Arabic name.
    pub name_ar: &'static str,
    /// Compliance-critical (`WRK.RUL.14`, `WRK.RUL.18`, the building flags);
    /// its tickets start at high priority and its visits cannot be cancelled
    /// at the end of a contract, only reassigned.
    pub critical: bool,
    /// Chargeable to the tenant: its tickets get charge lines and are invoiced
    /// to the tenant when they close (`confirm` or `close`), not at `resolve`
    /// (`BL-PRP-26` item 7).
    pub chargeable: bool,
}

/// The ticket categories 01 seeds. `billing_dispute` is the platform's own
/// (wl-ticketing seeds it with invoicing) and is not repeated.
pub const CATEGORIES: [CategorySpec; 6] = [
    CategorySpec {
        key: FIRE_LIFE_SAFETY,
        name_en: "Fire and life safety",
        name_ar: "الحماية من الحريق وسلامة الأرواح",
        critical: true,
        chargeable: false,
    },
    CategorySpec {
        key: LIFTS,
        name_en: "Lifts",
        name_ar: "المصاعد",
        critical: true,
        chargeable: false,
    },
    CategorySpec {
        key: crate::notice::MOVE_OUT_CATEGORY,
        name_en: "Move-out inspection",
        name_ar: "فحص الإخلاء",
        critical: false,
        chargeable: false,
    },
    CategorySpec {
        key: crate::legal_case::LEGAL_CASE_CATEGORY,
        name_en: "Legal case",
        name_ar: "قضية قانونية",
        critical: false,
        chargeable: false,
    },
    CategorySpec {
        key: crate::statement_dispute::DISPUTE_CATEGORY,
        name_en: "Owner statement dispute",
        name_ar: "نزاع على كشف حساب المالك",
        critical: false,
        chargeable: false,
    },
    CategorySpec {
        key: TENANT_REPAIR,
        name_en: "Repair charged to the tenant",
        name_ar: "إصلاح على حساب المستأجر",
        critical: false,
        chargeable: true,
    },
];

/// `WRK.RUL.12`: days before a service contract's end date (row 8).
pub const CONTRACT_EXPIRING_DAYS: i64 = 60;
/// `WRK.RUL.14`: days after a planned visit's due date (row 9).
pub const VISIT_MISSED_DAYS: i64 = 3;
/// `WRK.RUL.18`: working days to reassign or cancel (row 10).
pub const CONTRACT_ENDED_TASK_DAYS: i64 = 1;
/// `WRK.RUL.19`: hours after the scheduled visit time (row 11).
pub const NO_SHOW_HOURS: i64 = 2;
/// The reassignment reasons (row 12).
pub const REASSIGN_REASONS: [&str; 3] = [
    "vendor_no_show",
    "contract_ended",
    "vendor_document_expired",
];

/// `PRP.ACL.14`: the annual value above which a service contract needs a
/// property manager's approval, in fils (AED 50,000; row 13).
pub const SERVICE_CONTRACT_MANAGER_ABOVE: i64 = 5_000_000;
/// `PRP.ACL.14`: the annual value above which the owner's approval is
/// recorded, in fils (AED 200,000).
pub const SERVICE_CONTRACT_OWNER_ABOVE: i64 = 20_000_000;

/// The lead sources 01 uses (row 18), by `wl-sales` code. The module's ninth
/// source, `aggregator`, is not used by 01 and is turned off by the seed.
pub const LEAD_SOURCES: [&str; 8] = [
    "referral", "walk_in", "website", "social", "paid_ads", "partner", "event", "other",
];

/// The campaign channels of 01 (row 19) as (01's name, the `wl-sales` marketing
/// channel it is recorded as). The module's list is fixed: listing portals are
/// recorded as *other online*, signboards as *radio or outdoor*.
pub const CAMPAIGN_CHANNELS: [(&str, &str); 6] = [
    ("Listing portals", "other_online"),
    ("Social media", "meta"),
    ("Search ads", "google"),
    ("Email", "email"),
    ("Signboards", "radio_outdoor"),
    ("Brokers", "partner"),
];

/// One step of an onboarding checklist (row 20): title, owner (`hr`,
/// `manager`, `employee` or `role:<key>`), due days after the start and kind
/// (`task`, `sops` or `checklist:<template>`).
pub type StepSpec = (&'static str, &'static str, i64, &'static str);

/// An onboarding checklist for one role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnboardingSpec {
    /// The template's name.
    pub name: &'static str,
    /// The job role it applies to.
    pub role: &'static str,
    /// The steps, in order.
    pub steps: &'static [StepSpec],
}

/// The steps every checklist starts with: the company admin sets up the
/// account, the line manager welcomes, the new staff member uploads their
/// documents and acknowledges the SOPs (`HRM.ACL.10`: only they record
/// their own steps).
const COMMON_STEPS: [StepSpec; 4] = [
    ("System account and access", "role:admin", 1, "task"),
    ("Welcome and introductions", "manager", 1, "task"),
    ("Upload your personal documents", "employee", 3, "task"),
    ("SOPs acknowledged", "employee", 7, "sops"),
];

/// The onboarding checklists 01 seeds, one per role (row 20). Completion
/// blocks nothing: role activation does not wait for them.
pub const ONBOARDING: [OnboardingSpec; 5] = [
    OnboardingSpec {
        name: "Property manager onboarding",
        role: "property_manager",
        steps: &[
            COMMON_STEPS[0],
            COMMON_STEPS[1],
            (
                "Portfolio and building handover",
                "manager",
                3,
                "checklist:portfolio_handover",
            ),
            COMMON_STEPS[2],
            COMMON_STEPS[3],
        ],
    },
    OnboardingSpec {
        name: "Leasing agent onboarding",
        role: "leasing_agent",
        steps: &[
            COMMON_STEPS[0],
            COMMON_STEPS[1],
            (
                "Availability, pricing and enquiry process",
                "manager",
                3,
                "checklist:leasing_induction",
            ),
            COMMON_STEPS[2],
            COMMON_STEPS[3],
        ],
    },
    OnboardingSpec {
        name: "Accountant onboarding",
        role: "accountant",
        steps: &[
            COMMON_STEPS[0],
            COMMON_STEPS[1],
            (
                "Cheques, deposits and owner statements walk-through",
                "manager",
                3,
                "checklist:finance_induction",
            ),
            COMMON_STEPS[2],
            COMMON_STEPS[3],
        ],
    },
    OnboardingSpec {
        name: "Maintenance coordinator onboarding",
        role: "coordinator",
        steps: &[
            COMMON_STEPS[0],
            COMMON_STEPS[1],
            (
                "Vendors, service contracts and compliance visits",
                "manager",
                3,
                "checklist:coordinator_induction",
            ),
            COMMON_STEPS[2],
            COMMON_STEPS[3],
        ],
    },
    OnboardingSpec {
        name: "Technician onboarding",
        role: "technician",
        steps: &[
            COMMON_STEPS[0],
            COMMON_STEPS[1],
            (
                "Workplace and safety induction",
                "manager",
                3,
                "checklist:workplace_induction",
            ),
            (
                "Uniform or PPE handed over",
                "manager",
                3,
                "checklist:ppe_handover",
            ),
            COMMON_STEPS[2],
            COMMON_STEPS[3],
        ],
    },
];

/// An SOP of the library (row 21) with its owner role and required readers.
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

/// The SOP library 01 seeds: the property manager owns leasing, move-in,
/// move-out and maintenance; the accountant owns cheques, deposits and owner
/// statements. `DOC.RUL.04` asks each reader to acknowledge within 7 days,
/// reminds every 3 days and tells the reader's manager and the owner role
/// after 7 days overdue (the module's defaults).
pub const SOPS: [SopSpec; 7] = [
    SopSpec {
        code: "SOP-PRP-LEASING",
        title: "Leasing",
        category: "leasing",
        owner_role: PROPERTY_MANAGER,
        readers: &["leasing_agent", "property_manager"],
    },
    SopSpec {
        code: "SOP-PRP-MOVE-IN",
        title: "Move-in",
        category: "leasing",
        owner_role: PROPERTY_MANAGER,
        readers: &["leasing_agent", "property_manager"],
    },
    SopSpec {
        code: "SOP-PRP-MOVE-OUT",
        title: "Move-out",
        category: "leasing",
        owner_role: PROPERTY_MANAGER,
        readers: &["property_manager", "coordinator"],
    },
    SopSpec {
        code: "SOP-PRP-MAINTENANCE",
        title: "Maintenance",
        category: "maintenance",
        owner_role: PROPERTY_MANAGER,
        readers: &["coordinator", "technician", "property_manager"],
    },
    SopSpec {
        code: "SOP-PRP-CHEQUES",
        title: "Cheques",
        category: "finance",
        owner_role: "accountant",
        readers: &["accountant"],
    },
    SopSpec {
        code: "SOP-PRP-DEPOSITS",
        title: "Deposits",
        category: "finance",
        owner_role: "accountant",
        readers: &["accountant", "property_manager"],
    },
    SopSpec {
        code: "SOP-PRP-OWNER-STATEMENTS",
        title: "Owner statements",
        category: "finance",
        owner_role: "accountant",
        readers: &["accountant"],
    },
];

/// The rules 01 marks compliance-critical (row 2) that the shared modules do
/// not mark themselves: wl-documents runs `DOC.RUL.01` to `DOC.RUL.03` nightly
/// without the kernel's rule runs, so they are marked for the owner alert
/// without a schedule to watch (a missed run cannot be seen for them).
/// `PRP.RUL.09` is marked by [`crate::renewal`], `WRK.RUL.14` and `WRK.RUL.18`
/// by wl-ticketing.
pub const DOCUMENT_RULES: [&str; 3] = ["DOC.RUL.01", "DOC.RUL.02", "DOC.RUL.03"];

/// Every rule row 2 lists.
pub const CRITICAL_RULES: [&str; 6] = [
    "PRP.RUL.09",
    "WRK.RUL.14",
    "WRK.RUL.18",
    "DOC.RUL.01",
    "DOC.RUL.02",
    "DOC.RUL.03",
];

/// Marks [`DOCUMENT_RULES`] compliance-critical.
pub(crate) fn register_critical_rules() {
    for rule in DOCUMENT_RULES {
        CriticalRule::new(rule).register();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(key: &str) -> &'static KindSpec {
        KINDS.iter().find(|k| k.key == key).expect("a kind")
    }

    #[test]
    fn the_civil_defence_approval_is_a_legal_block_on_resolve_and_activate() {
        let approval = kind(CIVIL_DEFENCE_APPROVAL);
        assert!(approval.legal_block && approval.critical);
        assert_eq!(approval.look_ahead, 30);
        assert_eq!(approval.role, COORDINATOR_ROLE);
        let guarded: Vec<&str> = approval.blocks.iter().map(|b| b.guarded).collect();
        assert_eq!(guarded, ["service_contract.activate", "ticket.resolve"]);
        for block in approval.blocks {
            assert_eq!(block.mode, Mode::Block);
            assert_eq!(block.condition, Some(("category", &[FIRE_LIFE_SAFETY][..])));
        }
    }

    #[test]
    fn the_compliance_certificates_block_nothing() {
        for key in CERTIFICATE_KEYS {
            let certificate = kind(key);
            assert!(certificate.critical && !certificate.legal_block);
            assert_eq!(certificate.reminders, [90, 30, 7]);
            assert_eq!(certificate.blocks.len(), 1);
            assert_eq!(certificate.blocks[0].mode, Mode::Flag);
            assert_eq!(certificate.blocks[0].guarded, CERTIFY);
        }
    }

    const CERTIFICATE_KEYS: [&str; 2] = [BUILDING_SAFETY_CERTIFICATE, LIFT_CERTIFICATE];

    #[test]
    fn the_certificate_kinds_are_the_ones_the_compliance_page_watches() {
        assert_eq!(CERTIFICATE_KEYS, crate::compliance::CERTIFICATE_KINDS);
    }

    #[test]
    fn no_other_kind_blocks_a_lease_on_a_compliance_certificate() {
        for spec in KINDS {
            for block in spec.blocks {
                if block.guarded.starts_with("lease.") {
                    assert!(!spec.critical, "{} would block leasing", spec.key);
                }
            }
        }
    }

    #[test]
    fn every_kind_has_a_responsible_role_and_unique_key() {
        let mut keys: Vec<&str> = KINDS.iter().map(|k| k.key).collect();
        assert!(
            KINDS
                .iter()
                .all(|k| !k.role.is_empty() && !k.name_ar.is_empty())
        );
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count);
    }

    #[test]
    fn the_responsible_roles_follow_6a() {
        for key in [
            "emirates_id",
            "passport",
            "residence_visa",
            "trade_licence",
            "trn_certificate",
            "signed_tenancy_contract",
            "ejari_certificate",
            "tenant_acceptance",
            "repossession_basis",
            "judgment",
            "execution_record",
            "management_agreement",
        ] {
            assert_eq!(kind(key).role, PROPERTY_MANAGER, "{key}");
        }
        for key in [
            "civil_defence_approval",
            "building_safety_certificate",
            "lift_certificate",
            "service_contract",
            "service_report",
        ] {
            assert_eq!(kind(key).role, COORDINATOR_ROLE, "{key}");
        }
    }

    #[test]
    fn the_categories_are_the_two_critical_ones_and_the_three_fallbacks() {
        let critical: Vec<&str> = CATEGORIES
            .iter()
            .filter(|c| c.critical)
            .map(|c| c.key)
            .collect();
        assert_eq!(critical, COMPLIANCE_CATEGORIES);
        let others: Vec<&str> = CATEGORIES
            .iter()
            .filter(|c| !c.critical)
            .map(|c| c.key)
            .collect();
        assert_eq!(
            others,
            [
                "move_out_inspection",
                "legal_case",
                "owner_statement_dispute"
            ]
        );
    }

    #[test]
    fn there_is_a_checklist_for_each_role_and_each_ends_with_the_sops() {
        let roles: Vec<&str> = ONBOARDING.iter().map(|o| o.role).collect();
        assert_eq!(
            roles,
            [
                "property_manager",
                "leasing_agent",
                "accountant",
                "coordinator",
                "technician"
            ]
        );
        for spec in ONBOARDING {
            assert_eq!(spec.steps.last().map(|s| s.3), Some("sops"));
            assert!(spec.steps.iter().any(|s| s.1 == "employee"));
            assert!(spec.steps.iter().any(|s| s.1 == "role:admin"));
            assert!(spec.steps.iter().any(|s| s.1 == "manager"));
        }
    }

    #[test]
    fn the_sop_owners_are_the_property_manager_and_the_accountant() {
        let owners: Vec<(&str, &str)> = SOPS.iter().map(|s| (s.title, s.owner_role)).collect();
        assert_eq!(
            owners,
            [
                ("Leasing", "property_manager"),
                ("Move-in", "property_manager"),
                ("Move-out", "property_manager"),
                ("Maintenance", "property_manager"),
                ("Cheques", "accountant"),
                ("Deposits", "accountant"),
                ("Owner statements", "accountant"),
            ]
        );
        assert!(SOPS.iter().all(|s| !s.readers.is_empty()));
    }

    #[test]
    fn the_lead_sources_are_the_eight_of_the_spec() {
        assert_eq!(LEAD_SOURCES.len(), 8);
        assert!(!LEAD_SOURCES.contains(&"aggregator"));
    }
}
