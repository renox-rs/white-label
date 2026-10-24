//! The parts trade: sales orders of a workshop and a rental fleet with part
//! reserved and part back-ordered (one served when its goods receipt was
//! posted), the reorder suggestion, walk-in counter sales, the export sales
//! with and without exit evidence, and the recall of a brake pad batch traced
//! to the owner of a car, a fleet and the walk-in sales.

use auto::export_sale::BILL_OF_LADING;
use auto::{export_rule, export_sales, recalls};
use renox::prelude::*;
use wl_inventory::Recall;
use wl_kernel::Subject;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory, invoicer};
use wl_sales::sale_line::NewLine;
use wl_sales::sales_orders::{self, NewSalesOrder};

use super::super::demo_data::{at, day};
use super::{ADMIN_LOGIN, AISHA, HUSSEIN, OWNER_LOGIN, Seed};

/// Fils as dirhams for a form: `220.00`.
fn aed(fils: i64) -> String {
    format!("{}.{:02}", fils / 100, fils % 100)
}

impl Seed {
    /// A B2B order on `channel`, submitted: the lines (code, units, price).
    async fn sales_order(
        &self,
        party: &str,
        channel: &str,
        lines: &[(&str, i64, i64)],
    ) -> Result<i64> {
        let order = sales_orders::create(
            &self.state,
            None,
            NewSalesOrder::new(self.party(party)).channel(channel),
        )
        .await?;
        for (code, units, price) in lines {
            sales_orders::add_line(
                &self.state,
                order.id,
                NewLine::new(code, *price).quantity(*units),
            )
            .await?;
        }
        self.driver
            .post(
                ADMIN_LOGIN,
                &format!("/sales-orders/{}/submit", order.id),
                &[("x", "")],
            )
            .await?;
        Ok(order.id)
    }

    /// A walk-in counter sale of one line paid in cash.
    async fn counter_sale(&self, code: &str, price: i64) -> Result {
        self.driver
            .post(HUSSEIN, "/counter-sales", &[("x", "")])
            .await?;
        let sale = wl_sales::CounterSale::query()
            .order_by_desc(wl_sales::CounterSale::ID)
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no counter sale"))?;
        self.driver
            .post(
                HUSSEIN,
                &format!("/counter-sales/{}/lines", sale.id),
                &[
                    ("item_code", code),
                    ("unit_price", &aed(price)),
                    ("quantity", "1"),
                ],
            )
            .await?;
        let priced = wl_sales::CounterSale::find_or_404(self.db(), sale.id).await?;
        self.driver
            .post(
                HUSSEIN,
                &format!("/counter-sales/{}/complete", sale.id),
                &[("cash", &aed(priced.gross_total))],
            )
            .await?;
        Ok(())
    }

    /// An issued zero-rated invoice of AED 25,000 for the buyer in Mombasa,
    /// registered as an export sale supplied on `supply`.
    async fn export_sale(&self, n: i64, supply: i64, evidence: Option<&str>) -> Result {
        let books = invoicer(&self.state);
        let draft = InvoiceDraft::new(self.party("movers"), Subject::new("auto.export", n)).line(
            DraftLine::new(
                "Brake pads and filters, export to Mombasa",
                2_500_000,
                VatCategory::ZeroRated,
            ),
        );
        let id = books
            .draft(self.db(), draft)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: invoicing is off"))?;
        books.issue(&self.state, id, Some(self.who(AISHA))).await?;
        let sale = export_sales::record(&self.state, self.who(AISHA), id.0, day(supply)).await?;
        if let Some(reference) = evidence {
            export_sales::attach_evidence(
                &self.state,
                self.who(AISHA),
                sale.id,
                BILL_OF_LADING,
                reference,
            )
            .await?;
        }
        Ok(())
    }

    /// Back-orders, counter sales, export sales and the recall.
    pub(crate) async fn trade(&mut self) -> Result {
        // The workshop ordered five clutch kits when there were none: all
        // back-ordered; the goods receipt of day -20 reserves them.
        at!(self, day(-30), {
            self.sales_order("quick_fit", "phone", &[("CLUTCH-K", 5, 78_000)])
                .await
        })?;
        // Two orders for the timing kits, six on the shelf: ten asked, six
        // reserved, four back-ordered; the rental fleet's four wait behind.
        at!(self, day(-4), {
            self.sales_order(
                "quick_fit",
                "phone",
                &[("TIMING-KIT", 10, 26_000), ("OIL-F", 20, 4_500)],
            )
            .await
        })?;
        at!(self, day(-3), {
            self.sales_order("desert_rent", "sales_rep", &[("TIMING-KIT", 4, 26_000)])
                .await
        })?;
        at!(self, day(-1), {
            wl_inventory::reorder::run(&self.state).await
        })?;

        // Walk-in sales of brake pads at the Deira counter.
        at!(self, day(-45), {
            for _ in 0..4 {
                self.counter_sale("BRK-PAD", 14_000).await?;
            }
            self.counter_sale("OIL-F", 4_500).await
        })?;

        // Export sales: no evidence at 95 days (flagged at 90), none at 70
        // days (told at 60), and a bill of lading.
        at!(self, day(-95), { self.export_sale(1, -95, None).await })?;
        at!(self, day(-70), { self.export_sale(2, -70, None).await })?;
        at!(self, day(-40), {
            self.export_sale(3, -40, Some("BL-MSA-7788")).await
        })?;
        for on in [-35, -10, -5] {
            at!(self, day(on), {
                export_rule::run_daily(&self.state, day(on)).await
            })?;
        }

        // The recall of the brake pads of one supplier, received 110 to 125
        // days ago: the owner opens it and tells whom it can.
        let pads = self.items["BRK-PAD"].id.to_string();
        let supplier = self.party("brake_parts").0.to_string();
        let from = day(-125).to_string();
        let to = day(-110).to_string();
        at!(self, day(-2), {
            self.driver
                .post(
                    OWNER_LOGIN,
                    "/recalls",
                    &[
                        ("source", "supplier"),
                        ("reason", "Brake pads below the friction standard"),
                        ("supplier", supplier.as_str()),
                        ("items", pads.as_str()),
                        ("received_from", from.as_str()),
                        ("received_to", to.as_str()),
                    ],
                )
                .await?;
            Ok::<(), renox::Error>(())
        })?;
        let recall = Recall::query()
            .order_by_desc(Recall::ID)
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no recall"))?;
        at!(self, day(-1), {
            recalls::send(&self.state, self.who(OWNER_LOGIN), recall.id).await
        })?;
        Ok(())
    }
}
