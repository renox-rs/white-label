//! The cash-flow forecast sources of 01 Property (wl-invoicing §5.14, §8,
//! `FIN.CFL.01`; BL-PRP-26 item 25): the owner payouts counted as payments,
//! and the scheduled lease charges not yet invoiced counted as receipts.
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
//!
//! The lease keeps its own charge schedule ([`crate::charge`]): a *scheduled*
//! charge is billed by the daily run on its due date, so until then it is a
//! receipt the forecast knows of no invoice for (§5.14 step 3, "scheduled
//! charges not yet invoiced by due date"). Each one is an incoming line of the
//! entity that will issue its invoice (the charge's own entity, else the
//! entity of the lease's building), dated its due date (a past one counts in
//! week 1), for its amount with VAT as the invoice will carry it, naming the
//! tenant and the charge. A post-dated cheque linked to the charge
//! (`property.lease_charge:<id>`) names it too, so the forecast drops the
//! charge line and counts the cheque by its date (step 5). Once its invoice
//! is issued the charge counts through the invoice only, and a cheque still
//! linked to the charge covers that invoice (wl-invoicing reads a link to a
//! record a counted invoice bills as a link to the invoice), so the invoice
//! counts only what the cheque leaves; while the daily run's
//! invoice is still a draft (wl-invoicing does not count drafts) the charge
//! stays a line here, so it is counted once either way.
//! Decided where the spec is silent: only the charges the daily run will bill
//! are forecast, those of a lease in force (or of an amended version, which
//! still bills what it kept); a cheque that covers part of a charge replaces
//! the whole charge line, as for any source line.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::cash_forecast::{
    CashForecastSource, ForecastFuture, ForecastLine, register_cash_forecast_source,
};
use wl_kernel::documents::today;
use wl_kernel::entities::LegalEntity;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};
use wl_kernel::payments::Direction;

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use crate::agreement::ManagementAgreement;
use crate::building::BUILDING_SUBJECT_TYPE;
use crate::charge::{ChargeStatus, LeaseCharge};
use crate::lease::{Lease, LeaseStatus};
use crate::statement::{OwnerStatement, StatementStatus};
use crate::texts::english;

/// The name the source is registered under.
pub const SOURCE: &str = "property.owner_payouts";

/// The category of its lines.
pub const CATEGORY: &str = "owner_payout";

/// The owner payouts of the published statements.
#[derive(Debug, Clone, Copy, Default)]
pub struct OwnerPayouts;

/// The name the scheduled charges are registered under.
pub const CHARGES_SOURCE: &str = "property.lease_charges";

/// The category of the scheduled charge lines.
pub const CHARGES_CATEGORY: &str = "scheduled_charge";

/// The scheduled lease charges not yet invoiced.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScheduledCharges;

/// Registers [`OwnerPayouts`] and [`ScheduledCharges`] with the kernel.
pub(crate) fn register() {
    register_cash_forecast_source(SOURCE, OwnerPayouts);
    register_cash_forecast_source(CHARGES_SOURCE, ScheduledCharges);
}

/// What the invoice of `charge` will ask: its amount plus the VAT of its
/// category, rounded half up to the fil as the invoice line rounds it.
pub fn gross_of(charge: &LeaseCharge) -> i64 {
    let vat = (i128::from(charge.amount) * i128::from(charge.vat_category.rate_bp()) + 5_000)
        .div_euclid(10_000);
    charge.amount + i64::try_from(vat).unwrap_or(0)
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

/// Whether `charge` has no issued invoice yet: it is scheduled, or the
/// invoice the daily run made for it is still a draft.
async fn not_yet_invoiced(state: &AppState, charge: &LeaseCharge) -> Result<bool> {
    match (charge.status, charge.invoice_id) {
        (ChargeStatus::Scheduled, None) => Ok(true),
        (ChargeStatus::Invoiced, Some(id)) => Ok(invoicer(state)
            .find(&state.db, InvoiceId(id))
            .await?
            .is_some_and(|invoice| invoice.status == InvoiceStatus::Draft)),
        _ => Ok(false),
    }
}

impl CashForecastSource for ScheduledCharges {
    fn lines<'a>(
        &'a self,
        state: &'a AppState,
        legal_entity_id: i64,
        _from: NaiveDate,
        to: NaiveDate,
    ) -> ForecastFuture<'a, Vec<ForecastLine>> {
        Box::pin(async move {
            let charges = LeaseCharge::query()
                .where_(
                    LeaseCharge::STATUS.is_in([ChargeStatus::Scheduled, ChargeStatus::Invoiced]),
                )
                .where_(LeaseCharge::DUE_DATE.lte(to))
                .order_by(LeaseCharge::DUE_DATE)
                .order_by(LeaseCharge::ID)
                .get(&state.db)
                .await?;
            let mut leases: BTreeMap<i64, Option<Lease>> = BTreeMap::new();
            let mut issuers: BTreeMap<i64, i64> = BTreeMap::new();
            let mut lines = Vec::new();
            for charge in charges {
                if !not_yet_invoiced(state, &charge).await? {
                    continue;
                }
                if let Entry::Vacant(slot) = leases.entry(charge.lease_id) {
                    slot.insert(Lease::find(&state.db, charge.lease_id).await?);
                }
                let Some(Some(lease)) = leases.get(&charge.lease_id) else {
                    continue;
                };
                if !LeaseStatus::RUNNING.contains(&lease.status)
                    && lease.status != LeaseStatus::Amended
                {
                    continue;
                }
                let issuer = match charge.legal_entity_id {
                    Some(entity) => entity,
                    None => match issuers.get(&lease.building_id) {
                        Some(entity) => *entity,
                        None => {
                            let entity = LegalEntity::for_scope(
                                &state.db,
                                Some((BUILDING_SUBJECT_TYPE, lease.building_id)),
                            )
                            .await?
                            .id;
                            issuers.insert(lease.building_id, entity);
                            entity
                        }
                    },
                };
                if issuer != legal_entity_id {
                    continue;
                }
                let label = english(
                    "property.forecast.lease_charge",
                    &[("charge", &charge.description), ("lease", &lease.number())],
                );
                lines.push(
                    ForecastLine::new(
                        &label,
                        CHARGES_CATEGORY,
                        charge.due_date,
                        gross_of(&charge),
                        Direction::In,
                    )
                    .party(lease.tenant())
                    .document(charge.subject()),
                );
            }
            Ok(lines)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::VatCategory;

    #[test]
    fn the_gross_adds_the_vat_of_the_category() {
        let charge = |amount, vat_category| LeaseCharge {
            amount,
            vat_category,
            ..Default::default()
        };
        assert_eq!(gross_of(&charge(1_000_000, VatCategory::Exempt)), 1_000_000);
        assert_eq!(
            gross_of(&charge(1_000_000, VatCategory::Standard)),
            1_050_000
        );
        // 5% of 1,001 fils is 50.05: rounded to 50.
        assert_eq!(gross_of(&charge(1_001, VatCategory::Standard)), 1_051);
        // 5% of 1,010 fils is 50.5: rounded up to 51.
        assert_eq!(gross_of(&charge(1_010, VatCategory::Standard)), 1_061);
    }
}
