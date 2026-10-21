//! The cash-flow forecast source of 03 Logistics (wl-invoicing §5.14,
//! `FIN.CFL.01`; BL-LOG-26): the COD remittances owed to clients counted as
//! payments on their schedule.
//!
//! The forecast of wl-invoicing counts the invoices and the open cheques; it
//! does not know the money 03 holds for its clients. A remittance that is
//! `approved` is paid by the accountant now, so it is an outgoing line of
//! today; a `draft` is paid after it is approved, at the next run of the
//! weekly rule (`LOG.RUL.06`, Monday), so it is a line of that day; a
//! `payment_returned` one is owed again, today. Each names the client and the
//! remittance, so the forecast drops the line when it already counts the
//! remittance. The money goes out of the company's default legal entity.
//! Disbursements recharged to a client are invoices, which the forecast counts
//! as receipts on their own.

use renox::chrono::{Datelike, NaiveDate, TimeDelta};
use renox::prelude::*;
use wl_kernel::cash_forecast::{
    CashForecastSource, ForecastFuture, ForecastLine, register_cash_forecast_source,
};
use wl_kernel::documents::today;
use wl_kernel::entities::LegalEntity;
use wl_kernel::payments::Direction;

use crate::remit::DAY;
use crate::remittance::{Remittance, RemittanceStatus};
use crate::texts::english;

/// The name the source is registered under.
pub const SOURCE: &str = "logistics.cod_remittances";

/// The category of its lines.
pub const CATEGORY: &str = "cod_remittance";

/// The COD remittances owed to clients.
#[derive(Debug, Clone, Copy, Default)]
pub struct CodRemittances;

/// Registers [`CodRemittances`] with the kernel.
pub(crate) fn register() {
    register_cash_forecast_source(SOURCE, CodRemittances);
}

/// The day of the next weekly run on or after `day`.
pub fn next_run(day: NaiveDate) -> NaiveDate {
    let wanted = i64::from(DAY.num_days_from_monday());
    let have = i64::from(day.weekday().num_days_from_monday());
    day + TimeDelta::days((wanted - have).rem_euclid(7))
}

/// The day the payout of `remittance` is expected on `today`.
pub fn expected_on(remittance: &Remittance, today: NaiveDate) -> NaiveDate {
    match remittance.status {
        RemittanceStatus::Draft => next_run(today),
        _ => today,
    }
}

impl CashForecastSource for CodRemittances {
    fn lines<'a>(
        &'a self,
        state: &'a AppState,
        legal_entity_id: i64,
        _from: NaiveDate,
        _to: NaiveDate,
    ) -> ForecastFuture<'a, Vec<ForecastLine>> {
        Box::pin(async move {
            let default = LegalEntity::default_entity(&state.db).await?.id;
            if default != legal_entity_id {
                return Ok(Vec::new());
            }
            let owed = Remittance::query()
                .where_(Remittance::STATUS.is_in([
                    RemittanceStatus::Draft,
                    RemittanceStatus::Approved,
                    RemittanceStatus::PaymentReturned,
                ]))
                .order_by(Remittance::ID)
                .get(&state.db)
                .await?;
            let today = today();
            let mut lines = Vec::new();
            for remittance in owed.into_iter().filter(|r| r.net > 0) {
                let label = english(
                    "logistics.forecast.cod_remittance",
                    &[("number", &remittance.number)],
                );
                lines.push(
                    ForecastLine::new(
                        &label,
                        CATEGORY,
                        expected_on(&remittance, today),
                        remittance.net,
                        Direction::Out,
                    )
                    .party(remittance.client())
                    .document(remittance.subject()),
                );
            }
            Ok(lines)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().expect("a date")
    }

    #[test]
    fn a_draft_is_paid_at_the_next_monday_run() {
        // 2026-11-02 is a Monday.
        assert_eq!(next_run(day("2026-11-02")), day("2026-11-02"));
        assert_eq!(next_run(day("2026-11-03")), day("2026-11-09"));
        assert_eq!(next_run(day("2026-11-08")), day("2026-11-09"));
    }
}
