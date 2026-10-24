//! The daily closing screens (06 F&B §4.6 step 4, §7.1 *Daily closing*;
//! BL-FNB-15): the closings of the outlets by business day, and one closing with
//! its sales by channel, the till count, the paper orders of the day and the
//! sheets to submit, approve and reopen it.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /closings` | `fnb.closings` | `closing.view` |
//! | `POST /closings` | `fnb.closing_open` | `closing.submit`, field `outlet` |
//! | `GET /closings/{id}` | `fnb.closing` | `closing.view` |
//! | `POST /closings/{id}/submit` | `fnb.closing_submit` | `closing.submit`, fields `counted`, `checklist`, `cashier` |
//! | `POST /closings/{id}/approve` | `fnb.closing_approve` | `closing.approve` |
//! | `POST /closings/{id}/reopen` | `fnb.closing_reopen` | `closing.reopen`, field `reason` |
//! | `POST /outlets/{id}/cutoff` | `fnb.outlet_cutoff` | `closing.cutoff`, field `time` |
//!
//! A refused action answers 422 with the reason (submit before the cut-off says
//! the time the closing opens); a button that cannot be used is not shown and
//! the reason is in the notes.

use renox::grid::{Column, Grid, GridRequest, Summary};
use renox::prelude::*;
use renox::serde_json::json;
use serde::Serialize;

use crate::business_day::clock_text;
use crate::catalogue::Outlet;
use crate::closing::{ClosingStatus, DailyClosing};
use crate::closings::{self, SubmitAsk, Totals};
use crate::money::{aed, parse_dirhams};
use crate::pages::{Fields, Texts, field, tx};
use crate::paper_order::{PaperOrder, method};
use crate::permissions;
use crate::placing::refused;
use crate::texts::t;

/// The templates, compiled in.
pub(crate) const VIEWS: &[(&str, &str)] = &[
    (
        "fnb/closings.html",
        include_str!("../views/fnb/closings.html"),
    ),
    (
        "fnb/closing.html",
        include_str!("../views/fnb/closing.html"),
    ),
];

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/closings", index)
        .name("fnb.closings")
        .post("/closings", open)
        .name("fnb.closing_open")
        .get("/closings/{id}", show)
        .name("fnb.closing")
        .post("/closings/{id}/submit", submit)
        .name("fnb.closing_submit")
        .post("/closings/{id}/approve", approve)
        .name("fnb.closing_approve")
        .post("/closings/{id}/reopen", reopen)
        .name("fnb.closing_reopen")
        .post("/outlets/{id}/cutoff", cutoff)
        .name("fnb.outlet_cutoff")
        .require_auth()
}

fn status_word(lang: &Lang, status: ClosingStatus) -> String {
    t(
        lang,
        &format!("fnb.screen.closing.status.{}", status.as_str()),
    )
}

fn status_kind(status: ClosingStatus) -> &'static str {
    match status {
        ClosingStatus::Open => "info",
        ClosingStatus::Submitted => "success",
        ClosingStatus::Reopened => "warning",
        ClosingStatus::Approved => "success",
    }
}

fn ticked(form: &std::collections::BTreeMap<String, String>, name: &str) -> bool {
    form.get(name)
        .is_some_and(|v| !v.trim().is_empty() && v != "0" && v != "false")
}

/// The closings grid: search on the outlet, filters by status and day, sums
/// of the cash counted and the difference. The orders and sales of the day
/// are worked out live, so the template draws them (`custom`).
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let t = tx(lang);
    let statuses: Vec<(String, String)> = ["open", "submitted", "reopened", "approved"]
        .iter()
        .map(|code| {
            (
                (*code).to_owned(),
                crate::texts::t(lang, &format!("fnb.screen.closing.status.{code}")),
            )
        })
        .collect();
    let grid = Grid::new("fnb-closings")
        .title(&t["closing.title"])
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::date("business_date", &t["closing.date"])
                .frozen()
                .mobile()
                .link("/closings/{id}"),
        )
        .column(
            Column::related(
                "outlet",
                &t["closing.outlet"],
                "fnb_outlets",
                "outlet_id",
                "name",
            )
            .mobile()
            .searchable(),
        )
        .column(Column::custom("orders", &t["closing.orders"]))
        .column(Column::custom("sales", &t["closing.sales"]).mobile())
        .column(
            Column::money("counted_cash", &t["closing.counted"])
                .hidden()
                .summary(Summary::Sum),
        )
        .column(
            Column::money("variance", &t["closing.variance"])
                .mobile()
                .summary(Summary::Sum),
        )
        .column(
            Column::select("status", &t["state"], statuses)
                .badges(&[
                    ("open", "info"),
                    ("submitted", "success"),
                    ("reopened", "warning"),
                    ("approved", "success"),
                ])
                .mobile(),
        )
        .sort_by("-business_date")
        .row_url("/closings/{id}")
        .cards_on_mobile()
        .empty_state(&t["closing.empty"], Some(&t["closing.empty_text"]));
    if can_export { grid.exports() } else { grid }
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/closings.html")]
pub(crate) struct ClosingsPage {
    tx: Texts,
    rows: renox::grid::GridPage<DailyClosing>,
    outlets: Vec<[String; 2]>,
    can_open: bool,
}

async fn outlet_options(db: &Db) -> Result<Vec<[String; 2]>> {
    Ok(Outlet::query()
        .where_(Outlet::ACTIVE.eq(true))
        .order_by(Outlet::ID)
        .get(db)
        .await?
        .into_iter()
        .map(|o| [o.id.to_string(), o.name])
        .collect())
}

async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    if !user.has_permission(permissions::CLOSING_VIEW) {
        return Err(Error::Forbidden);
    }
    // Whoever sees the closings sees their counts: the approvers export.
    let can_export = user.has_permission(permissions::CLOSING_APPROVE);
    let grid = grid(&lang, can_export);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    if let Some(file) = grid.export(DailyClosing::query(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(DailyClosing::query(), &request).await?;
    let mut sums = std::collections::BTreeMap::new();
    for closing in page.items() {
        sums.insert(closing.id, closings::totals(&state.db, closing).await?);
    }
    let rows = page.extend(|closing| {
        let sums = sums.get(&closing.id);
        json!({
            "orders": sums.map(|s| s.orders),
            "sales": sums.map(|s| s.gross),
        })
    });
    Ok(View::typed(ClosingsPage {
        tx: tx(&lang),
        rows,
        outlets: outlet_options(&state.db).await?,
        can_open: user.has_permission(permissions::CLOSING_SUBMIT),
    })
    .into_response())
}

/// `POST /closings`: opens the closing of the outlet's business day now.
async fn open(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Fields,
) -> Result<Response> {
    let outlet: i64 = field(&form, "outlet").trim().parse().unwrap_or(0);
    let closing = closings::open_today(&state, &user, outlet).await?;
    Ok((
        Toast::success(t(&lang, "fnb.closing.opened")),
        Redirect::route("fnb.closing", &[&closing.id])?,
    )
        .into_response())
}

/// One channel of the day.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ChannelRow {
    channel: String,
    orders: i64,
    gross: String,
}

/// One paper order of the day.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct PaperRow {
    reference: String,
    channel: String,
    amount: String,
    business_time: String,
    late: bool,
}

/// What a closing's page needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/closing.html")]
pub(crate) struct ShowClosingPage {
    tx: Texts,
    closing: DailyClosing,
    outlet: Outlet,
    date: String,
    status: String,
    status_kind: &'static str,
    cutoff: String,
    cutoff_pending: String,
    cutoff_from: String,
    opens_at: String,
    orders: i64,
    sales: String,
    channels: Vec<ChannelRow>,
    paper: Vec<PaperRow>,
    late_count: i64,
    expected: String,
    counted: String,
    variance: String,
    cashiers: Vec<[String; 2]>,
    methods: Vec<[String; 2]>,
    paper_channels: Vec<[String; 2]>,
    today: String,
    notes: Vec<String>,
    can_submit: bool,
    can_approve: bool,
    can_reopen: bool,
    can_cutoff: bool,
    can_paper: bool,
}

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    if !user.has_permission(permissions::CLOSING_VIEW) {
        return Err(Error::Forbidden);
    }
    let closing = DailyClosing::find_or_404(&state.db, id).await?;
    let outlet = Outlet::find_or_404(&state.db, closing.outlet_id).await?;
    let date = closing
        .business_date
        .unwrap_or_else(wl_kernel::documents::today);
    let Totals {
        orders,
        gross,
        late_entries,
        channels,
        ..
    } = closings::totals(&state.db, &closing).await?;
    let paper = PaperOrder::of_day(&state.db, outlet.id, date)
        .await?
        .into_iter()
        .map(|p| PaperRow {
            reference: p.reference,
            channel: t(
                &lang,
                &format!("fnb.screen.channels.paper_channel.{}", p.channel),
            ),
            amount: aed(p.amount),
            business_time: p
                .occurred_at
                .map(wl_kernel::documents::dubai_text)
                .unwrap_or_default(),
            late: p.late,
        })
        .collect();
    let opens = outlet.opens_closing_at(date);
    let before_cutoff = renox::db::now() < opens;
    let mut notes = Vec::new();
    if closing.can_submit() && before_cutoff {
        notes.push(t(&lang, "fnb.screen.closing.note_cutoff"));
    }
    match closing.status {
        ClosingStatus::Approved => notes.push(t(&lang, "fnb.screen.closing.note_approved")),
        ClosingStatus::Submitted => notes.push(t(&lang, "fnb.screen.closing.note_submitted")),
        _ => {}
    }
    let cashiers =
        renox::auth::permissions::users_with_permission(&state.db, permissions::POS_SELL)
            .await?
            .into_iter()
            .map(|u| [u.id.to_string(), u.name])
            .collect();
    let methods = method::ALL
        .iter()
        .map(|m| {
            [
                (*m).to_owned(),
                t(&lang, &format!("fnb.screen.channels.paper_method.{m}")),
            ]
        })
        .collect();
    let paper_channels = [
        "counter",
        "phone",
        "website",
        "aggregator_a",
        "aggregator_b",
        "aggregator_c",
    ]
    .iter()
    .map(|c| {
        [
            (*c).to_owned(),
            t(&lang, &format!("fnb.screen.channels.paper_channel.{c}")),
        ]
    })
    .collect();
    Ok(View::typed(ShowClosingPage {
        tx: tx(&lang),
        date: date.to_string(),
        status: status_word(&lang, closing.status),
        status_kind: status_kind(closing.status),
        cutoff: clock_text(outlet.cutoff_for(date)),
        cutoff_pending: outlet
            .pending_cutoff_minutes
            .map(clock_text)
            .unwrap_or_default(),
        cutoff_from: outlet
            .pending_cutoff_from
            .map(|d| d.to_string())
            .unwrap_or_default(),
        opens_at: wl_kernel::documents::dubai_text(opens),
        orders,
        sales: aed(gross),
        channels: channels
            .into_iter()
            .map(|c| ChannelRow {
                channel: t(
                    &lang,
                    &format!("fnb.screen.channels.paper_channel.{}", c.channel),
                ),
                orders: c.orders,
                gross: aed(c.gross),
            })
            .collect(),
        paper,
        late_count: late_entries,
        expected: if closing.counted_cash.is_some() {
            aed(closing.expected_cash)
        } else {
            String::new()
        },
        counted: closing.counted_cash.map(aed).unwrap_or_default(),
        variance: closing.variance.map(aed).unwrap_or_default(),
        cashiers,
        methods,
        paper_channels,
        today: date.to_string(),
        notes,
        can_submit: closing.can_submit() && user.has_permission(permissions::CLOSING_SUBMIT),
        can_approve: closing.status == ClosingStatus::Submitted
            && user.has_permission(permissions::CLOSING_APPROVE),
        can_reopen: closing.status == ClosingStatus::Submitted
            && user.has_permission(permissions::CLOSING_REOPEN),
        can_cutoff: user.has_permission(permissions::CLOSING_CUTOFF),
        can_paper: closing.status != ClosingStatus::Approved
            && user.has_permission(permissions::PAPER_ORDERS_ENTER),
        closing,
        outlet,
    })
    .into_response())
}

fn back(lang: &Lang, key: &str, id: i64) -> Result<Response> {
    Ok((
        Toast::success(t(lang, key)),
        Redirect::route("fnb.closing", &[&id])?,
    )
        .into_response())
}

/// `POST /closings/{id}/submit`: counts the till and submits the closing.
async fn submit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let counted_text = field(&form, "counted");
    let counted = if counted_text.trim().is_empty() {
        None
    } else {
        parse_dirhams(&counted_text)
    };
    let Some(counted) = counted else {
        return Err(refused(t(&lang, "fnb.closing.problem.not_counted")));
    };
    let ask = SubmitAsk {
        counted,
        checklist_done: ticked(&form, "checklist"),
        cashier: field(&form, "cashier").trim().parse().ok(),
    };
    closings::submit(&state, Some(&user), id, &ask).await?;
    back(&lang, "fnb.closing.submitted", id)
}

/// `POST /closings/{id}/approve`.
async fn approve(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    closings::approve(&state, Some(&user), id).await?;
    back(&lang, "fnb.closing.approved", id)
}

/// `POST /closings/{id}/reopen`: the area manager reopens, with a reason.
async fn reopen(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    closings::reopen(&state, &user, id, &field(&form, "reason")).await?;
    back(&lang, "fnb.closing.reopened", id)
}

/// `POST /outlets/{id}/cutoff`: changes the cut-off from the next business day.
async fn cutoff(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    closings::set_cutoff(&state, &user, id, &field(&form, "time")).await?;
    Ok((
        Toast::success(t(&lang, "fnb.closing.cutoff_set")),
        Redirect::route("fnb.closings", &[])?,
    )
        .into_response())
}
