//! The monthly invoice of a client (03 Logistics §4.1 step 12, §7.2
//! `LOG.RUL.07`; BL-LOG-20).
//!
//! **The rule** ([`run_monthly`], on day 1 at 01:00 Asia/Dubai): for every
//! client with something to bill, [`draft_for`] makes one draft invoice
//! through the kernel's [`Invoicer`] (wl-invoicing's drafting, the same the
//! usage billing run of `FIN.USG.01` ends in; the hours-based usage records
//! of F2 do not fit a per-parcel price, so the lines are drafted directly
//! and the shipments and charge lines themselves are the "usage" that is
//! marked billed). It picks, cumulatively and never twice:
//!
//! - a **delivery line** for each shipment delivered before the run day (by
//!   `delivered_at`), for one returned to the client that was attempted at
//!   least once (by `returned_at`; a return before any attempt is billed the
//!   return charge only) and for one disposed of because of the client (by
//!   `disposed_at`);
//! - the **re-weigh surcharges** of those shipments that are not held by a
//!   ticket, also when the shipment was invoiced earlier (the issued invoice
//!   is never changed);
//! - the **return charge** of a shipment once it is `returned` (the month of
//!   the client's e-POD), the **disposal charge** of a client-caused
//!   disposal and the **cancellation fee** of a cancelled one.
//!
//! [`Shipment::billable`] leaves out what is voided, under review, lost,
//! reversed, on hold, cancelled or disposed for another cause; such a
//! shipment is simply picked up by the first draft after it is billable again.
//! A shipment's delivery line is billed once (`invoiced_on`), a charge line
//! once (`billed_on`), and a `logistics_billing_runs` row per client and
//! month makes running the rule twice leave one draft. The delivery price is
//! the shipment's, from the client's bands, the stand-in for the rate card
//! (`FIN.RAT.01`, see `DEFERRED-log.md`); `invoiced_on` is set when the draft
//! is made (a discarded draft frees nothing: the accountant corrects it).

use std::collections::{BTreeMap, BTreeSet};

use renox::auth::permissions::users_with_permission;
use renox::chrono::{Datelike, NaiveDate, TimeDelta};
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::invoicing::{self, DraftLine, InvoiceDraft, InvoiceId, VatCategory};
use wl_kernel::{PartyId, Subject};

use crate::notices::tell_user;
use crate::permissions;
use crate::rates::aed;
use crate::remit::period_end_of;
use crate::shipment::{Charge, Shipment, ShipmentStatus, TrackingEvent, charge_kind};
use crate::texts::english;

/// The scheduled task of `LOG.RUL.07`.
pub const TASK: &str = "logistics-monthly-invoices";

/// Day D of the rule (§7.2).
pub const DAY: u32 = 1;

/// The time the rule runs on day D, Asia/Dubai.
pub const TASK_AT: &str = "01:00";

/// The subject type of a billing run, the source of its draft invoice.
pub const RUN_SUBJECT_TYPE: &str = "logistics.billing_run";

/// One client's draft for one month: the key that keeps the rule from
/// drafting twice.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_billing_runs", unique(client_party_id, period))]
pub struct BillingRun {
    /// The primary key.
    pub id: i64,
    /// The client.
    pub client_party_id: i64,
    /// The month billed, `2026-10`.
    pub period: String,
    /// The draft invoice.
    pub invoice_id: Option<i64>,
    /// How many lines it has.
    pub line_count: i64,
    /// Their net total, in fils.
    pub net_total: i64,
    /// Return charges billed at AED 0 (no return price for the client).
    pub unpriced_returns: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl BillingRun {
    /// The run's subject.
    pub fn subject(&self) -> Subject {
        Subject::new(RUN_SUBJECT_TYPE, self.id)
    }
}

/// What one run drafted for one client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monthly {
    /// The client.
    pub client: PartyId,
    /// The month billed, `2026-10`.
    pub period: String,
    /// The draft invoice.
    pub invoice: InvoiceId,
    /// The billing run row.
    pub run_id: i64,
    /// The lines on the draft.
    pub lines: usize,
    /// Their net total, in fils.
    pub net: i64,
}

/// The re-weigh surcharges of `client` still to be billed on its next draft:
/// not on an invoice yet and not held by an open re-weigh ticket. A surcharge
/// recorded after its shipment was invoiced is among them; it goes on the
/// next draft and never changes the issued invoice.
///
/// # Errors
///
/// Database errors.
pub async fn unbilled_surcharges(db: &Db, client: PartyId) -> Result<Vec<Charge>> {
    Charge::query()
        .where_(Charge::CLIENT_PARTY_ID.eq(client.0))
        .where_(Charge::KIND.eq(charge_kind::REWEIGH))
        .where_(Charge::HELD.eq(false))
        .where_(Charge::BILLED_ON.is_null())
        .order_by(Charge::ID)
        .get(db)
        .await
}

/// The disposal charges of `client` still to be billed on its next draft:
/// the shipments destroyed because of the client (`LOG.RUL.07`, BL-LOG-06),
/// not on an invoice yet.
///
/// # Errors
///
/// Database errors.
pub async fn unbilled_disposals(db: &Db, client: PartyId) -> Result<Vec<Charge>> {
    Charge::query()
        .where_(Charge::CLIENT_PARTY_ID.eq(client.0))
        .where_(Charge::KIND.eq(charge_kind::DISPOSAL))
        .where_(Charge::HELD.eq(false))
        .where_(Charge::BILLED_ON.is_null())
        .order_by(Charge::ID)
        .get(db)
        .await
}

/// Marks the charge lines `ids` as billed `at` (the draft they went on).
///
/// # Errors
///
/// Database errors.
pub async fn mark_billed(db: &Db, ids: &[i64], at: DateTime) -> Result {
    let at = Some(at);
    for id in ids {
        Charge::query()
            .where_(Charge::ID.eq(*id))
            .update(db, &[("billed_on", &at)])
            .await?;
    }
    Ok(())
}

/// Marks shipment `id` as on an issued invoice `at`.
///
/// # Errors
///
/// Database errors.
pub async fn mark_shipment_invoiced(db: &Db, id: i64, at: DateTime) -> Result {
    let at = Some(at);
    Shipment::query()
        .where_(Shipment::ID.eq(id))
        .update(db, &[("invoiced_on", &at)])
        .await?;
    Ok(())
}


/// The first day of the month of `day`.
fn month_start(day: NaiveDate) -> NaiveDate {
    day.with_day(1).unwrap_or(day)
}

/// The month a run on `day` bills: the previous calendar month, `2026-10` for
/// a run on 2026-11-01.
pub fn period_of(day: NaiveDate) -> String {
    (month_start(day) - TimeDelta::days(1))
        .format("%Y-%m")
        .to_string()
}

/// When `shipment` becomes billable: the delivery, the e-POD of the return or
/// the disposal; none while it has not happened.
fn billing_date(shipment: &Shipment) -> Option<DateTime> {
    match shipment.status {
        ShipmentStatus::Delivered => shipment.delivered_at,
        ShipmentStatus::Returned => shipment.returned_at,
        ShipmentStatus::Disposed => shipment.disposed_at,
        _ => None,
    }
}

/// One line of the draft, with what it marks billed.
struct Entry {
    shipment_id: i64,
    order: u8,
    line: DraftLine,
    charge_id: Option<i64>,
    shipment_billed: bool,
    unpriced_return: bool,
}

fn with_note(text: String, note: &str) -> String {
    if note.trim().is_empty() {
        text
    } else {
        format!("{text} ({})", note.trim())
    }
}

/// Whether the shipment was attempted at least once: a failed visit counts,
/// except the one stopped with the reason *cancelled by client*.
async fn attempted(db: &Db, shipment_id: i64) -> Result<bool> {
    let count = TrackingEvent::query()
        .where_(TrackingEvent::SHIPMENT_ID.eq(shipment_id))
        .where_(TrackingEvent::KIND.eq("attempt_failed"))
        .where_(TrackingEvent::NOTE.ne("cancelled_by_client"))
        .count(db)
        .await?;
    Ok(count > 0)
}

/// The lines `client` is billed for in the run that ends at `period_end`.
async fn entries(db: &Db, client: PartyId, period_end: DateTime) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut by_id = BTreeMap::new();
    for shipment in Shipment::billable(db, client).await? {
        by_id.insert(shipment.id, shipment);
    }
    for shipment in by_id.values() {
        if !billing_date(shipment).is_some_and(|at| at < period_end) {
            continue;
        }
        let delivery = match shipment.status {
            ShipmentStatus::Delivered | ShipmentStatus::Disposed => true,
            ShipmentStatus::Returned => attempted(db, shipment.id).await?,
            _ => false,
        };
        if delivery && shipment.invoiced_on.is_none() {
            entries.push(Entry {
                shipment_id: shipment.id,
                order: 0,
                line: DraftLine::new(
                    &english("logistics.billing.delivery", &[("number", &shipment.number)]),
                    shipment.price,
                    VatCategory::Standard,
                )
                .origin(shipment.subject()),
                charge_id: None,
                shipment_billed: true,
                unpriced_return: false,
            });
        }
    }
    let charges = Charge::query()
        .where_(Charge::CLIENT_PARTY_ID.eq(client.0))
        .where_(Charge::HELD.eq(false))
        .where_(Charge::BILLED_ON.is_null())
        .order_by(Charge::ID)
        .get(db)
        .await?;
    let mut cancelled = BTreeMap::new();
    for charge in charges {
        let (key, order, shipment) = match charge.kind.as_str() {
            charge_kind::CANCELLATION => {
                // The fee of a shipment cancelled before pickup (it has no
                // delivery, so it is not among the billable ones).
                if !cancelled.contains_key(&charge.shipment_id) {
                    let found = Shipment::find(db, charge.shipment_id).await?;
                    cancelled.insert(charge.shipment_id, found);
                }
                let found = cancelled.get(&charge.shipment_id).cloned().flatten();
                match found {
                    Some(shipment)
                        if shipment.status == ShipmentStatus::Cancelled
                            && charge.recorded_at < period_end =>
                    {
                        ("logistics.billing.cancellation", 4, shipment)
                    }
                    _ => continue,
                }
            }
            charge_kind::REWEIGH | charge_kind::RETURN | charge_kind::DISPOSAL => {
                let Some(shipment) = by_id.get(&charge.shipment_id) else {
                    continue;
                };
                if !billing_date(shipment).is_some_and(|at| at < period_end) {
                    continue;
                }
                // A return charge waits for the e-POD; a disposal charge for
                // the disposal (the billing date is both).
                match (charge.kind.as_str(), shipment.status) {
                    (charge_kind::RETURN, ShipmentStatus::Returned) => {
                        ("logistics.billing.return", 2, shipment.clone())
                    }
                    (charge_kind::DISPOSAL, ShipmentStatus::Disposed) => {
                        ("logistics.billing.disposal", 3, shipment.clone())
                    }
                    (charge_kind::REWEIGH, _) => ("logistics.billing.reweigh", 1, shipment.clone()),
                    _ => continue,
                }
            }
            _ => continue,
        };
        let text = with_note(
            english(key, &[("number", &shipment.number)]),
            &charge.note,
        );
        entries.push(Entry {
            shipment_id: shipment.id,
            order,
            line: DraftLine::new(&text, charge.amount, VatCategory::Standard)
                .origin(Subject::new("logistics.charge", charge.id)),
            charge_id: Some(charge.id),
            shipment_billed: false,
            unpriced_return: charge.kind == charge_kind::RETURN && charge.amount == 0,
        });
    }
    entries.sort_by_key(|entry| (entry.shipment_id, entry.order));
    Ok(entries)
}

/// `LOG.RUL.07` for one client on `day` (the first of the month): drafts the
/// invoice of the previous month's billable shipments and charges, once.
/// `None` when the month was drafted already, when there is nothing to bill,
/// or when invoicing is off (nothing is then marked billed).
///
/// # Errors
///
/// Database errors and the invoicer's.
pub async fn draft_for(
    state: &AppState,
    client: PartyId,
    day: NaiveDate,
) -> Result<Option<Monthly>> {
    let period = period_of(day);
    let done = BillingRun::query()
        .where_(BillingRun::CLIENT_PARTY_ID.eq(client.0))
        .where_(BillingRun::PERIOD.eq(period.as_str()))
        .first(&state.db)
        .await?;
    if done.is_some() {
        return Ok(None);
    }
    let period_end = period_end_of(month_start(day));
    let entries = entries(&state.db, client, period_end).await?;
    if entries.is_empty() {
        return Ok(None);
    }
    let mut run = BillingRun {
        client_party_id: client.0,
        period: period.clone(),
        ..BillingRun::default()
    };
    run.insert(&state.db).await?;
    let mut draft = InvoiceDraft::new(client, run.subject()).date_of_supply(month_start(day) - TimeDelta::days(1));
    for entry in &entries {
        draft = draft.line(entry.line.clone());
    }
    let net: i64 = entries.iter().map(|entry| entry.line.net()).sum();
    let Some(invoice) = invoicing::invoicer(state).draft(&state.db, draft).await? else {
        BillingRun::query()
            .where_(BillingRun::ID.eq(run.id))
            .delete(&state.db)
            .await?;
        return Ok(None);
    };
    let at = renox::db::now();
    let charge_ids: Vec<i64> = entries.iter().filter_map(|entry| entry.charge_id).collect();
    mark_billed(&state.db, &charge_ids, at).await?;
    let shipments: BTreeSet<i64> = entries
        .iter()
        .filter(|entry| entry.shipment_billed)
        .map(|entry| entry.shipment_id)
        .collect();
    for id in shipments {
        mark_shipment_invoiced(&state.db, id, at).await?;
    }
    let unpriced = i64::try_from(entries.iter().filter(|e| e.unpriced_return).count())
        .unwrap_or(i64::MAX);
    let count = i64::try_from(entries.len()).unwrap_or(i64::MAX);
    let invoice_id = Some(invoice.0);
    BillingRun::query()
        .where_(BillingRun::ID.eq(run.id))
        .update(
            &state.db,
            &[
                ("invoice_id", &invoice_id),
                ("line_count", &count),
                ("net_total", &net),
                ("unpriced_returns", &unpriced),
            ],
        )
        .await?;
    tell_drafted(state, &run, client, &period, count, net, unpriced).await?;
    Ok(Some(Monthly {
        client,
        period,
        invoice,
        run_id: run.id,
        lines: entries.len(),
        net,
    }))
}

/// Tells the accountants (holders of `remittances.prepare`) that the monthly
/// draft of `client` is ready, and which return charges have no price.
async fn tell_drafted(
    state: &AppState,
    run: &BillingRun,
    client: PartyId,
    period: &str,
    lines: i64,
    net: i64,
    unpriced: i64,
) -> Result {
    let title = english(
        "logistics.billing.notice_title",
        &[("period", &period), ("client", &client.0)],
    );
    let mut body = english(
        "logistics.billing.notice_body",
        &[("lines", &lines), ("net", &aed(net))],
    );
    if unpriced > 0 {
        body.push(' ');
        body.push_str(&english(
            "logistics.billing.notice_unpriced",
            &[("count", &unpriced), ("client", &client.0)],
        ));
    }
    for user in users_with_permission(&state.db, permissions::REMITTANCES_PREPARE).await? {
        tell_user(state, run.subject(), period, user.id, &title, &body).await?;
    }
    Ok(())
}

/// `LOG.RUL.07` on `day`: the draft invoice of every client that has something
/// to bill for the previous month. Running it again leaves one draft per
/// client.
///
/// # Errors
///
/// Database errors and the invoicer's.
pub async fn run_monthly(state: &AppState, day: NaiveDate) -> Result<Vec<Monthly>> {
    let mut clients = BTreeSet::new();
    for status in [
        ShipmentStatus::Delivered,
        ShipmentStatus::Returned,
        ShipmentStatus::Disposed,
    ] {
        let shipments = Shipment::query()
            .where_(Shipment::STATUS.eq(status))
            .where_(Shipment::INVOICED_ON.is_null())
            .get(&state.db)
            .await?;
        clients.extend(shipments.iter().map(|s| s.client_party_id));
    }
    let charges = Charge::query()
        .where_(Charge::BILLED_ON.is_null())
        .where_(Charge::HELD.eq(false))
        .get(&state.db)
        .await?;
    clients.extend(charges.iter().map(|c| c.client_party_id));
    let mut drafted = Vec::new();
    for client in clients {
        if let Some(one) = draft_for(state, PartyId(client), day).await? {
            drafted.push(one);
        }
    }
    Ok(drafted)
}

/// Registers the monthly rule on the scheduler.
pub(crate) fn register(app: &mut Registry) {
    app.schedule()
        .monthly_on(DAY, TASK_AT, TASK, |state: AppState| async move {
            run_monthly(&state, wl_kernel::documents::today())
                .await
                .map(|_| ())
        })
        .timezone("Asia/Dubai");
}
