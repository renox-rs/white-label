//! Marketing channels (wl-sales §3 *Campaign*, BL-SAL-28): where a campaign
//! runs (Google, Meta, a listing portal, ...), not the order *Channel*. The
//! channels are a table like the lead sources ([`crate::lead_source`]): the
//! module's twelve §3 channels ([`crate::campaign::MarketingChannel`]) are the
//! seed, stored by the migration and by [`crate::seed::install`]; a vertical
//! adds its own and deactivates the ones it does not use
//! ([`CampaignChannel::seed_vertical`]). Keys are stable, names editable. A
//! channel a campaign names is deactivated, never deleted; a deactivated
//! channel stays on old campaigns and in the reports.
//!
//! A campaign stores the channel's key in `wl_sales_campaigns.marketing_channel`
//! (the column the fixed enum used), so a campaign created before the table
//! keeps its channel: the enum's stored name is the row's key.

use renox::db::{DateTime, Executor};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::campaign::{Campaign, MarketingChannel};

/// A marketing channel, e.g. `meta` or `listing_portals`.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_sales_marketing_channels", unique(code))]
pub struct CampaignChannel {
    /// The primary key.
    pub id: i64,
    /// The key, e.g. `listing_portals`: lower case letters, digits and `_`.
    pub code: String,
    /// The English name.
    pub name_en: String,
    /// The Arabic name.
    pub name_ar: String,
    /// Whether it can be chosen for new campaigns.
    pub active: bool,
    /// Whether the module or a vertical seed stored it.
    pub seeded: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Why a channel could not be added or deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelRefused {
    /// The key is not 2 to 30 characters of `a-z`, `0-9` and `_`.
    BadCode,
    /// A channel has that key.
    CodeTaken,
    /// The name is blank.
    NameRequired,
    /// A campaign names the channel; deactivate it instead.
    InUse,
}

impl ChannelRefused {
    /// The text key of the refusal (under `wl_sales.marketing_channels.`).
    pub fn key(self) -> &'static str {
        match self {
            ChannelRefused::BadCode => "bad_code",
            ChannelRefused::CodeTaken => "code_taken",
            ChannelRefused::NameRequired => "name_required",
            ChannelRefused::InUse => "in_use_refused",
        }
    }

    /// The English explanation.
    pub fn message(self) -> String {
        crate::texts::english(&format!("wl_sales.marketing_channels.{}", self.key()), &[])
    }
}

/// One channel a vertical seeds: key, English name, Arabic name.
pub type ChannelSeed<'a> = (&'a str, &'a str, &'a str);

/// What [`CampaignChannel::seed_vertical`] changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VerticalSeeded {
    /// Channels created.
    pub added: usize,
    /// Seeded channels the vertical does not use, switched off.
    pub deactivated: usize,
}

impl MarketingChannel {
    /// The module's English name of the channel (the seed's name).
    pub fn english_name(self) -> String {
        crate::texts::english(
            &format!("wl_sales.campaigns.channel.{}", self.as_str()),
            &[],
        )
    }
}

impl CampaignChannel {
    /// The channel with key `code`, if any.
    pub async fn by_code<'c>(db: impl Executor<'c>, code: &str) -> Result<Option<CampaignChannel>> {
        CampaignChannel::query()
            .where_(CampaignChannel::CODE.eq(code))
            .first(db)
            .await
    }

    /// Every channel, by key.
    pub async fn ordered(db: &Db) -> Result<Vec<CampaignChannel>> {
        CampaignChannel::query()
            .order_by(CampaignChannel::CODE)
            .get(db)
            .await
    }

    /// The channels a campaign form offers: the active ones, plus `keep` (the
    /// campaign's own channel) when it was deactivated since. As (key, name).
    pub async fn offered(
        db: &Db,
        locale: &str,
        keep: Option<&str>,
    ) -> Result<Vec<(String, String)>> {
        Ok(Self::ordered(db)
            .await?
            .into_iter()
            .filter(|c| c.active || Some(c.code.as_str()) == keep)
            .map(|c| (c.code.clone(), c.name(locale).to_owned()))
            .collect())
    }

    /// Every channel as (key, name), active or not: for filters and reports,
    /// where a deactivated channel still shows.
    pub async fn options(db: &Db, locale: &str) -> Result<Vec<(String, String)>> {
        Ok(Self::ordered(db)
            .await?
            .into_iter()
            .map(|c| (c.code.clone(), c.name(locale).to_owned()))
            .collect())
    }

    /// The name of the channel `code` in `locale`; a key with no row (it
    /// should not happen) is shown as the module's name, else as the key.
    pub async fn label(db: &Db, code: &str, locale: &str) -> Result<String> {
        if let Some(channel) = Self::by_code(db, code).await? {
            return Ok(channel.name(locale).to_owned());
        }
        Ok(code
            .parse::<MarketingChannel>()
            .map_or_else(|_| code.to_owned(), MarketingChannel::english_name))
    }

    /// The name in `locale`: the Arabic one for `ar` when it is set.
    pub fn name(&self, locale: &str) -> &str {
        if locale == "ar" && !self.name_ar.trim().is_empty() {
            &self.name_ar
        } else {
            &self.name_en
        }
    }

    /// Normalises a typed key: trimmed and lower case.
    pub fn normalize_code(code: &str) -> String {
        code.trim().to_lowercase()
    }

    /// Whether `code` (already normalised) is a valid key.
    pub fn is_valid_code(code: &str) -> bool {
        crate::lead_source::LeadSource::is_valid_code(code)
    }

    /// Adds a channel. The key is normalised; a bad or taken key and a blank
    /// name are refused.
    pub async fn add(
        db: &Db,
        code: &str,
        name_en: &str,
        name_ar: &str,
    ) -> Result<std::result::Result<CampaignChannel, ChannelRefused>> {
        let code = Self::normalize_code(code);
        if !Self::is_valid_code(&code) {
            return Ok(Err(ChannelRefused::BadCode));
        }
        if name_en.trim().is_empty() {
            return Ok(Err(ChannelRefused::NameRequired));
        }
        if Self::by_code(db, &code).await?.is_some() {
            return Ok(Err(ChannelRefused::CodeTaken));
        }
        let channel = CampaignChannel::create(
            db,
            CampaignChannel {
                code,
                name_en: name_en.trim().to_owned(),
                name_ar: name_ar.trim().to_owned(),
                active: true,
                ..CampaignChannel::default()
            },
        )
        .await?;
        Ok(Ok(channel))
    }

    /// Whether a campaign names this channel.
    pub async fn in_use(&self, db: &Db) -> Result<bool> {
        let used = Campaign::query()
            .where_(Campaign::MARKETING_CHANNEL.eq(self.code.clone()))
            .count(db)
            .await?;
        Ok(used > 0)
    }

    /// Deletes the channel unless a campaign names it ([`ChannelRefused::InUse`]).
    pub async fn delete_unused(
        mut self,
        db: &Db,
    ) -> Result<std::result::Result<(), ChannelRefused>> {
        if self.in_use(db).await? {
            return Ok(Err(ChannelRefused::InUse));
        }
        self.delete(db).await?;
        Ok(Ok(()))
    }

    /// Stores a vertical's channels (BL-SAL-28): each of `channels` the table
    /// lacks is added as seeded and active; one that is there is left as the
    /// client keeps it (names and switch). With `deactivate_others`, every
    /// other seeded channel still active is switched off (never deleted), so
    /// old campaigns keep it. A re-run changes nothing. wl-sales holds no
    /// vertical's list: the vertical's seed calls this with its own.
    pub async fn seed_vertical(
        db: &Db,
        channels: &[ChannelSeed<'_>],
        deactivate_others: bool,
    ) -> Result<VerticalSeeded> {
        let mut done = VerticalSeeded::default();
        let mut tx = db.begin().await?;
        for (code, name_en, name_ar) in channels {
            if Self::by_code(&mut tx, code).await?.is_none() {
                CampaignChannel::create(
                    &mut tx,
                    CampaignChannel {
                        code: (*code).to_owned(),
                        name_en: (*name_en).to_owned(),
                        name_ar: (*name_ar).to_owned(),
                        active: true,
                        seeded: true,
                        ..CampaignChannel::default()
                    },
                )
                .await?;
                done.added += 1;
            }
        }
        if deactivate_others {
            let all = CampaignChannel::query()
                .order_by(CampaignChannel::CODE)
                .get(&mut tx)
                .await?;
            for mut channel in all {
                let used = channels.iter().any(|(code, _, _)| *code == channel.code);
                if channel.seeded && channel.active && !used {
                    channel.active = false;
                    channel.save(&mut tx).await?;
                    done.deactivated += 1;
                }
            }
        }
        tx.commit().await?;
        Ok(done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_module_seed_names_come_from_the_texts() {
        assert_eq!(MarketingChannel::Meta.english_name(), "Meta");
        assert_eq!(MarketingChannel::OtherOnline.english_name(), "Other online");
        assert_eq!(MarketingChannel::ALL.len(), 12);
    }

    #[test]
    fn keys_follow_the_lead_source_rules() {
        assert_eq!(
            CampaignChannel::normalize_code(" Listing_Portals "),
            "listing_portals"
        );
        assert!(CampaignChannel::is_valid_code("tender_invitations"));
        assert!(!CampaignChannel::is_valid_code("listing portals"));
    }

    #[test]
    fn a_refusal_has_a_message() {
        assert!(!ChannelRefused::InUse.message().is_empty());
    }
}
