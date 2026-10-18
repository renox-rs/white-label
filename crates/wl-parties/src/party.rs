//! The Party register (wl-parties §3): a person or a company the client
//! deals with. Its status moves only through the machine in
//! [`crate::machine`].

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::{PartyId, PartyKind, PartyStatus};

/// The language a party is written to in.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Language {
    /// English.
    #[default]
    En,
    /// Arabic.
    Ar,
}

/// The channel a party prefers to be reached on.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Channel {
    /// E-mail.
    #[default]
    Email,
    /// Text message.
    Sms,
    /// WhatsApp.
    Whatsapp,
}

/// A person or a company known to the business.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_parties_parties",
    state_machine,
    unique(code),
    index(status)
)]
pub struct Party {
    /// The primary key.
    pub id: i64,
    /// The code from the `party` series, e.g. `P-00001`.
    pub code: String,
    /// Person or company.
    pub kind: PartyKind,
    /// The name to show.
    pub display_name: String,
    /// The legal name in English.
    pub legal_name_en: String,
    /// The legal name in Arabic.
    pub legal_name_ar: Option<String>,
    /// The lifecycle state; changed only by a transition.
    pub status: PartyStatus,
    /// The language documents and messages use.
    pub default_language: Language,
    /// The preferred channel.
    pub preferred_channel: Channel,
    /// The display block reason while blocked: the reason code, or `on_hold`
    /// when it is restricted. Never a restricted code (`PTY.ACL.02`).
    pub hold_reason: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Party {
    /// The party's id in the kernel contract.
    pub fn party_id(&self) -> PartyId {
        PartyId(self.id)
    }

    /// The name in `lang` (`en`, `ar`): the Arabic legal name for `ar` when
    /// there is one, else the display name.
    pub fn name_in(&self, lang: &str) -> &str {
        match (lang, self.legal_name_ar.as_deref()) {
            ("ar", Some(name)) if !name.trim().is_empty() => name,
            _ => &self.display_name,
        }
    }
}
