//! The campaign (wl-sales §3, §4.6, #138 A6, A10): a marketing action with a
//! period and a budget, or a message to a segment. Its status moves only
//! through the machine ([`crate::machine`]); [`Campaign::store`] saves the
//! other fields after normalising and checking the code and the dates, and
//! [`delete`] removes a draft that nothing refers to.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Executor, Transaction, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::sales::CampaignId;

use crate::promo_code::{PromoCode, PromoRefused};

/// Why a campaign exists.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Purpose {
    /// Wins new customers; leads and acquisitions are attributed to it.
    #[default]
    Acquisition,
    /// A message to a segment of existing customers.
    Retention,
}

/// The module's marketing channels (§3; not the order channel). Since
/// BL-SAL-28 the channels are a table ([`crate::marketing_channel`]): this list
/// is its seed, and its stored names are the seeded rows' keys. A campaign
/// holds the key (`MarketingChannel::Meta.into()` is `"meta"`).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MarketingChannel {
    /// Google.
    #[default]
    Google,
    /// Meta (Facebook, Instagram).
    Meta,
    /// TikTok.
    Tiktok,
    /// Another online channel.
    OtherOnline,
    /// Print.
    Print,
    /// Radio or outdoor.
    RadioOutdoor,
    /// An event.
    Event,
    /// E-mail.
    Email,
    /// SMS.
    Sms,
    /// WhatsApp.
    Whatsapp,
    /// A partner.
    Partner,
    /// Anything else.
    Other,
}

impl From<MarketingChannel> for String {
    fn from(channel: MarketingChannel) -> Self {
        channel.as_str().to_owned()
    }
}

/// Where a campaign stands (§4.6).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CampaignStatus {
    /// Being prepared.
    #[default]
    Draft,
    /// Running.
    Active,
    /// Past its end date or stopped; it can be reopened.
    Ended,
}

/// A campaign.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_campaigns",
    state_machine,
    unique(code),
    index(status, start_date)
)]
pub struct Campaign {
    /// The primary key.
    pub id: i64,
    /// The name.
    pub name: String,
    /// The code: trimmed, upper case, 2 to 20 of `A-Z`, `0-9`, `-`, `_`.
    pub code: String,
    /// `acquisition` or `retention`; needed to activate.
    pub purpose: Option<Purpose>,
    /// Where it runs, the key of a [`crate::marketing_channel::CampaignChannel`];
    /// needed to activate.
    pub marketing_channel: Option<String>,
    /// The lead source it implies, if any.
    #[model(references = "wl_sales_lead_sources")]
    pub lead_source_id: Option<i64>,
    /// The brand (one brand in F1).
    pub brand: Option<String>,
    /// The legal entity.
    pub legal_entity_id: Option<i64>,
    /// The branch scope's type; empty for global.
    pub scope_type: Option<String>,
    /// The branch scope's id; empty for global.
    pub scope_id: Option<String>,
    /// The staff member told of costs, approvals and the end; needed to activate.
    pub owner_id: Option<i64>,
    /// The first day; needed to activate.
    pub start_date: Option<NaiveDate>,
    /// The last day.
    pub end_date: Option<NaiveDate>,
    /// The budget in fils of AED; empty means no over-budget check.
    pub budget: Option<i64>,
    /// The linked promo code (normalised like the code).
    pub promo_code: Option<String>,
    /// Retention: the segment.
    pub segment_id: Option<i64>,
    /// Retention: the notification template key.
    pub template_key: Option<String>,
    /// Retention: the message channel.
    pub message_channel: Option<String>,
    /// Retention: when the message goes out.
    pub send_at: Option<DateTime>,
    /// Retention: who set the retention fields.
    pub retention_set_by: Option<i64>,
    /// Bumped to take the campaign's lock (#138 A12).
    pub lock_version: i64,
    /// Where it stands; changed only by the machine.
    pub status: CampaignStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The code of a deleted campaign, kept so nobody takes it again.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_sales_campaign_codes", unique(code))]
pub struct CampaignCode {
    /// The primary key.
    pub id: i64,
    /// The code.
    pub code: String,
    /// The deleted campaign.
    pub campaign_id: i64,
    /// When it was deleted.
    pub deleted_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Why a campaign could not be saved or deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CampaignRefused {
    /// The name is blank.
    NameRequired,
    /// The code is not 2 to 20 of `A-Z`, `0-9`, `-`, `_`.
    BadCode,
    /// A campaign has or had the code, in any state.
    CodeTaken,
    /// The promo code is badly formed.
    BadPromoCode,
    /// Another campaign has the promo code.
    PromoCodeTaken,
    /// The end date is before the start date.
    EndBeforeStart,
    /// Only a draft can be deleted.
    NotDraft,
    /// Something refers to the campaign; the text says what.
    InUse(String),
    /// There is no such campaign.
    NotFound,
}

impl CampaignRefused {
    /// The English explanation.
    pub fn message(&self) -> String {
        let key = match self {
            CampaignRefused::NameRequired => "wl_sales.campaigns.name_required",
            CampaignRefused::BadCode => "wl_sales.campaigns.bad_code",
            CampaignRefused::CodeTaken => "wl_sales.campaigns.code_taken",
            CampaignRefused::BadPromoCode => "wl_sales.campaigns.bad_promo_code",
            CampaignRefused::PromoCodeTaken => "wl_sales.campaigns.promo_code_taken",
            CampaignRefused::EndBeforeStart => "wl_sales.campaigns.end_before_start",
            CampaignRefused::NotDraft => "wl_sales.campaigns.not_draft",
            CampaignRefused::NotFound => "wl_sales.campaigns.not_found",
            CampaignRefused::InUse(text) => return text.clone(),
        };
        crate::texts::english(key, &[])
    }

    /// The refusal as an HTTP 422.
    pub fn into_error(self) -> Error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, self.message())
    }
}

impl From<PromoRefused> for CampaignRefused {
    fn from(refused: PromoRefused) -> Self {
        match refused {
            PromoRefused::BadCode => CampaignRefused::BadPromoCode,
            PromoRefused::CodeTaken => CampaignRefused::PromoCodeTaken,
        }
    }
}

/// Normalises a typed code: trimmed and upper case.
pub fn normalize_code(code: &str) -> String {
    code.trim().to_uppercase()
}

/// Whether `code` (already normalised) is a valid code.
pub fn is_valid_code(code: &str) -> bool {
    (2..=20).contains(&code.len())
        && code
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

impl Campaign {
    /// The campaign as an id.
    pub fn campaign_id(&self) -> CampaignId {
        CampaignId(self.id)
    }

    /// The branch scope as a subject, if the campaign has one.
    pub fn branch(&self) -> Option<Subject> {
        match (&self.scope_type, &self.scope_id) {
            (Some(kind), Some(id)) => id.parse().ok().map(|id| Subject::new(kind, id)),
            _ => None,
        }
    }

    /// The campaign with code `code` (normalised first), if any.
    pub async fn by_code<'c>(db: impl Executor<'c>, code: &str) -> Result<Option<Campaign>> {
        Campaign::query()
            .where_(Campaign::CODE.eq(normalize_code(code)))
            .first(db)
            .await
    }

    /// Normalises and checks the fields, then creates the campaign in
    /// `draft` (`id` 0) or updates it. The status and the lock are never
    /// taken from `self`. A linked promo code is stored with it.
    pub async fn store(
        mut self,
        db: &Db,
    ) -> Result<std::result::Result<Campaign, CampaignRefused>> {
        self.name = self.name.trim().to_owned();
        self.code = normalize_code(&self.code);
        self.promo_code = self
            .promo_code
            .as_deref()
            .map(normalize_code)
            .filter(|code| !code.is_empty());
        if self.name.is_empty() {
            return Ok(Err(CampaignRefused::NameRequired));
        }
        if !is_valid_code(&self.code) {
            return Ok(Err(CampaignRefused::BadCode));
        }
        if let (Some(start), Some(end)) = (self.start_date, self.end_date)
            && end < start
        {
            return Ok(Err(CampaignRefused::EndBeforeStart));
        }
        let mut tx = db.begin().await?;
        if Self::code_taken(&mut tx, &self.code, self.id).await? {
            return Ok(Err(CampaignRefused::CodeTaken));
        }
        let saved = if self.id == 0 {
            self.status = CampaignStatus::Draft;
            self.lock_version = 0;
            Campaign::create(&mut tx, self).await?
        } else {
            let Some(current) = Campaign::find(&mut tx, self.id).await? else {
                return Ok(Err(CampaignRefused::NotFound));
            };
            self.status = current.status;
            self.lock_version = current.lock_version;
            self.created_at = current.created_at;
            self.save(&mut tx).await?;
            self
        };
        if let Err(refused) = PromoCode::sync(&mut tx, &saved).await? {
            return Ok(Err(refused.into()));
        }
        tx.commit().await?;
        Ok(Ok(saved))
    }

    /// Whether `code` belongs to a campaign other than `except`, or to a
    /// deleted one.
    async fn code_taken(tx: &mut Transaction, code: &str, except: i64) -> Result<bool> {
        let live = Campaign::query()
            .where_(Campaign::CODE.eq(code))
            .where_(Campaign::ID.ne(except))
            .count(&mut *tx)
            .await?;
        let kept = CampaignCode::query()
            .where_(CampaignCode::CODE.eq(code))
            .count(&mut *tx)
            .await?;
        Ok(live + kept > 0)
    }
}

/// One thing that can stop a campaign being deleted: it returns the refusal
/// text when the campaign (by id) is still referred to. the cost entries are checked
/// by [`crate::costs::delete_check`] and #139 the leads and acquisitions.
pub type DeleteCheck = for<'a> fn(
    &'a mut Transaction,
    i64,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Option<String>>> + Send + 'a>,
>;

/// The checks `delete` runs, in order.
pub const DELETE_CHECKS: &[DeleteCheck] =
    &[crate::costs::delete_check, crate::acquisition::delete_check];

/// Deletes the draft campaign `id` when no check in [`DELETE_CHECKS`] objects.
/// Its code is kept in `wl_sales_campaign_codes` and its promo code goes.
pub async fn delete(db: &Db, id: i64) -> Result<std::result::Result<(), CampaignRefused>> {
    delete_with(db, id, DELETE_CHECKS).await
}

/// [`delete`] with an explicit check list.
pub async fn delete_with(
    db: &Db,
    id: i64,
    checks: &[DeleteCheck],
) -> Result<std::result::Result<(), CampaignRefused>> {
    let mut tx = db.begin().await?;
    let Some(mut campaign) = Campaign::find(&mut tx, id).await? else {
        return Ok(Err(CampaignRefused::NotFound));
    };
    if campaign.status != CampaignStatus::Draft {
        return Ok(Err(CampaignRefused::NotDraft));
    }
    for check in checks {
        if let Some(text) = check(&mut tx, id).await? {
            return Ok(Err(CampaignRefused::InUse(text)));
        }
    }
    CampaignCode::create(
        &mut tx,
        CampaignCode {
            code: campaign.code.clone(),
            campaign_id: id,
            deleted_at: now(),
            ..CampaignCode::default()
        },
    )
    .await?;
    PromoCode::remove_of(&mut tx, id).await?;
    campaign.delete(&mut tx).await?;
    tx.commit().await?;
    Ok(Ok(()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_trimmed_upper_case() {
        assert_eq!(normalize_code("  spring26 "), "SPRING26");
        assert!(is_valid_code("SPRING-26_A"));
        assert!(!is_valid_code("S"));
        assert!(!is_valid_code("SPRING 26"));
        assert!(!is_valid_code("spring"));
        assert!(!is_valid_code(&"A".repeat(21)));
        assert!(is_valid_code(&"A".repeat(20)));
    }

    #[test]
    fn a_new_campaign_is_a_draft_without_purpose_or_channel() {
        let c = Campaign::default();
        assert_eq!(c.status, CampaignStatus::Draft);
        assert!(c.purpose.is_none() && c.marketing_channel.is_none());
    }

    #[test]
    fn the_branch_is_a_subject_only_when_both_parts_are_set() {
        let mut c = Campaign::default();
        assert!(c.branch().is_none());
        c.scope_type = Some("property.site".into());
        c.scope_id = Some("4".into());
        assert_eq!(c.branch(), Some(Subject::new("property.site", 4)));
    }

    #[test]
    fn a_refusal_has_a_message() {
        assert!(!CampaignRefused::CodeTaken.message().is_empty());
        assert_eq!(CampaignRefused::InUse("x".into()).message(), "x");
    }
}
