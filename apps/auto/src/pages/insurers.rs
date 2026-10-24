//! The insurer pages (08 §6 *Insurer*, `AUT.INS.01`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /auto/insurers` | `auto.insurers` | `insurance.manage` |
//! | `POST /auto/insurers` | `auto.insurer_create` | `insurance.manage` |
//! | `GET /auto/insurers/{id}` | `auto.insurer` | `insurance.manage` |
//! | `POST /auto/insurers/{id}` | `auto.insurer_update` | `insurance.manage` |
//! | `POST /auto/insurers/{id}/active` | `auto.insurer_active` | `insurance.manage` |
//!
//! The insurance company is picked from the parties (`PartyDirectory::search`),
//! never typed as an id; the payer agreement and the rate card are those of
//! wl-invoicing.

use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};

use super::lists;
use super::{
    Texts, parse_money, parse_number, parse_optional_id, party_name, party_of, require, text,
};
use crate::insurer::Insurer;
use crate::insurers::{self, NewInsurer};
use crate::money_text;
use crate::permissions;
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/auto/insurers", index)
        .name("auto.insurers")
        .post("/auto/insurers", create)
        .name("auto.insurer_create")
        .get("/auto/insurers/{id}", show)
        .name("auto.insurer")
        .post("/auto/insurers/{id}", update)
        .name("auto.insurer_update")
        .post("/auto/insurers/{id}/active", set_active)
        .name("auto.insurer_active")
        .require_auth()
}

/// `"30 days"` and so on.
fn days_text(lang: &Lang, days: i64) -> String {
    texts::t_with(lang, "auto.insurers.days", &[("days", &days)])
}

/// The insurers grid: the name first and frozen (a link, drawn by the page),
/// where claims are emailed, the days to pay, the labour rate agreed in
/// dirhams and whether it takes new claims.
pub(crate) fn grid(t: lists::Text<'_>) -> Grid {
    let standing = vec![
        ("active".to_owned(), t("insurers.active")),
        ("stopped".to_owned(), t("insurers.stopped")),
    ];
    Grid::new("auto-insurers")
        .title(&t("insurers.title"))
        .column(
            Column::custom("name", &t("insurers.name"))
                .frozen()
                .mobile(),
        )
        .column(
            Column::text("claim_email", &t("insurers.email"))
                .searchable()
                .copyable(),
        )
        .column(Column::number("payment_terms_days", &t("insurers.terms")))
        .column(lists::money("labour_rate", &t("insurers.labour_rate")).mobile())
        .column(
            Column::select("standing", &t("fields.status"), standing)
                .computed("CASE WHEN {T}.\"active\" THEN 'active' ELSE 'stopped' END")
                .badges(&[("active", "success"), ("stopped", "neutral")])
                .mobile(),
        )
        .sort_by("-id")
        .row_url("/auto/insurers/{id}")
        .cards_on_mobile()
        .empty_state(&t("insurers.empty"), None)
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/insurers.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    insurers: GridPage<Insurer>,
}

/// `GET /auto/insurers`.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<View> {
    require(&user, permissions::INSURANCE_MANAGE)?;
    let page = grid(&lists::texts_in(&lang))
        .page(Insurer::query(), &request)
        .await?;
    let ids: Vec<i64> = page
        .items()
        .iter()
        .map(|insurer| insurer.party_id)
        .collect();
    let names = lists::party_names(&state, &lang, ids).await;
    let insurers = lists::with_currency(
        page,
        |insurer| json!({ "name": names.get(&insurer.party_id).cloned().unwrap_or_default() }),
    );
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        insurers,
    }))
}

/// The form that records or changes an insurer.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct InsurerForm {
    /// The insurance company, picked from the parties.
    #[serde(default)]
    pub party: String,
    /// The payer agreement of wl-invoicing.
    #[serde(default)]
    pub agreement: String,
    /// Where claims are emailed.
    #[serde(default)]
    pub email: String,
    /// The surveyors' contacts.
    #[serde(default)]
    pub surveyors: String,
    /// Days to pay.
    #[serde(default)]
    pub terms: String,
    /// The labour rate agreed, in dirhams.
    #[serde(default)]
    pub labour_rate: String,
    /// The storage rate agreed, in dirhams a day.
    #[serde(default)]
    pub storage_rate: String,
}

/// The terms the form asks for.
fn new_insurer(form: &InsurerForm, party: i64) -> Result<NewInsurer> {
    let mut new = NewInsurer::new(party);
    if let Some(agreement) = parse_optional_id(&form.agreement)? {
        new = new.agreement(agreement);
    }
    if !form.email.trim().is_empty() {
        new = new.email(&form.email);
    }
    if !form.surveyors.trim().is_empty() {
        new = new.surveyors(&form.surveyors);
    }
    if !form.terms.trim().is_empty() {
        new = new.terms(parse_number(&form.terms)?);
    }
    let labour = parse_money(&form.labour_rate)?;
    if labour > 0 {
        new = new.labour_rate(labour);
    }
    let storage = parse_money(&form.storage_rate)?;
    if storage > 0 {
        new = new.storage_rate(storage);
    }
    Ok(new)
}

/// `POST /auto/insurers`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<InsurerForm>,
) -> Result<Response> {
    require(&user, permissions::INSURANCE_MANAGE)?;
    let party = party_of(&state, &form.party).await?;
    let insurer = insurers::register(&state, &user, new_insurer(&form, party)?).await?;
    Ok((
        Toast::success(text(&lang, "insurers.recorded")),
        renox::HxRedirect(format!("/auto/insurers/{}", insurer.id)),
    )
        .into_response())
}

/// What the page of one insurer needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/insurer.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    id: i64,
    name: String,
    active: bool,
    email: String,
    surveyors: String,
    terms: String,
    labour_rate: String,
    storage_rate: String,
    agreement: String,
    agreement_options: Vec<[String; 2]>,
    rate_card: String,
    rate_card_url: String,
    raw_agreement: String,
    raw_email: String,
    raw_surveyors: String,
    raw_terms: String,
    raw_labour_rate: String,
    raw_storage_rate: String,
}

/// Dirhams as typed in a form, blank when there is none.
fn typed(fils: Option<i64>) -> String {
    fils.map_or_else(String::new, |f| format!("{}.{:02}", f / 100, f % 100))
}

/// `GET /auto/insurers/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    require(&user, permissions::INSURANCE_MANAGE)?;
    let insurer = Insurer::find_or_404(&state.db, id).await?;
    let none = text(&lang, "none");
    let card = insurers::rate_card_of(&state, &insurer, wl_kernel::documents::today()).await?;
    let agreement_name = match insurer.payer_agreement_id {
        Some(agreement) => super::choices::payer_agreement_name(&state.db, agreement).await,
        None => None,
    };
    let rate_card_url = match &card {
        Some(version) if user.has_permission("rate_cards.view") => {
            format!("/rate-cards/{}", version.rate_card)
        }
        _ => String::new(),
    };
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        id: insurer.id,
        name: party_name(&state, &lang.locale, insurer.party_id).await,
        active: insurer.active,
        email: insurer.claim_email.clone().unwrap_or_else(|| none.clone()),
        surveyors: insurer
            .surveyor_contacts
            .clone()
            .unwrap_or_else(|| none.clone()),
        terms: days_text(&lang, insurer.payment_terms_days),
        labour_rate: insurer.labour_rate.map_or_else(|| none.clone(), money_text),
        storage_rate: insurer
            .storage_rate_daily
            .map_or_else(|| none.clone(), money_text),
        agreement: agreement_name.clone().unwrap_or_else(|| none.clone()),
        agreement_options: match (insurer.payer_agreement_id, agreement_name) {
            (Some(agreement), Some(name)) => vec![[agreement.to_string(), name]],
            _ => Vec::new(),
        },
        rate_card: card.map_or_else(
            || text(&lang, "insurers.no_rate_card"),
            |v| {
                texts::t_with(
                    &lang,
                    "auto.insurers.rate_card_text",
                    &[("version", &v.version), ("from", &v.effective_from)],
                )
            },
        ),
        rate_card_url,
        raw_agreement: insurer
            .payer_agreement_id
            .map(|a| a.to_string())
            .unwrap_or_default(),
        raw_email: insurer.claim_email.clone().unwrap_or_default(),
        raw_surveyors: insurer.surveyor_contacts.clone().unwrap_or_default(),
        raw_terms: insurer.payment_terms_days.to_string(),
        raw_labour_rate: typed(insurer.labour_rate),
        raw_storage_rate: typed(insurer.storage_rate_daily),
    }))
}

/// `POST /auto/insurers/{id}`.
async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<InsurerForm>,
) -> Result<Response> {
    require(&user, permissions::INSURANCE_MANAGE)?;
    let insurer = Insurer::find_or_404(&state.db, id).await?;
    insurers::update(&state, &user, id, new_insurer(&form, insurer.party_id)?).await?;
    Ok((
        Toast::success(text(&lang, "insurers.saved")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /auto/insurers/{id}/active`: stops or resumes taking claims.
async fn set_active(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    require(&user, permissions::INSURANCE_MANAGE)?;
    let insurer = Insurer::find_or_404(&state.db, id).await?;
    insurers::set_active(&state, &user, id, !insurer.active).await?;
    Ok((
        Toast::success(text(&lang, "insurers.saved")),
        renox::HxRefresh,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_builds_the_terms() {
        let form = InsurerForm {
            email: "claims@orient.example".to_owned(),
            terms: "45".to_owned(),
            labour_rate: "120".to_owned(),
            agreement: "3".to_owned(),
            ..InsurerForm::default()
        };
        let new = new_insurer(&form, 9).unwrap();
        assert_eq!(new.party_id, 9);
        assert_eq!(new.payment_terms_days, 45);
        assert_eq!(new.labour_rate, Some(12_000));
        assert_eq!(new.payer_agreement_id, Some(3));
        assert_eq!(new.storage_rate_daily, None);
        assert!(new_insurer(&InsurerForm::default(), 9).is_ok());
    }

    #[test]
    fn rates_are_typed_in_dirhams() {
        assert_eq!(typed(Some(12_050)), "120.50");
        assert_eq!(typed(None), "");
    }
}
