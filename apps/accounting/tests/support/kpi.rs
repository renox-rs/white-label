//! What the KPI pack tests share: a small trial balance, the routes that
//! import, map, comment and publish, and the packs as stored.

use accounting::{KpiPack, KpiStatus};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::TestResponse;
use wl_kernel::PartyId;

use super::{Booted, d};

/// October's trial balance: sales 10,000, cost of sales 4,000, rent 1,000,
/// bank 5,000.
pub const TB_OCTOBER: &str = "code,name,debit,credit
1000,Bank,5000.00,
4000,Sales,,10000.00
5000,Cost of sales,4000.00,
6000,Rent,1000.00
";

/// The same month re-exported after late entries: sales are 12,000 and the
/// bank 7,000.
pub const TB_OCTOBER_LATE: &str = "code,name,debit,credit
1000,Bank,7000.00,
4000,Sales,,12000.00
5000,Cost of sales,4000.00,
6000,Rent,1000.00
";

/// [`TB_OCTOBER_LATE`] with an account the client never mapped.
pub const TB_OCTOBER_NEW_ACCOUNT: &str = "code,name,debit,credit
1000,Bank,7000.00,
4000,Sales,,12000.00
5000,Cost of sales,4000.00,
6000,Rent,1000.00
7000,Bank charges,0.00,
";

/// October 2026, as the form types it.
pub const OCTOBER: &str = "2026-10";

/// The last day of October 2026.
pub fn october() -> NaiveDate {
    d(10, 31)
}

impl Booted {
    /// `user` imports `csv` for `client` and `period` through the page.
    pub async fn kpi_import(
        &self,
        user: &User,
        client: PartyId,
        period: &str,
        csv: &str,
    ) -> TestResponse {
        self.post(
            user,
            "/accounting/kpi-packs/import",
            &[
                ("client", &client.0.to_string()),
                ("period", period),
                ("csv", csv),
            ],
        )
        .await
    }

    /// `user` posts `action` of pack `pack` with `pairs`.
    pub async fn kpi_act(
        &self,
        user: &User,
        pack: &KpiPack,
        action: &str,
        pairs: &[(&str, &str)],
    ) -> TestResponse {
        self.post(
            user,
            &format!("/accounting/kpi-packs/{}/{action}", pack.id),
            pairs,
        )
        .await
    }

    /// Every pack of `client` for October 2026, oldest version first.
    pub async fn kpi_packs(&self, client: PartyId) -> Vec<KpiPack> {
        KpiPack::of_period(self.db(), client, october())
            .await
            .unwrap()
    }

    /// The pack `pack`, as stored now.
    pub async fn kpi_now(&self, pack: &KpiPack) -> KpiPack {
        KpiPack::find(self.db(), pack.id).await.unwrap().unwrap()
    }

    /// The pack of `client` and October in `version`.
    pub async fn kpi_version(&self, client: PartyId, version: i64) -> KpiPack {
        self.kpi_packs(client)
            .await
            .into_iter()
            .find(|p| p.version == version)
            .expect("that version exists")
    }

    /// `user` maps the accounts of [`TB_OCTOBER`] to their categories.
    pub async fn kpi_map_all(&self, user: &User, pack: &KpiPack) {
        for (code, category) in [
            ("1000", "assets"),
            ("4000", "revenue"),
            ("5000", "cost_of_sales"),
            ("6000", "operating_expenses"),
        ] {
            self.kpi_act(user, pack, "map", &[("code", code), ("category", category)])
                .await
                .assert_ok();
        }
    }

    /// A published October pack for `client`: imported, mapped, commented,
    /// published by `user` (a manager). Answers it as stored.
    pub async fn kpi_published(&self, user: &User, client: PartyId, csv: &str) -> KpiPack {
        self.kpi_import(user, client, OCTOBER, csv)
            .await
            .assert_ok();
        let draft = self
            .kpi_packs(client)
            .await
            .into_iter()
            .find(|p| p.status == KpiStatus::Draft)
            .expect("the import made a draft");
        self.kpi_map_all(user, &draft).await;
        self.kpi_act(
            user,
            &draft,
            "commentary",
            &[("commentary", "Sales up on last month.")],
        )
        .await
        .assert_ok();
        self.kpi_act(user, &draft, "publish", &[]).await.assert_ok();
        self.kpi_now(&draft).await
    }
}
