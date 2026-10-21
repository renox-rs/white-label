//! The lot of every consumable issued to a visit (05 §4.6 step 2,
//! `STK.MOV.01`, `STK.LOT.01`; BL-CLN-13).
//!
//! The desk records a consumable on the open visit ([`crate::charges`]). When
//! the item is tracked by lot, the line names a lot the desk picked or
//! scanned; [`proposal`] is the lot with the earliest expiry among those at
//! the branch store that are released, not held, not expired and have a
//! balance. A lot that is not among those is refused ([`pick`]): a held or
//! expired lot is never issued. [`assign`] sets or corrects the lot of a line
//! until the visit is checked out.
//!
//! The visit is **checked out** when its booking is completed:
//! [`LotGuard`] (the app's [`CompleteGuard`]) refuses that while a lot-tracked
//! consumable line has no lot, except when the store has no lot to offer at
//! all (the whole quantity then becomes a pending issue). Right after the
//! check-out [`issue`] takes the stock from the branch store through the
//! kernel's `StockService`: what is there posts, first expiry first out (the
//! lot of the line first), a quantity larger than one lot is split into one
//! row per lot, and only the short quantity becomes a pending issue
//! (*lot uncertain* until stock arrives). A visit cancelled after the check-in
//! issues the same way; a line without a lot then takes first expiry first.
//!
//! Decisions:
//!
//! - The stock is issued when the visit ends, not when the line is entered,
//!   so a wrong lot is edited as the line, never as a posted movement. After
//!   the check-out a correction is a reversing movement and a new issue in
//!   wl-inventory (audited there); the lot rows of the visit are not edited.
//! - The branch store is the store of the scope of the visit's booking
//!   (`StockService::store_of`); a consumable whose item or store is unknown
//!   to wl-inventory is a charge only, nothing is issued (a clinic without the
//!   inventory module).
//! - One unit of a line is one base unit of the item (1000 thousandths).

use renox::db::{DateTime, now};
use renox::prelude::*;
use wl_kernel::inventory::{
    ConsumeRequest, LOT_SUBJECT_TYPE, StockChoice, StockLot, stock_service,
};
use wl_kernel::scheduling::{
    BOOKING_SUBJECT_TYPE, BookingId, CheckInAnswer, CompleteGuard, SchedulingFuture,
};
use wl_kernel::{Subject, scopes};

use crate::charge::{ChargeKind, ChargeLine};
use crate::consumable_issue::ConsumableIssue;
use crate::db::refused;
use crate::texts::english;
use crate::visit::Visit;

/// What the branch store holds of an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stock {
    /// The branch store, `inventory.location:2`.
    pub store: Subject,
    /// The item and the lots to take from, first expiry first.
    pub choice: StockChoice,
}

/// The branch store of a visit: the store of the scope of its booking.
///
/// # Errors
///
/// The database's and the inventory's errors.
pub async fn store_of(state: &AppState, visit: &Visit) -> Result<Option<Subject>> {
    let booking = Subject::new(BOOKING_SUBJECT_TYPE, visit.booking_id);
    let scope = scopes::scope_of(&state.db, &booking).await?;
    if scope.is_global() {
        return Ok(None);
    }
    let Ok(id) = scope.id().parse::<i64>() else {
        return Ok(None);
    };
    stock_service(state).store_of(state, scope.kind(), id).await
}

/// What the branch store of `visit` holds of the item `code`; `None` when the
/// visit has no store or wl-inventory does not know the item.
///
/// # Errors
///
/// The database's and the inventory's errors.
pub async fn stock_of(state: &AppState, visit: &Visit, code: &str) -> Result<Option<Stock>> {
    let Some(store) = store_of(state, visit).await? else {
        return Ok(None);
    };
    let choice = stock_service(state).choices(state, code, &store).await?;
    Ok(choice.map(|choice| Stock { store, choice }))
}

/// The lot the issue screen proposes for the consumable `code`: the earliest
/// expiry at the branch store. `None` when nothing is available or the item
/// is not tracked by lot.
///
/// # Errors
///
/// The database's and the inventory's errors.
pub async fn proposal(state: &AppState, visit: &Visit, code: &str) -> Result<Option<StockLot>> {
    Ok(stock_of(state, visit, code)
        .await?
        .and_then(|stock| stock.choice.lots.first().cloned()))
}

/// The lot `number` scanned or picked, when it can be issued.
///
/// # Errors
///
/// A 422 for a lot that is unknown at the store, on hold or expired.
pub fn pick(stock: &Stock, number: &str) -> Result<StockLot> {
    let wanted = number.trim();
    stock
        .choice
        .lots
        .iter()
        .find(|lot| lot.number.eq_ignore_ascii_case(wanted))
        .cloned()
        .ok_or_else(|| {
            refused(english(
                "clinics.lots.problem.not_available",
                &[("lot", &wanted)],
            ))
        })
}

/// Sets the lot of the consumable line `line` of the open visit `visit` to
/// the lot `number`, or to the proposed one when `number` is blank. Used to
/// give a line its lot and to correct a wrong one before the check-out.
///
/// # Errors
///
/// A 422 for a line that is not a lot-tracked consumable of the visit, one
/// issued already, a lot that cannot be issued, or no lot to propose.
pub async fn assign(
    state: &AppState,
    visit: i64,
    line: i64,
    number: Option<&str>,
) -> Result<ChargeLine> {
    let visit = crate::charges::open_visit(state, visit).await?;
    let line = ChargeLine::find_or_404(&state.db, line).await?;
    if line.visit_id != visit.id
        || line.kind != ChargeKind::Consumable
        || !line.is_active()
        || !line.lot_tracked
    {
        return Err(refused(english("clinics.lots.problem.no_line", &[])));
    }
    if line.issued_at.is_some() {
        return Err(refused(english("clinics.lots.problem.issued", &[])));
    }
    let stock = stock_of(state, &visit, &line.code)
        .await?
        .ok_or_else(|| refused(english("clinics.lots.problem.none_available", &[])))?;
    let lot = match number.map(str::trim).filter(|n| !n.is_empty()) {
        Some(number) => pick(&stock, number)?,
        None => stock
            .choice
            .lots
            .first()
            .cloned()
            .ok_or_else(|| refused(english("clinics.lots.problem.none_available", &[])))?,
    };
    ChargeLine::query()
        .where_(ChargeLine::ID.eq(line.id))
        .update(
            &state.db,
            &[
                ("lot_id", &Some(lot.lot.subject_id)),
                ("lot_number", &Some(lot.number.clone())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    ChargeLine::find_or_404(&state.db, line.id).await
}

/// The lot-tracked consumable lines of `visit` that have no lot although the
/// branch store has lots to offer: the check-out waits for them.
///
/// # Errors
///
/// The database's and the inventory's errors.
pub async fn missing(state: &AppState, visit: &Visit) -> Result<Vec<ChargeLine>> {
    let mut lines = Vec::new();
    for line in ChargeLine::active_of(&state.db, visit.id).await? {
        if line.kind != ChargeKind::Consumable
            || !line.lot_tracked
            || line.lot_id.is_some()
            || line.issued_at.is_some()
        {
            continue;
        }
        // No lot at the store: the quantity becomes a pending issue.
        let has_lots = stock_of(state, visit, &line.code)
            .await?
            .is_some_and(|stock| !stock.choice.lots.is_empty());
        if has_lots {
            lines.push(line);
        }
    }
    Ok(lines)
}

/// The lot-tracked consumable lines of `visit` whose lot is no longer among
/// those the branch store offers (put on hold, expired or used up after it was
/// assigned) while the store still has other lots to offer: the check-out
/// waits for the desk to pick again, instead of the stock silently leaving
/// with another lot than the one the line shows.
///
/// Decision: when the store has no lot left at all the quantity becomes a
/// pending issue, as for a line without a lot, so nothing is refused.
///
/// # Errors
///
/// The database's and the inventory's errors.
pub async fn no_longer_offered(state: &AppState, visit: &Visit) -> Result<Vec<ChargeLine>> {
    let mut lines = Vec::new();
    for line in ChargeLine::active_of(&state.db, visit.id).await? {
        let Some(wanted) = line.lot_id else { continue };
        if line.kind != ChargeKind::Consumable || !line.lot_tracked || line.issued_at.is_some() {
            continue;
        }
        let Some(stock) = stock_of(state, visit, &line.code).await? else {
            continue;
        };
        let offered = stock
            .choice
            .lots
            .iter()
            .any(|lot| lot.lot.subject_id == wanted);
        if !offered && !stock.choice.lots.is_empty() {
            lines.push(line);
        }
    }
    Ok(lines)
}

/// The codes of `lines`, comma separated.
fn codes_of(lines: &[ChargeLine]) -> String {
    lines
        .iter()
        .map(|line| line.code.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The app's [`CompleteGuard`]: the check-out of a visit with a lot-tracked
/// consumable that has no lot, or whose lot is no longer offered, is refused
/// with "lot required" / "lot no longer available".
#[derive(Debug, Clone, Copy, Default)]
pub struct LotGuard;

impl CompleteGuard for LotGuard {
    fn check<'a>(
        &'a self,
        state: &'a AppState,
        booking: BookingId,
        _at: DateTime,
    ) -> SchedulingFuture<'a, CheckInAnswer> {
        Box::pin(async move {
            let Some(visit) = Visit::of_booking(&state.db, booking.0).await? else {
                return Ok(CheckInAnswer::Allowed);
            };
            let lines = missing(state, &visit).await?;
            if !lines.is_empty() {
                let codes = codes_of(&lines);
                return Ok(CheckInAnswer::Refused(english(
                    "clinics.lots.problem.required",
                    &[("codes", &codes)],
                )));
            }
            let stale = no_longer_offered(state, &visit).await?;
            if stale.is_empty() {
                return Ok(CheckInAnswer::Allowed);
            }
            let codes = codes_of(&stale);
            Ok(CheckInAnswer::Refused(english(
                "clinics.lots.problem.not_offered",
                &[("codes", &codes)],
            )))
        })
    }
}

/// Issues the consumables of the visit `visit_id` from the branch store (see
/// the module docs). A line already issued is left alone, so a second call
/// issues nothing twice. Returns the rows it made.
///
/// # Errors
///
/// The database's and the inventory's errors. A line whose issue failed is
/// not marked issued and is issued by the next call.
pub async fn issue(state: &AppState, visit_id: i64) -> Result<Vec<ConsumableIssue>> {
    Ok(run(state, visit_id).await?.made)
}

/// What one run of [`issue`] did.
#[derive(Debug, Default)]
struct Run {
    /// The rows it made.
    made: Vec<ConsumableIssue>,
    /// The lot-tracked lines it could not place because the branch has no
    /// store or the store does not know the item any more.
    unplaced: Vec<String>,
}

async fn run(state: &AppState, visit_id: i64) -> Result<Run> {
    let visit = Visit::find_or_404(&state.db, visit_id).await?;
    let mut made = Vec::new();
    let mut unplaced = Vec::new();
    for line in ChargeLine::active_of(&state.db, visit.id).await? {
        if line.kind != ChargeKind::Consumable || line.issued_at.is_some() {
            continue;
        }
        let Some(stock) = stock_of(state, &visit, &line.code).await? else {
            // A line that was lot-tracked when it was entered and has no store
            // or item now is not a plain charge: the desk is told.
            if line.lot_tracked {
                unplaced.push(line.code.clone());
            }
            continue;
        };
        // One run issues a line: the claim is undone when the stock fails.
        let claimed = ChargeLine::query()
            .where_(ChargeLine::ID.eq(line.id))
            .where_(ChargeLine::ISSUED_AT.is_null())
            .update(
                &state.db,
                &[("issued_at", &Some(now())), ("updated_at", &now())],
            )
            .await?;
        if claimed == 0 {
            continue;
        }
        let source = visit.subject();
        let mut request = ConsumeRequest::new(
            stock.choice.item.clone(),
            stock.store.clone(),
            line.units * 1_000,
            source.clone(),
        )
        .cost_object(source);
        if let Some(lot) = line.lot_id {
            request = request.lot(Subject::new(LOT_SUBJECT_TYPE, lot));
        }
        let consumed = match stock_service(state).consume(state, request).await {
            Ok(consumed) => consumed,
            Err(error) => {
                let none: Option<DateTime> = None;
                ChargeLine::query()
                    .where_(ChargeLine::ID.eq(line.id))
                    .update(&state.db, &[("issued_at", &none)])
                    .await?;
                return Err(error);
            }
        };
        let issued_at = now();
        for part in &consumed.lots {
            let lot = part.lot.as_ref().and_then(|subject| {
                stock
                    .choice
                    .lots
                    .iter()
                    .find(|found| found.lot.subject_id == subject.subject_id)
            });
            let mut row = ConsumableIssue {
                visit_id: visit.id,
                charge_line_id: line.id,
                item_id: stock.choice.item.subject_id,
                location_id: stock.store.subject_id,
                lot_id: part.lot.as_ref().map(|subject| subject.subject_id),
                lot_number: lot.map(|found| found.number.clone()),
                expiry_date: lot.and_then(|found| found.expiry_date),
                quantity_milli: part.quantity_milli,
                issued_at: Some(issued_at),
                ..ConsumableIssue::default()
            };
            row.save(&state.db).await?;
            made.push(row);
        }
        if consumed.short_milli > 0 {
            let mut row = ConsumableIssue {
                visit_id: visit.id,
                charge_line_id: line.id,
                item_id: stock.choice.item.subject_id,
                location_id: stock.store.subject_id,
                quantity_milli: consumed.short_milli,
                pending_issue_id: consumed.pending_issue.as_ref().map(|s| s.subject_id),
                issued_at: Some(issued_at),
                ..ConsumableIssue::default()
            };
            row.save(&state.db).await?;
            made.push(row);
        }
    }
    Ok(Run { made, unplaced })
}

/// The reason a visit's issue is pending when the store or the item is not
/// known.
pub const UNKNOWN_STORE: &str = "unknown_store";

/// Issues the consumables of the visit that just ended without ever failing
/// the booking's check-out or the bill: a run that stops on an error, or
/// that cannot place a lot-tracked line, **flags** the visit
/// (`issue_pending_at` and `issue_pending_reason`, shown to the desk) and
/// [`retry`] or the daily [`retry_pending`] issues it later. A run that
/// finishes clears the flag. Returns the rows made.
///
/// # Errors
///
/// The database's errors (not the stock's: those are kept on the visit).
pub async fn issue_or_flag(state: &AppState, visit_id: i64) -> Result<Vec<ConsumableIssue>> {
    let (made, reason) = match run(state, visit_id).await {
        Ok(done) if done.unplaced.is_empty() => (done.made, None),
        Ok(done) => (done.made, Some(UNKNOWN_STORE.to_owned())),
        Err(error) => {
            (Vec::new(), Some(format!("stock: {error}")))
        }
    };
    let (at, why) = match &reason {
        Some(reason) => (Some(now()), Some(reason.clone())),
        None => (None, None),
    };
    Visit::query()
        .where_(Visit::ID.eq(visit_id))
        .update(
            &state.db,
            &[
                ("issue_pending_at", &at),
                ("issue_pending_reason", &why),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(made)
}

/// The desk's retry of the stock issue of a visit that is over
/// (`POST /clinics/visits/{id}/issue`): idempotent, a line issued already is
/// not issued again.
///
/// # Errors
///
/// A 422 for a visit that is still open.
pub async fn retry(state: &AppState, visit_id: i64) -> Result<Vec<ConsumableIssue>> {
    let visit = Visit::find_or_404(&state.db, visit_id).await?;
    if visit.status.is_open() {
        return Err(refused(english("clinics.lots.problem.open", &[])));
    }
    issue_or_flag(state, visit_id).await
}

/// The daily retry: every visit that is over and still flagged. Returns how
/// many visits are clear afterwards.
///
/// # Errors
///
/// The database's errors.
pub async fn retry_pending(state: &AppState) -> Result<usize> {
    let mut cleared = 0;
    for visit in Visit::query()
        .where_(Visit::ISSUE_PENDING_AT.is_not_null())
        .get(&state.db)
        .await?
    {
        if visit.status.is_open() {
            continue;
        }
        issue_or_flag(state, visit.id).await?;
        let after = Visit::find_or_404(&state.db, visit.id).await?;
        if after.issue_pending_at.is_none() {
            cleared += 1;
        }
    }
    Ok(cleared)
}

/// The scheduled task, and the command, of the daily retry.
pub const TASK: &str = "clinics:consumable_issues";

/// When the daily retry runs (Asia/Dubai).
pub const TASK_AT: &str = "03:00";

/// Registers the daily retry of the stock issues that did not go through.
pub(crate) fn register(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            retry_pending(&state).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
    app.command(
        TASK,
        "Issue again the consumables of finished visits whose stock issue did not go through (STK.MOV.01)",
        |_args, state: AppState| async move {
            let cleared = retry_pending(&state).await?;
            println!("{cleared} visits are clear of a pending stock issue.");
            Ok(())
        },
    );
}
