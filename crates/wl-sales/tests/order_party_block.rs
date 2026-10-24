//! BL-MFG-14 (#395), wl-parties §5.6, 07 §7.1 *Party blocked*: blocking a
//! customer holds its orders that are not delivered yet (`credit_hold`, the
//! reason *account on hold*), keeps their reservations and leaves the orders
//! of other customers alone.

mod support;

use support::changes::{boot_changes, clerk, place, reload};
use wl_kernel::parties::PartyBlocked;
use wl_kernel::sales::CreditFailure;
use wl_sales::SalesOrderStatus;

fn blocked(party: i64) -> PartyBlocked {
    PartyBlocked {
        party: wl_kernel::PartyId(party),
        display_reason: "on_hold".to_owned(),
    }
}

/// Done-when "Party block": the confirmed order of the blocked customer (party
/// 7 in these tests) is on `credit_hold` with the reason *account on hold* and
/// remembers the state it was held from.
#[renox::test]
async fn blocking_the_customer_holds_its_confirmed_order() {
    let (t, c) = boot_changes().await;
    let sam = clerk(&t).await;
    t.app.acting_as(&sam);
    let order = place(&t, &c, "phone", "on_account", &[("pump", "2", "100.00")]).await;

    t.state().emit(blocked(7)).await.unwrap();

    let held = reload(&t, order.id).await;
    assert_eq!(held.status, SalesOrderStatus::CreditHold);
    assert_eq!(held.held_from.as_deref(), Some("confirmed"));
    assert_eq!(
        held.hold_reason.as_deref(),
        Some(CreditFailure::AccountOnHold.as_str())
    );
}

/// Another customer's party is not touched: blocking party 8 holds nothing of
/// party 7.
#[renox::test]
async fn blocking_another_party_holds_nothing() {
    let (t, c) = boot_changes().await;
    let sam = clerk(&t).await;
    t.app.acting_as(&sam);
    let order = place(&t, &c, "phone", "on_account", &[("pump", "2", "100.00")]).await;

    t.state().emit(blocked(8)).await.unwrap();

    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Confirmed
    );
}
