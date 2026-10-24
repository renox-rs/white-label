//! The till session pages (wl-sales §4.3, `SAL.POS.01`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /till-sessions` | `wl_sales.till_sessions` | any of `pos.open_till`, `pos.close_till`, `pos.reopen_till`, `pos.review` |
//! | `POST /till-sessions` | `wl_sales.till_session_open` | `pos.open_till`, fields `till`, `location`, `float` |
//! | `GET /till-sessions/{id}` | `wl_sales.till_session` | as the list |
//! | `POST /till-sessions/{id}/count` | `wl_sales.till_session_count` | `pos.close_till`, field `counted` |
//! | `POST /till-sessions/{id}/reopen` | `wl_sales.till_session_reopen` | `pos.reopen_till`, field `reason` |
//! | `POST /till-sessions/{id}/review` | `wl_sales.till_session_review` | `pos.review` |
//!
//! A refused action answers 422 with the reason; the buttons that cannot be
//! used are not shown, with the reason in the notes.

use std::collections::BTreeMap;

use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::serde_json::json;
use serde::Serialize;
use wl_kernel::sales::OpenTill;

use crate::notices::aed;
use crate::pages::campaigns::parse_fils;
use crate::till_session::{TillSession, TillSessionStatus};
use crate::till_sessions;
use crate::{permissions, texts};

/// The texts a template reads as `tx['till.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/till-sessions", index)
        .name("wl_sales.till_sessions")
        .post("/till-sessions", open)
        .name("wl_sales.till_session_open")
        .get("/till-sessions/{id}", show)
        .name("wl_sales.till_session")
        .post("/till-sessions/{id}/count", count)
        .name("wl_sales.till_session_count")
        .post("/till-sessions/{id}/reopen", reopen)
        .name("wl_sales.till_session_reopen")
        .post("/till-sessions/{id}/review", review)
        .name("wl_sales.till_session_review")
        .require_auth()
}

fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_sales.{key}"))
}

fn money(fils: i64) -> String {
    format!("AED {}", aed(fils))
}

/// Whether the user works with the tills in any way.
fn can_see(user: &AuthUser) -> bool {
    [
        permissions::POS_OPEN_TILL,
        permissions::POS_CLOSE_TILL,
        permissions::POS_REOPEN_TILL,
        permissions::POS_REVIEW,
    ]
    .iter()
    .any(|p| user.has_permission(p))
}

fn status_kind(status: TillSessionStatus) -> &'static str {
    match status {
        TillSessionStatus::Open => "info",
        TillSessionStatus::Counting => "warning",
        TillSessionStatus::Closed | TillSessionStatus::Reviewed => "success",
    }
}

async fn cashier_name(db: &Db, id: i64) -> String {
    match User::find(db, id).await {
        Ok(Some(user)) => user.name,
        _ => format!("#{id}"),
    }
}

/// One session of the list.
#[derive(Serialize, Debug, Clone)]
pub struct SessionRow {
    /// The primary key.
    pub id: i64,
    /// The till.
    pub till: String,
    /// The outlet.
    pub location: String,
    /// The day.
    pub date: String,
    /// The cashier's name.
    pub cashier: String,
    /// What the till should hold.
    pub expected: String,
    /// What was counted.
    pub counted: String,
    /// The difference.
    pub variance: String,
    /// The status in words.
    pub status_label: String,
    /// The badge colour.
    pub status_kind: &'static str,
}

async fn row_of(state: &AppState, lang: &Lang, session: &TillSession) -> Result<SessionRow> {
    let view = till_sessions::view_of(&state.db, session).await?;
    Ok(SessionRow {
        id: session.id,
        till: session.till.clone(),
        location: session.location.clone().unwrap_or_default(),
        date: view.business_date.to_string(),
        cashier: cashier_name(&state.db, session.cashier_user_id).await,
        expected: money(view.expected),
        counted: view.counted.map(money).unwrap_or_default(),
        variance: view.variance.map(money).unwrap_or_default(),
        status_label: text(lang, &format!("till.status.{}", session.status.as_str())),
        status_kind: status_kind(session.status),
    })
}

/// The till sessions grid: search on the till, outlet and cashier, filters by
/// status and day, sum of the counted cash and the differences. What the till
/// should hold is worked out from the sales of an open session, so the
/// template draws it (`custom`).
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let t = |key: &str| text(lang, key);
    let statuses: Vec<(String, String)> = TillSessionStatus::ALL
        .iter()
        .map(|status| {
            (
                status.as_str().to_owned(),
                text(lang, &format!("till.status.{}", status.as_str())),
            )
        })
        .collect();
    let grid = Grid::new("sales-till-sessions")
        .title(&t("till.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("till", &t("till.till"))
                .frozen()
                .mobile()
                .searchable()
                .link("/till-sessions/{id}"),
        )
        .column(Column::text("location", &t("till.location")).searchable())
        .column(Column::date("business_date", &t("till.date")).mobile())
        .column(
            Column::related(
                "cashier",
                &t("till.cashier"),
                "users",
                "cashier_user_id",
                "name",
            )
            .searchable(),
        )
        .column(
            Column::money(
                "float_amount",
                &wl_kernel::money_label(&t("till.float_label")),
            )
            .hidden(),
        )
        .column(Column::custom("expected", &t("till.expected")).mobile())
        .column(
            Column::money("counted_cash", &wl_kernel::money_label(&t("till.counted")))
                .hidden()
                .summary(Summary::Sum),
        )
        .column(
            Column::money("variance", &wl_kernel::money_label(&t("till.variance")))
                .summary(Summary::Sum),
        )
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&[
                    ("open", "info"),
                    ("counting", "warning"),
                    ("closed", "success"),
                    ("reviewed", "success"),
                ])
                .mobile(),
        )
        .sort_by("-business_date")
        .row_url("/till-sessions/{id}")
        .cards_on_mobile()
        .empty_state(&t("till.empty"), Some(&t("till.empty_hint")));
    if can_export { grid.exports() } else { grid }
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/till_sessions.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    sessions: GridPage<TillSession>,
    can_open: bool,
}

/// `GET /till-sessions`: the sessions, newest day first (`?export=` with
/// `pos.review`).
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    if !can_see(&user) {
        return Err(Error::Forbidden);
    }
    let can_export = user.has_permission(permissions::POS_REVIEW);
    let grid = grid(&lang, can_export);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    let query = TillSession::query();
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut expected = Vec::new();
    for session in page.items() {
        let view = till_sessions::view_of(&state.db, session).await?;
        expected.push(money(view.expected));
    }
    let mut expected = expected.into_iter();
    let sessions = page.extend(|_| json!({ "expected": expected.next().unwrap_or_default() }));
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        sessions,
        can_open: user.has_permission(permissions::POS_OPEN_TILL),
    })
    .into_response())
}

/// `POST /till-sessions`: opens a session for the signed-in cashier, today.
async fn open(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let float = if get("float").is_empty() {
        0
    } else {
        parse_fils(get("float")).ok_or_else(|| {
            Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                text(&lang, "till.problem.amount"),
            )
        })?
    };
    let location = (!get("location").is_empty()).then(|| get("location").to_owned());
    let session = till_sessions::open(
        &state,
        Some(&user),
        OpenTill {
            till: get("till").to_owned(),
            location,
            legal_entity_id: None,
            business_date: wl_kernel::documents::today(),
            cashier_user: user.id,
            float,
        },
    )
    .await?;
    Ok((
        Toast::success(text(&lang, "till.opened")),
        Redirect::route("wl_sales.till_session", &[&session.id])?,
    )
        .into_response())
}

/// What the session page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/till_session.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    session: SessionRow,
    float: String,
    cash_sales: String,
    card_sales: String,
    reopened: i64,
    last_reason: String,
    can_count: bool,
    can_reopen: bool,
    can_review: bool,
}

/// `GET /till-sessions/{id}`: the session.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(session): Found<TillSession>,
) -> Result<View> {
    if !can_see(&user) {
        return Err(Error::Forbidden);
    }
    let view = till_sessions::view_of(&state.db, &session).await?;
    let open = session.status == TillSessionStatus::Open;
    let closed = session.status == TillSessionStatus::Closed;
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        session: row_of(&state, &lang, &session).await?,
        float: money(view.float),
        cash_sales: money(view.cash_sales),
        card_sales: money(view.card_sales),
        reopened: session.reopen_count,
        last_reason: session.reopen_reason.clone().unwrap_or_default(),
        can_count: open && user.has_permission(permissions::POS_CLOSE_TILL),
        can_reopen: closed && user.has_permission(permissions::POS_REOPEN_TILL),
        can_review: closed && user.has_permission(permissions::POS_REVIEW),
    }))
}

/// `POST /till-sessions/{id}/count`: counts the cash and closes the session.
async fn count(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let counted = form
        .get("counted")
        .and_then(|v| parse_fils(v))
        .ok_or_else(|| {
            Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                text(&lang, "till.problem.amount"),
            )
        })?;
    till_sessions::submit_count(&state, Some(&user), id, counted).await?;
    Ok((
        Toast::success(text(&lang, "till.counted_toast")),
        Redirect::route("wl_sales.till_session", &[&id])?,
    )
        .into_response())
}

/// `POST /till-sessions/{id}/reopen`: reopens a closed session.
async fn reopen(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let reason = form.get("reason").map_or("", |v| v.trim());
    till_sessions::reopen(&state, Some(&user), id, reason).await?;
    Ok((
        Toast::success(text(&lang, "till.reopened")),
        Redirect::route("wl_sales.till_session", &[&id])?,
    )
        .into_response())
}

/// `POST /till-sessions/{id}/review`: marks a closed session reviewed.
async fn review(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    till_sessions::review(&state, Some(&user), id).await?;
    Ok((
        Toast::success(text(&lang, "till.reviewed")),
        Redirect::route("wl_sales.till_session", &[&id])?,
    )
        .into_response())
}
