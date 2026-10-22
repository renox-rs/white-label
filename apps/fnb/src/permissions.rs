//! The vertical's permissions (06 F&B §3). Roles get them in
//! `host/src/app/roles.rs`; the role names the rules use are the consts at
//! the end.

/// See the orders, their tickets, delivery and failure.
pub const ORDERS_VIEW: &str = "orders.view";
/// Accept a placed order, assign its delivery details, dispatch and complete
/// it, change the party proposed as responsible for a failed delivery and
/// close a failed delivery by hand (§7.1 *Order*).
pub const ORDERS_MANAGE: &str = "orders.manage";
/// Cancel an order, with a reason and a responsible party, and cancel a line
/// (§7.1 `cancel`, `cancel_line`): the cashier and the kitchen manager. A
/// paid order above AED 50, or a line the kitchen started, still needs a
/// manager's yes in the sales module (`SAL.ACL.01`, `SAL.ACL.05`).
pub const ORDERS_CANCEL: &str = "orders.cancel";
/// Amend a placed or accepted direct order after calling the customer (§7.1
/// `amend`). The spec lists it under `orders.manage`; it is a permission of
/// its own so a cashier can amend without accepting, dispatching, completing
/// or closing orders.
pub const ORDERS_AMEND: &str = "orders.amend";
/// Request a refund the kitchen manager approves, from a cancelled order, for
/// a customer-side failure (`FNB.ACL.17`): the cashier. The approval is
/// `refunds.approve` of wl-payments, never the requester's (`FIN.ACL.04`).
pub const ORDERS_REFUND_REQUEST: &str = "orders.refund_request";
/// Start and bump the kitchen tickets of the user's station (`FNB.ACL.01`).
pub const KDS_WORK: &str = "kds.work";
/// Turn cash on delivery back on for a customer (`FNB.RUL.14`): the kitchen
/// manager.
pub const CUSTOMERS_COD: &str = "customers.cod";

/// See the aggregator payout statements, their lines and differences (§3.1
/// accountant: reconciliation).
pub const RECONCILIATION_VIEW: &str = "reconciliation.view";
/// Import, match, dispute, resolve and void an aggregator payout statement
/// (§7.1 *Payout statement*): the accountant. Voiding a matched or disputed
/// statement, whose receivable is booked, needs a second holder's approval
/// (`PLT.ACL.09`).
pub const RECONCILIATION_MANAGE: &str = "reconciliation.manage";
/// Enter an order a payout statement names and the kitchen never recorded, as
/// a late entry with the aggregator's order number (`PLT.ACL.10`): the cashier
/// and the accountant.
pub const LATE_ORDERS_ENTER: &str = "orders.late_entry";
/// The failed deliveries and the refunds after completion by category and
/// responsible party (§10): the accountant, the kitchen manager, the area
/// manager and the operations director.
pub const REPORTS_VIEW: &str = "fnb_reports.view";
/// The *rider fault* entries on a rider's record (BL-FNB-08): the kitchen
/// manager and HR; other riders never see them.
pub const RIDER_FAULTS_VIEW: &str = "rider_faults.view";

/// See the production batches and their lots (§3.1 head chef: production).
pub const PRODUCTION_VIEW: &str = "production.view";
/// Plan, start, finish and discard a production batch (§7.1 *Production
/// batch*): the head chef and the central kitchen chef. A discard above the
/// wastage limit still needs a second holder of `stock.approve`
/// (`STK.ACL.02`).
pub const PRODUCTION_MANAGE: &str = "production.manage";
/// wl-inventory's `stock.approve`: the second person of a wastage above the
/// limit (`STK.ACL.02`), the kitchen manager. The name is the stock module's;
/// the app only names the approval step of a batch discard with it.
pub const STOCK_APPROVE: &str = "stock.approve";
/// See the cold units, their readings and holds (§4.6, `FNB.RUL.05`).
pub const TEMPERATURE_VIEW: &str = "temperature.view";
/// Record a temperature reading of a cold unit (`WRK.CHK.01`, `FNB.FSF.01`):
/// the line cook, the chef and the kitchen manager.
pub const TEMPERATURE_LOG: &str = "temperature.log";
/// Set up the cold units of an outlet: their stock location and range
/// (`FNB.RUL.05`): the kitchen manager.
pub const COLD_UNITS_MANAGE: &str = "cold_units.manage";
/// See the recall trace of the ingredient lots: the customers with their
/// possible orders and the groups without a known customer (§4.4 recall).
pub const RECALL_TRACE_VIEW: &str = "fnb_recalls.view";
/// Tell the customers of a recall and record the owner's decision on the public
/// notice and the aggregators: the owner / operations director with the head
/// chef (§7.2 settings *Recall owner*).
pub const RECALL_TRACE_DECIDE: &str = "fnb_recalls.decide";

/// Every permission of the app.
pub const ALL: &[&str] = &[
    PRODUCTION_VIEW,
    PRODUCTION_MANAGE,
    TEMPERATURE_VIEW,
    TEMPERATURE_LOG,
    COLD_UNITS_MANAGE,
    RECALL_TRACE_VIEW,
    RECALL_TRACE_DECIDE,
    RECONCILIATION_VIEW,
    RECONCILIATION_MANAGE,
    LATE_ORDERS_ENTER,
    REPORTS_VIEW,
    RIDER_FAULTS_VIEW,
    ORDERS_VIEW,
    ORDERS_MANAGE,
    ORDERS_CANCEL,
    ORDERS_AMEND,
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
/// The accountant imports and reconciles the payout statements (§3.1). The
/// host's `roles::ACCOUNTANT`.
pub const ACCOUNTANT_ROLE: &str = "accountant";
/// The scope type of an outlet: roles are assigned per outlet (§3.2).
pub const OUTLET_SCOPE: &str = "fnb.outlet";
