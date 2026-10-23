//! The NCR services (07 Manufacturing §7.1 *NCR*, §4.3 row "A lot fails
//! incoming inspection", §4.4 steps 3 and 4; Base 07 *NCR lifecycle*,
//! BL-MFG-08): raise, change the cause, cancel, disposition, close and reopen.
//! One function per action, each gathering what the machine's guards need
//! ([`crate::ncr`]).
//!
//! Who may do what (the plant of the NCR): `raise` and `change_cause` need
//! `ncrs.raise`; `disposition`, `close`, `cancel` and `reopen` need
//! `ncrs.disposition`.
//!
//! - **Disposition** (BL-MFG-08, #389). *Rework* creates a planned work order
//!   of the item for the NCR quantity ([`crate::work_orders::create_by_system`]);
//!   *scrap* writes the quantity off through the kernel's `LotDisposal` (the
//!   lot's discard for the whole remaining quantity of a held or expired lot,
//!   else a `wastage` movement), a scrap above AED 2,000 waiting for a holder
//!   of `stock.approve` first (`STK.ACL.02`); *use as is* waits for a holder of
//!   `ncrs.approve` other than the requester, then releases the lot
//!   (`MFG.ACL.03`); *return to supplier* needs a supplier lot and creates a
//!   draft Return to supplier from the quarantine location, credit or replace,
//!   and is not offered for a lot made in-house. The action follows the commit
//!   ([`perform`]), once (a claim), and a failure reopens the NCR with the
//!   reason.
//! - **Close** is QC's, never the system's: a task *Close NCR* is opened when
//!   the rework work order completes, the scrap posts, the Return to supplier
//!   is dispatched or the lot is released. It is refused until then.
//! - **Reopen** by the system when the linked Return to supplier is cancelled
//!   or the rework work order is cancelled or aborted; the earlier
//!   disposition stays in [`NcrDecision`] and QC gets a task to choose a new
//!   one. QC may reopen by hand while the action did not happen.
//! - **A partial NCR** keeps its lot from release while it is `open` or
//!   `dispositioned` with the action not done ([`NcrReleaseGuard`]).
//!
//! Decisions where the spec is silent:
//!
//! - The plant of an NCR is the plant QC raises it in (the form offers the
//!   plants the person works for); an NCR raised by the system takes the first
//!   active plant.
//! - The listeners run twice for one transition (Renox announces a `run` and
//!   its `emit`), so each is idempotent: the claim of [`perform`], the status
//!   checks of the reopen and of the "done" mark.
//! - The scrap limit of `STK.ACL.02` is AED 2,000 here ([`crate::ncr::SCRAP_APPROVAL_FILS`]),
//!   as for the abort; wl-inventory's own limit is a setting of that module.

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine, Transitioned};
use wl_kernel::approvals::{ApprovalRequest, announce};
use wl_kernel::documents::today;
use wl_kernel::inventory::{ReturnOutcome, SUPPLIER_RETURN_SUBJECT_TYPE, SupplierReturnDispatched};
use wl_kernel::lot_disposal::{LotFuture, LotReleaseGuard, ReturnLot, ScrapLot, lot_disposal};
use wl_kernel::machines::approved;
use wl_kernel::numbering::{Reset, Series};
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, task_creator};
use wl_kernel::{Subject, approvals};

use crate::cost::value_of;
use crate::db::{answer, refused};
use crate::events::{NcrCancelled, NcrClosed, NcrDispositioned, NcrRaised, NcrReopened};
use crate::ncr::{
    AUTO_REOPEN, CANCEL, CLOSE, Cause, DISPOSITION, DISPOSITION_SCRAP, DISPOSITION_USE_AS_IS,
    Disposition, Ncr, NcrDecision, NcrInput, NcrStatus, Origin, REOPEN,
};
use crate::notices;
use crate::permissions;
use crate::plant::{Line, Plant};
use crate::texts::english;
use crate::work_order::{WorkOrder, WorkOrderStatus};
use crate::work_orders::{self, NewWorkOrder};

/// The numbers of NCRs: `NCR-2027-0001`.
pub const NCR_SERIES: Series = Series {
    key: "ncr",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "NCR",
    reset: Reset::Yearly,
    gapless: false,
    per_entity: false,
};

/// What a new NCR is made of.
#[derive(Debug, Clone, Default)]
pub struct NewNcr {
    /// The plant.
    pub plant_id: i64,
    /// The lot (`inventory.lot:4`) the defect is on.
    pub lot: Option<Subject>,
    /// The item (`inventory.item:9`), when there is no lot.
    pub item: Option<Subject>,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// The defect, in words.
    pub defect: String,
    /// The cause; the default is read from the lot.
    pub cause: Option<Cause>,
    /// Where it was raised from.
    pub origin: Origin,
    /// The record it was raised from (an invoice dispute, a dispatch).
    pub source: Option<Subject>,
}

/// What QC decides for an open NCR.
#[derive(Debug, Clone)]
pub struct DispositionAsk {
    /// The disposition.
    pub disposition: Disposition,
    /// For a return to supplier: `credit` or `replace`.
    pub outcome: Option<String>,
    /// A note.
    pub note: Option<String>,
    /// For rework: the line the work order is scheduled on.
    pub line_id: Option<i64>,
    /// For rework: the BOM version.
    pub bom_id: Option<i64>,
}

/// What a disposition did.
#[derive(Debug, Clone)]
pub struct Dispositioned {
    /// The NCR as it is now.
    pub ncr: Ncr,
    /// The approval request, while the disposition waits for one.
    pub approval_request: Option<i64>,
    /// The value of the scrap at the request, in fils.
    pub scrap_value: i64,
}

/// The NCR `id`, or 404.
async fn load(state: &AppState, id: i64) -> Result<Ncr> {
    Ncr::find_or_404(&state.db, id).await
}

/// Refuses unless `by` holds `permission` in the plant of `ncr`.
fn allow(ncr: &Ncr, by: &AuthUser, permission: &str) -> Result {
    if ncr.may(by, permission) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// The day, in Asia/Dubai.
fn raised_today() -> NaiveDate {
    today()
}

/// Raises an NCR as `by`.
///
/// # Errors
///
/// `Forbidden` without `ncrs.raise` for the plant; a 422 for an unknown or
/// inactive plant, no defect, a quantity that is not above zero or more than
/// the lot holds, or an unknown lot.
pub async fn raise(state: &AppState, by: &AuthUser, new: NewNcr) -> Result<Ncr> {
    let plant = Plant::find(&state.db, new.plant_id)
        .await?
        .filter(|plant| plant.active)
        .ok_or_else(|| refused(english("manufacturing.create.plant", &[])))?;
    if !by.has_permission_in(permissions::NCRS_RAISE, &plant.scope()) {
        return Err(Error::Forbidden);
    }
    insert(state, &plant, new, Some(by.id)).await
}

/// Raises an NCR as the system in the first active plant (a customer's
/// quality dispute, BL-MFG-13).
///
/// # Errors
///
/// As [`raise`]; a 422 when there is no active plant.
pub async fn raise_by_system(state: &AppState, mut new: NewNcr) -> Result<Ncr> {
    let plant = Plant::query()
        .where_(Plant::ACTIVE.eq(true))
        .order_by(Plant::ID)
        .first(&state.db)
        .await?
        .ok_or_else(|| refused(english("manufacturing.create.plant", &[])))?;
    new.plant_id = plant.id;
    insert(state, &plant, new, None).await
}

async fn insert(state: &AppState, plant: &Plant, new: NewNcr, by: Option<i64>) -> Result<Ncr> {
    if new.defect.trim().is_empty() {
        return Err(refused(english("manufacturing.ncr.defect", &[])));
    }
    if new.quantity_milli < 0 || (new.origin != Origin::Dispute && new.quantity_milli == 0) {
        return Err(refused(english("manufacturing.ncr.quantity", &[])));
    }
    let (item, lot_id, lot_number, supplier, own, remaining) = match &new.lot {
        Some(lot) => {
            let facts = lot_disposal(state)
                .facts(state, lot)
                .await?
                .ok_or_else(|| refused(english("manufacturing.ncr.lot_unknown", &[])))?;
            (
                facts.item.clone(),
                Some(lot.subject_id),
                Some(facts.number.clone()),
                facts.supplier,
                facts.produced_by.is_some(),
                Some(facts.quantity_milli),
            )
        }
        None => {
            let item = new
                .item
                .clone()
                .ok_or_else(|| refused(english("manufacturing.ncr.item", &[])))?;
            (item, None, None, None, false, None)
        }
    };
    if new.origin != Origin::Dispute
        && let Some(left) = remaining
        && new.quantity_milli > left
    {
        let left = crate::screens::quantity_text(left);
        return Err(refused(english(
            "manufacturing.ncr.too_much",
            &[("available", &left)],
        )));
    }
    // An NCR at incoming inspection defaults to the lot's supplier; one on
    // goods the plant made is its own process's; else it is not known.
    let cause = new.cause.unwrap_or(match (new.origin, supplier, own) {
        (Origin::Incoming, Some(_), _) => Cause::Supplier,
        (_, _, true) => Cause::InternalProcess,
        _ => Cause::Unknown,
    });
    let number = NCR_SERIES
        .next_allowing_gaps(&state.db, None, None, today())
        .await?;
    let ncr = Ncr::create(
        &state.db,
        Ncr {
            number,
            plant_id: plant.id,
            item_id: item.subject_id,
            lot_id,
            lot_number,
            supplier_party_id: supplier.map(|party| party.0),
            quantity_milli: new.quantity_milli,
            defect: new.defect.trim().to_owned(),
            cause,
            origin: new.origin,
            source_type: new.source.as_ref().map(|s| s.subject_type.clone()),
            source_id: new.source.as_ref().map(|s| s.subject_id),
            raised_by: by,
            raised_on: raised_today(),
            ..Ncr::default()
        },
    )
    .await?;
    state
        .emit(NcrRaised {
            ncr: ncr.subject(),
            lot: ncr.lot(),
            item: ncr.item(),
            supplier: ncr.supplier(),
            cause: ncr.cause.as_str().to_owned(),
            origin: ncr.origin.as_str().to_owned(),
            quantity_milli: ncr.quantity_milli,
        })
        .await?;
    Ok(ncr)
}

/// QC changes the cause of NCR `id` until it is closed, with a reason: the
/// supplier scorecard reads the live cause (BL-MFG-15).
///
/// # Errors
///
/// `Forbidden` without `ncrs.raise`; a 422 for a closed or cancelled NCR or
/// no reason.
pub async fn change_cause(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    cause: Cause,
    reason: &str,
) -> Result<Ncr> {
    let ncr = load(state, id).await?;
    allow(&ncr, by, permissions::NCRS_RAISE)?;
    if !matches!(ncr.status, NcrStatus::Open | NcrStatus::Dispositioned) {
        return Err(refused(english("manufacturing.ncr.cause_closed", &[])));
    }
    if reason.trim().is_empty() {
        return Err(refused(english("manufacturing.problems.reason", &[])));
    }
    if ncr.cause == cause {
        return Ok(ncr);
    }
    let stamp = now();
    Ncr::query()
        .where_(Ncr::ID.eq(id))
        .update(
            &state.db,
            &[
                ("cause", &cause.as_str().to_owned()),
                ("cause_reason", &Some(reason.trim().to_owned())),
                ("cause_changed_by", &Some(by.id)),
                ("cause_changed_at", &Some(stamp)),
                ("updated_at", &stamp),
            ],
        )
        .await?;
    load(state, id).await
}

/// QC cancels open NCR `id`, raised in error.
///
/// # Errors
///
/// `Forbidden` without `ncrs.disposition`; a 422 without a reason or when the
/// NCR is not `open`.
pub async fn cancel(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<Ncr> {
    let ncr = load(state, id).await?;
    allow(&ncr, by, permissions::NCRS_DISPOSITION)?;
    if waiting(state, &ncr).await? {
        return Err(refused(english("manufacturing.ncr.waiting", &[])));
    }
    let outcome = ncr
        .transition(CANCEL)
        .by(by)
        .reason(reason.trim())
        .run(state)
        .await
        .map_err(answer)?;
    outcome.emit(state).await?;
    close_task(state, &redisposition_key(id)).await?;
    state
        .emit(NcrCancelled {
            ncr: ncr.subject(),
            supplier: ncr.supplier(),
            reason: reason.trim().to_owned(),
        })
        .await?;
    load(state, id).await
}

/// Whether a disposition of `ncr` waits for an approval.
pub async fn waiting(state: &AppState, ncr: &Ncr) -> Result<bool> {
    Ok(!ApprovalRequest::pending_about(&state.db, &ncr.subject())
        .await?
        .is_empty())
}

/// The source key of the task to close NCR `id`.
pub fn close_key(id: i64) -> String {
    format!("manufacturing.close_ncr:{id}")
}

/// The source key of the task to choose a new disposition for NCR `id`.
pub fn redisposition_key(id: i64) -> String {
    format!("manufacturing.redisposition_ncr:{id}")
}

/// The QC inspector of the plant: a holder of the role in the plant's scope,
/// else whom the kernel picks for the role.
async fn inspector(state: &AppState, plant: i64) -> Result<Option<i64>> {
    if let Some(user) = renox::auth::permissions::users_with_role_in(
        &state.db,
        permissions::QC_ROLE,
        &permissions::plant_scope(plant),
    )
    .await?
    .first()
    {
        return Ok(Some(user.id));
    }
    wl_kernel::role_tasks::assignee(&state.db, permissions::QC_ROLE).await
}

/// Opens the task `key` for QC unless it is open already.
async fn open_task(
    state: &AppState,
    ncr: &Ncr,
    key: String,
    title: String,
    body: String,
) -> Result {
    let creator = task_creator(state);
    if creator.find_open(state, &key).await?.is_some() {
        return Ok(());
    }
    let mut ticket = OpenTicket::new(ncr.subject(), TASK_CATEGORY, &title)
        .description(&body)
        .source_key(&key);
    if let Some(user) = inspector(state, ncr.plant_id).await? {
        ticket = ticket.assign_to(Assignee::user(user));
    }
    creator.open(state, ticket).await?;
    Ok(())
}

/// Closes the open task `key`, if there is one.
async fn close_task(state: &AppState, key: &str) -> Result {
    let creator = task_creator(state);
    if let Some(found) = creator.find_open(state, key).await? {
        creator
            .complete(
                state,
                found.id,
                &english("manufacturing.ncr.task_done", &[]),
            )
            .await?;
    }
    Ok(())
}

/// The task *Close NCR* for QC: the action of the disposition happened.
async fn ask_to_close(state: &AppState, ncr: &Ncr) -> Result {
    open_task(
        state,
        ncr,
        close_key(ncr.id),
        english("manufacturing.ncr.close_title", &[("ncr", &ncr.number)]),
        english("manufacturing.ncr.close_body", &[("ncr", &ncr.number)]),
    )
    .await
}

/// Parses `credit` or `replace`.
fn outcome_of(text: &str) -> Option<ReturnOutcome> {
    match text {
        "credit" => Some(ReturnOutcome::Credit),
        "replace" => Some(ReturnOutcome::Replace),
        _ => None,
    }
}

/// Checks the disposition asked for `ncr` against the lot and the plant and
/// gathers its input; a 422 with every line that fails.
async fn check(
    state: &AppState,
    by: &AuthUser,
    ncr: &Ncr,
    ask: &DispositionAsk,
) -> Result<NcrInput> {
    let mut input = NcrInput {
        by: Some(by.id),
        disposition: Some(ask.disposition),
        outcome: ask.outcome.clone().filter(|o| !o.trim().is_empty()),
        note: ask.note.clone().filter(|n| !n.trim().is_empty()),
        ..NcrInput::default()
    };
    let facts = match ncr.lot() {
        Some(lot) => lot_disposal(state).facts(state, &lot).await?,
        None => None,
    };
    let mut problems = Vec::new();
    let needs_lot = || english("manufacturing.ncr.needs_lot", &[]);
    match ask.disposition {
        Disposition::Rework => {
            if let Some(line) = ask.line_id {
                let found = Line::find(&state.db, line).await?;
                if !found
                    .as_ref()
                    .is_some_and(|l| l.plant_id == ncr.plant_id && l.active)
                {
                    problems.push(english("manufacturing.create.line", &[]));
                }
            }
            input.rework_line_id = ask.line_id;
            input.rework_bom_id = ask.bom_id;
            input.outcome = None;
        }
        Disposition::Scrap => {
            match &facts {
                None => problems.push(needs_lot()),
                Some(facts) => {
                    if ncr.quantity_milli > facts.quantity_milli {
                        let left = crate::screens::quantity_text(facts.quantity_milli);
                        problems.push(english(
                            "manufacturing.ncr.too_much",
                            &[("available", &left)],
                        ));
                    }
                    input.scrap_value = value_of(ncr.quantity_milli, facts.unit_cost);
                }
            }
            input.outcome = None;
        }
        Disposition::UseAsIs => {
            match &facts {
                None => problems.push(needs_lot()),
                Some(facts) => {
                    if !["quarantine", "on_hold", "released"].contains(&facts.status.as_str()) {
                        problems.push(english("manufacturing.ncr.use_as_is_status", &[]));
                    }
                }
            }
            input.outcome = None;
        }
        Disposition::ReturnToSupplier => {
            match &facts {
                None => problems.push(needs_lot()),
                Some(facts) => {
                    if facts.produced_by.is_some() {
                        problems.push(english("manufacturing.ncr.own_production", &[]));
                    } else if facts.supplier.is_none() || facts.supplier_lot.is_none() {
                        problems.push(english("manufacturing.ncr.no_supplier_lot", &[]));
                    }
                    if ncr.quantity_milli > facts.quantity_milli {
                        let left = crate::screens::quantity_text(facts.quantity_milli);
                        problems.push(english(
                            "manufacturing.ncr.too_much",
                            &[("available", &left)],
                        ));
                    }
                }
            }
            if input.outcome.as_deref().and_then(outcome_of).is_none() {
                problems.push(english("manufacturing.ncr.outcome", &[]));
            }
        }
    }
    if !problems.is_empty() {
        return Err(refused(problems.join("\n")));
    }
    Ok(input)
}

/// QC decides what becomes of open NCR `id` (see the module). Rework, return
/// and a scrap up to AED 2,000 run at once; a larger scrap and *use as is*
/// wait for the approval, and nothing changes until it is given.
///
/// # Errors
///
/// `Forbidden` without `ncrs.disposition`; a 422 when the NCR is not `open`
/// or waits for an approval already, or a check of the disposition fails
/// (*return to supplier* of a lot with no supplier lot or made in-house, no
/// outcome, more than the lot holds).
pub async fn disposition(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    ask: DispositionAsk,
) -> Result<Dispositioned> {
    let ncr = load(state, id).await?;
    allow(&ncr, by, permissions::NCRS_DISPOSITION)?;
    if ncr.status != NcrStatus::Open {
        return Err(refused(english("manufacturing.ncr.not_open", &[])));
    }
    if waiting(state, &ncr).await? {
        return Err(refused(english("manufacturing.ncr.waiting", &[])));
    }
    let input = check(state, by, &ncr, &ask).await?;
    let name = match ask.disposition {
        Disposition::Scrap => DISPOSITION_SCRAP,
        Disposition::UseAsIs => DISPOSITION_USE_AS_IS,
        _ => DISPOSITION,
    };
    let outcome = ncr
        .transition(name)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(answer)?;
    announce(state, &outcome).await?;
    let approval_request = match &outcome {
        Outcome::Pending(pending) => pending
            .data
            .get(approvals::submit::REQUEST_KEY)
            .and_then(renox::serde_json::Value::as_i64),
        Outcome::Done(_) => None,
    };
    let done = matches!(outcome, Outcome::Done(_));
    outcome.emit(state).await?;
    if done {
        close_task(state, &redisposition_key(id)).await?;
        if let Err(error) = perform(state, id).await {
            let reason = match &error {
                Error::Status(_, message) => message.clone(),
                other => other.to_string(),
            };
            reopen_by_system(state, id, &reason).await?;
            return Err(match error {
                Error::Status(code, message) => Error::Status(code, message),
                other => refused(other.to_string()),
            });
        }
    }
    Ok(Dispositioned {
        ncr: load(state, id).await?,
        approval_request,
        scrap_value: input.scrap_value,
    })
}

/// Does what the disposition of dispositioned NCR `id` asks, once (a claim
/// makes a second run a no-op): creates the rework work order or the draft
/// Return to supplier, posts the scrap, releases the lot. Answers whether
/// this run did it.
///
/// # Errors
///
/// The kernel's and the database's errors; the claim is given back, so the
/// sweep or the next run tries again.
pub async fn perform(state: &AppState, id: i64) -> Result<bool> {
    let ncr = load(state, id).await?;
    let Some(kind) = ncr
        .disposition
        .filter(|_| ncr.status == NcrStatus::Dispositioned)
    else {
        return Ok(false);
    };
    let claimed = Ncr::query()
        .where_(Ncr::ID.eq(id))
        .where_(Ncr::ACTION_CLAIMED_AT.is_null())
        .update(&state.db, &[("action_claimed_at", &Some(now()))])
        .await?;
    if claimed == 0 {
        return Ok(false);
    }
    if let Err(error) = act(state, &ncr, kind).await {
        let none: Option<renox::db::DateTime> = None;
        Ncr::query()
            .where_(Ncr::ID.eq(id))
            .update(&state.db, &[("action_claimed_at", &none)])
            .await?;
        return Err(error);
    }
    let after = load(state, id).await?;
    state
        .emit(NcrDispositioned {
            ncr: after.subject(),
            disposition: kind.as_str().to_owned(),
            outcome: after.outcome.clone(),
            quantity_milli: after.quantity_milli,
            return_to_supplier: after.supplier_return(),
            rework_work_order: after
                .rework_work_order_id
                .map(|id| Subject::new(crate::work_order::SUBJECT_TYPE, id)),
        })
        .await?;
    if after.action_done_at.is_some() {
        ask_to_close(state, &after).await?;
    }
    Ok(true)
}

/// The action of `kind` for `ncr`.
async fn act(state: &AppState, ncr: &Ncr, kind: Disposition) -> Result {
    let stock = lot_disposal(state);
    let reason = ncr.defect.clone();
    match kind {
        Disposition::Rework => {
            let order = work_orders::create_by_system(
                state,
                NewWorkOrder {
                    plant_id: ncr.plant_id,
                    line_id: ncr.rework_line_id,
                    item_id: ncr.item_id,
                    bom_id: ncr.rework_bom_id,
                    quantity_milli: ncr.quantity_milli,
                    planned_start: Some(today()),
                    planned_end: None,
                    sales_order: None,
                },
                ncr.dispositioned_by,
            )
            .await?;
            Ncr::query()
                .where_(Ncr::ID.eq(ncr.id))
                .update(
                    &state.db,
                    &[
                        ("rework_work_order_id", &Some(order.id)),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
            let planners = notices::holders(state, permissions::PLANNER_ROLE, ncr.plant_id).await?;
            notices::tell(
                state,
                ncr.subject(),
                &ncr.number,
                &planners,
                &english("manufacturing.ncr.rework_title", &[("ncr", &ncr.number)]),
                &english(
                    "manufacturing.ncr.rework_body",
                    &[("ncr", &ncr.number), ("order", &order.number)],
                ),
            )
            .await?;
        }
        Disposition::ReturnToSupplier => {
            let lot = ncr
                .lot()
                .ok_or_else(|| refused(english("manufacturing.ncr.needs_lot", &[])))?;
            let outcome = ncr
                .outcome
                .as_deref()
                .and_then(outcome_of)
                .ok_or_else(|| refused(english("manufacturing.ncr.outcome", &[])))?;
            let made = stock
                .create_return(
                    state,
                    ReturnLot {
                        ncr: ncr.subject(),
                        lot,
                        quantity_milli: ncr.quantity_milli,
                        outcome,
                        reason,
                        user: ncr.dispositioned_by,
                    },
                )
                .await?
                .ok_or_else(|| refused(english("manufacturing.ncr.no_stock", &[])))?;
            Ncr::query()
                .where_(Ncr::ID.eq(ncr.id))
                .update(
                    &state.db,
                    &[
                        ("return_type", &Some(made.subject_type.clone())),
                        ("return_id", &Some(made.subject_id)),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
        }
        Disposition::Scrap => {
            let lot = ncr
                .lot()
                .ok_or_else(|| refused(english("manufacturing.ncr.needs_lot", &[])))?;
            let done = stock
                .scrap(
                    state,
                    ScrapLot {
                        ncr: ncr.subject(),
                        lot,
                        quantity_milli: ncr.quantity_milli,
                        reason,
                        user: ncr.dispositioned_by,
                    },
                )
                .await?
                .ok_or_else(|| refused(english("manufacturing.ncr.no_stock", &[])))?;
            let stamp = now();
            Ncr::query()
                .where_(Ncr::ID.eq(ncr.id))
                .update(
                    &state.db,
                    &[
                        ("scrap_value", &done.value),
                        ("action_done_at", &Some(stamp)),
                        ("updated_at", &stamp),
                    ],
                )
                .await?;
        }
        Disposition::UseAsIs => {
            let lot = ncr
                .lot()
                .ok_or_else(|| refused(english("manufacturing.ncr.needs_lot", &[])))?;
            // Done before the release: the guard of the lot keeps a lot with
            // an NCR whose action has not happened.
            let stamp = now();
            Ncr::query()
                .where_(Ncr::ID.eq(ncr.id))
                .update(
                    &state.db,
                    &[("action_done_at", &Some(stamp)), ("updated_at", &stamp)],
                )
                .await?;
            if !stock.release(state, &lot).await? {
                return Err(refused(english("manufacturing.ncr.use_as_is_status", &[])));
            }
        }
    }
    Ok(())
}

/// Whether the action of the disposition of `ncr` happened, read from the
/// work order or the Return to supplier when the mark is missing.
pub async fn action_done(state: &AppState, ncr: &Ncr) -> Result<bool> {
    if ncr.action_done_at.is_some() {
        return Ok(true);
    }
    match ncr.disposition {
        Some(Disposition::Rework) => Ok(match ncr.rework_work_order_id {
            Some(id) => WorkOrder::find(&state.db, id).await?.is_some_and(|order| {
                matches!(
                    order.status,
                    WorkOrderStatus::Completed | WorkOrderStatus::Closed
                )
            }),
            None => false,
        }),
        Some(Disposition::ReturnToSupplier) => Ok(match ncr.supplier_return() {
            Some(found) => lot_disposal(state)
                .return_status(state, &found)
                .await?
                .is_some_and(|status| status == "dispatched" || status == "credited"),
            None => false,
        }),
        _ => Ok(false),
    }
}

/// QC closes dispositioned NCR `id`: the action happened.
///
/// # Errors
///
/// `Forbidden` without `ncrs.disposition`; a 422 when the NCR is not
/// `dispositioned` or its action has not happened (a rework work order not
/// completed, a Return to supplier still `draft`).
pub async fn close(state: &AppState, by: &AuthUser, id: i64) -> Result<Ncr> {
    let ncr = load(state, id).await?;
    allow(&ncr, by, permissions::NCRS_DISPOSITION)?;
    let done = action_done(state, &ncr).await?;
    let input = NcrInput {
        by: Some(by.id),
        action_done: done,
        ..NcrInput::default()
    };
    let outcome = ncr
        .transition(CLOSE)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(answer)?;
    outcome.emit(state).await?;
    close_task(state, &close_key(id)).await?;
    state
        .emit(NcrClosed {
            ncr: ncr.subject(),
            disposition: ncr.disposition.map(|d| d.as_str().to_owned()),
        })
        .await?;
    load(state, id).await
}

/// QC reopens dispositioned NCR `id` for a new disposition while the action
/// did not happen.
///
/// # Errors
///
/// `Forbidden` without `ncrs.disposition`; a 422 without a reason, when the
/// action happened, or while the Return to supplier or the rework work order
/// of the disposition is still alive (cancel it, which reopens the NCR).
pub async fn reopen(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<Ncr> {
    let ncr = load(state, id).await?;
    allow(&ncr, by, permissions::NCRS_DISPOSITION)?;
    if let Some(found) = ncr.supplier_return()
        && let Some(status) = lot_disposal(state).return_status(state, &found).await?
        && status != "cancelled"
    {
        return Err(refused(english("manufacturing.ncr.return_alive", &[])));
    }
    if let Some(order) = ncr.rework_work_order_id
        && let Some(order) = WorkOrder::find(&state.db, order).await?
        && !matches!(
            order.status,
            WorkOrderStatus::Cancelled | WorkOrderStatus::Aborted
        )
    {
        return Err(refused(english("manufacturing.ncr.rework_alive", &[])));
    }
    let input = NcrInput {
        by: Some(by.id),
        action_done: ncr.action_done_at.is_some(),
        ..NcrInput::default()
    };
    let outcome = ncr
        .transition(REOPEN)
        .by(by)
        .reason(reason.trim())
        .input(&input)
        .run(state)
        .await
        .map_err(answer)?;
    outcome.emit(state).await?;
    after_reopen(state, id, reason.trim()).await?;
    load(state, id).await
}

/// The system reopens dispositioned NCR `id`: its action did not happen.
/// Answers whether it did (a second call finds the NCR open).
///
/// # Errors
///
/// The database's errors.
pub async fn reopen_by_system(state: &AppState, id: i64, reason: &str) -> Result<bool> {
    let ncr = load(state, id).await?;
    if ncr.status != NcrStatus::Dispositioned {
        return Ok(false);
    }
    let input = NcrInput {
        action_done: false,
        ..NcrInput::default()
    };
    let outcome = ncr
        .transition(AUTO_REOPEN)
        .by_system()
        .reason(reason)
        .input(&input)
        .run(state)
        .await
        .map_err(answer)?;
    outcome.emit(state).await?;
    after_reopen(state, id, reason).await?;
    Ok(true)
}

/// QC is asked to choose a new disposition; the task to close goes.
async fn after_reopen(state: &AppState, id: i64, reason: &str) -> Result {
    let ncr = load(state, id).await?;
    close_task(state, &close_key(id)).await?;
    open_task(
        state,
        &ncr,
        redisposition_key(id),
        english(
            "manufacturing.ncr.redisposition_title",
            &[("ncr", &ncr.number)],
        ),
        english(
            "manufacturing.ncr.redisposition_body",
            &[("ncr", &ncr.number), ("reason", &reason)],
        ),
    )
    .await?;
    state
        .emit(NcrReopened {
            ncr: ncr.subject(),
            reason: reason.to_owned(),
        })
        .await
}

/// The NCR whose Return to supplier is `found`, if it is one of ours.
async fn of_return(state: &AppState, found: &Subject) -> Result<Option<Ncr>> {
    if found.subject_type != SUPPLIER_RETURN_SUBJECT_TYPE {
        return Ok(None);
    }
    Ncr::query()
        .where_(Ncr::RETURN_ID.eq(Some(found.subject_id)))
        .where_(Ncr::STATUS.eq(NcrStatus::Dispositioned))
        .first(&state.db)
        .await
}

/// A Return to supplier with an NCR as its source was dispatched: the goods
/// left, the lot is rejected when nothing is left of it, and QC is asked to
/// close the NCR. Idempotent.
///
/// # Errors
///
/// The database's and the kernel's errors.
pub async fn on_return_dispatched(state: &AppState, event: &SupplierReturnDispatched) -> Result {
    let Some(ncr) = of_return(state, &event.supplier_return).await? else {
        return Ok(());
    };
    if ncr.action_done_at.is_some() {
        return Ok(());
    }
    let stamp = now();
    Ncr::query()
        .where_(Ncr::ID.eq(ncr.id))
        .where_(Ncr::ACTION_DONE_AT.is_null())
        .update(
            &state.db,
            &[("action_done_at", &Some(stamp)), ("updated_at", &stamp)],
        )
        .await?;
    if let Some(lot) = ncr.lot() {
        lot_disposal(state).close_out(state, &lot).await?;
    }
    ask_to_close(state, &load(state, ncr.id).await?).await
}

/// The rework work order of an NCR completed (QC is asked to close it) or was
/// cancelled or aborted (the NCR is reopened). Idempotent.
///
/// # Errors
///
/// The database's and the ticketing's errors.
pub async fn on_work_order(state: &AppState, id: i64, to: &str) -> Result {
    let Some(ncr) = Ncr::query()
        .where_(Ncr::REWORK_WORK_ORDER_ID.eq(Some(id)))
        .where_(Ncr::STATUS.eq(NcrStatus::Dispositioned))
        .first(&state.db)
        .await?
    else {
        return Ok(());
    };
    if to == WorkOrderStatus::Completed.as_str() {
        let stamp = now();
        Ncr::query()
            .where_(Ncr::ID.eq(ncr.id))
            .where_(Ncr::ACTION_DONE_AT.is_null())
            .update(
                &state.db,
                &[("action_done_at", &Some(stamp)), ("updated_at", &stamp)],
            )
            .await?;
        ask_to_close(state, &load(state, ncr.id).await?).await
    } else if to == WorkOrderStatus::Cancelled.as_str() || to == WorkOrderStatus::Aborted.as_str() {
        let key = if to == WorkOrderStatus::Cancelled.as_str() {
            "manufacturing.ncr.rework_cancelled"
        } else {
            "manufacturing.ncr.rework_aborted"
        };
        reopen_by_system(state, ncr.id, &english(key, &[])).await?;
        Ok(())
    } else {
        Ok(())
    }
}

/// The Return to supplier of an NCR was cancelled: the NCR is reopened.
///
/// # Errors
///
/// The database's errors.
pub async fn on_return_cancelled(state: &AppState, found: &Subject) -> Result {
    if let Some(ncr) = of_return(state, found).await? {
        reopen_by_system(
            state,
            ncr.id,
            &english("manufacturing.ncr.return_cancelled", &[]),
        )
        .await?;
    }
    Ok(())
}

/// The NCRs whose action did not run (a failure left the claim free): tried
/// again.
///
/// # Errors
///
/// As [`perform`], for the first that fails.
pub async fn sweep(state: &AppState) -> Result<usize> {
    let mut done = 0;
    for ncr in Ncr::query()
        .where_(Ncr::STATUS.eq(NcrStatus::Dispositioned))
        .where_(Ncr::ACTION_CLAIMED_AT.is_null())
        .where_(Ncr::ACTION_DONE_AT.is_null())
        .order_by(Ncr::ID)
        .get(&state.db)
        .await?
    {
        if perform(state, ncr.id).await? {
            done += 1;
        }
    }
    Ok(done)
}

/// Keeps a lot from release while an NCR is open on it, or dispositioned with
/// its action not done (§7.1 *NCR*). Provided to wl-inventory.
pub struct NcrReleaseGuard;

impl LotReleaseGuard for NcrReleaseGuard {
    fn blocked<'a>(&'a self, db: &'a Db, lot: &'a Subject) -> LotFuture<'a, Option<String>> {
        Box::pin(async move {
            if lot.subject_type != wl_kernel::inventory::LOT_SUBJECT_TYPE {
                return Ok(None);
            }
            let kept = Ncr::query()
                .where_(Ncr::LOT_ID.eq(Some(lot.subject_id)))
                .order_by(Ncr::ID)
                .get(db)
                .await?
                .into_iter()
                .find(Ncr::keeps_lot);
            Ok(kept.map(|ncr| {
                english(
                    "manufacturing.ncr.keeps_lot",
                    &[("ncr", &ncr.number), ("status", &ncr.status.as_str())],
                )
            }))
        })
    }
}

/// The decisions an NCR had, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn decisions(db: &Db, id: i64) -> Result<Vec<NcrDecision>> {
    NcrDecision::query()
        .where_(NcrDecision::NCR_ID.eq(id))
        .order_by(NcrDecision::ID)
        .get(db)
        .await
}

/// The listeners.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(|event: Transitioned<Ncr>, state: AppState| async move {
        let approved_scrap = approved(DISPOSITION_SCRAP);
        let approved_as_is = approved(DISPOSITION_USE_AS_IS);
        if event.transition == approved_scrap || event.transition == approved_as_is {
            close_task(&state, &redisposition_key(event.key)).await?;
            if let Err(error) = perform(&state, event.key).await {
                let reason = match &error {
                    Error::Status(_, message) => message.clone(),
                    other => other.to_string(),
                };
                reopen_by_system(&state, event.key, &reason).await?;
            }
        }
        Ok(())
    });
    app.listen(
        |event: SupplierReturnDispatched, state: AppState| async move {
            on_return_dispatched(&state, &event).await
        },
    );
    app.listen(
        |event: wl_kernel::lot_disposal::SupplierReturnCancelled, state: AppState| async move {
            on_return_cancelled(&state, &event.supplier_return).await
        },
    );
    app.listen(
        |event: Transitioned<WorkOrder>, state: AppState| async move {
            on_work_order(&state, event.key, &event.to).await
        },
    );
}
