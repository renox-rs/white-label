//! Receivables ageing (wl-invoicing §5.10, A15): the open balance of every
//! party, in buckets by days past the due date, with a *Disputed* column.
//!
//! [`report`] reads the invoices in `issued`, `partially_paid` and `overdue`
//! and groups them per party and currency. The bucket edges are the
//! `.ageing_buckets([30, 60, 90])` setting: 0-30 (which also holds invoices
//! not yet due), 31-60, 61-90 and 90+. For a disputed invoice the *Disputed*
//! column takes the lesser of the disputed amount and the balance, and the
//! bucket gets the rest, so every row adds up to its balance. [`build`] is
//! the pure part.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::invoicing::InvoiceStatus;

use crate::dispute::Dispute;
use crate::invoice::Invoice;

/// The states whose balance is receivable.
const OPEN: [InvoiceStatus; 3] = [
    InvoiceStatus::Issued,
    InvoiceStatus::PartiallyPaid,
    InvoiceStatus::Overdue,
];

/// The position of the bucket an invoice `days` past its due date falls in:
/// the first whose edge is not below `days`, else the last (`90+`). Invoices
/// not yet due (negative `days`) are in the first.
pub fn bucket_of(edges: &[u32], days: i64) -> usize {
    edges
        .iter()
        .position(|edge| days <= i64::from(*edge))
        .unwrap_or(edges.len())
}

/// The labels of the buckets of `edges`: `0-30`, `31-60`, `61-90`, `90+`.
pub fn bucket_labels(edges: &[u32]) -> Vec<String> {
    let mut labels: Vec<String> = Vec::with_capacity(edges.len() + 1);
    let mut from = 0;
    for edge in edges {
        labels.push(format!("{from}-{edge}"));
        from = edge + 1;
    }
    labels.push(match edges.last() {
        Some(last) => format!("{last}+"),
        None => "0+".to_owned(),
    });
    labels
}

/// One party's open balance in one currency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgeingRow {
    /// The party.
    pub party_id: i64,
    /// Its name on its invoices, else `#id`.
    pub party: String,
    /// The currency of the amounts.
    pub currency: String,
    /// The balance in each bucket, without the disputed part.
    pub buckets: Vec<i64>,
    /// The disputed part of the balances.
    pub disputed: i64,
    /// The balance: the buckets and the disputed part.
    pub total: i64,
}

/// The ageing of the receivables on one day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgeingReport {
    /// The day the days past due are counted to.
    pub on: NaiveDate,
    /// The label of each bucket.
    pub labels: Vec<String>,
    /// One row per party and currency, by party name.
    pub rows: Vec<AgeingRow>,
}

/// The ageing of `invoices` on `today`. `disputed` maps an invoice to its
/// disputed amount.
pub fn build(
    edges: &[u32],
    invoices: &[Invoice],
    disputed: &BTreeMap<i64, i64>,
    today: NaiveDate,
) -> AgeingReport {
    let count = edges.len() + 1;
    let mut rows: BTreeMap<(i64, String), AgeingRow> = BTreeMap::new();
    for invoice in invoices.iter().filter(|i| i.balance > 0) {
        let key = (invoice.party_id, invoice.currency.clone());
        let row = rows.entry(key).or_insert_with(|| AgeingRow {
            party_id: invoice.party_id,
            party: String::new(),
            currency: invoice.currency.clone(),
            buckets: vec![0; count],
            disputed: 0,
            total: 0,
        });
        if let Some(snapshot) = invoice.party_snapshot.as_ref() {
            row.party = snapshot.legal_name_en.clone();
        }
        let days = invoice.due_date.map_or(0, |due| (today - due).num_days());
        let held = if invoice.disputed {
            disputed
                .get(&invoice.id)
                .copied()
                .unwrap_or(0)
                .clamp(0, invoice.balance)
        } else {
            0
        };
        row.buckets[bucket_of(edges, days)] += invoice.balance - held;
        row.disputed += held;
        row.total += invoice.balance;
    }
    let mut rows: Vec<AgeingRow> = rows.into_values().collect();
    for row in &mut rows {
        if row.party.is_empty() {
            row.party = format!("#{}", row.party_id);
        }
    }
    rows.sort_by(|a, b| {
        a.party
            .to_lowercase()
            .cmp(&b.party.to_lowercase())
            .then(a.party_id.cmp(&b.party_id))
            .then_with(|| a.currency.cmp(&b.currency))
    });
    AgeingReport {
        on: today,
        labels: bucket_labels(edges),
        rows,
    }
}

/// The ageing on `today` of the receivables of the legal entity `entity`
/// (all entities when `None`), with `edges` from the `.ageing_buckets`
/// setting.
pub async fn report(
    db: &Db,
    edges: &[u32],
    entity: Option<i64>,
    today: NaiveDate,
) -> Result<AgeingReport> {
    let mut query = Invoice::query()
        .where_(Invoice::STATUS.is_in(OPEN))
        .where_(Invoice::BALANCE.gt(0));
    if let Some(entity) = entity {
        query = query.where_(Invoice::LEGAL_ENTITY_ID.eq(entity));
    }
    let invoices = query.order_by(Invoice::ID).get(db).await?;
    let ids: Vec<i64> = invoices.iter().filter_map(|i| i.dispute_id).collect();
    let by_invoice = if ids.is_empty() {
        BTreeMap::new()
    } else {
        Dispute::query()
            .where_(Dispute::ID.is_in(ids))
            .get(db)
            .await?
            .into_iter()
            .map(|d| (d.invoice_id, d.amount))
            .collect()
    };
    Ok(build(edges, &invoices, &by_invoice, today))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invoice::PartySnapshot;
    use renox::chrono::Days;

    const EDGES: [u32; 3] = [30, 60, 90];

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    fn invoice(id: i64, party: i64, due: NaiveDate, balance: i64) -> Invoice {
        Invoice {
            id,
            party_id: party,
            currency: "AED".into(),
            due_date: Some(due),
            balance,
            status: InvoiceStatus::Issued,
            party_snapshot: Some(
                PartySnapshot {
                    legal_name_en: format!("Party {party}"),
                    legal_name_ar: None,
                    trn: None,
                    billing_address: None,
                    billing_email: None,
                    taken_at: renox::db::now(),
                }
                .into(),
            ),
            ..Default::default()
        }
    }

    #[test]
    fn the_edges_make_the_buckets() {
        assert_eq!(bucket_labels(&EDGES), ["0-30", "31-60", "61-90", "90+"]);
        assert_eq!(bucket_labels(&[]), ["0+"]);
        assert_eq!(bucket_of(&EDGES, -5), 0);
        assert_eq!(bucket_of(&EDGES, 0), 0);
        assert_eq!(bucket_of(&EDGES, 30), 0);
        assert_eq!(bucket_of(&EDGES, 31), 1);
        assert_eq!(bucket_of(&EDGES, 60), 1);
        assert_eq!(bucket_of(&EDGES, 61), 2);
        assert_eq!(bucket_of(&EDGES, 90), 2);
        assert_eq!(bucket_of(&EDGES, 91), 3);
        assert_eq!(bucket_of(&[], 400), 0);
    }

    #[test]
    fn balances_fall_in_the_bucket_of_their_days_past_due() {
        let today = day(12, 31);
        let invoices = [
            invoice(1, 1, day(12, 30), 100),
            invoice(2, 1, day(11, 1), 200),
            invoice(3, 1, day(10, 15), 400),
            invoice(4, 1, day(1, 1), 800),
            invoice(5, 1, today + Days::new(20), 1_600),
        ];
        let report = build(&EDGES, &invoices, &BTreeMap::new(), today);
        assert_eq!(report.rows.len(), 1);
        let row = &report.rows[0];
        assert_eq!(row.buckets, [1_700, 200, 400, 800]);
        assert_eq!((row.disputed, row.total), (0, 3_100));
    }

    #[test]
    fn a_disputed_invoice_moves_the_disputed_part_to_its_column() {
        let mut disputed = invoice(1, 1, day(12, 1), 500_000);
        disputed.disputed = true;
        let amounts = BTreeMap::from([(1, 120_000)]);
        let report = build(&EDGES, &[disputed.clone()], &amounts, day(12, 31));
        let row = &report.rows[0];
        assert_eq!(row.disputed, 120_000);
        assert_eq!(row.buckets, [380_000, 0, 0, 0]);
        assert_eq!(row.total, 500_000);

        // After a payment the column shows the balance; the stored amount is
        // not changed.
        disputed.balance = 50_000;
        let report = build(&EDGES, &[disputed], &amounts, day(12, 31));
        let row = &report.rows[0];
        assert_eq!(
            (row.disputed, row.buckets[0], row.total),
            (50_000, 0, 50_000)
        );
    }

    #[test]
    fn rows_are_per_party_and_currency_by_name() {
        let mut usd = invoice(3, 1, day(12, 1), 700);
        usd.currency = "USD".into();
        let invoices = [
            invoice(1, 2, day(12, 1), 10),
            invoice(2, 1, day(12, 1), 20),
            usd,
            invoice(4, 2, day(12, 2), 5),
            invoice(5, 3, day(12, 2), 0),
        ];
        let report = build(&EDGES, &invoices, &BTreeMap::new(), day(12, 31));
        let seen: Vec<(&str, &str, i64)> = report
            .rows
            .iter()
            .map(|r| (r.party.as_str(), r.currency.as_str(), r.total))
            .collect();
        assert_eq!(
            seen,
            [
                ("Party 1", "AED", 20),
                ("Party 1", "USD", 700),
                ("Party 2", "AED", 15)
            ]
        );
    }

    #[test]
    fn a_party_without_a_snapshot_shows_its_id() {
        let mut bare = invoice(1, 9, day(12, 1), 10);
        bare.party_snapshot = None;
        let report = build(&EDGES, &[bare], &BTreeMap::new(), day(12, 31));
        assert_eq!(report.rows[0].party, "#9");
    }
}
