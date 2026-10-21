//! Procurement and stock: the items and the stores of the projects, the
//! suppliers of each item, purchase orders in every state the module can put
//! them in, goods receipts posted by the storekeeper (one with goods
//! rejected, which makes a return to the supplier and a draft debit note), a
//! reorder suggestion, a request to change the preferred supplier and the
//! supplier scores.
//!
//! Purchase orders and purchase requests have no screen that creates them
//! yet, so they are inserted in their first state (`draft`, and `submitted`
//! for the request that waits for the MD) with the module's models; from
//! there on everything goes through the routes: `issue` by procurement,
//! `post` by the storekeeper.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_inventory::goods_receipt::{DraftLine, DraftReceipt, create_draft};
use wl_inventory::{
    Item, Location, PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus, PurchaseRequest,
    PurchaseRequestStatus, RejectedHandling,
};
use wl_kernel::PartyId;

use super::at;
use super::people::{self, email_of};
use super::roles::{ADMIN, PROCUREMENT, STOREKEEPER};
use super::stock_plans::{ITEMS, ORDERS, Order};
use super::{Seed, day, text, upto};

/// `date` plus `days`, never later than today.
fn on(offset: i64) -> NaiveDate {
    upto(day(offset))
}

impl Seed {
    /// The store of project `code` (made when the project is awarded).
    pub(crate) async fn store_of(&self, code: &str) -> Result<Location> {
        Location::query()
            .where_(Location::CODE.eq(format!("STORE-{code}").as_str()))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no store for {code}").into())
    }

    /// The stores, items and suppliers of the items, then the orders.
    pub(crate) async fn stock(&mut self) -> Result {
        let procurement = email_of(PROCUREMENT);
        // The items, with their suppliers. The first supplier of an item is
        // set directly; a later change of the preferred one is a request.
        at!(self, day(-200), {
            for plan in ITEMS {
                let mut item = Item::new(plan.code, plan.en, plan.unit)
                    .reordered(plan.point * 1_000, plan.max * 1_000);
                item.name_ar = Some(plan.ar.to_owned());
                item.save(self.db()).await?;
                self.items.insert(plan.code, item.id);
                self.report.add("items", 1);
                let lead = plan.lead_days.to_string();
                let preferred = self.party(plan.preferred).id.to_string();
                self.driver
                    .post(
                        procurement,
                        &format!("/items/{}/suppliers/add", item.id),
                        &[
                            ("supplier", preferred.as_str()),
                            ("role", "preferred"),
                            ("lead_days", lead.as_str()),
                            ("unit", plan.unit),
                            ("code", plan.code),
                        ],
                    )
                    .await?;
                if let Some(alternative) = plan.alternative {
                    let alternative = self.party(alternative).id.to_string();
                    self.driver
                        .post(
                            procurement,
                            &format!("/items/{}/suppliers/add", item.id),
                            &[
                                ("supplier", alternative.as_str()),
                                ("role", "alternative"),
                                ("rank", "1"),
                                ("lead_days", "10"),
                            ],
                        )
                        .await?;
                }
            }
            Ok::<(), renox::Error>(())
        })?;
        for order in ORDERS {
            self.order(order).await?;
        }
        Ok(())
    }

    /// One purchase order, from the draft to its receipts.
    async fn order(&mut self, plan: &Order) -> Result {
        let procurement = email_of(PROCUREMENT);
        let storekeeper = email_of(STOREKEEPER);
        let store = self.store_of(plan.project).await?;
        let supplier = PartyId(self.party(plan.supplier).id);
        let written = on(plan.written);
        let order = at!(self, written, {
            let mut order = PurchaseOrder::new(supplier, store.id);
            // There is no screen that writes a purchase order yet: it is made
            // in its first state, the draft.
            let mut tx = self.db().begin().await?;
            order
                .insert_in_state(
                    &mut tx,
                    PurchaseOrderStatus::Draft,
                    "Written by procurement",
                )
                .await?;
            tx.commit().await?;
            for (position, (code, units, price)) in (1..).zip(plan.lines) {
                let line = PurchaseOrderLine::new(
                    order.id,
                    position,
                    self.items[code],
                    units * 1_000,
                    *price,
                );
                let mut line = line;
                line.save(self.db()).await?;
            }
            Ok::<PurchaseOrder, renox::Error>(order)
        })?;
        self.report.add("purchase orders", 1);
        if !plan.issue {
            return Ok(());
        }
        let issued = on(plan.written + 1);
        at!(self, issued, {
            self.driver
                .post(
                    procurement,
                    &format!("/purchase-orders/{}/issue", order.id),
                    &[],
                )
                .await
        })?;
        let lines = wl_inventory::purchase_order::lines_of(self.db(), order.id).await?;
        let storekeeper_id = self.user_id(storekeeper).await?;
        for receipt in plan.receipts {
            let when = on(receipt.day);
            let draft = DraftReceipt {
                purchase_order_id: order.id,
                location_id: store.id,
                delivery_note: Some(format!("DN-{}-{}", plan.key, -receipt.day)),
                lines: receipt
                    .lines
                    .iter()
                    .map(|(position, came, accepted)| {
                        let line = &lines[position - 1];
                        let mut draft = DraftLine::on(line.id, came * 1_000, accepted * 1_000);
                        if came > accepted {
                            let handling = if receipt.refused_at_door {
                                RejectedHandling::RefusedAtDoor
                            } else {
                                RejectedHandling::KeptForCollection
                            };
                            draft = draft
                                .rejected_for("damaged", handling)
                                .evidence("PHOTO-DAMAGED-BOARDS-01")
                                .credit();
                        }
                        draft
                    })
                    .collect(),
            };
            at!(self, when, {
                let made = create_draft(self.db(), Some(storekeeper_id), draft).await?;
                self.driver
                    .post(
                        storekeeper,
                        &format!("/goods-receipts/{}/post", made.id),
                        &[],
                    )
                    .await
            })?;
            self.report.add("goods receipts", 1);
        }
        Ok(())
    }

    /// The material request above the MD's limit, the reorder suggestions,
    /// the request to change a preferred supplier, the supplier scores.
    pub(crate) async fn stock_rules(&mut self) -> Result {
        let procurement = email_of(PROCUREMENT);
        let admin = email_of(ADMIN);
        let today = crate::app::demo_data::real_today();
        // The preferred supplier of the cable is questioned: procurement asks
        // to move it to the subcontractor, who is also cheaper; it waits.
        let cable = self.items["CABLE-25"].to_string();
        let mep = self.party(people::MEP).id.to_string();
        at!(self, day(-9), {
            self.driver
                .post(
                    procurement,
                    &format!("/items/{cable}/preferred-requests"),
                    &[("supplier", mep.as_str())],
                )
                .await
        })?;
        // The PVC pipe's preferred supplier was changed, and the MD approved.
        let pvc = self.items["PVC-110"].to_string();
        let gypsum = self.party(people::GYPSUM).id.to_string();
        at!(self, day(-14), {
            self.driver
                .post(
                    procurement,
                    &format!("/items/{pvc}/preferred-requests"),
                    &[("supplier", gypsum.as_str())],
                )
                .await
        })?;
        let request = wl_inventory::PreferredSupplierRequest::query()
            .where_(wl_inventory::PreferredSupplierRequest::ITEM_ID.eq(self.items["PVC-110"]))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no preferred request"))?;
        at!(self, day(-12), {
            self.driver
                .post(
                    admin,
                    &format!("/preferred-requests/{}/approve", request.id),
                    &[],
                )
                .await
        })?;
        // A material request above AED 50,000 waits for the MD. The request
        // has no submit screen yet, so it is made in the state it waits in.
        let store = self.store_of("TWR-01").await?;
        let requester = self.user_id(email_of(STOREKEEPER)).await?;
        at!(self, day(-3), {
            let mut request = PurchaseRequest::new(store.id);
            request.cost_object = Some("construction.cost_code:gypsum".to_owned());
            request.needed_by = Some(day(10));
            request.requester_id = Some(requester);
            request.status = PurchaseRequestStatus::Submitted;
            let mut tx = self.db().begin().await?;
            request
                .insert_in_state(
                    &mut tx,
                    PurchaseRequestStatus::Submitted,
                    "Submitted by the storekeeper; above the MD's limit",
                )
                .await?;
            tx.commit().await?;
            Ok::<(), renox::Error>(())
        })?;
        // The daily rules of stock: the reorder suggestions, the supplier
        // scores and their reviews.
        at!(self, today, {
            wl_inventory::reorder::run(&self.state).await?;
            wl_inventory::supplier_score::run(&self.state).await
        })?;
        let _ = text(today);
        Ok(())
    }
}
