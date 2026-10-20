//! The KPI pack: a client's periodic management report, calculated from an
//! imported trial balance (04 §6 *KPI pack*, §7.1 *KPI pack*, `ACC.KPI.01`;
//! BL-ACC-20, BL-ACC-21).
//!
//! A pack belongs to a client and a period (the last day of the month), in
//! versions 1, 2, 3… Its figures are the trial balance accounts
//! ([`KpiFigure`], a signed amount in fils: debit minus credit) each mapped to
//! a KPI category ([`CATEGORIES`]); the mapping is saved per client
//! ([`KpiMapping`]) and reused every period. The KPIs themselves are
//! calculated from the figures when shown ([`kpis`]), so they cannot drift
//! from them. The status moves only through the machine in
//! [`crate::kpi_machine`]; the services that run it are in
//! [`crate::kpi_packs`].
//!
//! Decision: the live ledger connectors and the presets for Zoho Books, Xero,
//! QuickBooks and Tally (`ACC.KPI.02`, Later) are not built; the import reads
//! pasted CSV in the generic layout `code, name, debit, credit` (or `code,
//! name, balance`).

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::{PartyId, Subject};

/// The subject type of a KPI pack.
pub const KPI_PACK_SUBJECT_TYPE: &str = "accounting.kpi_pack";

/// The KPI categories an account can be mapped to.
pub const CATEGORIES: &[&str] = &[
    "revenue",
    "cost_of_sales",
    "operating_expenses",
    "other_income",
    "other_expenses",
    "assets",
    "liabilities",
    "equity",
];

/// Where a KPI pack stands (§7.1 *KPI pack*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum KpiStatus {
    /// Figures imported; KPIs calculated; commentary in progress.
    #[default]
    Draft,
    /// Visible in the portal.
    Published,
    /// Replaced by a newer version of the same period (terminal).
    Superseded,
    /// Removed from the portal: wrong client or wrong figures (terminal).
    Withdrawn,
}

/// A client's KPI pack for a period, in one version.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "accounting_kpi_packs",
    state_machine,
    unique(client_party_id, period_end, version),
    index(client_party_id),
    index(status)
)]
pub struct KpiPack {
    /// The primary key.
    pub id: i64,
    /// The client company, a party.
    pub client_party_id: i64,
    /// The last day of the period it covers.
    pub period_end: NaiveDate,
    /// The version: 1, then one more for each new draft of the period.
    pub version: i64,
    /// The manager's commentary.
    pub commentary: Option<String>,
    /// Whether the template requires commentary before it is published.
    pub commentary_required: bool,
    /// The user who imported the figures (last import).
    pub imported_by: Option<i64>,
    /// When the figures were last imported.
    pub imported_at: Option<DateTime>,
    /// The withdrawn pack this one was copied from.
    pub copied_from_id: Option<i64>,
    /// The day it was published (Asia/Dubai).
    pub published_on: Option<NaiveDate>,
    /// The user who published it.
    pub published_by: Option<i64>,
    /// Why it was withdrawn ([`crate::kpi_machine::WITHDRAW_REASONS`]).
    pub withdraw_reason: Option<String>,
    /// The day it was withdrawn (Asia/Dubai).
    pub withdrawn_on: Option<NaiveDate>,
    /// The user who withdrew it.
    pub withdrawn_by: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: KpiStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl KpiPack {
    /// The pack as a kernel subject: `accounting.kpi_pack:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(KPI_PACK_SUBJECT_TYPE, self.id)
    }

    /// The client company.
    pub fn client(&self) -> PartyId {
        PartyId(self.client_party_id)
    }

    /// The pack shown to people: `KPI-0007`.
    pub fn number(&self) -> String {
        format!("KPI-{:04}", self.id)
    }

    /// The packs of `client` for `period_end`, oldest version first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_period(
        db: &Db,
        client: PartyId,
        period_end: NaiveDate,
    ) -> Result<Vec<KpiPack>> {
        KpiPack::query()
            .where_(KpiPack::CLIENT_PARTY_ID.eq(client.0))
            .where_(KpiPack::PERIOD_END.eq(period_end))
            .order_by(KpiPack::VERSION)
            .get(db)
            .await
    }

    /// The packs of `client`, newest period and version first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_client(db: &Db, client: PartyId) -> Result<Vec<KpiPack>> {
        let mut packs = KpiPack::query()
            .where_(KpiPack::CLIENT_PARTY_ID.eq(client.0))
            .get(db)
            .await?;
        packs.sort_by_key(|p| std::cmp::Reverse((p.period_end, p.version)));
        Ok(packs)
    }

    /// The figures of the pack, by account code.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn figures(&self, db: &Db) -> Result<Vec<KpiFigure>> {
        KpiFigure::query()
            .where_(KpiFigure::PACK_ID.eq(self.id))
            .order_by(KpiFigure::ACCOUNT_CODE)
            .get(db)
            .await
    }

    /// Whether a newer version of the same period is a draft: the published
    /// pack is *being revised* (`ACC.RUL.06`).
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn is_being_revised(&self, db: &Db) -> Result<bool> {
        Ok(self.status == KpiStatus::Published
            && KpiPack::of_period(db, self.client(), self.period_end)
                .await?
                .iter()
                .any(|p| p.status == KpiStatus::Draft && p.version > self.version))
    }
}

/// One account of the imported trial balance.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "accounting_kpi_figures", index(pack_id))]
pub struct KpiFigure {
    /// The primary key.
    pub id: i64,
    /// The pack.
    #[model(references = "accounting_kpi_packs")]
    pub pack_id: i64,
    /// The account's code in the client's ledger.
    pub account_code: String,
    /// The account's name.
    pub account_name: String,
    /// Debit minus credit, in fils.
    pub amount: i64,
    /// The KPI category it is mapped to; none while the account is unmapped.
    pub category: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A client's saved mapping of a ledger account to a KPI category.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "accounting_kpi_mappings",
    unique(client_party_id, account_code)
)]
pub struct KpiMapping {
    /// The primary key.
    pub id: i64,
    /// The client company, a party.
    pub client_party_id: i64,
    /// The account's code.
    pub account_code: String,
    /// The KPI category.
    pub category: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl KpiMapping {
    /// The saved mapping of `client`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_client(db: &Db, client: PartyId) -> Result<Vec<KpiMapping>> {
        KpiMapping::query()
            .where_(KpiMapping::CLIENT_PARTY_ID.eq(client.0))
            .order_by(KpiMapping::ACCOUNT_CODE)
            .get(db)
            .await
    }
}

/// How a KPI is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum KpiKind {
    /// An amount in fils.
    Money,
    /// A share in basis points (1 250 = 12.50%).
    Share,
}

/// One calculated KPI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Kpi {
    /// The KPI's key (`revenue`, `gross_profit`…).
    pub key: &'static str,
    /// Its value: fils, or basis points for a share.
    pub value: i64,
    /// How to show it.
    pub kind: KpiKind,
}

/// The sum of the figures mapped to `category`, debit minus credit.
fn net(figures: &[KpiFigure], category: &str) -> i64 {
    figures
        .iter()
        .filter(|f| f.category.as_deref() == Some(category))
        .map(|f| f.amount)
        .sum()
}

/// `part` as a share of `whole` in basis points, rounded to the nearest; 0
/// without a whole.
fn share(part: i64, whole: i64) -> i64 {
    if whole == 0 {
        return 0;
    }
    let scaled = i128::from(part) * 10_000;
    let whole = i128::from(whole);
    let half = whole.abs() / 2;
    let rounded = if scaled >= 0 {
        (scaled + half) / whole
    } else {
        (scaled - half) / whole
    };
    i64::try_from(rounded).unwrap_or(0)
}

/// The KPIs of `figures` (revenue and costs are credit and debit balances:
/// revenue is shown positive).
pub fn kpis(figures: &[KpiFigure]) -> Vec<Kpi> {
    let revenue = -net(figures, "revenue");
    let cost_of_sales = net(figures, "cost_of_sales");
    let operating_expenses = net(figures, "operating_expenses");
    let other_income = -net(figures, "other_income");
    let other_expenses = net(figures, "other_expenses");
    let gross_profit = revenue - cost_of_sales;
    let operating_profit = gross_profit - operating_expenses;
    let net_profit = operating_profit + other_income - other_expenses;
    let money = |key, value| Kpi {
        key,
        value,
        kind: KpiKind::Money,
    };
    let ratio = |key, value| Kpi {
        key,
        value,
        kind: KpiKind::Share,
    };
    vec![
        money("revenue", revenue),
        money("gross_profit", gross_profit),
        ratio("gross_margin", share(gross_profit, revenue)),
        money("operating_profit", operating_profit),
        money("net_profit", net_profit),
        ratio("net_margin", share(net_profit, revenue)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn figure(code: &str, amount: i64, category: Option<&str>) -> KpiFigure {
        KpiFigure {
            account_code: code.to_owned(),
            amount,
            category: category.map(str::to_owned),
            ..KpiFigure::default()
        }
    }

    #[test]
    fn the_kpis_follow_the_mapped_figures() {
        let figures = [
            figure("4000", -1_000_000, Some("revenue")),
            figure("5000", 400_000, Some("cost_of_sales")),
            figure("6000", 300_000, Some("operating_expenses")),
            figure("7000", -20_000, Some("other_income")),
            figure("8000", 10_000, Some("other_expenses")),
            figure("1000", 5_000_000, Some("assets")),
        ];
        let kpis = kpis(&figures);
        let value = |key: &str| kpis.iter().find(|k| k.key == key).unwrap().value;
        assert_eq!(value("revenue"), 1_000_000);
        assert_eq!(value("gross_profit"), 600_000);
        assert_eq!(value("gross_margin"), 6_000);
        assert_eq!(value("operating_profit"), 300_000);
        assert_eq!(value("net_profit"), 310_000);
        assert_eq!(value("net_margin"), 3_100);
    }

    #[test]
    fn a_share_without_revenue_is_zero() {
        assert_eq!(share(100, 0), 0);
        assert_eq!(share(1, 3), 3_333);
    }

    #[test]
    fn the_number_pads_the_id() {
        let pack = KpiPack {
            id: 7,
            ..KpiPack::default()
        };
        assert_eq!(pack.number(), "KPI-0007");
    }
}
