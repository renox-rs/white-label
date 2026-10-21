//! The back-office actions on a variation (02 Construction §4.5, §7.1
//! *Variation*; BL-CON-11):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /construction/projects/{id}/variations` | `construction.variation_create` | `variations.record` in the project |
//! | `POST /construction/variations/{id}/price` | `construction.variation_price` | `variations.record` in the project |
//! | `POST /construction/variations/{id}/submit` | `construction.variation_submit` | `variations.submit` in the project |
//! | `POST /construction/variations/{id}/approve` | `construction.variation_approve` | `variations.record` in the project |
//! | `POST /construction/variations/{id}/reject` | `construction.variation_reject` | `variations.record` in the project |
//! | `POST /construction/variations/{id}/resubmit` | `construction.variation_resubmit` | `variations.submit` in the project |
//! | `POST /construction/variations/{id}/claim` | `construction.variation_claim` | `variations.submit` in the project |
//! | `POST /construction/variations/{id}/settle-claim` | `construction.variation_settle_claim` | `variations.record` in the project |
//! | `POST /construction/variations/{id}/abandon` | `construction.variation_abandon` | `variations.submit` in the project |
//! | `POST /construction/variations/{id}/withdraw` | `construction.variation_withdraw` | `variations.submit` in the project |
//! | `GET /construction/projects/{id}/variation-report` | `construction.variation_report` | `projects.view`, `variations.record` or `variations.submit` in the project |
//!
//! The approval of an abandon is the kernel's: the MD finds the request under
//! "Needs your action".

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;

use crate::pages::{answer, date_field, refused, text};
use crate::permissions;
use crate::project::Project;
use crate::texts::t;
use crate::variation::{self, Abandoning, Decision, NewVariation, VariationKind};

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .post("/construction/projects/{id}/variations", create_variation)
        .name("construction.variation_create")
        .post("/construction/variations/{id}/price", price_variation)
        .name("construction.variation_price")
        .post("/construction/variations/{id}/submit", submit_variation)
        .name("construction.variation_submit")
        .post("/construction/variations/{id}/approve", approve_variation)
        .name("construction.variation_approve")
        .post("/construction/variations/{id}/reject", reject_variation)
        .name("construction.variation_reject")
        .post("/construction/variations/{id}/resubmit", resubmit_variation)
        .name("construction.variation_resubmit")
        .post("/construction/variations/{id}/claim", claim_variation)
        .name("construction.variation_claim")
        .post(
            "/construction/variations/{id}/settle-claim",
            settle_variation_claim,
        )
        .name("construction.variation_settle_claim")
        .post("/construction/variations/{id}/abandon", abandon_variation)
        .name("construction.variation_abandon")
        .post("/construction/variations/{id}/withdraw", withdraw_variation)
        .name("construction.variation_withdraw")
        .get(
            "/construction/projects/{id}/variation-report",
            variation_report,
        )
        .name("construction.variation_report")
        .require_auth()
}

/// A toast for a step of a variation, refreshing the page.
fn done(lang: &Lang, key: &str) -> Result<Response> {
    Ok((Toast::success(t(lang, key)), renox::HxRefresh).into_response())
}

/// The whole number field `name` of a form; `None` when empty, a 422 when it
/// is not a number.
fn number_field(form: &BTreeMap<String, String>, name: &str, lang: &Lang) -> Result<Option<i64>> {
    match text(form, name).trim() {
        "" => Ok(None),
        value => value
            .parse::<i64>()
            .map(Some)
            .map_err(|_| refused(t(lang, "construction.bad_input"))),
    }
}

/// Whether the checkbox or flag `name` of a form is on.
fn flag(form: &BTreeMap<String, String>, name: &str) -> bool {
    matches!(
        text(form, name).trim().to_lowercase().as_str(),
        "on" | "1" | "true" | "yes"
    )
}

/// What the form of a variation carries.
fn new_variation(form: &BTreeMap<String, String>, lang: &Lang) -> Result<NewVariation> {
    let kind = [
        VariationKind::Addition,
        VariationKind::Omission,
        VariationKind::Time,
    ]
    .into_iter()
    .find(|kind| kind.as_str() == text(form, "kind").trim())
    .ok_or_else(|| refused(t(lang, "construction.variation.kind_unknown")))?;
    let event_date: Option<NaiveDate> = date_field(form, "event_date", lang)?;
    Ok(NewVariation {
        kind,
        description: text(form, "description"),
        site_instruction: Some(text(form, "site_instruction")),
        executed_before_approval: flag(form, "executed"),
        priced_amount: number_field(form, "priced_amount", lang)?.unwrap_or_default(),
        days_asked: number_field(form, "days_asked", lang)?,
        event: Some(text(form, "event")),
        event_date,
        boq_line_id: number_field(form, "boq_line_id", lang)?,
        omitted_quantity: number_field(form, "omitted_quantity", lang)?,
    })
}

/// The client's decision a form carries.
fn decision(form: &BTreeMap<String, String>, lang: &Lang) -> Result<Decision> {
    Ok(Decision {
        client_reference: text(form, "client_reference"),
        approved_amount: number_field(form, "approved_amount", lang)?,
        days_granted: number_field(form, "days_granted", lang)?,
    })
}

/// `POST /construction/projects/{id}/variations`: `kind` (`addition`,
/// `omission` or `time`), `description`, `site_instruction`, `executed`,
/// `priced_amount` (fils), `days_asked`, `event` and `event_date`.
async fn create_variation(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let new = new_variation(&form, &lang)?;
    variation::create(&state, &user, id, &new)
        .await
        .map_err(answer)?;
    done(&lang, "construction.variation.created")
}

/// `POST /construction/variations/{id}/price`: the fields of a variation,
/// for a `draft`.
async fn price_variation(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let new = new_variation(&form, &lang)?;
    variation::update_draft(&state, &user, id, &new)
        .await
        .map_err(answer)?;
    done(&lang, "construction.variation.priced")
}

/// `POST /construction/variations/{id}/submit`.
async fn submit_variation(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    variation::submit(&state, &user, id).await.map_err(answer)?;
    done(&lang, "construction.variation.submitted")
}

/// `POST /construction/variations/{id}/approve`: `client_reference` and the
/// `approved_amount` (fils) or the `days_granted`.
async fn approve_variation(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let decided = decision(&form, &lang)?;
    variation::approve(&state, &user, id, &decided)
        .await
        .map_err(answer)?;
    done(&lang, "construction.variation.approved")
}

/// `POST /construction/variations/{id}/reject`: `reason`.
async fn reject_variation(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    variation::reject(&state, &user, id, &text(&form, "reason"))
        .await
        .map_err(answer)?;
    done(&lang, "construction.variation.rejected")
}

/// `POST /construction/variations/{id}/resubmit`.
async fn resubmit_variation(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    variation::resubmit(&state, &user, id)
        .await
        .map_err(answer)?;
    done(&lang, "construction.variation.resubmitted")
}

/// `POST /construction/variations/{id}/claim`.
async fn claim_variation(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    variation::claim(&state, &user, id).await.map_err(answer)?;
    done(&lang, "construction.variation.claimed")
}

/// `POST /construction/variations/{id}/settle-claim`: `client_reference` and
/// the agreed `approved_amount` (fils).
async fn settle_variation_claim(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let decided = decision(&form, &lang)?;
    variation::settle_claim(&state, &user, id, &decided)
        .await
        .map_err(answer)?;
    done(&lang, "construction.variation.claim_settled")
}

/// `POST /construction/variations/{id}/abandon`: asks the MD.
async fn abandon_variation(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let key = match variation::abandon(&state, &user, id)
        .await
        .map_err(answer)?
    {
        Abandoning::Done(_) => "construction.variation.abandoned",
        Abandoning::AwaitingMd(_) => "construction.variation.abandon_requested",
    };
    done(&lang, key)
}

/// `POST /construction/variations/{id}/withdraw`.
async fn withdraw_variation(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    variation::withdraw(&state, &user, id)
        .await
        .map_err(answer)?;
    done(&lang, "construction.variation.withdrawn")
}

/// `GET /construction/projects/{id}/variation-report`: every variation with
/// its price, the amount the client approved and the gap, in JSON.
async fn variation_report(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response> {
    let project = Project::find_or_404(&state.db, id).await?;
    let scope = project.scope();
    let may = [
        permissions::PROJECTS_VIEW,
        permissions::VARIATIONS_RECORD,
        permissions::VARIATIONS_SUBMIT,
    ]
    .iter()
    .any(|permission| user.has_permission_in(permission, &scope));
    if !may {
        return Err(Error::Forbidden);
    }
    let lines = variation::report_of(&state.db, id).await?;
    let body = renox::serde_json::to_string(&lines)
        .map_err(|error| Error::Internal(renox::anyhow::Error::new(error)))?;
    Ok(([("content-type", "application/json")], body).into_response())
}
