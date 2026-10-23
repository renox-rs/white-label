#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

mod arrival;
pub mod balance;
pub mod chase;
pub mod config;
pub mod dispatch;
pub mod dispatch_deliver;
mod dispatch_deliver_pages;
mod dispatch_events;
pub mod dispatch_loss;
mod dispatch_pages;
pub mod dispatch_pick;
pub mod dispatch_reverse;
mod dispatch_screens;
pub mod dispatch_send;
mod dispatch_texts;
mod events;
pub mod goods_receipt;
pub mod hold_decide;
pub mod hold_line;
pub mod hold_reminder;
pub mod hold_service;
pub mod item;
pub mod item_supplier;
pub mod landed_adjust;
pub mod landed_cost;
pub mod landed_draft;
pub mod location;
pub mod lot;
pub mod match_source;
pub mod movement;
pub mod pages;
pub mod pending_issue;
pub mod po_issue;
pub mod po_revise;
pub mod preferred_change;
pub mod production_stock;
pub mod purchase_order;
pub mod purchase_request;
pub mod recall;
pub mod recall_close;
pub mod recall_line;
pub mod recall_notice;
mod recall_pages;
mod recall_screens;
pub mod recall_trace;
pub mod receipt_post;
pub mod receipt_reverse;
mod receipt_screens;
pub mod reorder;
pub mod return_credit;
pub mod return_dispatch;
pub mod return_note;
pub mod scope_stock;
mod screen_texts;
mod screens;
pub mod shortage_alert;
pub mod stock_count;
pub mod stock_hold;
mod stock_pages;
mod stock_screens;
pub mod stock_service;
mod stock_ui_texts;
mod supplier_pages;
pub mod supplier_return;
pub mod supplier_review;
pub mod supplier_score;
mod texts;
pub mod transfer;
mod transfer_screens;

use std::sync::Arc;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use config::{InventoryConfig, RejectionReason, ScoreSettings};
pub use dispatch::{
    Dispatch, DispatchLine, DispatchStatus, Handover, NewDispatch, NewDispatchLine,
    create_draft as create_dispatch,
};
pub use dispatch_send::HandoverChoice;
pub use goods_receipt::{
    DraftLine, DraftReceipt, GoodsReceipt, GoodsReceiptLine, GoodsReceiptStatus,
};
pub use item::{Item, ItemType, ReorderPolicy};
pub use item_supplier::{ItemSupplier, PreferredSupplierRequest, SupplierRole};
pub use landed_adjust::{AdjustOutcome, AdjustmentStatus, LandedCostAdjustment, adjust};
pub use landed_cost::{AllocationBasis, CostKind, LandedCost, LandedCostStatus, allocate};
pub use location::{Location, LocationType, NegativeStockPolicy};
pub use lot::{Lot, LotStatus};
pub use match_source::PurchaseMatch;
pub use movement::{MovementType, StockMovement};
pub use pending_issue::{PendingIssue, PendingStatus};
pub use po_issue::{IssueOutcome, issue};
pub use po_revise::{PurchaseOrderRevision, ReviseInput, ReviseOutcome, revise};
pub use purchase_order::{PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus};
pub use purchase_request::{PurchaseRequest, PurchaseRequestStatus};
pub use recall::{Recall, RecallLot, RecallScope, RecallSource, RecallStatus};
pub use recall_line::{LineKind, NoticeStatus, RecallLine};
pub use receipt_post::{PostOutcome, post};
pub use receipt_reverse::reverse;
pub use reorder::{ReorderSuggestion, SuggestionStatus};
pub use return_credit::{CreditBasis, CreditOutcome, KeptGoods, cancel, credit};
pub use return_dispatch::{CollectionProof, DispatchOutcome, dispatch};
pub use shortage_alert::ShortageAlert;
pub use stock_count::{StockCount, StockCountStatus};
pub use stock_hold::{
    HoldLineStatus, HoldScope, HoldStatus, StockHold, StockHoldItem, StockHoldLine,
};
pub use stock_pages::PendingRow;
pub use supplier_return::{
    RejectedHandling, ReturnOrigin, ReturnStatus, SupplierReturn, SupplierReturnLine, WantedOutcome,
};
pub use supplier_review::{Decision, ReviewStatus, SupplierReview};
pub use supplier_score::{Figures, ScoreBand, SupplierScore, SupplierScoreNote};
pub use transfer::{Transfer, TransferLine, TransferStatus};

/// The module's permissions (wl-inventory §6).
pub mod permissions {
    /// See items.
    pub const ITEMS_VIEW: &str = "items.view";
    /// Create and change items.
    pub const ITEMS_MANAGE: &str = "items.manage";
    /// Create and change locations.
    pub const LOCATIONS_MANAGE: &str = "locations.manage";
    /// See stock levels.
    pub const STOCK_VIEW: &str = "stock.view";
    /// See purchase orders.
    pub const PURCHASE_ORDERS_VIEW: &str = "purchase_orders.view";
    /// Issue, revise, short-close and cancel purchase orders, and approve an
    /// over-receipt in the location's scope (`STK.ACL.04`).
    pub const PURCHASE_ORDERS_ISSUE: &str = "purchase_orders.issue";
    /// Post a goods receipt at the user's own locations.
    pub const GOODS_RECEIPTS_POST: &str = "goods_receipts.post";
    /// Reverse a posted goods receipt.
    pub const GOODS_RECEIPTS_REVERSE: &str = "goods_receipts.reverse";
    /// Pick, dispatch, cancel and confirm dispatches at the user's own
    /// locations; record the refused goods received back (`STK.DSP.01`).
    pub const DISPATCH_MANAGE: &str = "dispatch.manage";
    /// Manage returns to supplier.
    pub const SUPPLIER_RETURNS_MANAGE: &str = "supplier_returns.manage";
    /// List, rank and deactivate the suppliers of an item (`STK.SUP.01`).
    pub const ITEM_SUPPLIERS_MANAGE: &str = "item_suppliers.manage";
    /// See the cost side of stock: last prices and values (`stock.cost.view`).
    pub const STOCK_COST_VIEW: &str = "stock.cost.view";
    /// Approve or reject a change of an item's preferred supplier; never the
    /// requester (`STK.ACL.07`).
    pub const ITEM_SUPPLIERS_APPROVE: &str = "item_suppliers.approve";
    /// Cancel and approve purchase requests (wl-inventory §6).
    pub const PURCHASE_REQUESTS_APPROVE: &str = "purchase_requests.approve";
    /// Approve and cancel stock counts (`STK.CNT.01`).
    pub const STOCK_APPROVE: &str = "stock.approve";
    /// See the supplier score and its percentages (`STK.SUP.01`).
    pub const SUPPLIER_SCORES_VIEW: &str = "supplier_scores.view";
    /// Allocate, adjust and cancel landed costs, and approve an adjustment
    /// above the value (`STK.ACL.06`).
    pub const LANDED_COSTS_POST: &str = "landed_costs.post";
    /// Issue stock to a cost object in the scope of the location.
    pub const STOCK_ISSUE: &str = "stock.issue";
    /// Send and receive transfers between locations.
    pub const STOCK_TRANSFER: &str = "stock.transfer";
    /// Adjust stock, and clear a pending issue with a reason (`STK.ACL.02`).
    pub const STOCK_ADJUST: &str = "stock.adjust";
    /// Place a stock hold on whole lots or on the stock at a location.
    pub const STOCK_HOLD: &str = "stock.hold";
    /// Decide the lines of a stock hold, and be told when one is placed.
    pub const LOTS_RELEASE: &str = "lots.release";
    /// Open, trace, notify and close recalls.
    pub const RECALLS_MANAGE: &str = "recalls.manage";
    /// See the recalls and how far each is (the lots, where they are, the
    /// hold, how many were told), without naming anyone.
    pub const RECALLS_VIEW: &str = "recalls.view";
}

/// The inventory module. The host adds it with
/// `.module(wl_inventory::WlInventory::new())` when `client.toml` enables
/// `inventory`.
#[derive(Default)]
pub struct WlInventory {
    config: InventoryConfig,
}

impl WlInventory {
    /// The module with the §7 defaults ([`InventoryConfig::default`]).
    pub fn new() -> Self {
        Self::default()
    }

    /// How far the accepted quantity may go above the ordered one without
    /// approval, in basis points (default 500, 5%; `STK.ACL.04`).
    pub fn over_receipt_tolerance_bps(mut self, bps: i64) -> Self {
        self.config.over_receipt_tolerance_bps = bps.max(0);
        self
    }

    /// The value, in fils, above which dispatching a Return to supplier needs
    /// a second person's approval (default 500000, AED 5,000; `STK.ACL.06`).
    pub fn return_approval_fils(mut self, fils: i64) -> Self {
        self.config.return_approval_fils = fils.max(0);
        self
    }

    /// The value, in fils, above which wastage, such as a credit the
    /// supplier refused, needs a second person's approval (default 100000,
    /// AED 1,000; `STK.ACL.02`).
    pub fn wastage_approval_fils(mut self, fils: i64) -> Self {
        self.config.wastage_approval_fils = fils.max(0);
        self
    }

    /// The value, in fils, above which a landed cost adjustment needs a
    /// second person's approval (default 500000, AED 5,000; `STK.ACL.06`).
    pub fn landed_cost_approval_fils(mut self, fils: i64) -> Self {
        self.config.landed_cost_approval_fils = fils.max(0);
        self
    }

    /// The days a Return to supplier may stay `dispatched` before procurement
    /// and finance are told, and the days between two notices (default 30;
    /// `STK.RUL.08`).
    pub fn return_chase_days(mut self, days: i64) -> Self {
        self.config.return_chase_days = days.max(1);
        self
    }

    /// The value limits of a purchase order, in fils, as `STK.ACL.03` has
    /// them (default 1000000 and 5000000, AED 10,000 and 50,000; 06: one
    /// limit of 500000): a revision that raises the total above the first
    /// needs one approval, above the second two, and so on. Sorted, and a
    /// limit of 0 or less is dropped.
    pub fn order_limits_fils(mut self, limits: &[i64]) -> Self {
        let mut limits: Vec<i64> = limits.iter().copied().filter(|l| *l > 0).collect();
        limits.sort_unstable();
        limits.dedup();
        self.config.order_limits_fils = limits;
        self
    }

    /// The hours a pending issue or a negative balance may stay open before
    /// the storekeeper gets a task and the manager is told (default 24;
    /// `STK.RUL.06`).
    pub fn pending_alert_hours(mut self, hours: i64) -> Self {
        self.config.pending_alert_hours = hours.max(1);
        self
    }

    /// The hold reasons whose release needs a second holder of `lots.release`
    /// (default `recall`; `STK.ACL.05`). Other holds are released by one.
    pub fn hold_second_person_reasons(mut self, reasons: &[&str]) -> Self {
        self.config.hold_second_person_reasons = reasons
            .iter()
            .map(|reason| (*reason).trim().to_lowercase())
            .collect();
        self
    }

    /// H, the hours between two reminders of an open stock hold to the
    /// people who decide it, and half the hours between two notices to the
    /// escalation role (default 24; a vertical may set fewer for food, 06:
    /// 2; `STK.RUL.07`).
    pub fn hold_reminder_hours(mut self, hours: i64) -> Self {
        self.config.hold_reminder_hours = hours.max(1);
        self
    }

    /// H for the holds with `reason`; it wins over the default.
    pub fn hold_reminder_hours_for_reason(mut self, reason: &str, hours: i64) -> Self {
        let reason = reason.trim().to_lowercase();
        self.config
            .hold_reminder_hours_by_reason
            .retain(|(listed, _)| *listed != reason);
        self.config
            .hold_reminder_hours_by_reason
            .push((reason, hours.max(1)));
        self
    }

    /// H for the holds on the location with `code`; it wins over the reason.
    pub fn hold_reminder_hours_at(mut self, code: &str, hours: i64) -> Self {
        let code = code.trim().to_owned();
        self.config
            .hold_reminder_hours_by_location
            .retain(|(listed, _)| *listed != code);
        self.config
            .hold_reminder_hours_by_location
            .push((code, hours.max(1)));
        self
    }

    /// The role told every 2 x H about a hold nobody has decided (default
    /// the business owner; 06: the kitchen manager).
    pub fn hold_escalation_role(mut self, role: &str) -> Self {
        self.config.hold_escalation_role = Some(role.trim().to_owned());
        self
    }

    /// The escalation role for the holds with `reason`.
    pub fn hold_escalation_role_for_reason(mut self, reason: &str, role: &str) -> Self {
        let reason = reason.trim().to_lowercase();
        self.config
            .hold_escalation_role_by_reason
            .retain(|(listed, _)| *listed != reason);
        self.config
            .hold_escalation_role_by_reason
            .push((reason, role.trim().to_owned()));
        self
    }

    /// The escalation role for the holds on the location with `code`.
    pub fn hold_escalation_role_at(mut self, code: &str, role: &str) -> Self {
        let code = code.trim().to_owned();
        self.config
            .hold_escalation_role_by_location
            .retain(|(listed, _)| *listed != code);
        self.config
            .hold_escalation_role_by_location
            .push((code, role.trim().to_owned()));
        self
    }

    /// The route of the vertical's page of a recall (taking the recall's id):
    /// where its patients or customers are named and told (05 `clinics.recall`).
    /// The recall page links to it and leaves the notices to it.
    pub fn recall_notices_route(mut self, route: &str) -> Self {
        self.config.recall_notices_route = Some(route.trim().to_owned());
        self
    }

    /// The settings of the supplier score: weights, caps, grace days, window,
    /// minimum receipts and the threshold (defaults in [`ScoreSettings`]).
    pub fn supplier_score(mut self, settings: ScoreSettings) -> Self {
        self.config.score = settings;
        self
    }

    /// Whether changing the preferred supplier of an item needs a second
    /// person who holds `item_suppliers.approve` (default on; `STK.ACL.07`).
    /// Off, a change applies at once, audited with the user, and
    /// `PreferredSupplierChanged` names the requester as the approver.
    pub fn preferred_change_needs_approval(mut self, needed: bool) -> Self {
        self.config.preferred_change_needs_approval = needed;
        self
    }

    /// Adds a rejection reason, or changes whether an existing one needs a
    /// reading or a photo (06: `temperature`).
    pub fn rejection_reason(mut self, code: &str, evidence_required: bool) -> Self {
        let reasons = &mut self.config.rejection_reasons;
        match reasons.iter_mut().find(|r| r.code == code) {
            Some(reason) => reason.evidence_required = evidence_required,
            None => reasons.push(RejectionReason {
                code: code.to_owned(),
                evidence_required,
            }),
        }
        self
    }

    /// The module's settings.
    pub fn config(&self) -> &InventoryConfig {
        &self.config
    }
}

/// The module's settings in `state`: those `register` provided, else the §7
/// defaults.
pub(crate) fn config_of(state: &AppState) -> Arc<InventoryConfig> {
    state
        .provided::<InventoryConfig>()
        .unwrap_or_else(|| Arc::new(InventoryConfig::default()))
}

impl Module for WlInventory {
    fn name(&self) -> &'static str {
        "wl-inventory"
    }

    fn routes(&self) -> Routes {
        pages::routes()
            .merge(supplier_pages::routes())
            .merge(preferred_change::routes())
            .merge(stock_pages::routes())
            .merge(recall_pages::routes())
            .merge(stock_screens::routes())
            .merge(recall_screens::routes())
            .merge(transfer_screens::routes())
            .merge(receipt_screens::routes())
            .merge(dispatch_pages::routes())
            .merge(dispatch_deliver_pages::routes())
            .merge(dispatch_screens::routes())
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<screens::SuppliersPage>()
            .view_data::<screens::ReorderPage>()
            .view_data::<screens::ScorePage>()
            .view_data::<screens::RequestsPage>()
            .view_data::<screens::RequestPage>()
            .view_data::<stock_screens::StockPage>()
            .view_data::<stock_screens::LotPage>()
            .view_data::<stock_screens::HoldsPage>()
            .view_data::<stock_screens::PendingPage>()
            .view_data::<recall_screens::RecallsPage>()
            .view_data::<recall_screens::RecallPage>()
            .view_data::<transfer_screens::TransfersPage>()
            .view_data::<receipt_screens::OrdersPage>()
            .view_data::<receipt_screens::ReceiptsPage>()
            .view_data::<receipt_screens::ReceiptPage>()
            .view_data::<dispatch_screens::DispatchesPage>()
            .view_data::<dispatch_screens::DispatchPage>();
        app.templates(|env| {
            for (name, source) in screens::VIEWS
                .iter()
                .chain(stock_screens::VIEWS)
                .chain(recall_screens::VIEWS)
                .chain(dispatch_screens::VIEWS)
            {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        // The requests that wait for a second person, in the app group of the
        // sidebar; the label is the app's `nav.preferred_requests` text.
        app.nav(
            "app",
            NavLink::route("wl_inventory.preferred_requests", "nav.preferred_requests")
                .icon("truck")
                .group("nav.app")
                .order(60)
                .when(|user| {
                    [
                        permissions::ITEM_SUPPLIERS_APPROVE,
                        permissions::ITEM_SUPPLIERS_MANAGE,
                    ]
                    .iter()
                    .any(|permission| {
                        user.allows(permission) || user.has_permission_anywhere(permission)
                    })
                }),
        );
        // The stock screens, in the app group of the sidebar for those who
        // hold the permission of each somewhere; the labels are the app's
        // `nav.stock`, `nav.receive_stock`, `nav.stock_transfers`,
        // `nav.stock_holds`, `nav.pending_issues` and `nav.dispatches` texts.
        for (route, label, icon, order, held) in [
            (
                "wl_inventory.stock",
                "nav.stock",
                "package",
                64,
                &[permissions::STOCK_VIEW][..],
            ),
            (
                "wl_inventory.orders",
                "nav.receive_stock",
                "inbox",
                65,
                &[permissions::GOODS_RECEIPTS_POST][..],
            ),
            (
                "wl_inventory.transfers",
                "nav.stock_transfers",
                "truck",
                66,
                &[permissions::STOCK_TRANSFER][..],
            ),
            (
                "wl_inventory.holds",
                "nav.stock_holds",
                "shield",
                67,
                &[permissions::STOCK_HOLD, permissions::LOTS_RELEASE][..],
            ),
            (
                "wl_inventory.pending_list",
                "nav.pending_issues",
                "clipboard-list",
                68,
                &[permissions::STOCK_ADJUST][..],
            ),
            (
                "wl_inventory.recalls",
                "nav.recalls",
                "triangle-alert",
                69,
                &[permissions::RECALLS_VIEW, permissions::RECALLS_MANAGE][..],
            ),
            (
                "wl_inventory.dispatches",
                "nav.dispatches",
                "truck",
                70,
                &[permissions::DISPATCH_MANAGE][..],
            ),
        ] {
            app.nav(
                "app",
                NavLink::route(route, label)
                    .icon(icon)
                    .group("nav.app")
                    .order(order)
                    .when(move |user| {
                        held.iter().any(|permission| {
                            user.allows(permission) || user.has_permission_anywhere(permission)
                        })
                    }),
            );
        }
        // A page that holds only the id of a lot, a hold, a transfer or a
        // receipt (an approval, a message) shows its number.
        stock_screens::register_labels();
        dispatch_screens::register_labels();
        app.model::<Item>();
        app.model::<Location>();
        app.model::<ItemSupplier>();
        app.model::<PreferredSupplierRequest>();
        app.model::<ReorderSuggestion>();
        app.model::<supplier_score::SupplierDebitNoteRecord>();
        app.model::<SupplierScore>();
        app.model::<SupplierScoreNote>();
        app.model::<SupplierReview>();
        app.model::<PurchaseOrder>();
        app.model::<PurchaseOrderLine>();
        app.model::<PurchaseOrderRevision>();
        app.model::<PurchaseRequest>();
        app.model::<StockCount>();
        app.model::<LandedCost>();
        app.model::<landed_cost::LandedCostReceipt>();
        app.model::<landed_cost::CostLine>();
        app.model::<landed_cost::Allocation>();
        app.model::<landed_cost::Share>();
        app.model::<LandedCostAdjustment>();
        app.model::<Lot>();
        app.model::<StockMovement>();
        app.model::<Transfer>();
        app.model::<TransferLine>();
        app.model::<StockHold>();
        app.model::<StockHoldLine>();
        app.model::<StockHoldItem>();
        app.model::<Recall>();
        app.model::<RecallScope>();
        app.model::<RecallLot>();
        app.model::<RecallLine>();
        app.model::<PendingIssue>();
        app.model::<ShortageAlert>();
        app.model::<Dispatch>();
        app.model::<DispatchLine>();
        app.model::<GoodsReceipt>();
        app.model::<GoodsReceiptLine>();
        app.model::<SupplierReturn>();
        app.model::<SupplierReturnLine>();
        app.provide(self.config.clone());
        renox::state_machine::register::<PurchaseOrder>();
        renox::state_machine::register::<PurchaseRequest>();
        renox::state_machine::register::<StockCount>();
        renox::state_machine::register::<GoodsReceipt>();
        renox::state_machine::register::<Dispatch>();
        renox::state_machine::register::<Lot>();
        renox::state_machine::register::<Transfer>();
        renox::state_machine::register::<StockHold>();
        renox::state_machine::register::<StockHoldLine>();
        renox::state_machine::register::<Recall>();
        renox::state_machine::register::<PendingIssue>();
        renox::state_machine::register::<SupplierReturn>();
        renox::state_machine::register::<LandedCost>();
        renox::state_machine::register::<LandedCostAdjustment>();
        renox::state_machine::register::<ReorderSuggestion>();
        renox::state_machine::register::<SupplierReview>();
        renox::state_machine::register::<PreferredSupplierRequest>();
        // An over-receipt above the tolerance sends `post` to approval by a
        // holder of `purchase_orders.issue` in the location's scope
        // (`STK.ACL.04`, `PLT.APR.01`); the kernel runs `post_approved`.
        wl_kernel::SubjectMachine::of::<GoodsReceipt>(
            wl_kernel::inventory::GOODS_RECEIPT_SUBJECT_TYPE,
        )
        .action(receipt_post::POST, receipt_post::ACTION)
        .scope(|receipt: &GoodsReceipt| receipt.scope())
        .register();
        // Dispatching a return above the value needs a second holder of
        // `supplier_returns.manage`, never the requester (`STK.ACL.06`).
        wl_kernel::SubjectMachine::of::<SupplierReturn>(
            wl_kernel::inventory::SUPPLIER_RETURN_SUBJECT_TYPE,
        )
        .action(return_dispatch::DISPATCH, return_dispatch::ACTION)
        .action(return_credit::CREDIT, return_credit::CREDIT_ACTION)
        .scope(|_ret: &SupplierReturn| renox::auth::permissions::Scope::global())
        .register();
        // A revision that raises an order's total above a limit needs an
        // approval per limit the new total is above (`STK.ACL.03`); the
        // strictest policy that matches decides the steps.
        wl_kernel::SubjectMachine::of::<PurchaseOrder>(
            wl_kernel::inventory::PURCHASE_ORDER_SUBJECT_TYPE,
        )
        .action(po_revise::REVISE, po_revise::ACTION)
        .action(po_revise::REVISE_PART, po_revise::ACTION)
        .scope(|_order: &PurchaseOrder| renox::auth::permissions::Scope::global())
        // Issuing starts new business with the supplier: a blocked one waits
        // for an approved exception (`PTY.ACL.03`).
        .block_guarded(
            po_issue::ISSUE,
            wl_kernel::ActionKind::Start,
            |order: &PurchaseOrder| Some(order.supplier()),
        )
        .register();
        for (index, limit) in self.config.order_limits_fils.iter().enumerate() {
            let mut policy = wl_kernel::PolicyDefault::new(po_revise::ACTION).amount_above(*limit);
            for _ in 0..=index {
                policy = policy.step_permission(permissions::PURCHASE_ORDERS_ISSUE);
            }
            policy.register();
        }
        wl_kernel::approvals::register_final_check(
            wl_kernel::inventory::PURCHASE_ORDER_SUBJECT_TYPE,
            |tx, request| Box::pin(po_revise::recheck(tx, request)),
        );
        po_revise::register_types();
        // A late bill coded to a landed cost drafts an adjustment and tells
        // the purchaser.
        landed_draft::register_types();
        // Posting an adjustment of a landed cost above the value needs a
        // second holder of `landed_costs.post` (`STK.ACL.06`).
        wl_kernel::SubjectMachine::of::<LandedCost>(wl_kernel::inventory::LANDED_COST_SUBJECT_TYPE)
            .action(landed_adjust::ADJUST, landed_adjust::ACTION)
            .scope(|_landed: &LandedCost| renox::auth::permissions::Scope::global())
            .register();
        wl_kernel::PolicyDefault::new(landed_adjust::ACTION)
            .amount_above(self.config.landed_cost_approval_fils)
            .step_permission(permissions::LANDED_COSTS_POST)
            .register();
        wl_kernel::approvals::register_final_check(
            wl_kernel::inventory::LANDED_COST_SUBJECT_TYPE,
            |tx, request| Box::pin(landed_adjust::recheck(tx, request)),
        );
        // wl-invoicing codes a bill line to a landed cost it knows.
        wl_kernel::invoicing::provide_landed_costs(app, landed_cost::LandedCostBook);
        wl_kernel::PolicyDefault::new(return_dispatch::ACTION)
            .amount_above(self.config.return_approval_fils)
            .step_permission(permissions::SUPPLIER_RETURNS_MANAGE)
            .register();
        // Wastage above the value (a credit the supplier refused) needs a
        // second holder as well (`STK.ACL.02`).
        wl_kernel::PolicyDefault::new(return_credit::CREDIT_ACTION)
            .amount_above(self.config.wastage_approval_fils)
            .step_permission(permissions::SUPPLIER_RETURNS_MANAGE)
            .register();
        wl_kernel::PolicyDefault::new(receipt_post::ACTION)
            .percentage_above(self.config.over_receipt_tolerance_bps)
            .step_permission(permissions::PURCHASE_ORDERS_ISSUE)
            .register();
        // The supplier bill match reads the accepted quantity (§6).
        wl_kernel::inventory::provide_purchase_match(app, PurchaseMatch);
        // A vertical that stops a scope lists its open orders, cancels its
        // open purchase requests and opens the count of its stores.
        wl_kernel::inventory::provide_scope_stock(app, scope_stock::InventoryScopeStock);
        // `StockHoldService`: a vertical automation places a hold on the
        // stock at a location (06: an out-of-range reading).
        wl_kernel::inventory::provide_stock_hold_service(app, hold_service::InventoryHolds);
        hold_service::register_types();
        // `STK.RUL.06`: the storekeeper is told when stock runs short, and
        // after H hours gets a task and the manager is told.
        shortage_alert::register_types();
        shortage_alert::schedule(app);
        // `StockService`: another module consumes stock automatically; the
        // short part becomes a pending issue. Clearing one above the value
        // (`STK.ACL.02`) needs a second holder of `stock.approve`.
        wl_kernel::inventory::provide_stock_service(app, stock_service::InventoryStock);
        // 07 Manufacturing: the movements of a work order (return, wastage,
        // output) post inside the transaction of its transition.
        wl_kernel::production::provide_production_stock(app, production_stock::InventoryProduction);
        wl_kernel::SubjectMachine::of::<PendingIssue>(
            wl_kernel::inventory::PENDING_ISSUE_SUBJECT_TYPE,
        )
        .action(pending_issue::CLEAR, pending_issue::ACTION)
        .scope(|_pending: &PendingIssue| renox::auth::permissions::Scope::global())
        .register();
        wl_kernel::PolicyDefault::new(pending_issue::ACTION)
            .amount_above(self.config.wastage_approval_fils)
            .step_permission(permissions::STOCK_APPROVE)
            .register();
        // Each line of a hold is decided by a holder of `lots.release`. A
        // release for a listed reason (`recall`, `STK.ACL.05`) and a discard
        // above the wastage value (`STK.ACL.02`) wait for a second holder,
        // never the decider; the kernel runs `release_approved` or
        // `discard_approved` on the last yes.
        wl_kernel::SubjectMachine::of::<StockHoldLine>(
            wl_kernel::inventory::STOCK_HOLD_LINE_SUBJECT_TYPE,
        )
        .action(hold_line::RELEASE, hold_line::RELEASE_ACTION)
        .action(hold_line::DISCARD, hold_line::DISCARD_ACTION)
        .scope(|line: &StockHoldLine| line.scope())
        .register();
        wl_kernel::PolicyDefault::new(hold_line::RELEASE_ACTION)
            .amount_above(0)
            .input_key(hold_line::SECOND_PERSON_KEY)
            .step_permission(permissions::LOTS_RELEASE)
            .register();
        wl_kernel::PolicyDefault::new(hold_line::DISCARD_ACTION)
            .amount_above(self.config.wastage_approval_fils)
            .step_permission(permissions::LOTS_RELEASE)
            .register();
        events::register(app);
        dispatch_events::register(app);
        dispatch_deliver::register(app);
        dispatch_loss::register(app);
        dispatch_reverse::register(app);
        hold_decide::register(app);
        // `STK.RUL.07`: the deciders of an open hold are reminded every H
        // hours and the escalation role told every 2 x H.
        hold_reminder::register_types();
        // Opening a recall holds every lot in scope and tells the owner, the
        // storekeepers and procurement.
        recall::register_types();
        // The trace of a recall (who got the goods), its notices to the
        // parties and its close with the report.
        recall_notice::register_types();
        recall_notice::register(app);
        recall_trace::register(app);
        recall_close::register(app);
        wl_kernel::inventory::provide_lot_trace(app, recall_trace::InventoryTrace);
        hold_reminder::schedule(app);
        // `STK.RUL.08`: the notice type and the daily chase of the returns
        // that stay `dispatched`.
        chase::register_types();
        chase::schedule(app);
        // `STK.RUL.04`: the daily reorder suggestions, and the rows of a
        // merged supplier.
        reorder::register_types();
        reorder::schedule(app);
        // The supplier score: debit notes are kept as they are issued, the
        // snapshots are built daily at 06:00 and catch up a missed day 1.
        supplier_score::schedule(app);
        // `STK.RUL.09`: the review task after a snapshot under the threshold.
        supplier_review::register_types();
        // `STK.ACL.07`: the notice to the requester of a preferred supplier
        // change that was decided.
        preferred_change::register_types();
        app.listen(
            |event: wl_kernel::inventory::StockNegative, state: AppState| async move {
                shortage_alert::on_negative(&state, &event).await
            },
        );
        app.listen(
            |event: wl_kernel::invoicing::SupplierDebitNoteIssued, state: AppState| async move {
                supplier_score::on_debit_note_issued(&state, &event).await
            },
        );
        app.listen(
            |event: wl_kernel::invoicing::SupplierDebitNoteCancelled, state: AppState| async move {
                supplier_score::on_debit_note_cancelled(&state, &event).await
            },
        );
        app.listen(
            |event: wl_kernel::PartiesMerged, state: AppState| async move {
                item_supplier::on_merged(&state, &event).await
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_spec_values() {
        let module = WlInventory::new();
        assert_eq!(module.config().over_receipt_tolerance_bps, 500);
        assert_eq!(module.config().rejection_reasons.len(), 7);
    }

    #[test]
    fn the_reasons_that_need_a_second_person_are_recall_unless_set() {
        let module = WlInventory::new();
        assert!(module.config().release_needs_second_person("recall"));
        assert!(!module.config().release_needs_second_person("quality"));
        let module = WlInventory::new().hold_second_person_reasons(&[" Quality ", "recall"]);
        assert!(module.config().release_needs_second_person("quality"));
        assert!(module.config().release_needs_second_person("recall"));
        assert!(!module.config().release_needs_second_person("temperature"));
    }

    #[test]
    fn the_builder_changes_the_settings() {
        let module = WlInventory::new()
            .over_receipt_tolerance_bps(1_000)
            .rejection_reason("temperature", true)
            .rejection_reason("counterfeit", false);
        assert_eq!(module.config().over_receipt_tolerance_bps, 1_000);
        assert!(
            module
                .config()
                .reason("temperature")
                .is_some_and(|r| r.evidence_required)
        );
        assert_eq!(module.config().rejection_reasons.len(), 8);
        assert_eq!(
            WlInventory::new()
                .over_receipt_tolerance_bps(-5)
                .config()
                .over_receipt_tolerance_bps,
            0
        );
    }
}
