//! Party acquisitions and first sales (wl-sales §3 *Party acquisition*, §5.8
//! steps 3 and 4, issue #139).
//!
//! A [`PartyAcquisition`] says how a party became a customer: its source,
//! campaign and the first sale linked to it. It lives here, not on the
//! wl-parties record, one live row per party. Every sale reported for a party
//! ([`record_sale`]) is kept as an [`AcquisitionSale`] candidate, so that when
//! the first sale is voided, cancelled or credited in full the nightly run
//! ([`crate::acquisition_daily`]) links the next one.
//!
//! - **First-touch, never overwritten:** [`attribute_party`] creates the
//!   acquisition and leaves an existing one alone, whatever a later lead or
//!   promo code says. A change goes through [`crate::acquisition_change`].
//! - **First sale:** the first sale reported for a party with an acquisition
//!   is linked at once with its value excluding VAT, net of credit notes, and
//!   `CustomerAcquired` is emitted, once per party. A sale of a party with no
//!   acquisition (an import, an API sale) creates one with the source *other*
//!   and `source_missing`, listed on the exceptions report. Nothing marks a
//!   party as created before the capability was on, so every party without an
//!   acquisition that buys is counted as acquired through *other*; a cut-off
//!   date setting comes with the host's data migration.
//! - **Converted value:** the first sale's net value, or with the
//!   *attribution window* setting the party's sales in the first N months,
//!   net of credit notes ([`converted_value`]). It is recomputed whenever a
//!   sale or credit note changes, and the reports read it, not store it.
//! - **Amounts** are AED fils excluding VAT; an invoice in another currency
//!   is not converted here (the invoicing event carries the invoice
//!   currency's amounts), which is fine while the clients invoice in AED.

use renox::chrono::{Months, NaiveDate};
use renox::db::{DateTime, Executor, Transaction, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::sales::{CampaignId, CustomerAcquired, CustomerAcquisitionChanged, FirstSale};
use wl_kernel::{PartyId, Subject};

use crate::campaign::Campaign;
use crate::config::{SalesConfig, config_of};
use crate::lead_source::LeadSource;

/// The code of the source a sale without one gets.
pub const OTHER_SOURCE: &str = "other";

/// How a party became a customer.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_party_acquisitions",
    index(party_id),
    index(source_id),
    index(campaign_id)
)]
pub struct PartyAcquisition {
    /// The primary key.
    pub id: i64,
    /// The party.
    pub party_id: i64,
    /// The lead source.
    #[model(references = "wl_sales_lead_sources")]
    pub source_id: i64,
    /// The campaign, if known.
    #[model(references = "wl_sales_campaigns")]
    pub campaign_id: Option<i64>,
    /// `lead`, `staff`, `web_form`, `promo_code` or `import`.
    pub set_from: String,
    /// The converting lead, if set from one.
    pub lead_id: Option<i64>,
    /// A first sale arrived with no source: the source is `other` and the
    /// acquisition is on the exceptions report.
    pub source_missing: bool,
    /// The type of the first sale.
    pub first_sale_type: Option<String>,
    /// The id of the first sale.
    pub first_sale_id: Option<i64>,
    /// The document date of the first sale.
    pub first_sale_date: Option<NaiveDate>,
    /// The converted value, AED fils excluding VAT, net of credit notes.
    pub first_sale_value: i64,
    /// The branch scope's type (of the first sale, else of the lead).
    pub scope_type: Option<String>,
    /// The branch scope's id.
    pub scope_id: Option<String>,
    /// The legal entity in reports.
    pub legal_entity_id: Option<i64>,
    /// When the party was acquired; it stays when the first sale is replaced.
    pub acquired_at: DateTime,
    /// `CustomerAcquired` was emitted for the party.
    pub acquired_emitted: bool,
    /// After a party merge: the acquisition that was kept.
    pub superseded_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A sale reported for a party: a candidate for its first sale.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_acquisition_sales",
    unique(subject_type, subject_id),
    index(party_id, sale_date)
)]
pub struct AcquisitionSale {
    /// The primary key.
    pub id: i64,
    /// The party that bought.
    pub party_id: i64,
    /// The type of the sale.
    pub subject_type: String,
    /// The id of the sale.
    pub subject_id: i64,
    /// Its document date.
    pub sale_date: NaiveDate,
    /// Its value excluding VAT, AED fils.
    pub value: i64,
    /// The net of the credit notes on it.
    pub credited: i64,
    /// Voided or cancelled.
    pub gone: bool,
    /// The branch scope's type.
    pub scope_type: Option<String>,
    /// The branch scope's id.
    pub scope_id: Option<String>,
    /// The legal entity.
    pub legal_entity_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl AcquisitionSale {
    /// The sale as a subject.
    pub fn subject(&self) -> Subject {
        Subject::new(&self.subject_type, self.subject_id)
    }

    /// What it is worth now: its value less the credit notes, never below 0.
    pub fn net(&self) -> i64 {
        (self.value - self.credited).max(0)
    }

    /// Whether it still stands: not voided or cancelled, and not credited in
    /// full.
    pub fn standing(&self) -> bool {
        !self.gone && !(self.value > 0 && self.credited >= self.value)
    }
}

impl PartyAcquisition {
    /// The party.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// The first sale, if one is linked.
    pub fn first_sale(&self) -> Option<Subject> {
        match (&self.first_sale_type, self.first_sale_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }

    /// The branch scope as a subject, if the acquisition has one.
    pub fn branch(&self) -> Option<Subject> {
        match (&self.scope_type, &self.scope_id) {
            (Some(kind), Some(id)) => id.parse().ok().map(|id| Subject::new(kind, id)),
            _ => None,
        }
    }

    /// The live acquisition of `party` (not superseded by a merge).
    pub async fn of_party<'c>(
        db: impl Executor<'c>,
        party: PartyId,
    ) -> Result<Option<PartyAcquisition>> {
        PartyAcquisition::query()
            .where_(PartyAcquisition::PARTY_ID.eq(party.0))
            .where_(PartyAcquisition::SUPERSEDED_BY.is_null())
            .order_by(PartyAcquisition::ID)
            .first(db)
            .await
    }
}

/// Why a source or campaign could not be recorded or a sale completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquisitionRefused {
    /// A new party's sale needs a source.
    SourceRequired,
    /// There is no such lead source, or it is switched off.
    UnknownSource,
    /// The source needs a campaign (`SAL.CRM.02`) and none was given.
    CampaignRequired,
    /// There is no such campaign.
    UnknownCampaign,
}

impl AcquisitionRefused {
    /// The English explanation.
    pub fn message(self) -> String {
        let key = match self {
            AcquisitionRefused::SourceRequired => "wl_sales.acquisitions.source_required",
            AcquisitionRefused::UnknownSource => "wl_sales.acquisitions.unknown_source",
            AcquisitionRefused::CampaignRequired => "wl_sales.acquisitions.campaign_required",
            AcquisitionRefused::UnknownCampaign => "wl_sales.acquisitions.unknown_campaign",
        };
        crate::texts::english(key, &[])
    }

    /// The refusal as an HTTP 422.
    pub fn into_error(self) -> Error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, self.message())
    }
}

/// An active lead source by code.
async fn active_source<'c>(
    db: impl Executor<'c>,
    code: &str,
) -> Result<std::result::Result<LeadSource, AcquisitionRefused>> {
    Ok(
        match LeadSource::by_code(db, &LeadSource::normalize_code(code)).await? {
            Some(source) if source.active => Ok(source),
            _ => Err(AcquisitionRefused::UnknownSource),
        },
    )
}

/// The campaign with code `code`, if any.
async fn campaign_by_code<'c>(
    db: impl Executor<'c>,
    code: Option<&str>,
) -> Result<std::result::Result<Option<Campaign>, AcquisitionRefused>> {
    let Some(code) = code.map(str::trim).filter(|c| !c.is_empty()) else {
        return Ok(Ok(None));
    };
    Ok(match Campaign::by_code(db, code).await? {
        Some(campaign) => Ok(Some(campaign)),
        None => Err(AcquisitionRefused::UnknownCampaign),
    })
}

/// Whether a sale of `party` can complete with the source and campaign the
/// cashier picked (`SAL.CRM.02`): a party with an acquisition always can; a
/// new party needs an active source, and a campaign too when the source is in
/// the settings' list (05: *paid ads*, *social*). The counter sale's
/// `complete` guard calls it, then [`attribute_party`] stores the pick.
pub async fn check_sale_source(
    state: &AppState,
    party: PartyId,
    source: Option<&str>,
    campaign: Option<&str>,
) -> Result<std::result::Result<(), AcquisitionRefused>> {
    if PartyAcquisition::of_party(&state.db, party).await?.is_some() {
        return Ok(Ok(()));
    }
    let Some(source) = source.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(Err(AcquisitionRefused::SourceRequired));
    };
    let source = match active_source(&state.db, source).await? {
        Ok(source) => source,
        Err(refused) => return Ok(Err(refused)),
    };
    let campaign = match campaign_by_code(&state.db, campaign).await? {
        Ok(campaign) => campaign,
        Err(refused) => return Ok(Err(refused)),
    };
    if campaign.is_none() && config_of(state).campaign_required_for(&source.code) {
        return Ok(Err(AcquisitionRefused::CampaignRequired));
    }
    Ok(Ok(()))
}

/// What [`attribute_party`] takes.
#[derive(Debug, Clone, Default)]
pub struct AttributeParty {
    /// The party.
    pub party: PartyId,
    /// The lead source code.
    pub source: String,
    /// The campaign code, if known.
    pub campaign: Option<String>,
    /// `lead`, `staff`, `web_form`, `promo_code` or `import`.
    pub set_from: String,
    /// The converting lead, if set from one.
    pub lead_id: Option<i64>,
    /// The branch of the lead, until a first sale gives its own.
    pub branch: Option<Subject>,
    /// The legal entity.
    pub legal_entity_id: Option<i64>,
}

/// Records how `input.party` became a customer, unless it already has an
/// acquisition, which is never overwritten. Returns the new acquisition, or
/// `None` when the party already had one.
pub async fn attribute_party(
    state: &AppState,
    input: AttributeParty,
) -> Result<std::result::Result<Option<PartyAcquisition>, AcquisitionRefused>> {
    if PartyAcquisition::of_party(&state.db, input.party)
        .await?
        .is_some()
    {
        return Ok(Ok(None));
    }
    let source = match active_source(&state.db, &input.source).await? {
        Ok(source) => source,
        Err(refused) => return Ok(Err(refused)),
    };
    let campaign = match campaign_by_code(&state.db, input.campaign.as_deref()).await? {
        Ok(campaign) => campaign,
        Err(refused) => return Ok(Err(refused)),
    };
    if campaign.is_none() && config_of(state).campaign_required_for(&source.code) {
        return Ok(Err(AcquisitionRefused::CampaignRequired));
    }
    let entity = input
        .legal_entity_id
        .or_else(|| campaign.as_ref().and_then(|c| c.legal_entity_id));
    let created = PartyAcquisition::create(
        &state.db,
        PartyAcquisition {
            party_id: input.party.0,
            source_id: source.id,
            campaign_id: campaign.as_ref().map(|c| c.id),
            set_from: input.set_from,
            lead_id: input.lead_id,
            scope_type: input.branch.as_ref().map(|b| b.subject_type.clone()),
            scope_id: input.branch.as_ref().map(|b| b.subject_id.to_string()),
            legal_entity_id: entity,
            acquired_at: now(),
            ..PartyAcquisition::default()
        },
    )
    .await?;
    // A sale reported before the acquisition existed is linked now.
    relink(state, &created).await?;
    Ok(Ok(Some(PartyAcquisition::find_or_404(&state.db, created.id).await?)))
}

/// The party acquisition of a won `lead`: it takes the lead's source and
/// campaign when the party has none yet (never overwritten). The checks of
/// [`attribute_party`] are not repeated, since the lead passed them when it
/// was captured. Returns the new acquisition, `None` when the lead has no
/// party or the party already has one.
pub async fn attribute_from_lead(
    state: &AppState,
    lead: &crate::lead::Lead,
) -> Result<Option<PartyAcquisition>> {
    let Some(party) = lead.party() else {
        return Ok(None);
    };
    if PartyAcquisition::of_party(&state.db, party).await?.is_some() {
        return Ok(None);
    }
    let created = PartyAcquisition::create(
        &state.db,
        PartyAcquisition {
            party_id: party.0,
            source_id: lead.source_id,
            campaign_id: lead.campaign_id,
            set_from: "lead".to_owned(),
            lead_id: Some(lead.id),
            scope_type: lead.scope_type.clone(),
            scope_id: lead.scope_id.clone(),
            legal_entity_id: lead.legal_entity_id,
            acquired_at: now(),
            ..PartyAcquisition::default()
        },
    )
    .await?;
    relink(state, &created).await?;
    Ok(Some(PartyAcquisition::find_or_404(&state.db, created.id).await?))
}

/// The sale candidates of `party`, earliest first (by date, then recorded).
async fn candidates<'c>(db: impl Executor<'c>, party: i64) -> Result<Vec<AcquisitionSale>> {
    AcquisitionSale::query()
        .where_(AcquisitionSale::PARTY_ID.eq(party))
        .order_by(AcquisitionSale::SALE_DATE)
        .order_by(AcquisitionSale::ID)
        .get(db)
        .await
}

/// The converted value of an acquisition whose first sale is `first`: the
/// first sale's net, or with the attribution window the net of the party's
/// standing sales dated in the first N months from the first sale's date.
pub fn converted_value(
    config: &SalesConfig,
    first: &AcquisitionSale,
    sales: &[AcquisitionSale],
) -> i64 {
    let Some(months) = config.attribution_window_months else {
        return first.net();
    };
    let Some(end) = first
        .sale_date
        .checked_add_months(Months::new(months))
    else {
        return first.net();
    };
    sales
        .iter()
        .filter(|s| s.standing() && s.sale_date >= first.sale_date && s.sale_date < end)
        .map(AcquisitionSale::net)
        .sum()
}

/// Links the party's earliest standing sale as the first sale of
/// `acquisition` when it has none, and emits `CustomerAcquired` the first time
/// (once per party). Returns the sale that was linked, if any.
async fn relink(state: &AppState, acquisition: &PartyAcquisition) -> Result<Option<Subject>> {
    if acquisition.first_sale().is_some() || acquisition.superseded_by.is_some() {
        return Ok(None);
    }
    let sales = candidates(&state.db, acquisition.party_id).await?;
    let Some(first) = sales.iter().find(|s| s.standing()) else {
        return Ok(None);
    };
    let value = converted_value(&config_of(state), first, &sales);
    let (scope_type, scope_id) = if first.scope_type.is_some() {
        (first.scope_type.clone(), first.scope_id.clone())
    } else {
        (acquisition.scope_type.clone(), acquisition.scope_id.clone())
    };
    let entity = acquisition.legal_entity_id.or(first.legal_entity_id);
    PartyAcquisition::query()
        .where_(PartyAcquisition::ID.eq(acquisition.id))
        .update(
            &state.db,
            &[
                ("first_sale_type", &Some(first.subject_type.clone())),
                ("first_sale_id", &Some(first.subject_id)),
                ("first_sale_date", &Some(first.sale_date)),
                ("first_sale_value", &value),
                ("scope_type", &scope_type),
                ("scope_id", &scope_id),
                ("legal_entity_id", &entity),
                ("updated_at", &now()),
            ],
        )
        .await?;
    if !acquisition.acquired_emitted {
        PartyAcquisition::query()
            .where_(PartyAcquisition::ID.eq(acquisition.id))
            .update(&state.db, &[("acquired_emitted", &true)])
            .await?;
        let source = LeadSource::find(&state.db, acquisition.source_id)
            .await?
            .map(|s| s.code)
            .unwrap_or_default();
        state
            .emit(CustomerAcquired {
                party: acquisition.party(),
                source,
                campaign: acquisition.campaign_id.map(CampaignId),
                first_sale: first.subject(),
                value,
            })
            .await?;
    }
    Ok(Some(first.subject()))
}

/// Reports a sale of a party (`record_first_sale` of the kernel contract):
/// the sale is kept as a candidate, and linked as the party's first sale when
/// it has none. A party with no acquisition gets one with the source *other*
/// (`source_missing`). Returns whether this sale is the party's first sale;
/// a second report of the same sale, or of a later one, keeps the first.
pub async fn record_sale(state: &AppState, sale: FirstSale) -> Result<bool> {
    let existing = AcquisitionSale::query()
        .where_(AcquisitionSale::SUBJECT_TYPE.eq(sale.subject.subject_type.clone()))
        .where_(AcquisitionSale::SUBJECT_ID.eq(sale.subject.subject_id))
        .first(&state.db)
        .await?;
    if existing.is_none() {
        AcquisitionSale::create(
            &state.db,
            AcquisitionSale {
                party_id: sale.party.0,
                subject_type: sale.subject.subject_type.clone(),
                subject_id: sale.subject.subject_id,
                sale_date: sale.date,
                value: sale.value,
                scope_type: sale.branch.as_ref().map(|b| b.subject_type.clone()),
                scope_id: sale.branch.as_ref().map(|b| b.subject_id.to_string()),
                legal_entity_id: sale.legal_entity_id,
                ..AcquisitionSale::default()
            },
        )
        .await?;
    }
    let acquisition = match PartyAcquisition::of_party(&state.db, sale.party).await? {
        Some(acquisition) => acquisition,
        None => {
            let other = LeadSource::by_code(&state.db, OTHER_SOURCE).await?;
            let Some(other) = other else {
                // No sources installed: nothing to attribute to.
                return Ok(false);
            };
            PartyAcquisition::create(
                &state.db,
                PartyAcquisition {
                    party_id: sale.party.0,
                    source_id: other.id,
                    set_from: "import".to_owned(),
                    source_missing: true,
                    scope_type: sale.branch.as_ref().map(|b| b.subject_type.clone()),
                    scope_id: sale.branch.as_ref().map(|b| b.subject_id.to_string()),
                    legal_entity_id: sale.legal_entity_id,
                    acquired_at: now(),
                    ..PartyAcquisition::default()
                },
            )
            .await?
        }
    };
    relink(state, &acquisition).await?;
    let current = PartyAcquisition::find_or_404(&state.db, acquisition.id).await?;
    Ok(current.first_sale().as_ref() == Some(&sale.subject))
}

/// Takes `net` fils (a credit note's net) off the reported sale `sale`
/// (`CreditNoteIssued`); the converted value is recomputed. A sale credited in
/// full is replaced by the nightly run.
pub async fn credit_sale(state: &AppState, sale: &Subject, net: i64) -> Result {
    let Some(row) = AcquisitionSale::query()
        .where_(AcquisitionSale::SUBJECT_TYPE.eq(sale.subject_type.clone()))
        .where_(AcquisitionSale::SUBJECT_ID.eq(sale.subject_id))
        .first(&state.db)
        .await?
    else {
        return Ok(());
    };
    AcquisitionSale::query()
        .where_(AcquisitionSale::ID.eq(row.id))
        .update(
            &state.db,
            &[
                ("credited", &(row.credited + net)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    refresh_value(state, row.party_id).await
}

/// Marks the reported sale `sale` voided or cancelled (`RecordVoided`,
/// `sale_gone`); the nightly run links the party's next sale. Does nothing for
/// a record that is not a reported sale.
pub async fn sale_gone(state: &AppState, sale: &Subject) -> Result {
    let Some(row) = AcquisitionSale::query()
        .where_(AcquisitionSale::SUBJECT_TYPE.eq(sale.subject_type.clone()))
        .where_(AcquisitionSale::SUBJECT_ID.eq(sale.subject_id))
        .first(&state.db)
        .await?
    else {
        return Ok(());
    };
    AcquisitionSale::query()
        .where_(AcquisitionSale::ID.eq(row.id))
        .update(&state.db, &[("gone", &true), ("updated_at", &now())])
        .await?;
    refresh_value(state, row.party_id).await
}

/// Recomputes and stores the converted value of the live acquisition of
/// `party`, if it has a first sale.
pub async fn refresh_value(state: &AppState, party: i64) -> Result {
    let Some(acquisition) = PartyAcquisition::of_party(&state.db, PartyId(party)).await? else {
        return Ok(());
    };
    let Some(first) = acquisition.first_sale() else {
        return Ok(());
    };
    let sales = candidates(&state.db, party).await?;
    let Some(row) = sales.iter().find(|s| s.subject() == first) else {
        return Ok(());
    };
    let value = converted_value(&config_of(state), row, &sales);
    PartyAcquisition::query()
        .where_(PartyAcquisition::ID.eq(acquisition.id))
        .update(
            &state.db,
            &[("first_sale_value", &value), ("updated_at", &now())],
        )
        .await?;
    Ok(())
}

/// What a nightly pass did to one acquisition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Replaced {
    /// The next sale was linked as the first sale.
    By(Subject),
    /// No other sale: the first sale was cleared.
    Cleared,
}

/// Replaces the first sale of `acquisition` when it no longer stands (voided,
/// cancelled or credited in full): the party's earliest standing sale becomes
/// the first sale (the acquired date stays), or with none the first sale is
/// cleared, and `CustomerAcquisitionChanged` is emitted. Returns what it did,
/// `None` when the first sale still stands.
pub async fn replace_first_sale(
    state: &AppState,
    acquisition: &PartyAcquisition,
) -> Result<Option<Replaced>> {
    let Some(old) = acquisition.first_sale() else {
        return Ok(None);
    };
    let sales = candidates(&state.db, acquisition.party_id).await?;
    let still = sales.iter().any(|s| s.subject() == old && s.standing());
    if still {
        return Ok(None);
    }
    let next = sales.iter().find(|s| s.standing() && s.subject() != old);
    let source = LeadSource::find(&state.db, acquisition.source_id)
        .await?
        .map(|s| s.code);
    let campaign = acquisition.campaign_id.map(CampaignId);
    let (replaced, reason) = if let Some(next) = next {
        let value = converted_value(&config_of(state), next, &sales);
        PartyAcquisition::query()
            .where_(PartyAcquisition::ID.eq(acquisition.id))
            .update(
                &state.db,
                &[
                    ("first_sale_type", &Some(next.subject_type.clone())),
                    ("first_sale_id", &Some(next.subject_id)),
                    ("first_sale_date", &Some(next.sale_date)),
                    ("first_sale_value", &value),
                    ("updated_at", &now()),
                ],
            )
            .await?;
        (Replaced::By(next.subject()), "first_sale_replaced")
    } else {
        PartyAcquisition::query()
            .where_(PartyAcquisition::ID.eq(acquisition.id))
            .update(
                &state.db,
                &[
                    ("first_sale_type", &None::<String>),
                    ("first_sale_id", &None::<i64>),
                    ("first_sale_date", &None::<NaiveDate>),
                    ("first_sale_value", &0_i64),
                    ("updated_at", &now()),
                ],
            )
            .await?;
        (Replaced::Cleared, "first_sale_cleared")
    };
    let new_first_sale = match &replaced {
        Replaced::By(subject) => Some(subject.clone()),
        Replaced::Cleared => None,
    };
    state
        .emit(CustomerAcquisitionChanged {
            party: acquisition.party(),
            old_source: source.clone(),
            new_source: source,
            old_campaign: campaign,
            new_campaign: campaign,
            old_first_sale: Some(old),
            new_first_sale,
            reason: reason.to_owned(),
        })
        .await?;
    Ok(Some(replaced))
}

/// Deletes the acquisition set from lead `lead` when the lead's win is
/// reverted, unless the party has another standing sale (`revert_win`).
/// Emits `CustomerAcquisitionChanged` when it deleted it. Returns whether it
/// did.
pub async fn clear_for_reverted_lead(
    state: &AppState,
    lead: i64,
    voided: &Subject,
) -> Result<bool> {
    let Some(acquisition) = PartyAcquisition::query()
        .where_(PartyAcquisition::LEAD_ID.eq(lead))
        .where_(PartyAcquisition::SUPERSEDED_BY.is_null())
        .first(&state.db)
        .await?
    else {
        return Ok(false);
    };
    let sales = candidates(&state.db, acquisition.party_id).await?;
    if sales
        .iter()
        .any(|s| s.standing() && &s.subject() != voided)
    {
        return Ok(false);
    }
    let source = LeadSource::find(&state.db, acquisition.source_id)
        .await?
        .map(|s| s.code);
    let event = CustomerAcquisitionChanged {
        party: acquisition.party(),
        old_source: source,
        new_source: None,
        old_campaign: acquisition.campaign_id.map(CampaignId),
        new_campaign: None,
        old_first_sale: acquisition.first_sale(),
        new_first_sale: None,
        reason: "lead_win_reverted".to_owned(),
    };
    let mut gone = acquisition;
    gone.delete(&state.db).await?;
    state.emit(event).await?;
    Ok(true)
}

/// Folds the acquisition of `merged` into `survivor` (`PartiesMerged`): the
/// survivor keeps the acquisition acquired first, the other stays as history
/// (*superseded by*), so the customer is counted once; the reported sales
/// and the leads of the merged party move to the survivor.
pub async fn merge_parties(state: &AppState, survivor: PartyId, merged: PartyId) -> Result {
    // Read before the transaction: it holds the connection.
    let mine = PartyAcquisition::of_party(&state.db, survivor).await?;
    let theirs = PartyAcquisition::of_party(&state.db, merged).await?;
    let mut plan: Option<(PartyAcquisition, Option<PartyAcquisition>)> = None;
    let mut event = None;
    match (mine, theirs) {
        (_, None) => {}
        (None, Some(theirs)) => plan = Some((theirs, None)),
        (Some(mine), Some(theirs)) => {
            let (keep, drop) = if theirs.acquired_at < mine.acquired_at {
                (theirs, mine)
            } else {
                (mine, theirs)
            };
            let old = LeadSource::find(&state.db, drop.source_id)
                .await?
                .map(|s| s.code);
            let new = LeadSource::find(&state.db, keep.source_id)
                .await?
                .map(|s| s.code);
            event = Some(CustomerAcquisitionChanged {
                party: survivor,
                old_source: old,
                new_source: new,
                old_campaign: drop.campaign_id.map(CampaignId),
                new_campaign: keep.campaign_id.map(CampaignId),
                old_first_sale: drop.first_sale(),
                new_first_sale: keep.first_sale(),
                reason: "merged".to_owned(),
            });
            plan = Some((keep, Some(drop)));
        }
    }
    let mut tx = state.db.begin().await?;
    move_records(&mut tx, survivor, merged).await?;
    if let Some((keep, drop)) = plan {
        // The kept row is the survivor's; the other is history.
        set_party(&mut tx, keep.id, survivor.0).await?;
        if let Some(drop) = drop {
            PartyAcquisition::query()
                .where_(PartyAcquisition::ID.eq(drop.id))
                .update(
                    &mut tx,
                    &[("superseded_by", &Some(keep.id)), ("updated_at", &now())],
                )
                .await?;
        }
    }
    tx.commit().await?;
    if let Some(event) = event {
        state.emit(event).await?;
    }
    Ok(())
}

/// Points the acquisition `id` at `party`.
async fn set_party(tx: &mut Transaction, id: i64, party: i64) -> Result {
    PartyAcquisition::query()
        .where_(PartyAcquisition::ID.eq(id))
        .update(&mut *tx, &[("party_id", &party), ("updated_at", &now())])
        .await?;
    Ok(())
}

/// Moves the reported sales and the leads of `merged` to `survivor`.
async fn move_records(tx: &mut Transaction, survivor: PartyId, merged: PartyId) -> Result {
    AcquisitionSale::query()
        .where_(AcquisitionSale::PARTY_ID.eq(merged.0))
        .update(&mut *tx, &[("party_id", &survivor.0), ("updated_at", &now())])
        .await?;
    crate::lead::Lead::query()
        .where_(crate::lead::Lead::PARTY_ID.eq(merged.0))
        .update(&mut *tx, &[("party_id", &Some(survivor.0)), ("updated_at", &now())])
        .await?;
    Ok(())
}

/// The delete check of `campaigns::delete`: a lead or an acquisition on the
/// campaign keeps it.
pub fn delete_check<'a>(
    tx: &'a mut Transaction,
    campaign: i64,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<String>>> + Send + 'a>> {
    Box::pin(async move {
        let leads = crate::lead::Lead::query()
            .where_(crate::lead::Lead::CAMPAIGN_ID.eq(campaign))
            .count(&mut *tx)
            .await?;
        let acquisitions = PartyAcquisition::query()
            .where_(PartyAcquisition::CAMPAIGN_ID.eq(campaign))
            .count(&mut *tx)
            .await?;
        Ok((leads + acquisitions > 0)
            .then(|| crate::texts::english("wl_sales.acquisitions.campaign_in_use", &[])))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sale(id: i64, day: u32, value: i64) -> AcquisitionSale {
        AcquisitionSale {
            id,
            party_id: 1,
            subject_type: "invoicing.invoice".into(),
            subject_id: id,
            sale_date: NaiveDate::from_ymd_opt(2026, 4, day).unwrap(),
            value,
            ..AcquisitionSale::default()
        }
    }

    #[test]
    fn a_sale_credited_in_full_or_gone_no_longer_stands() {
        let mut s = sale(1, 2, 200_000);
        assert!(s.standing());
        s.credited = 50_000;
        assert!(s.standing());
        assert_eq!(s.net(), 150_000);
        s.credited = 200_000;
        assert!(!s.standing());
        assert_eq!(s.net(), 0);
        let mut gone = sale(2, 3, 100);
        gone.gone = true;
        assert!(!gone.standing());
    }

    #[test]
    fn the_converted_value_is_the_first_sale_or_the_window() {
        let sales = vec![sale(1, 2, 200_000), sale(2, 20, 300_000)];
        let mut config = SalesConfig::default();
        assert_eq!(converted_value(&config, &sales[0], &sales), 200_000);
        config.attribution_window_months = Some(12);
        assert_eq!(converted_value(&config, &sales[0], &sales), 500_000);
        // A sale after the window does not count.
        let mut late = sales.clone();
        late.push(AcquisitionSale {
            sale_date: NaiveDate::from_ymd_opt(2027, 4, 2).unwrap(),
            ..sale(3, 2, 900_000)
        });
        assert_eq!(converted_value(&config, &late[0], &late), 500_000);
    }
}
