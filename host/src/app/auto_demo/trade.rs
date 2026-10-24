//! The parts trade beside the workshop: walk-in sales at the Deira counter,
//! a B2B order of Quick Fix Garage on its credit account and an invoice of
//! theirs overdue, a zero-rated export to Afri Motors with its exit
//! evidence pending, and the supplier's recall of the Toyota brake pads,
//! traced to the Land Cruiser that had a set fitted.

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory, invoicer};

use super::{ACCOUNTANT_EMAIL, ADMIN_EMAIL, COUNTER_EMAIL, OWNER_EMAIL, Seed, day};

impl Seed {
    /// A walk-in sale at the counter of `lines` (code, quantity, unit price
    /// in AED), paid by `method`; each line reads as the part's name.
    async fn counter_sale(&self, lines: &[(&str, &str, &str)], method: &str) -> Result {
        let opened = self
            .driver
            .post(COUNTER_EMAIL, "/counter-sales", &[("x", "")])
            .await?;
        let id = opened
            .redirected_id()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the counter sale"))?;
        for (code, quantity, price) in lines {
            let name = wl_inventory::Item::query()
                .where_(wl_inventory::Item::CODE.eq(*code))
                .first(self.db())
                .await?
                .map(|item| item.name_en)
                .unwrap_or_else(|| (*code).to_owned());
            self.driver
                .post(
                    COUNTER_EMAIL,
                    &format!("/counter-sales/{id}/lines"),
                    &[
                        ("item_code", code),
                        ("description", &name),
                        ("quantity", quantity),
                        ("unit_price", price),
                        ("vat", "standard"),
                    ],
                )
                .await?;
        }
        let sale = wl_sales::CounterSale::find_or_404(self.db(), id).await?;
        let total = format!("{}.{:02}", sale.gross_total / 100, sale.gross_total % 100);
        self.driver
            .post(
                COUNTER_EMAIL,
                &format!("/counter-sales/{id}/complete"),
                &[(method, &total)],
            )
            .await?;
        Ok(())
    }

    /// An invoice of `amount` fils to the party `key`, issued by the
    /// accountant; its id.
    async fn invoice(
        &self,
        key: &str,
        what: &str,
        amount: i64,
        vat: VatCategory,
        due: i64,
        source: Subject,
    ) -> Result<i64> {
        let draft = InvoiceDraft::new(self.parties[key], source.clone())
            .line(DraftLine::new(what, amount, vat).origin(source))
            .due_date(day(due));
        let id = invoicer(&self.state)
            .draft(self.db(), draft)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("invoicing is off"))?;
        self.driver
            .post(ACCOUNTANT_EMAIL, &format!("/invoices/{}/issue", id.0), &[])
            .await?;
        Ok(id.0)
    }

    /// The trade.
    pub(crate) async fn trade(&mut self) -> Result {
        self.counter_sale(
            &[("OIL-5W30-4L", "2", "95"), ("FLT-OIL-TY", "1", "32")],
            "cash",
        )
        .await?;
        self.counter_sale(&[("BAT-70AH", "1", "445")], "card")
            .await?;
        self.counter_sale(
            &[("WPR-BLD-24", "2", "35"), ("SPK-PLG-IR", "4", "58")],
            "card",
        )
        .await?;

        // Quick Fix Garage buys on account; last month's invoice is overdue.
        self.driver
            .post(
                ADMIN_EMAIL,
                "/credit/accounts",
                &[
                    ("party_id", &self.parties["quickfix"].0.to_string()),
                    ("credit_limit", "25000"),
                    ("payment_terms_days", "30"),
                ],
            )
            .await?;
        self.invoice(
            "quickfix",
            "Brake parts and filters, September",
            2_874_000,
            VatCategory::Standard,
            -18,
            Subject::new("auto_demo.opening", 1),
        )
        .await?;
        wl_invoicing::overdue::run(&self.state).await?;

        // A zero-rated export of batteries and brake discs to Afri Motors;
        // the exit certificate is not in yet.
        let export = self
            .invoice(
                "afriparts",
                "20 batteries 70 Ah and 12 brake discs, shipped to Lagos",
                1_068_000,
                VatCategory::ZeroRated,
                30,
                Subject::new("auto_demo.opening", 2),
            )
            .await?;
        self.driver
            .post(
                ACCOUNTANT_EMAIL,
                "/auto/export-sales",
                &[
                    ("invoice", &export.to_string()),
                    ("supply_date", &day(-12).to_string()),
                ],
            )
            .await?;

        // The supplier recalls the Toyota brake pads.
        let item = wl_inventory::Item::query()
            .where_(wl_inventory::Item::CODE.eq("BRK-PAD-TY"))
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the brake pads"))?;
        self.driver
            .post(
                OWNER_EMAIL,
                "/recalls",
                &[
                    ("source", "supplier"),
                    (
                        "reason",
                        "The maker found a bonding fault in the friction material of this pad set.",
                    ),
                    ("items", &item.id.to_string()),
                    ("received_from", &day(-30).to_string()),
                    ("received_to", &day(0).to_string()),
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
}
