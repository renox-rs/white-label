//! The kernel's [`RateCards`] on the rate cards of this crate (wl-invoicing §6
//! `RateCards`): the verticals ask whether a contract has an accepted rate
//! card version in force on a day and whether it prices a role on a shift.
//! A version that is `pending`, `awaiting_approval` or `rejected` never
//! counts (`MPW.ACL.16`).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::rate_cards::{
    RateCardFuture, RateCardVersion as KernelVersion, RateCards, VersionRow,
};

use crate::rate_card::RateCard;
use crate::rate_pricing::version_in_force;

/// What `register` provides.
pub(crate) struct Cards;

impl RateCards for Cards {
    fn accepted_version<'a>(
        &'a self,
        state: &'a AppState,
        source: &'a Subject,
        on: NaiveDate,
    ) -> RateCardFuture<'a, Option<KernelVersion>> {
        Box::pin(async move {
            let Some(card) = RateCard::of_source(&state.db, source).await? else {
                return Ok(None);
            };
            Ok(version_in_force(&state.db, card.id, on)
                .await?
                .map(|version| KernelVersion {
                    rate_card: card.id,
                    version: version.version,
                    effective_from: version.effective_from,
                }))
        })
    }

    fn card_of<'a>(
        &'a self,
        state: &'a AppState,
        source: &'a Subject,
    ) -> RateCardFuture<'a, Option<i64>> {
        Box::pin(async move { Ok(RateCard::of_source(&state.db, source).await?.map(|c| c.id)) })
    }

    fn versions<'a>(
        &'a self,
        state: &'a AppState,
        source: &'a Subject,
    ) -> RateCardFuture<'a, Vec<VersionRow>> {
        Box::pin(async move {
            let Some(card) = RateCard::of_source(&state.db, source).await? else {
                return Ok(Vec::new());
            };
            let versions = crate::rate_card::RateCardVersion::query()
                .where_(crate::rate_card::RateCardVersion::RATE_CARD_ID.eq(card.id))
                .order_by(crate::rate_card::RateCardVersion::VERSION)
                .get(&state.db)
                .await?;
            let mut rows = Vec::new();
            for version in versions {
                rows.push(VersionRow {
                    id: version.id,
                    version: version.version,
                    effective_from: version.effective_from,
                    status: version.status.as_str().to_owned(),
                    accepted_on: version.accepted_on,
                    accepted_document: version.accepted_document.clone(),
                    lines: i64::try_from(version.lines(&state.db).await?.len()).unwrap_or(0),
                });
            }
            Ok(rows)
        })
    }

    fn covers<'a>(
        &'a self,
        state: &'a AppState,
        source: &'a Subject,
        role: &'a str,
        shift: &'a str,
        on: NaiveDate,
    ) -> RateCardFuture<'a, bool> {
        Box::pin(async move {
            let Some(card) = RateCard::of_source(&state.db, source).await? else {
                return Ok(false);
            };
            let Some(version) = version_in_force(&state.db, card.id, on).await? else {
                return Ok(false);
            };
            let role = role.trim().to_lowercase();
            let shift = shift.trim().to_lowercase();
            Ok(version
                .lines(&state.db)
                .await?
                .iter()
                .any(|line| line.role == role && line.shift_type == shift))
        })
    }
}
