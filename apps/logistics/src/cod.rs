//! A COD difference: who absorbs it and what is posted (03 Logistics §4.1
//! row *Recipient pays less than the COD amount or refuses to pay*, step 11,
//! §6, §7.2 `LOG.RUL.06`, `LOG.COD.01`, `WRK.ACL.09`, §9; BL-LOG-08).
//!
//! The driver enters the cash collected and a reason at the stop and asks the
//! operations manager from the phone; the approval is wl-delivery's (#159).
//! The operations manager must choose who absorbs the difference, and this
//! module is the shipment's rule on the choice ([`CodRule`], provided to
//! wl-delivery through the kernel's [`CodDifferenceRule`]):
//!
//! - **`customer`** is not offered: the courier does not invoice the
//!   recipient, so no invoice balance is ever made for the recipient.
//! - **`client`** only on the client's instruction (who, when, how), given
//!   with the approval or recorded earlier on the shipment. Without it the
//!   approval is refused. The difference is deducted on the client's
//!   remittance line with its reason ([`crate::remit::figures`]).
//! - **`business`**: the client still gets the full COD; a COD loss
//!   ([`CodLoss`]: shipment, amount, reason, approver) is recorded for the
//!   depot inside the shipment's `deliver`.
//! - **`driver`**: nothing is posted here; the full COD stays expected in the
//!   driver's cash handover (wl-delivery) and a short count follows the cash
//!   shortage flow (BL-PAY-17).
//!
//! The shipment stores the COD amount, the cash collected, the difference,
//! the reason, who absorbs it, the approver and the instruction. The report
//! *COD differences by who absorbed them* ([`report`], the page
//! `GET /cod-differences`) lists the outcomes.

use std::collections::BTreeMap;

use renox::db::{DateTime, Transaction};
use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::delivery::{CodAbsorber, CodDecision, CodDifferenceRule, DeliveryFuture};

use crate::pages::{Texts, tx};
use crate::permissions;
use crate::shipment::{SUBJECT_TYPE, Shipment, track};
use crate::shipment_machine::ShipmentInput;
use crate::texts::english;

/// The templates, compiled in.
pub(crate) const VIEWS: &[(&str, &str)] = &[(
    "logistics/cod_differences.html",
    include_str!("../views/logistics/cod_differences.html"),
)];

/// A COD loss: the difference the business absorbed on a shipment. The
/// client is still remitted the full COD.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_cod_losses", unique(shipment_id))]
pub struct CodLoss {
    /// The primary key.
    pub id: i64,
    /// The shipment.
    #[model(references = "logistics_shipments")]
    pub shipment_id: i64,
    /// The client whose COD it was.
    pub client_party_id: i64,
    /// The depot that bears it; none until the depot is a scope unit.
    pub depot: Option<String>,
    /// The difference, in fils.
    pub amount: i64,
    /// The reason of the difference.
    pub reason: String,
    /// The approver who chose the business.
    pub approved_by: Option<i64>,
    /// When it was recorded.
    pub recorded_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

fn refused(message: String) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)
}

/// The shipment's rule on who absorbs a COD difference of its delivery.
pub struct CodRule;

impl CodDifferenceRule for CodRule {
    fn check<'a>(&'a self, state: &'a AppState, decision: CodDecision) -> DeliveryFuture<'a, ()> {
        Box::pin(async move { check(state, &decision).await })
    }
}

/// Checks the approver's choice for a shipment's delivery, and records the
/// approver and, for *client*, the instruction on the shipment.
async fn check(state: &AppState, decision: &CodDecision) -> Result {
    if decision.source.subject_type != SUBJECT_TYPE {
        return Ok(());
    }
    let shipment = Shipment::find_or_404(&state.db, decision.source.subject_id).await?;
    let instruction = decision.instruction.trim();
    match decision.absorber {
        CodAbsorber::Customer => {
            return Err(refused(english("logistics.cod.customer_not_offered", &[])));
        }
        CodAbsorber::Client => {
            if instruction.is_empty() && !shipment.has_cod_instruction() {
                return Err(refused(english("logistics.cod.instruction_needed", &[])));
            }
        }
        CodAbsorber::Business | CodAbsorber::Driver => {}
    }
    let approver = Some(decision.approver);
    Shipment::query()
        .where_(Shipment::ID.eq(shipment.id))
        .update(&state.db, &[("cod_approved_by", &approver)])
        .await?;
    if decision.absorber == CodAbsorber::Client && !instruction.is_empty() {
        let text = Some(instruction.to_owned());
        Shipment::query()
            .where_(Shipment::ID.eq(shipment.id))
            .update(
                &state.db,
                &[
                    ("cod_instruction", &text),
                    ("cod_instruction_by", &approver),
                ],
            )
            .await?;
        track(
            &state.db,
            shipment.id,
            "cod_instruction",
            instruction,
            Some(decision.approver),
        )
        .await?;
    }
    Ok(())
}

/// Records the COD loss of a difference the business absorbs, inside the
/// transaction of the shipment's `deliver`; nothing for any other absorber.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn record_loss(
    tx: &mut Transaction,
    shipment: &Shipment,
    input: &ShipmentInput,
) -> Result {
    if input.absorbed_by.as_deref() != Some(CodAbsorber::Business.as_str())
        || input.cod_difference <= 0
    {
        return Ok(());
    }
    let mut loss = CodLoss {
        shipment_id: shipment.id,
        client_party_id: shipment.client_party_id,
        amount: input.cod_difference,
        reason: input.cod_difference_reason.clone().unwrap_or_default(),
        approved_by: shipment.cod_approved_by,
        recorded_at: renox::db::now(),
        ..CodLoss::default()
    };
    loss.insert(&mut *tx).await?;
    Ok(())
}

/// The COD loss recorded for `shipment`, if any.
///
/// # Errors
///
/// Database errors.
pub async fn loss_of(db: &Db, shipment: i64) -> Result<Option<CodLoss>> {
    CodLoss::query()
        .where_(CodLoss::SHIPMENT_ID.eq(shipment))
        .first(db)
        .await
}

/// What the business absorbed so far, in fils.
///
/// # Errors
///
/// Database errors.
pub async fn losses_total(db: &Db) -> Result<i64> {
    Ok(CodLoss::query()
        .get(db)
        .await?
        .iter()
        .map(|loss| loss.amount)
        .sum())
}

/// The total of one absorber.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct ByAbsorber {
    /// `client`, `business`, `driver`, `customer` or `none` (no absorber
    /// known: a difference within the tolerance).
    pub absorber: String,
    /// How many shipments.
    pub count: i64,
    /// The sum of their differences, in fils.
    pub total: i64,
}

/// One shipment with a COD difference.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct DifferenceRow {
    /// The shipment.
    pub shipment_id: i64,
    /// Its number.
    pub number: String,
    /// The client.
    pub client_party_id: i64,
    /// The COD amount, in fils.
    pub cod_amount: i64,
    /// The cash collected, in fils.
    pub collected: i64,
    /// The difference, in fils.
    pub difference: i64,
    /// Its reason.
    pub reason: String,
    /// Who absorbed it.
    pub absorbed_by: String,
    /// The approver.
    pub approved_by: Option<i64>,
    /// The client's instruction, for *client*.
    pub instruction: String,
}

/// The report *COD differences by who absorbed them*: the totals per absorber
/// and the shipments, oldest first.
///
/// # Errors
///
/// Database errors.
pub async fn report(db: &Db) -> Result<(Vec<ByAbsorber>, Vec<DifferenceRow>)> {
    let shipments = Shipment::query()
        .where_(Shipment::COD_DIFFERENCE.gt(0))
        .order_by(Shipment::ID)
        .get(db)
        .await?;
    let mut totals: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut rows = Vec::new();
    for shipment in shipments {
        let absorbed_by = shipment
            .cod_absorbed_by
            .clone()
            .unwrap_or_else(|| "none".to_owned());
        let entry = totals.entry(absorbed_by.clone()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += shipment.cod_difference;
        rows.push(DifferenceRow {
            shipment_id: shipment.id,
            number: shipment.number.clone(),
            client_party_id: shipment.client_party_id,
            cod_amount: shipment.cod_amount,
            collected: shipment.cod_collected,
            difference: shipment.cod_difference,
            reason: shipment.cod_difference_reason.clone().unwrap_or_default(),
            absorbed_by,
            approved_by: shipment.cod_approved_by,
            instruction: shipment.cod_instruction.clone().unwrap_or_default(),
        });
    }
    let totals = totals
        .into_iter()
        .map(|(absorber, (count, total))| ByAbsorber {
            absorber,
            count,
            total,
        })
        .collect();
    Ok((totals, rows))
}

/// Sets the instruction of the client to receive less on shipment `id` (who,
/// when, how), without an approval; the approver can then choose *client*.
///
/// # Errors
///
/// `Forbidden` without `shipments.manage`; a blank note is refused.
pub async fn record_instruction(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    note: &str,
) -> Result<Shipment> {
    if !user.has_permission(permissions::SHIPMENTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    if note.trim().is_empty() {
        return Err(refused(english("logistics.cod.instruction_needed", &[])));
    }
    let shipment = Shipment::find_or_404(&state.db, id).await?;
    let text = Some(note.trim().to_owned());
    let by = Some(user.id);
    Shipment::query()
        .where_(Shipment::ID.eq(shipment.id))
        .update(
            &state.db,
            &[("cod_instruction", &text), ("cod_instruction_by", &by)],
        )
        .await?;
    track(&state.db, shipment.id, "cod_instruction", note.trim(), by).await?;
    Shipment::find_or_404(&state.db, id).await
}

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/cod-differences", index)
        .name("logistics.cod_differences")
        .post("/shipments/{id}/cod-instruction", instruction)
        .name("logistics.shipment_cod_instruction")
        .require_auth()
}

/// The COD differences grid: the shipments with a difference, searchable on
/// the number, client, reason and instruction, with sums of the money.
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let text = |key: &str| crate::pages::text(lang, key);
    let grid = Grid::new("logistics-cod-differences")
        .title(&text("cod_title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("number", &text("number"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/shipments/{id}"),
        )
        .column(
            Column::related(
                "client",
                &text("client"),
                "wl_parties_parties",
                "client_party_id",
                "display_name",
            )
            .searchable(),
        )
        .column(Column::money("cod_amount", &text("cod")).summary(Summary::Sum))
        .column(Column::money("cod_collected", &text("cod_collected")).summary(Summary::Sum))
        .column(
            Column::money("cod_difference", &text("cod_difference"))
                .mobile()
                .summary(Summary::Sum),
        )
        .column(Column::text("cod_absorbed_by", &text("cod_absorbed_by")).mobile())
        .column(
            Column::text("cod_difference_reason", &text("reason"))
                .searchable()
                .hidden(),
        )
        .column(
            Column::text("cod_instruction", &text("cod_instruction"))
                .searchable()
                .hidden(),
        )
        .sort_by("number")
        .row_url("/shipments/{id}")
        .cards_on_mobile()
        .empty_state(&text("cod_empty"), Some(&text("cod_empty_text")));
    if can_export { grid.exports() } else { grid }
}

/// What the report needs.
#[derive(Serialize, ViewData)]
#[view(template = "logistics/cod_differences.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    totals: Vec<ByAbsorber>,
    rows: GridPage<Shipment>,
    losses: i64,
}

/// `GET /cod-differences`: the report (`remittances.view`).
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    if !user.has_permission(permissions::REMITTANCES_VIEW) {
        return Err(Error::Forbidden);
    }
    let can_export = user.has_permission(permissions::REMITTANCES_PREPARE);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    let grid = grid(&lang, can_export);
    let query = Shipment::query().where_(Shipment::COD_DIFFERENCE.gt(0));
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let rows = grid.page(query, &request).await?;
    let (totals, _) = report(&state.db).await?;
    Ok(View::typed(IndexPage {
        tx: tx(&lang),
        totals,
        rows,
        losses: losses_total(&state.db).await?,
    })
    .into_response())
}

/// `POST /shipments/{id}/cod-instruction`: `note` (`shipments.manage`).
async fn instruction(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): crate::pages::Fields,
) -> Result<Response> {
    record_instruction(&state, &user, id, &crate::pages::field(&form, "note")).await?;
    Ok((
        Toast::success(crate::texts::t(&lang, "logistics.cod_instruction_recorded")),
        Redirect::route("logistics.shipment", &[&id])?,
    )
        .into_response())
}
