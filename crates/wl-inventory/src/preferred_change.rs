//! Approving a change of an item's preferred supplier (wl-inventory §3
//! paragraph *Changing who is preferred*, `STK.ACL.07`, issue #250). Moving an
//! item's regular business to another supplier is a common way to favour a
//! related supplier, so a [`PreferredSupplierRequest`] needs a second person
//! who holds `item_suppliers.approve` and is not the requester:
//!
//! - a request is opened by [`request_preferred`] (make another supplier
//!   preferred; a supplier the item does not list yet is first added as an
//!   alternative, which needs no approval), by deactivating the preferred row
//!   ([`crate::item_supplier::deactivate`]) and by a supplier review that
//!   decides *change preferred*; until it is decided the rows stay as they
//!   are. One request per item may be pending;
//! - the approver sees both suppliers' score, lead time and last price
//!   ([`pages`](routes)) and decides with [`approve`] or [`reject`]; the
//!   requester can [`withdraw`] it. The `approve` transition checks that the
//!   approver is not the requester;
//! - on approval [`BlockGuard`](wl_kernel::block_guard) is checked again for
//!   the new supplier: a blocked one fails the approval, the request stays
//!   pending with a note and the requester is told (it is *returned*). The new
//!   supplier then becomes `preferred` and the old one `alternative` rank 1;
//!   when the request only removes the preferred row the item has no preferred
//!   supplier and `STK.RUL.04` uses the alternatives. `PreferredSupplierChanged`
//!   is emitted after the commit;
//! - a rejected or withdrawn request changes nothing, and the requester is
//!   told; the system cancels a pending request when the supplier it would
//!   make preferred is deactivated ([`cancel_targeting`]) or the item has no
//!   use for it any more ([`cancel_for_item`]);
//! - procurement managers (role `procurement_manager`) get a task under
//!   "Needs your action" for each pending request;
//! - with the §7 setting *Second person for a preferred supplier change* off
//!   ([`crate::WlInventory::preferred_change_needs_approval`]), the change
//!   applies at once through the system `apply` transition, audited with the
//!   user, and the event names the requester as the approver.
//!
//! Decisions: the request cannot be edited and resubmitted as the kernel's
//! approval returned feature would allow, because this request is a record of
//! the module with its own decision, not a kernel approval request: a returned
//! request is rejected or withdrawn and asked again. The module has no item
//! archive yet, so [`cancel_for_item`] is what the archive will call.

use renox::axum::http::HeaderMap;
use renox::db::Transaction;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::Serialize;
use wl_kernel::inventory::{ITEM_SUBJECT_TYPE, PreferredSupplierChanged};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets;
use wl_kernel::{PartyId, Subject};

use crate::config_of;
use crate::item::Item;
use crate::item_supplier::{
    ItemSupplier, LastPrice, NewItemSupplier, PreferredSupplierRequest, RequestStatus,
    SupplierRole, add, audit, holds, is_on_hold, last_price, refused, renumber, require, rows_in,
    rows_of,
};
use crate::permissions;
use crate::supplier_review::MANAGER_ROLE;
use crate::texts::{english, said, t};

/// The subject type of a request.
pub const SUBJECT_TYPE: &str = "inventory.preferred_supplier_request";

/// The `approve` transition's name.
pub const APPROVE: &str = "approve";
/// The `reject` transition's name.
pub const REJECT: &str = "reject";
/// The `withdraw` transition's name.
pub const WITHDRAW: &str = "withdraw";
/// The system's `cancel` transition's name.
pub const CANCEL: &str = "cancel";
/// The system's `apply` transition's name: the change applied without a
/// second person, when the setting is off.
pub const APPLY: &str = "apply";

/// The kind of a request that only takes the preferred row out of use.
pub const KIND_DEACTIVATE: &str = "deactivate";
/// The kind of a request that makes another supplier preferred.
pub const KIND_REPLACE: &str = "replace";

/// The notification type of the notice to the requester about a decision:
/// staff, transactional, by e-mail.
pub const NOTICE: &str = "inventory.preferred_supplier_decided";

impl PreferredSupplierRequest {
    /// The request as a [`Subject`].
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The supplier to make preferred, if the request names one.
    pub fn target(&self) -> Option<PartyId> {
        self.to_supplier_party_id.map(PartyId)
    }
}

/// `Err(message)` when the user running the transition is the requester.
fn other_than_requester(
    request: &PreferredSupplierRequest,
    ctx: &TransitionCtx,
) -> std::result::Result<(), String> {
    match ctx.actor() {
        Some(actor) if request.requested_by == Some(actor.id) => {
            Err(said("wl_inventory.preferred.same_person", &[]))
        }
        _ => Ok(()),
    }
}

/// `Err(message)` unless the user running the transition is the requester.
fn only_requester(
    request: &PreferredSupplierRequest,
    ctx: &TransitionCtx,
) -> std::result::Result<(), String> {
    match ctx.actor() {
        Some(actor) if request.requested_by == Some(actor.id) => Ok(()),
        Some(_) => Err(said("wl_inventory.preferred.only_requester", &[])),
        None => Ok(()),
    }
}

impl StateMachine for PreferredSupplierRequest {
    type State = RequestStatus;
    const COLUMN: renox::db::Col<PreferredSupplierRequest, RequestStatus> =
        PreferredSupplierRequest::STATUS;

    fn machine() -> Machine<Self> {
        use RequestStatus::{Approved, Cancelled, Pending, Rejected, Withdrawn};
        Machine::new()
            .initial([Pending])
            .state(Pending)
            .tone("warning")
            .state(Approved)
            .tone("success")
            .terminal()
            .state(Rejected)
            .tone("danger")
            .terminal()
            .state(Withdrawn)
            .tone("neutral")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(APPROVE)
                    .label("Approve")
                    .tone("primary")
                    .permission(permissions::ITEM_SUPPLIERS_APPROVE)
                    .from([Pending])
                    .to(Approved)
                    .guard(
                        "not_the_requester",
                        |request: &PreferredSupplierRequest, ctx| {
                            Box::pin(async move { other_than_requester(request, ctx) })
                        },
                    )
                    .effect(|request: &PreferredSupplierRequest, ctx, tx| {
                        let request = request.clone();
                        let by = ctx.actor().map(|actor| actor.id);
                        Box::pin(async move {
                            apply_in(tx, &request).await?;
                            stamp(tx, request.id, by, None).await
                        })
                    }),
            )
            .transition(
                Transition::new(APPLY)
                    .label("Apply")
                    .system()
                    .from([Pending])
                    .to(Approved)
                    .effect(|request: &PreferredSupplierRequest, _ctx, tx| {
                        let request = request.clone();
                        Box::pin(async move {
                            apply_in(tx, &request).await?;
                            stamp(tx, request.id, request.requested_by, None).await
                        })
                    }),
            )
            .transition(
                Transition::new(REJECT)
                    .label("Reject")
                    .tone("danger")
                    .permission(permissions::ITEM_SUPPLIERS_APPROVE)
                    .reason_required()
                    .from([Pending])
                    .to(Rejected)
                    .guard(
                        "not_the_requester",
                        |request: &PreferredSupplierRequest, ctx| {
                            Box::pin(async move { other_than_requester(request, ctx) })
                        },
                    )
                    .effect(|request: &PreferredSupplierRequest, ctx, tx| {
                        let id = request.id;
                        let by = ctx.actor().map(|actor| actor.id);
                        let reason = ctx.reason().map(str::to_owned);
                        Box::pin(async move { stamp(tx, id, by, reason).await })
                    }),
            )
            .transition(
                Transition::new(WITHDRAW)
                    .label("Withdraw")
                    .permission(permissions::ITEM_SUPPLIERS_MANAGE)
                    .from([Pending])
                    .to(Withdrawn)
                    .guard(
                        "only_the_requester",
                        |request: &PreferredSupplierRequest, ctx| {
                            Box::pin(async move { only_requester(request, ctx) })
                        },
                    )
                    .effect(|request: &PreferredSupplierRequest, ctx, tx| {
                        let id = request.id;
                        let by = ctx.actor().map(|actor| actor.id);
                        let reason = ctx.reason().map(str::to_owned);
                        Box::pin(async move { stamp(tx, id, by, reason).await })
                    }),
            )
            .transition(
                Transition::new(CANCEL)
                    .label("Cancel")
                    .system()
                    .from([Pending])
                    .to(Cancelled)
                    .effect(|request: &PreferredSupplierRequest, ctx, tx| {
                        let id = request.id;
                        let reason = ctx.reason().map(str::to_owned);
                        Box::pin(async move { stamp(tx, id, None, reason).await })
                    }),
            )
    }
}

/// Records who decided request `id`, when and why.
async fn stamp(tx: &mut Transaction, id: i64, by: Option<i64>, reason: Option<String>) -> Result {
    let now = renox::db::now();
    PreferredSupplierRequest::query()
        .where_(PreferredSupplierRequest::ID.eq(id))
        .update(
            &mut *tx,
            &[
                ("decided_by", &by),
                ("decided_at", &now),
                ("decision_reason", &reason),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

/// The effect of an approval: the item's rows change as the request says.
async fn apply_in(tx: &mut Transaction, request: &PreferredSupplierRequest) -> Result {
    let rows = rows_in(tx, request.item_id).await?;
    let now = renox::db::now();
    let alternative = SupplierRole::Alternative.as_str().to_owned();
    let preferred = SupplierRole::Preferred.as_str().to_owned();
    let last = i64::from(i32::MAX);
    let old = rows
        .iter()
        .find(|row| row.role == SupplierRole::Preferred)
        .cloned();
    if request.kind == KIND_DEACTIVATE {
        let Some(old) = old.filter(|row| row.supplier_party_id == request.from_supplier_party_id)
        else {
            return Err(refused(said("wl_inventory.preferred.stale", &[])));
        };
        let off = false;
        ItemSupplier::query()
            .where_(ItemSupplier::ID.eq(old.id))
            .update(
                &mut *tx,
                &[
                    ("role", &alternative),
                    ("rank", &last),
                    ("active", &off),
                    ("updated_at", &now),
                ],
            )
            .await?;
        renumber(tx, request.item_id, None).await?;
        return Ok(());
    }
    let Some(new) = rows.iter().find(|row| {
        Some(row.supplier_party_id) == request.to_supplier_party_id
            && row.active
            && row.role != SupplierRole::Preferred
    }) else {
        return Err(refused(said("wl_inventory.preferred.stale", &[])));
    };
    let first = 0_i64;
    ItemSupplier::query()
        .where_(ItemSupplier::ID.eq(new.id))
        .update(
            &mut *tx,
            &[("role", &preferred), ("rank", &first), ("updated_at", &now)],
        )
        .await?;
    if let Some(old) = &old {
        ItemSupplier::query()
            .where_(ItemSupplier::ID.eq(old.id))
            .update(
                &mut *tx,
                &[
                    ("role", &alternative),
                    ("rank", &last),
                    ("updated_at", &now),
                ],
            )
            .await?;
    }
    let had = true;
    Item::query()
        .where_(Item::ID.eq(request.item_id))
        .update(&mut *tx, &[("had_preferred_supplier", &had)])
        .await?;
    renumber(tx, request.item_id, old.map(|old| (old.id, 1))).await?;
    Ok(())
}

/// The notification type of the notice to the requester.
pub fn notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(NOTICE, "inventory")
        .group("purchase_orders")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_inventory.preferred.subject")),
            english("wl_inventory.preferred.body"),
        )
}

/// Registers the notification type.
pub(crate) fn register_types() {
    notify::register_type(notice_type());
}

/// The key of the task to decide on request `id`.
pub fn task_key(id: i64) -> String {
    format!("inventory.preferred_change:{id}")
}

/// The pending request of `item_id`, if there is one.
///
/// # Errors
///
/// A database error.
pub async fn pending_of(db: &Db, item_id: i64) -> Result<Option<PreferredSupplierRequest>> {
    PreferredSupplierRequest::query()
        .where_(PreferredSupplierRequest::ITEM_ID.eq(item_id))
        .where_(PreferredSupplierRequest::STATUS.eq(RequestStatus::Pending))
        .first(db)
        .await
}

/// Tells the requester of `request` what became of it. Nobody is told of a
/// decision the requester made.
async fn tell_requester(
    state: &AppState,
    request: &PreferredSupplierRequest,
    outcome: &str,
    note: &str,
) -> Result<()> {
    let Some(requester) = request.requested_by else {
        return Ok(());
    };
    if request.decided_by == Some(requester) {
        return Ok(());
    }
    let item = Item::find_or_404(&state.db, request.item_id).await?;
    let notice = NotifyRequest::new(NOTICE, request.subject())
        .subject_label("preferred supplier request")
        .data(json!({
            "item": item.code,
            "outcome": outcome,
            "note": note,
        }))
        .to_user(requester);
    notify::notifier(state).notify(state, notice).await?;
    Ok(())
}

/// Opens the task for the procurement managers to decide on `request`.
async fn open_task(state: &AppState, request: &PreferredSupplierRequest) -> Result<()> {
    let item = Item::find_or_404(&state.db, request.item_id).await?;
    let title = said("wl_inventory.preferred.task_title", &[("item", &item.code)]);
    let description = said(
        "wl_inventory.preferred.task_body",
        &[("request", &request.id.to_string())],
    );
    let task = RoleTask::new(
        request.subject(),
        MANAGER_ROLE,
        &title,
        &task_key(request.id),
    )
    .description(&description);
    role_tasks::open(state, task).await?;
    Ok(())
}

/// Resolves the task of `request` once it is decided.
async fn close_task(state: &AppState, request: &PreferredSupplierRequest) -> Result<()> {
    let creator = tickets::task_creator(state);
    if let Some(task) = creator.find_open(state, &task_key(request.id)).await? {
        let note = said("wl_inventory.preferred.task_done", &[]);
        creator.resolve_from_source(state, task.id, &note).await?;
    }
    Ok(())
}

/// The event of the change `request` made.
fn changed_event(
    request: &PreferredSupplierRequest,
    approver: Option<i64>,
) -> PreferredSupplierChanged {
    let old =
        (request.from_supplier_party_id != 0).then_some(PartyId(request.from_supplier_party_id));
    PreferredSupplierChanged {
        item: Subject::new(ITEM_SUBJECT_TYPE, request.item_id),
        old_supplier: old,
        new_supplier: if request.kind == KIND_REPLACE {
            request.target()
        } else {
            None
        },
        requester: request.requested_by,
        approver,
    }
}

/// After an approved or applied change: closes the task, tells the
/// requester, audits and emits `PreferredSupplierChanged`.
async fn announce_change(
    state: &AppState,
    request: &PreferredSupplierRequest,
    approver: Option<i64>,
) -> Result<()> {
    close_task(state, request).await?;
    tell_requester(state, request, "approved", "").await?;
    audit(
        state,
        approver,
        "inventory.preferred_supplier_changed",
        PreferredSupplierRequest::TABLE,
        request.id,
        json!({
            "item": request.item_id,
            "from": request.from_supplier_party_id,
            "to": request.to_supplier_party_id,
            "kind": request.kind,
            "requester": request.requested_by,
        }),
    )
    .await?;
    state.emit(changed_event(request, approver)).await?;
    Ok(())
}

/// What follows a request that was just opened: the setting off applies it
/// at once, audited with the requester; otherwise the procurement managers
/// get a task. Returns the request as it stands.
///
/// # Errors
///
/// A database error, or the transition's refusal when the rows moved since.
pub async fn after_request(state: &AppState, id: i64) -> Result<PreferredSupplierRequest> {
    let request = PreferredSupplierRequest::find_or_404(&state.db, id).await?;
    if request.status != RequestStatus::Pending {
        return Ok(request);
    }
    if config_of(state).preferred_change_needs_approval {
        open_task(state, &request).await?;
        return Ok(request);
    }
    let _outcome = request
        .transition(APPLY)
        .by_system()
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    let applied = PreferredSupplierRequest::find_or_404(&state.db, id).await?;
    announce_change(state, &applied, applied.requested_by).await?;
    Ok(applied)
}

/// Asks to make `supplier` the preferred supplier of `item_id`
/// (`item_suppliers.manage`). A supplier the item does not list is added as
/// an alternative first. The rows change when a second person approves.
///
/// # Errors
///
/// `Forbidden` without the permission; 422 when a request is pending, when
/// the supplier already is the preferred one or is not in use for the item,
/// or when the item never had a preferred supplier (add one directly).
pub async fn request_preferred(
    state: &AppState,
    user: &AuthUser,
    item_id: i64,
    supplier: i64,
) -> Result<PreferredSupplierRequest> {
    require(state, user, permissions::ITEM_SUPPLIERS_MANAGE).await?;
    let item = Item::find_or_404(&state.db, item_id).await?;
    if pending_of(&state.db, item.id).await?.is_some() {
        return Err(refused(said("wl_inventory.preferred.pending_exists", &[])));
    }
    let rows = rows_of(&state.db, item.id).await?;
    let current = rows
        .iter()
        .find(|row| row.role == SupplierRole::Preferred && row.active)
        .cloned();
    if current.is_none() && !item.had_preferred_supplier {
        return Err(refused(said("wl_inventory.preferred.add_directly", &[])));
    }
    if current
        .as_ref()
        .is_some_and(|row| row.supplier_party_id == supplier)
    {
        return Err(refused(said("wl_inventory.preferred.already", &[])));
    }
    match rows.iter().find(|row| row.supplier_party_id == supplier) {
        Some(row) if !row.active => {
            return Err(refused(said("wl_inventory.supplier.inactive", &[])));
        }
        Some(_) => {}
        None => {
            add(
                state,
                user,
                NewItemSupplier::alternative(item.id, PartyId(supplier), None),
            )
            .await?;
        }
    }
    let mut request = PreferredSupplierRequest {
        item_id: item.id,
        from_supplier_party_id: current.map_or(0, |row| row.supplier_party_id),
        to_supplier_party_id: Some(supplier),
        kind: KIND_REPLACE.to_owned(),
        status: RequestStatus::Pending,
        requested_by: Some(user.id),
        ..PreferredSupplierRequest::default()
    };
    request.save(&state.db).await?;
    audit(
        state,
        Some(user.id),
        "inventory.preferred_supplier_requested",
        PreferredSupplierRequest::TABLE,
        request.id,
        json!({ "item": item.id, "supplier": supplier, "kind": KIND_REPLACE }),
    )
    .await?;
    after_request(state, request.id).await
}

/// A refusal of a decision, as the transition's own error.
fn guard(key: &str) -> TransitionError {
    TransitionError::Guard(said(key, &[]))
}

/// The request `id` when it is pending and `user` may decide it: holds
/// `item_suppliers.approve` and is not the requester.
async fn decidable(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<PreferredSupplierRequest, TransitionError> {
    require(state, user, permissions::ITEM_SUPPLIERS_APPROVE).await?;
    let request = PreferredSupplierRequest::find_or_404(&state.db, id).await?;
    if request.status != RequestStatus::Pending {
        return Err(guard("wl_inventory.preferred.not_pending"));
    }
    if request.requested_by == Some(user.id) {
        return Err(guard("wl_inventory.preferred.same_person"));
    }
    Ok(request)
}

/// Approves request `id` (`item_suppliers.approve`, not the requester). The
/// block guard is checked again for the new supplier: a blocked one returns
/// the request to the requester with a note and the rows stay as they are.
///
/// # Errors
///
/// `Forbidden` without the permission; `Guard` (422) for the requester, for a
/// request that is not pending, for a supplier on hold now, and when the rows
/// no longer allow the change.
pub async fn approve(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<PreferredSupplierRequest, TransitionError> {
    let request = decidable(state, user, id).await?;
    if request.kind == KIND_REPLACE
        && let Some(to) = request.to_supplier_party_id
        && is_on_hold(state, to).await?
    {
        let note = said("wl_inventory.preferred.new_on_hold", &[]);
        let now = renox::db::now();
        PreferredSupplierRequest::query()
            .where_(PreferredSupplierRequest::ID.eq(request.id))
            .update(
                &state.db,
                &[("returned_note", &Some(note.clone())), ("updated_at", &now)],
            )
            .await?;
        let returned = PreferredSupplierRequest::find_or_404(&state.db, id).await?;
        tell_requester(state, &returned, "returned", &note).await?;
        return Err(TransitionError::Guard(note));
    }
    let _outcome = request.transition(APPROVE).by(user).run(state).await?;
    let approved = PreferredSupplierRequest::find_or_404(&state.db, id).await?;
    announce_change(state, &approved, Some(user.id)).await?;
    Ok(approved)
}

/// Rejects request `id` with `reason` (`item_suppliers.approve`, not the
/// requester): nothing changes and the requester is told.
///
/// # Errors
///
/// `Forbidden` without the permission; `Guard` (422) for the requester and
/// for a request that is not pending; `ReasonRequired` without a reason.
pub async fn reject(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: &str,
) -> std::result::Result<PreferredSupplierRequest, TransitionError> {
    let request = decidable(state, user, id).await?;
    let _outcome = request
        .transition(REJECT)
        .by(user)
        .reason(reason.trim())
        .run(state)
        .await?;
    let rejected = PreferredSupplierRequest::find_or_404(&state.db, id).await?;
    close_task(state, &rejected).await?;
    tell_requester(state, &rejected, "rejected", reason.trim()).await?;
    audit(
        state,
        Some(user.id),
        "inventory.preferred_supplier_rejected",
        PreferredSupplierRequest::TABLE,
        rejected.id,
        json!({ "item": rejected.item_id, "reason": reason.trim() }),
    )
    .await?;
    Ok(rejected)
}

/// Takes request `id` back (`item_suppliers.manage`, the requester only).
///
/// # Errors
///
/// `Forbidden` without the permission; `Guard` (422) for another user and
/// for a request that is not pending.
pub async fn withdraw(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<PreferredSupplierRequest, TransitionError> {
    require(state, user, permissions::ITEM_SUPPLIERS_MANAGE).await?;
    let request = PreferredSupplierRequest::find_or_404(&state.db, id).await?;
    if request.status != RequestStatus::Pending {
        return Err(guard("wl_inventory.preferred.not_pending"));
    }
    let _outcome = request.transition(WITHDRAW).by(user).run(state).await?;
    let withdrawn = PreferredSupplierRequest::find_or_404(&state.db, id).await?;
    close_task(state, &withdrawn).await?;
    audit(
        state,
        Some(user.id),
        "inventory.preferred_supplier_withdrawn",
        PreferredSupplierRequest::TABLE,
        withdrawn.id,
        json!({ "item": withdrawn.item_id }),
    )
    .await?;
    Ok(withdrawn)
}

/// Cancels `request` as the system, with `reason`, then closes its task and
/// tells the requester.
async fn cancel_request(
    state: &AppState,
    request: PreferredSupplierRequest,
    reason: &str,
) -> Result<()> {
    let _outcome = request
        .transition(CANCEL)
        .by_system()
        .reason(reason)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    let cancelled = PreferredSupplierRequest::find_or_404(&state.db, request.id).await?;
    close_task(state, &cancelled).await?;
    tell_requester(state, &cancelled, "cancelled", reason).await?;
    Ok(())
}

/// Cancels the pending request of `item_id` that would make `supplier`
/// preferred, because its row was deactivated.
///
/// # Errors
///
/// A database error.
pub async fn cancel_targeting(state: &AppState, item_id: i64, supplier: i64) -> Result<()> {
    if let Some(request) = pending_of(&state.db, item_id).await?
        && request.to_supplier_party_id == Some(supplier)
    {
        let reason = said("wl_inventory.preferred.cancel_row_off", &[]);
        cancel_request(state, request, &reason).await?;
    }
    Ok(())
}

/// Cancels the pending request of `item_id`, because the item was archived.
///
/// # Errors
///
/// A database error.
pub async fn cancel_for_item(state: &AppState, item_id: i64) -> Result<()> {
    if let Some(request) = pending_of(&state.db, item_id).await? {
        let reason = said("wl_inventory.preferred.cancel_archived", &[]);
        cancel_request(state, request, &reason).await?;
    }
    Ok(())
}

/// One supplier of a request as the approver reads it.
#[derive(Debug, Serialize)]
pub(crate) struct SideView {
    pub(crate) supplier_party_id: i64,
    pub(crate) lead_time_days: Option<i64>,
    pub(crate) on_hold: bool,
    pub(crate) score: Option<i64>,
    pub(crate) band: &'static str,
    pub(crate) last_price: Option<LastPrice>,
}

/// A request as the back office reads it.
#[derive(Debug, Serialize)]
pub(crate) struct RequestView {
    pub(crate) id: i64,
    pub(crate) item_id: i64,
    pub(crate) kind: String,
    pub(crate) status: &'static str,
    pub(crate) requested_by: Option<i64>,
    pub(crate) decided_by: Option<i64>,
    pub(crate) decision_reason: Option<String>,
    pub(crate) returned_note: Option<String>,
    pub(crate) from: Option<SideView>,
    pub(crate) to: Option<SideView>,
}

/// `supplier` of `item_id` with its score, lead time and last price; the
/// price only with `costs`.
async fn side(state: &AppState, item_id: i64, supplier: i64, costs: bool) -> Result<SideView> {
    let rows = rows_of(&state.db, item_id).await?;
    let lead = rows
        .iter()
        .find(|row| row.supplier_party_id == supplier)
        .map(|row| row.lead_time_days);
    let figures = crate::supplier_score::live(state, supplier).await?;
    let price = if costs {
        last_price(&state.db, item_id, supplier).await?
    } else {
        None
    };
    Ok(SideView {
        supplier_party_id: supplier,
        lead_time_days: lead,
        on_hold: is_on_hold(state, supplier).await?,
        score: figures.score,
        band: figures.band.as_str(),
        last_price: price,
    })
}

async fn view_of(
    state: &AppState,
    request: &PreferredSupplierRequest,
    costs: bool,
) -> Result<RequestView> {
    let from = if request.from_supplier_party_id == 0 {
        None
    } else {
        Some(
            side(
                state,
                request.item_id,
                request.from_supplier_party_id,
                costs,
            )
            .await?,
        )
    };
    let to = match request.to_supplier_party_id {
        Some(to) => Some(side(state, request.item_id, to, costs).await?),
        None => None,
    };
    Ok(RequestView {
        id: request.id,
        item_id: request.item_id,
        kind: request.kind.clone(),
        status: request.status.as_str(),
        requested_by: request.requested_by,
        decided_by: request.decided_by,
        decision_reason: request.decision_reason.clone(),
        returned_note: request.returned_note.clone(),
        from,
        to,
    })
}

/// Whether `user` may read the requests: approves or manages suppliers.
pub(crate) async fn may_read(state: &AppState, user: &AuthUser) -> Result<bool> {
    Ok(
        holds(state, user, permissions::ITEM_SUPPLIERS_APPROVE).await?
            || holds(state, user, permissions::ITEM_SUPPLIERS_MANAGE).await?,
    )
}

fn json_response<T: Serialize>(value: &T) -> Result<Response> {
    let body = renox::serde_json::to_string(value)
        .map_err(|error| Error::Internal(renox::anyhow::Error::new(error)))?;
    Ok(([("content-type", "application/json")], body).into_response())
}

/// A transition's refusal as an answer.
fn answer(error: TransitionError) -> Error {
    match error {
        TransitionError::Guard(message) => refused(message),
        other => other.into_error(),
    }
}

/// The routes:
///
/// | Route | Name | Needs |
/// |---|---|---|
/// | `POST /items/{id}/preferred-requests` | `wl_inventory.preferred_request_open` | `item_suppliers.manage` |
/// | `GET /preferred-requests` | `wl_inventory.preferred_requests` | approve or manage |
/// | `GET /preferred-requests/{id}` | `wl_inventory.preferred_request_show` | approve or manage |
/// | `POST /preferred-requests/{id}/approve` | `wl_inventory.preferred_request_approve` | `item_suppliers.approve` |
/// | `POST /preferred-requests/{id}/reject` | `wl_inventory.preferred_request_reject` | `item_suppliers.approve` |
/// | `POST /preferred-requests/{id}/withdraw` | `wl_inventory.preferred_request_withdraw` | `item_suppliers.manage` |
pub(crate) fn routes() -> Routes {
    Routes::new()
        .post("/items/{id}/preferred-requests", open_request)
        .name("wl_inventory.preferred_request_open")
        .get("/preferred-requests", list_requests)
        .name("wl_inventory.preferred_requests")
        .get("/preferred-requests/{id}", show_request)
        .name("wl_inventory.preferred_request_show")
        .post("/preferred-requests/{id}/approve", approve_request)
        .name("wl_inventory.preferred_request_approve")
        .post("/preferred-requests/{id}/reject", reject_request)
        .name("wl_inventory.preferred_request_reject")
        .post("/preferred-requests/{id}/withdraw", withdraw_request)
        .name("wl_inventory.preferred_request_withdraw")
        .require_auth()
}

/// `POST /items/{id}/preferred-requests`: `supplier`.
async fn open_request(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<std::collections::BTreeMap<String, String>>,
) -> Result<Response> {
    let supplier: i64 = form
        .get("supplier")
        .and_then(|v| v.trim().parse().ok())
        .ok_or_else(|| refused(t(&lang, "wl_inventory.supplier.bad_input")))?;
    let request = request_preferred(&state, &user, id, supplier).await?;
    let key = if request.status == RequestStatus::Pending {
        "wl_inventory.preferred.requested"
    } else {
        "wl_inventory.preferred.applied"
    };
    Ok((Toast::success(t(&lang, key)), renox::HxRefresh).into_response())
}

/// `GET /preferred-requests`: the pending requests, oldest first.
async fn list_requests(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    request: renox::grid::GridRequest,
) -> Result<Response> {
    if !may_read(&state, &user).await? {
        return Err(Error::Forbidden);
    }
    // A browser gets the grid (a grid refresh with htmx too); everything else
    // gets the JSON.
    if wl_kernel::negotiate::wants_html(&headers) || headers.contains_key("hx-request") {
        return crate::screens::requests(&state, &lang, request).await;
    }
    let costs = holds(&state, &user, permissions::STOCK_COST_VIEW).await?;
    let mut pending = PreferredSupplierRequest::query()
        .where_(PreferredSupplierRequest::STATUS.eq(RequestStatus::Pending))
        .get(&state.db)
        .await?;
    pending.sort_by_key(|request| request.id);
    let mut views = Vec::new();
    for request in &pending {
        views.push(view_of(&state, request, costs).await?);
    }
    json_response(&views)
}

/// `GET /preferred-requests/{id}`: both suppliers' score, lead time and last
/// price (the price for a holder of `stock.cost.view`).
async fn show_request(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response> {
    if !may_read(&state, &user).await? {
        return Err(Error::Forbidden);
    }
    let costs = holds(&state, &user, permissions::STOCK_COST_VIEW).await?;
    let request = PreferredSupplierRequest::find_or_404(&state.db, id).await?;
    let view = view_of(&state, &request, costs).await?;
    if wl_kernel::negotiate::wants_html(&headers) {
        let reach = crate::screens::Reach {
            scores: holds(&state, &user, permissions::SUPPLIER_SCORES_VIEW).await?,
            costs,
            request: None,
        };
        // Never the requester (`STK.ACL.07`): the buttons are for another approver.
        let can_decide = request.requested_by != Some(user.id)
            && holds(&state, &user, permissions::ITEM_SUPPLIERS_APPROVE).await?;
        return crate::screens::request(&state, &lang, &view, &reach, can_decide).await;
    }
    json_response(&view)
}

/// `POST /preferred-requests/{id}/approve`.
async fn approve_request(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    approve(&state, &user, id).await.map_err(answer)?;
    Ok((
        Toast::success(t(&lang, "wl_inventory.preferred.approved")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /preferred-requests/{id}/reject`: `reason`.
async fn reject_request(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<std::collections::BTreeMap<String, String>>,
) -> Result<Response> {
    let reason = form.get("reason").map(String::as_str).unwrap_or_default();
    if reason.trim().is_empty() {
        return Err(refused(t(&lang, "wl_inventory.preferred.reason_needed")));
    }
    reject(&state, &user, id, reason).await.map_err(answer)?;
    Ok((
        Toast::success(t(&lang, "wl_inventory.preferred.rejected")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /preferred-requests/{id}/withdraw`.
async fn withdraw_request(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    withdraw(&state, &user, id).await.map_err(answer)?;
    Ok((
        Toast::success(t(&lang, "wl_inventory.preferred.withdrawn")),
        renox::HxRefresh,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_decided_once_and_ends_for_good() {
        let machine = PreferredSupplierRequest::machine();
        assert_eq!(machine.initial_states(), ["pending"]);
        for name in [APPROVE, REJECT, WITHDRAW] {
            let transition = machine.find(name).expect("transition");
            assert!(!transition.is_system(), "{name}");
            assert_eq!(transition.from_states(), ["pending"]);
        }
        for name in [APPLY, CANCEL] {
            assert!(machine.find(name).expect("transition").is_system());
        }
        assert!(machine.find(REJECT).expect("transition").needs_reason());
        assert_eq!(
            machine.find(APPROVE).expect("transition").to_state(),
            Some("approved")
        );
    }

    #[test]
    fn the_task_key_names_the_request() {
        assert_eq!(task_key(4), "inventory.preferred_change:4");
    }

    #[test]
    fn the_event_names_the_old_and_the_new_supplier() {
        let request = PreferredSupplierRequest {
            id: 1,
            item_id: 9,
            from_supplier_party_id: 21,
            to_supplier_party_id: Some(22),
            kind: KIND_REPLACE.to_owned(),
            requested_by: Some(3),
            ..PreferredSupplierRequest::default()
        };
        let event = changed_event(&request, Some(4));
        assert_eq!(event.old_supplier, Some(PartyId(21)));
        assert_eq!(event.new_supplier, Some(PartyId(22)));
        assert_eq!(event.item.to_string(), "inventory.item:9");
        let removal = PreferredSupplierRequest {
            kind: KIND_DEACTIVATE.to_owned(),
            to_supplier_party_id: None,
            ..request
        };
        assert_eq!(changed_event(&removal, None).new_supplier, None);
    }
}
