//! The owner statement pages: the list, a statement with its lines and the
//! actions on it (01 Property §4.4, §7.1 *Owner statement*; BL-PRP-19).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /property/statements` | `property.statements` | `owner_statements.prepare` or `.approve` |
//! | `GET /property/statements/{id}` | `property.statement` | `owner_statements.prepare` or `.approve` |
//! | `POST /property/statements/{id}/lines` | `property.statement_line_add` | `owner_statements.prepare` |
//! | `POST /property/statements/{id}/lines/remove` | `property.statement_line_remove` | `owner_statements.prepare` |
//! | `POST /property/statements/{id}/transitions/{name}` | `property.statement_transitions.*` | the transition's permission |
//!
//! The transitions are the machine's ([`crate::statement_machine`]): `approve`,
//! `return`, `publish`, `revise`; the page offers the ones the user may run now.

use renox::prelude::*;
use renox::state_machine::StateMachine;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::parties::directory;

use super::{Texts, text, user_name};
use crate::agreement::ManagementAgreement;
use crate::money_text;
use crate::parse::parse_aed;
use crate::permissions;
use crate::statement::{self, LineKind, OwnerStatement, StatementLine, StatementStatus};
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/property/statements", index)
        .name("property.statements")
        .get("/property/statements/{id}", show)
        .name("property.statement")
        .post("/property/statements/{id}/lines", add_line)
        .name("property.statement_line_add")
        .post("/property/statements/{id}/lines/remove", remove_line)
        .name("property.statement_line_remove")
        .group(
            "/property/statements",
            "property.statement_transitions.",
            wl_kernel::machines::routes::<OwnerStatement>(),
        )
        .require_auth()
}

/// Whether `user` may see the statements.
fn may_see(user: &AuthUser) -> bool {
    user.has_permission(permissions::OWNER_STATEMENTS_PREPARE)
        || user.has_permission(permissions::OWNER_STATEMENTS_APPROVE)
}

/// The badge kind of a statement status.
fn status_kind(status: StatementStatus) -> &'static str {
    match status {
        StatementStatus::Settled => "success",
        StatementStatus::Draft | StatementStatus::Approved | StatementStatus::Published => "info",
        StatementStatus::Superseded => "info",
    }
}

/// An amount with its sign, `+AED 2,000.00` or `-AED 6,000.00`.
fn signed(fils: i64) -> String {
    if fils > 0 {
        format!("+{}", money_text(fils))
    } else {
        money_text(fils)
    }
}

/// The name of the owner party `id`, else `#id`.
async fn owner_name(state: &AppState, lang: &Lang, id: i64) -> String {
    directory(state)
        .display_name(&state.db, PartyId(id), &lang.locale)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{id}"))
}

/// One statement of the list.
#[derive(Serialize, Debug, Clone)]
pub struct StatementRow {
    /// The primary key.
    pub id: i64,
    /// The number.
    pub number: String,
    /// The owner's name.
    pub owner: String,
    /// The period.
    pub period: String,
    /// The balance.
    pub balance: String,
    /// The status in the page's language.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "property/statements.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    statements: Vec<StatementRow>,
}

/// `GET /property/statements`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    if !may_see(&user) {
        return Err(Error::Forbidden);
    }
    let mut rows = Vec::new();
    for found in OwnerStatement::query()
        .order_by_desc(OwnerStatement::PERIOD_START)
        .order_by_desc(OwnerStatement::ID)
        .get(&state.db)
        .await?
    {
        rows.push(StatementRow {
            id: found.id,
            number: found.number(),
            owner: owner_name(&state, &lang, found.owner_party_id).await,
            period: format!("{} → {}", found.period_start, found.period_end),
            balance: money_text(found.balance()),
            status: text(&lang, &format!("statements.status.{}", found.status.as_str())),
            status_kind: status_kind(found.status),
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        statements: rows,
    }))
}

/// A line of the statement.
#[derive(Serialize, Debug, Clone)]
pub struct LineRow {
    /// The kind in the page's language.
    pub kind: String,
    /// The text.
    pub description: String,
    /// The signed amount.
    pub amount: String,
    /// What it comes from.
    pub source: String,
}

/// A transition offered on the statement.
#[derive(Serialize, Debug, Clone)]
pub struct ActionView {
    /// The transition's name.
    pub name: String,
    /// Its label.
    pub label: String,
}

/// The statement as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct StatementView {
    /// The primary key.
    pub id: i64,
    /// The number.
    pub number: String,
    /// The owner's name.
    pub owner: String,
    /// The agreement.
    pub agreement: String,
    /// The period.
    pub period: String,
    /// The balance brought forward.
    pub brought_forward: String,
    /// The rent collected.
    pub collected: String,
    /// The expenses.
    pub expenses: String,
    /// The management fee.
    pub fee: String,
    /// The adjustments.
    pub adjustments: String,
    /// The balance.
    pub balance: String,
    /// The preparer.
    pub prepared_by: String,
    /// The approver.
    pub approved_by: String,
    /// The day it was published.
    pub published_on: String,
    /// Why it was returned.
    pub return_reason: String,
    /// Why it was revised.
    pub revise_reason: String,
}

/// What a statement page needs.
#[derive(Serialize, ViewData)]
#[view(template = "property/statement.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    statement: StatementView,
    status: String,
    status_kind: &'static str,
    lines: Vec<LineRow>,
    line_options: Vec<[String; 2]>,
    plain_actions: Vec<ActionView>,
    reason_actions: Vec<ActionView>,
    can_edit: bool,
}

/// `GET /property/statements/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    if !may_see(&user) {
        return Err(Error::Forbidden);
    }
    let found = OwnerStatement::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let agreement = ManagementAgreement::find(&state.db, found.agreement_id)
        .await?
        .map(|a| a.number())
        .unwrap_or_default();
    let lines: Vec<StatementLine> = found.lines(&state.db).await?;
    let line_options = lines
        .iter()
        .map(|l| [l.id.to_string(), format!("{} {}", l.description, signed(l.amount))])
        .collect();
    let rows = lines
        .iter()
        .map(|l| LineRow {
            kind: text(&lang, &format!("statements.kind.{}", l.kind.as_str())),
            description: l.description.clone(),
            amount: signed(l.amount),
            source: l.source.clone().unwrap_or_default(),
        })
        .collect();
    let mut plain_actions = Vec::new();
    let mut reason_actions = Vec::new();
    for action in found.available(&user, &state.db).await? {
        let view = ActionView {
            name: action.name.clone(),
            label: action.label.clone(),
        };
        if action.needs_reason {
            reason_actions.push(view);
        } else {
            plain_actions.push(view);
        }
    }
    let prepared_by = match found.prepared_by {
        Some(id) => user_name(&state.db, id).await,
        None => text(&lang, "statements.system"),
    };
    let approved_by = match found.approved_by {
        Some(id) => user_name(&state.db, id).await,
        None => String::new(),
    };
    let view = StatementView {
        id: found.id,
        number: found.number(),
        owner: owner_name(&state, &lang, found.owner_party_id).await,
        agreement,
        period: format!("{} → {}", found.period_start, found.period_end),
        brought_forward: money_text(found.brought_forward),
        collected: money_text(found.collected),
        expenses: money_text(found.expenses),
        fee: money_text(found.fee),
        adjustments: signed(found.adjustments),
        balance: money_text(found.balance()),
        prepared_by,
        approved_by,
        published_on: found.published_on.map(|d| d.to_string()).unwrap_or_default(),
        return_reason: found.return_reason.clone().unwrap_or_default(),
        revise_reason: found.revise_reason.clone().unwrap_or_default(),
    };
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        statement: view,
        status: text(&lang, &format!("statements.status.{}", found.status.as_str())),
        status_kind: status_kind(found.status),
        lines: rows,
        line_options,
        plain_actions,
        reason_actions,
        can_edit: found.status == StatementStatus::Draft
            && user.has_permission(permissions::OWNER_STATEMENTS_PREPARE),
    }))
}

/// The form that adds a line.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct LineForm {
    /// `collected`, `expense`, `fee` or `adjustment`.
    #[serde(default)]
    pub kind: String,
    /// The text.
    #[serde(default)]
    pub description: String,
    /// The amount in dirhams.
    #[serde(default)]
    pub amount: String,
    /// `credit` or `debit`, for an adjustment.
    #[serde(default)]
    pub direction: String,
}

/// `POST /property/statements/{id}/lines`.
async fn add_line(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<LineForm>,
) -> Result<Response> {
    let kind: LineKind = form
        .kind
        .trim()
        .parse()
        .map_err(|_| Error::BadRequest("kind".to_owned()))?;
    let amount = parse_aed(&form.amount).ok_or_else(|| {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, "amount: AED 0.00".to_owned())
    })?;
    let amount = if kind == LineKind::Adjustment && form.direction.trim() == "debit" {
        -amount
    } else {
        amount
    };
    statement::add_line(&state, &user, id, kind, &form.description, amount).await?;
    Ok((Toast::success(text(&lang, "statements.line_added")), renox::HxRefresh).into_response())
}

/// The form that removes a line.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct RemoveLineForm {
    /// The line.
    #[serde(default)]
    pub line: String,
}

/// `POST /property/statements/{id}/lines/remove`.
async fn remove_line(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<RemoveLineForm>,
) -> Result<Response> {
    let line: i64 = form.line.trim().parse().map_err(|_| {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, "line: 1, 2, 3 ...".to_owned())
    })?;
    statement::remove_line(&state, &user, id, line).await?;
    Ok((Toast::success(text(&lang, "statements.line_removed")), renox::HxRefresh).into_response())
}
