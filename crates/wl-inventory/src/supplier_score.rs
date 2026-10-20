//! The supplier score and its monthly snapshots (wl-inventory §3 *Supplier
//! score*, §5.9 step 2, `STK.SUP.01`, issue #248). How reliable a supplier is,
//! from 0 to 100, over the last three calendar months: a live view up to
//! today, and on day 1 of each month (06:00 Asia/Dubai) a snapshot that is
//! never recomputed, so trends and the review of a low score use fixed
//! numbers.
//!
//! The inputs of a window (the receipts count by the day the goods arrived):
//!
//! 1. **On time:** a posted goods receipt against an order is on time when
//!    its business date (the day the goods arrived: [`GoodsReceipt::arrived_at`],
//!    else the posting) is on or before the expected date plus the grace
//!    days. The expected date is the first issued revision's; a later one set
//!    by a revision counts only when it records *requested by us*
//!    ([`expected_date_of`]). A receipt against an order with no expected
//!    date counts as a receipt but not in the on-time share. A reversed
//!    receipt and a free replacement are not counted.
//! 2. **Rejected value:** the rejected quantity at receipt (reason `excess`
//!    left out) at the order's price, plus the lines of a Return to supplier
//!    taken from stock, at their unit cost, counted when the return is
//!    `dispatched` (it stays counted when it is later `credited`), over the
//!    value received at the order's price. A debit note is not counted on
//!    top.
//! 3. **Quality incidents:** those the vertical's `SupplierIncidentSource`
//!    reports whose cause is the supplier and that are not voided. Recalls
//!    are not built yet, so there are none from them.
//! 4. **Debit notes:** their count and value, shown, not scored (the
//!    `SupplierDebitNoteIssued` and `...Cancelled` events keep
//!    [`SupplierDebitNoteRecord`]).
//! 5. **Price variance:** for the order lines issued in the window that have
//!    a last price, the sum of (price - last price) x quantity over the sum
//!    of last price x quantity; the last price is the latest receipt line
//!    posted before the revision in force was issued (receipts against that
//!    order not used), else the latest order line issued before it. Lines
//!    are compared in the order's currency and a line whose last price is in
//!    another currency drops out (no exchange rates are kept here). Only an
//!    increase lowers the score.
//!
//! The score is `W1 x on time + W2 x (1 - min(1, rejected / R)) + W3 x
//! max(0, 1 - incidents / I) + W4 x (1 - min(1, increase / P))`, a component
//! without data dropping out with its weight spread over the others, rounded
//! half up; with fewer than N receipts there is no score (`not_enough_data`).
//! Fractions are kept in basis points (four decimals of a percentage as a
//! fraction), whole-number arithmetic throughout. The settings are
//! [`ScoreSettings`].
//!
//! Decisions: values of different currencies are added at face value (no
//! exchange rate); the snapshot run is daily and idempotent, so a missed day
//! 1 is caught up by the next run, which builds every missing month from the
//! month after the supplier's first receipt, once per supplier and month.

use std::collections::BTreeMap;

use renox::chrono::{Datelike, Days, Months, NaiveDate};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::inventory::supplier_incidents;
use wl_kernel::invoicing::{SupplierDebitNoteCancelled, SupplierDebitNoteIssued};

use crate::config::ScoreSettings;
use crate::config_of;
use crate::goods_receipt::{GoodsReceipt, GoodsReceiptLine, GoodsReceiptStatus};
use crate::item::value_of;
use crate::item_supplier::{audit, last_price_before, refused, require};
use crate::permissions;
use crate::po_revise::revisions_of;
use crate::purchase_order::{PurchaseOrder, PurchaseOrderStatus, lines_of};
use crate::supplier_return::{self, ReturnOrigin, ReturnStatus, SupplierReturn};
use crate::texts::said;

/// The daily scheduled task that runs [`run`].
pub const TASK: &str = "inventory:supplier-scores";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "06:00";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The score at or above which a supplier is `good`.
pub const GOOD_FROM: i64 = 85;

/// 100% in basis points.
const WHOLE_BP: i128 = 10_000;

/// How a supplier stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScoreBand {
    /// Fewer receipts than the minimum: no score.
    #[default]
    NotEnoughData,
    /// 85 or more.
    Good,
    /// From the threshold to 84.
    Watch,
    /// Under the threshold (`STK.RUL.09`).
    BelowThreshold,
}

impl ScoreBand {
    /// The stored key.
    pub fn as_str(self) -> &'static str {
        match self {
            ScoreBand::NotEnoughData => "not_enough_data",
            ScoreBand::Good => "good",
            ScoreBand::Watch => "watch",
            ScoreBand::BelowThreshold => "below_threshold",
        }
    }
}

/// The numbers of a window, live or kept in a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Figures {
    /// First day of the window.
    pub from: NaiveDate,
    /// Last day of the window.
    pub to: NaiveDate,
    /// The receipts counted.
    pub receipts: i64,
    /// Those on time.
    pub on_time: i64,
    /// The on-time share, in basis points; none without a dated receipt.
    pub on_time_bp: Option<i64>,
    /// The value received at the order's price.
    pub received_value: i64,
    /// The value rejected.
    pub rejected_value: i64,
    /// The rejected share, in basis points; none without value received.
    pub rejected_bp: Option<i64>,
    /// The quality incidents.
    pub incidents: i64,
    /// The debit notes issued and not cancelled.
    pub debit_notes: i64,
    /// Their value.
    pub debit_note_value: i64,
    /// The price variance, in basis points (negative when prices fell); none
    /// without an order line that has a last price.
    pub price_variance_bp: Option<i64>,
    /// The score 0 to 100; none for `not_enough_data`.
    pub score: Option<i64>,
    /// The band.
    pub band: ScoreBand,
}

/// A supplier debit note an event told about, kept for the score.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_supplier_debit_notes",
    unique(debit_note_id),
    index(supplier_party_id)
)]
pub struct SupplierDebitNoteRecord {
    /// The primary key.
    pub id: i64,
    /// The debit note in wl-invoicing.
    pub debit_note_id: i64,
    /// The supplier.
    pub supplier_party_id: i64,
    /// The total with VAT, in fils of the bill's currency.
    pub amount: i64,
    /// The Asia/Dubai day it was issued.
    pub issued_on: NaiveDate,
    /// Whether it was cancelled.
    pub cancelled: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A monthly snapshot of one supplier's performance. Derived and never
/// edited; a review note is added beside it.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_supplier_scores",
    unique(supplier_party_id, snapshot_date)
)]
pub struct SupplierScore {
    /// The primary key.
    pub id: i64,
    /// The supplier.
    pub supplier_party_id: i64,
    /// Day 1 of the month it is named by.
    pub snapshot_date: NaiveDate,
    /// First day of the window.
    pub window_from: NaiveDate,
    /// Last day of the window.
    pub window_to: NaiveDate,
    /// The receipts counted.
    pub receipts: i64,
    /// Those on time.
    pub on_time: i64,
    /// The on-time share, in basis points.
    pub on_time_bp: Option<i64>,
    /// The rejected share, in basis points.
    pub rejected_bp: Option<i64>,
    /// The value received.
    pub received_value: i64,
    /// The value rejected.
    pub rejected_value: i64,
    /// The quality incidents.
    pub incidents: i64,
    /// The debit notes.
    pub debit_notes: i64,
    /// Their value.
    pub debit_note_value: i64,
    /// The price variance, in basis points.
    pub price_variance_bp: Option<i64>,
    /// The score; none for `not_enough_data`.
    pub score: Option<i64>,
    /// The band.
    pub band: ScoreBand,
    /// When it was computed.
    pub computed_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl SupplierScore {
    /// The supplier.
    pub fn supplier(&self) -> PartyId {
        PartyId(self.supplier_party_id)
    }

    /// The numbers the snapshot kept.
    pub fn figures(&self) -> Figures {
        Figures {
            from: self.window_from,
            to: self.window_to,
            receipts: self.receipts,
            on_time: self.on_time,
            on_time_bp: self.on_time_bp,
            received_value: self.received_value,
            rejected_value: self.rejected_value,
            rejected_bp: self.rejected_bp,
            incidents: self.incidents,
            debit_notes: self.debit_notes,
            debit_note_value: self.debit_note_value,
            price_variance_bp: self.price_variance_bp,
            score: self.score,
            band: self.band,
        }
    }
}

/// A review note on a snapshot: added, never changed.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_inventory_supplier_score_notes", index(supplier_score_id))]
pub struct SupplierScoreNote {
    /// The primary key.
    pub id: i64,
    /// The snapshot.
    pub supplier_score_id: i64,
    /// The text.
    pub note: String,
    /// Who wrote it.
    pub written_by: Option<i64>,
    /// When it was written.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The business date of a receipt: the day the goods arrived.
pub fn business_date(receipt: &GoodsReceipt) -> Option<NaiveDate> {
    receipt.arrived_at.or(receipt.posted_at).map(dubai_date)
}

/// `numerator` over `denominator` in basis points, rounded half up (half
/// away from zero for a negative one); 0 over nothing.
pub fn ratio_bp(numerator: i64, denominator: i64) -> i64 {
    if denominator <= 0 {
        return 0;
    }
    if numerator < 0 {
        return -ratio_bp(-numerator, denominator);
    }
    let value = (2 * i128::from(numerator) * WHOLE_BP + i128::from(denominator))
        / (2 * i128::from(denominator));
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// `value` basis points as a share of the cap, taken off the whole: 10000
/// at 0, 0 at the cap and beyond.
fn after_cap(value_bp: i64, cap_bp: i64) -> i128 {
    if cap_bp <= 0 {
        return WHOLE_BP;
    }
    let used = (i128::from(value_bp.max(0)) * WHOLE_BP / i128::from(cap_bp)).min(WHOLE_BP);
    WHOLE_BP - used
}

/// The score of `figures` under `settings`, rounded half up, when there are
/// enough receipts for one.
pub fn score_of(figures: &Figures, settings: &ScoreSettings) -> Option<i64> {
    if figures.receipts < settings.min_receipts {
        return None;
    }
    let weights = settings.weights.map(i128::from);
    let mut sum: i128 = 0;
    let mut spread: i128 = 0;
    if let Some(bp) = figures.on_time_bp {
        sum += weights[0] * i128::from(bp);
        spread += weights[0];
    }
    if let Some(bp) = figures.rejected_bp {
        sum += weights[1] * after_cap(bp, settings.rejected_cap_bp);
        spread += weights[1];
    }
    let incidents = if settings.incident_cap <= 0 {
        WHOLE_BP
    } else {
        (WHOLE_BP - i128::from(figures.incidents) * WHOLE_BP / i128::from(settings.incident_cap))
            .max(0)
    };
    sum += weights[2] * incidents;
    spread += weights[2];
    if let Some(bp) = figures.price_variance_bp {
        sum += weights[3] * after_cap(bp, settings.price_cap_bp);
        spread += weights[3];
    }
    if spread == 0 {
        return None;
    }
    let value = (2 * sum + spread * 100) / (2 * spread * 100);
    i64::try_from(value).ok().map(|score| score.clamp(0, 100))
}

/// The band of `score` under `settings`.
pub fn band_of(score: Option<i64>, settings: &ScoreSettings) -> ScoreBand {
    match score {
        None => ScoreBand::NotEnoughData,
        Some(score) if score >= GOOD_FROM => ScoreBand::Good,
        Some(score) if score >= settings.threshold => ScoreBand::Watch,
        Some(_) => ScoreBand::BelowThreshold,
    }
}

/// The expected date of `order` for the score: the date of the first issued
/// revision, moved later only by a revision that records *requested by us*.
///
/// # Errors
///
/// A database error.
pub async fn expected_date_of(db: &Db, order: &PurchaseOrder) -> Result<Option<NaiveDate>> {
    let mut sequence: Vec<(Option<NaiveDate>, Option<String>)> = revisions_of(db, order.id)
        .await?
        .iter()
        .map(|kept| (kept.expected_date, kept.date_requested_by.clone()))
        .collect();
    sequence.push((order.expected_date, order.date_requested_by.clone()));
    let mut revisions = sequence.into_iter();
    let Some((first, _)) = revisions.next() else {
        return Ok(None);
    };
    let mut expected = first;
    for (date, by) in revisions {
        if let Some(date) = date
            && by.as_deref() == Some("us")
            && expected.is_none_or(|current| date > current)
        {
            expected = Some(date);
        }
    }
    Ok(expected)
}

/// The window of the snapshot named by `day` (day 1): the `window_months`
/// calendar months before it.
pub fn snapshot_window(day: NaiveDate, settings: &ScoreSettings) -> (NaiveDate, NaiveDate) {
    let from = day
        .checked_sub_months(Months::new(settings.window_months))
        .unwrap_or(day);
    let to = day.pred_opt().unwrap_or(day);
    (from, to)
}

/// Whether `day` is inside the window.
fn within(day: NaiveDate, from: NaiveDate, to: NaiveDate) -> bool {
    day >= from && day <= to
}

/// The figures of `supplier` for the days from `from` to `to`.
///
/// # Errors
///
/// A database error.
pub async fn compute(
    state: &AppState,
    supplier: i64,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Figures> {
    let settings = config_of(state).score.clone();
    let db = &state.db;
    let grace = Days::new(u64::try_from(settings.grace_days.max(0)).unwrap_or(0));
    let mut figures = Figures {
        from,
        to,
        ..Figures::default()
    };
    let mut dated = 0;
    let receipts = GoodsReceipt::query()
        .where_(GoodsReceipt::SUPPLIER_PARTY_ID.eq(supplier))
        .where_(GoodsReceipt::STATUS.eq(GoodsReceiptStatus::Posted))
        .get(db)
        .await?;
    for receipt in receipts {
        let Some(order_id) = receipt.purchase_order_id else {
            continue;
        };
        let Some(day) = business_date(&receipt) else {
            continue;
        };
        if !within(day, from, to) {
            continue;
        }
        figures.receipts += 1;
        let order = PurchaseOrder::find_or_404(db, order_id).await?;
        if let Some(expected) = expected_date_of(db, &order).await? {
            dated += 1;
            if expected.checked_add_days(grace).is_some_and(|due| day <= due) {
                figures.on_time += 1;
            }
        }
        let lines = GoodsReceiptLine::query()
            .where_(GoodsReceiptLine::GOODS_RECEIPT_ID.eq(receipt.id))
            .get(db)
            .await?;
        for line in lines.iter().filter(|line| line.po_line_id.is_some()) {
            figures.received_value += value_of(line.received_qty_milli, line.unit_cost);
            if line.rejection_reason.as_deref() != Some("excess") {
                figures.rejected_value += value_of(line.rejected_qty_milli, line.unit_cost);
            }
        }
    }
    let returns = SupplierReturn::query()
        .where_(SupplierReturn::SUPPLIER_PARTY_ID.eq(supplier))
        .where_(SupplierReturn::STATUS.is_in([ReturnStatus::Dispatched, ReturnStatus::Credited]))
        .get(db)
        .await?;
    for ret in returns {
        let Some(sent) = ret.dispatched_at.map(dubai_date) else {
            continue;
        };
        if !within(sent, from, to) {
            continue;
        }
        for line in supplier_return::lines_of(db, ret.id).await? {
            if line.origin == ReturnOrigin::FromStock {
                figures.rejected_value += value_of(line.quantity_milli, line.unit_cost);
            }
        }
    }
    figures.on_time_bp = (dated > 0).then(|| ratio_bp(figures.on_time, dated));
    figures.rejected_bp =
        (figures.received_value > 0).then(|| ratio_bp(figures.rejected_value, figures.received_value));
    let found = supplier_incidents(state)
        .incidents(db, PartyId(supplier), from, to)
        .await?;
    figures.incidents = i64::try_from(found.len()).unwrap_or(i64::MAX);
    let notes = SupplierDebitNoteRecord::query()
        .where_(SupplierDebitNoteRecord::SUPPLIER_PARTY_ID.eq(supplier))
        .where_(SupplierDebitNoteRecord::CANCELLED.eq(false))
        .get(db)
        .await?;
    for note in notes.iter().filter(|note| within(note.issued_on, from, to)) {
        figures.debit_notes += 1;
        figures.debit_note_value += note.amount;
    }
    figures.price_variance_bp = price_variance_bp(state, supplier, from, to).await?;
    figures.score = score_of(&figures, &settings);
    figures.band = band_of(figures.score, &settings);
    Ok(figures)
}

/// The price variance of the order lines of `supplier` issued from `from` to
/// `to`, in basis points; none when no line has a last price.
async fn price_variance_bp(
    state: &AppState,
    supplier: i64,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Option<i64>> {
    let db = &state.db;
    let orders = PurchaseOrder::query()
        .where_(PurchaseOrder::SUPPLIER_PARTY_ID.eq(supplier))
        .where_(PurchaseOrder::STATUS.is_in([
            PurchaseOrderStatus::Issued,
            PurchaseOrderStatus::PartiallyReceived,
            PurchaseOrderStatus::Received,
            PurchaseOrderStatus::Closed,
        ]))
        .get(db)
        .await?;
    let mut increase: i64 = 0;
    let mut base: i64 = 0;
    for order in orders {
        let Some(issued) = order.issued_at else {
            continue;
        };
        if !within(dubai_date(issued), from, to) {
            continue;
        }
        let at = order.revision_issued_at.unwrap_or(issued);
        for line in lines_of(db, order.id).await? {
            let Some(last) =
                last_price_before(db, line.item_id, supplier, Some(at), Some(order.id)).await?
            else {
                continue;
            };
            if last.currency != order.currency {
                continue;
            }
            increase += value_of(line.quantity_milli, line.unit_price - last.unit_price);
            base += value_of(line.quantity_milli, last.unit_price);
        }
    }
    Ok((base > 0).then(|| ratio_bp(increase, base)))
}

/// The live view of `supplier`: the window up to today.
///
/// # Errors
///
/// A database error.
pub async fn live(state: &AppState, supplier: i64) -> Result<Figures> {
    let settings = config_of(state).score.clone();
    let day = today();
    let from = day
        .checked_sub_months(Months::new(settings.window_months))
        .unwrap_or(day);
    compute(state, supplier, from, day).await
}

/// Builds the snapshots that are missing on `on`: for every supplier with a
/// posted receipt, each month's day 1 from the month after its first receipt
/// to the latest day 1 that is not after `on`, once per supplier and month.
/// A snapshot that exists is left as it is. Returns those made now, oldest
/// month first.
///
/// # Errors
///
/// A database error.
pub async fn snapshot_run(state: &AppState, on: NaiveDate) -> Result<Vec<SupplierScore>> {
    let settings = config_of(state).score.clone();
    let db = &state.db;
    let receipts = GoodsReceipt::query()
        .where_(GoodsReceipt::STATUS.eq(GoodsReceiptStatus::Posted))
        .get(db)
        .await?;
    let mut first_day: BTreeMap<i64, NaiveDate> = BTreeMap::new();
    for receipt in receipts.iter().filter(|r| r.purchase_order_id.is_some()) {
        if let Some(day) = business_date(receipt) {
            first_day
                .entry(receipt.supplier_party_id)
                .and_modify(|first| *first = (*first).min(day))
                .or_insert(day);
        }
    }
    let mut made = Vec::new();
    for (supplier, first) in first_day {
        let mut next = NaiveDate::from_ymd_opt(first.year(), first.month(), 1)
            .and_then(|month| month.checked_add_months(Months::new(1)));
        while let Some(day) = next {
            if day > on {
                break;
            }
            next = day.checked_add_months(Months::new(1));
            let kept = SupplierScore::query()
                .where_(SupplierScore::SUPPLIER_PARTY_ID.eq(supplier))
                .where_(SupplierScore::SNAPSHOT_DATE.eq(day))
                .first(db)
                .await?;
            if kept.is_some() {
                continue;
            }
            let (from, to) = snapshot_window(day, &settings);
            let figures = compute(state, supplier, from, to).await?;
            let mut snapshot = SupplierScore {
                supplier_party_id: supplier,
                snapshot_date: day,
                window_from: from,
                window_to: to,
                receipts: figures.receipts,
                on_time: figures.on_time,
                on_time_bp: figures.on_time_bp,
                rejected_bp: figures.rejected_bp,
                received_value: figures.received_value,
                rejected_value: figures.rejected_value,
                incidents: figures.incidents,
                debit_notes: figures.debit_notes,
                debit_note_value: figures.debit_note_value,
                price_variance_bp: figures.price_variance_bp,
                score: figures.score,
                band: figures.band,
                computed_at: Some(renox::db::now()),
                ..SupplierScore::default()
            };
            snapshot.save(db).await?;
            made.push(snapshot);
        }
    }
    Ok(made)
}

/// The daily run: the snapshots missing today. Returns how many were made.
///
/// # Errors
///
/// A database error.
pub async fn run(state: &AppState) -> Result<usize> {
    Ok(snapshot_run(state, today()).await?.len())
}

/// Registers the daily [`TASK`].
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
}

/// `SupplierDebitNoteIssued`: keeps the debit note for the score, once.
///
/// # Errors
///
/// A database error.
pub(crate) async fn on_debit_note_issued(
    state: &AppState,
    event: &SupplierDebitNoteIssued,
) -> Result<()> {
    let kept = SupplierDebitNoteRecord::query()
        .where_(SupplierDebitNoteRecord::DEBIT_NOTE_ID.eq(event.debit_note.0))
        .first(&state.db)
        .await?;
    if kept.is_some() {
        return Ok(());
    }
    let mut row = SupplierDebitNoteRecord {
        debit_note_id: event.debit_note.0,
        supplier_party_id: event.supplier.0,
        amount: event.amount,
        issued_on: dubai_date(renox::db::now()),
        cancelled: false,
        ..SupplierDebitNoteRecord::default()
    };
    row.save(&state.db).await?;
    Ok(())
}

/// `SupplierDebitNoteCancelled`: the debit note leaves the live view.
///
/// # Errors
///
/// A database error.
pub(crate) async fn on_debit_note_cancelled(
    state: &AppState,
    event: &SupplierDebitNoteCancelled,
) -> Result<()> {
    let now = renox::db::now();
    SupplierDebitNoteRecord::query()
        .where_(SupplierDebitNoteRecord::DEBIT_NOTE_ID.eq(event.debit_note.0))
        .update(&state.db, &[("cancelled", &true), ("updated_at", &now)])
        .await?;
    Ok(())
}

/// Adds a review note to snapshot `score_id` (`purchase_orders.issue`). The
/// numbers of the snapshot do not change.
///
/// # Errors
///
/// `Forbidden` without the permission; 422 for an empty note.
pub async fn add_note(
    state: &AppState,
    user: &AuthUser,
    score_id: i64,
    note: &str,
) -> Result<SupplierScoreNote> {
    require(state, user, permissions::PURCHASE_ORDERS_ISSUE).await?;
    let snapshot = SupplierScore::find_or_404(&state.db, score_id).await?;
    let note = note.trim();
    if note.is_empty() {
        return Err(refused(said("wl_inventory.score.note_needed", &[])));
    }
    let mut row = SupplierScoreNote {
        supplier_score_id: snapshot.id,
        note: note.to_owned(),
        written_by: Some(user.id),
        ..SupplierScoreNote::default()
    };
    row.save(&state.db).await?;
    audit(
        state,
        Some(user.id),
        "inventory.supplier_score_noted",
        SupplierScore::TABLE,
        snapshot.id,
        json!({ "supplier": snapshot.supplier_party_id, "note": note }),
    )
    .await?;
    Ok(row)
}

/// The review notes of snapshot `score_id`, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn notes_of(db: &Db, score_id: i64) -> Result<Vec<SupplierScoreNote>> {
    SupplierScoreNote::query()
        .where_(SupplierScoreNote::SUPPLIER_SCORE_ID.eq(score_id))
        .order_by(SupplierScoreNote::ID)
        .get(db)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn figures(receipts: i64, on_time_bp: i64, rejected_bp: i64, price: Option<i64>) -> Figures {
        Figures {
            receipts,
            on_time_bp: Some(on_time_bp),
            rejected_bp: Some(rejected_bp),
            price_variance_bp: price,
            ..Figures::default()
        }
    }

    #[test]
    fn the_spec_example_scores_79_and_is_watch() {
        // 10 receipts, 8 on time, 2% rejected, 0 incidents, +5% price.
        let settings = ScoreSettings::default();
        let score = score_of(&figures(10, 8_000, 200, Some(500)), &settings);
        assert_eq!(score, Some(79));
        assert_eq!(band_of(score, &settings), ScoreBand::Watch);
    }

    #[test]
    fn a_component_without_data_drops_out_and_the_weights_spread() {
        // No price data: W1-W3 become 40/85, 30/85, 15/85 of 100.
        let settings = ScoreSettings::default();
        let mut f = figures(10, 10_000, 0, None);
        assert_eq!(score_of(&f, &settings), Some(100));
        f.on_time_bp = Some(8_000);
        // (40 x 0.8 + 30 + 15) / 85 = 0.9059
        assert_eq!(score_of(&f, &settings), Some(91));
    }

    #[test]
    fn fewer_receipts_than_the_minimum_leave_no_score() {
        let settings = ScoreSettings::default();
        let score = score_of(&figures(2, 10_000, 0, None), &settings);
        assert_eq!(score, None);
        assert_eq!(band_of(score, &settings), ScoreBand::NotEnoughData);
    }

    #[test]
    fn only_a_price_increase_lowers_the_score_and_the_caps_zero_a_component() {
        let settings = ScoreSettings::default();
        let down = score_of(&figures(10, 10_000, 0, Some(-800)), &settings);
        assert_eq!(down, Some(100));
        let capped = score_of(&figures(10, 10_000, 1_000, Some(1_000)), &settings);
        // Rejected and price components are gone: 40 + 15.
        assert_eq!(capped, Some(55));
        let mut many = figures(10, 10_000, 0, Some(0));
        many.incidents = 3;
        assert_eq!(score_of(&many, &settings), Some(85));
    }

    #[test]
    fn the_bands_follow_the_threshold() {
        let settings = ScoreSettings::default();
        assert_eq!(band_of(Some(85), &settings), ScoreBand::Good);
        assert_eq!(band_of(Some(84), &settings), ScoreBand::Watch);
        assert_eq!(band_of(Some(70), &settings), ScoreBand::Watch);
        assert_eq!(band_of(Some(69), &settings), ScoreBand::BelowThreshold);
    }

    #[test]
    fn a_ratio_is_rounded_half_up_in_basis_points() {
        assert_eq!(ratio_bp(8, 10), 8_000);
        assert_eq!(ratio_bp(1, 3), 3_333);
        assert_eq!(ratio_bp(2, 3), 6_667);
        assert_eq!(ratio_bp(-1, 5), -2_000);
        assert_eq!(ratio_bp(5, 0), 0);
    }

    #[test]
    fn the_snapshot_covers_the_three_months_before_its_day() {
        let day = NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();
        let (from, to) = snapshot_window(day, &ScoreSettings::default());
        assert_eq!(from, NaiveDate::from_ymd_opt(2026, 9, 1).unwrap());
        assert_eq!(to, NaiveDate::from_ymd_opt(2026, 11, 30).unwrap());
    }
}
