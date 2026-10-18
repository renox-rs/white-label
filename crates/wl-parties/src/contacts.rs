//! Contact points (wl-parties §3): the ways to reach a party. At most one
//! of each type is primary (also enforced by a unique index).

use renox::prelude::*;
use serde::{Deserialize, Serialize};

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
