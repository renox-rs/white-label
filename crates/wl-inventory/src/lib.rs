#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod config;
mod events;
pub mod goods_receipt;
pub mod item;
pub mod location;
pub mod lot;
pub mod match_source;
pub mod movement;
pub mod pages;
pub mod purchase_order;
pub mod receipt_post;
pub mod receipt_reverse;
pub mod return_dispatch;
pub mod return_note;
pub mod supplier_return;
mod texts;

use std::sync::Arc;

use renox::db::Migration;
use renox::prelude::*;

pub use config::{InventoryConfig, RejectionReason};
pub use goods_receipt::{
    DraftLine, DraftReceipt, GoodsReceipt, GoodsReceiptLine, GoodsReceiptStatus,
};
pub use item::{Item, ItemType};
pub use location::{Location, LocationType, NegativeStockPolicy};
pub use lot::{Lot, LotStatus};
pub use match_source::PurchaseMatch;
pub use movement::{MovementType, StockMovement};
pub use purchase_order::{PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus};
pub use receipt_post::{PostOutcome, post};
pub use receipt_reverse::reverse;
pub use return_dispatch::{CollectionProof, DispatchOutcome, dispatch};
pub use supplier_return::{
    RejectedHandling, ReturnOrigin, ReturnStatus, SupplierReturn, SupplierReturnLine, WantedOutcome,
};

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
    /// Manage returns to supplier.
    pub const SUPPLIER_RETURNS_MANAGE: &str = "supplier_returns.manage";
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
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Item>();
        app.model::<Location>();
        app.model::<PurchaseOrder>();
        app.model::<PurchaseOrderLine>();
        app.model::<Lot>();
        app.model::<StockMovement>();
        app.model::<GoodsReceipt>();
        app.model::<GoodsReceiptLine>();
        app.model::<SupplierReturn>();
        app.model::<SupplierReturnLine>();
        app.provide(self.config.clone());
        renox::state_machine::register::<PurchaseOrder>();
        renox::state_machine::register::<GoodsReceipt>();
        renox::state_machine::register::<Lot>();
        renox::state_machine::register::<SupplierReturn>();
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
        .scope(|_ret: &SupplierReturn| renox::auth::permissions::Scope::global())
        .register();
        wl_kernel::PolicyDefault::new(return_dispatch::ACTION)
            .amount_above(self.config.return_approval_fils)
            .step_permission(permissions::SUPPLIER_RETURNS_MANAGE)
            .register();
        wl_kernel::PolicyDefault::new(receipt_post::ACTION)
            .percentage_above(self.config.over_receipt_tolerance_bps)
            .step_permission(permissions::PURCHASE_ORDERS_ISSUE)
            .register();
        // The supplier bill match reads the accepted quantity (§6).
        wl_kernel::inventory::provide_purchase_match(app, PurchaseMatch);
        events::register(app);
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
