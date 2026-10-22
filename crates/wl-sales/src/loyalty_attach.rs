//! Attaching a member to a sale (wl-sales §5.9 step 2, `SAL.ACL.08`;
//! BL-SAL-22): a counter sale or a sales order earns only for a member
//! attached **before** it completes, by the member's code, a portal login (the
//! member's party) or the mobile number; never afterwards, and never by a
//! cashier to their own party.
//!
//! Decisions:
//! - "The cashier's own party" is the party of the employee whose account is
//!   the user (`Workforce::employee_of_user`, `Workforce::staff`); a guardian
//!   who accepted the terms for a minor counts as the same person.
//! - A sale completes when a counter sale is `completed` and when a sales
//!   order is `fulfilled`; an order that was cancelled or rejected never
//!   earns, so a member is not attached to it either.
//! - The one-time code that confirms a *redemption* is BL-SAL-23; attaching
//!   needs only the member's code, the portal login or the mobile number.

use renox::db::now;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::workforce::workforce;

use crate::counter_sale::CounterSale;
use crate::loyalty_account::LoyaltyAccount;
use crate::loyalty_programme::LoyaltyProgramme;
use crate::sales_order::{SalesOrder, SalesOrderStatus};
use crate::texts::english;

/// How the member is named at the counter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberRef {
    /// The member's code, e.g. `LYL-0001`.
    Code(String),
    /// The mobile number, written any way.
    Mobile(String),
    /// The member's party: a portal login.
    Party(PartyId),
}

/// A 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// The open account `member` names, if any.
pub async fn find_member(db: &Db, member: &MemberRef) -> Result<Option<LoyaltyAccount>> {
    match member {
        MemberRef::Code(code) => LoyaltyAccount::by_code(db, code).await,
        MemberRef::Mobile(mobile) => LoyaltyAccount::by_mobile(db, mobile).await,
        MemberRef::Party(party) => LoyaltyAccount::of_party(db, *party).await,
    }
}

/// The party of the employee whose account is the user `user_id`, if the
/// user is staff.
pub async fn cashier_party(state: &AppState, user_id: i64) -> Result<Option<PartyId>> {
    let people = workforce(state);
    let Some(employee) = people.employee_of_user(&state.db, user_id).await? else {
        return Ok(None);
    };
    Ok(people.staff(state, employee).await?.map(|p| p.party))
}

/// The account `member` names, checked for `SAL.ACL.08`: it exists, its
/// programme earns today, and it is not the cashier's own.
async fn account_for(
    state: &AppState,
    by: &AuthUser,
    member: &MemberRef,
) -> Result<LoyaltyAccount> {
    let Some(account) = find_member(&state.db, member).await? else {
        return Err(refused("wl_sales.loyalty.problem.no_member"));
    };
    if account.paused {
        return Err(crate::loyalty_account::on_hold(account.party()));
    }
    let on = wl_kernel::documents::today();
    // A programme that was turned off still takes a member to the sale, to
    // redeem through the closing date; the sale earns nothing.
    let earns = LoyaltyProgramme::find(&state.db, account.programme_id)
        .await?
        .is_some_and(|p| {
            p.earns_on(on) || (p.active && p.closing_date.is_some_and(|last| last >= on))
        });
    if !earns {
        return Err(refused("wl_sales.loyalty.problem.programme_off"));
    }
    if let Some(own) = cashier_party(state, by.id).await?
        && (own == account.party() || Some(own.0) == account.accepted_by_party_id)
    {
        return Err(refused("wl_sales.loyalty.problem.own_account"));
    }
    Ok(account)
}

/// Attaches `member` to the open counter sale `id` as `by`.
///
/// # Errors
///
/// 422 when the sale is completed (never afterwards), the member is unknown
/// or closed, the programme is off, or the member is the cashier's own party.
pub async fn attach_to_counter_sale(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    member: &MemberRef,
) -> Result<CounterSale> {
    let sale = CounterSale::find_or_404(&state.db, id).await?;
    if !sale.is_open() {
        return Err(refused("wl_sales.loyalty.problem.sale_completed"));
    }
    let account = account_for(state, by, member).await?;
    CounterSale::query()
        .where_(CounterSale::ID.eq(id))
        .update(
            &state.db,
            &[
                ("loyalty_account_id", &Some(account.id)),
                ("loyalty_attached_by", &Some(by.id)),
                ("loyalty_attached_at", &Some(now())),
            ],
        )
        .await?;
    CounterSale::find_or_404(&state.db, id).await
}

/// Attaches `member` to the sales order `id` as `by`, until it is fulfilled.
///
/// # Errors
///
/// As [`attach_to_counter_sale`]; an order that is `fulfilled`, `closed`,
/// `cancelled` or `rejected` takes no member.
pub async fn attach_to_sales_order(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    member: &MemberRef,
) -> Result<SalesOrder> {
    let order = SalesOrder::find_or_404(&state.db, id).await?;
    if matches!(
        order.status,
        SalesOrderStatus::Fulfilled
            | SalesOrderStatus::Closed
            | SalesOrderStatus::Cancelled
            | SalesOrderStatus::Rejected
    ) {
        return Err(refused("wl_sales.loyalty.problem.sale_completed"));
    }
    let account = account_for(state, by, member).await?;
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(id))
        .update(
            &state.db,
            &[
                ("loyalty_account_id", &Some(account.id)),
                ("loyalty_attached_by", &Some(by.id)),
                ("loyalty_attached_at", &Some(now())),
            ],
        )
        .await?;
    SalesOrder::find_or_404(&state.db, id).await
}
