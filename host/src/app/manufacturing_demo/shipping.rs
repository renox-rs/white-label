//! The dispatches of the demo company: a draft, one picked, one that left and
//! came back in part (returned goods for QC and sales), and two exports at
//! zero rate, one with its exit evidence and one 65 days out without it.

use renox::chrono::Duration;
use renox::prelude::*;
use wl_inventory::{Dispatch, NewDispatch, NewDispatchLine, create_dispatch};
use wl_kernel::delivery::{
    DeliveredLine, DeliveryDelivered, DeliveryId, DeliveryRefusedReceived, RefusedLineReceived,
    RunId,
};
use wl_kernel::invoicing::VatCategory;
use wl_kernel::{PartyId, Subject};

use manufacturing::returns::ReturnInspection;

use super::{ACCOUNTANT_EMAIL, QC_EMAIL, STORE_EMAIL, Seed, day};

impl Seed {
    /// A draft dispatch from the finished goods store of `lines` (item code,
    /// units, sale line) for `customer`'s order `order`.
    pub(crate) async fn dispatch(
        &self,
        customer: &str,
        order: &str,
        lines: &[(&str, i64, i64)],
    ) -> Result<Dispatch> {
        let party: PartyId = self.customers[customer];
        let (order_id, _) = self.orders[order];
        let name = wl_parties::Party::find_or_404(self.db(), party.0)
            .await?
            .legal_name_en;
        let by = self.user(STORE_EMAIL).await?.id;
        create_dispatch(
            self.db(),
            Some(by),
            NewDispatch {
                source: Subject::new("sales.sales_order", order_id),
                recipient: Some(party),
                recipient_name: name,
                address: "Sharjah Industrial Area, Sharjah".to_owned(),
                location_id: self.fg,
                lines: lines
                    .iter()
                    .map(|(code, units, sale_line)| {
                        let lot = match *code {
                            "BOX-FOOD" => "F-BOX-0901",
                            _ => "F-RTL-0901",
                        };
                        NewDispatchLine::of(self.items[code], units * 1_000)
                            .lot(self.lots[lot])
                            .for_sale_line(*sale_line)
                    })
                    .collect(),
            },
        )
        .await
    }

    pub(crate) async fn leave(&self, dispatch: &Dispatch, tracking: &str, days_ago: i64) -> Result {
        self.driver
            .post(
                STORE_EMAIL,
                &format!("/dispatches/{}/pick", dispatch.id),
                &[],
            )
            .await?;
        self.driver
            .post(
                STORE_EMAIL,
                &format!("/dispatches/{}/dispatch", dispatch.id),
                &[("handover", "courier"), ("tracking_number", tracking)],
            )
            .await?;
        if days_ago > 0 {
            Dispatch::query()
                .where_(Dispatch::ID.eq(dispatch.id))
                .update(
                    self.db(),
                    &[(
                        "dispatched_at",
                        &Some(renox::db::now() - Duration::days(days_ago)),
                    )],
                )
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn shipping(&mut self) -> Result {
        let (_, noor_tub_line) = self.orders["noor"];
        let noor_lines = wl_sales::SaleLine::of(
            self.db(),
            &Subject::new("sales.sales_order", self.orders["noor"].0),
        )
        .await?;
        let noor_lid_line = noor_lines.get(1).map_or(noor_tub_line, |l| l.id);

        // Left with a courier, delivered in part: 40 boxes came back damaged.
        let left = self
            .dispatch("noor", "noor", &[("BOX-FOOD", 600, noor_tub_line)])
            .await?;
        self.leave(&left, "AWB-77120", 3).await?;
        let line = wl_inventory::dispatch::lines_of(self.db(), left.id)
            .await?
            .remove(0);
        self.state
            .emit(DeliveryDelivered {
                delivery: DeliveryId(1),
                source: left.subject(),
                attempt: 1,
                proof: 1,
                lines: vec![DeliveredLine {
                    line: Some(line.subject()),
                    description: "Food-grade box 40x30x20".to_owned(),
                    quantity_milli: 600_000,
                    delivered_milli: 560_000,
                    refused_milli: 40_000,
                    refusal_reason: Some("damaged in transit".to_owned()),
                }],
                cash_expected: 0,
                cash_collected: 0,
                cod_difference: 0,
                cod_difference_reason: None,
                absorbed_by: None,
                cheque: None,
            })
            .await?;
        self.state
            .emit(DeliveryRefusedReceived {
                delivery: DeliveryId(1),
                source: left.subject(),
                run: RunId(1),
                lines: vec![RefusedLineReceived {
                    line: Some(line.subject()),
                    description: "Food-grade box 40x30x20".to_owned(),
                    refused_milli: 40_000,
                    received_milli: 40_000,
                    missing_milli: 0,
                    refusal_reason: Some("damaged in transit".to_owned()),
                }],
            })
            .await?;

        // A draft for Al Noor's retail boxes, and Gulf Fresh Foods' boxes picked and
        // waiting for the courier.
        self.dispatch("noor", "noor", &[("BOX-RTL", 100, noor_lid_line)])
            .await?;
        let (_, fresh_line) = self.orders["fresh"];
        let picked = self
            .dispatch("fresh", "fresh", &[("BOX-FOOD", 400, fresh_line)])
            .await?;
        self.driver
            .post(STORE_EMAIL, &format!("/dispatches/{}/pick", picked.id), &[])
            .await?;

        // Two exports at zero rate to Atlas Gulf Export.
        let (_, atlas_line) = self.orders["atlas"];
        let atlas = self.customers["atlas"];
        for (units, days_ago, tracking, evidence) in [
            (300, 65, "BL-ATL-0042", false),
            (150, 20, "BL-ATL-0057", true),
        ] {
            let sent = self
                .dispatch("atlas", "atlas", &[("BOX-RTL", units, atlas_line)])
                .await?;
            self.leave(&sent, tracking, days_ago).await?;
            self.invoice(
                atlas,
                "Cartons for export",
                units * 240,
                VatCategory::ZeroRated,
                60 - days_ago,
                sent.subject(),
            )
            .await?;
            if evidence {
                let kind = wl_documents::DocumentKind::by_key(
                    self.db(),
                    manufacturing::exports::CUSTOMS_EXIT,
                )
                .await?
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the customs exit kind"))?;
                wl_documents::Document::create(
                    self.db(),
                    wl_documents::Document {
                        subject_type: sent.subject().subject_type,
                        subject_id: sent.id,
                        kind_id: kind.id,
                        title: "Customs exit certificate".to_owned(),
                        number: Some("CEC-2026-88413".to_owned()),
                        expiry_date: None,
                        current_version: 1,
                        status: wl_documents::DocumentStatus::Accepted,
                        ..Default::default()
                    },
                )
                .await?;
            }
        }
        // The nightly rule has run: the accountant is warned about the first.
        manufacturing::exports::run_daily(&self.state, wl_kernel::documents::today()).await?;
        let _ = (ACCOUNTANT_EMAIL, day(0));
        Ok(())
    }

    /// A delivery of `dispatch` that came back in part: `refused` of the
    /// `units` sent were refused for `reason` and received at the plant, so
    /// the lot waits for QC under the delivery number `delivery`.
    async fn came_back(
        &self,
        dispatch: &Dispatch,
        delivery: i64,
        units: i64,
        refused: i64,
        reason: &str,
    ) -> Result {
        let line = wl_inventory::dispatch::lines_of(self.db(), dispatch.id)
            .await?
            .remove(0);
        self.state
            .emit(DeliveryDelivered {
                delivery: DeliveryId(delivery),
                source: dispatch.subject(),
                attempt: 1,
                proof: delivery,
                lines: vec![DeliveredLine {
                    line: Some(line.subject()),
                    description: "Retail display box".to_owned(),
                    quantity_milli: units * 1_000,
                    delivered_milli: (units - refused) * 1_000,
                    refused_milli: refused * 1_000,
                    refusal_reason: Some(reason.to_owned()),
                }],
                cash_expected: 0,
                cash_collected: 0,
                cod_difference: 0,
                cod_difference_reason: None,
                absorbed_by: None,
                cheque: None,
            })
            .await?;
        self.state
            .emit(DeliveryRefusedReceived {
                delivery: DeliveryId(delivery),
                source: dispatch.subject(),
                run: RunId(delivery),
                lines: vec![RefusedLineReceived {
                    line: Some(line.subject()),
                    description: "Retail display box".to_owned(),
                    refused_milli: refused * 1_000,
                    received_milli: refused * 1_000,
                    missing_milli: 0,
                    refusal_reason: Some(reason.to_owned()),
                }],
            })
            .await?;
        Ok(())
    }

    /// Two more deliveries of Al Noor's retail boxes that came back in part:
    /// QC passes the first (the lot is released) and fails the second (an NCR
    /// is raised on the lot), so the list of returned lots has a lot waiting,
    /// one passed and one failed.
    pub(crate) async fn decided_returns(&mut self) -> Result {
        let noor_lines = wl_sales::SaleLine::of(
            self.db(),
            &Subject::new("sales.sales_order", self.orders["noor"].0),
        )
        .await?;
        let lid_line = noor_lines
            .get(1)
            .or(noor_lines.first())
            .map(|l| l.id)
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the lines of Al Noor's order"))?;
        for (delivery, units, refused, reason, tracking, days_ago, verdict) in [
            (
                2,
                150,
                12,
                "printing out of register",
                "AWB-77188",
                9,
                "pass",
            ),
            (3, 100, 8, "boxes arrived wet", "AWB-77201", 6, "fail"),
        ] {
            let sent = self
                .dispatch("noor", "noor", &[("BOX-RTL", units, lid_line)])
                .await?;
            self.leave(&sent, tracking, days_ago).await?;
            self.came_back(&sent, delivery, units, refused, reason)
                .await?;
            let found = ReturnInspection::query()
                .where_(ReturnInspection::DISPATCH_ID.eq(sent.id))
                .first(self.db())
                .await?
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the inspection of {tracking}"))?;
            let (action, form): (&str, Vec<(&str, &str)>) = if verdict == "pass" {
                ("pass", vec![("note", "Reprinted labels checked, released")])
            } else {
                ("fail", vec![("defect", "Water damage to the board")])
            };
            self.driver
                .post(
                    QC_EMAIL,
                    &format!("/manufacturing/inspections/{}/{action}", found.id),
                    &form,
                )
                .await?;
        }
        Ok(())
    }
}
