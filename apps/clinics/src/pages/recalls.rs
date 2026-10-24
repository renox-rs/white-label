//! The recall pages (05 §4.6, `CLN.ACL.01`; BL-CLN-14).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /clinics/recalls/{id}` | `clinics.recall` | `recalls.trace`, everywhere or in a branch (that branch's rows only; every view is audited) |
//! | `POST /clinics/recalls/{id}/notices` | `clinics.recall_notices` | `recalls.notify` |
//! | `GET /clinics/recall-calls` | `clinics.recall_calls` | `recalls.call` |
//! | `POST /clinics/recall-calls/{id}` | `clinics.recall_call` | `recalls.call` in the notice's branch |
//!
//! The trace list shows patient name, visit day and branch, never why the
//! patient came; a line that cannot prove the lot says *lot uncertain*. It
//! names the lot of each line (its number, never the product) and, for the
//! recall's owner (`recalls.manage`), a *Waive* button per line still to tell,
//! which posts to wl-inventory's route. The recall itself (its reason, its
//! lots, the hold, the close and the report) is wl-inventory's page, which
//! links here for the patients.

use std::collections::BTreeMap;

use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;

use super::{Texts, party_name, require, text, text_with};
use crate::grid_lookup;
use crate::permissions;
use crate::recall_notice::RecallNotice;
use crate::recalls;
use crate::texts;

/// Whether `user` may record calls somewhere: globally or in a branch.
fn may_call(user: &AuthUser) -> bool {
    user.allows(permissions::RECALLS_CALL)
        || user.has_permission_anywhere(permissions::RECALLS_CALL)
}

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/clinics/recalls/{id}", show)
        .name("clinics.recall")
        .post("/clinics/recalls/{id}/notices", send)
        .name("clinics.recall_notices")
        .get("/clinics/recall-calls", calls)
        .name("clinics.recall_calls")
        .post("/clinics/recall-calls/{id}", call)
        .name("clinics.recall_call")
        .require_auth()
}

/// One line of the trace list.
#[derive(Serialize, Debug, Clone)]
pub struct TraceLine {
    /// The line of the recall (what *Waive* posts to).
    pub line: i64,
    /// The recalled lot's number, never the product.
    pub lot: String,
    /// The patient.
    pub patient: String,
    /// The visit day.
    pub visit_date: String,
    /// The branch.
    pub branch: String,
    /// "Lot uncertain" when the line cannot prove the lot, else empty.
    pub uncertain: String,
    /// Not told, sent or waived.
    pub notice: String,
    /// The badge colour of the notice.
    pub notice_kind: &'static str,
    /// Whether the owner may waive the line: it is not told yet.
    pub can_waive: bool,
    /// Who the notice goes to when it is not the patient (the guardian).
    pub told_through: String,
}

/// The badge colour of a notice status.
fn notice_kind(notice: &str) -> &'static str {
    match notice {
        "sent" => "success",
        "waived" => "info",
        _ => "warning",
    }
}

/// What the recall page needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/recall.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    id: i64,
    lines: Vec<TraceLine>,
    can_notify: bool,
    can_waive: bool,
    progress: String,
}

/// `GET /clinics/recalls/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    // Everywhere, or in the branches of the visits: a manager of one branch
    // reads the rows of that branch only.
    if !user.has_permission(permissions::RECALLS_TRACE)
        && !user.has_permission_anywhere(permissions::RECALLS_TRACE)
    {
        return Err(Error::Forbidden);
    }
    let all = recalls::trace(&state, id).await?;
    let rows = recalls::readable(&user, all);
    if rows.is_empty() && !user.has_permission(permissions::RECALLS_TRACE) {
        return Err(Error::Forbidden);
    }
    recalls::audit_view(&state, &user, id).await?;
    let mut lines = Vec::with_capacity(rows.len());
    for row in &rows {
        let patient = match row.patient {
            Some(party) => party_name(&state, &lang, PartyId(party)).await,
            None => text(&lang, "none"),
        };
        let told_through =
            if row.patient.is_some_and(|party| party != row.addressee) && row.addressee != 0 {
                party_name(&state, &lang, PartyId(row.addressee)).await
            } else {
                text(&lang, "none")
            };
        let lot = match &row.lot {
            Some(lot) => wl_kernel::documents::subject_label(&state.db, lot)
                .await?
                .unwrap_or_else(|| text(&lang, "none")),
            None => text(&lang, "none"),
        };
        lines.push(TraceLine {
            line: row.line.subject_id,
            lot,
            patient,
            visit_date: row
                .on
                .map_or_else(|| text(&lang, "none"), |day| day.to_string()),
            branch: row.branch.clone(),
            uncertain: if row.is_uncertain() {
                text(&lang, "recall.lot_uncertain")
            } else {
                String::new()
            },
            notice: text(&lang, &format!("recall.notice.{}", row.notice)),
            notice_kind: notice_kind(&row.notice),
            can_waive: row.notice == "pending",
            told_through,
        });
    }
    let count = |wanted: &str| rows.iter().filter(|row| row.notice == wanted).count() as i64;
    let progress = text_with(
        &lang,
        "recall.progress",
        &[
            ("told", &count("sent")),
            ("waived", &count("waived")),
            ("to_tell", &count("pending")),
        ],
    );
    Ok(View::typed(ShowPage {
        progress,
        tx: texts::all(&lang),
        id,
        lines,
        can_notify: user.has_permission(permissions::RECALLS_NOTIFY),
        can_waive: user.has_permission(permissions::RECALLS_MANAGE),
    }))
}

/// `POST /clinics/recalls/{id}/notices`.
async fn send(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    require(&user, permissions::RECALLS_NOTIFY)?;
    recalls::send(&state, &user, id).await?;
    Ok((Toast::success(text(&lang, "recall.sent")), renox::HxRefresh).into_response())
}

/// What the calls page needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/recall_calls.html")]
pub(crate) struct CallsPage {
    tx: Texts,
    calls: GridPage<RecallNotice>,
    /// The notices on this page: each has a sheet that records the call.
    sheets: Vec<i64>,
}

/// The recall calls grid: search on the visit days and (through
/// `patient_match`, see `crate::grid_lookup`) the patient's name, a filter by
/// branch (`branch_sql` and `branches`, worked out from the notice's scope).
/// The patient and the button that records the call are drawn by the
/// template; no export, the list names patients (`CLN.ACL`), and no row link:
/// the call is recorded in the row's sheet.
pub(crate) fn calls_grid(
    lang: &Lang,
    patient_match: &str,
    branch_sql: &str,
    branches: Vec<(String, String)>,
) -> Grid {
    Grid::new("clinics-recall-calls")
        .title(&text(lang, "recall.calls_title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::custom("patient", &text(lang, "recall.patient"))
                .frozen()
                .mobile(),
        )
        .column(
            Column::text("visit_dates", &text(lang, "recall.call_days"))
                .mobile()
                .searchable(),
        )
        .column(
            Column::select("branch", &text(lang, "recall.branch"), branches)
                .computed(branch_sql)
                .mobile(),
        )
        .column(
            Column::text("patient_match", &text(lang, "recall.patient"))
                .computed(patient_match)
                .hidden()
                .searchable()
                .sortable(false)
                .filterable(false),
        )
        .column(Column::datetime(
            "gave_up_at",
            &text(lang, "recall.call_gave_up"),
        ))
        .column(Column::custom("record", &text(lang, "recall.call_record")).frozen_right())
        .sort_by("gave_up_at")
        .cards_on_mobile()
        .empty_state(
            &text(lang, "recall.calls_empty"),
            Some(&text(lang, "recall.calls_subtitle")),
        )
}

/// `GET /clinics/recall-calls`: the calls of the user's branches.
async fn calls(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    session: Session,
    request: GridRequest,
) -> Result<View> {
    if !may_call(&user) {
        return Err(Error::Forbidden);
    }
    // The notices in the user's branches, with the branch of each.
    let mut visible = Vec::new();
    let mut branches = BTreeMap::new();
    for notice in RecallNotice::waiting_for_a_call(&state.db).await? {
        let scope = recalls::notice_scope(&state.db, &notice).await?;
        if !user.has_permission_in(permissions::RECALLS_CALL, &scope) {
            continue;
        }
        branches.insert(
            notice.id,
            wl_kernel::scopes::label_of(&state.db, &scope).await?,
        );
        visible.push(notice.id);
    }
    let query = RecallNotice::query().where_(RecallNotice::ID.is_in(if visible.is_empty() {
        vec![-1]
    } else {
        visible
    }));
    let term = grid_lookup::searched(&request, &session, "clinics-recall-calls");
    let patient_match =
        grid_lookup::party_match(&state, term.as_deref(), &["patient_party_id", "party_id"]).await;
    let (branch_sql, branch_options) = grid_lookup::labelled("id", &branches);
    let page = calls_grid(&lang, &patient_match, &branch_sql, branch_options)
        .page(query, &request)
        .await?;
    let shown: Vec<i64> = page.items().iter().map(|notice| notice.id).collect();
    let mut patients = BTreeMap::new();
    for notice in page.items() {
        let patient = party_name(
            &state,
            &lang,
            PartyId(notice.patient_party_id.unwrap_or(notice.party_id)),
        )
        .await;
        patients.insert(notice.id, patient);
    }
    // The list shows names and visit days: every view is audited.
    recalls::audit_calls_view(&state, &user, &shown).await?;
    let page = page.extend(|notice| {
        json!({
            "patient": patients.get(&notice.id),
            "branch": branches.get(&notice.id),
        })
    });
    Ok(View::typed(CallsPage {
        tx: texts::all(&lang),
        calls: page,
        sheets: shown,
    }))
}

/// The form that records a call.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CallForm {
    /// What came of the call.
    #[serde(default)]
    pub outcome: String,
}

/// `POST /clinics/recall-calls/{id}`.
async fn call(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<CallForm>,
) -> Result<Response> {
    if !may_call(&user) {
        return Err(Error::Forbidden);
    }
    recalls::record_call(&state, &user, id, &form.outcome).await?;
    Ok((
        Toast::success(text(&lang, "recall.call_recorded")),
        renox::HxRefresh,
    )
        .into_response())
}
