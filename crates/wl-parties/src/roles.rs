//! Party roles (wl-parties §3): what a party is to the business; a party
//! can have several (`tenant`, `owner`, `vendor`…, from the vertical).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// One role of a party.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_parties_roles", index(party_id, role_key))]
pub struct PartyRole {
    /// The primary key.
    pub id: i64,
    /// The party.
    pub party_id: i64,
    /// The role key from the vertical's role list, e.g. `tenant`.
    pub role_key: String,
    /// When the role started.
    pub since: Option<NaiveDate>,
    /// When the role ended.
    pub until: Option<NaiveDate>,
    /// A reference in another system, e.g. a customer code.
    pub external_reference: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}
