//! The stock of the demo clinic: one store per branch with consumables and
//! their lots (one near its expiry, one on hold for quality, one recalled
//! by its supplier and traced to the patients it was used on), a transfer
//! between the branches, one in transit, and an issue beyond the stock that
//! waits as a pending issue.
//!
//! Items, stores and the opening balances are the module's own records (the
//! receipts of the opening stock are movements, as a count would leave
//! them); holds, the recall, the trace, the transfers and the notices are
//! requests to the app's router as the owner, the clinic manager and the storekeeper,
//! and the consumables used on visits are issued by the visit's own billing.

use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_inventory::item::value_of;
use wl_inventory::movement::StockMovement;
use wl_inventory::transfer::{self, NewTransfer, NewTransferLine};
use wl_inventory::{Item, Location, Lot, LotStatus};
use wl_kernel::Subject;

use super::super::demo_data::day;
use super::{BUSINESS_BAY, JUMEIRAH, Seed, branch};

const STORE: &str = "store@palmgrove.example";
const OWNER: &str = "owner@palmgrove.example";
const BAY_MANAGER: &str = "manager.bay@palmgrove.example";

/// The lot of anaesthetic cartridges the supplier recalls.
pub(crate) const RECALLED_LOT: &str = "L-A2199";

impl Seed {
    async fn item(&self, item: Item) -> Result<Item> {
        let mut item = item;
        item.save(self.db()).await?;
        Ok(item)
    }

    async fn receive(
        &self,
        item: &Item,
        store: &Location,
        lot: Option<i64>,
        units: i64,
        cost: i64,
    ) -> Result {
        let quantity = units * 1_000;
        let mut movement = StockMovement::receive(
            item.id,
            lot,
            store.id,
            quantity,
            cost,
            value_of(quantity, cost),
            &Subject::new("inventory.goods_receipt", 900 + item.id),
            Some(self.user(STORE).id),
        );
        movement.save(self.db()).await?;
        Ok(())
    }

    /// A released lot of `item` expiring in `left` days.
    async fn lot(&self, item: &Item, number: &str, left: i64) -> Result<Lot> {
        let mut lot = Lot {
            item_id: item.id,
            number: number.to_owned(),
            supplier_lot: Some(format!("SUP-{number}")),
            expiry_date: Some(day(left)),
            status: LotStatus::Released,
            ..Lot::default()
        };
        let mut tx = self.db().begin().await?;
        lot.insert_in_state(&mut tx, LotStatus::Released, "opening stock")
            .await?;
        tx.commit().await?;
        Ok(lot)
    }

    pub(crate) async fn stock(&mut self) -> Result {
        let jumeirah = self.store("JUM-STORE", "Jumeirah store", JUMEIRAH).await?;
        let bay = self
            .store("BAY-STORE", "Business Bay store", BUSINESS_BAY)
            .await?;
        let gloves = self
            .item(
                Item::new("GLOVES-M", "Nitrile gloves, medium (box of 100)", "box")
                    .reordered(10_000, 60_000),
            )
            .await?;
        let syringes = self
            .item(
                Item::new("SYRINGE-5ML", "Syringes 5 ml (box of 100)", "box")
                    .reordered(5_000, 20_000),
            )
            .await?;
        let cartridges = self
            .item(
                Item::new(
                    "ANAES-CART",
                    "Local anaesthetic cartridges (box of 50)",
                    "box",
                )
                .lot_tracked(),
            )
            .await?;
        self.receive(&gloves, &jumeirah, None, 40, 1_200).await?;
        self.receive(&gloves, &bay, None, 4, 1_200).await?;
        // Syringes: none at Jumeirah, so a consumable issued there waits as
        // a pending issue.
        self.receive(&syringes, &bay, None, 12, 900).await?;
        // The lots of cartridges at Business Bay: one expires in 18 days,
        // one is for the quality hold, one is the recalled lot, one is fine.
        for (number, left, units) in [
            ("L-A2301", 18, 20),
            ("L-A2307", 400, 30),
            ("L-A2288", 200, 15),
            (RECALLED_LOT, 45, 10),
        ] {
            let lot = self.lot(&cartridges, number, left).await?;
            self.receive(&cartridges, &bay, Some(lot.id), units, 4_500)
                .await?;
        }
        self.stores = vec![jumeirah.id, bay.id];
        self.items = vec![gloves.id, syringes.id, cartridges.id];
        Ok(())
    }

    async fn store(&self, code: &str, name: &str, at: i64) -> Result<Location> {
        let mut store = Location::new(code, name).in_scope(&branch(at));
        store.save(self.db()).await?;
        Ok(store)
    }

    /// The holds, the recall, the transfers and the notices, told after the
    /// visits used the lots.
    pub(crate) async fn recall(&mut self) -> Result {
        let (from, to) = (self.stores[0], self.stores[1]);
        let (gloves, cartridges) = (self.items[0], self.items[2]);
        let storekeeper = self.user(STORE).id;
        // Ten boxes of gloves go to Business Bay: one transfer arrived, one
        // is on the road.
        for (boxes, received) in [(10, true), (6, false)] {
            let draft = transfer::create_draft(
                self.db(),
                Some(storekeeper),
                NewTransfer {
                    from_location_id: from,
                    to_location_id: to,
                    lines: vec![NewTransferLine::of(gloves, boxes * 1_000)],
                },
            )
            .await?;
            self.driver
                .post(STORE, &format!("/transfers/{}/send", draft.id), &[])
                .await?;
            if received {
                self.driver
                    .post(STORE, &format!("/transfers/{}/receive", draft.id), &[])
                    .await?;
            }
        }
        // A lot is held for quality: a cartridge cracked in the box.
        let suspect = Lot::query()
            .where_(Lot::NUMBER.eq("L-A2288"))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the lot L-A2288 is missing"))?;
        self.driver
            .post(
                BAY_MANAGER,
                "/stock-holds",
                &[
                    ("reason", "quality"),
                    ("reference", "ticketing.ticket:1"),
                    ("lots", &suspect.id.to_string()),
                ],
            )
            .await?;
        // The supplier recalls a lot: it is held everywhere, traced to the
        // visits it was used on, and the patients are told.
        self.driver
            .post(
                OWNER,
                "/recalls",
                &[
                    ("source", "supplier"),
                    (
                        "reason",
                        "The supplier found particles in lot L-A2199 of anaesthetic cartridges.",
                    ),
                    ("items", &cartridges.to_string()),
                    ("lot_numbers", RECALLED_LOT),
                ],
            )
            .await?;
        let recall = wl_inventory::Recall::query()
            .order_by_desc(wl_inventory::Recall::ID)
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the recall was not opened"))?;
        self.driver
            .post(OWNER, &format!("/recalls/{}/trace", recall.id), &[])
            .await?;
        self.driver
            .post(
                BAY_MANAGER,
                &format!("/clinics/recalls/{}/notices", recall.id),
                &[],
            )
            .await?;
        self.recall_id = recall.id;
        Ok(())
    }
}
