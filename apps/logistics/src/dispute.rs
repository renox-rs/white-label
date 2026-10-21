//! A client contests a COD statement (03 Logistics §4.1 row *Client disputes
//! a COD statement*, §6 *COD remittance*, §7.1 rows `dispute` and
//! `resolve_dispute`, `LOG.ACL.12`; BL-LOG-18).
//!
//! - **The flag** ([`raise`]): `disputed` is a field of the remittance, not a
//!   state. A client user of the company disputes a `paid` remittance (its
//!   statement is published at `pay`) with a reason; customer service may also
//!   flag an `approved` one on the client's call. A ticket for the accountant
//!   is opened and linked ([`crate::remittance::Remittance::dispute_ticket_id`]).
//!   A second dispute of the same remittance adds the message to the open
//!   ticket. The dispute is not offered while the transfer is returned
//!   (`payment_returned`): the client's question is a normal ticket.
//! - **The outcome** ([`resolve`]): a user with `remittances.approve` closes
//!   it as *upheld* (the statement stands) or *adjusted* with an amount: an
//!   adjustment line on the next draft remittance, prepared by the resolver,
//!   which another user approves ([`crate::adjustment`], `LOG.ACL.12`). The
//!   flag is cleared, the ticket closed and the client told.
//! - **Nothing pauses.** The flag is read by nothing in `pay`, in the weekly
//!   rule `LOG.RUL.06` or in the next drafts, and the disputed remittance is
//!   not edited.

use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::{self, TaskComment, TicketId};

use crate::adjustment::{self, NewAdjustment, source};
use crate::booking;
use crate::notices::{STATEMENT_NOTICE, tell_party_as};
use crate::pages::{Fields, field};
use crate::permissions;
use crate::rates::aed;
use crate::remittance::{Remittance, RemittanceStatus};
use crate::texts::{english, t};

/// The statement stands.
pub const UPHELD: &str = "upheld";
/// The statement is corrected by an adjustment line on the next draft.
pub const ADJUSTED: &str = "adjusted";

/// One dispute of a statement and how it ended.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_statement_disputes", index(remittance_id))]
pub struct StatementDispute {
    /// The primary key.
    pub id: i64,
    /// The remittance.
    #[model(references = "logistics_remittances")]
    pub remittance_id: i64,
    /// The client's reason.
    pub reason: String,
    /// The user who raised it.
    pub raised_by: i64,
    /// When.
    pub raised_at: DateTime,
    /// `staff` or `portal`.
    pub via: String,
    /// The ticket for the accountant.
    pub ticket_id: Option<i64>,
    /// Its number.
    pub ticket_number: Option<String>,
    /// [`UPHELD`] or [`ADJUSTED`].
    pub outcome: Option<String>,
    /// The amount of the adjustment line, signed, in fils.
    pub outcome_amount: Option<i64>,
    /// The resolver's note.
    pub outcome_note: Option<String>,
    /// The adjustment line an *adjusted* outcome made.
    pub adjustment_line_id: Option<i64>,
    /// The user who resolved it.
    pub resolved_by: Option<i64>,
    /// When.
    pub resolved_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The open dispute of remittance `id`, if there is one.
///
/// # Errors
///
/// Database errors.
pub async fn open_of(db: &Db, id: i64) -> Result<Option<StatementDispute>> {
    StatementDispute::query()
        .where_(StatementDispute::REMITTANCE_ID.eq(id))
        .where_(StatementDispute::RESOLVED_AT.is_null())
        .order_by_desc(StatementDispute::ID)
        .first(db)
        .await
}

/// What raising a dispute did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Raised {
    /// The flag was set and the ticket opened.
    Flagged,
    /// The remittance was already disputed: the message went to the open
    /// ticket.
    Added,
}

fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

async fn load(state: &AppState, id: i64) -> std::result::Result<Remittance, TransitionError> {
    Remittance::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// Whether `user` may raise a dispute on `remittance`, and how: `staff`
/// (customer service, the accountant) or `portal` (a client user of the
/// company); `None` for anybody else.
async fn who(
    state: &AppState,
    user: &AuthUser,
    remittance: &Remittance,
) -> Result<Option<&'static str>> {
    if !user.has_permission(permissions::REMITTANCES_DISPUTE) {
        return Ok(None);
    }
    if user.has_permission(permissions::REMITTANCES_VIEW) {
        return Ok(Some("staff"));
    }
    if booking::acts_for(state, user, remittance.client()).await? {
        return Ok(Some("portal"));
    }
    Ok(None)
}

/// `user` disputes the statement of remittance `id` with `reason`: the
/// `disputed` flag is set and a ticket is opened for the accountant; when it
/// is disputed already, the reason is added to the open ticket.
///
/// # Errors
///
/// `Forbidden` for anybody but customer service, the accountant and a client
/// user of the company; `Guard` (422) for a draft, for a remittance whose
/// transfer was returned, for a client user on an `approved` remittance (its
/// statement is not published yet) and without a reason.
pub async fn raise(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: &str,
) -> std::result::Result<Raised, TransitionError> {
    let remittance = load(state, id).await?;
    let Some(via) = who(state, user, &remittance).await? else {
        return Err(TransitionError::Forbidden);
    };
    match remittance.status {
        RemittanceStatus::PaymentReturned => {
            return Err(guard(english("logistics.dispute.returned", &[])));
        }
        RemittanceStatus::Draft => return Err(guard(english("logistics.dispute.draft", &[]))),
        RemittanceStatus::Approved if via == "portal" => {
            return Err(guard(english("logistics.dispute.not_paid", &[])));
        }
        RemittanceStatus::Approved | RemittanceStatus::Paid => {}
    }
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(guard(english("logistics.dispute.reason_needed", &[])));
    }
    if remittance.disputed {
        if let Some(ticket) = remittance.dispute_ticket_id {
            tickets::task_creator(state)
                .comment(state, TicketId(ticket), TaskComment::visible(reason))
                .await?;
        }
        return Ok(Raised::Added);
    }
    let title = english(
        "logistics.dispute.ticket_title",
        &[("number", &remittance.number)],
    );
    let description = english(
        "logistics.dispute.ticket_description",
        &[("number", &remittance.number), ("reason", &reason)],
    );
    let task = RoleTask::new(
        remittance.subject(),
        permissions::INVOICE_OWNER_ROLE,
        &title,
        &format!("logistics.statement_dispute:{}", remittance.id),
    )
    .description(&description);
    let ticket = role_tasks::open(state, task).await?;
    let ticket_id = ticket.as_ref().map(|ticket| ticket.id.0);
    let ticket_number = ticket.map(|ticket| ticket.number);
    let mut dispute = StatementDispute {
        remittance_id: remittance.id,
        reason: reason.to_owned(),
        raised_by: user.id,
        raised_at: renox::db::now(),
        via: via.to_owned(),
        ticket_id,
        ticket_number,
        ..StatementDispute::default()
    };
    let mut tx = state.db.begin().await?;
    dispute.insert(&mut tx).await?;
    Remittance::query()
        .where_(Remittance::ID.eq(remittance.id))
        .update(
            &mut tx,
            &[("disputed", &true), ("dispute_ticket_id", &ticket_id)],
        )
        .await?;
    tx.commit().await?;
    Ok(Raised::Flagged)
}

/// `user` (`remittances.approve`) resolves the dispute of remittance `id` as
/// `outcome` ([`UPHELD`] or [`ADJUSTED`]) with `note`. *Adjusted* needs an
/// `amount` (signed, in fils): an adjustment line on the next draft, prepared
/// by `user`, which another user approves. The flag is cleared, the ticket is
/// closed and the client is told.
///
/// # Errors
///
/// `Forbidden` without `remittances.approve`; `Guard` (422) when the
/// remittance is not disputed, for an unknown outcome, a blank note, an
/// adjusted outcome without an amount, and the refusals of the adjustment
/// line.
pub async fn resolve(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    outcome: &str,
    amount: i64,
    note: &str,
) -> std::result::Result<Remittance, TransitionError> {
    if !user.has_permission(permissions::REMITTANCES_APPROVE) {
        return Err(TransitionError::Forbidden);
    }
    let remittance = load(state, id).await?;
    let Some(dispute) = open_of(&state.db, id).await?.filter(|_| remittance.disputed) else {
        return Err(guard(english("logistics.dispute.not_disputed", &[])));
    };
    let outcome = outcome.trim();
    if outcome != UPHELD && outcome != ADJUSTED {
        return Err(guard(english("logistics.dispute.outcome_unknown", &[])));
    }
    let note = note.trim();
    if note.is_empty() {
        return Err(guard(english("logistics.dispute.note_needed", &[])));
    }
    if outcome == ADJUSTED && amount == 0 {
        return Err(guard(english("logistics.dispute.amount_needed", &[])));
    }
    let mut line_id = None;
    if outcome == ADJUSTED {
        let line = adjustment::add(
            state,
            user,
            &NewAdjustment {
                client: remittance.client(),
                amount,
                reason: note.to_owned(),
                source: source::STATEMENT_DISPUTE.to_owned(),
                corrects_remittance_id: Some(remittance.id),
                ..NewAdjustment::default()
            },
        )
        .await?;
        line_id = Some(line.id);
    }
    let resolved_at = Some(renox::db::now());
    let resolved_by = Some(user.id);
    let outcome_text = Some(outcome.to_owned());
    let outcome_amount = (outcome == ADJUSTED).then_some(amount);
    let outcome_note = Some(note.to_owned());
    let no_ticket: Option<i64> = None;
    let mut tx = state.db.begin().await?;
    StatementDispute::query()
        .where_(StatementDispute::ID.eq(dispute.id))
        .update(
            &mut tx,
            &[
                ("outcome", &outcome_text),
                ("outcome_amount", &outcome_amount),
                ("outcome_note", &outcome_note),
                ("adjustment_line_id", &line_id),
                ("resolved_by", &resolved_by),
                ("resolved_at", &resolved_at),
            ],
        )
        .await?;
    Remittance::query()
        .where_(Remittance::ID.eq(remittance.id))
        .update(
            &mut tx,
            &[("disputed", &false), ("dispute_ticket_id", &no_ticket)],
        )
        .await?;
    tx.commit().await?;
    if let Some(ticket) = dispute.ticket_id {
        tickets::task_creator(state)
            .complete(state, TicketId(ticket), note)
            .await?;
    }
    let title = english(
        "logistics.dispute.resolved_title",
        &[("number", &remittance.number)],
    );
    let body = if outcome == ADJUSTED {
        english(
            "logistics.dispute.adjusted_body",
            &[
                ("number", &remittance.number),
                ("amount", &aed(amount)),
                ("note", &note),
            ],
        )
    } else {
        english(
            "logistics.dispute.upheld_body",
            &[("number", &remittance.number), ("note", &note)],
        )
    };
    tell_party_as(
        state,
        STATEMENT_NOTICE,
        remittance.subject(),
        &remittance.number,
        remittance.client(),
        &title,
        &body,
    )
    .await?;
    load(state, id).await
}

/// The routes: the client's dispute and the accountant's resolution.
pub fn routes() -> Routes {
    Routes::new()
        .post("/remittances/{id}/dispute", dispute)
        .name("logistics.remittance_dispute")
        .post("/remittances/{id}/resolve-dispute", resolve_dispute)
        .name("logistics.remittance_resolve_dispute")
        .require_auth()
}

/// Where the user goes back to: the staff page, or the statement for a client
/// user who cannot see it.
fn back(user: &AuthUser, lang: &Lang, key: &str, id: i64) -> Result<Response> {
    let route = if user.has_permission(permissions::REMITTANCES_VIEW) {
        "logistics.remittance"
    } else {
        "logistics.remittance_statement"
    };
    Ok((Toast::success(t(lang, key)), Redirect::route(route, &[&id])?).into_response())
}

/// `POST /remittances/{id}/dispute`: `reason`.
async fn dispute(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let raised = raise(&state, &user, id, &field(&form, "reason"))
        .await
        .map_err(TransitionError::into_error)?;
    let key = match raised {
        Raised::Flagged => "logistics.remittance_disputed",
        Raised::Added => "logistics.remittance_dispute_added",
    };
    back(&user, &lang, key, id)
}

/// `POST /remittances/{id}/resolve-dispute`: `outcome` (`upheld` or
/// `adjusted`), `amount` (dirhams, signed; for *adjusted*) and `note`.
async fn resolve_dispute(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let amount = match field(&form, "amount").trim() {
        "" => 0,
        text => adjustment::parse_signed_dirhams(text).ok_or_else(|| {
            Error::Status(StatusCode::UNPROCESSABLE_ENTITY, t(&lang, "logistics.bad_input"))
        })?,
    };
    resolve(
        &state,
        &user,
        id,
        &field(&form, "outcome"),
        amount,
        &field(&form, "note"),
    )
    .await
    .map_err(TransitionError::into_error)?;
    back(&user, &lang, "logistics.remittance_dispute_resolved", id)
}
