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
/// Approve a forced termination (`PRP.ACL.19`): the property manager
/// requests it with `leases.manage`, another holder of this permission (the
/// company admin by default) approves.
pub const LEASES_FORCE_TERMINATE: &str = "leases.force_terminate";

/// Draft, edit, publish and revise the owner statements, and ask for a payout
/// (`PRP.ACL.08`: prepared by one user, approved by another). The accountant's.
pub const OWNER_STATEMENTS_PREPARE: &str = "owner_statements.prepare";
/// Approve or return an owner statement, and approve a payout or an owner
/// invoice. The property manager's.
pub const OWNER_STATEMENTS_APPROVE: &str = "owner_statements.approve";
/// Draft management agreements and put units under them.
pub const AGREEMENTS_MANAGE: &str = "agreements.manage";

/// The permission of wl-payments that lets the accountant forfeit a deposit;
/// the forfeiture that applies a deposit to the deductions is checked against
/// it in the lease's building (BL-PRP-17).
pub const DEPOSITS_REFUND: &str = "deposits.refund";

/// The permission of wl-invoicing that lets a user write off an invoice; the
/// page offers the write-off of a lease's invoice to its holders (BL-PRP-18).
pub const INVOICES_WRITE_OFF: &str = "invoices.write_off";

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
    OWNER_STATEMENTS_PREPARE,
    OWNER_STATEMENTS_APPROVE,
    AGREEMENTS_MANAGE,
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
/// The finance manager: gives the second approval of deposit deductions above
/// the threshold of `PRP.ACL.05`. The host's `roles::FINANCE_MANAGER`.
pub const FINANCE_MANAGER_ROLE: &str = "finance_manager";
/// The owner: may give the second approval of deposit deductions instead of
/// finance. The host's `roles::OWNER`.
pub const OWNER_ROLE: &str = "owner";
/// The roles whose holders give the second approval (`PRP.ACL.05`).
pub const SECOND_APPROVER_ROLES: [&str; 2] = [FINANCE_MANAGER_ROLE, OWNER_ROLE];
