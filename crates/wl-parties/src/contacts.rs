//! Contact points (wl-parties §3): the ways to reach a party. At most one
//! of each type is primary (also enforced by a unique index).
//!
//! A contact point is `valid` until a hard delivery failure marks it
//! `invalid` (#58, through [`crate::Directory`]); [`update_contact`] with a
//! new value and [`verify_contact`] make it `valid` again. It is never
//! deleted and keeps its primary flag.

use renox::prelude::*;
use renox::validation::{Errors, ValidationError};
use serde::{Deserialize, Serialize};

use crate::permissions;
use crate::texts;

/// The kind of a contact point.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ContactType {
    /// An e-mail address.
    #[default]
    Email,
    /// A mobile number.
    Mobile,
    /// A landline number.
    Landline,
    /// A postal address.
    Address,
}

/// Whether a contact point can be reached.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ContactStatus {
    /// Usable.
    #[default]
    Valid,
    /// A hard delivery failure marked it; messages skip it until it is
    /// edited or verified.
    Invalid,
}

/// One way to reach a party.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_parties_contact_points", index(party_id))]
pub struct ContactPoint {
    /// The primary key.
    pub id: i64,
    /// The party.
    pub party_id: i64,
    /// E-mail, mobile, landline or address.
    pub contact_type: ContactType,
    /// The address or number.
    pub value: String,
    /// A label, e.g. `work`.
    pub label: Option<String>,
    /// Whether it is the party's primary contact point of its type.
    pub is_primary: bool,
    /// When it was verified.
    pub verified_at: Option<DateTime>,
    /// Valid, or invalid after a hard delivery failure.
    pub status: ContactStatus,
    /// When it was marked invalid; `None` while valid.
    pub invalid_since: Option<DateTime>,
    /// Why it was marked invalid (`hard_bounce`, `invalid_number`,
    /// `unknown_account`); `None` while valid.
    pub invalid_reason: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The first type that has more than one primary contact point in
/// `contacts`, if any.
pub fn second_primary(contacts: &[ContactPoint]) -> Option<ContactType> {
    let mut seen = Vec::new();
    for contact in contacts.iter().filter(|c| c.is_primary) {
        if seen.contains(&contact.contact_type) {
            return Some(contact.contact_type);
        }
        seen.push(contact.contact_type);
    }
    None
}

impl ContactPoint {
    /// Makes it valid again and clears the since and reason.
    fn reset_status(&mut self) {
        self.status = ContactStatus::Valid;
        self.invalid_since = None;
        self.invalid_reason = None;
    }
}

/// Whether `value` looks like an e-mail address.
fn is_email(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !value.chars().any(char::is_whitespace)
        && !domain.contains('@')
        && domain.contains('.')
        && domain.split('.').all(|part| !part.is_empty())
}

/// Whether `value` is an E.164 number: `+`, then 8 to 15 digits, the first
/// not zero.
fn is_e164(value: &str) -> bool {
    value.strip_prefix('+').is_some_and(|digits| {
        (8..=15).contains(&digits.len())
            && digits.chars().all(|c| c.is_ascii_digit())
            && !digits.starts_with('0')
    })
}

/// The text key for what is wrong with `value` for a point of type
/// `contact_type`, if anything.
fn value_error(contact_type: ContactType, value: &str) -> Option<&'static str> {
    if value.is_empty() {
        return Some("wl_parties.errors.contact_required");
    }
    match contact_type {
        ContactType::Email if !is_email(value) => Some("wl_parties.errors.contact_email_invalid"),
        ContactType::Mobile if !is_e164(value) => Some("wl_parties.errors.contact_mobile_invalid"),
        _ => None,
    }
}

/// Loads the contact point `id`, 404 when there is none.
async fn load(state: &AppState, id: i64) -> Result<ContactPoint> {
    ContactPoint::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)
}

/// `user` changes the value and label of contact point `id`. Needs
/// `parties.update`. The value is checked as on creation (e-mail format,
/// E.164 mobile). When the value changes the point becomes `valid` again
/// (since and reason cleared); changing only the label keeps the status. The
/// primary flag is kept.
///
/// # Errors
///
/// `Forbidden` without the permission, `NotFound` for an unknown point, a
/// validation error (422) naming `value` when it is wrong.
pub async fn update_contact(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    value: &str,
    label: Option<&str>,
) -> Result<ContactPoint> {
    if !user.has_permission(permissions::UPDATE) {
        return Err(Error::Forbidden);
    }
    let mut point = load(state, id).await?;
    let value = value.trim();
    if let Some(key) = value_error(point.contact_type, value) {
        let mut errors = Errors::new();
        errors.add("value", texts::t(&state.current_lang(), key));
        return Err(Error::Validation(ValidationError::new(errors)));
    }
    if point.value != value {
        point.value = value.to_owned();
        point.reset_status();
    }
    point.label = label
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_owned);
    point.save(&state.db).await?;
    // The party is found by its contact points (BL-AGT-04).
    crate::search::index(&state.db, point.party_id).await?;
    Ok(point)
}

/// `user` verifies contact point `id`: `verified_at` is now and the point is
/// `valid` again. Needs `parties.update`. Wl-notify's suppression list is
/// not touched.
///
/// # Errors
///
/// `Forbidden` without the permission, `NotFound` for an unknown point.
pub async fn verify_contact(state: &AppState, user: &AuthUser, id: i64) -> Result<ContactPoint> {
    if !user.has_permission(permissions::UPDATE) {
        return Err(Error::Forbidden);
    }
    let mut point = load(state, id).await?;
    point.verified_at = Some(renox::db::now());
    point.reset_status();
    point.save(&state.db).await?;
    // The party is found by its contact points (BL-AGT-04).
    crate::search::index(&state.db, point.party_id).await?;
    Ok(point)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails_need_a_local_part_and_a_dotted_domain() {
        assert!(is_email("ali.r@bluedunes.ae"));
        assert!(!is_email("ali@"));
        assert!(!is_email("@bluedunes.ae"));
        assert!(!is_email("ali@bluedunes"));
        assert!(!is_email("a li@bluedunes.ae"));
        assert!(!is_email("a@b@c.ae"));
    }

    #[test]
    fn mobiles_are_e164() {
        assert!(is_e164("+971501234567"));
        assert!(!is_e164("0501234567"));
        assert!(!is_e164("+0501234567"));
        assert!(!is_e164("+97150"));
        assert!(!is_e164("+97150123456789012"));
        assert!(!is_e164("+97150abc4567"));
    }

    #[test]
    fn a_value_is_checked_by_type() {
        assert_eq!(
            value_error(ContactType::Email, ""),
            Some("wl_parties.errors.contact_required")
        );
        assert_eq!(
            value_error(ContactType::Email, "nope"),
            Some("wl_parties.errors.contact_email_invalid")
        );
        assert_eq!(
            value_error(ContactType::Mobile, "0501234567"),
            Some("wl_parties.errors.contact_mobile_invalid")
        );
        assert_eq!(value_error(ContactType::Landline, "04 333 4444"), None);
        assert_eq!(value_error(ContactType::Address, "Dubai"), None);
    }

    #[test]
    fn resetting_makes_a_point_valid_and_clears_since_and_reason() {
        let mut point = ContactPoint {
            status: ContactStatus::Invalid,
            invalid_since: Some(renox::db::now()),
            invalid_reason: Some("hard_bounce".into()),
            is_primary: true,
            ..Default::default()
        };
        point.reset_status();
        assert_eq!(point.status, ContactStatus::Valid);
        assert_eq!((point.invalid_since, point.invalid_reason), (None, None));
        assert!(point.is_primary);
    }
}
