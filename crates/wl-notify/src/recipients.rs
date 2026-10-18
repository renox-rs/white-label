//! Who gets a message, on which channel, at which address and in which
//! language (#55 A5).
//!
//! The type's channels are tried in order. A channel is kept when it is
//! open (it has a provider that can send now), the recipient has an
//! address for it and the address is not on the suppression list:
//!
//! - a party: the primary contact point of the channel's kind, else the
//!   first (through the kernel's `PartyDirectory::contacts`);
//! - a user: the account e-mail, on the e-mail channel only;
//! - an address: as given, on its own channel only.
//!
//! The first kept channel gets the message. With none, the message is
//! suppressed: `suppressed` when an address was there but on the list,
//! else `no_contact_point`. The language is the party's, else the client's
//! first locale, else `en`.
//!
//! A type that announces a pause (`block_effects::announces_pause`) to a
//! party that is blocked is suppressed with the reason `party_on_hold`,
//! whatever the channels (#36 A11).

use std::sync::Arc;

use renox::prelude::*;
use wl_kernel::notify::ContactPurpose;
use wl_kernel::notify::{NotificationTypeDef, NotifyChannel, NotifyRecipient, PartyContacts};
use wl_kernel::parties::{RelatedContact, choose_contact};
use wl_kernel::{ClientConfig, PartyId};

use crate::message::{NO_CONTACT, NO_CONTACT_POINT, PARTY_ON_HOLD, SUPPRESSED_ADDRESS};
use crate::suppressions;

/// One channel a recipient could be reached on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The channel.
    pub channel: NotifyChannel,
    /// The recipient's address on it, if any.
    pub address: Option<String>,
    /// The party contact point the address comes from.
    pub contact_point: Option<i64>,
    /// Whether a provider can send on it now.
    pub open: bool,
    /// Whether the address is on the suppression list.
    pub suppressed: bool,
}

impl Candidate {
    fn usable(&self) -> bool {
        self.open && self.address.is_some() && !self.suppressed
    }
}

/// Where a message goes, or why it does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    /// The channel.
    pub channel: NotifyChannel,
    /// The address, if any.
    pub address: Option<String>,
    /// The party contact point the address comes from.
    pub contact_point: Option<i64>,
    /// `None` to send; else why it is suppressed.
    pub suppress: Option<&'static str>,
}

/// A recipient resolved for one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The person a recipient rule chose; none for any other recipient.
    pub chosen: Option<PartyId>,
    /// The language to write in.
    pub language: String,
    /// Where it goes.
    pub route: Route,
}

/// The route among `candidates` (in the type's order): the first usable
/// one; else suppressed on the first suppressed address (reason
/// `suppressed`); else on the first channel with no address
/// (`no_contact_point`). `fallback_channel` is the channel stored when
/// there is no candidate at all.
pub fn choose(candidates: &[Candidate], fallback_channel: NotifyChannel) -> Route {
    if let Some(found) = candidates.iter().find(|c| c.usable()) {
        return Route {
            channel: found.channel,
            address: found.address.clone(),
            contact_point: found.contact_point,
            suppress: None,
        };
    }
    if let Some(found) = candidates
        .iter()
        .find(|c| c.open && c.address.is_some() && c.suppressed)
    {
        return Route {
            channel: found.channel,
            address: found.address.clone(),
            contact_point: found.contact_point,
            suppress: Some(SUPPRESSED_ADDRESS),
        };
    }
    Route {
        channel: candidates.first().map_or(fallback_channel, |c| c.channel),
        address: None,
        contact_point: None,
        suppress: Some(NO_CONTACT_POINT),
    }
}

/// The address `recipient` has on `channel`, with its contact point.
fn address_on(
    recipient: &NotifyRecipient,
    channel: NotifyChannel,
    contacts: Option<&PartyContacts>,
    user_email: Option<&str>,
) -> Option<(String, Option<i64>)> {
    let present = |address: &str| !address.trim().is_empty();
    match recipient {
        NotifyRecipient::Party { .. } => contacts
            .and_then(|c| c.point_for(channel))
            .filter(|p| present(&p.address))
            .map(|p| (p.address.trim().to_owned(), Some(p.id))),
        NotifyRecipient::User { .. } => user_email
            .filter(|_| channel == NotifyChannel::Email)
            .filter(|email| present(email))
            .map(|email| (email.trim().to_owned(), None)),
        NotifyRecipient::Contact { .. } => None,
        NotifyRecipient::Address {
            channel: own,
            address,
        } => (*own == channel && present(address)).then(|| (address.trim().to_owned(), None)),
    }
}

/// The person a rule picks: the first related contact of `of` for
/// `purpose` that is not in `excluded` (the primary comes first).
pub async fn choose_person(
    state: &AppState,
    of: PartyId,
    purpose: &ContactPurpose,
    excluded: &[PartyId],
) -> Result<Option<PartyId>> {
    let found: Vec<RelatedContact> = wl_kernel::parties::directory(state)
        .related_contacts(&state.db, of, purpose)
        .await?;
    Ok(choose_contact(&found, excluded).map(|c| c.party))
}

/// The client's first locale, else `en`.
pub fn default_language(state: &AppState) -> String {
    state
        .provided::<Arc<ClientConfig>>()
        .map_or_else(|| "en".to_owned(), |c| c.default_locale().to_owned())
}

/// Resolves `recipient` for a message of `def`, over the type's channels
/// that are not in `skip` (the channels a fallback chain already used).
/// `open` lists the channels a provider can send on now.
pub async fn resolve(
    state: &AppState,
    def: &NotificationTypeDef,
    recipient: &NotifyRecipient,
    open: &[NotifyChannel],
    skip: &[NotifyChannel],
) -> Result<Resolved> {
    let mut on_hold = false;
    let mut chosen = None;
    let mut language = None;
    let mut contacts = None;
    let mut user_email = None;
    // A rule is resolved to the person it picks, who is then reached as a party.
    let person = match recipient {
        NotifyRecipient::Contact { of, purpose } => {
            chosen = choose_person(state, *of, purpose, &[]).await?;
            chosen.map(|id| NotifyRecipient::Party { id })
        }
        _ => None,
    };
    if chosen.is_none() && matches!(recipient, NotifyRecipient::Contact { .. }) {
        // Nobody is related: nothing to send, and not a give-up.
        let fallback_channel = def.channels.first().copied().unwrap_or_default();
        let channel = def
            .channels
            .iter()
            .find(|c| !skip.contains(c))
            .copied()
            .unwrap_or(fallback_channel);
        return Ok(Resolved {
            chosen: None,
            language: default_language(state),
            route: Route {
                channel,
                address: None,
                contact_point: None,
                suppress: Some(NO_CONTACT),
            },
        });
    }
    let recipient = person.as_ref().unwrap_or(recipient);
    match recipient {
        NotifyRecipient::Party { id } => {
            on_hold =
                wl_kernel::block_effects::suppressed_while_blocked(state, &def.key, *id).await?;
            contacts = wl_kernel::parties::directory(state)
                .contacts(&state.db, *id)
                .await?;
            language = contacts
                .as_ref()
                .map(|c| c.language.trim().to_owned())
                .filter(|l| !l.is_empty());
        }
        NotifyRecipient::User { id } => {
            user_email = User::find(&state.db, *id).await?.map(|u| u.email);
        }
        NotifyRecipient::Contact { .. } | NotifyRecipient::Address { .. } => {}
    }
    let mut candidates = Vec::new();
    for &channel in def.channels.iter().filter(|c| !skip.contains(c)) {
        let found = address_on(recipient, channel, contacts.as_ref(), user_email.as_deref());
        let suppressed = match &found {
            Some((address, _)) => suppressions::is_suppressed(&state.db, channel, address).await?,
            None => false,
        };
        let (address, contact_point) = found.unzip();
        candidates.push(Candidate {
            channel,
            address,
            contact_point: contact_point.flatten(),
            open: open.contains(&channel),
            suppressed,
        });
    }
    let fallback_channel = def.channels.first().copied().unwrap_or_default();
    let mut route = choose(&candidates, fallback_channel);
    if on_hold {
        route.suppress = Some(PARTY_ON_HOLD);
    }
    Ok(Resolved {
        chosen,
        language: language.unwrap_or_else(|| default_language(state)),
        route,
    })
}

#[cfg(test)]
mod tests {
    use wl_kernel::PartyId;
    use wl_kernel::notify::{ContactKind, PartyContactPoint};

    use super::*;

    fn candidate(channel: NotifyChannel, address: Option<&str>) -> Candidate {
        Candidate {
            channel,
            address: address.map(str::to_owned),
            contact_point: None,
            open: true,
            suppressed: false,
        }
    }

    #[test]
    fn the_first_usable_channel_gets_the_message() {
        let mut email = candidate(NotifyChannel::Email, Some("a@example.com"));
        let sms = candidate(NotifyChannel::Sms, Some("+971501234567"));
        let route = choose(&[email.clone(), sms.clone()], NotifyChannel::Email);
        assert_eq!(route.channel, NotifyChannel::Email);
        assert_eq!(route.suppress, None);

        email.open = false;
        let route = choose(&[email.clone(), sms.clone()], NotifyChannel::Email);
        assert_eq!(
            (route.channel, route.address.as_deref()),
            (NotifyChannel::Sms, Some("+971501234567"))
        );
    }

    #[test]
    fn a_suppressed_address_is_not_used() {
        let mut email = candidate(NotifyChannel::Email, Some("a@example.com"));
        email.suppressed = true;
        let none = candidate(NotifyChannel::Sms, None);
        let route = choose(&[email, none], NotifyChannel::Email);
        assert_eq!(route.suppress, Some(SUPPRESSED_ADDRESS));
        assert_eq!(route.address.as_deref(), Some("a@example.com"));
    }

    #[test]
    fn no_address_on_any_channel_is_no_contact_point() {
        let route = choose(
            &[
                candidate(NotifyChannel::Email, None),
                candidate(NotifyChannel::Sms, None),
            ],
            NotifyChannel::Whatsapp,
        );
        assert_eq!(route.suppress, Some(NO_CONTACT_POINT));
        assert_eq!((route.channel, route.address), (NotifyChannel::Email, None));
        let route = choose(&[], NotifyChannel::Sms);
        assert_eq!(route.channel, NotifyChannel::Sms);
    }

    #[test]
    fn a_rule_has_no_address_of_its_own() {
        let rule = NotifyRecipient::Contact {
            of: PartyId(1),
            purpose: ContactPurpose::contact_of("finance"),
        };
        assert_eq!(address_on(&rule, NotifyChannel::Email, None, None), None);
    }

    #[test]
    fn no_contact_is_its_own_reason() {
        assert_eq!(NO_CONTACT, "no_contact");
        assert_ne!(NO_CONTACT, NO_CONTACT_POINT);
    }

    #[test]
    fn each_recipient_kind_has_its_own_addresses() {
        let contacts = PartyContacts {
            party: PartyId(1),
            language: "ar".into(),
            preferred_channel: None,
            points: vec![PartyContactPoint {
                id: 9,
                kind: ContactKind::Mobile,
                address: "+971501234567".into(),
                primary: true,
                verified: false,
                valid: true,
            }],
        };
        let party = NotifyRecipient::Party { id: PartyId(1) };
        assert_eq!(
            address_on(&party, NotifyChannel::Sms, Some(&contacts), None),
            Some(("+971501234567".to_owned(), Some(9)))
        );
        assert_eq!(
            address_on(&party, NotifyChannel::Email, Some(&contacts), None),
            None
        );
        let user = NotifyRecipient::User { id: 2 };
        assert_eq!(
            address_on(&user, NotifyChannel::Email, None, Some("u@example.com")),
            Some(("u@example.com".to_owned(), None))
        );
        assert_eq!(
            address_on(&user, NotifyChannel::Sms, None, Some("u@example.com")),
            None
        );
        let address = NotifyRecipient::Address {
            channel: NotifyChannel::Sms,
            address: " +971500000000 ".into(),
        };
        assert_eq!(
            address_on(&address, NotifyChannel::Sms, None, None),
            Some(("+971500000000".to_owned(), None))
        );
        assert_eq!(address_on(&address, NotifyChannel::Email, None, None), None);
    }
}
