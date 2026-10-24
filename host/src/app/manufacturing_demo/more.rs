//! More of the demo company, so that the lists have something to page and
//! filter and the platform screens F7 uses are not empty: a purchase order
//! with its goods receipt, a stock transfer, a budget, a delivery claim as a
//! ticket and a recall that reaches a lot already shipped (the dispatch that
//! has not left is flagged for a re-pick, the one that left is to be brought
//! back).

use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_inventory::goods_receipt::{DraftLine, DraftReceipt, create_draft};
use wl_inventory::transfer::{self, NewTransfer, NewTransferLine};
use wl_inventory::{PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus};
use wl_kernel::{PartyId, PartyKind};
use wl_parties::{CompanyDetails, NewParty, PartyRole};

use super::{ACCOUNTANT_EMAIL, ADMIN_EMAIL, OWNER_EMAIL, SALES_EMAIL, STORE_EMAIL, Seed};

impl Seed {
    /// The supplier of the board, the purchase order for it, the goods
    /// receipt of the first delivery, and two transfers between the stores.
    async fn procurement(&mut self) -> Result {
        let supplier = wl_parties::create(
            &self.state,
            NewParty {
                kind: PartyKind::Company,
                legal_name_en: "Gulf Paper Mills FZE".to_owned(),
                company: Some(CompanyDetails {
                    legal_form: Some("fze".to_owned()),
                    ..CompanyDetails::default()
                }),
                roles: vec![PartyRole {
                    role_key: "supplier".to_owned(),
                    ..PartyRole::default()
                }],
                source: "seed".to_owned(),
                ..NewParty::default()
            },
        )
        .await?;
        let supplier = PartyId(supplier.party_id().0);
        let mut made = Vec::new();
        for lines in [
            vec![("KRAFT-LINER", 2_000, 380), ("FLUTING-MED", 1_000, 320)],
            vec![("SLEEVE-FILM", 300, 1_200)],
        ] {
            let mut order = PurchaseOrder::new(supplier, self.rm);
            order.expected_date = Some(super::day(5));
            let mut tx = self.db().begin().await?;
            order
                .insert_in_state(&mut tx, PurchaseOrderStatus::Draft, "Written by the plant")
                .await?;
            tx.commit().await?;
            for (position, (code, units, price)) in (1..).zip(lines) {
                PurchaseOrderLine::new(order.id, position, self.items[code], units * 1_000, price)
                    .save(self.db())
                    .await?;
            }
            self.driver
                .post(
                    ADMIN_EMAIL,
                    &format!("/purchase-orders/{}/issue", order.id),
                    &[],
                )
                .await?;
            made.push(order.id);
        }
        // The first delivery arrived: the storekeeper posts the receipt.
        let lines = wl_inventory::purchase_order::lines_of(self.db(), made[0]).await?;
        let keeper = self.user(STORE_EMAIL).await?.id;
        let receipt = create_draft(
            self.db(),
            Some(keeper),
            DraftReceipt {
                purchase_order_id: made[0],
                location_id: self.rm,
                delivery_note: Some("DN-GPM-4410".to_owned()),
                lines: vec![
                    DraftLine::on(lines[0].id, 2_000_000, 2_000_000)
                        .lot("KL-2342", super::day(330)),
                ],
            },
        )
        .await?;
        self.driver
            .post(
                STORE_EMAIL,
                &format!("/goods-receipts/{}/post", receipt.id),
                &[],
            )
            .await?;
        // Straps to the finished goods store: one transfer arrived, one is on
        // the road.
        for (units, received) in [(40, true), (25, false)] {
            let draft = transfer::create_draft(
                self.db(),
                Some(keeper),
                NewTransfer {
                    from_location_id: self.rm,
                    to_location_id: self.fg,
                    lines: vec![NewTransferLine::of(self.items["STRAP-1000"], units * 1_000)],
                },
            )
            .await?;
            self.driver
                .post(STORE_EMAIL, &format!("/transfers/{}/send", draft.id), &[])
                .await?;
            if received {
                self.driver
                    .post(
                        STORE_EMAIL,
                        &format!("/transfers/{}/receive", draft.id),
                        &[],
                    )
                    .await?;
            }
        }
        Ok(())
    }

    /// The company budget the accountant keeps, sent for approval.
    async fn budget(&mut self) -> Result {
        let year = renox::chrono::Datelike::year(&wl_kernel::documents::today());
        let reply = self
            .driver
            .post(
                ACCOUNTANT_EMAIL,
                "/budgets",
                &[
                    ("legal_entity_id", &self.entity.to_string()),
                    ("fiscal_year", &year.to_string()),
                    ("name", "Company budget"),
                    ("start", "empty"),
                    ("revision_reason", ""),
                ],
            )
            .await?;
        let id = reply
            .redirected_id()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the budget"))?;
        for (code, amount) in [
            ("sales_services", "4800000"),
            ("cost_of_sales", "2900000"),
            ("salaries_staff", "900000"),
            ("rent_utilities", "240000"),
            ("transport_fuel", "120000"),
            ("other_expense", "60000"),
        ] {
            let category = wl_invoicing::BudgetCategory::query()
                .where_(wl_invoicing::BudgetCategory::CODE.eq(code))
                .first(self.db())
                .await?
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the budget category {code}"))?;
            self.driver
                .post(
                    ACCOUNTANT_EMAIL,
                    &format!("/budgets/{id}/lines"),
                    &[
                        ("category_id", &category.id.to_string()),
                        ("from", &format!("{year}-01")),
                        ("to", &format!("{year}-12")),
                        ("amount", amount),
                    ],
                )
                .await?;
        }
        self.driver
            .post(ACCOUNTANT_EMAIL, &format!("/budgets/{id}/submit"), &[])
            .await?;
        Ok(())
    }

    /// Al Noor's claim for the damaged boxes, opened by sales.
    async fn claim(&mut self) -> Result {
        self.driver
            .post(
                SALES_EMAIL,
                "/tickets",
                &[
                    ("subject_type", "sales.sales_order"),
                    ("subject_id", &self.orders["noor"].0.to_string()),
                    ("category", "delivery_claim"),
                    ("title", "Al Noor: 40 boxes arrived damaged"),
                    (
                        "description",
                        "The driver's note says the pallet was crushed at the dock. Photos are on the delivery.",
                    ),
                ],
            )
            .await?;
        Ok(())
    }

    /// The internal recall of the finished food boxes lot: the dispatch that
    /// left (to bring back) and the one picked and waiting (re-pick flag).
    async fn finished_recall(&mut self) -> Result {
        self.driver
            .post(
                OWNER_EMAIL,
                "/recalls",
                &[
                    ("source", "internal"),
                    (
                        "reason",
                        "A glue batch failed the peel test on finished food boxes lot F-BOX-0901.",
                    ),
                    ("items", &self.items["BOX-FOOD"].to_string()),
                    ("lot_numbers", "F-BOX-0901"),
                ],
            )
            .await?;
        let recall = wl_inventory::Recall::query()
            .order_by_desc(wl_inventory::Recall::ID)
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the recall was not opened"))?;
        self.driver
            .post(OWNER_EMAIL, &format!("/recalls/{}/trace", recall.id), &[])
            .await?;
        Ok(())
    }

    /// Everything the lists and the platform screens need beyond the story.
    pub(crate) async fn more(&mut self) -> Result {
        self.more_work().await?;
        self.decided_returns().await?;
        self.procurement().await?;
        self.budget().await?;
        self.claim().await?;
        self.finished_recall().await?;
        Ok(())
    }
}
