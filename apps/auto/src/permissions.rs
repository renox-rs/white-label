//! The vertical's permissions (08 Auto §3). Roles get them in
//! `host/src/app/roles.rs`; the role names the rules use are the consts at
//! the end. The workshop bookings keep wl-scheduling's own permissions
//! (`bookings.*`); the stock and the invoices keep those of wl-inventory and
//! wl-invoicing.

/// See the vehicles of the register.
pub const VEHICLES_VIEW: &str = "vehicles.view";
/// Register and change a vehicle (`AUT.VEH.01`).
pub const VEHICLES_MANAGE: &str = "vehicles.manage";
/// See every job card of the workshop (`AUT.ACL.08`: a technician without it
/// sees only the job cards assigned to them, and no contact details).
pub const JOB_CARDS_VIEW: &str = "job_cards.view";
/// Work a job card: inspect, record findings and the work done, request and
/// fit parts, finish the work (`AUT.JOB.01`).
pub const JOB_CARDS_WORK: &str = "job_cards.work";
/// Advise: check in, price the estimate, record the customer's approval,
/// decline, cancel, void and deliver (`AUT.JOB.01`, `AUT.JOB.02`).
pub const JOB_CARDS_ADVISE: &str = "job_cards.advise";
/// Quality control: pass or fail the QC checklist (`AUT.ACL.09`).
pub const JOB_CARDS_QC: &str = "job_cards.qc";
/// Approve what a service advisor may not decide alone: billing less than the
/// work done, a storage fee waived, a refused signature (`AUT.ACL.21`).
pub const JOB_CARDS_APPROVE: &str = "job_cards.approve";
/// Decide whether the accident damage flag of a job card is cleared: the
/// workshop manager, never the one who asked (`AUT.ACL.20`).
pub const JOB_CARDS_ACCIDENT: &str = "job_cards.accident";
/// Decide the liability of a vehicle incident: the branch manager, or the owner
/// when the branch manager was involved (`AUT.ACL.22`).
pub const JOB_CARDS_LIABILITY: &str = "job_cards.liability";
/// Keep the labour operations catalogue: the menu and the flat-rate prices
/// (`AUT.JOB.03`).
pub const LABOUR_MENU_MANAGE: &str = "labour_menu.manage";
/// Register and run insurance claims (`AUT.INS.01` .. `AUT.INS.04`).
pub const INSURANCE_MANAGE: &str = "insurance.manage";
/// Run fleet service contracts (`AUT.FLT.01`).
pub const FLEET_CONTRACTS_MANAGE: &str = "fleet_contracts.manage";
/// Approve the early termination of a fleet contract, and a waiver of its fee:
/// the owner / GM, never the fleet account manager who asked (`PLT.APR.01`).
pub const FLEET_CONTRACTS_APPROVE: &str = "fleet_contracts.approve";
/// Raise and recover supplier warranty claims (`AUT.WAR.02`).
pub const SUPPLIER_CLAIMS_MANAGE: &str = "supplier_claims.manage";
/// Approve the write-off of a supplier warranty claim: the branch manager,
/// never the purchaser who asked (`AUT.WAR.02`, `PLT.APR.01`).
pub const SUPPLIER_CLAIMS_APPROVE: &str = "supplier_claims.approve";
/// See the export sales and their exit evidence (`AUT.EXP.01`).
pub const EXPORT_SALES_VIEW: &str = "export_sales.view";
/// Register an export sale and attach its exit evidence: the accountant
/// (`AUT.EXP.01`, `AUT.RUL.15`).
pub const EXPORT_SALES_MANAGE: &str = "export_sales.manage";
/// Issue the invoices of a claim or of a fleet contract: wl-invoicing's own
/// permission (`invoices.issue`), which the accountant holds (`AUT.INS.04`).
pub const INVOICES_ISSUE: &str = "invoices.issue";
/// Issue parts from stock to a job card: wl-inventory's own permission
/// (`stock.issue`), which the storekeeper holds, is enough besides
/// `job_cards.work` and `job_cards.advise`.
pub const STOCK_ISSUE: &str = "stock.issue";

/// The owner / GM: pricing policy, credit decisions, told when a
/// compliance-critical rule fails.
pub const OWNER_ROLE: &str = "owner";
/// The branch manager: runs a branch or a workshop; approves discounts,
/// returns, write-offs, warranty claims and what the advisor may not decide.
pub const BRANCH_MANAGER_ROLE: &str = "branch_manager";
/// The service advisor: bookings, vehicle check-in, estimates, customer
/// approval and delivery.
pub const SERVICE_ADVISOR_ROLE: &str = "service_advisor";
/// The workshop manager: assigns bays and technicians, quality control and
/// productivity.
pub const WORKSHOP_MANAGER_ROLE: &str = "workshop_manager";
/// The technician: works the job cards assigned and records findings (the
/// host's own `technician` role).
pub const TECHNICIAN_ROLE: &str = "technician";
/// Counter sales (the parts counter): counter sales and phone orders.
pub const COUNTER_SALES_ROLE: &str = "counter_sales";
/// The storekeeper: receives, puts away, picks and issues stock (the host's
/// own `storekeeper` role).
pub const STOREKEEPER_ROLE: &str = "storekeeper";
/// The purchaser / importer: reorders, imports and recovers supplier
/// warranty (the host's own `procurement` role).
pub const PURCHASER_ROLE: &str = "procurement";
/// The accountant: invoices, receipts, cheques and credit.
pub const ACCOUNTANT_ROLE: &str = "accountant";
/// The insurance coordinator: registers claims, collects the police report
/// and policy documents, submits estimates and chases the insurers.
pub const INSURANCE_COORDINATOR_ROLE: &str = "insurance_coordinator";
/// The fleet account manager: agrees the fleet contracts and follows their
/// services and invoices.
pub const FLEET_ACCOUNT_MANAGER_ROLE: &str = "fleet_account_manager";
