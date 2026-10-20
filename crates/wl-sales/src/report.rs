//! The acquisition report (wl-sales §5.8 step 5, issue #140): per source,
//! campaign, marketing channel, brand, branch and month, the leads, the leads
//! won, the conversion rate, the new customers, the cost, the cost per lead,
//! the customer acquisition cost (CAC) and the return on ad spend (ROAS).
//! Everything is computed on read from the leads, the party acquisitions and
//! the approved cost entries; nothing is stored.
//!
//! - **Periods** are calendar months in Asia/Dubai: a cost by its entry date,
//!   a lead by the day it was created, a lead won by the day it was won, a
//!   new customer by the date of its first sale. So a campaign's cost and its
//!   customers can fall in different months.
//! - **Leads** closed as *duplicate* or *spam or invalid* are left out of the
//!   leads, the conversion and the cost per lead.
//! - **Cost** is the `approved` cost entries only, split into *linked to
//!   bills* and *manual*. A group without a campaign has no cost: its cost,
//!   cost per lead, CAC and ROAS are undefined ("—").
//! - **Converted value** is the first sale excluding VAT net of credit notes,
//!   or with the attribution window the sales of the first N months
//!   ([`crate::acquisition::converted_value`]), recomputed here from the
//!   reported sales, so a changed setting shows at once.
//! - **A ratio with a zero divisor is undefined** ("—"), never 0.
//! - **Scope** ([`Viewer`]): a user sees the leads and customers of their
//!   branches. A campaign with a branch belongs to it. A campaign without
//!   one is entity-wide: its cost is shown only to a user who sees every
//!   branch; for a user scoped to some branches the cost, cost per lead, CAC
//!   and ROAS of the groups it feeds are "—" ([`Row::cost_hidden`]).
//! - **Rounding:** money to 2 decimals, rates to 1 decimal, ROAS to 2
//!   decimals, half up.

use std::collections::{BTreeMap, BTreeSet};

use renox::chrono::{Datelike, NaiveDate};
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::documents::dubai_date;

use crate::acquisition::{AcquisitionSale, PartyAcquisition, converted_value};
use crate::campaign::Campaign;
use crate::config::SalesConfig;
use crate::cost_entry::{CostEntry, CostStatus};
use crate::lead::{Lead, LeadStatus};
use crate::lead_source::LeadSource;
use crate::notices::aed;

/// What the report is grouped by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum GroupBy {
    /// The lead source.
    #[default]
    Source,
    /// The campaign.
    Campaign,
    /// The marketing channel of the campaign.
    Channel,
    /// The brand of the campaign.
    Brand,
    /// The branch.
    Branch,
    /// The calendar month.
    Month,
}

impl GroupBy {
    /// Every grouping, in the order the page offers them.
    pub const ALL: [GroupBy; 6] = [
        GroupBy::Source,
        GroupBy::Campaign,
        GroupBy::Channel,
        GroupBy::Brand,
        GroupBy::Branch,
        GroupBy::Month,
    ];

    /// The name in a URL.
    pub fn name(self) -> &'static str {
        match self {
            GroupBy::Source => "source",
            GroupBy::Campaign => "campaign",
            GroupBy::Channel => "channel",
            GroupBy::Brand => "brand",
            GroupBy::Branch => "branch",
            GroupBy::Month => "month",
        }
    }

    /// The grouping named `name`, if there is one.
    pub fn parse(name: &str) -> Option<GroupBy> {
        GroupBy::ALL.into_iter().find(|g| g.name() == name.trim())
    }
}

/// What a fact is about: the attributes the filters and groupings read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attrs {
    /// The lead source.
    pub source: Option<i64>,
    /// The campaign.
    pub campaign: Option<i64>,
    /// The marketing channel of the campaign (stored name).
    pub channel: Option<String>,
    /// The brand of the campaign.
    pub brand: Option<String>,
    /// The branch.
    pub branch: Option<Subject>,
    /// The legal entity.
    pub entity: Option<i64>,
}

/// A lead.
#[derive(Debug, Clone)]
pub struct LeadFact {
    /// What it is about.
    pub attrs: Attrs,
    /// The day it was created.
    pub created: NaiveDate,
    /// The day it was won, if it is won.
    pub won_on: Option<NaiveDate>,
    /// Whether it counts (not closed as duplicate or spam).
    pub counts: bool,
}

/// A new customer: an acquisition with a first sale.
#[derive(Debug, Clone)]
pub struct CustomerFact {
    /// What it is about.
    pub attrs: Attrs,
    /// The date of the first sale.
    pub date: NaiveDate,
    /// The converted value, AED fils excluding VAT.
    pub value: i64,
}

/// An approved cost entry.
#[derive(Debug, Clone)]
pub struct CostFact {
    /// The campaign's attributes.
    pub attrs: Attrs,
    /// The entry date.
    pub date: NaiveDate,
    /// AED fils.
    pub amount: i64,
    /// Linked to a supplier bill line.
    pub linked: bool,
}

/// Everything the report is computed from.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    /// The leads.
    pub leads: Vec<LeadFact>,
    /// The new customers.
    pub customers: Vec<CustomerFact>,
    /// The approved cost entries.
    pub costs: Vec<CostFact>,
}

impl Facts {
    /// The distinct branches the facts mention.
    pub fn branches(&self) -> Vec<Subject> {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut branches = Vec::new();
        let all = self
            .leads
            .iter()
            .map(|f| &f.attrs)
            .chain(self.customers.iter().map(|f| &f.attrs))
            .chain(self.costs.iter().map(|f| &f.attrs));
        for attrs in all {
            if let Some(branch) = &attrs.branch
                && seen.insert(branch.to_string())
            {
                branches.push(branch.clone());
            }
        }
        branches
    }
}

/// Which branches a user sees.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Viewer {
    /// Every branch (a global role), including records with no branch.
    pub all: bool,
    /// The branches a scoped user sees.
    pub branches: Vec<Subject>,
}

impl Viewer {
    /// A user who sees every branch.
    pub fn all() -> Self {
        Viewer {
            all: true,
            branches: Vec::new(),
        }
    }

    /// A user scoped to `branches`.
    pub fn scoped(branches: Vec<Subject>) -> Self {
        Viewer {
            all: false,
            branches,
        }
    }

    /// Whether the viewer sees something at `branch`.
    pub fn sees(&self, branch: Option<&Subject>) -> bool {
        self.all || branch.is_some_and(|b| self.branches.contains(b))
    }

    /// Whether the viewer sees nothing at all.
    pub fn is_blind(&self) -> bool {
        !self.all && self.branches.is_empty()
    }
}

/// The filters of the report; an unset one keeps everything.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    /// The first day, inclusive.
    pub from: Option<NaiveDate>,
    /// The last day, inclusive.
    pub to: Option<NaiveDate>,
    /// The lead source.
    pub source: Option<i64>,
    /// The campaign.
    pub campaign: Option<i64>,
    /// The marketing channel (stored name).
    pub channel: Option<String>,
    /// The brand.
    pub brand: Option<String>,
    /// The branch.
    pub branch: Option<Subject>,
    /// The legal entity.
    pub entity: Option<i64>,
}

impl Filter {
    /// The calendar month `year`-`month` as the period.
    pub fn month(mut self, year: i32, month: u32) -> Self {
        self.from = NaiveDate::from_ymd_opt(year, month, 1);
        self.to = self
            .from
            .and_then(|first| first.checked_add_months(renox::chrono::Months::new(1)))
            .and_then(|next| next.pred_opt());
        self
    }

    fn in_period(&self, date: NaiveDate) -> bool {
        self.from.is_none_or(|from| date >= from) && self.to.is_none_or(|to| date <= to)
    }

    fn matches(&self, attrs: &Attrs) -> bool {
        self.source.is_none_or(|s| attrs.source == Some(s))
            && self.campaign.is_none_or(|c| attrs.campaign == Some(c))
            && self
                .channel
                .as_ref()
                .is_none_or(|c| attrs.channel.as_ref() == Some(c))
            && self
                .brand
                .as_ref()
                .is_none_or(|b| attrs.brand.as_ref() == Some(b))
            && self
                .branch
                .as_ref()
                .is_none_or(|b| attrs.branch.as_ref() == Some(b))
            && self.entity.is_none_or(|e| attrs.entity == Some(e))
    }
}

/// The key of a group: what it is grouped by and the value, `None` for
/// "none" (no campaign, no brand, ...).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroupKey {
    /// The grouping.
    pub by: GroupBy,
    /// The value: an id, a stored name, `type:id` or `YYYY-MM`.
    pub value: Option<String>,
}

fn key_of(by: GroupBy, attrs: &Attrs, date: NaiveDate) -> GroupKey {
    let value = match by {
        GroupBy::Source => attrs.source.map(|s| s.to_string()),
        GroupBy::Campaign => attrs.campaign.map(|c| c.to_string()),
        GroupBy::Channel => attrs.channel.clone(),
        GroupBy::Brand => attrs.brand.clone(),
        GroupBy::Branch => attrs.branch.as_ref().map(Subject::to_string),
        GroupBy::Month => Some(format!("{:04}-{:02}", date.year(), date.month())),
    };
    GroupKey { by, value }
}

/// One group of the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The group.
    pub key: GroupKey,
    /// Leads created in the period, without duplicates and spam.
    pub leads: i64,
    /// Leads won in the period.
    pub won: i64,
    /// New customers: first sales in the period.
    pub customers: i64,
    /// Their converted value, AED fils excluding VAT.
    pub value: i64,
    /// The approved cost, AED fils; `None` when no cost entry feeds the group.
    pub cost: Option<i64>,
    /// Of it, linked to supplier bills.
    pub cost_linked: i64,
    /// Of it, entered by hand.
    pub cost_manual: i64,
    /// Entity-wide cost the viewer may not see feeds the group.
    pub cost_hidden: bool,
}

/// Half-up division of non-negative `num` by positive `den`.
fn div_half_up(num: i128, den: i128) -> i64 {
    i64::try_from((num * 2 + den) / (den * 2)).unwrap_or(i64::MAX)
}

impl Row {
    fn new(key: GroupKey) -> Self {
        Row {
            key,
            leads: 0,
            won: 0,
            customers: 0,
            value: 0,
            cost: None,
            cost_linked: 0,
            cost_manual: 0,
            cost_hidden: false,
        }
    }

    /// The cost, unless there is none or it is hidden.
    fn known_cost(&self) -> Option<i64> {
        if self.cost_hidden { None } else { self.cost }
    }

    /// Leads won ÷ leads, in tenths of a percent (20.0% is 200).
    pub fn conversion_tenths(&self) -> Option<i64> {
        (self.leads > 0).then(|| div_half_up(i128::from(self.won) * 1000, i128::from(self.leads)))
    }

    /// Cost ÷ leads, in fils.
    pub fn cost_per_lead(&self) -> Option<i64> {
        let cost = self.known_cost()?;
        (self.leads > 0).then(|| div_half_up(i128::from(cost), i128::from(self.leads)))
    }

    /// Cost ÷ new customers, in fils.
    pub fn cac(&self) -> Option<i64> {
        let cost = self.known_cost()?;
        (self.customers > 0).then(|| div_half_up(i128::from(cost), i128::from(self.customers)))
    }

    /// Converted value ÷ cost, in hundredths (4.00 is 400).
    pub fn roas_hundredths(&self) -> Option<i64> {
        let cost = self.known_cost()?;
        (cost > 0).then(|| div_half_up(i128::from(self.value) * 100, i128::from(cost)))
    }
}

/// The report: the rows of what the viewer sees, and their total.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The grouping.
    pub by: GroupBy,
    /// One row per group, ordered by key.
    pub rows: Vec<Row>,
    /// The sum of the rows; ratios of the sums.
    pub total: Row,
}

/// Computes the report of `facts` for `filter`, grouped by `by`, as `viewer`
/// sees it.
pub fn compute(facts: &Facts, filter: &Filter, by: GroupBy, viewer: &Viewer) -> Report {
    let mut rows: BTreeMap<GroupKey, Row> = BTreeMap::new();
    let mut hidden: BTreeSet<GroupKey> = BTreeSet::new();
    for lead in &facts.leads {
        if !filter.matches(&lead.attrs) || !viewer.sees(lead.attrs.branch.as_ref()) {
            continue;
        }
        if lead.counts && filter.in_period(lead.created) {
            touch(&mut rows, key_of(by, &lead.attrs, lead.created)).leads += 1;
        }
        if let Some(won_on) = lead.won_on.filter(|d| filter.in_period(*d)) {
            touch(&mut rows, key_of(by, &lead.attrs, won_on)).won += 1;
        }
    }
    for customer in &facts.customers {
        if !filter.matches(&customer.attrs)
            || !viewer.sees(customer.attrs.branch.as_ref())
            || !filter.in_period(customer.date)
        {
            continue;
        }
        let r = touch(&mut rows, key_of(by, &customer.attrs, customer.date));
        r.customers += 1;
        r.value += customer.value;
    }
    for cost in &facts.costs {
        if !filter.matches(&cost.attrs) || !filter.in_period(cost.date) {
            continue;
        }
        let key = key_of(by, &cost.attrs, cost.date);
        match &cost.attrs.branch {
            // A campaign of a branch belongs to it.
            Some(branch) if !viewer.sees(Some(branch)) => {}
            Some(_) => add_cost(touch(&mut rows, key), cost),
            // An entity-wide campaign: its cost is for those who see all.
            None if viewer.all => add_cost(touch(&mut rows, key), cost),
            None => {
                hidden.insert(key);
            }
        }
    }
    for key in hidden {
        if let Some(r) = rows.get_mut(&key) {
            r.cost_hidden = true;
        }
    }
    let rows: Vec<Row> = rows.into_values().collect();
    let mut total = Row::new(GroupKey { by, value: None });
    for r in &rows {
        total.leads += r.leads;
        total.won += r.won;
        total.customers += r.customers;
        total.value += r.value;
        total.cost_linked += r.cost_linked;
        total.cost_manual += r.cost_manual;
        if let Some(cost) = r.cost {
            total.cost = Some(total.cost.unwrap_or(0) + cost);
        }
        total.cost_hidden |= r.cost_hidden;
    }
    Report { by, rows, total }
}

/// The row of `key`, made when it is not there yet.
fn touch(rows: &mut BTreeMap<GroupKey, Row>, key: GroupKey) -> &mut Row {
    rows.entry(key.clone()).or_insert_with(|| Row::new(key))
}

fn add_cost(row: &mut Row, cost: &CostFact) {
    row.cost = Some(row.cost.unwrap_or(0) + cost.amount);
    if cost.linked {
        row.cost_linked += cost.amount;
    } else {
        row.cost_manual += cost.amount;
    }
}

/// `fils` as `1,500.00`, "—" when undefined.
fn money(fils: Option<i64>) -> String {
    fils.map_or_else(|| "—".to_owned(), aed)
}

/// `tenths` of a percent as `20.0%`.
fn percent(tenths: Option<i64>) -> String {
    tenths.map_or_else(|| "—".to_owned(), |t| format!("{}.{}%", t / 10, t % 10))
}

/// `hundredths` as `4.00`.
fn ratio(hundredths: Option<i64>) -> String {
    hundredths.map_or_else(|| "—".to_owned(), |h| format!("{}.{:02}", h / 100, h % 100))
}

/// A row as the page and the CSV show it; every ratio that is undefined is
/// "—".
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Cells {
    /// The leads.
    pub leads: String,
    /// The leads won.
    pub won: String,
    /// Leads won ÷ leads, `20.0%`.
    pub conversion: String,
    /// The new customers.
    pub customers: String,
    /// The converted value, AED.
    pub value: String,
    /// The cost, AED.
    pub cost: String,
    /// Of it, linked to bills, AED.
    pub cost_linked: String,
    /// Of it, manual, AED.
    pub cost_manual: String,
    /// Cost per lead, AED.
    pub cost_per_lead: String,
    /// Customer acquisition cost, AED.
    pub cac: String,
    /// Return on ad spend, `4.00`.
    pub roas: String,
}

impl Row {
    /// The row as text.
    pub fn cells(&self) -> Cells {
        let shown = |fils: i64| {
            if self.cost_hidden || self.cost.is_none() {
                None
            } else {
                Some(fils)
            }
        };
        Cells {
            leads: self.leads.to_string(),
            won: self.won.to_string(),
            conversion: percent(self.conversion_tenths()),
            customers: self.customers.to_string(),
            value: money(Some(self.value)),
            cost: money(self.known_cost()),
            cost_linked: money(shown(self.cost_linked)),
            cost_manual: money(shown(self.cost_manual)),
            cost_per_lead: money(self.cost_per_lead()),
            cac: money(self.cac()),
            roas: ratio(self.roas_hundredths()),
        }
    }
}

/// The name of group `key`, in `locale`.
pub async fn label_of(db: &Db, key: &GroupKey, locale: &str) -> Result<String> {
    let none = || crate::texts::english(&format!("wl_sales.report.none.{}", key.by.name()), &[]);
    let Some(value) = key.value.as_deref() else {
        return Ok(none());
    };
    Ok(match key.by {
        GroupBy::Source => match value.parse::<i64>().ok() {
            Some(id) => LeadSource::find(db, id)
                .await?
                .map_or_else(|| value.to_owned(), |s| s.name(locale).to_owned()),
            None => value.to_owned(),
        },
        GroupBy::Campaign => match value.parse::<i64>().ok() {
            Some(id) => Campaign::find(db, id)
                .await?
                .map_or_else(|| value.to_owned(), |c| c.code),
            None => value.to_owned(),
        },
        GroupBy::Channel | GroupBy::Brand | GroupBy::Branch | GroupBy::Month => value.to_owned(),
    })
}

/// The report as CSV: a header, a row per group and the total, with the same
/// numbers as the page. `labels` are the names of the rows, then of the total.
pub fn to_csv(report: &Report, labels: &[String], total_label: &str) -> String {
    fn field(text: &str) -> String {
        if text.contains([',', '"', '\n']) {
            format!("\"{}\"", text.replace('"', "\"\""))
        } else {
            text.to_owned()
        }
    }
    let mut out = String::from(
        "group,leads,won,conversion,new_customers,value_aed,cost_aed,cost_linked_aed,cost_manual_aed,cost_per_lead_aed,cac_aed,roas\n",
    );
    // The thousands separators of the page are left out of the file.
    let plain = |text: String| text.replace(',', "");
    let mut line = |label: &str, row: &Row| {
        let c = row.cells();
        let cells = [
            field(label),
            c.leads,
            c.won,
            c.conversion,
            c.customers,
            plain(c.value),
            plain(c.cost),
            plain(c.cost_linked),
            plain(c.cost_manual),
            plain(c.cost_per_lead),
            plain(c.cac),
            c.roas,
        ];
        out.push_str(&cells.join(","));
        out.push('\n');
    };
    for (row, label) in report.rows.iter().zip(labels) {
        line(label, row);
    }
    line(total_label, &report.total);
    out
}

/// Loads the facts: every lead, every live acquisition with a first sale and
/// every approved cost entry. The filters and the scope are applied by
/// [`compute`].
pub async fn load(db: &Db, config: &SalesConfig) -> Result<Facts> {
    let campaigns: BTreeMap<i64, Campaign> = Campaign::query()
        .get(db)
        .await?
        .into_iter()
        .map(|c| (c.id, c))
        .collect();
    let campaign_attrs =
        |id: Option<i64>| -> (Option<String>, Option<String>, Option<Subject>, Option<i64>) {
            match id.and_then(|id| campaigns.get(&id)) {
                Some(c) => (
                    c.marketing_channel.map(|m| m.to_string()),
                    c.brand.clone(),
                    c.branch(),
                    c.legal_entity_id,
                ),
                None => (None, None, None, None),
            }
        };
    let mut facts = Facts::default();
    for lead in Lead::query().order_by(Lead::ID).get(db).await? {
        let (channel, brand, campaign_branch, campaign_entity) = campaign_attrs(lead.campaign_id);
        let created = lead.created_at.map(dubai_date).unwrap_or_default();
        facts.leads.push(LeadFact {
            attrs: Attrs {
                source: Some(lead.source_id),
                campaign: lead.campaign_id,
                channel,
                brand,
                branch: lead.branch().or(campaign_branch),
                entity: campaign_entity.or(lead.legal_entity_id),
            },
            created,
            won_on: (lead.status == LeadStatus::Won)
                .then(|| lead.won_at.map(dubai_date))
                .flatten(),
            counts: !lead.left_out_of_counts(),
        });
    }
    let acquisitions = PartyAcquisition::query()
        .where_(PartyAcquisition::SUPERSEDED_BY.is_null())
        .where_(PartyAcquisition::FIRST_SALE_ID.is_not_null())
        .order_by(PartyAcquisition::ID)
        .get(db)
        .await?;
    for acquisition in acquisitions {
        let Some(first) = acquisition.first_sale() else {
            continue;
        };
        let sales = AcquisitionSale::query()
            .where_(AcquisitionSale::PARTY_ID.eq(acquisition.party_id))
            .order_by(AcquisitionSale::SALE_DATE)
            .order_by(AcquisitionSale::ID)
            .get(db)
            .await?;
        let Some(row) = sales.iter().find(|s| s.subject() == first) else {
            continue;
        };
        let (channel, brand, campaign_branch, campaign_entity) =
            campaign_attrs(acquisition.campaign_id);
        facts.customers.push(CustomerFact {
            attrs: Attrs {
                source: Some(acquisition.source_id),
                campaign: acquisition.campaign_id,
                channel,
                brand,
                branch: acquisition.branch().or(campaign_branch),
                entity: campaign_entity.or(acquisition.legal_entity_id),
            },
            date: row.sale_date,
            value: converted_value(config, row, &sales),
        });
    }
    let entries = CostEntry::query()
        .where_(CostEntry::STATUS.eq(CostStatus::Approved))
        .order_by(CostEntry::ID)
        .get(db)
        .await?;
    for entry in entries {
        let Some(campaign) = campaigns.get(&entry.campaign_id) else {
            continue;
        };
        let (channel, brand, branch, entity) = campaign_attrs(Some(campaign.id));
        facts.costs.push(CostFact {
            attrs: Attrs {
                source: campaign.lead_source_id,
                campaign: Some(campaign.id),
                channel,
                brand,
                branch,
                entity,
            },
            date: entry.entry_date,
            amount: entry.amount,
            linked: entry.supplier_bill_id.is_some(),
        });
    }
    Ok(facts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    fn c1() -> Attrs {
        Attrs {
            source: Some(5),
            campaign: Some(1),
            ..Attrs::default()
        }
    }

    /// C1: 50 leads, 10 won, 8 new customers with AED 20,000, cost AED 5,000
    /// (AED 3,000 linked to bills, 2,000 manual).
    fn campaign_facts() -> Facts {
        let mut facts = Facts::default();
        for i in 0..50 {
            facts.leads.push(LeadFact {
                attrs: c1(),
                created: day(3, 1 + i % 28),
                won_on: (i < 10).then(|| day(3, 20)),
                counts: true,
            });
        }
        for _ in 0..8 {
            facts.customers.push(CustomerFact {
                attrs: c1(),
                date: day(3, 25),
                value: 250_000,
            });
        }
        facts.costs.push(CostFact {
            attrs: c1(),
            date: day(3, 2),
            amount: 300_000,
            linked: true,
        });
        facts.costs.push(CostFact {
            attrs: c1(),
            date: day(3, 3),
            amount: 200_000,
            linked: false,
        });
        facts
    }

    #[test]
    fn the_ratios_of_a_campaign_are_rounded_and_split() {
        let report = compute(
            &campaign_facts(),
            &Filter::default(),
            GroupBy::Campaign,
            &Viewer::all(),
        );
        assert_eq!(report.rows.len(), 1);
        let cells = report.rows[0].cells();
        assert_eq!(cells.leads, "50");
        assert_eq!(cells.conversion, "20.0%");
        assert_eq!(cells.cost, "5,000.00");
        assert_eq!(cells.cost_linked, "3,000.00");
        assert_eq!(cells.cost_manual, "2,000.00");
        assert_eq!(cells.cost_per_lead, "100.00");
        assert_eq!(cells.cac, "625.00");
        assert_eq!(cells.value, "20,000.00");
        assert_eq!(cells.roas, "4.00");
    }

    #[test]
    fn a_group_without_cost_shows_dashes_and_never_zero() {
        let mut facts = Facts::default();
        for _ in 0..12 {
            facts.leads.push(LeadFact {
                attrs: Attrs {
                    source: Some(2),
                    ..Attrs::default()
                },
                created: day(3, 4),
                won_on: None,
                counts: true,
            });
        }
        let report = compute(&facts, &Filter::default(), GroupBy::Source, &Viewer::all());
        let cells = report.rows[0].cells();
        assert_eq!(cells.leads, "12");
        assert_eq!(cells.conversion, "0.0%");
        assert_eq!(
            (
                cells.cost.as_str(),
                cells.cost_per_lead.as_str(),
                cells.cac.as_str(),
                cells.roas.as_str()
            ),
            ("—", "—", "—", "—")
        );
    }

    #[test]
    fn leads_closed_as_spam_are_left_out_and_the_cost_per_lead_follows() {
        let mut facts = campaign_facts();
        for lead in facts.leads.iter_mut().skip(45) {
            lead.counts = false;
        }
        let report = compute(
            &facts,
            &Filter::default(),
            GroupBy::Campaign,
            &Viewer::all(),
        );
        let cells = report.rows[0].cells();
        assert_eq!(cells.leads, "45");
        assert_eq!(cells.cost_per_lead, "111.11");
    }

    #[test]
    fn periods_use_each_facts_own_date() {
        let facts = campaign_facts();
        let april = Filter::default().month(2026, 4);
        let report = compute(&facts, &april, GroupBy::Campaign, &Viewer::all());
        assert!(report.rows.is_empty());
        let by_month = compute(&facts, &Filter::default(), GroupBy::Month, &Viewer::all());
        assert_eq!(by_month.rows.len(), 1);
        assert_eq!(by_month.rows[0].key.value.as_deref(), Some("2026-03"));
        // A customer in April, a cost in March: separate months.
        let mut split = campaign_facts();
        split.customers[0].date = day(4, 2);
        let by_month = compute(&split, &Filter::default(), GroupBy::Month, &Viewer::all());
        assert_eq!(by_month.rows.len(), 2);
        assert_eq!(by_month.rows[1].customers, 1);
        assert_eq!(by_month.rows[1].cost, None);
    }

    #[test]
    fn an_entity_wide_campaign_hides_its_cost_from_a_branch_scoped_user() {
        let b1 = Subject::new("test.branch", 1);
        let b2 = Subject::new("test.branch", 2);
        let c9 = |branch: &Subject| Attrs {
            source: Some(5),
            campaign: Some(9),
            branch: Some(branch.clone()),
            ..Attrs::default()
        };
        let mut facts = Facts::default();
        for branch in [&b1, &b2] {
            facts.leads.push(LeadFact {
                attrs: c9(branch),
                created: day(3, 4),
                won_on: None,
                counts: true,
            });
        }
        facts.costs.push(CostFact {
            attrs: Attrs {
                source: Some(5),
                campaign: Some(9),
                ..Attrs::default()
            },
            date: day(3, 5),
            amount: 100_000,
            linked: false,
        });
        let scoped = compute(
            &facts,
            &Filter::default(),
            GroupBy::Campaign,
            &Viewer::scoped(vec![b1.clone()]),
        );
        assert_eq!(scoped.rows.len(), 1);
        assert_eq!(scoped.rows[0].leads, 1, "only B1's lead");
        let cells = scoped.rows[0].cells();
        assert_eq!(
            (
                cells.cost.as_str(),
                cells.cost_per_lead.as_str(),
                cells.cac.as_str(),
                cells.roas.as_str()
            ),
            ("—", "—", "—", "—")
        );
        let everything = compute(
            &facts,
            &Filter::default(),
            GroupBy::Campaign,
            &Viewer::all(),
        );
        assert_eq!(everything.rows[0].leads, 2);
        assert_eq!(everything.rows[0].cells().cost, "1,000.00");
        assert_eq!(everything.rows[0].cells().cost_per_lead, "500.00");
        assert!(Viewer::scoped(Vec::new()).is_blind());
    }

    #[test]
    fn the_csv_has_the_same_numbers() {
        let report = compute(
            &campaign_facts(),
            &Filter::default(),
            GroupBy::Campaign,
            &Viewer::all(),
        );
        let csv = to_csv(&report, &["SPRING26".to_owned()], "Total");
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 3);
        // The same numbers as the page, without its thousands separators.
        assert!(
            lines[1].starts_with("SPRING26,50,10,20.0%,8,20000.00"),
            "{}",
            lines[1]
        );
    }
}
