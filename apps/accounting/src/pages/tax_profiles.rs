//! The tax profile page of a client: the version in force, the versions, the
//! change waiting for approval, and the forms to ask for a change and to
//! update a contact (04 §6 *Tax profile*, `ACC.ACL.13`; BL-ACC-14).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /accounting/clients/{party}/tax-profile` | `accounting.tax_profile` | `tax_profiles.view` |
//! | `POST /accounting/clients/{party}/tax-profile` | `accounting.tax_profile_record` | `tax_profiles.manage` (the first version only) |
//! | `POST /accounting/clients/{party}/tax-profile/change` | `accounting.tax_profile_change` | `tax_profiles.manage` |
//! | `POST /accounting/clients/{party}/tax-profile/contact` | `accounting.tax_profile_contact` | `tax_profiles.manage` |
//! | `GET /accounting/tax-profile-changes/{id}` | `accounting.tax_profile_change_show` | `tax_profiles.view` (the link of the approval request) |
//!
//! The approval itself is the kernel's ("Needs your action"). A pending
//! change shows as *pending approval* with its effective date, highlighted
//! when it lies in the past.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::documents::today;

use super::{Texts, invalid, party_name, require, text, user_name};
use crate::permissions;
use crate::tax_profile::{self, TaxProfile, VatPeriod};
use crate::tax_profile_change::{self, TaxProfileChange};
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/accounting/clients/{party}/tax-profile", show)
        .name("accounting.tax_profile")
        .post("/accounting/clients/{party}/tax-profile", record_first)
        .name("accounting.tax_profile_record")
        .post("/accounting/clients/{party}/tax-profile/change", change)
        .name("accounting.tax_profile_change")
        .post("/accounting/clients/{party}/tax-profile/contact", contact)
        .name("accounting.tax_profile_contact")
        .get("/accounting/tax-profile-changes/{id}", show_change)
        .name("accounting.tax_profile_change_show")
        .require_auth()
}

/// One version of the profile, as the table shows it.
#[derive(Serialize, Debug, Clone)]
pub struct VersionRow {
    /// The day it takes effect.
    pub effective: String,
    /// The VAT period and stagger.
    pub vat: String,
    /// The financial year end.
    pub year_end: String,
    /// Who approved it.
    pub approved_by: String,
}

/// The change waiting for approval.
#[derive(Serialize, Debug, Clone)]
pub struct PendingView {
    /// The day it would take effect.
    pub effective: String,
    /// Whether that day is in the past: highlighted for the approver.
    pub past: bool,
    /// The fields it changes, in the page's language.
    pub fields: String,
    /// Who asked.
    pub requested_by: String,
    /// Why.
    pub note: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "accounting/tax_profile.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    party: i64,
    client: String,
    versions: Vec<VersionRow>,
    waiting: Option<PendingView>,
    can_manage: bool,
    can_record: bool,
    periods: Vec<[String; 2]>,
    period_selected: String,
    trn: String,
    vat_registered: bool,
    vat_first_period_end: String,
    financial_year_end: String,
    ct_registration_no: String,
    small_business_relief: bool,
    qfzp: bool,
    contact_email: String,
    notes: String,
    today: String,
}

/// The names of `fields` in the page's language, comma separated.
fn field_names(lang: &Lang, fields: &[&str]) -> String {
    fields
        .iter()
        .map(|name| text(lang, &format!("tax_profile.field.{name}")))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `GET /accounting/clients/{party}/tax-profile`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(party): Path<i64>,
) -> Result<View> {
    require(&user, permissions::TAX_PROFILES_VIEW)?;
    let client = PartyId(party);
    let versions = TaxProfile::versions(&state.db, client).await?;
    let latest = versions.last().cloned().unwrap_or_default();
    let mut rows = Vec::new();
    for version in &versions {
        let approver = match version.approved_by {
            Some(id) => user_name(&state.db, id).await,
            None => text(&lang, "none"),
        };
        rows.push(VersionRow {
            effective: version.effective_date.to_string(),
            vat: if version.vat_registered {
                format!(
                    "{} {}",
                    text(
                        &lang,
                        &format!("tax_profile.period.{}", version.vat_period.as_str())
                    ),
                    version
                        .vat_first_period_end
                        .map(|d| d.to_string())
                        .unwrap_or_default()
                )
            } else {
                text(&lang, "tax_profile.period.not_registered")
            },
            year_end: version
                .financial_year_end
                .map(|d| d.to_string())
                .unwrap_or_default(),
            approved_by: approver,
        });
    }
    let pending = match TaxProfileChange::pending(&state.db, client).await? {
        Some(change) => Some(PendingView {
            effective: change.effective_date.to_string(),
            past: change.is_in_the_past(today()),
            fields: field_names(&lang, &change.fields()),
            requested_by: match change.requested_by {
                Some(id) => user_name(&state.db, id).await,
                None => text(&lang, "none"),
            },
            note: change.note.clone().unwrap_or_default(),
        }),
        None => None,
    };
    let periods = VatPeriod::ALL
        .iter()
        .map(|p| {
            [
                p.as_str().to_owned(),
                text(&lang, &format!("tax_profile.period.{}", p.as_str())),
            ]
        })
        .collect();
    let date = |d: Option<NaiveDate>| d.map(|d| d.to_string()).unwrap_or_default();
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        party,
        client: party_name(&state, &lang, client).await,
        versions: rows,
        waiting: pending,
        can_manage: user.has_permission(permissions::TAX_PROFILES_MANAGE) && !versions.is_empty(),
        can_record: user.has_permission(permissions::TAX_PROFILES_MANAGE) && versions.is_empty(),
        periods,
        period_selected: latest.vat_period.as_str().to_owned(),
        trn: latest.trn.clone().unwrap_or_default(),
        vat_registered: latest.vat_registered,
        vat_first_period_end: date(latest.vat_first_period_end),
        financial_year_end: date(latest.financial_year_end),
        ct_registration_no: latest.ct_registration_no.clone().unwrap_or_default(),
        small_business_relief: latest.small_business_relief,
        qfzp: latest.qfzp,
        contact_email: latest.contact_email.clone().unwrap_or_default(),
        notes: latest.notes.clone().unwrap_or_default(),
        today: today().to_string(),
    }))
}

/// The form that asks for a change.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ChangeForm {
    /// The VAT TRN.
    #[serde(default)]
    pub trn: String,
    /// `1` when VAT registered.
    #[serde(default)]
    pub vat_registered: String,
    /// `not_registered`, `monthly` or `quarterly`.
    #[serde(default)]
    pub vat_period: String,
    /// The last day of the first VAT period.
    #[serde(default)]
    pub vat_first_period_end: String,
    /// The last day of the financial year.
    #[serde(default)]
    pub financial_year_end: String,
    /// The corporate tax registration number.
    #[serde(default)]
    pub ct_registration_no: String,
    /// `1` when Small Business Relief.
    #[serde(default)]
    pub small_business_relief: String,
    /// `1` when a QFZP.
    #[serde(default)]
    pub qfzp: String,
    /// The day the change applies from.
    #[serde(default)]
    pub effective_date: String,
    /// Why.
    #[serde(default)]
    pub note: String,
}

fn ticked(value: &str) -> bool {
    matches!(value.trim(), "1" | "on" | "true")
}

fn optional(value: &str) -> Option<String> {
    Some(value.trim().to_owned()).filter(|v| !v.is_empty())
}

fn date_of(value: &str, field: &str) -> Result<Option<NaiveDate>> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    value
        .trim()
        .parse()
        .map(Some)
        .map_err(|_| invalid(&format!("{field}: YYYY-MM-DD")))
}

/// The profile the form describes, without its effective date.
fn profile_of(form: &ChangeForm, party: i64) -> Result<TaxProfile> {
    let vat_period = VatPeriod::ALL
        .iter()
        .copied()
        .find(|p| p.as_str() == form.vat_period.trim())
        .unwrap_or_default();
    Ok(TaxProfile {
        client_party_id: party,
        trn: optional(&form.trn),
        vat_registered: ticked(&form.vat_registered),
        vat_period,
        vat_first_period_end: date_of(&form.vat_first_period_end, "vat_first_period_end")?,
        financial_year_end: date_of(&form.financial_year_end, "financial_year_end")?,
        ct_registration_no: optional(&form.ct_registration_no),
        small_business_relief: ticked(&form.small_business_relief),
        qfzp: ticked(&form.qfzp),
        ..TaxProfile::default()
    })
}

/// `POST /accounting/clients/{party}/tax-profile`: records the first version
/// of the client's profile, the onboarding record `activate` waits for (04
/// §4.1 step 7). It counts as approved by the user who records it
/// (`tax_profile::record`); a client that has a profile changes it with
/// [`change`], which another manager approves (`ACC.ACL.13`).
async fn record_first(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(party): Path<i64>,
    Form(form): Form<ChangeForm>,
) -> Result<Response> {
    require(&user, permissions::TAX_PROFILES_MANAGE)?;
    let effective = date_of(&form.effective_date, "effective_date")?
        .ok_or_else(|| invalid("effective_date: YYYY-MM-DD"))?;
    let mut profile = profile_of(&form, party)?;
    profile.effective_date = effective;
    profile.notes = optional(&form.note);
    tax_profile::record(&state.db, Some(user.id), profile).await?;
    Ok((
        Toast::success(text(&lang, "tax_profile.recorded")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /accounting/clients/{party}/tax-profile/change`.
async fn change(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(party): Path<i64>,
    Form(form): Form<ChangeForm>,
) -> Result<Response> {
    require(&user, permissions::TAX_PROFILES_MANAGE)?;
    let effective = date_of(&form.effective_date, "effective_date")?
        .ok_or_else(|| invalid("effective_date: YYYY-MM-DD"))?;
    let proposed = profile_of(&form, party)?;
    tax_profile_change::request(
        &state,
        &user,
        PartyId(party),
        proposed,
        effective,
        Some(form.note.as_str()),
    )
    .await?;
    Ok((
        Toast::success(text(&lang, "tax_profile.requested")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that updates the contact and the notes.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ContactForm {
    /// The contact e-mail.
    #[serde(default)]
    pub contact_email: String,
    /// The notes.
    #[serde(default)]
    pub notes: String,
}

/// `POST /accounting/clients/{party}/tax-profile/contact`.
async fn contact(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(party): Path<i64>,
    Form(form): Form<ContactForm>,
) -> Result<Response> {
    tax_profile_change::update_contact(
        &state,
        &user,
        PartyId(party),
        Some(form.contact_email.as_str()),
        Some(form.notes.as_str()),
    )
    .await?;
    Ok((
        Toast::success(text(&lang, "tax_profile.contact_saved")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `GET /accounting/tax-profile-changes/{id}`: the link of the approval
/// request goes to the client's profile page, where the change is shown.
async fn show_change(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response> {
    require(&user, permissions::TAX_PROFILES_VIEW)?;
    let change = TaxProfileChange::find_or_404(&state.db, id).await?;
    Ok(Redirect::route("accounting.tax_profile", &[&change.client_party_id])?.into_response())
}
