//! The vertical's permissions (04 Accounting §3, §7.1). Roles get them in
//! `host/src/app/roles.rs`; the role names the rules use are the consts at
//! the end.

/// See engagements, their history and the export.
pub const ENGAGEMENTS_VIEW: &str = "engagements.view";
/// Create an engagement; record the signed letter; `activate`, `decline`,
/// `suspend` and `resume` (§7.1 *Engagement*).
pub const ENGAGEMENTS_MANAGE: &str = "engagements.manage";
/// `end` an engagement (the partner's, §7.1 *Engagement*).
pub const ENGAGEMENTS_END: &str = "engagements.end";
/// See CDD files (never their exit or rejection reason, which needs
/// [`BLOCK_REASON_VIEW`]).
pub const CDD_VIEW: &str = "cdd.view";
/// Open a CDD file, list its screened parties, rate it, `submit` and
/// `reopen` it (the preparer).
pub const CDD_PREPARE: &str = "cdd.prepare";
/// `approve`, `reject` and `exit` a CDD file (the MLRO).
pub const CDD_APPROVE: &str = "cdd.approve";
/// See a client's tax profile.
pub const TAX_PROFILES_VIEW: &str = "tax_profiles.view";
/// Record a client's tax profile (its change control is `ACC.ACL.13`,
/// BL-ACC-14).
pub const TAX_PROFILES_MANAGE: &str = "tax_profiles.manage";

/// The permission of wl-parties that shows restricted block reasons and
/// Block records (`PTY.ACL.02`; the MLRO and deputy). In 04 it also shows a
/// possible match in a refusal, the CDD file's exit or rejection reason, and
/// lets its holder choose the neutral template that tells the client
/// (`ACC.ACL.14`).
pub const BLOCK_REASON_VIEW: &str = "parties.block_reason.view";

/// Every permission of the app.
pub const ALL: &[&str] = &[
    ENGAGEMENTS_VIEW,
    ENGAGEMENTS_MANAGE,
    ENGAGEMENTS_END,
    CDD_VIEW,
    CDD_PREPARE,
    CDD_APPROVE,
    TAX_PROFILES_VIEW,
    TAX_PROFILES_MANAGE,
];

/// The partner: owns the client relationship, ends engagements (§3.1). The
/// host's `roles::PARTNER`.
pub const PARTNER_ROLE: &str = "partner";
/// The manager of a portfolio of clients: runs the engagements (§3.1). The
/// host's `roles::ENGAGEMENT_MANAGER`.
pub const MANAGER_ROLE: &str = "engagement_manager";
/// The accountant or associate: prepares the work and the CDD file (§3.1).
/// The host's `roles::ACCOUNTANT`, shared with the other verticals.
pub const ACCOUNTANT_ROLE: &str = "accountant";
/// The tax specialist: returns and tax profiles (§3.1). The host's
/// `roles::TAX_SPECIALIST`.
pub const TAX_SPECIALIST_ROLE: &str = "tax_specialist";
/// The compliance officer (MLRO) and the deputy: CDD approvals, screening,
/// restricted reasons (§3.1). The host's `roles::COMPLIANCE`, shared with
/// wl-parties.
pub const MLRO_ROLE: &str = "compliance";
/// Billing and admin: proposals, retainers, invoices (§3.1). The host's
/// `roles::BILLING`.
pub const BILLING_ROLE: &str = "billing";
