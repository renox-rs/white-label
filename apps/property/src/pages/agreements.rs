//! The management agreement pages: the list, an agreement with its units,
//! the handover pack and the actions on it (01 Property §4.4, §7.1
//! *Management agreement*; BL-PRP-22, BL-PRP-23).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /property/agreements` | `property.agreements` | `agreements.manage` |
//! | `POST /property/agreements` | `property.agreement_create` | `agreements.manage` |
//! | `GET /property/agreements/{id}` | `property.agreement` | `agreements.manage` |
//! | `POST /property/agreements/{id}/units` | `property.agreement_unit_add` | `agreements.manage` |
//! | `POST /property/agreements/{id}/activate` | `property.agreement_activate` | `agreements.manage` |
//! | `POST /property/agreements/{id}/give-notice` | `property.agreement_give_notice` | `agreements.manage` |
//! | `POST /property/agreements/{id}/receipts` | `property.agreement_receipt` | `agreements.manage` |
//! | `POST /property/agreements/{id}/final-statement` | `property.agreement_final_statement` | `owner_statements.prepare` |
//! | `POST /property/agreements/{id}/end` | `property.agreement_end` | `agreements.manage` |
//! | `POST /property/agreements/{id}/transfers` | `property.agreement_transfer_request` | `agreements.manage` |
//! | `POST /property/agreements/{id}/transfer-receipts` | `property.agreement_transfer_receipt` | `agreements.manage` |
//! | `POST /property/transfers/{id}/approve` | `property.transfer_approve` | `agreements.manage`, not the requester |
//! | `POST /property/transfers/{id}/reject` | `property.transfer_reject` | `agreements.manage` |
//!
//! The actions answer with a toast and a refresh, as the sheets post them by
//! htmx; a refusal answers 422 with its reason.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::documents::today;

use super::statements::owner_name;
use super::{Texts, text};
use crate::agreement::{
    self, AGREEMENT_SUBJECT_TYPE, AgreementStatus, AgreementUnit, ManagementAgreement, NewAgreement,
};
use crate::agreement_lifecycle::{self as lifecycle, HandoverItem, Notice, NoticeParty};
use crate::building::Building;
use crate::money_text;
use crate::parse::{parse_aed, parse_date};
use crate::permissions;
use crate::statement::OwnerStatement;
use crate::texts;
use crate::unit::Unit;
use crate::unit_transfer::{self, TransferKind, TransferRequest, TransferStatus, UnitTransfer};

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/property/agreements", index)
        .name("property.agreements")
        .post("/property/agreements", create)
        .name("property.agreement_create")
        .get("/property/agreements/{id}", show)
        .name("property.agreement")
        .post("/property/agreements/{id}/units", add_unit)
        .name("property.agreement_unit_add")
        .post("/property/agreements/{id}/activate", activate)
        .name("property.agreement_activate")
        .post("/property/agreements/{id}/give-notice", give_notice)
        .name("property.agreement_give_notice")
        .post("/property/agreements/{id}/receipts", receipt)
        .name("property.agreement_receipt")
        .post("/property/agreements/{id}/final-statement", final_statement)
        .name("property.agreement_final_statement")
        .post("/property/agreements/{id}/end", end)
        .name("property.agreement_end")
        .post("/property/agreements/{id}/transfers", request_transfer)
        .name("property.agreement_transfer_request")
        .post(
            "/property/agreements/{id}/transfer-receipts",
            transfer_receipt,
        )
        .name("property.agreement_transfer_receipt")
        .post("/property/transfers/{id}/approve", approve_transfer)
        .name("property.transfer_approve")
        .post("/property/transfers/{id}/reject", reject_transfer)
        .name("property.transfer_reject")
        .require_auth()
}

/// The badge kind of an agreement status.
pub(crate) fn status_kind(status: AgreementStatus) -> &'static str {
    match status {
        AgreementStatus::Active => "success",
        AgreementStatus::Ending => "warning",
        AgreementStatus::Draft | AgreementStatus::Ended => "info",
    }
}

/// The fee as people read it: `5.00%` and the fixed fee when there is one.
fn fee_text(agreement: &ManagementAgreement) -> String {
    let percent = format!("{}.{:02}%", agreement.fee_bp / 100, agreement.fee_bp % 100);
    if agreement.fee_fixed > 0 {
        format!("{percent} + {}", money_text(agreement.fee_fixed))
    } else {
        percent
    }
}

/// A 422 with `message`.
fn invalid(message: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.to_owned())
}

/// `user` must hold `agreements.manage`.
fn may_manage(user: &AuthUser) -> Result {
    if user.has_permission(permissions::AGREEMENTS_MANAGE) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// Where `user` holds `agreements.manage`: everywhere, or in some buildings
/// (403 in none). An agreement is in the buildings of its units.
async fn manage_reach(state: &AppState, user: &AuthUser) -> Result<wl_kernel::scopes::Reach> {
    wl_kernel::scopes::reach(&state.db, user, permissions::AGREEMENTS_MANAGE).await
}

/// `Forbidden` unless the agreement `id` is in `reach`.
async fn ensure_in_reach(
    state: &AppState,
    reach: &wl_kernel::scopes::Reach,
    id: i64,
) -> Result<()> {
    match reach.ids(&state.db, AGREEMENT_SUBJECT_TYPE).await? {
        Some(ids) if !ids.contains(&id) => Err(Error::Forbidden),
        _ => Ok(()),
    }
}

/// One agreement of the list.
#[derive(Serialize, Debug, Clone)]
pub struct AgreementRow {
    /// The primary key.
    pub id: i64,
    /// The number.
    pub number: String,
    /// The owner's name.
    pub owner: String,
    /// The fee.
    pub fee: String,
    /// The first day.
    pub start: String,
    /// The status in the page's language.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "property/agreements.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    agreements: Vec<AgreementRow>,
    today: String,
}

/// `GET /property/agreements`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    // The agreements of the user's buildings (all, for a company-wide holder).
    let reach = manage_reach(&state, &user).await?;
    let query = reach
        .apply_ids(
            &state.db,
            ManagementAgreement::query(),
            "id",
            AGREEMENT_SUBJECT_TYPE,
        )
        .await?;
    let mut rows = Vec::new();
    for found in query
        .order_by_desc(ManagementAgreement::ID)
        .get(&state.db)
        .await?
    {
        rows.push(AgreementRow {
            id: found.id,
            number: found.number(),
            owner: owner_name(&state, &lang, found.owner_party_id).await,
            fee: fee_text(&found),
            start: found.start_date.to_string(),
            status: text(
                &lang,
                &format!("agreements.status.{}", found.status.as_str()),
            ),
            status_kind: status_kind(found.status),
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        agreements: rows,
        today: today().to_string(),
    }))
}

/// The form that drafts an agreement.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CreateForm {
    /// The owner, a party's number.
    #[serde(default)]
    pub owner: String,
    /// The fee in percent of the rent collected, `5` or `5.25`.
    #[serde(default)]
    pub fee: String,
    /// The fixed fee per statement, in dirhams.
    #[serde(default)]
    pub fee_fixed: String,
    /// The first day.
    #[serde(default)]
    pub start: String,
}

/// `POST /property/agreements`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<CreateForm>,
) -> Result<Response> {
    may_manage(&user)?;
    let owner = super::party_of(&state, &form.owner, "owner").await?.0;
    let fee_bp = if form.fee.trim().is_empty() {
        0
    } else {
        // Percent with two decimals is basis points: 5.25 is 525.
        parse_aed(&form.fee)
            .and_then(|bp| i32::try_from(bp).ok())
            .ok_or_else(|| invalid("fee: 5.00"))?
    };
    let start = parse_date(&form.start).ok_or_else(|| invalid("start: YYYY-MM-DD"))?;
    let mut new = NewAgreement::new(PartyId(owner), start, fee_bp);
    if !form.fee_fixed.trim().is_empty() {
        new.fee_fixed = parse_aed(&form.fee_fixed).ok_or_else(|| invalid("fee_fixed: AED 0.00"))?;
    }
    agreement::create_draft(&state, Some(user.id), new).await?;
    Ok((
        Toast::success(text(&lang, "agreements.created")),
        renox::HxRefresh,
    )
        .into_response())
}

/// A unit of the agreement.
#[derive(Serialize, Debug, Clone)]
pub struct UnitRow {
    /// The building and the unit.
    pub unit: String,
    /// The day it comes in.
    pub date_in: String,
    /// The day it goes out.
    pub date_out: String,
}

/// A line of the handover pack.
#[derive(Serialize, Debug, Clone)]
pub struct PackLine {
    /// The primary key.
    pub id: i64,
    /// What it is.
    pub description: String,
    /// The amount.
    pub amount: String,
    /// The receipt's reference.
    pub receipt: String,
    /// Whether it was handed over.
    pub done: bool,
}

/// An open invoice of the pack.
#[derive(Serialize, Debug, Clone)]
pub struct InvoiceLine {
    /// The lease.
    pub lease: String,
    /// The due date.
    pub due: String,
    /// What is owed.
    pub balance: String,
}

/// An Ejari number of the pack.
#[derive(Serialize, Debug, Clone)]
pub struct EjariLine {
    /// The lease.
    pub lease: String,
    /// The number.
    pub number: String,
}

/// A statement of the agreement.
#[derive(Serialize, Debug, Clone)]
pub struct StatementLink {
    /// The primary key.
    pub id: i64,
    /// The number.
    pub number: String,
    /// The period.
    pub period: String,
    /// The status in the page's language.
    pub status: String,
}

/// A transfer of a unit out of the agreement.
#[derive(Serialize, Debug, Clone)]
pub struct TransferRow {
    /// The primary key.
    pub id: i64,
    /// The building and the unit.
    pub unit: String,
    /// The transfer date.
    pub date: String,
    /// What the unit becomes, in the page's language.
    pub kind: String,
    /// The new owner, for a sale.
    pub new_owner: String,
    /// The status in the page's language.
    pub status: String,
    /// The badge kind of the status.
    pub status_kind: &'static str,
    /// Whether it still waits for a decision.
    pub open: bool,
    /// Whether it is a sale (the new owner's deposit receipt is asked).
    pub sale: bool,
}

/// The agreement as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct AgreementView {
    /// The primary key.
    pub id: i64,
    /// The number.
    pub number: String,
    /// The owner's name.
    pub owner: String,
    /// The fee.
    pub fee: String,
    /// The first day.
    pub start: String,
    /// The notice, as words.
    pub notice: String,
    /// The handover date.
    pub handover: String,
    /// The day it ended.
    pub ended: String,
}

/// What an agreement page needs.
#[derive(Serialize, ViewData)]
#[view(template = "property/agreement.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    agreement: AgreementView,
    status: String,
    status_kind: &'static str,
    units: Vec<UnitRow>,
    unit_options: Vec<[String; 2]>,
    pack_deposits: Vec<PackLine>,
    pack_cheques: Vec<PackLine>,
    pack_invoices: Vec<InvoiceLine>,
    pack_ejari: Vec<EjariLine>,
    pack_lines: Vec<[String; 2]>,
    deposit_total: String,
    statements: Vec<StatementLink>,
    today: String,
    show_pack: bool,
    can_add_unit: bool,
    can_activate: bool,
    can_give_notice: bool,
    can_receipt: bool,
    can_final_statement: bool,
    can_end: bool,
    transfers: Vec<TransferRow>,
    open_transfers: Vec<TransferRow>,
    transfer_units: Vec<[String; 2]>,
    transfer_lines: Vec<[String; 2]>,
    can_transfer: bool,
}

/// The line of the pack as the page shows it.
fn pack_line(item: &HandoverItem) -> PackLine {
    PackLine {
        id: item.id,
        description: item.description.clone(),
        amount: money_text(item.amount),
        receipt: item.receipt.clone().unwrap_or_default(),
        done: item.done,
    }
}

/// The units of `agreement` as rows: building, number and the days.
async fn unit_rows(db: &Db, rows: &[AgreementUnit]) -> Result<Vec<UnitRow>> {
    let mut found = Vec::new();
    for row in rows {
        let label = match Unit::find(db, row.unit_id).await? {
            Some(unit) => {
                let building = Building::find(db, unit.building_id)
                    .await?
                    .map(|b| b.code)
                    .unwrap_or_default();
                format!("{building} {}", unit.number)
            }
            None => format!("#{}", row.unit_id),
        };
        found.push(UnitRow {
            unit: label,
            date_in: row.date_in.to_string(),
            date_out: row.date_out.map(|d| d.to_string()).unwrap_or_default(),
        });
    }
    Ok(found)
}

/// `GET /property/agreements/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let reach = manage_reach(&state, &user).await?;
    ensure_in_reach(&state, &reach, id).await?;
    let found = ManagementAgreement::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let rows = found.units(&state.db).await?;
    let units = unit_rows(&state.db, &rows).await?;
    let mut unit_options: Vec<[String; 2]> = Vec::new();
    for unit in Unit::query()
        .order_by(Unit::BUILDING_ID)
        .order_by(Unit::NUMBER)
        .get(&state.db)
        .await?
    {
        let building = Building::find(&state.db, unit.building_id)
            .await?
            .map(|b| b.code)
            .unwrap_or_default();
        unit_options.push([unit.id.to_string(), format!("{building} {}", unit.number)]);
    }
    let pack = lifecycle::pack(&state, &found).await?;
    let statements = OwnerStatement::of_agreement(&state.db, found.id)
        .await?
        .iter()
        .map(|s| StatementLink {
            id: s.id,
            number: s.number(),
            period: format!("{} → {}", s.period_start, s.period_end),
            status: text(&lang, &format!("statements.status.{}", s.status.as_str())),
        })
        .collect();
    let notice = match (&found.notice_party, found.notice_on) {
        (Some(party), Some(on)) => format!(
            "{} · {on}",
            text(&lang, &format!("agreements.party_{party}"))
        ),
        _ => String::new(),
    };
    let ending = found.status == AgreementStatus::Ending;
    let may_prepare = user.has_permission(permissions::OWNER_STATEMENTS_PREPARE);
    let view = AgreementView {
        id: found.id,
        number: found.number(),
        owner: owner_name(&state, &lang, found.owner_party_id).await,
        fee: fee_text(&found),
        start: found.start_date.to_string(),
        notice,
        handover: found
            .handover_date
            .map(|d| d.to_string())
            .unwrap_or_default(),
        ended: found.ended_on.map(|d| d.to_string()).unwrap_or_default(),
    };
    let pack_lines: Vec<[String; 2]> = pack
        .deposits
        .iter()
        .chain(pack.cheques.iter())
        .filter(|item| !item.done)
        .map(|item| [item.id.to_string(), item.description.clone()])
        .collect();
    let mut transfers = Vec::new();
    let mut transfer_lines: Vec<[String; 2]> = Vec::new();
    for transfer in UnitTransfer::of_agreement(&state.db, found.id).await? {
        let unit = match Unit::find(&state.db, transfer.unit_id).await? {
            Some(unit) => unit.number,
            None => format!("#{}", transfer.unit_id),
        };
        let new_owner = match transfer.new_owner_party_id {
            Some(owner) => owner_name(&state, &lang, owner).await,
            None => String::new(),
        };
        if transfer.is_open() && transfer.kind == TransferKind::Unmanaged {
            for item in HandoverItem::of_transfer(&state.db, transfer.id).await? {
                if !item.done {
                    transfer_lines.push([
                        item.id.to_string(),
                        format!("{unit} · {}", item.description),
                    ]);
                }
            }
        }
        transfers.push(TransferRow {
            id: transfer.id,
            unit,
            date: transfer.transfer_date.to_string(),
            kind: text(&lang, &format!("transfers.kind_{}", transfer.kind.as_str())),
            new_owner,
            status: text(
                &lang,
                &format!("transfers.status_{}", transfer.status.as_str()),
            ),
            status_kind: match transfer.status {
                TransferStatus::Requested => "warning",
                TransferStatus::Approved => "success",
                TransferStatus::Rejected => "info",
            },
            open: transfer.is_open(),
            sale: transfer.kind == TransferKind::Sale,
        });
    }
    let open_transfers: Vec<TransferRow> = transfers.iter().filter(|t| t.open).cloned().collect();
    let mut transfer_units: Vec<[String; 2]> = Vec::new();
    for row in found.units(&state.db).await? {
        if row.date_out.is_none() && !row.awaiting_transfer {
            let unit = Unit::find(&state.db, row.unit_id).await?;
            let label = unit.map_or_else(|| format!("#{}", row.unit_id), |u| u.number);
            transfer_units.push([row.unit_id.to_string(), label]);
        }
    }
    let reporting = AgreementStatus::REPORTING.contains(&found.status);
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        agreement: view,
        status: text(
            &lang,
            &format!("agreements.status.{}", found.status.as_str()),
        ),
        status_kind: status_kind(found.status),
        units,
        unit_options,
        pack_deposits: pack.deposits.iter().map(pack_line).collect(),
        pack_cheques: pack.cheques.iter().map(pack_line).collect(),
        pack_invoices: pack
            .invoices
            .iter()
            .map(|i| InvoiceLine {
                lease: i.lease.clone(),
                due: i.due.to_string(),
                balance: money_text(i.balance),
            })
            .collect(),
        pack_ejari: pack
            .ejari
            .iter()
            .map(|e| EjariLine {
                lease: e.lease.clone(),
                number: e.number.clone(),
            })
            .collect(),
        pack_lines,
        deposit_total: money_text(pack.deposit_total()),
        statements,
        today: today().to_string(),
        show_pack: matches!(
            found.status,
            AgreementStatus::Ending | AgreementStatus::Ended
        ),
        can_add_unit: matches!(
            found.status,
            AgreementStatus::Draft | AgreementStatus::Active
        ),
        can_activate: found.status == AgreementStatus::Draft,
        can_give_notice: found.status == AgreementStatus::Active,
        can_receipt: ending,
        can_final_statement: ending && may_prepare,
        can_end: ending,
        transfers,
        open_transfers,
        transfer_units,
        transfer_lines,
        can_transfer: reporting,
    }))
}

/// The form that puts a unit under the agreement.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct UnitForm {
    /// The unit.
    #[serde(default)]
    pub unit: String,
    /// The day it comes in.
    #[serde(default)]
    pub date_in: String,
}

/// `POST /property/agreements/{id}/units`.
async fn add_unit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<UnitForm>,
) -> Result<Response> {
    let reach = manage_reach(&state, &user).await?;
    ensure_in_reach(&state, &reach, id).await?;
    let found = ManagementAgreement::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    if !matches!(
        found.status,
        AgreementStatus::Draft | AgreementStatus::Active
    ) {
        return Err(invalid(&text(&lang, "agreements.units_locked")));
    }
    let unit: i64 = form
        .unit
        .trim()
        .parse()
        .map_err(|_| invalid("unit: 1, 2, 3 ..."))?;
    let date_in = parse_date(&form.date_in).ok_or_else(|| invalid("date_in: YYYY-MM-DD"))?;
    // A manager of some buildings puts only the units of those buildings
    // under an agreement.
    if let Some(units) = reach.ids(&state.db, crate::unit::UNIT_SUBJECT_TYPE).await?
        && !units.contains(&unit)
    {
        return Err(Error::Forbidden);
    }
    agreement::add_unit(&state.db, &found, unit, date_in).await?;
    Ok((
        Toast::success(text(&lang, "agreements.unit_added")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /property/agreements/{id}/activate`.
async fn activate(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    lifecycle::activate(&state, &user, id)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(text(&lang, "agreements.activated")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that records the notice.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct NoticeForm {
    /// The day notice was given.
    #[serde(default)]
    pub notice_on: String,
    /// `owner` or `company`.
    #[serde(default)]
    pub party: String,
    /// The day the units are handed over.
    #[serde(default)]
    pub handover: String,
    /// The party the handover goes to; empty for the owner.
    #[serde(default)]
    pub holder: String,
}

/// `POST /property/agreements/{id}/give-notice`.
async fn give_notice(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<NoticeForm>,
) -> Result<Response> {
    let reach = manage_reach(&state, &user).await?;
    ensure_in_reach(&state, &reach, id).await?;
    let on: NaiveDate =
        parse_date(&form.notice_on).ok_or_else(|| invalid("notice_on: YYYY-MM-DD"))?;
    let handover = parse_date(&form.handover).ok_or_else(|| invalid("handover: YYYY-MM-DD"))?;
    let party = NoticeParty::parse(&form.party).ok_or_else(|| invalid("party: owner, company"))?;
    let new_holder = if form.holder.trim().is_empty() {
        None
    } else {
        Some(super::party_of(&state, &form.holder, "holder").await?)
    };
    lifecycle::give_notice(
        &state,
        &user,
        id,
        Notice {
            on,
            party,
            handover,
            new_holder,
        },
    )
    .await
    .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(text(&lang, "agreements.notice_given")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that records a receipt.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ReceiptForm {
    /// The line of the pack.
    #[serde(default)]
    pub item: String,
    /// The receipt's reference.
    #[serde(default)]
    pub reference: String,
}

/// `POST /property/agreements/{id}/receipts`.
async fn receipt(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ReceiptForm>,
) -> Result<Response> {
    let item: i64 = form
        .item
        .trim()
        .parse()
        .map_err(|_| invalid("item: 1, 2, 3 ..."))?;
    lifecycle::record_receipt(&state, &user, id, item, &form.reference).await?;
    Ok((
        Toast::success(text(&lang, "handover.receipt_recorded")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /property/agreements/{id}/final-statement`.
async fn final_statement(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let made = lifecycle::prepare_final_statement(&state, &user, id).await?;
    let key = if made.is_some() {
        "handover.final_statement_made"
    } else {
        "handover.final_statement_exists"
    };
    Ok((Toast::success(text(&lang, key)), renox::HxRefresh).into_response())
}

/// `POST /property/agreements/{id}/end`.
async fn end(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let report = lifecycle::end(&state, &user, id)
        .await
        .map_err(TransitionError::into_error)?;
    let toast = if report.problems.is_empty() {
        Toast::success(text(&lang, "agreements.ended"))
    } else {
        Toast::warning(text(&lang, "agreements.ended_with_problems"))
    };
    Ok((toast, renox::HxRefresh).into_response())
}

/// The form that asks to move a unit out.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct TransferForm {
    /// The unit.
    #[serde(default)]
    pub unit: String,
    /// The transfer date.
    #[serde(default)]
    pub date: String,
    /// The new owner's party number; empty for "no longer managed".
    #[serde(default)]
    pub new_owner: String,
    /// A note.
    #[serde(default)]
    pub note: String,
}

/// `POST /property/agreements/{id}/transfers`.
async fn request_transfer(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<TransferForm>,
) -> Result<Response> {
    let unit_id: i64 = form
        .unit
        .trim()
        .parse()
        .map_err(|_| invalid("unit: 1, 2, 3 ..."))?;
    let date = parse_date(&form.date).ok_or_else(|| invalid("date: YYYY-MM-DD"))?;
    let new_owner = if form.new_owner.trim().is_empty() {
        None
    } else {
        Some(super::party_of(&state, &form.new_owner, "new_owner").await?)
    };
    let asked = TransferRequest {
        unit_id,
        date,
        new_owner,
        note: Some(form.note.clone()),
    };
    unit_transfer::request(&state, &user, id, asked).await?;
    Ok((
        Toast::success(text(&lang, "transfers.requested")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /property/agreements/{id}/transfer-receipts`.
async fn transfer_receipt(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ReceiptForm>,
) -> Result<Response> {
    let item_id: i64 = form
        .item
        .trim()
        .parse()
        .map_err(|_| invalid("item: 1, 2, 3 ..."))?;
    let item = HandoverItem::find(&state.db, item_id)
        .await?
        .filter(|item| item.agreement_id == id)
        .ok_or(Error::NotFound)?;
    let transfer = item.transfer_id.ok_or(Error::NotFound)?;
    unit_transfer::record_receipt(&state, &user, transfer, item_id, &form.reference).await?;
    Ok((
        Toast::success(text(&lang, "handover.receipt_recorded")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that approves a transfer.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ApproveForm {
    /// The new owner's receipt for the deposit that stays held (a sale).
    #[serde(default)]
    pub receipt: String,
}

/// `POST /property/transfers/{id}/approve`.
async fn approve_transfer(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ApproveForm>,
) -> Result<Response> {
    let report = unit_transfer::approve(&state, &user, id, Some(form.receipt.as_str())).await?;
    let toast = if report.problems.is_empty() {
        Toast::success(text(&lang, "transfers.approved"))
    } else {
        Toast::warning(text(&lang, "transfers.approved_with_problems"))
    };
    Ok((toast, renox::HxRefresh).into_response())
}

/// `POST /property/transfers/{id}/reject`.
async fn reject_transfer(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    unit_transfer::reject(&state, &user, id).await?;
    Ok((
        Toast::success(text(&lang, "transfers.rejected")),
        renox::HxRefresh,
    )
        .into_response())
}
