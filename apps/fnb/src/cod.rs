//! Cash on delivery off after repeated failures (06 F&B §7.2 `FNB.RUL.14`;
//! BL-FNB-03).
//!
//! A customer whose cash-on-delivery orders keep failing for customer-side
//! reasons (unreachable, wrong address, refused for another cause, no cash)
//! costs the outlet food each time. When such a failure is the Nth within D
//! days (N = 2, D = 90 by default, [`crate::settings`]) cash on delivery is
//! turned off for the customer: the checkout offers only online payment, with
//! the line "Online payment only for this order", the kitchen manager of the
//! outlet of the failure is told, and can turn it back on.
//!
//! - The customer is the customer party (found by phone number: a direct
//!   order needs the OTP login, a phone order is linked to the party with that
//!   number), counted across all brands and outlets.
//! - The count is of `fail_delivery` failures of cash orders with a
//!   customer-side reason, by failure time, in the rolling window that ends now
//!   (the current failure counts). Failures before the time the manager last
//!   turned cash on delivery back on are not counted. A failure on the outlet's
//!   side (a wrong item, a late order, a rider issue) and a prepaid order never
//!   count.
//! - The customer is never blocked from ordering: only the payment choice
//!   changes.

use renox::chrono::TimeDelta;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_kernel::portal::portal_users;

use crate::notices::{about, tell_managers};
use crate::order::{FnbOrder, FnbOrderStatus, payment, side};
use crate::permissions;
use crate::placing::refused;
use crate::settings::settings;
use crate::texts::english;

/// The per-customer cash on delivery flag.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_customer_flags", unique(party_id))]
pub struct CustomerFlag {
    /// The primary key.
    pub id: i64,
    /// The customer.
    pub party_id: i64,
    /// Whether the checkout offers cash on delivery.
    pub cod_allowed: bool,
    /// When the rule turned it off.
    pub cod_off_at: Option<DateTime>,
    /// The order whose failure turned it off.
    pub cod_off_order_id: Option<i64>,
    /// When a kitchen manager last turned it on.
    pub cod_on_at: Option<DateTime>,
    /// The kitchen manager.
    pub cod_on_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CustomerFlag {
    /// The flag of the customer `party`, if there is one.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of(db: &Db, party: PartyId) -> Result<Option<CustomerFlag>> {
        CustomerFlag::query()
            .where_(CustomerFlag::PARTY_ID.eq(party.0))
            .first(db)
            .await
    }

    /// The customers whose cash on delivery is off, the latest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn turned_off(db: &Db) -> Result<Vec<CustomerFlag>> {
        CustomerFlag::query()
            .where_(CustomerFlag::COD_ALLOWED.eq(false))
            .order_by_desc(CustomerFlag::ID)
            .get(db)
            .await
    }
}

/// What the checkout of the brand's ordering site offers a customer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PaymentOptions {
    /// Online payment is always offered.
    pub online: bool,
    /// Cash on delivery is offered unless `FNB.RUL.14` turned it off.
    pub cash_on_delivery: bool,
}

/// The payment options of the customer `party`.
///
/// # Errors
///
/// Database errors.
pub async fn payment_options(db: &Db, party: PartyId) -> Result<PaymentOptions> {
    let allowed = match CustomerFlag::of(db, party).await? {
        Some(flag) => flag.cod_allowed,
        None => true,
    };
    Ok(PaymentOptions {
        online: true,
        cash_on_delivery: allowed,
    })
}

/// Whether `order` counts for `FNB.RUL.14`: a cash order that failed on the
/// customer's side at or after `since`.
fn counts(order: &FnbOrder, since: DateTime) -> bool {
    order.payment_mode == payment::CASH_ON_DELIVERY
        && order.failure_side.as_deref() == Some(side::CUSTOMER)
        && order.failed_at.is_some_and(|at| at >= since)
}

/// The failures of the customer `party` that count at `at`, with the start of
/// the window: the last D days, never before cash on delivery was turned back
/// on.
///
/// # Errors
///
/// Database errors.
pub async fn failures_counted(state: &AppState, party: PartyId, at: DateTime) -> Result<i64> {
    let window = settings(state).cod_window_days;
    let mut since = at - TimeDelta::days(window);
    if let Some(flag) = CustomerFlag::of(&state.db, party).await?
        && let Some(on) = flag.cod_on_at
        && on > since
    {
        since = on;
    }
    let orders = FnbOrder::query()
        .where_(FnbOrder::PARTY_ID.eq(party.0))
        .where_(FnbOrder::PAYMENT_MODE.eq(payment::CASH_ON_DELIVERY))
        .get(&state.db)
        .await?;
    let counted = orders
        .iter()
        .filter(|order| {
            matches!(
                order.status,
                FnbOrderStatus::DeliveryFailed | FnbOrderStatus::Cancelled
            ) && counts(order, since)
        })
        .count();
    Ok(i64::try_from(counted).unwrap_or(i64::MAX))
}

/// `FNB.RUL.14` after the failure of `order` was recorded: when this is the
/// Nth counted failure of its customer, cash on delivery is turned off and the
/// kitchen manager of the outlet is told.
///
/// # Errors
///
/// Database errors and the notifier's.
pub(crate) async fn on_failure(state: &AppState, order: &FnbOrder) -> Result {
    let rule = settings(state);
    if !rule.cod_rule
        || order.payment_mode != payment::CASH_ON_DELIVERY
        || order.failure_side.as_deref() != Some(side::CUSTOMER)
    {
        return Ok(());
    }
    let party = order.party();
    let now = renox::db::now();
    let counted = failures_counted(state, party, now).await?;
    if counted < rule.cod_failures {
        return Ok(());
    }
    let existing = CustomerFlag::of(&state.db, party).await?;
    match existing {
        Some(flag) if !flag.cod_allowed => return Ok(()),
        Some(flag) => {
            CustomerFlag::query()
                .where_(CustomerFlag::ID.eq(flag.id))
                .update(
                    &state.db,
                    &[
                        ("cod_allowed", &false),
                        ("cod_off_at", &Some(now)),
                        ("cod_off_order_id", &Some(order.id)),
                    ],
                )
                .await?;
        }
        None => {
            let mut flag = CustomerFlag {
                party_id: party.0,
                cod_allowed: false,
                cod_off_at: Some(now),
                cod_off_order_id: Some(order.id),
                ..CustomerFlag::default()
            };
            flag.insert(&state.db).await?;
        }
    }
    let days = rule.cod_window_days.to_string();
    tell_managers(
        state,
        order,
        permissions::CUSTOMERS_COD,
        &about("fnb.cod.off_title", order),
        &english(
            "fnb.cod.off_body",
            &[
                ("number", &order.number),
                ("customer", &order.recipient_name),
                ("count", &counted),
                ("days", &days),
            ],
        ),
    )
    .await?;
    Ok(())
}

/// The kitchen manager (`customers.cod`) turns cash on delivery back on for
/// the customer `party`: failures before now are no longer counted.
///
/// # Errors
///
/// `Forbidden` without `customers.cod`; 422 when it is on already.
pub async fn turn_on(state: &AppState, user: &AuthUser, party: PartyId) -> Result<CustomerFlag> {
    if !user.has_permission(permissions::CUSTOMERS_COD) {
        return Err(Error::Forbidden);
    }
    let Some(flag) = CustomerFlag::of(&state.db, party).await? else {
        return Err(refused(english("fnb.cod.already_on", &[])));
    };
    if flag.cod_allowed {
        return Err(refused(english("fnb.cod.already_on", &[])));
    }
    let now = Some(renox::db::now());
    let by = Some(user.id);
    CustomerFlag::query()
        .where_(CustomerFlag::ID.eq(flag.id))
        .update(
            &state.db,
            &[
                ("cod_allowed", &true),
                ("cod_on_at", &now),
                ("cod_on_by", &by),
            ],
        )
        .await?;
    CustomerFlag::find_or_404(&state.db, flag.id).await
}

/// The customer party the signed-in `user` orders for at the checkout: the
/// party the portal says they act for.
///
/// # Errors
///
/// Database errors.
pub async fn customer_of(state: &AppState, user: &AuthUser) -> Result<Option<PartyId>> {
    let acting = portal_users(state)
        .parties_of(&state.db, user.id, today())
        .await?;
    Ok(acting.first().map(|portal| portal.acting_for))
}
