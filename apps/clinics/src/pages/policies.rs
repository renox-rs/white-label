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

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::documents::today;

use super::{Texts, invalid, parse_date, parse_id, party_name, require, text};
use crate::card_requests;
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

/// One policy of the list.
#[derive(Serialize, Debug, Clone)]
pub struct PolicyRow {
    /// The primary key.
    pub id: i64,
    /// The patient's name.
    pub patient: String,
    /// The payer's name.
    pub payer: String,
    /// The plan.
    pub plan: String,
    /// The member id.
    pub member_id: String,
    /// The first day of cover.
    pub valid_from: String,
    /// The last day of cover, or a dash.
    pub valid_to: String,
    /// The last day the card is valid, or a dash.
    pub card_valid_to: String,
    /// *Valid* or *Expired* today, in the page's language.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/policies.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    policies: Vec<PolicyRow>,
    can_manage: bool,
    today: String,
}

/// `GET /clinics/policies`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    require(&user, permissions::POLICIES_VIEW)?;
    let all = InsurancePolicy::query()
        .order_by_desc(InsurancePolicy::ID)
        .get(&state.db)
        .await?;
    let dash = text(&lang, "none");
    let day = today();
    let mut rows = Vec::with_capacity(all.len());
    for policy in &all {
        let valid = policy.is_valid_on(day);
        rows.push(PolicyRow {
            id: policy.id,
            patient: party_name(&state, &lang, policy.patient()).await,
            payer: party_name(&state, &lang, policy.payer()).await,
            plan: policy.plan.clone(),
            member_id: policy.member_id.clone(),
            valid_from: policy.valid_from.to_string(),
            valid_to: policy
                .valid_to
                .map_or_else(|| dash.clone(), |d| d.to_string()),
            card_valid_to: policy
                .card_valid_to
                .map_or_else(|| dash.clone(), |d| d.to_string()),
            status: text(
                &lang,
                if valid {
                    "policies.valid_today"
                } else {
                    "policies.expired"
                },
            ),
            status_kind: if valid { "success" } else { "warning" },
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        policies: rows,
        can_manage: user.has_permission(permissions::POLICIES_MANAGE),
        today: day.to_string(),
    }))
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
    let patient = parse_id(&form.patient).ok_or_else(|| invalid("patient: 1, 2, 3 ..."))?;
    let payer = parse_id(&form.payer).ok_or_else(|| invalid("payer: 1, 2, 3 ..."))?;
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
