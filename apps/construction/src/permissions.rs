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

/// The role of the project manager.
pub const PROJECT_MANAGER_ROLE: &str = "project_manager";

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
        assert!(all.iter().all(|p| p.starts_with("projects.")));
    }
}
