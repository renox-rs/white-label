//! The vertical's permissions (07 Manufacturing §3). Roles get them in
//! `host/src/app/roles.rs`. The scope units are the plant and, within it, the
//! production line (§3.2): production managers, planners and QC inspectors
//! hold the permissions in their plant, supervisors and operators in their
//! line. A holder in the plant also acts on the lines of the plant; the
//! work order records both ([`crate::work_order::WorkOrder::may`]).

use renox::auth::permissions::Scope;

/// The scope type of a plant; the id is the plant's id.
pub const PLANT_SCOPE_TYPE: &str = "manufacturing.plant";

/// The scope type of a production line; the id is the line's id.
pub const LINE_SCOPE_TYPE: &str = "manufacturing.line";

/// The scope of plant `plant`.
pub fn plant_scope(plant: i64) -> Scope {
    Scope::new(PLANT_SCOPE_TYPE, plant)
}

/// The scope of line `line`.
pub fn line_scope(line: i64) -> Scope {
    Scope::new(LINE_SCOPE_TYPE, line)
}

/// See the work orders.
pub const WORK_ORDERS_VIEW: &str = "work_orders.view";
/// Write a work order (the planner's).
pub const WORK_ORDERS_CREATE: &str = "work_orders.create";
/// Release, cancel, unlink and ask to abort a work order (planner,
/// supervisor; `MFG.ACL.12`).
pub const WORK_ORDERS_RELEASE: &str = "work_orders.release";
/// Start, hold, resume and complete a work order and report its output
/// (supervisor, operator; `MFG.ACL.01`).
pub const WORK_ORDERS_REPORT: &str = "work_orders.report";
/// Issue materials to a work order (the storekeeper's).
pub const WORK_ORDERS_ISSUE: &str = "work_orders.issue";
/// Close a completed work order.
pub const WORK_ORDERS_CLOSE: &str = "work_orders.close";
/// Approve the abort of a work order: step 1 of `MFG.ACL.12` (the production
/// manager's).
pub const WORK_ORDERS_ABORT_APPROVE: &str = "work_orders.abort_approve";
/// See the costs of a work order: actual cost, scrap value and loss
/// (`FIN.ACL.08` hides them from the floor roles).
pub const WORK_ORDERS_COST_VIEW: &str = "work_orders.cost_view";
/// Approve the void of a production report: the line's supervisor and the
/// production manager (`PLT.ACL.09`, `MFG.ACL.10`); never the requester.
pub const WORK_ORDERS_VOID_APPROVE: &str = "work_orders.void_approve";
/// See the non-conformance reports (QC, the production manager).
pub const NCRS_VIEW: &str = "ncrs.view";
/// Raise an NCR on a lot and change its cause (QC inspector).
pub const NCRS_RAISE: &str = "ncrs.raise";
/// Disposition, close, reopen and cancel an NCR (QC inspector, the
/// production manager).
pub const NCRS_DISPOSITION: &str = "ncrs.disposition";
/// Approve a *use as is* disposition: someone other than the inspector who
/// asks (`MFG.ACL.03`).
pub const NCRS_APPROVE: &str = "ncrs.approve";
/// Keep the plants and the production lines.
pub const PLANTS_MANAGE: &str = "plants.manage";

/// The permission of `STK.ACL.02` (wl-inventory's `stock.approve`): the
/// second step of an abort whose scrap is above the limit.
pub const STOCK_APPROVE: &str = "stock.approve";

/// The role of the production planner.
pub const PLANNER_ROLE: &str = "production_planner";
/// The role of the production manager, who approves an abort.
pub const PRODUCTION_MANAGER_ROLE: &str = "production_manager";
/// The role of the QC inspector.
pub const QC_ROLE: &str = "qc_inspector";
/// The role of the storekeeper.
pub const STOREKEEPER_ROLE: &str = "storekeeper";
/// The role of the line supervisor.
pub const SUPERVISOR_ROLE: &str = "supervisor";
/// The role of sales, told what became of the order's work.
pub const SALES_ROLE: &str = "sales";

/// Every permission of the module.
pub const ALL: &[&str] = &[
    WORK_ORDERS_VIEW,
    WORK_ORDERS_CREATE,
    WORK_ORDERS_RELEASE,
    WORK_ORDERS_REPORT,
    WORK_ORDERS_ISSUE,
    WORK_ORDERS_CLOSE,
    WORK_ORDERS_ABORT_APPROVE,
    WORK_ORDERS_VOID_APPROVE,
    WORK_ORDERS_COST_VIEW,
    NCRS_VIEW,
    NCRS_RAISE,
    NCRS_DISPOSITION,
    NCRS_APPROVE,
    PLANTS_MANAGE,
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
    }

    #[test]
    fn a_plant_and_a_line_are_scopes() {
        assert_eq!(plant_scope(7), Scope::new("manufacturing.plant", 7));
        assert_eq!(line_scope(3), Scope::new("manufacturing.line", 3));
    }
}
