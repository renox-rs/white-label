//! The kernel's `SaleCustomers` provided by wl-sales (wl-sales §6; used by the
//! 08 recall trace, BL-AUT-16): the customer of a counter sale or a sales
//! order, a walk-in sale when a counter sale names nobody.

mod support;

use renox::prelude::*;
use wl_kernel::sales::{SaleCustomer, sale_customers};
use wl_kernel::{PartyId, Subject};
use wl_sales::CounterSale;
use wl_sales::permissions as perm;

use support::boot;

#[renox::test]
async fn a_counter_sale_names_its_customer_or_is_a_walk_in_sale() {
    let t = boot().await;
    let clerk = t.user("Clara", &[perm::POS_SELL]).await;
    t.app.acting_as(&clerk);
    t.app
        .post("/counter-sales", &[("x", "")])
        .await
        .assert_status(303);
    let sale = CounterSale::query()
        .order_by_desc(CounterSale::ID)
        .first(t.db())
        .await
        .unwrap()
        .expect("a cart");
    let customers = sale_customers(t.state());

    assert_eq!(
        customers
            .customer(t.state(), &sale.subject())
            .await
            .unwrap(),
        SaleCustomer::WalkIn
    );

    CounterSale::query()
        .where_(CounterSale::ID.eq(sale.id))
        .update(t.db(), &[("party_id", &Some(9_i64))])
        .await
        .unwrap();
    assert_eq!(
        customers
            .customer(t.state(), &sale.subject())
            .await
            .unwrap(),
        SaleCustomer::Party(PartyId(9))
    );

    // A sale that does not exist, and a record that is not a sale.
    let missing = Subject::new("sales.counter_sale", sale.id + 100);
    assert_eq!(
        customers.customer(t.state(), &missing).await.unwrap(),
        SaleCustomer::Unknown
    );
    let other = Subject::new("sales.lead", 1);
    assert_eq!(
        customers.customer(t.state(), &other).await.unwrap(),
        SaleCustomer::Unknown
    );
}
