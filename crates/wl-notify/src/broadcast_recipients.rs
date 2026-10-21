//! Who a broadcast reaches (wl-notify §4.3, #61 A7, A8).
//!
//! [`resolve`] is the one function the estimate and `start` both use, so the
//! two always agree. For each member of the segment, the first failing check
//! names the reason, in this order:
//!
//! 1. the party is not `active` (blocked, archived, merged): `blocked_party`;
//! 2. a minor (a party with guardians) is replaced by each guardian, whose own
//!    status, contact point, suppression and consent are checked; none passes:
//!    `no_consent`;
//! 3. no valid contact point of the channel: `no_contact_point`; with two
//!    points the primary is used, so a party gets one message;
//! 4. the address is on the suppression list: `opted_out` for an `opt_out`
//!    row, else `suppressed`;
//! 5. no marketing consent for the channel: `no_consent`.
//!
//! A party reached both directly and as a guardian is counted once. The
//! counts hold no names. The frequency cap is not part of it.
//!
//! [`lacks_consent`] is the `send` guard of #61 A8 for any marketing message.

use std::collections::HashSet;

use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::consents::{self, Consents, MARKETING};
use wl_kernel::notify::{Category, NotifyChannel};
use wl_kernel::parties::{self, PartyDirectory, PartyStatus};
use wl_kernel::segments;

use crate::broadcast::{Broadcast, ExcludedCounts};
use crate::message::{Message, NO_CONTACT_POINT, SUPPRESSED_ADDRESS};
use crate::suppressions::{self, SuppressionReason};

/// Left out because the party is not active.
pub const BLOCKED_PARTY: &str = "blocked_party";
/// Left out because there is no marketing consent.
pub const NO_CONSENT: &str = "no_consent";
/// Left out because the address opted out.
pub const OPTED_OUT: &str = "opted_out";

/// One party a broadcast message goes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipient {
    /// The party the message is addressed to (the guardian for a minor).
    pub party: PartyId,
    /// The segment member it stands for when it is not that member.
    pub on_behalf_of: Option<PartyId>,
    /// The address, as the suppression list reads it.
    pub address: String,
    /// The contact point the address comes from.
    pub contact_point: i64,
    /// The language to write in.
    pub language: String,
}

/// The result of resolving a segment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolution {
    /// Distinct segment members.
    pub members: i64,
    /// Who gets a message, in segment order.
    pub recipients: Vec<Recipient>,
    /// Members left out, by reason.
    pub excluded: ExcludedCounts,
}

/// Resolves the segment of `broadcast` at `at` with the providers of `state`.
pub async fn resolve(state: &AppState, broadcast: &Broadcast, at: DateTime) -> Result<Resolution> {
    let members = segments::segments(state)
        .members_in(state, broadcast.segment(), at)
        .await?;
    resolve_members(
        &state.db,
        parties::directory(state).as_ref(),
        consents::consents(state).as_ref(),
        &members,
        broadcast.channel,
    )
    .await
}

/// As [`resolve`], over the given `members` and providers.
pub async fn resolve_members(
    db: &Db,
    directory: &dyn PartyDirectory,
    consents: &dyn Consents,
    members: &[PartyId],
    channel: NotifyChannel,
) -> Result<Resolution> {
    let mut resolution = Resolution::default();
    let mut seen_members = HashSet::new();
    let mut reached = HashSet::new();
    for &member in members {
        if !seen_members.insert(member) {
            continue;
        }
        resolution.members += 1;
        match member_recipients(db, directory, consents, member, channel).await? {
            Ok(found) => {
                for recipient in found {
                    if reached.insert(recipient.party) {
                        resolution.recipients.push(recipient);
                    }
                }
            }
            Err(reason) => resolution.excluded.add(reason),
        }
    }
    Ok(resolution)
}

/// The recipients of one member, or the reason it is left out.
async fn member_recipients(
    db: &Db,
    directory: &dyn PartyDirectory,
    consents: &dyn Consents,
    member: PartyId,
    channel: NotifyChannel,
) -> Result<std::result::Result<Vec<Recipient>, &'static str>> {
    if !is_active(db, directory, member).await? {
        return Ok(Err(BLOCKED_PARTY));
    }
    let guardians = directory.guardians(db, member).await?;
    if guardians.is_empty() {
        return Ok(reach(db, directory, consents, member, channel)
            .await?
            .map(|r| vec![r]));
    }
    let mut found = Vec::new();
    for guardian in guardians {
        if !is_active(db, directory, guardian).await? {
            continue;
        }
        if let Ok(mut recipient) = reach(db, directory, consents, guardian, channel).await? {
            recipient.on_behalf_of = Some(member);
            found.push(recipient);
        }
    }
    Ok(if found.is_empty() {
        Err(NO_CONSENT)
    } else {
        Ok(found)
    })
}

async fn is_active(db: &Db, directory: &dyn PartyDirectory, party: PartyId) -> Result<bool> {
    Ok(directory.status(db, party).await? == Some(PartyStatus::Active))
}

/// Contact point, suppression and consent of one party.
async fn reach(
    db: &Db,
    directory: &dyn PartyDirectory,
    consents: &dyn Consents,
    party: PartyId,
    channel: NotifyChannel,
) -> Result<std::result::Result<Recipient, &'static str>> {
    let contacts = directory.contacts(db, party).await?;
    let Some((contacts, point)) = contacts.as_ref().and_then(|c| {
        c.point_for(channel)
            .filter(|p| !p.address.trim().is_empty())
            .map(|p| (c, p))
    }) else {
        return Ok(Err(NO_CONTACT_POINT));
    };
    let address = suppressions::normalize(channel, &point.address);
    match suppressions::reason_for(db, channel, &address).await? {
        Some(SuppressionReason::OptOut) => return Ok(Err(OPTED_OUT)),
        Some(_) => return Ok(Err(SUPPRESSED_ADDRESS)),
        None => {}
    }
    if !consents.has_consent(db, party, MARKETING, channel).await? {
        return Ok(Err(NO_CONSENT));
    }
    let language = contacts.language.trim();
    Ok(Ok(Recipient {
        party,
        on_behalf_of: None,
        address,
        contact_point: point.id,
        language: if language.is_empty() { "en" } else { language }.to_owned(),
    }))
}

/// The `send` consent guard (`NTF.ACL.01`, #61 A8): whether `message` is a
/// marketing message to a party that has no marketing consent for its channel
/// now. Other categories and non-party recipients pass.
pub async fn lacks_consent(state: &AppState, message: &Message) -> Result<bool> {
    let Some(party) = message.party_id else {
        return Ok(false);
    };
    let def = crate::notifier::type_def(&state.db, &message.type_key).await?;
    if def.category != Category::Marketing {
        return Ok(false);
    }
    let consent = consents::consents(state)
        .has_consent(&state.db, PartyId(party), MARKETING, message.channel)
        .await?;
    Ok(!consent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reasons_are_stable() {
        assert_eq!(BLOCKED_PARTY, "blocked_party");
        assert_eq!(NO_CONSENT, "no_consent");
        assert_eq!(OPTED_OUT, "opted_out");
    }

    #[test]
    fn a_resolution_starts_empty() {
        let resolution = Resolution::default();
        assert_eq!(resolution.members, 0);
        assert!(resolution.recipients.is_empty());
        assert_eq!(resolution.excluded.total(), 0);
    }
}
