//! Parts issued to a job card and returned to stock (08 §4.4 step 5,
//! `STK.MOV.01`; the return of what is not fitted, `AUT.ACL.21`).
//!
//! The kernel's stock service does the stock: [`issue`] takes an approved part
//! line from the workshop's store with a `consume` against the job card (what
//! the store cannot give becomes a pending issue in wl-inventory, so the work
//! is never blocked), [`fit`] records what went onto the vehicle, and
//! [`return_part`] and [`return_unfitted`] give back what was issued and not
//! fitted with a `return_in` movement.
//!
//! Decisions:
//!
//! - The workshop's store is `StockService::store_of` of the job card's
//!   workshop (its scope); a line without an item code, a job card without a
//!   workshop, or an item the store does not know, is a charge only: the
//!   quantity is recorded as handed to the job and nothing posts to stock.
//! - One run issues a quantity: the line is claimed with the quantity it held
//!   (an optimistic lock) before the stock moves, and the claim is undone when
//!   the stock fails, so a double click never issues twice.
//! - What is returned comes out of the posted quantity first: a line issued 3
//!   of which 2 posted and 1 is short, with 1 fitted, gives back 2 to stock
//!   and 0 when more is not posted.
//! - When the last outstanding part of a job card that waits for parts is
//!   issued, the machine's `parts_arrived` runs as the system.

use renox::db::now;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::inventory::{ConsumeRequest, LOCATION_SUBJECT_TYPE, ReturnRequest, stock_service};

use crate::db::refused;
use crate::estimate::Estimate;
use crate::job_card::{JobCard, JobCardStatus};
use crate::job_card_machine::{JobInput, PARTS_ARRIVED};
use crate::job_cards::{may_work, run_system};
use crate::labour_line::LineStatus;
use crate::part_line::PartLine;
use crate::permissions;
use crate::texts::english;

/// The store of the workshop of `card`: the inventory location its parts come
/// from; `None` when the job card has no workshop or the workshop no store.
///
/// # Errors
///
/// The inventory's errors.
pub async fn store_of(state: &AppState, card: &JobCard) -> Result<Option<Subject>> {
    let (Some(kind), Some(id)) = (&card.workshop_type, card.workshop_id) else {
        return Ok(None);
    };
    stock_service(state).store_of(state, kind, id).await
}

/// Whether `by` may issue parts to `card`: whoever works it, or holds the
/// stock permission of the storekeeper.
fn may_issue(by: &AuthUser, card: &JobCard) -> bool {
    may_work(by, card) || by.has_permission(permissions::STOCK_ISSUE)
}

/// Issues `quantity_milli` of the approved part line `line_id` to its job
/// card, the whole outstanding quantity when `None`.
///
/// # Errors
///
/// A 403 for someone who may not issue parts to the job card; a 422 for a line
/// the customer has not approved, a job card that is not under way, or a
/// quantity that is not above zero or above what is outstanding.
pub async fn issue(
    state: &AppState,
    by: &AuthUser,
    line_id: i64,
    quantity_milli: Option<i64>,
) -> Result<PartLine> {
    let line = PartLine::find_or_404(&state.db, line_id).await?;
    let card = JobCard::find_or_404(&state.db, line.job_card_id).await?;
    if !may_issue(by, &card) {
        return Err(Error::Forbidden);
    }
    if line.approval != LineStatus::Approved
        || !matches!(
            card.status,
            JobCardStatus::InProgress | JobCardStatus::AwaitingParts
        )
    {
        return Err(refused(english("auto.parts.problem.not_now", &[])));
    }
    let quantity = quantity_milli.unwrap_or_else(|| line.outstanding_milli());
    if quantity <= 0 || quantity > line.outstanding_milli() {
        return Err(refused(english("auto.parts.problem.quantity", &[])));
    }
    // The claim: the line holds what it held when it was read.
    let claimed = PartLine::query()
        .where_(PartLine::ID.eq(line.id))
        .where_(PartLine::ISSUED_MILLI.eq(line.issued_milli))
        .update(
            &state.db,
            &[
                ("issued_milli", &(line.issued_milli + quantity)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    if claimed == 0 {
        return Err(refused(english("auto.parts.problem.changed", &[])));
    }
    let posted = match post(state, &card, &line, quantity).await {
        Ok(posted) => posted,
        Err(error) => {
            PartLine::query()
                .where_(PartLine::ID.eq(line.id))
                .update(
                    &state.db,
                    &[("issued_milli", &line.issued_milli), ("updated_at", &now())],
                )
                .await?;
            return Err(error);
        }
    };
    if posted.store.is_some() {
        PartLine::query()
            .where_(PartLine::ID.eq(line.id))
            .update(
                &state.db,
                &[
                    ("posted_milli", &(line.posted_milli + posted.posted_milli)),
                    ("short_milli", &(line.short_milli + posted.short_milli)),
                    (
                        "store_location_id",
                        &posted.store.as_ref().map(|store| store.subject_id),
                    ),
                    ("updated_at", &now()),
                ],
            )
            .await?;
    }
    parts_arrived(state, card.id).await?;
    PartLine::find_or_404(&state.db, line.id).await
}

/// What the stock did for an issue.
struct Posted {
    /// The store the part was taken from; `None` for a charge only.
    store: Option<Subject>,
    /// The quantity that posted.
    posted_milli: i64,
    /// The quantity the store could not give.
    short_milli: i64,
}

/// Takes `quantity` of `line` from the workshop's store for `card`.
async fn post(state: &AppState, card: &JobCard, line: &PartLine, quantity: i64) -> Result<Posted> {
    let charge_only = Posted {
        store: None,
        posted_milli: 0,
        short_milli: 0,
    };
    let Some(code) = &line.item_code else {
        return Ok(charge_only);
    };
    let Some(store) = store_of(state, card).await? else {
        return Ok(charge_only);
    };
    let service = stock_service(state);
    let Some(choice) = service.choices(state, code, &store).await? else {
        return Ok(charge_only);
    };
    let request = ConsumeRequest::new(choice.item, store.clone(), quantity, card.subject())
        .cost_object(card.subject());
    let consumed = service.consume(state, request).await?;
    Ok(Posted {
        store: Some(store),
        posted_milli: consumed.posted_milli,
        short_milli: consumed.short_milli,
    })
}

/// A job card that waited for parts goes on when every approved part is
/// issued (`parts_arrived`, as the system).
async fn parts_arrived(state: &AppState, job_card_id: i64) -> Result {
    let card = JobCard::find_or_404(&state.db, job_card_id).await?;
    if card.status != JobCardStatus::AwaitingParts {
        return Ok(());
    }
    let estimate = Estimate::load(&state.db, card.id).await?;
    let all_in = estimate
        .parts
        .iter()
        .filter(|line| line.approval == LineStatus::Approved)
        .all(|line| line.outstanding_milli() == 0);
    if all_in {
        run_system(state, &card, PARTS_ARRIVED, None, JobInput::default()).await?;
    }
    Ok(())
}

/// Records that `quantity_milli` of the part line `line_id` was fitted to the
/// vehicle, the whole quantity not yet fitted when `None`.
///
/// # Errors
///
/// A 403 for someone who may not work the job card; a 422 for a quantity that
/// is not above zero or above what was issued and not fitted or given back.
pub async fn fit(
    state: &AppState,
    by: &AuthUser,
    line_id: i64,
    quantity_milli: Option<i64>,
) -> Result<PartLine> {
    let line = PartLine::find_or_404(&state.db, line_id).await?;
    let card = JobCard::find_or_404(&state.db, line.job_card_id).await?;
    if !may_work(by, &card) {
        return Err(Error::Forbidden);
    }
    if card.status != JobCardStatus::InProgress {
        return Err(refused(english("auto.parts.problem.not_now", &[])));
    }
    let quantity = quantity_milli.unwrap_or_else(|| line.unfitted_milli());
    if quantity <= 0 || quantity > line.unfitted_milli() {
        return Err(refused(english("auto.parts.problem.fit_quantity", &[])));
    }
    PartLine::query()
        .where_(PartLine::ID.eq(line.id))
        .update(
            &state.db,
            &[
                ("fitted_milli", &(line.fitted_milli + quantity)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    PartLine::find_or_404(&state.db, line.id).await
}

/// Gives back what was issued of the part line `line` and not fitted: the
/// posted quantity returns to the store with a `return_in` movement.
async fn give_back(state: &AppState, card: &JobCard, line: &PartLine) -> Result<PartLine> {
    let quantity = line.unfitted_milli();
    if quantity <= 0 {
        return Ok(line.clone());
    }
    let in_stock = quantity.min((line.posted_milli - line.returned_milli).max(0));
    let place = (line.item_code.clone(), line.store_location_id);
    if in_stock > 0 {
        if let (Some(code), Some(store)) = place {
            let location = Subject::new(LOCATION_SUBJECT_TYPE, store);
            let service = stock_service(state);
            let found = service.choices(state, &code, &location).await?;
            if let Some(choice) = found {
                let request = ReturnRequest::new(choice.item, location, in_stock, card.subject())
                    .cost_object(card.subject())
                    .reason("not fitted");
                service.return_to_stock(state, request).await?;
            }
        }
    }
    PartLine::query()
        .where_(PartLine::ID.eq(line.id))
        .update(
            &state.db,
            &[
                ("returned_milli", &(line.returned_milli + quantity)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    PartLine::find_or_404(&state.db, line.id).await
}

/// Gives back what was issued of the part line `line_id` and not fitted.
///
/// # Errors
///
/// A 403 for someone who may not issue parts to the job card; a 422 for a job
/// card that is over.
pub async fn return_part(state: &AppState, by: &AuthUser, line_id: i64) -> Result<PartLine> {
    let line = PartLine::find_or_404(&state.db, line_id).await?;
    let card = JobCard::find_or_404(&state.db, line.job_card_id).await?;
    if !may_issue(by, &card) {
        return Err(Error::Forbidden);
    }
    if card.status.is_closed() && card.status != JobCardStatus::Cancelled {
        return Err(refused(english("auto.parts.problem.over", &[])));
    }
    give_back(state, &card, &line).await
}

/// Gives back every part of the job card that was issued and not fitted: what
/// a stopped job does before it bills (`AUT.ACL.21`). Returns the lines that
/// had something to give back.
///
/// # Errors
///
/// The database's and the inventory's errors.
pub async fn return_unfitted(state: &AppState, job_card_id: i64) -> Result<Vec<PartLine>> {
    let card = JobCard::find_or_404(&state.db, job_card_id).await?;
    let estimate = Estimate::load(&state.db, card.id).await?;
    let mut back = Vec::new();
    for line in estimate.parts.iter().filter(|l| l.unfitted_milli() > 0) {
        back.push(give_back(state, &card, line).await?);
    }
    Ok(back)
}
