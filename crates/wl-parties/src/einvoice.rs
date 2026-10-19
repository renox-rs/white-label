//! A company's e-invoicing recipient identifier (wl-parties §3 *Company
//! details*, issue #43): the Peppol participant ID `wl-invoicing` needs
//! before it issues an invoice to a recipient in e-invoicing scope.
//!
//! - [`set_recipient_id`] stores it trimmed and as entered (no format check
//!   until the ASP's format is confirmed) and emits
//!   [`PartyUpdated`] once per change with the field name only, never the
//!   value, so a rejected transmission can be resubmitted after the fix.
//! - [`lookup_recipient`] asks the ASP adapter of the legal entity through
//!   the kernel ([`wl_kernel::einvoice`]) and saves nothing: the user
//!   confirms the found ID by saving the form. The user's scope is not
//!   known on the party page, so the entity is the one of the global scope,
//!   the default entity.
//! - [`lookup_available`] says whether the *Look up* button is enabled: the
//!   entity's ASP connection is `up`.
//!
//! Persons do not have the field.

use renox::prelude::*;
use renox::validation::{Errors, ValidationError};
use wl_kernel::LegalEntity;
use wl_kernel::PartyKind;
use wl_kernel::einvoice::{self, Lookup};
use wl_kernel::entities::is_trn;
use wl_kernel::parties::PartyUpdated;

use crate::details::CompanyDetails;
use crate::party::Party;
use crate::permissions;
use crate::texts;

/// The name of the field in forms and in [`PartyUpdated::changed_fields`].
pub const FIELD: &str = "einvoice_recipient_id";

/// `value` trimmed; `None` when blank.
pub fn clean(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// A validation error (422) naming `field`.
fn refuse(state: &AppState, field: &str, key: &str) -> Error {
    let mut errors = Errors::new();
    errors.add(field, texts::t(&state.current_lang(), key));
    Error::Validation(ValidationError::new(errors))
}

/// The company party `id` with its details.
async fn company_of(state: &AppState, id: i64) -> Result<(Party, CompanyDetails)> {
    let party = Party::find_or_404(&state.db, id).await?;
    if party.kind != PartyKind::Company {
        return Err(refuse(state, FIELD, "wl_parties.errors.einvoice_person"));
    }
    let company = CompanyDetails::query()
        .where_(CompanyDetails::PARTY_ID.eq(party.id))
        .first(&state.db)
        .await?
        .ok_or(Error::NotFound)?;
    Ok((party, company))
}

/// Sets the recipient identifier of company `party` to `value` (trimmed;
/// blank clears it), by `user`. A change emits [`PartyUpdated`] with
/// `changed_fields = ["einvoice_recipient_id"]` after the save; saving the
/// value it already has changes and emits nothing. Needs `parties.update`.
///
/// # Errors
///
/// `Forbidden` without the permission, `NotFound` for an unknown party, a
/// validation error (422) naming `einvoice_recipient_id` for a person.
pub async fn set_recipient_id(
    state: &AppState,
    user: &AuthUser,
    party: i64,
    value: Option<&str>,
) -> Result<CompanyDetails> {
    if !user.has_permission(permissions::UPDATE) {
        return Err(Error::Forbidden);
    }
    let (party, mut company) = company_of(state, party).await?;
    let value = clean(value);
    if company.einvoice_recipient_id == value {
        return Ok(company);
    }
    company.einvoice_recipient_id = value;
    company.save(&state.db).await?;
    state
        .emit(PartyUpdated {
            party: party.party_id(),
            changed_fields: vec![FIELD.to_owned()],
        })
        .await?;
    Ok(company)
}

/// Whether the *Look up* button is enabled: the ASP connection of the
/// entity is `up`.
pub async fn lookup_available(state: &AppState) -> Result<bool> {
    let entity = LegalEntity::for_scope(&state.db, None).await?;
    Ok(einvoice::available_connection(state, &entity)
        .await?
        .is_some())
}

/// Looks up the identifier of company `party` by its TRN through the ASP
/// adapter of the legal entity, for `user` to confirm; nothing is saved.
/// Needs `parties.update`.
///
/// # Errors
///
/// `Forbidden` without the permission, `NotFound` for an unknown party, a
/// validation error (422) naming `trn` for a company without a (valid) TRN
/// and `einvoice_recipient_id` for a person.
pub async fn lookup_recipient(state: &AppState, user: &AuthUser, party: i64) -> Result<Lookup> {
    if !user.has_permission(permissions::UPDATE) {
        return Err(Error::Forbidden);
    }
    let (_party, company) = company_of(state, party).await?;
    let trn = clean(company.trn.as_deref()).filter(|trn| is_trn(trn));
    let Some(trn) = trn else {
        return Err(refuse(state, "trn", "wl_parties.errors.einvoice_lookup_trn"));
    };
    let entity = LegalEntity::for_scope(&state.db, None).await?;
    einvoice::lookup(state, &entity, &trn).await
}

#[cfg(test)]
mod tests {
    use super::clean;

    #[test]
    fn the_identifier_is_trimmed_and_a_blank_one_is_none() {
        assert_eq!(
            clean(Some("  0235:1234567890 ")).as_deref(),
            Some("0235:1234567890")
        );
        assert_eq!(clean(Some("   ")), None);
        assert_eq!(clean(None), None);
    }
}
