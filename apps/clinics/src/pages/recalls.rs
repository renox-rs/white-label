//! The recall pages (05 §4.6, `CLN.ACL.01`; BL-CLN-14).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /clinics/recalls/{id}` | `clinics.recall` | `recalls.trace` (every view is audited) |
//! | `POST /clinics/recalls/{id}/notices` | `clinics.recall_notices` | `recalls.notify` |
//! | `GET /clinics/recall-calls` | `clinics.recall_calls` | `recalls.call` |
//! | `POST /clinics/recall-calls/{id}` | `clinics.recall_call` | `recalls.call` in the notice's branch |
//!
//! The trace list shows patient name, visit day and branch, never why the
//! patient came; a line that cannot prove the lot says *lot uncertain*.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;

use super::{Texts, party_name, require, text};
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
    /// Who the notice goes to when it is not the patient (the guardian).
    pub told_through: String,
}

/// What the recall page needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/recall.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    id: i64,
    lines: Vec<TraceLine>,
    can_notify: bool,
}

/// `GET /clinics/recalls/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    require(&user, permissions::RECALLS_TRACE)?;
    let rows = recalls::trace(&state, id).await?;
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
        lines.push(TraceLine {
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
            told_through,
        });
    }
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        id,
        lines,
        can_notify: user.has_permission(permissions::RECALLS_NOTIFY),
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

/// One call waiting for the front desk.
#[derive(Serialize, Debug, Clone)]
pub struct CallRow {
    /// The notice.
    pub id: i64,
    /// The patient.
    pub patient: String,
    /// The visit days.
    pub dates: String,
    /// The branch.
    pub branch: String,
    /// When the message gave up.
    pub gave_up: String,
}

/// What the calls page needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/recall_calls.html")]
pub(crate) struct CallsPage {
    tx: Texts,
    calls: Vec<CallRow>,
}

/// `GET /clinics/recall-calls`: the calls of the user's branches.
async fn calls(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    if !may_call(&user) {
        return Err(Error::Forbidden);
    }
    let mut rows = Vec::new();
    let mut shown = Vec::new();
    for notice in RecallNotice::waiting_for_a_call(&state.db).await? {
        let scope = recalls::notice_scope(&state.db, &notice).await?;
        if !user.has_permission_in(permissions::RECALLS_CALL, &scope) {
            continue;
        }
        let patient = party_name(
            &state,
            &lang,
            PartyId(notice.patient_party_id.unwrap_or(notice.party_id)),
        )
        .await;
        shown.push(notice.id);
        rows.push(CallRow {
            id: notice.id,
            patient,
            dates: notice.visit_dates.clone(),
            branch: wl_kernel::scopes::label_of(&state.db, &scope).await?,
            gave_up: notice
                .gave_up_at
                .map_or_else(|| text(&lang, "none"), wl_kernel::documents::dubai_text),
        });
    }
    // The list shows names and visit days: every view is audited.
    recalls::audit_calls_view(&state, &user, &shown).await?;
    Ok(View::typed(CallsPage {
        tx: texts::all(&lang),
        calls: rows,
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
