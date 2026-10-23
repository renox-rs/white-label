//! Importing, matching, disputing, resolving, settling and voiding an
//! aggregator payout statement (06 F&B §4.3, §7.1 *Payout statement*,
//! `FNB.REC.01`, `FNB.RUL.10`; BL-FNB-09 and BL-FNB-10).
//!
//! - **Import** ([`import`]): the accountant (`reconciliation.manage`) pastes
//!   or uploads the aggregator's CSV. A statement with the same aggregator and
//!   reference (trimmed, without case) as one that is not voided is refused,
//!   and the refusal names the existing one. Each line is matched to the order
//!   with the aggregator's own number (the sales order desk), or to an order
//!   entered late ([`enter_late_order`]); the rest are *unmatched*. Unmatched
//!   lines above AED 100 together give the accountant a task (`FNB.RUL.10`).
//! - **Match** ([`match_statement`]): refused while a line has no order and no
//!   explanation, or a refund line has no responsible party (an unmapped
//!   reason is set by the accountant, [`set_party`]). It creates one *no goods*
//!   sales return per refund line, resolved *refunded by the channel* (no
//!   refund payment, no credit note, no approval), books the receivable and
//!   flags the cancelled lines and compensations the statement paid. The
//!   returns are made before the transition and recorded on the line, so a
//!   second run, after a dispute, never makes a second.
//! - **Dispute** and **resolve** ([`dispute`], [`resolve`]): a follow-up task;
//!   a refund the aggregator takes back runs `reverse` on its sales return
//!   (reason *aggregator took the refund back*) and the receivable grows by it.
//! - **Void**: the kernel's void pattern (`PLT.ACL.09`). Once it committed
//!   ([`on_voided`]) the sales returns of the refund lines are reversed (reason
//!   *statement voided*) and the open tasks close. A settled statement cannot
//!   be voided.
//!
//! Decisions: the refund of a line is spread over the order's item lines in
//! order, each up to what it is worth (the file names no line); the sales
//! module refuses a refund above a line's or the order's value. A refund line of
//! an order entered late has no sales order, so it makes no return; the
//! statement page shows it.

use renox::chrono::{NaiveDate, NaiveTime};
use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::StateMachine;
use wl_kernel::documents::DUBAI_OFFSET_SECONDS;
use wl_kernel::documents::today;
use wl_kernel::late_entries::{self, Entered, EntrySpec};
use wl_kernel::machines::BoxFuture;
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::sales::{ChannelRefund, SalesOrderId, order_desk};
use wl_kernel::tickets::task_creator;
use wl_kernel::voids::RecordVoided;
use wl_kernel::{LateEntryDef, NewRecord};

use crate::changes::ExpectedRefund;
use crate::order::FnbOrder;
use crate::payout::{
    ImportPreset, LateOrder, PayoutLine, PayoutStatement, PayoutStatus, ReasonMap,
    STATEMENT_SERIES, SUBJECT_TYPE, aggregator_key, line_state, reference_key,
};
use crate::payout_csv::{self, ParsedLine};
use crate::payout_machine::{DISPUTE, MATCH, MATCH_SYSTEM, PayoutInput, RESOLVE, SETTLE};
use crate::permissions;
use crate::placing::refused;
use crate::settings::settings;
use crate::texts::english;

/// The subject type of an order entered late.
pub const LATE_ORDER_SUBJECT: &str = "fnb.late_order";

/// The parties a refund line may be set to: the kitchen's own fault is
/// *business*; a refund the kitchen did not cause is the *aggregator's*.
pub const PARTIES: [&str; 2] = ["business", "aggregator"];

/// The complaint categories of the sales settings' default list.
pub const CATEGORIES: [&str; 7] = [
    "cold",
    "late",
    "missing_item",
    "wrong_item",
    "quality",
    "service_not_delivered",
    "other",
];

/// The category of a refund whose reason is not mapped.
pub const OTHER_CATEGORY: &str = "other";

/// The reason the sales return of a refund line is reversed with when the
/// statement is voided.
pub const VOIDED_REASON: &str = "statement voided";

/// The reason the sales return is reversed with when the aggregator took the
/// refund back.
pub const TAKEN_BACK_REASON: &str = "aggregator took the refund back";

/// What an import needs.
#[derive(Debug, Clone, Default)]
pub struct ImportAsk {
    /// The aggregator: the sales channel of its orders.
    pub aggregator: String,
    /// The aggregator's reference of the payout.
    pub reference: String,
    /// The name of the file.
    pub file_name: String,
    /// The first day of the period.
    pub period_from: Option<NaiveDate>,
    /// The last day of the period.
    pub period_to: Option<NaiveDate>,
    /// The file's text.
    pub content: String,
}

/// Where a line's order was found.
#[derive(Debug, Clone, Copy, Default)]
struct Found {
    sales_order_id: Option<i64>,
    order_id: Option<i64>,
    late_order_id: Option<i64>,
}

impl Found {
    fn is_some(&self) -> bool {
        self.sales_order_id.is_some() || self.late_order_id.is_some()
    }
}

/// 422 unless the user holds `reconciliation.manage`; 403 without it.
fn ensure_manager(user: &AuthUser) -> Result {
    if user.has_permission(permissions::RECONCILIATION_MANAGE) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// The order of the aggregator's `number`, recorded by the kitchen or entered
/// late.
async fn find_order(state: &AppState, aggregator: &str, number: &str) -> Result<Found> {
    if let Some(view) = order_desk(state)
        .find_external(state, aggregator, number)
        .await?
    {
        let order = FnbOrder::of_sales_order(&state.db, view.id.0).await?;
        return Ok(Found {
            sales_order_id: Some(view.id.0),
            order_id: order.map(|o| o.id),
            late_order_id: None,
        });
    }
    if let Some(late) = LateOrder::find_number(&state.db, aggregator, number).await? {
        return Ok(Found {
            late_order_id: Some(late.id),
            ..Found::default()
        });
    }
    Ok(Found::default())
}

/// The category and the proposed party of a refund reason, from the
/// aggregator's mapping.
async fn mapped(
    state: &AppState,
    aggregator: &str,
    reason: &str,
) -> Result<(Option<String>, Option<String>)> {
    if reason.trim().is_empty() {
        return Ok((None, None));
    }
    Ok(
        match ReasonMap::find_for(&state.db, aggregator, reason).await? {
            Some(map) => (Some(map.category), Some(map.party)),
            None => (None, None),
        },
    )
}

/// Imports the statement of `ask` as `user`.
///
/// # Errors
///
/// 403 without `reconciliation.manage`; 422 for a missing aggregator or
/// reference, an aggregator with no column preset, a file that cannot be
/// read, and a statement that is a duplicate (the text names the existing
/// one).
pub async fn import(state: &AppState, user: &AuthUser, ask: &ImportAsk) -> Result<PayoutStatement> {
    ensure_manager(user)?;
    import_as(state, Some(user.id), ask).await
}

/// [`import`], for the person `by` or the system (the demo company).
///
/// # Errors
///
/// As [`import`], less the permission.
pub async fn import_as(
    state: &AppState,
    by: Option<i64>,
    ask: &ImportAsk,
) -> Result<PayoutStatement> {
    let aggregator = aggregator_key(&ask.aggregator);
    let reference = ask.reference.trim().to_owned();
    if aggregator.is_empty() || reference.is_empty() {
        return Err(refused(english("fnb.payout.problem.identity", &[])));
    }
    if let (Some(from), Some(to)) = (ask.period_from, ask.period_to)
        && to < from
    {
        return Err(refused(english("fnb.payout.problem.period", &[])));
    }
    if let Some(existing) = PayoutStatement::using(&state.db, &aggregator, &reference).await? {
        return Err(duplicate(&existing, &reference));
    }
    let Some(preset) = ImportPreset::of(&state.db, &aggregator).await? else {
        return Err(refused(english(
            "fnb.payout.problem.no_preset",
            &[("aggregator", &aggregator_name(&aggregator))],
        )));
    };
    let parsed = payout_csv::parse(&ask.content, &preset)
        .map_err(|problem| refused(english("fnb.payout.problem.file", &[("problem", &problem)])))?;
    // Look every line up first: nothing is written while the desk reads.
    let mut rows = Vec::new();
    for line in &parsed {
        let found = find_order(state, &aggregator, &line.order_number).await?;
        let (category, party) = if line.refund > 0 {
            mapped(state, &aggregator, &line.reason).await?
        } else {
            (None, None)
        };
        rows.push((line, found, category, party));
    }
    let number = STATEMENT_SERIES
        .next_allowing_gaps(&state.db, None, None, today())
        .await?;
    let mut statement = PayoutStatement {
        number,
        aggregator: aggregator.clone(),
        reference: reference.clone(),
        reference_key: reference_key(&reference),
        file_name: ask.file_name.trim().to_owned(),
        preset: preset.name.clone(),
        period_from: ask.period_from,
        period_to: ask.period_to,
        line_count: i64::try_from(parsed.len()).unwrap_or(i64::MAX),
        gross_total: parsed.iter().map(|l| l.gross).sum(),
        commission_total: parsed.iter().map(|l| l.commission).sum(),
        promo_total: parsed.iter().map(|l| l.promo).sum(),
        refund_total: parsed.iter().map(|l| l.refund).sum(),
        net_total: parsed.iter().map(|l| l.net).sum(),
        imported_by: by,
        ..PayoutStatement::default()
    };
    let mut tx = state.db.begin().await?;
    let history = english(
        "fnb.payout.history.imported",
        &[("file", &statement.file_name)],
    );
    // The partial unique index refuses a duplicate that raced this import.
    let inserted = statement
        .insert_in_state(&mut tx, PayoutStatus::Imported, &history)
        .await;
    if let Err(error) = inserted {
        drop(tx);
        if let Some(existing) = PayoutStatement::using(&state.db, &aggregator, &reference).await? {
            return Err(duplicate(&existing, &reference));
        }
        return Err(error);
    }
    for (line, found, category, party) in rows {
        let state_name = if found.is_some() {
            line_state::MATCHED
        } else {
            line_state::UNMATCHED
        };
        PayoutLine::create(
            &mut tx,
            PayoutLine {
                statement_id: statement.id,
                line_no: line.line_no,
                order_number: line.order_number.clone(),
                order_key: reference_key(&line.order_number),
                gross: line.gross,
                commission: line.commission,
                promo: line.promo,
                refund: line.refund,
                net: line.net,
                reason: line.reason.clone(),
                category,
                party,
                sales_order_id: found.sales_order_id,
                order_id: found.order_id,
                late_order_id: found.late_order_id,
                match_state: state_name.to_owned(),
                ..PayoutLine::default()
            },
        )
        .await?;
    }
    tx.commit().await?;
    unmatched_task(state, statement.id).await?;
    PayoutStatement::find_or_404(&state.db, statement.id).await
}

/// The name of an aggregator for a message: `aggregator_a` is "Aggregator A".
pub(crate) fn aggregator_name(code: &str) -> String {
    let key = format!("fnb.screen.channels.paper_channel.{code}");
    let name = english(&key, &[]);
    if name == key {
        code.replace('_', " ")
    } else {
        name
    }
}

/// 422 telling the accountant the statement exists, with its number and page.
fn duplicate(existing: &PayoutStatement, reference: &str) -> Error {
    let id = existing.id.to_string();
    refused(english(
        "fnb.payout.problem.duplicate",
        &[
            ("reference", &reference.trim().to_owned()),
            ("aggregator", &aggregator_name(&existing.aggregator)),
            ("number", &existing.number),
            ("id", &id),
        ],
    ))
}

/// The key of the reconciliation task of `FNB.RUL.10`.
fn unmatched_key(id: i64) -> String {
    format!("fnb.payout_unmatched:{id}")
}

/// The key of the follow-up task of a dispute.
fn dispute_key(id: i64) -> String {
    format!("fnb.payout_dispute:{id}")
}

/// The amount of the lines with no order that are not explained, in fils.
pub async fn unmatched_amount(db: &Db, statement_id: i64) -> Result<i64> {
    let lines = PayoutLine::of_statement(db, statement_id).await?;
    Ok(lines
        .iter()
        .filter(|l| l.is_unmatched())
        .map(|l| l.gross.abs().max(l.net.abs()))
        .sum())
}

/// `FNB.RUL.10`: an imported statement whose unmatched lines add up to more
/// than X gives the accountant a reconciliation task, once.
async fn unmatched_task(state: &AppState, id: i64) -> Result {
    let rules = settings(state);
    if !rules.payout_rule {
        return Ok(());
    }
    let statement = PayoutStatement::find_or_404(&state.db, id).await?;
    let amount = unmatched_amount(&state.db, id).await?;
    if amount <= rules.payout_task_fils {
        return Ok(());
    }
    let shown = crate::money::aed(amount);
    let task = RoleTask::new(
        statement.subject(),
        permissions::ACCOUNTANT_ROLE,
        &english(
            "fnb.payout.task.unmatched_title",
            &[("number", &statement.number)],
        ),
        &unmatched_key(id),
    )
    .description(&english(
        "fnb.payout.task.unmatched_body",
        &[
            ("number", &statement.number),
            ("amount", &shown),
            ("link", &format!("/payouts/{id}")),
        ],
    ));
    let ticket = role_tasks::open(state, task)
        .await?
        .map(|opened| opened.id.0);
    PayoutStatement::query()
        .where_(PayoutStatement::ID.eq(id))
        .update(&state.db, &[("task_id", &ticket)])
        .await?;
    Ok(())
}

/// Looks the unmatched lines of the statement up again: an order recorded or
/// entered late since the import. Returns how many lines found their order.
///
/// # Errors
///
/// Database errors.
pub async fn rematch(state: &AppState, statement_id: i64) -> Result<usize> {
    let statement = PayoutStatement::find_or_404(&state.db, statement_id).await?;
    let mut found_now = 0;
    for line in PayoutLine::of_statement(&state.db, statement_id).await? {
        if !line.is_unmatched() {
            continue;
        }
        let found = find_order(state, &statement.aggregator, &line.order_number).await?;
        if !found.is_some() {
            continue;
        }
        let state_name = line_state::MATCHED.to_owned();
        PayoutLine::query()
            .where_(PayoutLine::ID.eq(line.id))
            .update(
                &state.db,
                &[
                    ("sales_order_id", &found.sales_order_id),
                    ("order_id", &found.order_id),
                    ("late_order_id", &found.late_order_id),
                    ("match_state", &state_name),
                ],
            )
            .await?;
        found_now += 1;
    }
    Ok(found_now)
}

/// The statement `id` and its line `line_id`, for an action that changes the
/// line while the statement is still open (`imported` or `disputed`).
async fn open_line(
    state: &AppState,
    id: i64,
    line_id: i64,
) -> Result<(PayoutStatement, PayoutLine)> {
    let statement = PayoutStatement::find_or_404(&state.db, id).await?;
    if !matches!(
        statement.status,
        PayoutStatus::Imported | PayoutStatus::Disputed
    ) {
        return Err(refused(english("fnb.payout.problem.closed", &[])));
    }
    let line = PayoutLine::find_or_404(&state.db, line_id).await?;
    if line.statement_id != id {
        return Err(Error::NotFound);
    }
    Ok((statement, line))
}

/// Sets the responsible party (and the category) of a refund line, before
/// `match` (BL-FNB-10): *business* or *aggregator*. The aggregator's reason is
/// remembered, so the next statement proposes the same.
///
/// # Errors
///
/// 403 without `reconciliation.manage`; 422 for a line that is no refund, a
/// party or category that is not on the lists, a statement past `match`.
pub async fn set_party(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    line_id: i64,
    party: &str,
    category: Option<&str>,
) -> Result<PayoutLine> {
    ensure_manager(user)?;
    let (statement, line) = open_line(state, id, line_id).await?;
    let party = party.trim().to_lowercase();
    if !PARTIES.contains(&party.as_str()) {
        return Err(refused(english("fnb.payout.problem.party", &[])));
    }
    if !line.is_refund() {
        return Err(refused(english("fnb.payout.problem.not_refund", &[])));
    }
    let category = category
        .map(|c| c.trim().to_lowercase())
        .filter(|c| !c.is_empty())
        .or_else(|| line.category.clone())
        .unwrap_or_else(|| OTHER_CATEGORY.to_owned());
    if !CATEGORIES.contains(&category.as_str()) {
        return Err(refused(english("fnb.payout.problem.category", &[])));
    }
    let new_party = Some(party.clone());
    let set_category = Some(category.clone());
    PayoutLine::query()
        .where_(PayoutLine::ID.eq(line.id))
        .update(
            &state.db,
            &[("party", &new_party), ("category", &set_category)],
        )
        .await?;
    if !line.reason.trim().is_empty() {
        match ReasonMap::find_for(&state.db, &statement.aggregator, &line.reason).await? {
            Some(map) => {
                ReasonMap::query()
                    .where_(ReasonMap::ID.eq(map.id))
                    .update(&state.db, &[("category", &category), ("party", &party)])
                    .await?;
            }
            None => {
                ReasonMap::create(
                    &state.db,
                    ReasonMap {
                        aggregator: statement.aggregator.clone(),
                        reason_key: reference_key(&line.reason),
                        category,
                        party,
                        ..ReasonMap::default()
                    },
                )
                .await?;
            }
        }
    }
    PayoutLine::find_or_404(&state.db, line.id).await
}

/// Explains a line with no order: the accountant says why that is fine, and
/// `match` no longer waits for it (BL-FNB-10).
///
/// # Errors
///
/// 403 without `reconciliation.manage`; 422 without a note, for a line that
/// has its order or a statement past `match`.
pub async fn explain(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    line_id: i64,
    note: &str,
) -> Result<PayoutLine> {
    ensure_manager(user)?;
    let (_statement, line) = open_line(state, id, line_id).await?;
    if note.trim().is_empty() {
        return Err(refused(english("fnb.payout.problem.note", &[])));
    }
    if !line.is_unmatched() {
        return Err(refused(english("fnb.payout.problem.has_order", &[])));
    }
    let name = line_state::EXPLAINED.to_owned();
    let text = Some(note.trim().to_owned());
    PayoutLine::query()
        .where_(PayoutLine::ID.eq(line.id))
        .update(&state.db, &[("match_state", &name), ("explanation", &text)])
        .await?;
    PayoutLine::find_or_404(&state.db, line.id).await
}

/// Lines with no order that are not explained, and refund lines with an order
/// and no party, as a transition's guards read them.
fn input_for(lines: &[PayoutLine]) -> PayoutInput {
    PayoutInput {
        unmatched: lines.iter().filter(|l| l.is_unmatched()).count() as i64,
        refunds_without_party: lines
            .iter()
            .filter(|l| l.is_refund() && l.party.is_none())
            .count() as i64,
        ..PayoutInput::default()
    }
}

/// Refuses (422) while the statement cannot be matched, with the guards' words
/// and before anything is created.
fn ensure_ready(input: &PayoutInput) -> Result {
    if input.unmatched > 0 {
        return Err(refused(english("fnb.payout.problem.unmatched", &[])));
    }
    if input.refunds_without_party > 0 {
        return Err(refused(english("fnb.payout.problem.party_missing", &[])));
    }
    Ok(())
}

/// Spreads `amount` over the item lines of the order, each up to its value.
fn spread(view: &wl_kernel::sales::OrderView, amount: i64) -> Vec<(i64, i64)> {
    let mut left = amount;
    let mut lines = Vec::new();
    for line in view.lines.iter().filter(|l| l.is_item && l.gross > 0) {
        if left <= 0 {
            break;
        }
        let take = left.min(line.gross);
        lines.push((line.id, take));
        left -= take;
    }
    // What the lines cannot hold goes on the last one: the sales module
    // refuses a refund above its value with its own words.
    if left > 0
        && let Some(last) = lines.last_mut()
    {
        last.1 += left;
    }
    lines
}

/// Makes the sales return of every refund line that has an order and no
/// return yet: *no goods*, resolved *refunded by the channel*.
async fn make_returns(state: &AppState, statement: &PayoutStatement) -> Result {
    for line in PayoutLine::of_statement(&state.db, statement.id).await? {
        if !line.is_refund() || line.return_id.is_some() {
            continue;
        }
        let Some(sales_order) = line.sales_order_id else {
            continue;
        };
        let (Some(category), Some(party)) = (line.category.clone(), line.party.clone()) else {
            continue;
        };
        let Some(view) = order_desk(state)
            .find(state, SalesOrderId(sales_order))
            .await?
        else {
            continue;
        };
        let reason = if line.reason.trim().is_empty() {
            english("fnb.payout.refund_reason", &[("number", &statement.number)])
        } else {
            line.reason.trim().to_owned()
        };
        let return_id = order_desk(state)
            .refund_by_channel(
                state,
                ChannelRefund {
                    order: SalesOrderId(sales_order),
                    lines: spread(&view, line.refund),
                    category,
                    // The sales settings' party for the aggregator is
                    // `aggregator`, for the kitchen `business`.
                    party,
                    reason,
                },
            )
            .await?;
        let made = Some(return_id);
        PayoutLine::query()
            .where_(PayoutLine::ID.eq(line.id))
            .update(&state.db, &[("return_id", &made)])
            .await?;
    }
    Ok(())
}

/// What `match` flags on the orders: a cancelled line or a compensation the
/// statement paid is matched (`FNB.RUL.10`).
async fn flag_paid(state: &AppState, statement: &PayoutStatement) -> Result {
    let at = Some(renox::db::now());
    let statement_id = Some(statement.id);
    for line in PayoutLine::of_statement(&state.db, statement.id).await? {
        let Some(order_id) = line.order_id else {
            continue;
        };
        if line.is_refund() {
            for expected in ExpectedRefund::of_order(&state.db, order_id).await? {
                if expected.matched_at.is_none() {
                    ExpectedRefund::query()
                        .where_(ExpectedRefund::ID.eq(expected.id))
                        .update(
                            &state.db,
                            &[("matched_at", &at), ("statement_id", &statement_id)],
                        )
                        .await?;
                }
            }
        }
        if line.net > 0
            && let Some(order) = FnbOrder::find(&state.db, order_id).await?
            && order.compensation_expected
            && order.compensation_matched_at.is_none()
        {
            FnbOrder::query()
                .where_(FnbOrder::ID.eq(order_id))
                .update(
                    &state.db,
                    &[
                        ("compensation_matched_at", &at),
                        ("compensation_statement_id", &statement_id),
                    ],
                )
                .await?;
        }
    }
    Ok(())
}

/// Matches the `imported` statement `id`. A person runs it as `by`; the system
/// (the demo company) passes `None`.
///
/// # Errors
///
/// 403 without `reconciliation.manage`; 422 while a line has no order and no
/// explanation, a refund line has no party, or the statement is not
/// `imported`; and the sales module's words for a refund it refuses.
pub async fn match_statement(
    state: &AppState,
    by: Option<&AuthUser>,
    id: i64,
) -> Result<PayoutStatement> {
    if let Some(user) = by {
        ensure_manager(user)?;
    }
    let statement = PayoutStatement::find_or_404(&state.db, id).await?;
    if statement.status != PayoutStatus::Imported {
        return Err(refused(english("fnb.payout.problem.not_imported", &[])));
    }
    let _ = rematch(state, id).await?;
    let lines = PayoutLine::of_statement(&state.db, id).await?;
    let input = input_for(&lines);
    ensure_ready(&input)?;
    make_returns(state, &statement).await?;
    let history = english("fnb.payout.history.matched", &[]);
    let builder = statement.transition(if by.is_some() { MATCH } else { MATCH_SYSTEM });
    let outcome = match by {
        Some(user) => {
            builder
                .by(user)
                .reason(history)
                .input(&input)
                .run(state)
                .await
        }
        None => {
            builder
                .by_system()
                .reason(history)
                .input(&input)
                .run(state)
                .await
        }
    };
    let _ = outcome.map_err(|e| e.into_error())?;
    let statement = PayoutStatement::find_or_404(&state.db, id).await?;
    flag_paid(state, &statement).await?;
    close_task(state, &unmatched_key(id), "fnb.payout.task.matched").await?;
    Ok(statement)
}

/// Disputes the lines `lines` of the `imported` or `matched` statement `id`
/// with the aggregator, for `reason`; the accountant gets a follow-up task.
///
/// # Errors
///
/// 403 without `reconciliation.manage`; 422 without a reason or a line, for a
/// line of another statement, or a statement that is not `imported` or
/// `matched`.
pub async fn dispute(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    lines: &[i64],
    reason: &str,
) -> Result<PayoutStatement> {
    ensure_manager(user)?;
    let statement = PayoutStatement::find_or_404(&state.db, id).await?;
    let own = PayoutLine::of_statement(&state.db, id).await?;
    if lines.iter().any(|l| !own.iter().any(|o| o.id == *l)) {
        return Err(Error::NotFound);
    }
    let input = PayoutInput {
        disputed_lines: lines.to_vec(),
        ..PayoutInput::default()
    };
    let _ = statement
        .transition(DISPUTE)
        .by(user)
        .reason(reason.trim().to_owned())
        .input(&input)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    let statement = PayoutStatement::find_or_404(&state.db, id).await?;
    let task = RoleTask::new(
        statement.subject(),
        permissions::ACCOUNTANT_ROLE,
        &english(
            "fnb.payout.task.dispute_title",
            &[("number", &statement.number)],
        ),
        &dispute_key(id),
    )
    .description(&english(
        "fnb.payout.task.dispute_body",
        &[
            ("number", &statement.number),
            ("reason", &reason.trim().to_owned()),
            ("link", &format!("/payouts/{id}")),
        ],
    ));
    let ticket = role_tasks::open(state, task)
        .await?
        .map(|opened| opened.id.0);
    PayoutStatement::query()
        .where_(PayoutStatement::ID.eq(id))
        .update(&state.db, &[("dispute_task_id", &ticket)])
        .await?;
    PayoutStatement::find_or_404(&state.db, id).await
}

/// Resolves the `disputed` statement `id` with the aggregator's `answer`.
/// `reversed` are the disputed refund lines the aggregator takes back: each
/// sales return runs `reverse` (reason *aggregator took the refund back*) and
/// the receivable grows by the refund. `adjustment` is any other amount the
/// answer adds to (or, negative, takes from) the receivable, in fils.
///
/// # Errors
///
/// 403 without `reconciliation.manage`; 422 without an answer, for a statement
/// that is not `disputed`, a line that is not a disputed refund line of it, and
/// what `match` refuses when the statement was disputed before it was matched.
pub async fn resolve(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    answer: &str,
    reversed: &[i64],
    adjustment: i64,
) -> Result<PayoutStatement> {
    ensure_manager(user)?;
    let statement = PayoutStatement::find_or_404(&state.db, id).await?;
    if statement.status != PayoutStatus::Disputed {
        return Err(refused(english("fnb.payout.problem.not_disputed", &[])));
    }
    if answer.trim().is_empty() {
        return Err(refused(english("fnb.payout.problem.answer", &[])));
    }
    let _ = rematch(state, id).await?;
    let lines = PayoutLine::of_statement(&state.db, id).await?;
    let booked = statement.receivable_booked_at.is_some();
    if !booked {
        // Disputed before it was matched: the answer also matches it.
        ensure_ready(&input_for(&lines))?;
        make_returns(state, &statement).await?;
    }
    for line_id in reversed {
        let Some(line) = lines.iter().find(|l| l.id == *line_id) else {
            return Err(Error::NotFound);
        };
        if !line.disputed || !line.is_refund() {
            return Err(refused(english(
                "fnb.payout.problem.not_disputed_refund",
                &[],
            )));
        }
    }
    let mut added = adjustment;
    for line in PayoutLine::of_statement(&state.db, id)
        .await?
        .iter()
        .filter(|l| reversed.contains(&l.id))
    {
        if let (Some(return_id), None) = (line.return_id, line.return_reversed_at) {
            order_desk(state)
                .reverse_by_channel(state, return_id, TAKEN_BACK_REASON)
                .await?;
            let at = Some(renox::db::now());
            PayoutLine::query()
                .where_(PayoutLine::ID.eq(line.id))
                .update(&state.db, &[("return_reversed_at", &at)])
                .await?;
            added += line.refund;
        }
    }
    let input = PayoutInput {
        adjustment: added,
        ..PayoutInput::default()
    };
    let _ = statement
        .transition(RESOLVE)
        .by(user)
        .reason(answer.trim().to_owned())
        .input(&input)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    let statement = PayoutStatement::find_or_404(&state.db, id).await?;
    if !booked {
        flag_paid(state, &statement).await?;
        close_task(state, &unmatched_key(id), "fnb.payout.task.matched").await?;
    }
    close_task(state, &dispute_key(id), "fnb.payout.task.resolved").await?;
    Ok(statement)
}

/// Settles the `matched` statement `id`: the bank receipt was allocated
/// against the receivable (the system's transition, `settle`).
///
/// # Errors
///
/// 422 unless the statement is `matched`.
pub async fn settle(state: &AppState, id: i64) -> Result<PayoutStatement> {
    let statement = PayoutStatement::find_or_404(&state.db, id).await?;
    let _ = statement
        .transition(SETTLE)
        .by_system()
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    PayoutStatement::find_or_404(&state.db, id).await
}

/// Completes the open task `key`, if any, with the text `note_key`.
async fn close_task(state: &AppState, key: &str, note_key: &str) -> Result {
    let tickets = task_creator(state);
    if let Some(open) = tickets.find_open(state, key).await? {
        tickets
            .complete(state, open.id, &english(note_key, &[]))
            .await?;
    }
    Ok(())
}

/// `RecordVoided` for a payout statement: its sales returns are reversed (reason
/// *statement voided*, no money moves, the orders' amount refunded goes back)
/// and its open tasks close with the void's reason.
///
/// # Errors
///
/// Database errors and the sales module's.
pub(crate) async fn on_voided(state: &AppState, event: RecordVoided) -> Result {
    if event.subject.subject_type != SUBJECT_TYPE {
        return Ok(());
    }
    let id = event.subject.subject_id;
    for line in PayoutLine::of_statement(&state.db, id).await? {
        if let (Some(return_id), None) = (line.return_id, line.return_reversed_at) {
            order_desk(state)
                .reverse_by_channel(state, return_id, VOIDED_REASON)
                .await?;
            let at = Some(renox::db::now());
            PayoutLine::query()
                .where_(PayoutLine::ID.eq(line.id))
                .update(&state.db, &[("return_reversed_at", &at)])
                .await?;
        }
    }
    let tickets = task_creator(state);
    for key in [unmatched_key(id), dispute_key(id)] {
        if let Some(open) = tickets.find_open(state, &key).await? {
            tickets.complete(state, open.id, &event.reason).await?;
        }
    }
    Ok(())
}

/// A difference between the orders and the statement the accountant should
/// look at (`FNB.RUL.10`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    /// `compensation` (an aggregator-cancelled order the statement does not
    /// pay) or `refund` (a cancelled line the statement does not refund).
    pub kind: &'static str,
    /// The order's number.
    pub order_number: String,
    /// The order.
    pub order_id: i64,
    /// What was expected, in fils.
    pub amount: i64,
}

/// The orders of the statement's aggregator that were flagged for the payout
/// reconciliation (an aggregator-cancelled order, a cancelled line) and that
/// the statement does not pay: listed as unmatched differences.
///
/// # Errors
///
/// Database errors.
pub async fn differences(db: &Db, statement: &PayoutStatement) -> Result<Vec<Difference>> {
    if matches!(
        statement.status,
        PayoutStatus::Voided | PayoutStatus::Settled
    ) {
        return Ok(Vec::new());
    }
    let lines = PayoutLine::of_statement(db, statement.id).await?;
    let orders = FnbOrder::query()
        .where_(FnbOrder::CHANNEL.eq(statement.aggregator.clone()))
        .get(db)
        .await?;
    let mut found = Vec::new();
    for order in &orders {
        let on_statement = |paid: &dyn Fn(&PayoutLine) -> bool| {
            lines
                .iter()
                .any(|l| l.order_id == Some(order.id) && paid(l))
        };
        if order.compensation_expected
            && order.compensation_matched_at.is_none()
            && !on_statement(&|l| l.net > 0)
        {
            found.push(Difference {
                kind: "compensation",
                order_number: order.number.clone(),
                order_id: order.id,
                amount: order.total,
            });
        }
        for expected in ExpectedRefund::of_order(db, order.id).await? {
            if expected.matched_at.is_none() && !on_statement(&|l| l.refund > 0) {
                found.push(Difference {
                    kind: "refund",
                    order_number: order.number.clone(),
                    order_id: order.id,
                    amount: expected.amount,
                });
            }
        }
    }
    Ok(found)
}

/// The kernel's handler: writes the order in its transaction.
fn create_late_order<'a>(
    tx: &'a mut renox::db::Transaction,
    new: &'a NewRecord,
) -> BoxFuture<'a, Result<i64>> {
    Box::pin(async move {
        let aggregator = aggregator_key(new.payload["aggregator"].as_str().unwrap_or_default());
        let number = new.payload["order_number"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_owned();
        let gross = new.payload["gross"].as_i64().unwrap_or_default();
        let row = LateOrder::create(
            &mut *tx,
            LateOrder {
                aggregator,
                order_key: reference_key(&number),
                order_number: number,
                occurred_at: Some(new.occurred_at),
                entered_at: Some(new.entered_at),
                late: new.late,
                late_reason: new.reason.clone(),
                late_entry_id: new.late_entry_id,
                gross,
                entered_by: Some(new.user_id),
                ..LateOrder::default()
            },
        )
        .await?;
        Ok(row.id)
    })
}

/// Registers the late order with the kernel's late-entry check.
pub(crate) fn register_late_entry() {
    LateEntryDef::of(LATE_ORDER_SUBJECT, create_late_order)
        .business_time("occurred_at")
        .register();
}

/// `LateEntryRecorded` of a late order that a manager approved: the statements
/// of its aggregator look their unmatched lines up again, as they do when the
/// entry needs no approval.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn on_late_order(
    state: &AppState,
    event: wl_kernel::late_entries::LateEntryRecorded,
) -> Result {
    if event.subject.subject_type != LATE_ORDER_SUBJECT {
        return Ok(());
    }
    let Some(late) = LateOrder::find(&state.db, event.subject.subject_id).await? else {
        return Ok(());
    };
    let open = PayoutStatement::query()
        .where_(PayoutStatement::AGGREGATOR.eq(late.aggregator))
        .where_(PayoutStatement::STATUS.is_in([PayoutStatus::Imported, PayoutStatus::Disputed]))
        .get(&state.db)
        .await?;
    for statement in open {
        let _ = rematch(state, statement.id).await?;
    }
    Ok(())
}

/// What the cashier types for an order the statement names and the kitchen
/// never recorded.
#[derive(Debug, Clone, Default)]
pub struct LateOrderAsk {
    /// The aggregator.
    pub aggregator: String,
    /// The aggregator's order number.
    pub order_number: String,
    /// The day it happened (Asia/Dubai).
    pub occurred_on: Option<NaiveDate>,
    /// The time it happened, `HH:MM` (Asia/Dubai); noon when blank.
    pub occurred_time: String,
    /// What the order was worth with VAT, in fils.
    pub gross: i64,
    /// Why it is late, when it is.
    pub reason: String,
}

/// The business time of the ask, as an instant.
fn occurred_at(ask: &LateOrderAsk) -> Result<DateTime> {
    let Some(day) = ask.occurred_on else {
        return Err(refused(english("fnb.payout.problem.late_day", &[])));
    };
    let time = if ask.occurred_time.trim().is_empty() {
        NaiveTime::from_hms_opt(12, 0, 0).expect("noon is a time")
    } else {
        NaiveTime::parse_from_str(ask.occurred_time.trim(), "%H:%M")
            .map_err(|_| refused(english("fnb.payout.problem.late_time", &[])))?
    };
    let local = day.and_time(time);
    let utc = local - renox::chrono::TimeDelta::seconds(i64::from(DUBAI_OFFSET_SECONDS));
    Ok(utc.and_utc())
}

/// The cashier enters an order the statement names and the kitchen never
/// recorded, as a late entry with the aggregator's number and the business
/// time from the statement; every statement of the aggregator is matched
/// again (BL-FNB-10, `PLT.ACL.10`).
///
/// # Errors
///
/// 403 without `orders.late_entry`; 422 for a blank number, a number already
/// recorded, a late entry without its reason (the kernel's words), and a time
/// that is not `HH:MM`.
pub async fn enter_late_order(
    state: &AppState,
    user: &AuthUser,
    ask: &LateOrderAsk,
) -> Result<Entered> {
    if !user.has_permission(permissions::LATE_ORDERS_ENTER) {
        return Err(Error::Forbidden);
    }
    let aggregator = aggregator_key(&ask.aggregator);
    let number = ask.order_number.trim().to_owned();
    if aggregator.is_empty() || number.is_empty() {
        return Err(refused(english("fnb.payout.problem.late_number", &[])));
    }
    if find_order(state, &aggregator, &number).await?.is_some() {
        return Err(refused(english("fnb.payout.problem.late_exists", &[])));
    }
    let spec = EntrySpec::new(LATE_ORDER_SUBJECT, user.id, occurred_at(ask)?)
        .reason(&ask.reason)
        .payload(json!({
            "aggregator": aggregator,
            "order_number": number,
            "gross": ask.gross,
        }));
    let entered = late_entries::enter(state, spec).await?;
    if entered.record_id().is_some() {
        let open = PayoutStatement::query()
            .where_(PayoutStatement::AGGREGATOR.eq(aggregator))
            .where_(PayoutStatement::STATUS.is_in([PayoutStatus::Imported, PayoutStatus::Disputed]))
            .get(&state.db)
            .await?;
        for statement in open {
            let _ = rematch(state, statement.id).await?;
        }
    }
    Ok(entered)
}

/// Parsed lines for a preview (the import form's check), without writing.
///
/// # Errors
///
/// The reason the file cannot be read.
pub async fn preview(db: &Db, aggregator: &str, content: &str) -> Result<Vec<ParsedLine>> {
    let Some(preset) = ImportPreset::of(db, aggregator).await? else {
        return Err(refused(english(
            "fnb.payout.problem.no_preset",
            &[("aggregator", &aggregator_key(aggregator))],
        )));
    };
    payout_csv::parse(content, &preset)
        .map_err(|problem| refused(english("fnb.payout.problem.file", &[("problem", &problem)])))
}
