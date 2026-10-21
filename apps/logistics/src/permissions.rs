//! The vertical's permissions (03 Logistics §3, §5). Roles get them in
//! `host/src/app/roles.rs`; the role names the rules use are the consts at
//! the end.

/// See shipments, their parcels and tracking events.
pub const SHIPMENTS_VIEW: &str = "shipments.view";
/// Book a shipment: staff for a client, or a client user for its own company
/// (`LOG.SHP.01`).
pub const SHIPMENTS_BOOK: &str = "shipments.book";
/// Dispatcher and operations manager: `cancel_return`, `void` of any
/// shipment, `hold` and `release` (§7.1 *Shipment*, `LOG.ACL.09`).
pub const SHIPMENTS_MANAGE: &str = "shipments.manage";
/// Scan parcels at pickup and at the hub (`LOG.HUB.01`).
pub const SHIPMENTS_SCAN: &str = "shipments.scan";
/// Record the client's e-POD of a returned shipment (`return_delivered`);
/// the driver's `deliver` and `fail_attempt` come from the delivery events.
pub const SHIPMENTS_DELIVER: &str = "shipments.deliver";
/// `void` a `created` shipment: staff with `shipments.manage`, or the client
/// user who booked it (`PLT.ACL.09`).
pub const SHIPMENTS_VOID: &str = "shipments.void";
/// `cancel` a `created` shipment: staff with `shipments.manage`, or a client
/// user of the shipment's company (`LOG.ACL.09`).
pub const SHIPMENTS_CANCEL: &str = "shipments.cancel";

/// Open and work freight jobs (03 §6 *Freight job*): the dispatcher and the
/// operations manager (BL-LOG-19).
pub const FREIGHT_MANAGE: &str = "freight.manage";

/// See freight jobs, their milestones and (for staff, `FIN.ACL.08`) their
/// costs and margin: the documentation officer, the accountant and the
/// operations manager (03 §3.1, `LOG.ACL.11`).
pub const JOBS_VIEW: &str = "jobs.view";
/// Work a freight job: request documents, depart, arrive, clear, hold and
/// release, cancel before departure, and record its costs: the
/// documentation officer (§7.1 *Freight job*).
pub const JOBS_MANAGE: &str = "jobs.manage";
/// Record the e-POD that delivers a freight job.
pub const JOBS_DELIVER: &str = "jobs.deliver";
/// Close a freight job and reopen a closed one for a late cost.
pub const JOBS_CLOSE: &str = "jobs.close";

/// Approve a request to mark a shipment off a delivery lost (`LOG.ACL.14`):
/// the operations manager, and the company admin, who holds every permission.
/// The requester never approves.
pub const SHIPMENTS_APPROVE_LOSS: &str = "shipments.approve_loss";

/// Approve a request to dispose of a held shipment (`LOG.ACL.14`): the
/// operations manager, and the company admin, who holds every permission.
/// The requester never approves.
pub const SHIPMENTS_APPROVE_DISPOSAL: &str = "shipments.approve_disposal";

/// See COD remittances, their lines and their statements (`LOG.COD.01`).
pub const REMITTANCES_VIEW: &str = "remittances.view";
/// Build a draft COD remittance and prepare adjustment lines on it
/// (`LOG.RUL.06`, `LOG.ACL.07`, `LOG.ACL.12`): the accountant.
pub const REMITTANCES_PREPARE: &str = "remittances.prepare";
/// Approve a remittance and its adjustment lines, and take an approved one
/// back to a draft (`LOG.ACL.07`, `LOG.ACL.12`); never the preparer's own.
pub const REMITTANCES_APPROVE: &str = "remittances.approve";
/// Pay an approved remittance to the client (`pay`, `PTY.ACL.03`), mark a
/// paid one as returned by the bank (`return_payment`) and pay it again
/// (`repay`).
pub const REMITTANCES_PAY: &str = "remittances.pay";
/// Flag a COD statement the client contests (`dispute`): customer service,
/// the accountant, and a client user of the company in the portal (BL-LOG-18).
/// The outcome is decided with `remittances.approve`.
pub const REMITTANCES_DISPUTE: &str = "remittances.dispute";
/// Ask to change the client's remittance bank account and take the request
/// back (`LOG.ACL.13`): the accountant, and a client user of the company in
/// the portal. A second user with `remittances.approve` approves it.
pub const REMITTANCES_REQUEST_ACCOUNT: &str = "remittances.request_account";

/// Every permission of the app.
pub const ALL: &[&str] = &[
    SHIPMENTS_VIEW,
    SHIPMENTS_BOOK,
    SHIPMENTS_MANAGE,
    SHIPMENTS_SCAN,
    SHIPMENTS_DELIVER,
    SHIPMENTS_VOID,
    SHIPMENTS_CANCEL,
    SHIPMENTS_APPROVE_LOSS,
    SHIPMENTS_APPROVE_DISPOSAL,
    REMITTANCES_VIEW,
    REMITTANCES_PREPARE,
    REMITTANCES_APPROVE,
    REMITTANCES_PAY,
    REMITTANCES_REQUEST_ACCOUNT,
    REMITTANCES_DISPUTE,
    FREIGHT_MANAGE,
    JOBS_VIEW,
    JOBS_MANAGE,
    JOBS_DELIVER,
    JOBS_CLOSE,
];

/// The operations manager runs a depot (§3.1). The host's
/// `roles::OPERATIONS_MANAGER`.
pub const OPERATIONS_MANAGER_ROLE: &str = "operations_manager";
/// The dispatcher builds runs and handles reattempts (§3.1). The host's
/// `roles::DISPATCHER`.
pub const DISPATCHER_ROLE: &str = "dispatcher";
/// Hub staff scan shipments in and out (§3.1). The host's `roles::HUB_STAFF`.
pub const HUB_STAFF_ROLE: &str = "hub_staff";
/// The driver works the assigned run (§3.1). The host's `roles::DRIVER`.
pub const DRIVER_ROLE: &str = "driver";
/// Customer service answers client and recipient queries and flags the COD
/// statements a client contests (§3.1). The host's `roles::CUSTOMER_SERVICE`.
pub const CUSTOMER_SERVICE_ROLE: &str = "customer_service";
/// A client user books, tracks and cancels its own shipments (§3.1). The
/// host's `roles::SHIPPING_CLIENT`.
pub const CLIENT_ROLE: &str = "shipping_client";

/// The documentation officer works the freight jobs and their customs and
/// shipping documents (§3.1). The host's `roles::DOCUMENTATION_OFFICER`.
pub const DOCUMENTATION_OFFICER_ROLE: &str = "documentation_officer";

/// The invoice owner: the role whose holders get the task to credit the
/// charge of an invoiced shipment that was reversed or lost (BL-LOG-10,
/// BL-LOG-11). The host's `roles::ACCOUNTANT`.
pub const INVOICE_OWNER_ROLE: &str = "accountant";
/// The HR role: the owner of the HR case opened for a driver who is liable
/// (BL-LOG-10). The host's `roles::HR`.
pub const HR_ROLE: &str = "hr";
