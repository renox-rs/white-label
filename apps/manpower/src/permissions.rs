//! The vertical's permissions (09 Manpower §3). Roles get them in
//! `host/src/app/roles.rs`. The scope units are the client company and, within
//! it, the site (§3.2): an account manager holds the permissions in their
//! client companies, a site supervisor in their sites, so a holder acts on
//! those records only.

use renox::auth::permissions::Scope;

/// The scope type of a client company; the id is the client's party id.
pub const CLIENT_SCOPE_TYPE: &str = "manpower.client";

/// The scope type of a site; the id is the site's id.
pub const SITE_SCOPE_TYPE: &str = "manpower.site";

/// The scope of the client company `party`.
pub fn client_scope(party: i64) -> Scope {
    Scope::new(CLIENT_SCOPE_TYPE, party)
}

/// The scope of the site `site`.
pub fn site_scope(site: i64) -> Scope {
    Scope::new(SITE_SCOPE_TYPE, site)
}

/// See the client contracts.
pub const CONTRACTS_VIEW: &str = "contracts.view";
/// Create, sign, activate and cancel a client contract (the account
/// manager's).
pub const CONTRACTS_MANAGE: &str = "contracts.manage";
/// Keep the sites of a client (the account manager's).
pub const SITES_MANAGE: &str = "sites.manage";
/// See the manpower requests.
pub const REQUESTS_VIEW: &str = "requests.view";
/// Write and submit a manpower request for a client (staff). A client
/// approver does the same for their own company through the portal grant
/// (`MPW.ACL.01`), not through this permission.
pub const REQUESTS_CREATE: &str = "requests.create";
/// Cancel a manpower request and open a cover request by hand (the account
/// manager's).
pub const REQUESTS_MANAGE: &str = "requests.manage";
/// See the deployments.
pub const DEPLOYMENTS_VIEW: &str = "deployments.view";
/// Plan, activate, extend, end and cancel a deployment (the account
/// manager's).
pub const DEPLOYMENTS_MANAGE: &str = "deployments.manage";

/// See, record the MoHRE report of and close the absence cases of workers (HR's,
/// `MPW.RUL.09`). Not in [`ALL`]: that list is the account manager's.
pub const ABSENCE_MANAGE: &str = "absence.manage";

/// The role of the account manager, who owns the client accounts.
pub const ACCOUNT_MANAGER_ROLE: &str = "account_manager";

/// The role of the MD, who is told of what only the MD decides; the company
/// admin when nobody holds it.
pub const MD_ROLE: &str = "md";

/// The role of the PRO (visa officer), who is told when a visa case step
/// passes its target (`MPW.RUL.14`).
pub const PRO_ROLE: &str = "pro";

/// The role of HR, told of what passes its target twice (`MPW.RUL.14`) and
/// who looks after absence cases.
pub const HR_ROLE: &str = "hr";

/// The role of the site supervisor, who sees the deployments of a site.
pub const SUPERVISOR_ROLE: &str = "supervisor";

/// The portal roles that act for a client company: the client approver
/// submits and cancels its requests (`MPW.ACL.01`); the viewer only sees.
pub const CLIENT_APPROVER_PORTAL_ROLES: &[&str] = &["owner", "approver"];

/// Every permission of the module.
pub const ALL: &[&str] = &[
    CONTRACTS_VIEW,
    CONTRACTS_MANAGE,
    SITES_MANAGE,
    REQUESTS_VIEW,
    REQUESTS_CREATE,
    REQUESTS_MANAGE,
    DEPLOYMENTS_VIEW,
    DEPLOYMENTS_MANAGE,
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
            ["contracts.", "sites.", "requests.", "deployments."]
                .iter()
                .any(|prefix| p.starts_with(prefix))
        }));
    }

    #[test]
    fn a_client_and_a_site_are_scopes() {
        assert_eq!(client_scope(7), Scope::new("manpower.client", 7));
        assert_eq!(site_scope(3), Scope::new("manpower.site", 3));
    }
}
