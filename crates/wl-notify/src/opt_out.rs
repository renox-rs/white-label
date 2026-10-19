//! Opting out of marketing (wl-notify §4.3, #61 A10).
//!
//! Every marketing message of a broadcast ends with the brand and a way out
//! ([`footer`]):
//!
//! - e-mail: a link to the public page `/unsubscribe/{token}`. The token
//!   ([`token`]) is the message id sealed with the app key (AES-GCM, so a
//!   changed token does not open: [`message_of`] answers `None`). Confirming
//!   ([`confirm`]) withdraws the e-mail marketing consent and adds an
//!   `opt_out` suppression row; doing it twice changes nothing more;
//! - SMS and WhatsApp: "Reply STOP to opt out" (the keyword stays English in
//!   Arabic messages). [`handle_reply`] is what the provider webhook calls
//!   with an incoming text: `STOP` in any letter case, with spaces around,
//!   withdraws the consent for that channel of the party with that number and
//!   adds the suppression row. The webhook itself needs the SMS gateway,
//!   which is not built, so only this function is.
//!
//! The party with a number is found from the messages sent to it (the
//! directory has no lookup by number).

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::consents::{self, MARKETING};
use wl_kernel::notify::NotifyChannel;

use crate::message::Message;
use crate::suppressions::{self, SuppressionReason};
use crate::texts;

/// The route name of the public page.
pub const ROUTE: &str = "wl_notify.unsubscribe";

/// The keyword that opts out of SMS and WhatsApp.
pub const STOP: &str = "STOP";

const PREFIX: &str = "unsubscribe:";

/// The token of the unsubscribe link of message `id`: URL safe, signed.
pub fn token(state: &AppState, id: i64) -> String {
    state
        .encrypt(&format!("{PREFIX}{id}"))
        .bytes()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The message id `token` was made for; `None` for anything else.
pub fn message_of(state: &AppState, token: &str) -> Option<i64> {
    if token.is_empty() || !token.len().is_multiple_of(2) || !token.is_ascii() {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..token.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&token[i..i + 2], 16).ok())
        .collect();
    let sealed = String::from_utf8(bytes?).ok()?;
    let plain = state.decrypt(&sealed).ok()?;
    plain.strip_prefix(PREFIX)?.parse().ok()
}

/// The unsubscribe link of message `id`, absolute.
pub fn link(state: &AppState, id: i64) -> Result<String> {
    state.absolute_url(ROUTE, &[&token(state, id)])
}

/// The closing lines of a marketing `message`: the brand and the way out for
/// its channel, in its language. The message must be stored (the link names
/// its id).
pub fn footer(state: &AppState, message: &Message) -> Result<String> {
    let brand = state.config.name.as_str();
    let arabic = message.language == "ar";
    Ok(match message.channel {
        NotifyChannel::Email => {
            let link = link(state, message.id)?;
            let key = if arabic {
                "wl_notify.optout.email_ar"
            } else {
                "wl_notify.optout.email"
            };
            texts::english_with(key, &[("brand", brand), ("link", &link)])
        }
        NotifyChannel::Sms | NotifyChannel::Whatsapp => {
            let key = if arabic {
                "wl_notify.optout.sms_ar"
            } else {
                "wl_notify.optout.sms"
            };
            texts::english_with(key, &[("brand", brand)])
        }
    })
}

/// Puts the [`footer`] under the body of the stored marketing `message`.
pub fn add_footer(state: &AppState, message: &mut Message) -> Result<()> {
    let footer = footer(state, message)?;
    message.rendered_body = format!("{}\n\n{footer}", message.rendered_body.trim_end());
    Ok(())
}

/// The brand the unsubscribe page shows.
pub fn brand(state: &AppState) -> String {
    state.config.name.clone()
}

/// Whether `body` is the opt-out keyword: `STOP` in any letter case, with
/// spaces around it.
pub fn is_stop(body: &str) -> bool {
    body.trim().eq_ignore_ascii_case(STOP)
}

/// Opts the address of `message` out of marketing on its channel: withdraws
/// the party's consent, then adds the suppression row.
async fn opt_out(
    state: &AppState,
    party: PartyId,
    channel: NotifyChannel,
    address: &str,
    source: &str,
) -> Result<()> {
    consents::consents(state)
        .withdraw(&state.db, party, MARKETING, channel, source)
        .await?;
    let mut tx = state.db.begin().await?;
    suppressions::add(
        &mut tx,
        channel,
        address,
        SuppressionReason::OptOut,
        Some(source),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Confirms the unsubscribe link of message `id`: the e-mail marketing
/// consent of its party is withdrawn and its address is suppressed as
/// `opt_out`. Idempotent. `false` when the message is not an e-mail to a
/// party (nothing changed).
pub async fn confirm(state: &AppState, id: i64) -> Result<bool> {
    let Some(message) = Message::find(&state.db, id).await? else {
        return Ok(false);
    };
    let (Some(party), Some(address)) = (message.party_id, message.address.as_deref()) else {
        return Ok(false);
    };
    if message.channel != NotifyChannel::Email {
        return Ok(false);
    }
    opt_out(
        state,
        PartyId(party),
        NotifyChannel::Email,
        address,
        "unsubscribe_link",
    )
    .await?;
    Ok(true)
}

/// What [`handle_reply`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyOutcome {
    /// The text is not the keyword: nothing done.
    Ignored,
    /// The number is opted out; this many parties had it as their contact.
    OptedOut(usize),
}

/// An incoming text `body` on `channel` from the number `from`.
///
/// `STOP` (any letter case, spaces around) withdraws the marketing consent
/// of each party that was sent a message at that number and puts the number on
/// the suppression list as `opt_out`, so a repeated reply is harmless. Any other text is ignored. A number never written to is still
/// suppressed.
pub async fn handle_reply(
    state: &AppState,
    channel: NotifyChannel,
    from: &str,
    body: &str,
) -> Result<ReplyOutcome> {
    if channel == NotifyChannel::Email || !is_stop(body) {
        return Ok(ReplyOutcome::Ignored);
    }
    let address = suppressions::normalize(channel, from);
    let sent = Message::query()
        .where_(Message::CHANNEL.eq(channel))
        .where_(Message::ADDRESS.eq(address.clone()))
        .get(&state.db)
        .await?;
    let mut parties: Vec<i64> = sent.iter().filter_map(|m| m.party_id).collect();
    parties.sort_unstable();
    parties.dedup();
    let source = format!("{}_stop", channel.as_str());
    for party in &parties {
        consents::consents(state)
            .withdraw(&state.db, PartyId(*party), MARKETING, channel, &source)
            .await?;
    }
    let mut tx = state.db.begin().await?;
    suppressions::add(
        &mut tx,
        channel,
        &address,
        SuppressionReason::OptOut,
        Some(&source),
    )
    .await?;
    tx.commit().await?;
    Ok(ReplyOutcome::OptedOut(parties.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keyword_is_stop_in_any_letter_case() {
        for body in ["STOP", "stop", "Stop", "STOP ", "  sToP\n"] {
            assert!(is_stop(body), "{body:?}");
        }
        for body in ["", "stop it", "STOPP", "please stop", "إيقاف"] {
            assert!(!is_stop(body), "{body:?}");
        }
    }

    #[test]
    fn the_footers_name_the_brand_and_the_way_out() {
        let english = texts::english_with("wl_notify.optout.sms", &[("brand", "Palm Grove")]);
        assert_eq!(english, "Palm Grove. Reply STOP to opt out");
        let arabic = texts::english_with("wl_notify.optout.sms_ar", &[("brand", "Palm Grove")]);
        assert!(arabic.contains("Palm Grove") && arabic.contains("STOP"));
        let mail = texts::english_with(
            "wl_notify.optout.email",
            &[
                ("brand", "Palm Grove"),
                ("link", "https://x.example/unsubscribe/ab"),
            ],
        );
        assert!(mail.contains("Palm Grove") && mail.ends_with("https://x.example/unsubscribe/ab"));
    }
}
