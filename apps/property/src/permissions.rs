//! The vertical's permissions (01 Property §3, §7.1). Roles get them in
//! `host/src/app/roles.rs`; scoped to a building, a holder acts on that
//! building's units and leases only (§3.2).

/// See the portfolio: buildings, units, their status and asking rent.
pub const UNITS_VIEW: &str = "units.view";
/// Add and change buildings and units; take a unit off the market, put it
/// back, mark it ready.
pub const UNITS_MANAGE: &str = "units.manage";
/// Reserve a unit for a prospect and release the reservation (Later).
pub const UNITS_RESERVE: &str = "units.reserve";
/// See leases, their charges and history.
pub const LEASES_VIEW: &str = "leases.view";
/// Draft a lease, a renewal or an amendment, and submit it for signature.
pub const LEASES_CREATE: &str = "leases.create";
/// Cancel a draft or a lease waiting for signature (`PRP.ACL.18`: asked by
/// the leasing agent or the accountant, approved by a property manager).
pub const LEASES_CANCEL: &str = "leases.cancel";
/// Activate a signed lease; record the Ejari registration.
pub const LEASES_ACTIVATE: &str = "leases.activate";
/// Notice, renewal decision, termination and the other changes to a running
/// lease.
pub const LEASES_MANAGE: &str = "leases.manage";
/// Approve a forced termination (`PRP.ACL.19`).
pub const LEASES_FORCE_TERMINATE: &str = "leases.force_terminate";

/// Every permission of the module.
pub const ALL: &[&str] = &[
    UNITS_VIEW,
    UNITS_MANAGE,
    UNITS_RESERVE,
    LEASES_VIEW,
    LEASES_CREATE,
    LEASES_CANCEL,
    LEASES_ACTIVATE,
    LEASES_MANAGE,
    LEASES_FORCE_TERMINATE,
];

/// The role whose holders approve a cancellation after money was received
/// (`PRP.ACL.18`), and who are told about a lease waiting for signature
/// (`PRP.RUL.12`). The host's `roles::PROPERTY_MANAGER` is this role.
pub const PROPERTY_MANAGER_ROLE: &str = "property_manager";
/// The role whose holders are reminded of a lease waiting for signature
/// (`PRP.RUL.12`). The host's `roles::LEASING_AGENT` is this role.
pub const LEASING_AGENT_ROLE: &str = "leasing_agent";
/// The role that gets the tasks about cheques and refunds. The host's
/// `roles::ACCOUNTANT` is this role.
pub const ACCOUNTANT_ROLE: &str = "accountant";
