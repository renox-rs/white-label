//! The accident damage of a job card: the police report it needs and the
//! request to clear the flag (`AUT.ACL.20`, BL-AUT-07). The job card page shows
//! the card; these are the routes its forms post to.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /auto/job-cards/{id}/accident/clear` | `auto.job_card_accident_clear` | `job_cards.advise` |
//! | `POST /auto/job-cards/{id}/accident/decide` | `auto.job_card_accident_decide` | `job_cards.accident` |
//!
//! The police report itself is a document of the job card or its claim, added
//! on the documents page of the record (`/documents/of/auto.job_card/{id}`) and
//! accepted there by a second user.

use std::collections::BTreeMap;

use renox::prelude::*;
use serde::Serialize;

use super::text;
use crate::accident_flag;
use crate::documents::{GUARDED_JOB_CARD_APPROVE, PoliceVerdict, police_verdict};
use crate::job_card::JobCard;
use crate::permissions;

/// The routes of the accident flag.
pub fn routes() -> Routes {
    Routes::new()
        .post("/auto/job-cards/{id}/accident/clear", clear)
        .name("auto.job_card_accident_clear")
        .post("/auto/job-cards/{id}/accident/decide", decide)
        .name("auto.job_card_accident_decide")
        .require_auth()
}

/// What the job card page shows of the accident damage.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct AccidentView {
    /// Whether the job card is flagged as accident damage.
    pub flagged: bool,
    /// The police report in words: attached and accepted, or what is missing.
    pub report: String,
    /// Whether the report is attached and accepted.
    pub report_ok: bool,
    /// The documents page of the record, where the report is added.
    pub documents_link: String,
    /// Whether the advisor can ask to clear the flag.
    pub can_ask: bool,
    /// Whether a request to clear the flag waits.
    pub waiting: bool,
    /// Why it was asked, and by whom.
    pub waiting_text: String,
    /// Whether the user can decide the request.
    pub can_decide: bool,
}

/// What the page shows of the accident damage of `card` for `user`.
pub(crate) async fn view_of(
    state: &AppState,
    lang: &Lang,
    user: &AuthUser,
    card: &JobCard,
) -> Result<AccidentView> {
    let subject = card.subject();
    let documents_link = format!("/documents/of/{}/{}", subject.subject_type, subject.subject_id);
    if !card.accident_damage {
        return Ok(AccidentView {
            flagged: false,
            report: String::new(),
            report_ok: false,
            documents_link,
            can_ask: false,
            waiting: false,
            waiting_text: String::new(),
            can_decide: false,
        });
    }
    let verdict = police_verdict(state, GUARDED_JOB_CARD_APPROVE, card.id, None).await?;
    let (report, report_ok) = match verdict {
        PoliceVerdict::Valid => (text(lang, "police.attached"), true),
        PoliceVerdict::Refused(words) => (words, false),
    };
    let waiting = card.accident_clear_at.is_some();
    let waiting_text = match (&card.accident_clear_reason, card.accident_clear_by) {
        (Some(reason), Some(by)) => format!("{reason} · {}", super::user_name(&state.db, by).await),
        _ => String::new(),
    };
    let open = card.status.is_open();
    Ok(AccidentView {
        flagged: true,
        report,
        report_ok,
        documents_link,
        can_ask: open
            && !waiting
            && user.has_permission(permissions::JOB_CARDS_ADVISE)
            && !accident_flag::claim_linked(&state.db, card.id).await?,
        waiting,
        waiting_text,
        can_decide: waiting
            && user.has_permission(permissions::JOB_CARDS_ACCIDENT)
            && card.accident_clear_by != Some(user.id),
    })
}

/// `POST /auto/job-cards/{id}/accident/clear`: `reason`.
async fn clear(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let reason = form.get("reason").map_or("", String::as_str);
    accident_flag::request_clear(&state, &user, id, reason).await?;
    Ok((
        Toast::success(text(&lang, "police.clear_asked")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /auto/job-cards/{id}/accident/decide`: `decision` is `approve` or
/// `refuse`.
async fn decide(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let approve = form.get("decision").map(|d| d.trim()) == Some("approve");
    accident_flag::decide_clear(&state, &user, id, approve).await?;
    let key = if approve {
        "police.cleared"
    } else {
        "police.kept"
    };
    Ok((Toast::success(text(&lang, key)), renox::HxRefresh).into_response())
}
