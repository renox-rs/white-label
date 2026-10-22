//! The vertical's permissions (06 F&B §3). Roles get them in
//! `host/src/app/roles.rs`; the role names the rules use are the consts at
//! the end.

/// See the orders, their tickets, delivery and failure.
pub const ORDERS_VIEW: &str = "orders.view";
/// Accept a placed order, assign its delivery details, dispatch and complete
/// it, change the party proposed as responsible for a failed delivery and
/// close a failed delivery by hand (§7.1 *Order*).
pub const ORDERS_MANAGE: &str = "orders.manage";
/// Request a refund the kitchen manager approves, from a cancelled order, for
/// a customer-side failure (`FNB.ACL.17`): the cashier. The approval is
/// `refunds.approve` of wl-payments, never the requester's (`FIN.ACL.04`).
pub const ORDERS_REFUND_REQUEST: &str = "orders.refund_request";
/// Start and bump the kitchen tickets of the user's station (`FNB.ACL.01`).
pub const KDS_WORK: &str = "kds.work";
/// Turn cash on delivery back on for a customer (`FNB.RUL.14`): the kitchen
/// manager.
pub const CUSTOMERS_COD: &str = "customers.cod";

/// Every permission of the app.
pub const ALL: &[&str] = &[
    ORDERS_VIEW,
    ORDERS_MANAGE,
    ORDERS_REFUND_REQUEST,
    KDS_WORK,
    CUSTOMERS_COD,
];

/// The owner / operations director sees every brand and outlet (§3.1). The
/// host's `roles::OPERATIONS_DIRECTOR`.
pub const OPERATIONS_DIRECTOR_ROLE: &str = "operations_director";
/// The area manager oversees several outlets (§3.1). The host's
/// `roles::AREA_MANAGER`.
pub const AREA_MANAGER_ROLE: &str = "area_manager";
/// The outlet / kitchen manager runs one kitchen (§3.1). The host's
/// `roles::KITCHEN_MANAGER`.
pub const KITCHEN_MANAGER_ROLE: &str = "kitchen_manager";
/// The head chef: recipes, production, quality (§3.1). The host's
/// `roles::HEAD_CHEF`.
pub const HEAD_CHEF_ROLE: &str = "head_chef";
/// The line cook sees and bumps the tickets of their station (§3.1). The
/// host's `roles::LINE_COOK`.
pub const LINE_COOK_ROLE: &str = "line_cook";
/// The menu manager: menus, prices, promotions (§3.1). The host's
/// `roles::MENU_MANAGER`.
pub const MENU_MANAGER_ROLE: &str = "menu_manager";
/// The cashier enters aggregator and phone orders and requests small refunds
/// (§3.1). The host's `roles::CASHIER`; the same role name as 03's cashier,
/// with the 06 grants added for a 06 deployment only.
pub const CASHIER_ROLE: &str = "cashier";
/// The rider (own fleet) delivers in the driver PWA (§3.1). The host's
/// `roles::DRIVER`.
pub const RIDER_ROLE: &str = "driver";
/// The scope type of an outlet: roles are assigned per outlet (§3.2).
pub const OUTLET_SCOPE: &str = "fnb.outlet";
