//! Creating a party (wl-parties §5.1): checks the input, takes the code from
//! the kernel's `party` series, writes the party with its details, roles and
//! contact points in one transaction, and emits [`PartyCreated`].
//!
//! The duplicate check and KYC requirements of §5.1 come with the issues
//! that need them.

use renox::prelude::*;
use renox::validation::{Errors, ValidationError};
use wl_kernel::entities::is_trn;
use wl_kernel::parties::PARTY_SERIES;
use wl_kernel::{PartyCreated, PartyKind, PartyStatus};

use crate::contacts::{ContactPoint, second_primary};
use crate::details::{CompanyDetails, PersonDetails};
use crate::party::{Channel, Language, Party};
use crate::roles::PartyRole;
use crate::texts;

/// What a new party is made of. Ids and `party_id`s in the parts are
/// ignored; the service sets them.
#[derive(Debug, Clone, Default)]
pub struct NewParty {
    /// Person or company.
    pub kind: PartyKind,
    /// The name to show; the English legal name when blank.
    pub display_name: String,
    /// The legal name in English (required).
    pub legal_name_en: String,
    /// The legal name in Arabic.
    pub legal_name_ar: Option<String>,
    /// The language documents and messages use.
    pub default_language: Language,
    /// The preferred channel.
    pub preferred_channel: Channel,
    /// A person's details (persons only).
    pub person: Option<PersonDetails>,
    /// A company's details (companies only).
    pub company: Option<CompanyDetails>,
    /// Its roles; `since` defaults to today.
    pub roles: Vec<PartyRole>,
    /// Its contact points; at most one primary per type.
    pub contacts: Vec<ContactPoint>,
    /// Where it comes from, for [`PartyCreated`]: `back_office`, `import`,
    /// `portal`, `seed`…
    pub source: String,
}

/// Creates the party in `new` as `active` and emits [`PartyCreated`] after
/// the commit. Invalid input is a validation error (422) naming the field.
pub async fn create(state: &AppState, new: NewParty) -> Result<Party> {
    check(state, &new)?;
    let today = renox::db::now().date_naive();
    // Gaps allowed: the number is taken in its own transaction, before ours
    // (SQLite has one writer).
    let code = PARTY_SERIES
        .next_allowing_gaps(&state.db, None, None, today)
        .await?;

    let legal_name_en = new.legal_name_en.trim().to_owned();
    let display_name = match new.display_name.trim() {
        "" => legal_name_en.clone(),
        name => name.to_owned(),
    };
    let mut party = Party {
        code,
        kind: new.kind,
        display_name,
        legal_name_en,
        legal_name_ar: new
            .legal_name_ar
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty()),
        status: PartyStatus::Active,
        default_language: new.default_language,
        preferred_channel: new.preferred_channel,
        ..Default::default()
    };
    let role_keys: Vec<String> = new.roles.iter().map(|r| r.role_key.clone()).collect();

    let mut tx = state.db.begin().await?;
    party.insert(&mut tx).await?;
    if let Some(mut person) = new.person {
        person.id = 0;
        person.party_id = party.id;
        person.insert(&mut tx).await?;
    }
    if let Some(mut company) = new.company {
        company.id = 0;
        company.party_id = party.id;
        company.insert(&mut tx).await?;
    }
    for mut role in new.roles {
        role.id = 0;
        role.party_id = party.id;
        role.since = role.since.or(Some(today));
        role.insert(&mut tx).await?;
    }
    for mut contact in new.contacts {
        contact.id = 0;
        contact.party_id = party.id;
        contact.value = contact.value.trim().to_owned();
        contact.insert(&mut tx).await?;
    }
    tx.commit().await?;

    state
        .emit(PartyCreated {
            party: party.party_id(),
            kind: party.kind,
            roles: role_keys,
            source: new.source,
        })
        .await?;
    Ok(party)
}

/// Refuses input that cannot make a party, naming each wrong field.
fn check(state: &AppState, new: &NewParty) -> Result<()> {
    let lang = state.current_lang();
    let mut errors = Errors::new();
    if new.legal_name_en.trim().is_empty() {
        errors.add(
            "legal_name_en",
            texts::t(&lang, "wl_parties.errors.legal_name_required"),
        );
    }
    if (new.kind == PartyKind::Person && new.company.is_some())
        || (new.kind == PartyKind::Company && new.person.is_some())
    {
        errors.add("kind", texts::t(&lang, "wl_parties.errors.details_kind"));
    }
    if let Some(trn) = new.company.as_ref().and_then(|c| c.trn.as_deref())
        && !is_trn(trn)
    {
        errors.add("trn", texts::t(&lang, "wl_parties.errors.trn_invalid"));
    }
    if new.roles.iter().any(|r| r.role_key.trim().is_empty()) {
        errors.add("roles", texts::t(&lang, "wl_parties.errors.role_required"));
    }
    if new.contacts.iter().any(|c| c.value.trim().is_empty()) {
        errors.add(
            "contacts",
            texts::t(&lang, "wl_parties.errors.contact_required"),
        );
    }
    if second_primary(&new.contacts).is_some() {
        errors.add("contacts", texts::t(&lang, "wl_parties.errors.one_primary"));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(Error::Validation(ValidationError::new(errors)))
    }
}
