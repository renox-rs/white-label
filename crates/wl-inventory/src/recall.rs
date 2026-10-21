//! The Recall (wl-inventory §3 *Recall*, §4 *Recall* `open`, issue #233): the
//! withdrawal of lots that may be unsafe or defective, opened from a supplier
//! or authority notice or an internal finding.
//!
//! ```text
//! [*] --open--> open --trace--> traced --close--> closed
//! ```
//!
//! `open` (issue #233) is here; `trace` ([`crate::recall_trace`], issue #234)
//! runs right after it, `send_notices` ([`crate::recall_notice`]) and `close`
//! ([`crate::recall_close`], issue #235) follow. The transitions are declared
//! here:
//!
//! - `trace` (a holder of `recalls.manage` to run it again, or the system
//!   right after `open`, `trace_by_system`): `open` or `traced` to `traced`;
//!   the lines and the output lots are made by the service before it, in the
//!   same transaction, and the effect stamps the run;
//! - `send_notices` (`recalls.manage`): `traced` to `traced`; guards *notice
//!   template chosen* and *lines selected*; the effect marks the lines `sent`;
//! - `close` (`recalls.manage`): `traced` to `closed`; guards *the recall's
//!   stock holds are closed* and *no affected line is pending*; the effect
//!   stamps it and keeps the report.
//!
//! [`open`] (a holder of `recalls.manage`) takes a source (`supplier`,
//! `authority`, `internal` or `complaint`), a reason and a scope of items with
//! the lot numbers or the supplier lot numbers recalled. In one transaction it
//! finds the lots (a lot number matches a lot's number, a supplier lot number
//! its *supplier lot* field, both exactly after trimming, and the item must be
//! the item named), numbers the recall `RCL-2026-0001`, and places one stock
//! hold with reason `recall` on every lot found, at every location including
//! the in-transit one ([`crate::hold_service::place_in`]): each lot goes
//! `on_hold` and gets a line per location. After the commit the owner, the
//! storekeepers and procurement are told and [`RecallOpened`] follows.
//!
//! Decisions:
//! - a lot with no balance anywhere stays in the scope (the trace needs it)
//!   and gets no line; when no lot found has any stock the recall still opens
//!   and its hold has no line (it closes at once: nothing to decide);
//! - a scope that matches no lot is refused, and nothing is created: the
//!   recall and its hold are one transaction;
//! - an item that is not tracked by lot cannot be recalled by lot number
//!   (BL-STK-17 recalls it by item and received dates), so such a scope
//!   matches nothing;
//! - the owner is the holders of the business owner role (the company admins
//!   when nobody holds it), the storekeepers the holders of `stock.adjust` or
//!   `goods_receipts.post` in the scope of the locations of the held lines,
//!   procurement the holders of the `procurement` role;
//! - picked dispatches with a recalled lot are flagged by the lot being
//!   `on_hold` and by its hold lines, which is what the `dispatch` guard of
//!   BL-STK-21 reads; no dispatch exists yet to flag.

use std::collections::BTreeSet;

use renox::auth::permissions::{Scope, users_with_permission_in, users_with_role};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::COMPANY_ADMIN_ROLE;
use wl_kernel::Subject;
use wl_kernel::compliance::business_owner_role;
use wl_kernel::documents::today;
use wl_kernel::inventory::{LOT_SUBJECT_TYPE, RECALL_SUBJECT_TYPE, RecallOpened};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::numbering::{Reset, Series};

use crate::chase::PROCUREMENT_ROLE;
use crate::hold_decide::settle;
use crate::hold_service::{NewHold, NewScope, announce_placed, place_in};
use crate::location::Location;
use crate::lot::Lot;
use crate::permissions;
use crate::recall_line::{NoticeStatus, RecallLine};
use crate::stock_hold::lines_of;
use crate::texts::{english, said};

/// The notification type told to the owner, the storekeepers and procurement
/// when a recall is opened.
pub const OPENED_NOTICE: &str = "inventory.recall_opened";

/// The `trace` transition's name (run again by a holder of `recalls.manage`).
pub const TRACE: &str = "trace";

/// The system's `trace_by_system` transition's name (right after `open`).
pub const TRACE_BY_SYSTEM: &str = "trace_by_system";

/// The `send_notices` transition's name.
pub const SEND_NOTICES: &str = "send_notices";

/// The `close` transition's name.
pub const CLOSE: &str = "close";

/// The numbers of recalls: `RCL-2026-0001`.
pub const RECALL_SERIES: Series = Series {
    key: "recall",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "RCL",
    reset: Reset::Yearly,
    gapless: false,
    per_entity: false,
};

/// Where a recall comes from (§3).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RecallSource {
    /// The supplier says its lots are unsafe.
    #[default]
    Supplier,
    /// An authority orders it.
    Authority,
    /// An internal check found the problem.
    Internal,
    /// A complaint pointed at the lot.
    Complaint,
}

impl RecallSource {
    /// The source with the stored name `name` (`supplier`...), if any.
    pub fn parse(name: &str) -> Option<Self> {
        [
            RecallSource::Supplier,
            RecallSource::Authority,
            RecallSource::Internal,
            RecallSource::Complaint,
        ]
        .into_iter()
        .find(|source| source.as_str() == name.trim().to_lowercase())
    }
}

/// Where a recall stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RecallStatus {
    /// Lots in scope on hold; trace not yet built.
    #[default]
    Open,
    /// Affected subjects and parties listed; notices being sent.
    Traced,
    /// Every lot has an outcome and every affected party was told or waived
    /// (terminal).
    Closed,
}

/// The withdrawal of lots that may be unsafe or defective.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_recalls",
    state_machine,
    unique(number),
    index(status)
)]
pub struct Recall {
    /// The primary key.
    pub id: i64,
    /// The number from the `RCL-` series.
    pub number: Option<String>,
    /// Where it comes from.
    pub source: RecallSource,
    /// Why it was opened.
    pub reason: String,
    /// The supplier behind a supplier recall, when known.
    pub supplier_party_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: RecallStatus,
    /// The one stock hold (reason `recall`) that blocks the lots.
    pub stock_hold_id: Option<i64>,
    /// The user who opened it.
    pub opened_by: Option<i64>,
    /// When it was opened.
    pub opened_at: Option<DateTime>,
    /// When the trace last ran.
    pub traced_at: Option<DateTime>,
    /// How often the trace ran.
    pub trace_runs: i64,
    /// When it was closed.
    pub closed_at: Option<DateTime>,
    /// The user who closed it.
    pub closed_by: Option<i64>,
    /// The recall report (scope, trace, notices, outcomes), a PDF document
    /// kept as text; set when it closes.
    pub report: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Recall {
    /// The recall as a [`Subject`]: `inventory.recall:2`.
    pub fn subject(&self) -> Subject {
        Subject::new(RECALL_SUBJECT_TYPE, self.id)
    }
}

/// One thing recalled: the lot number or the supplier lot number of an item.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_inventory_recall_scope", index(recall_id))]
pub struct RecallScope {
    /// The primary key.
    pub id: i64,
    /// The recall.
    pub recall_id: i64,
    /// The item.
    pub item_id: i64,
    /// A lot number of the item.
    pub lot_number: Option<String>,
    /// A supplier lot number of the item.
    pub supplier_lot: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A lot found in the scope of a recall.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_inventory_recall_lots", unique(recall_id, lot_id))]
pub struct RecallLot {
    /// The primary key.
    pub id: i64,
    /// The recall.
    pub recall_id: i64,
    /// The lot.
    pub lot_id: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What a `send_notices` run carries to its guards and effect.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NoticeInput {
    /// The notice template: `customer`, `patient` or `internal`; empty when
    /// none was chosen.
    pub template: String,
    /// The ids of the affected lines to tell.
    pub lines: Vec<i64>,
    /// The user who sends them.
    pub by: Option<i64>,
    /// What is wrong with the selection (a line of another recall, one that
    /// is not pending, one with no party), `None` when nothing is.
    pub problem: Option<String>,
}

/// What a `close` run carries to its guards and effect.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CloseInput {
    /// Every stock hold of the recall is `closed`.
    pub holds_closed: bool,
    /// The affected lines still `pending`.
    pub pending_lines: i64,
    /// The user who closes it.
    pub by: Option<i64>,
    /// The report document to keep.
    pub report: String,
}

fn notice_input(ctx: &TransitionCtx) -> Option<NoticeInput> {
    ctx.input_as::<NoticeInput>().ok()
}

fn close_input(ctx: &TransitionCtx) -> Option<CloseInput> {
    ctx.input_as::<CloseInput>().ok()
}

/// The two `trace` transitions: the user's re-run and the system's first run.
fn trace_rules(transition: Transition<Recall>) -> Transition<Recall> {
    transition
        .from([RecallStatus::Open, RecallStatus::Traced])
        .to(RecallStatus::Traced)
        .effect(|recall: &Recall, _ctx, tx| {
            let id = recall.id;
            let runs = recall.trace_runs + 1;
            Box::pin(async move {
                let now = renox::db::now();
                Recall::query()
                    .where_(Recall::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("traced_at", &Some(now)),
                            ("trace_runs", &runs),
                            ("updated_at", &now),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

impl StateMachine for Recall {
    type State = RecallStatus;
    const COLUMN: renox::db::Col<Recall, RecallStatus> = Recall::STATUS;

    fn machine() -> Machine<Self> {
        use RecallStatus::{Closed, Open, Traced};
        Machine::new()
            .initial([Open])
            .state(Open)
            .tone("error")
            .state(Traced)
            .tone("warning")
            .state(Closed)
            .tone("success")
            .terminal()
            .transition(trace_rules(
                Transition::new(TRACE)
                    .label("Trace again")
                    .tone("primary")
                    .permission(permissions::RECALLS_MANAGE),
            ))
            .transition(trace_rules(
                Transition::new(TRACE_BY_SYSTEM).label("Trace").system(),
            ))
            .transition(
                Transition::new(SEND_NOTICES)
                    .label("Send notices")
                    .tone("primary")
                    .permission(permissions::RECALLS_MANAGE)
                    .from([Traced])
                    .to(Traced)
                    .guard("template_chosen", |_recall: &Recall, ctx| {
                        let chosen = notice_input(ctx).is_some_and(|input| {
                            crate::recall_notice::is_template(&input.template)
                        });
                        Box::pin(async move {
                            if chosen {
                                Ok(())
                            } else {
                                Err(english("wl_inventory.recall.no_template").to_owned())
                            }
                        })
                    })
                    .guard("lines_selected", |_recall: &Recall, ctx| {
                        let input = notice_input(ctx);
                        let none = input.as_ref().is_none_or(|input| input.lines.is_empty());
                        let problem = input.and_then(|input| input.problem);
                        Box::pin(async move {
                            if let Some(problem) = problem {
                                Err(problem)
                            } else if none {
                                Err(english("wl_inventory.recall.no_lines").to_owned())
                            } else {
                                Ok(())
                            }
                        })
                    })
                    .effect(|_recall: &Recall, ctx, tx| {
                        let input = notice_input(ctx).unwrap_or_default();
                        Box::pin(async move {
                            let now = renox::db::now();
                            let sent = NoticeStatus::Sent.as_str().to_owned();
                            let template = Some(input.template.clone());
                            for id in input.lines {
                                RecallLine::query()
                                    .where_(RecallLine::ID.eq(id))
                                    .update(
                                        &mut *tx,
                                        &[
                                            ("notice_status", &sent),
                                            ("template", &template),
                                            ("sent_at", &Some(now)),
                                            ("sent_by", &input.by),
                                            ("updated_at", &now),
                                        ],
                                    )
                                    .await?;
                            }
                            Ok(())
                        })
                    }),
            )
            .transition(
                Transition::new(CLOSE)
                    .label("Close")
                    .tone("success")
                    .permission(permissions::RECALLS_MANAGE)
                    .from([Traced])
                    .to(Closed)
                    .guard("holds_closed", |_recall: &Recall, ctx| {
                        let closed = close_input(ctx).is_some_and(|input| input.holds_closed);
                        Box::pin(async move {
                            if closed {
                                Ok(())
                            } else {
                                Err(english("wl_inventory.recall.hold_open").to_owned())
                            }
                        })
                    })
                    .guard("no_line_pending", |_recall: &Recall, ctx| {
                        let pending = close_input(ctx).map_or(1, |input| input.pending_lines);
                        Box::pin(async move {
                            if pending == 0 {
                                Ok(())
                            } else {
                                Err(said(
                                    "wl_inventory.recall.lines_pending",
                                    &[("count", &pending.to_string())],
                                ))
                            }
                        })
                    })
                    .effect(|recall: &Recall, ctx, tx| {
                        let id = recall.id;
                        let input = close_input(ctx).unwrap_or_default();
                        Box::pin(async move {
                            let now = renox::db::now();
                            Recall::query()
                                .where_(Recall::ID.eq(id))
                                .update(
                                    &mut *tx,
                                    &[
                                        ("closed_at", &Some(now)),
                                        ("closed_by", &input.by),
                                        ("report", &Some(input.report.clone())),
                                        ("updated_at", &now),
                                    ],
                                )
                                .await?;
                            Ok(())
                        })
                    }),
            )
    }
}

/// One lot or supplier lot number to recall.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeEntry {
    /// The item.
    pub item_id: i64,
    /// A lot number of the item.
    pub lot_number: Option<String>,
    /// A supplier lot number of the item.
    pub supplier_lot: Option<String>,
}

impl ScopeEntry {
    /// The lot `number` of `item_id`.
    pub fn lot(item_id: i64, number: &str) -> Self {
        ScopeEntry {
            item_id,
            lot_number: Some(number.trim().to_owned()),
            supplier_lot: None,
        }
    }

    /// The supplier lot `number` of `item_id`.
    pub fn supplier_lot(item_id: i64, number: &str) -> Self {
        ScopeEntry {
            item_id,
            lot_number: None,
            supplier_lot: Some(number.trim().to_owned()),
        }
    }
}

/// A recall to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRecall {
    /// Where it comes from.
    pub source: RecallSource,
    /// Why.
    pub reason: String,
    /// The supplier behind a supplier recall, when known.
    pub supplier_party_id: Option<i64>,
    /// What is recalled.
    pub scope: Vec<ScopeEntry>,
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The notification type of the notice about an opened recall.
pub fn opened_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(OPENED_NOTICE, "inventory")
        .group("recalls")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_inventory.recall.opened_subject")),
            english("wl_inventory.recall.opened_body"),
        )
}

/// Registers the notification types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(opened_notice_type());
}

/// The ids of the lots `entry` names, in `tx`.
async fn lots_of(tx: &mut renox::db::Transaction, entry: &ScopeEntry) -> Result<Vec<i64>> {
    let mut query = Lot::query().where_(Lot::ITEM_ID.eq(entry.item_id));
    if let Some(number) = &entry.lot_number {
        query = query.where_(Lot::NUMBER.eq(number.as_str()));
    } else if let Some(supplier_lot) = &entry.supplier_lot {
        query = query.where_(Lot::SUPPLIER_LOT.eq(Some(supplier_lot.clone())));
    } else {
        return Ok(Vec::new());
    }
    Ok(query
        .order_by(Lot::ID)
        .get(&mut *tx)
        .await?
        .iter()
        .map(|lot| lot.id)
        .collect())
}

/// The ids of the users who hold `permission` in the scope of each of
/// `locations` (everywhere when there is none).
async fn holders_at(db: &Db, permission: &str, locations: &BTreeSet<i64>) -> Result<Vec<i64>> {
    let mut ids = BTreeSet::new();
    if locations.is_empty() {
        for user in users_with_permission_in(db, permission, &Scope::global()).await? {
            ids.insert(user.id);
        }
    }
    for location_id in locations {
        let Some(location) = Location::find(db, *location_id).await? else {
            continue;
        };
        for user in users_with_permission_in(db, permission, &location.scope()).await? {
            ids.insert(user.id);
        }
    }
    Ok(ids.into_iter().collect())
}

/// The holders of `role`, else the company admins.
async fn holders_of_role(db: &Db, role: &str) -> Result<Vec<i64>> {
    for role in [role, COMPANY_ADMIN_ROLE] {
        let users = users_with_role(db, role).await?;
        if !users.is_empty() {
            return Ok(users.iter().map(|user| user.id).collect());
        }
    }
    Ok(Vec::new())
}

/// Tells the owner, the storekeepers and procurement about `recall`.
async fn announce(
    state: &AppState,
    recall: &Recall,
    locations: &BTreeSet<i64>,
    lots: usize,
) -> Result {
    let mut people: BTreeSet<i64> = BTreeSet::new();
    people.extend(holders_of_role(&state.db, &business_owner_role(state)).await?);
    people.extend(holders_at(&state.db, permissions::STOCK_ADJUST, locations).await?);
    people.extend(holders_at(&state.db, permissions::GOODS_RECEIPTS_POST, locations).await?);
    people.extend(
        users_with_role(&state.db, PROCUREMENT_ROLE)
            .await?
            .iter()
            .map(|user| user.id),
    );
    if people.is_empty() {
        return Ok(());
    }
    let number = recall.number.clone().unwrap_or_default();
    let mut request = NotifyRequest::new(OPENED_NOTICE, recall.subject())
        .subject_label(&number)
        .data(renox::serde_json::json!({
            "number": number,
            "source": recall.source.as_str(),
            "reason": recall.reason,
            "lots": lots.to_string(),
        }));
    for id in people {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// `user` (a holder of `recalls.manage`) opens `new`.
///
/// # Errors
///
/// `Forbidden` without the permission; 422 without a reason, with an empty
/// scope or an entry with no number, or when the scope matches no lot;
/// a database error.
pub async fn open(state: &AppState, user: &AuthUser, new: NewRecall) -> Result<Recall> {
    if !user.has_permission(permissions::RECALLS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let reason = new.reason.trim().to_owned();
    if reason.is_empty() {
        return Err(refused(said("wl_inventory.recall.no_reason", &[])));
    }
    let blank = |value: &Option<String>| value.as_deref().is_none_or(|v| v.trim().is_empty());
    if new.scope.is_empty()
        || new
            .scope
            .iter()
            .any(|entry| blank(&entry.lot_number) && blank(&entry.supplier_lot))
    {
        return Err(refused(said("wl_inventory.recall.bad_scope", &[])));
    }
    let mut tx = state.db.begin().await?;
    let mut lot_ids: BTreeSet<i64> = BTreeSet::new();
    for entry in &new.scope {
        lot_ids.extend(lots_of(&mut tx, entry).await?);
    }
    if lot_ids.is_empty() {
        return Err(refused(said("wl_inventory.recall.no_lot", &[])));
    }
    let mut recall = Recall {
        number: Some(RECALL_SERIES.next(&mut tx, None, None, today()).await?),
        source: new.source,
        reason: reason.clone(),
        supplier_party_id: new.supplier_party_id,
        status: RecallStatus::Open,
        opened_by: Some(user.id),
        opened_at: Some(renox::db::now()),
        ..Recall::default()
    };
    recall.insert(&mut tx).await?;
    for entry in &new.scope {
        let mut row = RecallScope {
            recall_id: recall.id,
            item_id: entry.item_id,
            lot_number: entry.lot_number.clone().filter(|v| !v.trim().is_empty()),
            supplier_lot: entry.supplier_lot.clone().filter(|v| !v.trim().is_empty()),
            ..RecallScope::default()
        };
        row.insert(&mut tx).await?;
    }
    for lot_id in &lot_ids {
        let mut row = RecallLot {
            recall_id: recall.id,
            lot_id: *lot_id,
            ..RecallLot::default()
        };
        row.insert(&mut tx).await?;
    }
    let placed = place_in(
        &mut tx,
        &NewHold {
            reason: "recall".to_owned(),
            reference: recall.subject(),
            scope: NewScope::Lots(lot_ids.iter().copied().collect()),
            by: Some(user.id),
        },
        true,
    )
    .await?;
    Recall::query()
        .where_(Recall::ID.eq(recall.id))
        .update(&mut tx, &[("stock_hold_id", &Some(placed.hold.id))])
        .await?;
    tx.commit().await?;

    let hold = announce_placed(state, placed).await?;
    let recall = Recall::find_or_404(&state.db, recall.id).await?;
    let lines = lines_of(&state.db, hold.id).await?;
    let locations: BTreeSet<i64> = lines.iter().map(|line| line.location_id).collect();
    announce(state, &recall, &locations, lot_ids.len()).await?;
    state
        .emit(RecallOpened {
            recall: recall.subject(),
            number: recall.number.clone().unwrap_or_default(),
            source: recall.source.as_str().to_owned(),
            reason: recall.reason.clone(),
            lots: lot_ids
                .iter()
                .map(|id| Subject::new(LOT_SUBJECT_TYPE, *id))
                .collect(),
            stock_hold: hold.subject(),
        })
        .await?;
    if lines.is_empty() {
        // Nothing of the lots is left to decide.
        settle(state, hold.id).await?;
    }
    // The trace starts at once (§4 *Recall*): who got the goods, and the lots
    // made from them.
    crate::recall_trace::trace(state, None, recall.id)
        .await
        .map_err(TransitionError::into_error)?;
    Recall::find_or_404(&state.db, recall.id).await
}

/// The lots found in the scope of `recall_id`, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn lots_in_scope(db: &Db, recall_id: i64) -> Result<Vec<i64>> {
    Ok(RecallLot::query()
        .where_(RecallLot::RECALL_ID.eq(recall_id))
        .order_by(RecallLot::ID)
        .get(db)
        .await?
        .iter()
        .map(|row| row.lot_id)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sources_are_the_four_of_the_spec() {
        for name in ["supplier", "authority", "internal", "complaint"] {
            assert_eq!(
                RecallSource::parse(name).map(|source| source.as_str()),
                Some(name)
            );
        }
        assert_eq!(
            RecallSource::parse(" Supplier "),
            Some(RecallSource::Supplier)
        );
        assert_eq!(RecallSource::parse("rumour"), None);
    }

    #[test]
    fn a_recall_is_open_until_traced_and_closed() {
        let machine = Recall::machine();
        assert_eq!(machine.initial_states(), ["open"]);
        let names: Vec<_> = machine.states().iter().map(|s| s.name.as_str()).collect();
        for state in ["open", "traced", "closed"] {
            assert!(names.contains(&state), "{state}");
        }
        let closed = machine.states().iter().find(|s| s.name == "closed");
        assert!(closed.is_some_and(|s| s.terminal));
    }

    #[test]
    fn the_numbers_follow_the_series() {
        assert_eq!(RECALL_SERIES.prefix, "RCL");
    }

    #[test]
    fn a_scope_entry_is_one_number_trimmed() {
        let entry = ScopeEntry::lot(3, " L9 ");
        assert_eq!(entry.lot_number.as_deref(), Some("L9"));
        assert_eq!(entry.supplier_lot, None);
        let entry = ScopeEntry::supplier_lot(3, "S-1 ");
        assert_eq!(entry.supplier_lot.as_deref(), Some("S-1"));
    }
}
