//! The back-office pages (wl-parties §6, DESIGN-SYSTEM):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /parties` | `wl_parties.index` | `parties.view` (`?export=` also `parties.export`) |
//! | `GET /parties/new` | `wl_parties.create` | `parties.create` |
//! | `POST /parties` | `wl_parties.store` | `parties.create` |
//! | `GET /parties/{id}` | `wl_parties.show` | `parties.view` |
//!
//! Lists, search, filters and exports read the party's `hold_reason`, which
//! is only ever the display code, so they cannot show a restricted reason
//! (`PTY.ACL.02`). Every template is compiled in and can be replaced by the
//! app (`resources/views/wl-parties/…`).

use std::collections::BTreeMap;

use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::state_machine;
use serde::{Deserialize, Serialize};
use wl_kernel::{PartyKind, PartyStatus};

use crate::contacts::{ContactPoint, ContactType};
use crate::create::{NewParty, create};
use crate::details::{CompanyDetails, PersonDetails};
use crate::party::{Channel, Language, Party};
use crate::permissions;
use crate::roles::PartyRole;
use crate::texts;

/// The hold reasons the list's filter offers: the neutral code and the
/// non-restricted defaults (wl-parties §7). Restricted codes are never in
/// `hold_reason`, so they are never offered (`PTY.ACL.02`).
pub const FILTER_REASONS: [&str; 5] = [
    wl_kernel::NEUTRAL_BLOCK_REASON,
    "non_payment",
    "legal_dispute",
    "fraud_suspected",
    "other",
];

/// The texts a template reads as `tx['fields.code']`.
type Texts = BTreeMap<&'static str, String>;

/// The module's routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .get("/parties/new", new)
        .name("wl_parties.create")
        .post("/parties", store)
        .name("wl_parties.store")
        .require_permission(permissions::CREATE)
        .merge(
            Routes::new()
                .get("/parties", index)
                .name("wl_parties.index")
                .get("/parties/{id}", show)
                .name("wl_parties.show")
                .require_permission(permissions::VIEW),
        )
}

/// The package text `key` (without the `wl_parties.` prefix) in a language.
type Text<'a> = &'a dyn Fn(&str) -> String;

/// The package texts in `lang`.
fn texts_in(lang: &Lang) -> impl Fn(&str) -> String + '_ {
    move |key: &str| texts::t(lang, &format!("wl_parties.{key}"))
}

/// `(value, label)` pairs for a select, labelled with `prefix.value`.
fn options<'a>(
    t: Text<'_>,
    prefix: &str,
    values: impl IntoIterator<Item = &'a str>,
) -> Vec<(String, String)> {
    values
        .into_iter()
        .map(|value| (value.to_owned(), t(&format!("{prefix}.{value}"))))
        .collect()
}

/// The parties grid: search on the code and names, filters by kind,
/// status and hold reason, and the export menu for `can_export`.
pub fn grid(lang: &Lang, can_export: bool) -> Grid {
    grid_with(&texts_in(lang), can_export)
}

/// [`grid`] with the texts of `t`.
fn grid_with(t: Text<'_>, can_export: bool) -> Grid {
    let grid = Grid::new("wl-parties")
        .title(&t("title"))
        .column(
            Column::text("code", &t("fields.code"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/parties/{id}"),
        )
        .column(
            Column::text("display_name", &t("fields.display_name"))
                .mobile()
                .searchable(),
        )
        .column(Column::text("legal_name_en", &t("fields.legal_name_en")).searchable())
        .column(
            Column::text("legal_name_ar", &t("fields.legal_name_ar"))
                .searchable()
                .hidden(),
        )
        .column(Column::select(
            "kind",
            &t("fields.kind"),
            options(t, "kind", PartyKind::ALL.iter().map(PartyKind::as_str)),
        ))
        .column(
            Column::select(
                "status",
                &t("fields.status"),
                options(
                    t,
                    "status",
                    PartyStatus::ALL.iter().map(PartyStatus::as_str),
                ),
            )
            .badges(&[
                ("active", "success"),
                ("blocked", "danger"),
                ("archived", "neutral"),
                ("merged", "neutral"),
            ])
            .mobile(),
        )
        .column(Column::select(
            "hold_reason",
            &t("fields.hold_reason"),
            options(t, "reason", FILTER_REASONS),
        ))
        .column(Column::datetime("created_at", &t("fields.created_at")).hidden())
        .sort_by("code")
        .row_url("/parties/{id}")
        .cards_on_mobile()
        .empty_state(&t("empty"), Some(&t("empty_hint")));
    if can_export { grid.exports() } else { grid }
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-parties/index.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    parties: GridPage<Party>,
}

/// `GET /parties`: the grid, or its export (`?export=csv|xlsx|print`,
/// with `parties.export`).
async fn index(user: AuthUser, lang: Lang, request: GridRequest) -> Result<Response> {
    let can_export = user.has_permission(permissions::EXPORT);
    let grid = grid(&lang, can_export);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    if let Some(file) = grid.export(Party::query(), &request).await? {
        return Ok(file);
    }
    let parties = grid.page(Party::query(), &request).await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        parties,
    })
    .into_response())
}

/// What the create form needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-parties/new.html")]
pub(crate) struct NewPage {
    tx: Texts,
    kinds: Vec<(String, String)>,
    languages: Vec<(String, String)>,
    channels: Vec<(String, String)>,
}

/// `GET /parties/new`: the create form.
async fn new(lang: Lang) -> Result<View> {
    let t = texts_in(&lang);
    Ok(View::typed(NewPage {
        tx: texts::all(&lang),
        kinds: options(&t, "kind", PartyKind::ALL.iter().map(PartyKind::as_str)),
        languages: options(&t, "language", Language::ALL.iter().map(Language::as_str)),
        channels: options(&t, "channel", Channel::ALL.iter().map(Channel::as_str)),
    }))
}

/// The create form. The service checks the rest (a 15-digit TRN, one
/// primary per contact type).
#[derive(Deserialize, Validate, Debug)]
pub struct PartyForm {
    /// Person or company.
    pub kind: PartyKind,
    /// The name to show; the English legal name when blank.
    #[validate(max = 200)]
    pub display_name: Option<String>,
    /// The legal name in English.
    #[validate(required, max = 200)]
    pub legal_name_en: String,
    /// The legal name in Arabic.
    #[validate(max = 200)]
    pub legal_name_ar: Option<String>,
    /// The language documents and messages use.
    #[serde(default)]
    pub default_language: Language,
    /// The preferred channel.
    #[serde(default)]
    pub preferred_channel: Channel,
    /// The primary e-mail address.
    #[validate(email, max = 200)]
    pub email: Option<String>,
    /// The primary mobile number.
    #[validate(max = 30)]
    pub mobile: Option<String>,
    /// A company's TRN (sent only for a company).
    #[validate(max = 15)]
    pub trn: Option<String>,
}

impl PartyForm {
    /// The service's input for this form.
    pub fn into_new_party(self) -> NewParty {
        let primary = |contact_type, value: Option<String>| {
            value
                .filter(|v| !v.trim().is_empty())
                .map(|value| ContactPoint {
                    contact_type,
                    value,
                    is_primary: true,
                    ..Default::default()
                })
        };
        let trn = self.trn.filter(|trn| !trn.trim().is_empty());
        NewParty {
            kind: self.kind,
            display_name: self.display_name.unwrap_or_default(),
            legal_name_en: self.legal_name_en,
            legal_name_ar: self.legal_name_ar,
            default_language: self.default_language,
            preferred_channel: self.preferred_channel,
            person: (self.kind == PartyKind::Person).then(PersonDetails::default),
            company: (self.kind == PartyKind::Company).then(|| CompanyDetails {
                vat_registered: trn.is_some(),
                trn,
                ..Default::default()
            }),
            roles: Vec::new(),
            contacts: [
                primary(ContactType::Email, self.email),
                primary(ContactType::Mobile, self.mobile),
            ]
            .into_iter()
            .flatten()
            .collect(),
            source: "back_office".to_owned(),
        }
    }
}

/// `POST /parties`: creates the party and opens its page.
async fn store(
    State(state): State<AppState>,
    lang: Lang,
    Valid(form): Valid<PartyForm>,
) -> Result<Response> {
    let party = create(&state, form.into_new_party()).await?;
    Ok((
        Toast::success(texts::t(&lang, "wl_parties.created")),
        Redirect::route("wl_parties.show", &[&party.id])?,
    )
        .into_response())
}

/// What the party page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-parties/show.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    party: Party,
    person: Option<PersonDetails>,
    company: Option<CompanyDetails>,
    contacts: Vec<ContactPoint>,
    roles: Vec<PartyRole>,
    timeline: Vec<renox::serde_json::Value>,
}

/// `GET /parties/{id}`: the card, contact points, roles and history.
async fn show(State(db): State<Db>, lang: Lang, Found(party): Found<Party>) -> Result<View> {
    let person = PersonDetails::query()
        .where_(PersonDetails::PARTY_ID.eq(party.id))
        .first(&db)
        .await?;
    let company = CompanyDetails::query()
        .where_(CompanyDetails::PARTY_ID.eq(party.id))
        .first(&db)
        .await?;
    let contacts = ContactPoint::query()
        .where_(ContactPoint::PARTY_ID.eq(party.id))
        .order_by(ContactPoint::CONTACT_TYPE)
        .order_by_desc(ContactPoint::IS_PRIMARY)
        .order_by(ContactPoint::ID)
        .get(&db)
        .await?;
    let roles = PartyRole::query()
        .where_(PartyRole::PARTY_ID.eq(party.id))
        .order_by(PartyRole::ROLE_KEY)
        .get(&db)
        .await?;
    let history = state_machine::history(&db, Party::TABLE, party.id).await?;
    let timeline = state_machine::timeline(&history, Some(&state_machine::info_of::<Party>()));
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        party,
        person,
        company,
        contacts,
        roles,
        timeline,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(kind: PartyKind) -> PartyForm {
        PartyForm {
            kind,
            display_name: None,
            legal_name_en: "Al Noor Trading LLC".to_owned(),
            legal_name_ar: None,
            default_language: Language::En,
            preferred_channel: Channel::Email,
            email: Some("accounts@alnoor.example".to_owned()),
            mobile: Some(" ".to_owned()),
            trn: Some("100123456700003".to_owned()),
        }
    }

    #[test]
    fn a_company_form_keeps_the_trn_and_one_primary_email() {
        let new = form(PartyKind::Company).into_new_party();
        let company = new.company.expect("company details");
        assert_eq!(company.trn.as_deref(), Some("100123456700003"));
        assert!(company.vat_registered);
        assert!(new.person.is_none());
        assert_eq!(new.contacts.len(), 1);
        assert_eq!(new.contacts[0].contact_type, ContactType::Email);
        assert!(new.contacts[0].is_primary);
        assert_eq!(new.source, "back_office");
    }

    #[test]
    fn a_person_form_has_no_company_details() {
        let new = form(PartyKind::Person).into_new_party();
        assert!(new.company.is_none());
        assert!(new.person.is_some());
    }

    #[test]
    fn the_grid_offers_no_restricted_reason_and_shows_blocked_as_on_hold() {
        let english = |key: &str| texts::english(&format!("wl_parties.{key}"));
        let grid = grid_with(&english, false);
        let reasons = grid
            .columns()
            .iter()
            .find(|c| c.key() == "hold_reason")
            .expect("hold reason column")
            .options()
            .iter()
            .map(|(value, _)| value.as_str())
            .collect::<Vec<_>>();
        assert!(reasons.contains(&"on_hold"));
        assert!(!reasons.contains(&"sanctions"));
        assert!(!reasons.contains(&"aml_concern"));
        let status = grid
            .columns()
            .iter()
            .find(|c| c.key() == "status")
            .expect("status column");
        assert!(
            status
                .options()
                .iter()
                .any(|(value, label)| value == "blocked" && label == "On hold")
        );
    }
}
