//! The static values of 04 that the host turns into the modules' own seeds
//! (BL-ACC-23 wires them): the client KYC document kinds (§9 *Document
//! blocks*).
//!
//! Each KYC kind blocks engagement `activate` (`engagement.activate`, block
//! mode) with the override of `DOC.ACL.02` (the MLRO's approval, 72 hours),
//! for the parties whose role its condition names: the client company's
//! trade licence, and the passport and Emirates ID of each UBO
//! ([`crate::documents::PARTY_ROLE_FIELD`]).

/// One KYC document kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KycKind {
    /// The kind's key.
    pub key: &'static str,
    /// The English name.
    pub name_en: &'static str,
    /// The Arabic name.
    pub name_ar: &'static str,
    /// The roles of the screened parties that must hold it (`client`,
    /// `ubo`).
    pub roles: &'static [&'static str],
    /// Days before expiry the responsible role is reminded (look-ahead).
    pub look_ahead_days: i64,
}

/// The client company's trade licence.
pub const TRADE_LICENCE: &str = "trade_licence";
/// A UBO's passport.
pub const PASSPORT: &str = "passport";
/// A UBO's Emirates ID.
pub const EMIRATES_ID: &str = "emirates_id";

/// The KYC kinds of §9 *Document blocks*, renewed by the compliance officer
/// (MLRO), 30 days of look-ahead.
pub const KYC_KINDS: &[KycKind] = &[
    KycKind {
        key: TRADE_LICENCE,
        name_en: "Trade licence",
        name_ar: "الرخصة التجارية",
        roles: &["client"],
        look_ahead_days: 30,
    },
    KycKind {
        key: PASSPORT,
        name_en: "Passport",
        name_ar: "جواز السفر",
        roles: &["ubo"],
        look_ahead_days: 30,
    },
    KycKind {
        key: EMIRATES_ID,
        name_en: "Emirates ID",
        name_ar: "الهوية الإماراتية",
        roles: &["ubo"],
        look_ahead_days: 30,
    },
];

/// The responsible role of the registration and certification kinds
/// (*Trademark registration*, *Other certification*, `DOC.EXP.01`): the
/// partner (§9, BL-ACC-23).
pub const REGISTRATION_ROLE: &str = crate::permissions::PARTNER_ROLE;

/// The document kind of an FTA tax agent registration: a qualification of the
/// firm's staff (`HRM.QUA.01`), not a document of a party.
///
/// Decision: the Qualification records of `wl-hr` (BL-HRM-01) are not built,
/// so the kind cannot be seeded yet; this is the value its seed will take.
/// Filing already asks the kernel's `Qualifications` guard for it
/// ([`crate::filing`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaxAgentKind {
    /// The kind's key.
    pub key: &'static str,
    /// The guarded transition it blocks.
    pub blocks: &'static str,
    /// Days before expiry the responsible role is reminded (look-ahead).
    pub look_ahead_days: i64,
    /// The role that renews it.
    pub role: &'static str,
    /// Whether a block can be overridden: never, it is a legal block
    /// ([S5], [S16]).
    pub override_allowed: bool,
}

/// The FTA tax agent registration (§9 *Document blocks*).
pub const TAX_AGENT: TaxAgentKind = TaxAgentKind {
    key: crate::filing::TAX_AGENT_KIND,
    blocks: crate::documents::RETURN_FILE_GUARDED,
    look_ahead_days: 30,
    role: crate::permissions::PARTNER_ROLE,
    override_allowed: false,
};

/// The lead sources 04 uses (`SAL.CRM.02`), by `wl-sales` code. The module's
/// *walk-in* and *aggregator* sources are not used by an accounting firm and
/// are turned off by the seed.
pub const LEAD_SOURCES: [&str; 7] = [
    "referral", "website", "social", "paid_ads", "partner", "event", "other",
];

/// The campaign channels of 04 as (04's name, the `wl-sales` marketing channel
/// it is recorded as). The module's list is fixed: LinkedIn is recorded as
/// *other online*, seminars and webinars as *event*.
pub const CAMPAIGN_CHANNELS: [(&str, &str); 4] = [
    ("Search ads", "google"),
    ("LinkedIn", "other_online"),
    ("Email newsletters", "email"),
    ("Seminars and webinars", "event"),
];

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

/// The steps every checklist starts and ends with: the company admin sets up
/// the account, the line manager welcomes, the new member of staff uploads
/// their documents and acknowledges the SOPs (`HRM.ACL.10`: only they record
/// their own steps).
const COMMON_STEPS: [StepSpec; 4] = [
    ("System account and access", "role:admin", 1, "task"),
    ("Welcome and introductions", "manager", 1, "task"),
    ("Upload your personal documents", "employee", 3, "task"),
    ("SOPs acknowledged", "employee", 7, "sops"),
];

/// The onboarding checklists 04 seeds: accountant / associate, tax
/// specialist and billing / admin. Completion blocks nothing: the role is
/// active before the checklist is complete.
pub const ONBOARDING: [OnboardingSpec; 3] = [
    OnboardingSpec {
        name: "Accountant onboarding",
        role: "accountant",
        steps: &[
            COMMON_STEPS[0],
            COMMON_STEPS[1],
            (
                "Client files, CDD and the compliance calendar",
                "manager",
                3,
                "checklist:accountant_induction",
            ),
            COMMON_STEPS[2],
            COMMON_STEPS[3],
        ],
    },
    OnboardingSpec {
        name: "Tax specialist onboarding",
        role: "tax_specialist",
        steps: &[
            COMMON_STEPS[0],
            COMMON_STEPS[1],
            (
                "VAT and corporate tax filing, EmaraTax access",
                "manager",
                3,
                "checklist:tax_induction",
            ),
            COMMON_STEPS[2],
            COMMON_STEPS[3],
        ],
    },
    OnboardingSpec {
        name: "Billing and admin onboarding",
        role: "billing",
        steps: &[
            COMMON_STEPS[0],
            COMMON_STEPS[1],
            (
                "Retainers, invoices and collections",
                "manager",
                3,
                "checklist:billing_induction",
            ),
            COMMON_STEPS[2],
            COMMON_STEPS[3],
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
    /// The roles that must read and acknowledge it.
    pub readers: &'static [&'static str],
}

/// The roles that face clients: the readers of the firm's AML and CDD
/// procedures.
pub const CLIENT_FACING_ROLES: [&str; 6] = [
    crate::permissions::PARTNER_ROLE,
    crate::permissions::MANAGER_ROLE,
    crate::permissions::ACCOUNTANT_ROLE,
    crate::permissions::TAX_SPECIALIST_ROLE,
    crate::permissions::BILLING_ROLE,
    crate::permissions::MLRO_ROLE,
];

/// The SOP library 04 seeds: the MLRO owns the firm's AML and CDD
/// procedures, read by every client-facing role; the manager owns the
/// engagement, return and month-end close procedures. `DOC.RUL.04` asks each
/// reader to acknowledge within 7 days, reminds every 3 days and tells the
/// reader's manager and the owner role after 7 days overdue (the module's
/// defaults).
pub const SOPS: [SopSpec; 4] = [
    SopSpec {
        code: "SOP-ACC-AML-CDD",
        title: "AML and CDD procedures",
        category: "compliance",
        owner_role: crate::permissions::MLRO_ROLE,
        readers: &CLIENT_FACING_ROLES,
    },
    SopSpec {
        code: "SOP-ACC-ENGAGEMENT",
        title: "Engagements",
        category: "engagements",
        owner_role: crate::permissions::MANAGER_ROLE,
        readers: &[
            crate::permissions::PARTNER_ROLE,
            crate::permissions::MANAGER_ROLE,
            crate::permissions::ACCOUNTANT_ROLE,
            crate::permissions::BILLING_ROLE,
        ],
    },
    SopSpec {
        code: "SOP-ACC-RETURNS",
        title: "VAT and corporate tax returns",
        category: "returns",
        owner_role: crate::permissions::MANAGER_ROLE,
        readers: &[
            crate::permissions::MANAGER_ROLE,
            crate::permissions::ACCOUNTANT_ROLE,
            crate::permissions::TAX_SPECIALIST_ROLE,
        ],
    },
    SopSpec {
        code: "SOP-ACC-MONTH-END",
        title: "Month-end close",
        category: "engagements",
        owner_role: crate::permissions::MANAGER_ROLE,
        readers: &[
            crate::permissions::MANAGER_ROLE,
            crate::permissions::ACCOUNTANT_ROLE,
        ],
    },
];

/// The rules §7.2 lists as compliance-critical (owner alert `PLT.RUL.04` to
/// the partner role and the company admin).
pub const CRITICAL_RULES: [&str; 7] = [
    "ACC.RUL.04",
    "ACC.RUL.05",
    "ACC.RUL.09",
    "ACC.RUL.12",
    "PTY.RUL.01",
    "PTY.RUL.02",
    "DOC.RUL.03",
];

/// The critical rules the shared modules and the other parts of the app do not
/// mark themselves: `PTY.RUL.01` (the re-screening), `ACC.RUL.05` (the CDD review date; its daily scan is not
/// built yet, so there is no schedule to watch) and `DOC.RUL.03` (wl-documents
/// runs its nightly document rules without the kernel's rule runs).
pub const UNMARKED_RULES: [&str; 4] = ["ACC.RUL.05", "PTY.RUL.01", "PTY.RUL.02", "DOC.RUL.03"];

/// Marks [`UNMARKED_RULES`] compliance-critical, owned by the partner role.
pub(crate) fn register_critical_rules() {
    for rule in UNMARKED_RULES {
        wl_kernel::compliance::CriticalRule::new(rule)
            .owner_role(crate::permissions::PARTNER_ROLE)
            .register();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kyc_kinds_are_the_trade_licence_and_the_ubo_documents() {
        let keys: Vec<&str> = KYC_KINDS.iter().map(|k| k.key).collect();
        assert_eq!(keys, [TRADE_LICENCE, PASSPORT, EMIRATES_ID]);
        assert!(KYC_KINDS.iter().all(|k| k.look_ahead_days == 30));
    }

    #[test]
    fn the_tax_agent_registration_is_a_legal_block_the_partner_renews() {
        assert_eq!(TAX_AGENT.blocks, "return.file");
        assert!(!TAX_AGENT.override_allowed);
        assert_eq!(TAX_AGENT.look_ahead_days, 30);
        assert_eq!(TAX_AGENT.role, "partner");
    }

    #[test]
    fn the_lead_sources_are_the_seven_of_the_spec() {
        assert_eq!(LEAD_SOURCES.len(), 7);
        assert!(!LEAD_SOURCES.contains(&"aggregator"));
        assert!(!LEAD_SOURCES.contains(&"walk_in"));
    }

    #[test]
    fn there_is_a_checklist_for_each_staff_role_and_each_ends_with_the_sops() {
        let roles: Vec<&str> = ONBOARDING.iter().map(|o| o.role).collect();
        assert_eq!(roles, ["accountant", "tax_specialist", "billing"]);
        for spec in ONBOARDING {
            assert_eq!(spec.steps.last().map(|s| s.3), Some("sops"));
            assert!(spec.steps.iter().any(|s| s.1 == "employee"));
            assert!(spec.steps.iter().any(|s| s.1 == "role:admin"));
            assert!(spec.steps.iter().any(|s| s.1 == "manager"));
        }
    }

    #[test]
    fn the_sop_owners_are_the_mlro_and_the_manager() {
        let owners: Vec<(&str, &str)> = SOPS.iter().map(|s| (s.code, s.owner_role)).collect();
        assert_eq!(
            owners,
            [
                ("SOP-ACC-AML-CDD", "compliance"),
                ("SOP-ACC-ENGAGEMENT", "engagement_manager"),
                ("SOP-ACC-RETURNS", "engagement_manager"),
                ("SOP-ACC-MONTH-END", "engagement_manager"),
            ]
        );
        assert_eq!(SOPS[0].readers.len(), CLIENT_FACING_ROLES.len());
    }
}
