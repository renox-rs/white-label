//! The daily closing services (06 F&B §4.6 step 4, §7.1 *Daily closing*, §7.2;
//! BL-FNB-15).
//!
//! - **Business day** ([`ensure`], [`totals`]): an order carries the business
//!   date its creation time gives by the outlet's cut-off
//!   ([`crate::catalogue::Outlet::business_date`]); the closing of an outlet and
//!   date adds up its orders and its paper orders. A cut-off change applies from
//!   the next business day only ([`set_cutoff`]).
//! - **Submit** ([`submit`]): refused until the cut-off of the next morning has
//!   passed (with the time it opens), without the closing checklist and without
//!   the cash counted. The cash is counted in the wl-sales till session of the
//!   outlet's till (opened here when nobody opened it): the session's `close`
//!   emits `TillSessionClosed`, which makes the till's cash handover in
//!   wl-payments, so a shortage is recovered from the cashier or written off
//!   (`FIN.ACL.06`).
//! - **Approve / reopen** ([`approve`], [`reopen`]): the area manager (or the
//!   accountant) approves; only the area manager reopens, with a reason, and the
//!   till session goes back to `open` with it. A late entry into the submitted
//!   closing reopens it the same way once approved ([`on_late_entry`]).
//!
//! Decisions: the closing checklist (`WRK.CHK.01`) is not built in wl-ticketing,
//! so the screen asks the person to confirm it; the till is run as the system
//! after this service checked `closing.submit` (the kitchen manager has no till
//! permission of the sales module); a missing cashier on a closing the system
//! submits is an error, the person who submits is the cashier otherwise.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use serde::Serialize;
use wl_kernel::late_entries::LateEntryRecorded;
use wl_kernel::sales::{OpenTill, TillView, till_desk};

use crate::business_day::{clock_text, parse_clock};
use crate::catalogue::Outlet;
use crate::closing::{
    APPROVE, APPROVE_SYSTEM, ClosingInput, ClosingStatus, DailyClosing, REOPEN, SUBMIT,
    SUBMIT_SYSTEM,
};
use crate::notices::tell_holders;
use crate::order::{FnbOrder, FnbOrderStatus};
use crate::paper_order::{PaperOrder, is_paper_subject};
use crate::permissions;
use crate::placing::refused;
use crate::texts::english;

fn require(user: &AuthUser, permission: &str) -> Result {
    if user.has_permission(permission) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// What one channel sold on a business date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelTotal {
    /// The channel.
    pub channel: String,
    /// How many orders (paper orders included).
    pub orders: i64,
    /// The sales with VAT, in fils.
    pub gross: i64,
}

/// What a closing adds up.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Totals {
    /// Orders of the business date, paper orders included.
    pub orders: i64,
    /// Their sales with VAT, in fils.
    pub gross: i64,
    /// The paper orders among them.
    pub paper_orders: i64,
    /// Those entered late.
    pub late_entries: i64,
    /// Sales by channel, largest first.
    pub channels: Vec<ChannelTotal>,
}

/// Adds an order of `gross` fils on `channel` to the totals.
fn add_to(totals: &mut Totals, channel: &str, gross: i64) {
    totals.orders += 1;
    totals.gross += gross;
    match totals.channels.iter_mut().find(|c| c.channel == channel) {
        Some(found) => {
            found.orders += 1;
            found.gross += gross;
        }
        None => totals.channels.push(ChannelTotal {
            channel: channel.to_owned(),
            orders: 1,
            gross,
        }),
    }
}

/// The closing of `outlet_id` for `date`, made `open` when there is none.
///
/// # Errors
///
/// Database errors.
pub async fn ensure(db: &Db, outlet_id: i64, date: NaiveDate) -> Result<DailyClosing> {
    if let Some(found) = DailyClosing::of_day(db, outlet_id, date).await? {
        return Ok(found);
    }
    let mut closing = DailyClosing {
        outlet_id,
        business_date: Some(date),
        ..DailyClosing::default()
    };
    let mut tx = db.begin().await?;
    match closing
        .insert_in_state(
            &mut tx,
            ClosingStatus::Open,
            &english("fnb.closing.history.opened", &[]),
        )
        .await
    {
        Ok(_) => {
            tx.commit().await?;
            Ok(closing)
        }
        Err(error) if error.is_unique_violation() => {
            drop(tx);
            DailyClosing::of_day(db, outlet_id, date)
                .await?
                .ok_or(Error::NotFound)
        }
        Err(error) => Err(error),
    }
}

/// The closing of the outlet's business date now.
///
/// # Errors
///
/// 403 without `closing.submit`; 404 for an unknown outlet.
pub async fn open_today(state: &AppState, user: &AuthUser, outlet_id: i64) -> Result<DailyClosing> {
    require(user, permissions::CLOSING_SUBMIT)?;
    let outlet = Outlet::find_or_404(&state.db, outlet_id).await?;
    ensure(&state.db, outlet.id, outlet.business_date(renox::db::now())).await
}

/// What the closing adds up: its orders (not the cancelled or rejected ones)
/// and its paper orders.
///
/// # Errors
///
/// Database errors.
pub async fn totals(db: &Db, closing: &DailyClosing) -> Result<Totals> {
    let Some(date) = closing.business_date else {
        return Ok(Totals::default());
    };
    let orders = FnbOrder::query()
        .where_(FnbOrder::OUTLET_ID.eq(closing.outlet_id))
        .where_(FnbOrder::BUSINESS_DATE.eq(date))
        .get(db)
        .await?;
    let mut totals = Totals::default();
    for order in orders.iter().filter(|o| {
        !matches!(
            o.status,
            FnbOrderStatus::Cancelled | FnbOrderStatus::Rejected
        )
    }) {
        add_to(&mut totals, &order.channel, order.total);
    }
    let paper = PaperOrder::of_day(db, closing.outlet_id, date).await?;
    totals.paper_orders = paper.len() as i64;
    totals.late_entries = paper.iter().filter(|p| p.late).count() as i64;
    for order in &paper {
        add_to(&mut totals, &order.channel, order.amount);
    }
    totals.channels.sort_by(|a, b| b.gross.cmp(&a.gross));
    Ok(totals)
}

/// When the closing of `closing` may be submitted, in words.
pub fn opens_text(outlet: &Outlet, date: NaiveDate) -> String {
    let next = date.succ_opt().unwrap_or(date);
    english(
        "fnb.closing.opens",
        &[
            ("time", &clock_text(outlet.cutoff_for(date))),
            ("date", &next),
        ],
    )
}

/// What the person enters to submit.
#[derive(Debug, Clone, Default)]
pub struct SubmitAsk {
    /// The cash counted in the till, in fils.
    pub counted: i64,
    /// Whether the closing checklist was completed.
    pub checklist_done: bool,
    /// The cashier the shortage is recovered from; the person who submits
    /// when blank.
    pub cashier: Option<i64>,
}

/// The till session of `closing`: the one linked, the one the cashier opened
/// for the outlet's till and day, or a new one with no float.
async fn till_of(
    state: &AppState,
    closing: &DailyClosing,
    outlet: &Outlet,
    date: NaiveDate,
    cashier: Option<i64>,
) -> Result<TillView> {
    let desk = till_desk(state);
    if let Some(id) = closing.till_session_id
        && let Some(view) = desk.find(state, id).await?
    {
        return Ok(view);
    }
    let till = outlet.till_key();
    let view = match desk.of_day(state, &till, date).await? {
        Some(view) => view,
        None => {
            let Some(cashier_user) = cashier else {
                return Err(refused(english("fnb.closing.problem.cashier", &[])));
            };
            desk.open(
                state,
                None,
                OpenTill {
                    till,
                    location: Some(outlet.code.clone()),
                    legal_entity_id: outlet.legal_entity_id,
                    business_date: date,
                    cashier_user,
                    float: 0,
                },
            )
            .await?
            .ok_or_else(|| refused(english("fnb.closing.problem.no_sales", &[])))?
        }
    };
    DailyClosing::query()
        .where_(DailyClosing::ID.eq(closing.id))
        .update(&state.db, &[("till_session_id", &Some(view.id))])
        .await?;
    Ok(view)
}

/// Submits the closing `id` as `by` (`None`: the system, for the demo company):
/// counts the till, which makes the cash handover of a shortage, and moves the
/// closing to `submitted`.
///
/// # Errors
///
/// 403 without `closing.submit`; 422 before the cut-off of the next morning
/// (with the time it opens), without the checklist, without the counted cash,
/// for a closing that is not `open` or `reopened`, or without a cashier.
pub async fn submit(
    state: &AppState,
    by: Option<&AuthUser>,
    id: i64,
    ask: &SubmitAsk,
) -> Result<DailyClosing> {
    if let Some(user) = by {
        require(user, permissions::CLOSING_SUBMIT)?;
    }
    let closing = DailyClosing::find_or_404(&state.db, id).await?;
    if !closing.can_submit() {
        return Err(refused(english("fnb.closing.problem.not_submittable", &[])));
    }
    let outlet = Outlet::find_or_404(&state.db, closing.outlet_id).await?;
    let date = closing
        .business_date
        .unwrap_or_else(wl_kernel::documents::today);
    // The guards are checked before the till is counted: counting the till is
    // not undone.
    if !ask.checklist_done {
        return Err(refused(english("fnb.closing.problem.checklist", &[])));
    }
    if ask.counted < 0 {
        return Err(refused(english("fnb.closing.problem.not_counted", &[])));
    }
    let opens: DateTime = outlet.opens_closing_at(date);
    if renox::db::now() < opens {
        return Err(refused(opens_text_problem(&outlet, date)));
    }
    let desk = till_desk(state);
    let cashier = ask.cashier.or(by.map(|user| user.id));
    let view = till_of(state, &closing, &outlet, date, cashier).await?;
    if view.status == "reviewed" {
        return Err(refused(english("fnb.closing.problem.till_reviewed", &[])));
    }
    if view.status == "closed" {
        desk.reopen(
            state,
            None,
            view.id,
            &english("fnb.closing.till_reopened", &[]),
        )
        .await?;
    }
    let counted_view = desk.submit_count(state, None, view.id, ask.counted).await?;
    let sums = totals(&state.db, &closing).await?;
    let input = ClosingInput {
        checklist_done: true,
        counted: Some(ask.counted),
        expected: counted_view.expected,
        cutoff_passed: true,
        opens_text: opens_text(&outlet, date),
        sales_total: sums.gross,
        orders_count: sums.orders,
    };
    let history = english("fnb.closing.history.submitted", &[]);
    let builder = closing.transition(if by.is_some() { SUBMIT } else { SUBMIT_SYSTEM });
    let outcome = match by {
        Some(user) => builder.by(user).reason(history).input(&input).run(state).await,
        None => builder.by_system().reason(history).input(&input).run(state).await,
    };
    let _ = outcome.map_err(|e| e.into_error())?;
    DailyClosing::find_or_404(&state.db, id).await
}

/// The refusal text of a submit before the cut-off.
fn opens_text_problem(outlet: &Outlet, date: NaiveDate) -> String {
    english(
        "fnb.closing.problem.cutoff",
        &[("opens", &opens_text(outlet, date))],
    )
}

/// Approves the submitted closing `id` (the area manager or the accountant);
/// the till session is marked reviewed.
///
/// # Errors
///
/// 403 without `closing.approve`; 422 for a closing that is not `submitted`.
pub async fn approve(state: &AppState, by: Option<&AuthUser>, id: i64) -> Result<DailyClosing> {
    if let Some(user) = by {
        require(user, permissions::CLOSING_APPROVE)?;
    }
    let closing = DailyClosing::find_or_404(&state.db, id).await?;
    let builder = closing.transition(if by.is_some() { APPROVE } else { APPROVE_SYSTEM });
    let outcome = match by {
        Some(user) => builder.by(user).run(state).await,
        None => builder.by_system().run(state).await,
    };
    let _ = outcome.map_err(|e| e.into_error())?;
    if let Some(till) = closing.till_session_id {
        let desk = till_desk(state);
        if desk
            .find(state, till)
            .await?
            .is_some_and(|view| view.status == "closed")
        {
            desk.review(state, None, till).await?;
        }
    }
    DailyClosing::find_or_404(&state.db, id).await
}

/// Puts the till session of `closing` back to `open` when it is closed.
async fn reopen_till(state: &AppState, closing: &DailyClosing, reason: &str) -> Result {
    let Some(till) = closing.till_session_id else {
        return Ok(());
    };
    let desk = till_desk(state);
    if desk
        .find(state, till)
        .await?
        .is_some_and(|view| view.status == "closed")
    {
        desk.reopen(state, None, till, reason).await?;
    }
    Ok(())
}

/// Reopens the submitted closing `id` for correction, with a reason (the area
/// manager only).
///
/// # Errors
///
/// 403 without `closing.reopen`; 422 without a reason or for a closing that is
/// not `submitted` (an `approved` one is never reopened).
pub async fn reopen(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: &str,
) -> Result<DailyClosing> {
    require(user, permissions::CLOSING_REOPEN)?;
    let closing = DailyClosing::find_or_404(&state.db, id).await?;
    let outcome = closing
        .transition(REOPEN)
        .by(user)
        .reason(reason.trim().to_owned())
        .run(state)
        .await;
    let _ = outcome.map_err(|e| e.into_error())?;
    reopen_till(state, &closing, reason.trim()).await?;
    DailyClosing::find_or_404(&state.db, id).await
}

/// `LateEntryRecorded` of a paper order: when the entry reopened a submitted
/// closing (the area manager approved it), the till session goes back to `open`
/// and the kitchen manager is told to count and submit again.
///
/// # Errors
///
/// Database errors and the notifier's.
pub(crate) async fn on_late_entry(state: &AppState, event: LateEntryRecorded) -> Result {
    if !is_paper_subject(&event.subject.subject_type) {
        return Ok(());
    }
    let Some(paper) = PaperOrder::find(&state.db, event.subject.subject_id).await? else {
        return Ok(());
    };
    let Some(closing_id) = paper.reopened_closing_id else {
        return Ok(());
    };
    let closing = DailyClosing::find_or_404(&state.db, closing_id).await?;
    let outlet = Outlet::find_or_404(&state.db, closing.outlet_id).await?;
    let date = closing
        .business_date
        .unwrap_or_else(wl_kernel::documents::today);
    reopen_till(state, &closing, &english("fnb.closing.reopen_late_reason", &[])).await?;
    tell_holders(
        state,
        closing.subject(),
        &outlet.name,
        permissions::CLOSING_SUBMIT,
        Some(outlet.scope()),
        &english(
            "fnb.closing.reopened_title",
            &[("outlet", &outlet.name), ("date", &date)],
        ),
        &english(
            "fnb.closing.reopened_body",
            &[
                ("outlet", &outlet.name),
                ("date", &date),
                ("reference", &paper.reference),
            ],
        ),
        &format!("/closings/{}", closing.id),
    )
    .await?;
    Ok(())
}

/// Sets the business-day cut-off of the outlet `id` to `time` (`HH:MM`). It
/// applies from the next business day only: today's closing keeps the cut-off
/// it was opened under.
///
/// # Errors
///
/// 403 without `closing.cutoff`; 422 for a text that is not a time of day.
pub async fn set_cutoff(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    time: &str,
) -> Result<Outlet> {
    require(user, permissions::CLOSING_CUTOFF)?;
    let Some(minutes) = parse_clock(time) else {
        return Err(refused(english("fnb.closing.problem.cutoff_time", &[])));
    };
    let outlet = Outlet::find_or_404(&state.db, id).await?;
    let today = outlet.business_date(renox::db::now());
    let from = today.succ_opt().unwrap_or(today);
    Outlet::query()
        .where_(Outlet::ID.eq(id))
        .update(
            &state.db,
            &[
                ("pending_cutoff_minutes", &Some(minutes)),
                ("pending_cutoff_from", &Some(from)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Outlet::find_or_404(&state.db, id).await
}

/// The closings of every outlet, newest business date first.
///
/// # Errors
///
/// Database errors.
pub async fn latest(db: &Db, limit: u64) -> Result<Vec<DailyClosing>> {
    DailyClosing::query()
        .order_by_desc(DailyClosing::BUSINESS_DATE)
        .order_by_desc(DailyClosing::ID)
        .limit(limit)
        .get(db)
        .await
}

/// Registers the listener of a recorded late entry.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: LateEntryRecorded, state: AppState| async move {
        on_late_entry(&state, event).await
    });
}
