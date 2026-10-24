//! The insurance policy pages (05 `CLN.INS.02`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /clinics/policies` | `clinics.policies` | `insurance_policies.view` |
//! | `POST /clinics/policies` | `clinics.policy_create` | `insurance_policies.manage` |
//! | `POST /clinics/policies/{id}/card` | `clinics.policy_card` | `insurance_policies.manage` (staff accept the card; the flag of `CLN.RUL.10` clears) |
//!
//! The list shows the plan, the member id and the dates; the card image is
//! not here (`CLN.ACL.12`).

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::documents::today;

use super::{Texts, invalid, parse_date, parse_party, party_exists, party_name, require, text};
use crate::card_requests;
use crate::grid_lookup;
use crate::permissions;
use crate::policies::{self, NewPolicy};
use crate::policy::InsurancePolicy;
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/clinics/policies", index)
        .name("clinics.policies")
        .post("/clinics/policies", create)
        .name("clinics.policy_create")
        .post("/clinics/policies/{id}/card", card)
        .name("clinics.policy_card")
        .require_auth()
}

/// The policies grid: search on the plan, the member id and (through
/// `patient_match`, see [`crate::grid_lookup`]) the patient's name, filters on
/// the payer (`payers`, the options), the dates and on whether the policy is
/// valid today. The names are drawn by the template. The card image is not
/// here (`CLN.ACL.12`).
pub(crate) fn grid(
    lang: &Lang,
    day: NaiveDate,
    payers: Vec<(String, String)>,
    patient_match: &str,
    can_export: bool,
) -> Grid {
    let state = format!(
        "CASE WHEN {{T}}.\"valid_from\" <= '{day}' \
         AND ({{T}}.\"valid_to\" IS NULL OR {{T}}.\"valid_to\" >= '{day}') \
         AND ({{T}}.\"card_valid_to\" IS NULL OR {{T}}.\"card_valid_to\" >= '{day}') \
         THEN 'valid' ELSE 'expired' END"
    );
    let grid = Grid::new("clinics-policies")
        .title(&text(lang, "policies.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::custom("patient", &text(lang, "policies.patient"))
                .frozen()
                .mobile(),
        )
        .column(
            Column::select("payer_party_id", &text(lang, "policies.payer"), payers).sortable(false),
        )
        .column(
            Column::text("patient_match", &text(lang, "policies.patient"))
                .computed(patient_match)
                .hidden()
                .searchable()
                .sortable(false)
                .filterable(false),
        )
        .column(
            Column::text("plan", &text(lang, "policies.plan"))
                .mobile()
                .searchable(),
        )
        .column(Column::text("member_id", &text(lang, "policies.member_id")).searchable())
        .column(Column::date("valid_from", &text(lang, "policies.valid_from")).hidden())
        .column(Column::date("valid_to", &text(lang, "policies.valid_to")).mobile())
        .column(Column::date("card_valid_to", &text(lang, "policies.card_valid_to")).hidden())
        .column(
            Column::select(
                "state",
                &text(lang, "fields.status"),
                vec![
                    ("valid", text(lang, "policies.valid_today")),
                    ("expired", text(lang, "policies.expired")),
                ],
            )
            .computed(&state)
            .badges(&[("valid", "success"), ("expired", "warning")])
            .mobile(),
        )
        // No row link: a policy has no page of its own; the card is accepted
        // on the appointment.
        .sort_by("-id")
        .cards_on_mobile()
        .empty_state(
            &text(lang, "policies.empty"),
            Some(&text(lang, "policies.subtitle")),
        );
    if can_export { grid.exports() } else { grid }
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/policies.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    policies: GridPage<InsurancePolicy>,
    can_manage: bool,
    today: String,
}

/// `GET /clinics/policies` (`?export=` needs `insurance_policies.manage`).
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    session: Session,
    request: GridRequest,
) -> Result<Response> {
    require(&user, permissions::POLICIES_VIEW)?;
    let day = today();
    let can_manage = user.has_permission(permissions::POLICIES_MANAGE);
    let payer_ids = InsurancePolicy::query()
        .pluck::<i64, _>(&state.db, "payer_party_id")
        .await?;
    let payers = grid_lookup::party_options(&state, &lang.locale, payer_ids).await;
    let term = grid_lookup::searched(&request, &session, "clinics-policies");
    let patient_match =
        grid_lookup::party_match(&state, term.as_deref(), &["patient_party_id"]).await;
    let grid = grid(&lang, day, payers, &patient_match, can_manage);
    if request.param("export").is_some() && !can_manage {
        return Err(Error::Forbidden);
    }
    let query = InsurancePolicy::query();
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut names = BTreeMap::new();
    for policy in page.items() {
        for party in [policy.patient(), policy.payer()] {
            if let std::collections::btree_map::Entry::Vacant(slot) = names.entry(party.0) {
                slot.insert(party_name(&state, &lang, party).await);
            }
        }
    }
    let page = page.extend(|policy| {
        json!({
            "patient": names.get(&policy.patient_party_id),
            "payer": names.get(&policy.payer_party_id),
        })
    });
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        policies: page,
        can_manage,
        today: day.to_string(),
    })
    .into_response())
}

/// The form that records a policy.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CreateForm {
    /// The patient, a party's id.
    #[serde(default)]
    pub patient: String,
    /// The payer, a party's id.
    #[serde(default)]
    pub payer: String,
    /// The plan.
    #[serde(default)]
    pub plan: String,
    /// The member id.
    #[serde(default)]
    pub member_id: String,
    /// The first day of cover.
    #[serde(default)]
    pub valid_from: String,
    /// The last day of cover.
    #[serde(default)]
    pub valid_to: String,
    /// The last day the card is valid.
    #[serde(default)]
    pub card_valid_to: String,
}

/// `POST /clinics/policies`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<CreateForm>,
) -> Result<Response> {
    require(&user, permissions::POLICIES_MANAGE)?;
    let patient = parse_party(&form.patient)
        .ok_or_else(|| invalid(&text(&lang, "policies.problem.patient")))?;
    let payer =
        parse_party(&form.payer).ok_or_else(|| invalid(&text(&lang, "policies.problem.payer")))?;
    if patient == payer {
        return Err(invalid(&text(&lang, "policies.problem.payer_is_patient")));
    }
    if !party_exists(&state, patient).await? {
        return Err(invalid(&text(&lang, "policies.problem.patient_unknown")));
    }
    if !party_exists(&state, payer).await? {
        return Err(invalid(&text(&lang, "policies.problem.payer_unknown")));
    }
    let from = parse_date(&form.valid_from)?.unwrap_or_else(today);
    let mut new = NewPolicy::new(
        PartyId(patient),
        PartyId(payer),
        &form.plan,
        &form.member_id,
        from,
    );
    if let Some(to) = parse_date(&form.valid_to)? {
        new = new.valid_to(to);
    }
    if let Some(to) = parse_date(&form.card_valid_to)? {
        new = new.card_valid_to(to);
    }
    if policies::is_on_file(&state, &new).await? {
        return Err(invalid(&text(&lang, "policies.problem.duplicate")));
    }
    policies::create(&state, Some(user.id), new).await?;
    Ok((
        Toast::success(text(&lang, "policies.created")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that records an updated card.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CardForm {
    /// The last day the new card is valid.
    #[serde(default)]
    pub card_valid_to: String,
}

/// `POST /clinics/policies/{id}/card`.
async fn card(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<CardForm>,
) -> Result<Response> {
    require(&user, permissions::POLICIES_MANAGE)?;
    let valid_to = parse_date(&form.card_valid_to)?;
    // Accepting the card also clears the flag of the appointments it now
    // covers (`CLN.RUL.10`).
    card_requests::accept_card(&state, id, valid_to).await?;
    Ok((
        Toast::success(text(&lang, "policies.card_updated")),
        renox::HxRefresh,
    )
        .into_response())
}
