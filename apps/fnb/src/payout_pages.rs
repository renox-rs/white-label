//! The payout statement screens (06 F&B §4.3, §7.1 *Payout statement*;
//! BL-FNB-09 and BL-FNB-10): the list with the import form, a statement's page
//! with its lines, the differences, and the buttons for match, dispute,
//! resolve and void, and the cashier's late entry of a missing order.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /payouts` | `fnb.payouts` | `reconciliation.view` |
//! | `POST /payouts` | `fnb.payout_import` | `reconciliation.manage`, fields `aggregator`, `reference`, `file_name`, `period_from`, `period_to`, `content` |
//! | `GET /payouts/{id}` | `fnb.payout` | `reconciliation.view` |
//! | `POST /payouts/{id}/match` | `fnb.payout_match` | `reconciliation.manage` |
//! | `POST /payouts/{id}/dispute` | `fnb.payout_dispute` | `reconciliation.manage`, fields `reason`, `line_<id>` |
//! | `POST /payouts/{id}/resolve` | `fnb.payout_resolve` | `reconciliation.manage`, fields `answer`, `adjustment`, `reverse_<id>` |
//! | `POST /payouts/{id}/lines/{line}/party` | `fnb.payout_line_party` | `reconciliation.manage`, fields `party`, `category` |
//! | `POST /payouts/{id}/lines/{line}/explain` | `fnb.payout_line_explain` | `reconciliation.manage`, field `note` |
//! | `POST /payouts/late-order` | `fnb.late_order` | `orders.late_entry`, fields `aggregator`, `order_number`, `occurred_on`, `occurred_time`, `gross`, `reason`, `back` |
//!
//! Void is the kernel's (`POST /voids/fnb.payout_statement/{id}`), offered
//! from the statement's page.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::StateMachine;
use serde::Serialize;

use crate::money::parse_dirhams;
use crate::pages::{Fields, Texts, field, tx};
use crate::payout::{ImportPreset, PayoutLine, PayoutStatement, PayoutStatus, line_state};
use crate::payouts::{
    self, CATEGORIES, ImportAsk, LateOrderAsk, PARTIES, differences, unmatched_amount,
};
use crate::permissions;
use crate::texts::t;

/// The templates, compiled in.
pub(crate) const VIEWS: &[(&str, &str)] = &[
    (
        "fnb/payouts.html",
        include_str!("../views/fnb/payouts.html"),
    ),
    ("fnb/payout.html", include_str!("../views/fnb/payout.html")),
];

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/payouts", index)
        .name("fnb.payouts")
        .post("/payouts", import)
        .name("fnb.payout_import")
        .post("/payouts/late-order", late_order)
        .name("fnb.late_order")
        .get("/payouts/{id}", show)
        .name("fnb.payout")
        .post("/payouts/{id}/match", match_one)
        .name("fnb.payout_match")
        .post("/payouts/{id}/dispute", dispute)
        .name("fnb.payout_dispute")
        .post("/payouts/{id}/resolve", resolve)
        .name("fnb.payout_resolve")
        .post("/payouts/{id}/lines/{line}/party", line_party)
        .name("fnb.payout_line_party")
        .post("/payouts/{id}/lines/{line}/explain", line_explain)
        .name("fnb.payout_line_explain")
        .require_auth()
}

/// The badge colour of a status.
fn status_kind(status: PayoutStatus) -> &'static str {
    match status {
        PayoutStatus::Imported | PayoutStatus::Voided => "info",
        PayoutStatus::Matched | PayoutStatus::Settled => "success",
        PayoutStatus::Disputed => "warning",
    }
}

/// The word of a status.
fn status_word(lang: &Lang, status: PayoutStatus) -> String {
    t(
        lang,
        &format!("fnb.screen.payout.status.{}", status.as_str()),
    )
}

/// Whether a form field is ticked.
fn ticked(form: &std::collections::BTreeMap<String, String>, name: &str) -> bool {
    form.get(name)
        .is_some_and(|v| !v.trim().is_empty() && v != "0" && v != "false")
}

/// The date typed in `name`, if it is one.
fn date_of(form: &std::collections::BTreeMap<String, String>, name: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(field(form, name).trim(), "%Y-%m-%d").ok()
}

/// One statement of the list.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct StatementRow {
    id: i64,
    number: String,
    aggregator: String,
    reference: String,
    period: String,
    lines: i64,
    net: i64,
    status: String,
    status_kind: &'static str,
    voided: bool,
    url: String,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/payouts.html")]
pub(crate) struct PayoutsPage {
    tx: Texts,
    statements: Vec<StatementRow>,
    aggregators: Vec<[String; 2]>,
    can_import: bool,
}

/// The period as text: `2026-10-05 to 2026-10-11`.
fn period_text(statement: &PayoutStatement) -> String {
    match (statement.period_from, statement.period_to) {
        (Some(from), Some(to)) => format!("{from} - {to}"),
        (Some(from), None) => from.to_string(),
        (None, Some(to)) => to.to_string(),
        (None, None) => String::new(),
    }
}

/// The aggregators that have a column preset, as select options.
async fn aggregator_options(state: &AppState) -> Result<Vec<[String; 2]>> {
    Ok(ImportPreset::query()
        .order_by(ImportPreset::AGGREGATOR)
        .get(&state.db)
        .await?
        .into_iter()
        .map(|p| [p.aggregator, p.name])
        .collect())
}

async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<Response> {
    if !user.has_permission(permissions::RECONCILIATION_VIEW) {
        return Err(Error::Forbidden);
    }
    let statements = PayoutStatement::newest(&state.db, 200)
        .await?
        .iter()
        .map(|s| StatementRow {
            id: s.id,
            number: s.number.clone(),
            aggregator: s.aggregator.clone(),
            reference: s.reference.clone(),
            period: period_text(s),
            lines: s.line_count,
            net: s.net_total,
            status: status_word(&lang, s.status),
            status_kind: status_kind(s.status),
            voided: s.status == PayoutStatus::Voided,
            url: format!("/payouts/{}", s.id),
        })
        .collect();
    Ok(View::typed(PayoutsPage {
        tx: tx(&lang),
        statements,
        aggregators: aggregator_options(&state).await?,
        can_import: user.has_permission(permissions::RECONCILIATION_MANAGE),
    })
    .into_response())
}

/// `POST /payouts`: imports the statement; a duplicate is refused (422) and the
/// text names the statement that exists.
async fn import(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Fields,
) -> Result<Response> {
    let ask = ImportAsk {
        aggregator: field(&form, "aggregator"),
        reference: field(&form, "reference"),
        file_name: field(&form, "file_name"),
        period_from: date_of(&form, "period_from"),
        period_to: date_of(&form, "period_to"),
        content: field(&form, "content"),
    };
    let statement = payouts::import(&state, &user, &ask).await?;
    Ok((
        Toast::success(t(&lang, "fnb.payout.imported")),
        Redirect::route("fnb.payout", &[&statement.id])?,
    )
        .into_response())
}

/// A line of a statement's page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct LineView {
    id: i64,
    line_no: i64,
    order_number: String,
    order_url: String,
    gross: i64,
    commission: i64,
    promo: i64,
    refund: i64,
    net: i64,
    reason: String,
    category: String,
    party: String,
    party_label: String,
    state_label: String,
    state_kind: &'static str,
    explanation: String,
    return_id: Option<i64>,
    return_url: String,
    returned: bool,
    taken_back: bool,
    disputed: bool,
    is_refund: bool,
    needs_party: bool,
    can_explain: bool,
}

/// A difference of a statement's page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct DifferenceView {
    kind: String,
    order_number: String,
    order_url: String,
    amount: i64,
}

/// What a statement's page needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/payout.html")]
pub(crate) struct ShowPayoutPage {
    tx: Texts,
    statement: PayoutStatement,
    status_label: String,
    status_kind: &'static str,
    period: String,
    lines: Vec<LineView>,
    differences: Vec<DifferenceView>,
    notes: Vec<String>,
    can_match: bool,
    can_dispute: bool,
    can_resolve: bool,
    can_late: bool,
    can_edit_lines: bool,
    void_action: Option<Value>,
    parties: Vec<[String; 2]>,
    categories: Vec<[String; 2]>,
    unmatched: i64,
    unmatched_amount: i64,
    disputable: Vec<LineView>,
    reversible: Vec<LineView>,
    aggregator: String,
    today: String,
}

/// The name of a complaint category.
fn category_word(lang: &Lang, category: &str) -> String {
    t(lang, &format!("fnb.screen.payout.category.{category}"))
}

/// The name of a responsible party.
fn party_word(lang: &Lang, party: &str) -> String {
    t(lang, &format!("fnb.screen.payout.party.{party}"))
}

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    if !user.has_permission(permissions::RECONCILIATION_VIEW) {
        return Err(Error::Forbidden);
    }
    let statement = PayoutStatement::find_or_404(&state.db, id).await?;
    let manager = user.has_permission(permissions::RECONCILIATION_MANAGE);
    let open = matches!(
        statement.status,
        PayoutStatus::Imported | PayoutStatus::Disputed
    );
    let raw = PayoutLine::of_statement(&state.db, id).await?;
    let mut lines = Vec::new();
    for line in &raw {
        let order_url = match line.order_id {
            Some(order) => format!("/orders/{order}"),
            None => String::new(),
        };
        let (state_label, state_kind) = if line.disputed {
            (t(&lang, "fnb.screen.payout.line_disputed"), "warning")
        } else {
            match line.match_state.as_str() {
                line_state::MATCHED if line.late_order_id.is_some() => {
                    (t(&lang, "fnb.screen.payout.line_late"), "info")
                }
                line_state::MATCHED => (t(&lang, "fnb.screen.payout.line_matched"), "success"),
                line_state::EXPLAINED => (t(&lang, "fnb.screen.payout.line_explained"), "info"),
                _ => (t(&lang, "fnb.screen.payout.line_unmatched"), "error"),
            }
        };
        lines.push(LineView {
            id: line.id,
            line_no: line.line_no,
            order_number: line.order_number.clone(),
            order_url,
            gross: line.gross,
            commission: line.commission,
            promo: line.promo,
            refund: line.refund,
            net: line.net,
            reason: line.reason.clone(),
            category: line
                .category
                .as_deref()
                .map(|c| category_word(&lang, c))
                .unwrap_or_default(),
            party: line.party.clone().unwrap_or_default(),
            party_label: line
                .party
                .as_deref()
                .map(|p| party_word(&lang, p))
                .unwrap_or_default(),
            state_label,
            state_kind,
            explanation: line.explanation.clone().unwrap_or_default(),
            return_id: line.return_id,
            return_url: line
                .return_id
                .map(|r| format!("/sales-returns/{r}"))
                .unwrap_or_default(),
            returned: line.return_id.is_some() && line.return_reversed_at.is_none(),
            taken_back: line.return_reversed_at.is_some(),
            disputed: line.disputed,
            is_refund: line.is_refund(),
            needs_party: line.is_refund() && open,
            can_explain: line.is_unmatched() && open,
        });
    }
    let unmatched = i64::try_from(raw.iter().filter(|l| l.is_unmatched()).count()).unwrap_or(0);
    let without_party = i64::try_from(
        raw.iter()
            .filter(|l| l.is_refund() && l.party.is_none())
            .count(),
    )
    .unwrap_or(0);
    let mut notes = Vec::new();
    if statement.status == PayoutStatus::Imported {
        if unmatched > 0 {
            notes.push(t(&lang, "fnb.screen.payout.note_unmatched"));
        }
        if without_party > 0 {
            notes.push(t(&lang, "fnb.screen.payout.note_party"));
        }
    }
    if statement.status == PayoutStatus::Settled {
        notes.push(t(&lang, "fnb.screen.payout.note_settled"));
    }
    let mut diffs = Vec::new();
    for d in differences(&state.db, &statement).await? {
        diffs.push(DifferenceView {
            kind: t(&lang, &format!("fnb.screen.payout.difference.{}", d.kind)),
            order_number: d.order_number,
            order_url: format!("/orders/{}", d.order_id),
            amount: d.amount,
        });
    }
    let actions = statement.available(&user, &state.db).await?;
    let void_action = wl_kernel::voids::web::action_for(&statement.subject(), &actions, &lang);
    let can_match = manager && statement.status == PayoutStatus::Imported;
    let disputable: Vec<LineView> = lines.iter().filter(|l| !l.disputed).cloned().collect();
    let reversible: Vec<LineView> = lines
        .iter()
        .filter(|l| l.disputed && l.returned)
        .cloned()
        .collect();
    let parties = PARTIES
        .iter()
        .map(|p| [(*p).to_owned(), party_word(&lang, p)])
        .collect();
    let categories = CATEGORIES
        .iter()
        .map(|c| [(*c).to_owned(), category_word(&lang, c)])
        .collect();
    Ok(View::typed(ShowPayoutPage {
        tx: tx(&lang),
        status_label: status_word(&lang, statement.status),
        status_kind: status_kind(statement.status),
        period: period_text(&statement),
        lines,
        differences: diffs,
        notes,
        can_match: can_match && unmatched == 0 && without_party == 0,
        can_dispute: manager
            && matches!(
                statement.status,
                PayoutStatus::Imported | PayoutStatus::Matched
            ),
        can_resolve: manager && statement.status == PayoutStatus::Disputed,
        can_late: user.has_permission(permissions::LATE_ORDERS_ENTER)
            && matches!(
                statement.status,
                PayoutStatus::Imported | PayoutStatus::Disputed
            ),
        can_edit_lines: manager && open,
        void_action,
        parties,
        categories,
        unmatched,
        unmatched_amount: unmatched_amount(&state.db, id).await?,
        disputable,
        reversible,
        aggregator: statement.aggregator.clone(),
        today: wl_kernel::documents::today().to_string(),
        statement,
    })
    .into_response())
}

/// A redirect to the statement's page with a toast.
fn back_to(lang: &Lang, key: &str, id: i64) -> Result<Response> {
    Ok((
        Toast::success(t(lang, key)),
        Redirect::route("fnb.payout", &[&id])?,
    )
        .into_response())
}

async fn match_one(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let _ = payouts::match_statement(&state, Some(&user), id).await?;
    back_to(&lang, "fnb.payout.matched", id)
}

async fn dispute(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let lines: Vec<i64> = PayoutLine::of_statement(&state.db, id)
        .await?
        .iter()
        .filter(|l| ticked(&form, &format!("line_{}", l.id)))
        .map(|l| l.id)
        .collect();
    let _ = payouts::dispute(&state, &user, id, &lines, &field(&form, "reason")).await?;
    back_to(&lang, "fnb.payout.disputed", id)
}

async fn resolve(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let reversed: Vec<i64> = PayoutLine::of_statement(&state.db, id)
        .await?
        .iter()
        .filter(|l| ticked(&form, &format!("reverse_{}", l.id)))
        .map(|l| l.id)
        .collect();
    let adjustment = match field(&form, "adjustment").trim() {
        "" => 0,
        text => {
            let (negative, digits) = match text.strip_prefix('-') {
                Some(rest) => (true, rest),
                None => (false, text),
            };
            let fils = parse_dirhams(digits)
                .ok_or_else(|| crate::placing::refused(t(&lang, "fnb.payout.problem.amount")))?;
            if negative { -fils } else { fils }
        }
    };
    let _ = payouts::resolve(
        &state,
        &user,
        id,
        &field(&form, "answer"),
        &reversed,
        adjustment,
    )
    .await?;
    back_to(&lang, "fnb.payout.resolved", id)
}

async fn line_party(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((id, line)): Path<(i64, i64)>,
    Form(form): Fields,
) -> Result<Response> {
    let category = field(&form, "category");
    let _ = payouts::set_party(
        &state,
        &user,
        id,
        line,
        &field(&form, "party"),
        Some(category.as_str()),
    )
    .await?;
    back_to(&lang, "fnb.payout.party_set", id)
}

async fn line_explain(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((id, line)): Path<(i64, i64)>,
    Form(form): Fields,
) -> Result<Response> {
    let _ = payouts::explain(&state, &user, id, line, &field(&form, "note")).await?;
    back_to(&lang, "fnb.payout.explained", id)
}

/// `POST /payouts/late-order`: the cashier enters an order the statement names
/// and the kitchen never recorded; `back` is the statement's id to return to.
async fn late_order(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Fields,
) -> Result<Response> {
    let gross = parse_dirhams(&field(&form, "gross"))
        .ok_or_else(|| crate::placing::refused(t(&lang, "fnb.payout.problem.amount")))?;
    let ask = LateOrderAsk {
        aggregator: field(&form, "aggregator"),
        order_number: field(&form, "order_number"),
        occurred_on: date_of(&form, "occurred_on"),
        occurred_time: field(&form, "occurred_time"),
        gross,
        reason: field(&form, "reason"),
    };
    let entered = payouts::enter_late_order(&state, &user, &ask).await?;
    let key = if entered.is_pending() {
        "fnb.payout.late_pending"
    } else {
        "fnb.payout.late_entered"
    };
    let toast = Toast::success(t(&lang, key));
    let back = field(&form, "back").trim().parse::<i64>().ok();
    let target = match back {
        Some(id) => Redirect::route("fnb.payout", &[&id])?,
        None => Redirect::route("fnb.payouts", &[])?,
    };
    Ok((toast, target).into_response())
}
