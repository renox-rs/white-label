//! Leads, sales orders, counter sales, sales returns and packages in the
//! search index and the Related panel (BL-AGT-11): each type's document is
//! written as the record is made, the order is found by its number and its
//! customer's PO, documents follow the transitions, and the relations are
//! produced (a sale's returns, a return's sale, a package's customer).

mod support;

use renox::prelude::*;
use wl_kernel::invoicing::InvoiceId;
use wl_kernel::related::{self, RelatedGroup};
use wl_kernel::sales::{
    COUNTER_SALE_SUBJECT_TYPE, LEAD_SUBJECT_TYPE, PACKAGE_SUBJECT_TYPE, SALES_ORDER_SUBJECT_TYPE,
    SALES_RETURN_SUBJECT_TYPE,
};
use wl_kernel::scopes::Reach;
use wl_kernel::search::{self, Access};
use wl_kernel::{PartyId, Subject};
use wl_sales::lead::{Captured, NewLead};
use wl_sales::packages::{self, NewPackage};
use wl_sales::permissions as perm;
use wl_sales::sales_orders::{self, NewSalesOrder};
use wl_sales::{CounterSale, SalesReturn};

use support::boot;

fn count(groups: &[RelatedGroup], name: &str) -> Option<u64> {
    groups.iter().find(|g| g.name == name).map(|g| g.count)
}

async fn found(db: &Db, text: &str, subject: &Subject) -> bool {
    search::lookup_identifier(db, text)
        .await
        .unwrap()
        .iter()
        .any(|m| &m.subject == subject)
}

/// Someone who may see every sales type, everywhere.
fn everyone() -> Access {
    let permissions = [
        perm::LEADS_VIEW,
        perm::SALES_ORDERS_VIEW,
        perm::POS_SELL,
        perm::RETURNS_REQUEST,
        perm::PACKAGES_VIEW,
        "parties.view",
        "invoices.view",
    ];
    Access::with(
        1,
        permissions
            .iter()
            .map(|p| ((*p).to_owned(), Reach::everything())),
    )
}

#[renox::test]
async fn an_order_is_found_by_its_number_and_its_customer_po() {
    let t = boot().await;
    let order = sales_orders::create(
        t.state(),
        None,
        NewSalesOrder::new(PartyId(7))
            .channel("phone")
            .payment_mode("on_account")
            .customer_po("PO-ALN-5521"),
    )
    .await
    .unwrap();
    let subject = Subject::new(SALES_ORDER_SUBJECT_TYPE, order.id);
    let document = search::document_of(t.db(), &subject)
        .await
        .unwrap()
        .expect("the order is indexed as it is entered");
    assert_eq!(document.title, order.number);
    assert_eq!(document.status.as_deref(), Some("draft"));
    assert!(found(t.db(), &order.number.to_lowercase(), &subject).await);
    assert!(found(t.db(), "po aln 5521", &subject).await);
    // No amount reaches the document.
    for text in [&document.title, &document.secondary, &document.keywords] {
        assert!(!text.contains("AED"));
    }

    // A new reference is found at once.
    wl_sales::po_number::set_reference(t.state(), order.id, "PO-ALN-6000", "")
        .await
        .unwrap();
    assert!(found(t.db(), "PO-ALN-6000", &subject).await);
    let groups = related::related(t.db(), &everyone(), &subject, 1)
        .await
        .unwrap();
    assert_eq!(count(&groups, "returns"), Some(0));
    assert_eq!(count(&groups, "customer"), Some(1));
}

#[renox::test]
async fn a_counter_sale_its_return_a_lead_and_a_package_are_indexed_with_their_relations() {
    let t = boot().await;
    let clerk = t
        .user(
            "Clara",
            &[perm::POS_SELL, perm::RETURNS_REQUEST, perm::RETURNS_HANDLE],
        )
        .await;
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
        .unwrap();
    let sale_subject = Subject::new(COUNTER_SALE_SUBJECT_TYPE, sale.id);
    assert!(found(t.db(), &sale.number, &sale_subject).await);
    t.app
        .post(
            &format!("/counter-sales/{}/lines", sale.id),
            &[
                ("item_code", "tee"),
                ("unit_price", "50.00"),
                ("quantity", "2"),
            ],
        )
        .await
        .assert_status(303);
    t.app
        .post(
            &format!("/counter-sales/{}/complete", sale.id),
            &[("cash", "105.00")],
        )
        .await
        .assert_status(303);
    // `complete` (a transition) writes the new status.
    let document = search::document_of(t.db(), &sale_subject)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(document.status.as_deref(), Some("completed"));

    let line = wl_sales::sale_line::SaleLine::of(t.db(), &sale.subject())
        .await
        .unwrap()
        .remove(0);
    t.app
        .post(
            "/sales-returns",
            &[
                ("sale_type", "counter_sale"),
                ("sale_id", &sale.id.to_string()),
                ("lines", &format!("{}:1", line.id)),
                ("reason", "too small"),
            ],
        )
        .await
        .assert_status(303);
    let ret = SalesReturn::query()
        .order_by_desc(SalesReturn::ID)
        .first(t.db())
        .await
        .unwrap()
        .expect("the return was requested");
    let return_subject = Subject::new(SALES_RETURN_SUBJECT_TYPE, ret.id);
    assert!(found(t.db(), &ret.number, &return_subject).await);

    let access = everyone();
    let groups = related::related(t.db(), &access, &sale_subject, 1)
        .await
        .unwrap();
    assert_eq!(count(&groups, "returns"), Some(1));
    let groups = related::related(t.db(), &access, &return_subject, 1)
        .await
        .unwrap();
    assert_eq!(count(&groups, "counter_sale"), Some(1));
    assert_eq!(count(&groups, "order"), Some(0));

    // A lead, found by the contact's name in the text.
    let captured = wl_sales::lead::capture(
        t.state(),
        NewLead::web_form(None, "Omar Haddad").interest("Corporate fit-out"),
        wl_kernel::documents::today(),
    )
    .await
    .unwrap()
    .expect("the lead is captured");
    let Captured::Created(lead) = captured else {
        panic!("a new lead");
    };
    let document = search::document_of(t.db(), &Subject::new(LEAD_SUBJECT_TYPE, lead.id))
        .await
        .unwrap()
        .expect("the lead is indexed");
    assert_eq!(document.title, "Omar Haddad");
    assert!(document.secondary.contains("Corporate fit-out"));

    // A package, with its customer and invoice.
    let package = packages::sell(
        t.state(),
        NewPackage::new(PartyId(31), "PT-10", 10)
            .price(150_000)
            .invoice(InvoiceId(55)),
    )
    .await
    .unwrap();
    let package_subject = Subject::new(PACKAGE_SUBJECT_TYPE, package.id);
    assert!(
        search::document_of(t.db(), &package_subject)
            .await
            .unwrap()
            .is_some()
    );
    let groups = related::related(t.db(), &access, &package_subject, 1)
        .await
        .unwrap();
    assert_eq!(count(&groups, "customer"), Some(1));
    assert_eq!(count(&groups, "invoice"), Some(1));
    assert_eq!(count(&groups, "movements"), Some(0));
}
