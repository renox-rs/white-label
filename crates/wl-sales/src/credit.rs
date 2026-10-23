//! The credit check of sales orders (wl-sales §3 *Credit check*, §4.2 *Credit
//! re-check at dispatch*, §5.2, §6 `CreditChecker`, `SAL.ACL.03`,
//! `SAL.RUL.01`; BL-SAL-10): what a customer owes, whether one more order
//! fits, and the hold when it does not.
//!
//! Decisions:
//! - **Exposure** is open invoices (the `Invoicer`'s `open_of`, balances with
//!   VAT) + uncleared received cheques (`Cheques::uncleared_amount`) + the
//!   undelivered value, with VAT, of the customer's other `confirmed` and
//!   `partially_fulfilled` orders. Orders on `credit_hold` are not counted
//!   until released; a fulfilled order is counted by its invoice.
//! - The order being checked is counted once: the exposure leaves it out and
//!   its own undelivered value is added. It fails when the sum is above the
//!   limit (equal passes); a credit group adds the exposure of every member
//!   account and compares with the combined limit.
//! - **Overdue** is the balance of invoices more than the grace days past
//!   their due date (the account's own grace, else the setting). A disputed
//!   invoice is left out: a dispute is not a refusal to pay.
//! - **Order of failures:** the account on hold, then overdue, then the
//!   limit; the first that applies is the reason.
//! - Only an order **on account** is checked, and only when its customer has
//!   a [`CustomerAccount`]. Prepaid, cash and cash-on-delivery orders and an
//!   order settled by set-off are not.
//! - **A released order** (BL-SAL-11) passes the next re-check without
//!   checking again when nothing has got worse since the release: its
//!   exposure and overdue balance, taken at the release, are not exceeded.
//!   An account on hold always fails.
//! - [`credit_recheck`] runs `hold` itself on a failure, so every caller (a
//!   dispatch note in `wl-inventory`, the counter collection, a vertical)
//!   behaves the same; [`Checker`] is the kernel's `CreditChecker`.

use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::PartyId;
use wl_kernel::cheques::cheques;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceSummary, invoicer};
use wl_kernel::sales::{CreditChecker, CreditFailure, CreditVerdict, SalesFuture, SalesOrderId};

use crate::config::config_of;
use crate::customer_account::{CreditGroup, CustomerAccount};
use crate::sale_line::{SaleLine, ratio_half_up};
use crate::sales_order::{SalesOrder, SalesOrderStatus};
use crate::sales_order_machine::{HOLD, HoldInput};

/// The currency of the exposure: the only one the customer accounts use.
const CURRENCY: &str = "AED";

/// What a customer owes, in fils.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Exposure {
    /// The balances of the open invoices.
    pub invoices: i64,
    /// The received cheques not cleared yet.
    pub cheques: i64,
    /// The undelivered value of the other confirmed orders.
    pub orders: i64,
}

impl Exposure {
    /// The three added up.
    pub fn total(&self) -> i64 {
        self.invoices + self.cheques + self.orders
    }
}

/// The figures and the verdict of one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assessment {
    /// Why it fails; `None` when it passes.
    pub failure: Option<CreditFailure>,
    /// What the customer owes without the order.
    pub exposure: i64,
    /// The undelivered value of the order checked.
    pub order_value: i64,
    /// The overdue balance beyond the grace days.
    pub overdue: i64,
    /// The customer's limit.
    pub limit: Option<i64>,
    /// The exposure of the whole credit group without the order.
    pub group_exposure: Option<i64>,
    /// The group's combined limit.
    pub group_limit: Option<i64>,
}

impl Assessment {
    /// What the customer owes with the order counted: exposure plus the
    /// order's value.
    pub fn open_balance(&self) -> i64 {
        self.exposure + self.order_value
    }

    /// The hold the figures call for, if the check failed.
    pub fn hold(&self) -> Option<HoldInput> {
        self.failure.map(|reason| HoldInput {
            reason,
            open_balance: self.open_balance(),
            overdue: self.overdue,
            limit: self.limit,
        })
    }
}

/// The undelivered value of `lines` with VAT, in fils: each item line's
/// gross in proportion to its open quantity.
pub fn undelivered_value(lines: &[SaleLine]) -> i64 {
    lines
        .iter()
        .filter(|line| line.kind == crate::sale_line::KIND_ITEM)
        .map(|line| ratio_half_up(line.gross(), line.open_milli(), line.quantity_milli))
        .sum()
}

/// What `party` owes: its open invoices, uncleared cheques and the
/// undelivered value of its other confirmed orders (`except` is left out).
pub async fn exposure_of(
    state: &AppState,
    party: PartyId,
    except: Option<i64>,
) -> Result<Exposure> {
    let invoices: i64 = open_invoices(state, party)
        .await?
        .iter()
        .map(|invoice| invoice.balance)
        .sum();
    let held = cheques(state)
        .uncleared_amount(&state.db, party, CURRENCY)
        .await?;
    let mut orders = 0;
    for order in SalesOrder::query()
        .where_(SalesOrder::PARTY_ID.eq(party.0))
        .where_(SalesOrder::STATUS.is_in([
            SalesOrderStatus::Confirmed,
            SalesOrderStatus::PartiallyFulfilled,
        ]))
        .get(&state.db)
        .await?
    {
        if Some(order.id) != except {
            orders += undelivered_value(&SaleLine::of(&state.db, &order.subject()).await?);
        }
    }
    Ok(Exposure {
        invoices,
        cheques: held,
        orders,
    })
}

/// The open invoices of `party` with a balance.
async fn open_invoices(state: &AppState, party: PartyId) -> Result<Vec<InvoiceSummary>> {
    invoicer(state).open_of(&state.db, party).await
}

/// The balance of the invoices of `party` more than `grace` days past their
/// due date; disputed invoices are left out.
pub async fn overdue_of(state: &AppState, party: PartyId, grace: i64) -> Result<i64> {
    let day = today();
    Ok(open_invoices(state, party)
        .await?
        .iter()
        .filter(|invoice| !invoice.disputed)
        .filter(|invoice| {
            invoice
                .due_date
                .is_some_and(|due| (day - due).num_days() > grace)
        })
        .map(|invoice| invoice.balance)
        .sum())
}

/// Checks `order_value` fils (on top of the exposure without the order
/// `except`) against the account's limits, overdue balance and on-hold flag.
pub async fn assess(
    state: &AppState,
    account: &CustomerAccount,
    order_value: i64,
    except: Option<i64>,
) -> Result<Assessment> {
    let config = config_of(state);
    let own = exposure_of(state, account.party(), except).await?.total();
    let overdue = overdue_of(
        state,
        account.party(),
        account.grace(config.credit_grace_days),
    )
    .await?;
    let group: Option<CreditGroup> = account.group(&state.db).await?;
    let (group_exposure, group_limit) = match &group {
        Some(group) if group.combined_limit.is_some() => {
            let mut total = 0;
            for member in account.group_members(&state.db).await? {
                total += if member.id == account.id {
                    own
                } else {
                    exposure_of(state, member.party(), None).await?.total()
                };
            }
            (Some(total), group.combined_limit)
        }
        _ => (None, None),
    };
    let failure = if account.is_on_hold(&state.db).await? {
        Some(CreditFailure::AccountOnHold)
    } else if config.hold_on_overdue && overdue > 0 {
        Some(CreditFailure::Overdue)
    } else if config.hold_on_limit
        && (account
            .credit_limit
            .is_some_and(|limit| own + order_value > limit)
            || group_exposure
                .zip(group_limit)
                .is_some_and(|(total, limit)| total + order_value > limit))
    {
        Some(CreditFailure::Limit)
    } else {
        None
    };
    Ok(Assessment {
        failure,
        exposure: own,
        order_value,
        overdue,
        limit: account.credit_limit,
        group_exposure,
        group_limit,
    })
}

/// The check an order on account goes through at `submit` and `accept`: the
/// hold it calls for, or `None` when it passes or is not checked (not on
/// account, no customer account, the setting off).
pub async fn hold_for_placing(state: &AppState, order: &SalesOrder) -> Result<Option<HoldInput>> {
    if !config_of(state).credit_check || !order.is_on_account() {
        return Ok(None);
    }
    let Some(account) = CustomerAccount::of_party(&state.db, order.party_id).await? else {
        return Ok(None);
    };
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let value = undelivered_value(&lines);
    Ok(assess(state, &account, value, Some(order.id)).await?.hold())
}

/// Re-checks the credit of order `id` before its goods leave
/// (`credit_recheck`, §4.2): a failure runs `hold` on the order (it keeps its
/// reservations) and answers [`CreditVerdict::Failed`].
///
/// An order already on `credit_hold` fails again with its stored reason, so
/// the goods cannot leave while it is held.
pub async fn credit_recheck(state: &AppState, id: SalesOrderId) -> Result<CreditVerdict> {
    let config = config_of(state);
    let Some(order) = SalesOrder::find(&state.db, id.0).await? else {
        return Ok(CreditVerdict::NotApplicable);
    };
    if !config.credit_check || !config.credit_recheck_at_dispatch || !order.is_on_account() {
        return Ok(CreditVerdict::NotApplicable);
    }
    match order.status {
        SalesOrderStatus::CreditHold => {
            let reason = order
                .hold_reason
                .as_deref()
                .and_then(CreditFailure::parse)
                .unwrap_or(CreditFailure::Limit);
            return Ok(CreditVerdict::Failed(reason));
        }
        SalesOrderStatus::Confirmed | SalesOrderStatus::PartiallyFulfilled => {}
        _ => return Ok(CreditVerdict::NotApplicable),
    }
    let Some(account) = CustomerAccount::of_party(&state.db, order.party_id).await? else {
        return Ok(CreditVerdict::NotApplicable);
    };
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let found = assess(state, &account, undelivered_value(&lines), Some(order.id)).await?;
    if found.failure != Some(CreditFailure::AccountOnHold) && released_and_no_worse(&order, &found)
    {
        return Ok(CreditVerdict::Passed);
    }
    let Some(hold) = found.hold() else {
        return Ok(CreditVerdict::Passed);
    };
    hold_order(state, &order, hold).await?;
    Ok(CreditVerdict::Failed(hold.reason))
}

/// Whether the order was released after a hold and the customer owes no more
/// than at the release (nothing changed the exposure since).
fn released_and_no_worse(order: &SalesOrder, found: &Assessment) -> bool {
    order.released_at.is_some()
        && order
            .release_exposure
            .is_some_and(|before| found.exposure <= before)
        && order
            .release_overdue
            .is_some_and(|before| found.overdue <= before)
}

/// Notes what the customer owes at the moment `order` was released, so the
/// next dispatch passes while it owes no more (BL-SAL-11).
pub(crate) async fn remember_release(state: &AppState, order: &SalesOrder) -> Result {
    let Some(account) = CustomerAccount::of_party(&state.db, order.party_id).await? else {
        return Ok(());
    };
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let found = assess(state, &account, undelivered_value(&lines), Some(order.id)).await?;
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(order.id))
        .update(
            &state.db,
            &[
                ("release_exposure", &Some(found.exposure)),
                ("release_overdue", &Some(found.overdue)),
            ],
        )
        .await?;
    Ok(())
}

/// Runs `hold` on the confirmed or partly fulfilled `order` with `hold`.
pub async fn hold_order(state: &AppState, order: &SalesOrder, hold: HoldInput) -> Result {
    let _ = order
        .transition(HOLD)
        .by_system()
        .input(&hold)
        .meta(renox::serde_json::to_value(hold).unwrap_or_default())
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    Ok(())
}

/// The kernel's [`CreditChecker`].
pub(crate) struct Checker;

impl CreditChecker for Checker {
    fn credit_recheck<'a>(
        &'a self,
        state: &'a AppState,
        order: SalesOrderId,
    ) -> SalesFuture<'a, CreditVerdict> {
        Box::pin(async move { credit_recheck(state, order).await })
    }

    fn check_amount<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
        amount: i64,
    ) -> SalesFuture<'a, CreditVerdict> {
        Box::pin(async move {
            if !config_of(state).credit_check {
                return Ok(CreditVerdict::NotApplicable);
            }
            let Some(account) = CustomerAccount::of_party(&state.db, party.0).await? else {
                return Ok(CreditVerdict::NotApplicable);
            };
            Ok(match assess(state, &account, amount, None).await?.failure {
                Some(failure) => CreditVerdict::Failed(failure),
                None => CreditVerdict::Passed,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::invoicing::VatCategory;

    fn line(price: i64, quantity: i64, fulfilled: i64) -> SaleLine {
        SaleLine {
            kind: crate::sale_line::KIND_ITEM.into(),
            quantity_milli: quantity,
            fulfilled_milli: fulfilled,
            unit_price: price,
            vat_category: VatCategory::ZeroRated,
            ..SaleLine::default()
        }
    }

    #[test]
    fn the_undelivered_value_is_the_open_share_of_each_line() {
        assert_eq!(undelivered_value(&[line(800_000, 1000, 0)]), 800_000);
        assert_eq!(
            undelivered_value(&[line(10_000, 4000, 1000), line(5_000, 1000, 1000)]),
            30_000
        );
        assert_eq!(undelivered_value(&[]), 0);
    }

    #[test]
    fn the_open_balance_counts_the_order_once() {
        let found = Assessment {
            failure: Some(CreditFailure::Limit),
            exposure: 4_600_000,
            order_value: 800_000,
            overdue: 0,
            limit: Some(5_000_000),
            group_exposure: None,
            group_limit: None,
        };
        assert_eq!(found.open_balance(), 5_400_000);
        let hold = found.hold().unwrap();
        assert_eq!(
            (hold.reason, hold.open_balance),
            (CreditFailure::Limit, 5_400_000)
        );
        assert!(
            Assessment {
                failure: None,
                ..found
            }
            .hold()
            .is_none()
        );
    }

    #[test]
    fn a_release_holds_while_the_customer_owes_no_more() {
        let order = SalesOrder {
            released_at: Some(renox::db::now()),
            release_exposure: Some(4_600_000),
            release_overdue: Some(0),
            ..SalesOrder::default()
        };
        let found = |exposure, overdue| Assessment {
            failure: Some(CreditFailure::Limit),
            exposure,
            order_value: 0,
            overdue,
            limit: None,
            group_exposure: None,
            group_limit: None,
        };
        assert!(released_and_no_worse(&order, &found(4_600_000, 0)));
        assert!(!released_and_no_worse(&order, &found(4_600_001, 0)));
        assert!(!released_and_no_worse(&order, &found(4_000_000, 1)));
        assert!(!released_and_no_worse(&SalesOrder::default(), &found(0, 0)));
    }
}
