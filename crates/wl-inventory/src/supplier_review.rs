//! Reviewing a supplier whose score fell under the threshold (wl-inventory
//! §5.9 steps 4 and 5, `STK.RUL.09`, issue #249). After the monthly snapshot
//! run, [`on_snapshots`] looks at each new snapshot:
//!
//! - a snapshot `below_threshold` (never `not_enough_data`) opens one
//!   [`SupplierReview`] for the supplier, a task for procurement under the
//!   key `inventory.supplier_review:<supplier>`, tells the procurement
//!   manager (holders of the role `procurement_manager`) and emits
//!   `SupplierScoreBelowThreshold`. The task carries the score breakdown and
//!   the items where the supplier is preferred, with their alternatives;
//! - while a review is open, every new snapshot of the supplier is added to
//!   it instead of opening another (a score back above the threshold too);
//!   an event still follows a new snapshot under the threshold;
//! - a snapshot built late by the catch-up run fires the rule once, like any
//!   other.
//!
//! Procurement closes the review with a decision ([`close`], the `close`
//! transition): *keep* changes nothing; *watch* shows the supplier as
//! *watched* ([`is_watched`]) until a later snapshot is at or above the
//! threshold; *change preferred* opens one request per item selected
//! (`PreferredSupplierRequest`, decided under BL-STK-31) to make the first
//! usable alternative preferred, and changes no item supplier by itself;
//! *request block* opens a task for a holder of `parties.block` with the
//! review, and blocks nobody. Every decision closes the review and is kept on
//! it with the snapshots it covered; a later snapshot under the threshold
//! opens a new review once this one is closed.
//!
//! Decisions: the task cannot be changed after it is opened (the ticket
//! contract only opens and resolves), so the review record is what "updated"
//! means and carries the snapshots; the task is resolved when the review is
//! closed. The block task is assigned to the first holder of `parties.block`
//! and stays unassigned when nobody holds it.

use std::collections::BTreeSet;

use renox::auth::permissions::{Scope, users_with_permission_in};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::inventory::SupplierScoreBelowThreshold;
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::{self, Assignee, OpenTicket, TASK_CATEGORY};
use wl_kernel::{PartyId, Subject};

use crate::chase::{PROCUREMENT_ROLE, holders};
use crate::config_of;
use crate::item::Item;
use crate::item_supplier::{
    PreferredSupplierRequest, RequestStatus, SupplierRole, audit, is_on_hold, rows_of,
};
use crate::permissions;
use crate::supplier_score::SupplierScore;
use crate::texts::{english, said};

/// The subject type of a supplier review.
pub const SUBJECT_TYPE: &str = "inventory.supplier_review";

/// The `close` transition's name.
pub const CLOSE: &str = "close";

/// The role of the procurement manager, who is told of a review.
pub const MANAGER_ROLE: &str = "procurement_manager";

/// The permission of the holders of the task to decide on a block
/// (wl-parties §4 `block`).
pub const BLOCK_PERMISSION: &str = "parties.block";

/// The notification type of the notice to the procurement manager.
pub const NOTICE: &str = "inventory.supplier_score_low";

/// Where a review stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReviewStatus {
    /// Waiting for procurement's decision.
    #[default]
    Open,
    /// Decided.
    Closed,
}

/// The review of a supplier under the threshold.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_supplier_reviews",
    state_machine,
    index(supplier_party_id)
)]
pub struct SupplierReview {
    /// The primary key.
    pub id: i64,
    /// The supplier.
    pub supplier_party_id: i64,
    /// The lifecycle state; changed only by a transition.
    pub status: ReviewStatus,
    /// The snapshots added to the review, as JSON ids, oldest first.
    pub snapshot_ids: String,
    /// `keep`, `watch`, `change_preferred` or `request_block`.
    pub decision: Option<String>,
    /// Who decided.
    pub decided_by: Option<i64>,
    /// When.
    pub decided_at: Option<DateTime>,
    /// The items a change of the preferred supplier was asked for, as JSON.
    pub decision_items: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl SupplierReview {
    /// The review as a [`Subject`].
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The supplier.
    pub fn supplier(&self) -> PartyId {
        PartyId(self.supplier_party_id)
    }

    /// The snapshots the review covers.
    pub fn snapshots(&self) -> Vec<i64> {
        renox::serde_json::from_str(&self.snapshot_ids).unwrap_or_default()
    }

    /// The items a change of preferred supplier was asked for.
    pub fn items(&self) -> Vec<i64> {
        renox::serde_json::from_str(&self.decision_items).unwrap_or_default()
    }
}

/// What procurement decides.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Decision {
    /// Nothing else changes.
    #[default]
    Keep,
    /// The supplier shows as watched.
    Watch,
    /// A request per item to change its preferred supplier.
    ChangePreferred {
        /// The items, each one with the supplier as preferred.
        items: Vec<i64>,
    },
    /// A task for the holders of `parties.block`.
    RequestBlock,
}

impl Decision {
    /// The stored key.
    pub fn key(&self) -> &'static str {
        match self {
            Decision::Keep => "keep",
            Decision::Watch => "watch",
            Decision::ChangePreferred { .. } => "change_preferred",
            Decision::RequestBlock => "request_block",
        }
    }
}

/// One change of preferred supplier asked for by *change preferred*.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replacement {
    /// The item.
    pub item_id: i64,
    /// The preferred supplier now.
    pub from: i64,
    /// The first usable alternative; none when there is not one.
    pub to: Option<i64>,
}

/// What a `close` run carries to its effect.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CloseInput {
    /// The decision.
    pub decision: Decision,
    /// The requests *change preferred* makes.
    pub replacements: Vec<Replacement>,
    /// The user who decides.
    pub by: Option<i64>,
}

fn input(ctx: &renox::state_machine::TransitionCtx) -> CloseInput {
    ctx.input_as::<CloseInput>().unwrap_or_default()
}

impl StateMachine for SupplierReview {
    type State = ReviewStatus;
    const COLUMN: renox::db::Col<SupplierReview, ReviewStatus> = SupplierReview::STATUS;

    fn machine() -> Machine<Self> {
        use ReviewStatus::{Closed, Open};
        Machine::new()
            .initial([Open])
            .state(Open)
            .tone("warning")
            .state(Closed)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(CLOSE)
                    .label("Close")
                    .tone("primary")
                    .permission(permissions::PURCHASE_ORDERS_ISSUE)
                    .from([Open])
                    .to(Closed)
                    .effect(|review: &SupplierReview, ctx, tx| {
                        let review = review.clone();
                        let run = input(ctx);
                        Box::pin(async move {
                            let now = renox::db::now();
                            let items: Vec<i64> =
                                run.replacements.iter().map(|r| r.item_id).collect();
                            let items_json = renox::serde_json::to_string(&items)
                                .unwrap_or_else(|_| "[]".to_owned());
                            let decision = Some(run.decision.key().to_owned());
                            let decided_at = Some(now);
                            SupplierReview::query()
                                .where_(SupplierReview::ID.eq(review.id))
                                .update(
                                    &mut *tx,
                                    &[
                                        ("decision", &decision),
                                        ("decided_by", &run.by),
                                        ("decided_at", &decided_at),
                                        ("decision_items", &items_json),
                                        ("updated_at", &now),
                                    ],
                                )
                                .await?;
                            for change in &run.replacements {
                                let open = PreferredSupplierRequest::query()
                                    .where_(PreferredSupplierRequest::ITEM_ID.eq(change.item_id))
                                    .where_(
                                        PreferredSupplierRequest::STATUS.eq(RequestStatus::Pending),
                                    )
                                    .first(&mut *tx)
                                    .await?;
                                if open.is_some() {
                                    continue;
                                }
                                let mut request = PreferredSupplierRequest {
                                    item_id: change.item_id,
                                    from_supplier_party_id: change.from,
                                    to_supplier_party_id: change.to,
                                    kind: "replace".to_owned(),
                                    status: RequestStatus::Pending,
                                    requested_by: run.by,
                                    ..PreferredSupplierRequest::default()
                                };
                                request.insert(&mut *tx).await?;
                            }
                            Ok(())
                        })
                    }),
            )
    }
}

/// The key of the task to review `supplier`.
pub fn task_key(supplier: i64) -> String {
    format!("inventory.supplier_review:{supplier}")
}

/// The key of the task to decide on blocking `supplier`.
pub fn block_task_key(supplier: i64) -> String {
    format!("inventory.supplier_block_request:{supplier}")
}

/// The notification type of the notice to the procurement manager: staff,
/// transactional, by e-mail.
pub fn notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(NOTICE, "inventory")
        .group("purchase_orders")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_inventory.review.subject")),
            english("wl_inventory.review.body"),
        )
}

/// Registers the notification type.
pub(crate) fn register_types() {
    notify::register_type(notice_type());
}

/// The open review of `supplier`, if there is one.
///
/// # Errors
///
/// A database error.
pub async fn open_review(db: &Db, supplier: i64) -> Result<Option<SupplierReview>> {
    SupplierReview::query()
        .where_(SupplierReview::SUPPLIER_PARTY_ID.eq(supplier))
        .where_(SupplierReview::STATUS.eq(ReviewStatus::Open))
        .first(db)
        .await
}

/// The text of the task: the score breakdown and the items where the
/// supplier is preferred with their alternatives.
async fn describe(state: &AppState, snapshot: &SupplierScore) -> Result<String> {
    let percent = |bp: Option<i64>| bp.map_or_else(|| "-".to_owned(), |bp| format!("{}%", bp / 100));
    let mut text = said(
        "wl_inventory.review.breakdown",
        &[
            ("score", &snapshot.score.unwrap_or(0).to_string()),
            ("from", &snapshot.window_from.to_string()),
            ("to", &snapshot.window_to.to_string()),
            ("receipts", &snapshot.receipts.to_string()),
            ("on_time", &percent(snapshot.on_time_bp)),
            ("rejected", &percent(snapshot.rejected_bp)),
            ("incidents", &snapshot.incidents.to_string()),
            ("price", &percent(snapshot.price_variance_bp)),
        ],
    );
    let rows = crate::item_supplier::ItemSupplier::query()
        .where_(crate::item_supplier::ItemSupplier::SUPPLIER_PARTY_ID.eq(snapshot.supplier_party_id))
        .where_(crate::item_supplier::ItemSupplier::ROLE.eq(SupplierRole::Preferred))
        .get(&state.db)
        .await?;
    for row in rows {
        let item = Item::find_or_404(&state.db, row.item_id).await?;
        let others: Vec<String> = rows_of(&state.db, row.item_id)
            .await?
            .iter()
            .filter(|other| other.role == SupplierRole::Alternative && other.active)
            .map(|other| other.supplier_party_id.to_string())
            .collect();
        text.push('\n');
        text.push_str(&said(
            "wl_inventory.review.item",
            &[("item", &item.code), ("alternatives", &others.join(", "))],
        ));
    }
    Ok(text)
}

/// `STK.RUL.09` for the snapshots just made, oldest first (see the module).
///
/// # Errors
///
/// A database error, or the notifier's.
pub async fn on_snapshots(state: &AppState, made: &[SupplierScore]) -> Result<()> {
    for snapshot in made {
        let supplier = snapshot.supplier_party_id;
        let below = snapshot.band == crate::supplier_score::ScoreBand::BelowThreshold;
        if let Some(open) = open_review(&state.db, supplier).await? {
            if snapshot.score.is_some() {
                let mut ids = open.snapshots();
                ids.push(snapshot.id);
                let json = renox::serde_json::to_string(&ids).unwrap_or_else(|_| "[]".to_owned());
                let now = renox::db::now();
                SupplierReview::query()
                    .where_(SupplierReview::ID.eq(open.id))
                    .update(&state.db, &[("snapshot_ids", &json), ("updated_at", &now)])
                    .await?;
            }
        } else if below {
            let mut review = SupplierReview {
                supplier_party_id: supplier,
                status: ReviewStatus::Open,
                snapshot_ids: renox::serde_json::to_string(&[snapshot.id])
                    .unwrap_or_else(|_| "[]".to_owned()),
                decision_items: "[]".to_owned(),
                ..SupplierReview::default()
            };
            review.save(&state.db).await?;
            let title = said(
                "wl_inventory.review.title",
                &[("supplier", &supplier.to_string())],
            );
            let task = RoleTask::new(
                review.subject(),
                PROCUREMENT_ROLE,
                &title,
                &task_key(supplier),
            )
            .description(&describe(state, snapshot).await?);
            role_tasks::open(state, task).await?;
            tell_manager(state, &review, snapshot).await?;
        }
        if below {
            state
                .emit(SupplierScoreBelowThreshold {
                    supplier: PartyId(supplier),
                    window_from: snapshot.window_from,
                    window_to: snapshot.window_to,
                    score: snapshot.score.unwrap_or(0),
                    receipts: snapshot.receipts,
                    on_time_bp: snapshot.on_time_bp,
                    rejected_bp: snapshot.rejected_bp,
                    incidents: snapshot.incidents,
                    price_variance_bp: snapshot.price_variance_bp,
                })
                .await?;
        }
    }
    Ok(())
}

/// Tells the procurement manager that `review` was opened.
async fn tell_manager(
    state: &AppState,
    review: &SupplierReview,
    snapshot: &SupplierScore,
) -> Result<()> {
    let people = holders(&state.db, MANAGER_ROLE).await?;
    if people.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(NOTICE, review.subject())
        .subject_label("supplier review")
        .data(json!({
            "supplier": review.supplier_party_id.to_string(),
            "score": snapshot.score.unwrap_or(0).to_string(),
        }));
    for id in people {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Procurement closes review `id` with `decision` (`purchase_orders.issue`).
///
/// # Errors
///
/// `Forbidden` without the permission; `Guard` (422) for a review that is
/// not open, and for *change preferred* with no item, or an item where the
/// supplier is not the preferred one.
pub async fn close(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    decision: Decision,
) -> std::result::Result<SupplierReview, TransitionError> {
    let review = SupplierReview::find_or_404(&state.db, id).await?;
    if review.status != ReviewStatus::Open {
        return Err(TransitionError::Guard(said(
            "wl_inventory.review.not_open",
            &[],
        )));
    }
    let mut run = CloseInput {
        decision: decision.clone(),
        replacements: Vec::new(),
        by: Some(user.id),
    };
    if let Decision::ChangePreferred { items } = &decision {
        if items.is_empty() {
            return Err(TransitionError::Guard(said(
                "wl_inventory.review.items_needed",
                &[],
            )));
        }
        let mut seen = BTreeSet::new();
        for item_id in items.iter().filter(|item| seen.insert(**item)) {
            let rows = rows_of(&state.db, *item_id).await?;
            let Some(preferred) = rows.iter().find(|row| {
                row.role == SupplierRole::Preferred
                    && row.active
                    && row.supplier_party_id == review.supplier_party_id
            }) else {
                return Err(TransitionError::Guard(said(
                    "wl_inventory.review.not_preferred",
                    &[("item", &item_id.to_string())],
                )));
            };
            let mut to = None;
            for row in rows
                .iter()
                .filter(|row| row.role == SupplierRole::Alternative && row.active)
            {
                if !is_on_hold(state, row.supplier_party_id).await? {
                    to = Some(row.supplier_party_id);
                    break;
                }
            }
            run.replacements.push(Replacement {
                item_id: *item_id,
                from: preferred.supplier_party_id,
                to,
            });
        }
    }
    let _outcome = review
        .transition(CLOSE)
        .by(user)
        .input(&run)
        .run(state)
        .await?;
    // After the commit: the review task is done, and a block is asked for.
    let creator = tickets::task_creator(state);
    if let Some(task) = creator
        .find_open(state, &task_key(review.supplier_party_id))
        .await?
    {
        let note = said("wl_inventory.review.resolved", &[]);
        creator.resolve_from_source(state, task.id, &note).await?;
    }
    if decision == Decision::RequestBlock {
        open_block_task(state, &review).await?;
    }
    audit(
        state,
        Some(user.id),
        "inventory.supplier_review_closed",
        SupplierReview::TABLE,
        review.id,
        json!({
            "supplier": review.supplier_party_id,
            "decision": decision.key(),
            "items": run.replacements.iter().map(|r| r.item_id).collect::<Vec<_>>(),
        }),
    )
    .await?;
    let closed = SupplierReview::find_or_404(&state.db, review.id).await?;
    Ok(closed)
}

/// The task for the holders of `parties.block` to decide on blocking the
/// supplier of `review`; nothing is blocked.
async fn open_block_task(state: &AppState, review: &SupplierReview) -> Result<()> {
    let title = said(
        "wl_inventory.review.block_title",
        &[("supplier", &review.supplier_party_id.to_string())],
    );
    let description = said(
        "wl_inventory.review.block_body",
        &[("review", &review.id.to_string())],
    );
    let mut ticket = OpenTicket::new(review.subject(), TASK_CATEGORY, &title)
        .description(&description)
        .source_key(&block_task_key(review.supplier_party_id));
    let holders = users_with_permission_in(&state.db, BLOCK_PERMISSION, &Scope::global()).await?;
    if let Some(holder) = holders.first() {
        ticket = ticket.assign_to(Assignee::user(holder.id));
    }
    tickets::task_creator(state).open(state, ticket).await?;
    Ok(())
}

/// Whether `supplier` shows as *watched*: its latest closed review was
/// decided *watch*, and no snapshot after those it covered is at or above
/// the threshold.
///
/// # Errors
///
/// A database error.
pub async fn is_watched(state: &AppState, supplier: i64) -> Result<bool> {
    let reviews = SupplierReview::query()
        .where_(SupplierReview::SUPPLIER_PARTY_ID.eq(supplier))
        .where_(SupplierReview::STATUS.eq(ReviewStatus::Closed))
        .get(&state.db)
        .await?;
    let Some(last) = reviews
        .iter()
        .max_by_key(|review| (review.decided_at, review.id))
    else {
        return Ok(false);
    };
    if last.decision.as_deref() != Some("watch") {
        return Ok(false);
    }
    let covered = SupplierScore::query()
        .where_(SupplierScore::ID.is_in(last.snapshots()))
        .get(&state.db)
        .await?;
    let Some(latest) = covered.iter().map(|snapshot| snapshot.snapshot_date).max() else {
        return Ok(true);
    };
    let threshold = config_of(state).score.threshold;
    let later = SupplierScore::query()
        .where_(SupplierScore::SUPPLIER_PARTY_ID.eq(supplier))
        .where_(SupplierScore::SNAPSHOT_DATE.gt(latest))
        .get(&state.db)
        .await?;
    Ok(!later
        .iter()
        .any(|snapshot| snapshot.score.is_some_and(|score| score >= threshold)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_decision_has_a_stored_key() {
        assert_eq!(Decision::Keep.key(), "keep");
        assert_eq!(Decision::Watch.key(), "watch");
        assert_eq!(
            Decision::ChangePreferred { items: vec![1] }.key(),
            "change_preferred"
        );
        assert_eq!(Decision::RequestBlock.key(), "request_block");
    }

    #[test]
    fn a_decision_round_trips_through_json() {
        let decision = Decision::ChangePreferred { items: vec![1, 2] };
        let json = renox::serde_json::to_string(&decision).unwrap();
        assert!(json.contains("change_preferred"), "{json}");
        assert_eq!(
            renox::serde_json::from_str::<Decision>(&json).unwrap(),
            decision
        );
        assert_eq!(
            renox::serde_json::from_str::<Decision>(r#"{"kind":"keep"}"#).unwrap(),
            Decision::Keep
        );
    }

    #[test]
    fn a_review_closes_once_and_is_closed_for_good() {
        let machine = SupplierReview::machine();
        assert_eq!(machine.initial_states(), ["open"]);
        let close = machine.find(CLOSE).expect("transition");
        assert!(!close.is_system());
        assert_eq!(close.from_states(), ["open"]);
        assert_eq!(close.to_state(), Some("closed"));
    }

    #[test]
    fn the_task_keys_name_the_supplier() {
        assert_eq!(task_key(7), "inventory.supplier_review:7");
        assert_eq!(block_task_key(7), "inventory.supplier_block_request:7");
    }
}
