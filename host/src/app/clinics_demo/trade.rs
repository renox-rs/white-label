//! The sales side of the demo clinic: patients enrolled in the rewards
//! programme, counter sales at the front desk (one with a member's points
//! earned, one returned), a manual adjustment of points, and the campaigns
//! of the marketing manager (one active under its advertising permit, one
//! still a draft for the lapsed patients).
//!
//! Everything goes through the sales pages as the receptionist, the clinic
//! manager, the marketing manager and the owner; the programme and the
//! lapsed-patients segment are the ones the clinic installs.

use renox::prelude::*;
use wl_kernel::Subject;
use wl_sales::{CounterSale, LoyaltyAccount, SaleLine, SalesReturn};

use super::super::demo_data::day;
use super::Seed;
use super::people::{AHMED, DANIEL, ELENA, FATIMA, PRIYA, SOFIA};

const DESK: &str = "reception.jumeirah@palmgrove.example";
const MANAGER: &str = "manager.jumeirah@palmgrove.example";
const MARKETING: &str = "marketing@palmgrove.example";
const ADMIN: &str = "admin@palmgrove.example";
const OWNER: &str = "owner@palmgrove.example";

/// Fils as dirhams for a form: `220.00`.
pub(crate) fn aed(fils: i64) -> String {
    format!("{}.{:02}", fils / 100, fils % 100)
}

impl Seed {
    async fn latest_sale(&self) -> Result<CounterSale> {
        CounterSale::query()
            .order_by_desc(CounterSale::ID)
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: no counter sale").into())
    }

    /// Rings up a counter sale of one line paid in cash; the sale.
    async fn counter_sale(
        &self,
        item: &str,
        price: i64,
        member: Option<&str>,
    ) -> Result<CounterSale> {
        self.driver
            .post(DESK, "/counter-sales", &[("x", "")])
            .await?;
        let sale = self.latest_sale().await?;
        self.driver
            .post(
                DESK,
                &format!("/counter-sales/{}/lines", sale.id),
                &[
                    ("item_code", item),
                    ("unit_price", &aed(price)),
                    ("quantity", "1"),
                ],
            )
            .await?;
        if let Some(code) = member {
            self.driver
                .post(
                    DESK,
                    &format!("/counter-sales/{}/member", sale.id),
                    &[("code", code)],
                )
                .await?;
        }
        let priced = CounterSale::find_or_404(self.db(), sale.id).await?;
        self.driver
            .post(
                DESK,
                &format!("/counter-sales/{}/complete", sale.id),
                &[("cash", &aed(priced.gross_total))],
            )
            .await?;
        CounterSale::find_or_404(self.db(), sale.id).await
    }

    async fn account_of(&self, patient: usize) -> Result<LoyaltyAccount> {
        LoyaltyAccount::query()
            .where_(LoyaltyAccount::PARTY_ID.eq(self.patients[patient].0))
            .first(self.db())
            .await?
            .ok_or_else(|| {
                renox::anyhow::anyhow!("demo seed: patient {patient} is no member").into()
            })
    }

    pub(crate) async fn trade(&mut self) -> Result {
        // Five patients are enrolled at the desk with their mobile.
        for (patient, mobile) in [
            (AHMED, "0501000001"),
            (FATIMA, "0501000002"),
            (ELENA, "0501000007"),
            (SOFIA, "0501000009"),
            (PRIYA, "0501000003"),
        ] {
            self.driver
                .post(
                    DESK,
                    "/loyalty/accounts",
                    &[
                        ("party_id", &self.patients[patient].0.to_string()),
                        ("mobile", mobile),
                        ("terms", "1"),
                    ],
                )
                .await?;
        }
        // Counter sales: two members earn points, a walk-in does not; a
        // third sale is returned.
        let elena = self.account_of(ELENA).await?;
        let sofia = self.account_of(SOFIA).await?;
        self.counter_sale("Skin care serum", 22_000, Some(&elena.code))
            .await?;
        let band = self
            .counter_sale("Resistance band set", 6_000, Some(&sofia.code))
            .await?;
        self.counter_sale("Face masks (pack of 10)", 2_500, None)
            .await?;
        let returned = self.counter_sale("Compression sleeve", 9_500, None).await?;
        let line = SaleLine::of(self.db(), &Subject::new("sales.counter_sale", returned.id))
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the sale has no line"))?;
        self.driver
            .post(
                ADMIN,
                "/sales-returns",
                &[
                    ("sale_type", "counter_sale"),
                    ("sale_id", &returned.id.to_string()),
                    ("kind", "goods"),
                    ("reason", "Wrong size"),
                    ("lines", &format!("{}:1", line.id)),
                ],
            )
            .await?;
        let request = SalesReturn::query()
            .order_by_desc(SalesReturn::ID)
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the return was not requested"))?;
        self.driver
            .post(
                ADMIN,
                &format!("/sales-returns/{}/receive", request.id),
                &[("approved", "1")],
            )
            .await?;
        let _ = band;
        // Goodwill points for a patient who waited: below 500, so at once.
        let ahmed = self.account_of(AHMED).await?;
        self.driver
            .post(
                MANAGER,
                &format!("/loyalty/accounts/{}/adjust", ahmed.id),
                &[("points", "150"), ("reason", "Goodwill after a long wait")],
            )
            .await?;
        self.campaigns().await
    }

    /// An acquisition campaign under its permit, and a draft for the
    /// patients who have not been for six months.
    async fn campaigns(&self) -> Result {
        let owner = self.user(MARKETING).id.to_string();
        let dates = [
            ("start_date", day(-10).to_string()),
            ("end_date", day(20).to_string()),
        ];
        let mut form: Vec<(&str, String)> = vec![
            ("name", "Smile month".into()),
            ("code", "SMILE-OCT".into()),
            ("purpose", "acquisition".into()),
            ("marketing_channel", "meta".into()),
            ("budget", "5000".into()),
            ("owner_id", owner.clone()),
        ];
        form.extend(dates.iter().map(|(k, v)| (*k, v.clone())));
        let pairs: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
        self.driver.post(MARKETING, "/campaigns", &pairs).await?;
        let smile = wl_sales::Campaign::query()
            .order_by_desc(wl_sales::Campaign::ID)
            .first(self.db())
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the campaign was not saved"))?;
        // The medical director records the permit, then the campaign starts.
        self.driver
            .post(
                OWNER,
                "/clinics/campaign-permits",
                &[
                    ("campaign", &smile.id.to_string()),
                    ("promotes", "yes"),
                    ("permit_number", "DHA-ADV-2026-0417"),
                    ("valid_to", &day(120).to_string()),
                ],
            )
            .await?;
        self.driver
            .post(
                MARKETING,
                &format!("/campaigns/{}/transitions/activate", smile.id),
                &[],
            )
            .await?;
        // A retention campaign stays a draft: its message is not written.
        self.driver
            .post(
                MARKETING,
                "/campaigns",
                &[
                    ("name", "We miss you"),
                    ("code", "MISS-YOU"),
                    ("purpose", "retention"),
                    ("marketing_channel", "email"),
                    ("owner_id", &owner),
                    ("start_date", &day(7).to_string()),
                    ("end_date", &day(30).to_string()),
                ],
            )
            .await?;
        let _ = DANIEL;
        Ok(())
    }
}
