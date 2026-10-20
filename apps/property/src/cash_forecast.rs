//! The cash-flow forecast source of 01 Property (wl-invoicing §5.14, §8,
//! `FIN.CFL.01`; BL-PRP-26 item 25): the owner payouts counted as payments.
//!
//! The forecast of wl-invoicing already counts the post-dated rent cheques at
//! their cheque date (it reads the open cheques of wl-payments through the
//! kernel `Cheques` contract), so the cheques need no source here. What it
//! does not know is the money 01 owes its owners: a published owner statement
//! with a positive balance, not disputed and not yet settled, is a payout the
//! accountant will ask for (`pay_out`, `PRP.OWN.01`). Each such statement is
//! an outgoing line of the management company's entity, dated the day it was
//! published (a date in the past counts in week 1), naming the owner and the
//! statement, so the forecast drops it when it already counts the statement.
//!
//! Decided where the spec is silent: the payouts of statements that are not
//! published yet (drafts, approved ones) are not forecast, and no payout is
//! guessed for a month whose statement does not exist: a forecast line stands
//! for a document that exists.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::cash_forecast::{
    CashForecastSource, ForecastFuture, ForecastLine, register_cash_forecast_source,
};
use wl_kernel::documents::today;
use wl_kernel::entities::LegalEntity;
use wl_kernel::payments::Direction;

use crate::agreement::ManagementAgreement;
use crate::statement::{OwnerStatement, StatementStatus};
use crate::texts::english;

/// The name the source is registered under.
pub const SOURCE: &str = "property.owner_payouts";

/// The category of its lines.
pub const CATEGORY: &str = "owner_payout";

/// The owner payouts of the published statements.
#[derive(Debug, Clone, Copy, Default)]
pub struct OwnerPayouts;

/// Registers [`OwnerPayouts`] with the kernel.
pub(crate) fn register() {
    register_cash_forecast_source(SOURCE, OwnerPayouts);
}

/// The day a payout of `statement` is expected: the day it was published, or
/// `today` when that is earlier.
fn expected_on(statement: &OwnerStatement, today: NaiveDate) -> NaiveDate {
    statement.published_on.map_or(today, |day| day.max(today))
}

impl CashForecastSource for OwnerPayouts {
    fn lines<'a>(
        &'a self,
        state: &'a AppState,
        legal_entity_id: i64,
        _from: NaiveDate,
        _to: NaiveDate,
    ) -> ForecastFuture<'a, Vec<ForecastLine>> {
        Box::pin(async move {
            let default = LegalEntity::default_entity(&state.db).await?.id;
            let statements = OwnerStatement::query()
                .where_(OwnerStatement::STATUS.eq(StatementStatus::Published))
                .where_(OwnerStatement::DISPUTED.eq(false))
                .order_by(OwnerStatement::ID)
                .get(&state.db)
                .await?;
            let today = today();
            let mut lines = Vec::new();
            for statement in statements {
                if statement.balance() <= 0 {
                    continue;
                }
                let Some(agreement) =
                    ManagementAgreement::find(&state.db, statement.agreement_id).await?
                else {
                    continue;
                };
                if agreement.legal_entity_id.unwrap_or(default) != legal_entity_id {
                    continue;
                }
                let label = english(
                    "property.forecast.owner_payout",
                    &[("statement", &statement.number())],
                );
                lines.push(
                    ForecastLine::new(
                        &label,
                        CATEGORY,
                        expected_on(&statement, today),
                        statement.balance(),
                        Direction::Out,
                    )
                    .party(PartyId(statement.owner_party_id))
                    .document(statement.subject()),
                );
            }
            Ok(lines)
        })
    }
}
