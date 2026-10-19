//! The suppression list (wl-notify §3 *Suppression*): addresses no message
//! goes to. A message to an address on it is `suppressed`, not sent.
//!
//! A hard bounce or an invalid number adds the address ([`add`], from the
//! `fail` of #55 part 5); opt-outs and complaints come later.

use renox::db::Executor;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::notify::NotifyChannel;

/// Why an address is on the list.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SuppressionReason {
    /// The person opted out.
    OptOut,
    /// The mailbox does not exist.
    #[default]
    HardBounce,
    /// The person marked a message as spam.
    Complaint,
    /// The number or address is invalid.
    Invalid,
}

/// One blocked address on one channel.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_notify_suppressions", unique(channel, address))]
pub struct Suppression {
    /// The primary key.
    pub id: i64,
    /// The address, as [`normalize`] gives it.
    pub address: String,
    /// The channel.
    pub channel: NotifyChannel,
    /// Why.
    pub reason: SuppressionReason,
    /// What put it there, e.g. `message:42`.
    pub source: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// `address` as the list stores it: trimmed, and lowercased for e-mail.
pub fn normalize(channel: NotifyChannel, address: &str) -> String {
    let address = address.trim();
    match channel {
        NotifyChannel::Email => address.to_lowercase(),
        NotifyChannel::Sms | NotifyChannel::Whatsapp => address.to_owned(),
    }
}

/// Whether `address` is on the list for `channel`.
pub async fn is_suppressed<'c>(
    db: impl Executor<'c>,
    channel: NotifyChannel,
    address: &str,
) -> Result<bool> {
    Suppression::query()
        .where_(Suppression::CHANNEL.eq(channel))
        .where_(Suppression::ADDRESS.eq(normalize(channel, address)))
        .exists(db)
        .await
}

/// Why `address` is on the list for `channel`, if it is.
pub async fn reason_for<'c>(
    db: impl Executor<'c>,
    channel: NotifyChannel,
    address: &str,
) -> Result<Option<SuppressionReason>> {
    Ok(Suppression::query()
        .where_(Suppression::CHANNEL.eq(channel))
        .where_(Suppression::ADDRESS.eq(normalize(channel, address)))
        .first(db)
        .await?
        .map(|row| row.reason))
}

/// Puts `address` on the list for `channel`, unless it is there already.
/// Returns whether it was added.
pub async fn add(
    tx: &mut renox::db::Transaction,
    channel: NotifyChannel,
    address: &str,
    reason: SuppressionReason,
    source: Option<&str>,
) -> Result<bool> {
    if is_suppressed(&mut *tx, channel, address).await? {
        return Ok(false);
    }
    Suppression::create(
        &mut *tx,
        Suppression {
            address: normalize(channel, address),
            channel,
            reason,
            source: source.map(str::to_owned),
            ..Default::default()
        },
    )
    .await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_addresses_are_matched_without_case() {
        assert_eq!(
            normalize(NotifyChannel::Email, "  Ali@Example.COM "),
            "ali@example.com"
        );
        assert_eq!(
            normalize(NotifyChannel::Sms, " +971501234567 "),
            "+971501234567"
        );
    }

    #[test]
    fn reasons_are_stored_in_snake_case() {
        let names: Vec<_> = SuppressionReason::ALL.iter().map(|r| r.as_str()).collect();
        assert_eq!(names, ["opt_out", "hard_bounce", "complaint", "invalid"]);
    }
}
