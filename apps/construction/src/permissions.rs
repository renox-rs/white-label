//! The vertical's permissions (02 Construction §3). Roles get them in
//! `host/src/app/roles.rs`; scoped to a project, a holder acts on that
//! project only (§3.2).

/// See projects.
pub const PROJECTS_VIEW: &str = "projects.view";
/// Start, hold, resume and settle the final account of a project (the
/// project manager's).
pub const PROJECTS_MANAGE: &str = "projects.manage";
/// Request the termination of a project mid-way (`CON.ACL.15`): the project
/// manager asks, the MD approves.
pub const PROJECTS_TERMINATE: &str = "projects.terminate";
/// Cancel a project before work started, and approve a termination
/// (`CON.ACL.15`): the MD's, which the company admin holds.
pub const PROJECTS_CANCEL: &str = "projects.cancel";

/// Accept an award and start a project from it (the step the quotation's
/// `accept` ends with, BL-CON-03).
pub const PROJECTS_AWARD: &str = "projects.award";

/// Submit an IPA to the client (the project manager's, `CON.ACL.05`).
pub const IPAS_SUBMIT: &str = "ipas.submit";
/// Record the client's certificate of an IPA (the QS's).
pub const IPAS_CERTIFY: &str = "ipas.certify";

/// Issue the tax invoice of a certified IPA: wl-invoicing's `invoices.issue`
/// (the accountant's), which the transition `invoice` asks for.
pub const INVOICES_ISSUE: &str = "invoices.issue";
/// Record the deduction lines of a certificate (the QS's, `CON.ACL.14`).
pub const IPAS_DEDUCTIONS_RECORD: &str = "ipas.deductions.record";
/// Accept or contest a deduction line (the project manager's, never for a
/// line they recorded, `CON.ACL.14`).
pub const IPAS_DEDUCTIONS_DECIDE: &str = "ipas.deductions.decide";

/// Link a bond to a retention part, make a part due when a bond replaces it
/// and record the client's deductions from it (the accountant's and the
/// QS's, `CON.ACL.11`, `CON.ACL.14`).
pub const RETENTIONS_MANAGE: &str = "retentions.manage";
/// Register, issue, reduce, extend and release a guarantee (the
/// accountant's).
pub const GUARANTEES_MANAGE: &str = "guarantees.manage";

/// Record and price a variation, and record the client's decision on it:
/// approve, reject, settle a claim (the QS's, `CON.ACL.06`).
pub const VARIATIONS_RECORD: &str = "variations.record";
/// Submit a variation to the client, resubmit, claim, abandon or withdraw it
/// (the project manager's, `CON.ACL.06`, `CON.ACL.16`).
pub const VARIATIONS_SUBMIT: &str = "variations.submit";

/// The role of the project manager.
pub const PROJECT_MANAGER_ROLE: &str = "project_manager";

/// The role of the quantity surveyor (QS), who gets the tasks of the IPAs.
pub const QS_ROLE: &str = "quantity_surveyor";

/// The role of the MD, who is told of what only the MD decides; the company
/// admin when nobody holds it.
pub const MD_ROLE: &str = "md";

/// The role the tasks of the accountant go to.
pub const ACCOUNTANT_ROLE: &str = "accountant";

/// The role the tasks of procurement go to.
pub const PROCUREMENT_ROLE: &str = "procurement";

/// Every permission of the module.
pub const ALL: &[&str] = &[
    PROJECTS_VIEW,
    PROJECTS_MANAGE,
    PROJECTS_TERMINATE,
    PROJECTS_CANCEL,
    PROJECTS_AWARD,
    IPAS_SUBMIT,
    IPAS_CERTIFY,
    IPAS_DEDUCTIONS_RECORD,
    IPAS_DEDUCTIONS_DECIDE,
    RETENTIONS_MANAGE,
    GUARANTEES_MANAGE,
    VARIATIONS_RECORD,
    VARIATIONS_SUBMIT,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_permission_is_listed_once() {
        let mut all = ALL.to_vec();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), ALL.len());
        assert!(all.iter().all(|p| {
            [
                "projects.",
                "ipas.",
                "retentions.",
                "guarantees.",
                "variations.",
            ]
            .iter()
            .any(|prefix| p.starts_with(prefix))
        }));
    }
}
