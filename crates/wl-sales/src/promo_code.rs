//! The promo code a campaign carries (wl-sales §3, `SAL.DSC.01`, #138 A9).
//! Only the part the campaign needs is here: the code, its uses and its
//! validity. The discount rule and redeeming the code come with `SAL.DSC.01`.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::campaign::{Campaign, is_valid_code};

/// A promo code, e.g. `SPRING26`, linked to one campaign.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_sales_promo_codes", unique(code), unique(campaign_id))]
pub struct PromoCode {
    /// The primary key.
    pub id: i64,
    /// The code (upper case).
    pub code: String,
    /// The campaign.
    #[model(references = "wl_sales_campaigns")]
    pub campaign_id: i64,
    /// How often it was used.
    pub uses: i64,
    /// How often it may be used; empty means without limit.
    pub max_uses: Option<i64>,
    /// The last day it is valid; empty means no end.
    pub valid_to: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Why a promo code could not be stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromoRefused {
    /// Not 2 to 20 of `A-Z`, `0-9`, `-`, `_`.
    BadCode,
    /// Another campaign has it.
    CodeTaken,
}

impl PromoCode {
    /// The promo code of `campaign`, if any.
    pub async fn of_campaign(db: &Db, campaign: i64) -> Result<Option<PromoCode>> {
        PromoCode::query()
            .where_(PromoCode::CAMPAIGN_ID.eq(campaign))
            .first(db)
            .await
    }

    /// Whether its uses are used up.
    pub fn used_up(&self) -> bool {
        self.max_uses.is_some_and(|max| self.uses >= max)
    }

    /// Makes the promo code row follow the campaign's `promo_code` field:
    /// created, renamed or removed. A new code starts valid without an end;
    /// the machine sets the validity.
    pub(crate) async fn sync(
        tx: &mut Transaction,
        campaign: &Campaign,
    ) -> Result<std::result::Result<(), PromoRefused>> {
        let current = PromoCode::query()
            .where_(PromoCode::CAMPAIGN_ID.eq(campaign.id))
            .first(&mut *tx)
            .await?;
        let Some(code) = campaign.promo_code.as_deref() else {
            if current.is_some() {
                Self::remove_of(tx, campaign.id).await?;
            }
            return Ok(Ok(()));
        };
        if !is_valid_code(code) {
            return Ok(Err(PromoRefused::BadCode));
        }
        let taken = PromoCode::query()
            .where_(PromoCode::CODE.eq(code))
            .where_(PromoCode::CAMPAIGN_ID.ne(campaign.id))
            .count(&mut *tx)
            .await?;
        if taken > 0 {
            return Ok(Err(PromoRefused::CodeTaken));
        }
        match current {
            Some(mut row) => {
                if row.code != code {
                    row.code = code.to_owned();
                    row.save(&mut *tx).await?;
                }
            }
            None => {
                PromoCode::create(
                    &mut *tx,
                    PromoCode {
                        code: code.to_owned(),
                        campaign_id: campaign.id,
                        ..PromoCode::default()
                    },
                )
                .await?;
            }
        }
        Ok(Ok(()))
    }

    /// Removes the promo code of `campaign`.
    pub(crate) async fn remove_of(tx: &mut Transaction, campaign: i64) -> Result {
        PromoCode::query()
            .where_(PromoCode::CAMPAIGN_ID.eq(campaign))
            .delete(&mut *tx)
            .await?;
        Ok(())
    }

    /// Sets the last valid day of the promo code of `campaign`, if it has one.
    pub(crate) async fn set_valid_to(
        tx: &mut Transaction,
        campaign: i64,
        valid_to: Option<NaiveDate>,
    ) -> Result {
        PromoCode::query()
            .where_(PromoCode::CAMPAIGN_ID.eq(campaign))
            .update(&mut *tx, &[("valid_to", &valid_to)])
            .await?;
        Ok(())
    }

    /// Valid again to `valid_to`, unless the uses are used up (`reopen`).
    pub(crate) async fn reopen(tx: &mut Transaction, campaign: i64, valid_to: NaiveDate) -> Result {
        let row = PromoCode::query()
            .where_(PromoCode::CAMPAIGN_ID.eq(campaign))
            .first(&mut *tx)
            .await?;
        if let Some(row) = row
            && !row.used_up()
        {
            Self::set_valid_to(tx, campaign, Some(valid_to)).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_are_used_up_at_the_maximum() {
        let mut code = PromoCode::default();
        assert!(!code.used_up());
        code.max_uses = Some(3);
        code.uses = 2;
        assert!(!code.used_up());
        code.uses = 3;
        assert!(code.used_up());
    }
}
