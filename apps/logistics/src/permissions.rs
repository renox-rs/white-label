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

/// See COD remittances, their lines and their statements (`LOG.COD.01`).
pub const REMITTANCES_VIEW: &str = "remittances.view";
/// Build a draft COD remittance and prepare adjustment lines on it
/// (`LOG.RUL.06`, `LOG.ACL.07`, `LOG.ACL.12`): the accountant.
pub const REMITTANCES_PREPARE: &str = "remittances.prepare";
/// Approve a remittance and its adjustment lines, and take an approved one
/// back to a draft (`LOG.ACL.07`, `LOG.ACL.12`); never the preparer's own.
pub const REMITTANCES_APPROVE: &str = "remittances.approve";
/// Pay an approved remittance to the client (`pay`, `PTY.ACL.03`).
pub const REMITTANCES_PAY: &str = "remittances.pay";

/// Every permission of the app.
pub const ALL: &[&str] = &[
    SHIPMENTS_VIEW,
    SHIPMENTS_BOOK,
    SHIPMENTS_MANAGE,
    SHIPMENTS_SCAN,
    SHIPMENTS_DELIVER,
    SHIPMENTS_VOID,
    SHIPMENTS_CANCEL,
    REMITTANCES_VIEW,
    REMITTANCES_PREPARE,
    REMITTANCES_APPROVE,
    REMITTANCES_PAY,
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
/// A client user books, tracks and cancels its own shipments (§3.1). The
/// host's `roles::SHIPPING_CLIENT`.
pub const CLIENT_ROLE: &str = "shipping_client";

/// The invoice owner: the role whose holders get the task to credit the
/// charge of an invoiced shipment that was reversed or lost (BL-LOG-10,
/// BL-LOG-11). The host's `roles::ACCOUNTANT`.
pub const INVOICE_OWNER_ROLE: &str = "accountant";
/// The HR role: the owner of the HR case opened for a driver who is liable
/// (BL-LOG-10). The host's `roles::HR`.
pub const HR_ROLE: &str = "hr";
