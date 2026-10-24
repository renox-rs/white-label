//! BL-AGT-05 (#545) in the real Property host: the portal search finds a
//! tenant's own invoices by number, never another tenant's, and never an
//! internal record that mentions their invoice.

mod support;

use host::roles::{ACCOUNTANT, MEMBER};
use renox::prelude::*;
use renox::testing::TestApp;
use support::*;
use wl_invoicing::Invoice;
use wl_kernel::documents::today;
use wl_kernel::invoicing::VatCategory;
use wl_kernel::tickets::OpenTicket;
use wl_kernel::{PartyId, Subject};
use wl_portal::{NewGrant, PortalAccess};
use wl_ticketing::Tickets;

/// An issued invoice of AED 1,000 to `party`.
async fn invoice_of(app: &TestApp, accountant: &User, party: PartyId) -> Invoice {
    issued_invoice(
        app,
        accountant,
        party,
        100_000,
        VatCategory::Exempt,
        today() + renox::chrono::Days::new(30),
    )
    .await
}

/// A portal user with an `active` tenant grant for `party`.
async fn portal_user(app: &TestApp, name: &str, party: PartyId) -> User {
    let user = person(app, name, MEMBER).await;
    PortalAccess::register(
        app.state(),
        user.id,
        NewGrant::new(party, party, "tenant", &["primary_contact"]),
    )
    .await
    .unwrap();
    user
}

fn search_url(query: &str) -> String {
    format!("/portal/search?q={}", query.replace(' ', "+"))
}

#[renox::test]
async fn a_tenant_finds_their_own_invoice_and_nothing_of_anyone_else() {
    let app = booted().await;
    let accountant = person(&app, "Omar", ACCOUNTANT).await;
    let tenant_a = company(&app, "Al Noor Trading LLC").await;
    let tenant_b = company(&app, "Blue Dune Services LLC").await;
    let own = invoice_of(&app, &accountant, tenant_a).await;
    let other = invoice_of(&app, &accountant, tenant_b).await;
    let own_number = own.number.clone().expect("issued");
    let other_number = other.number.clone().expect("issued");
    // An internal ticket that mentions tenant A's invoice: tickets are not
    // portal-visible.
    Tickets::open_at(
        app.state(),
        OpenTicket::new(
            Subject::new("property.unit", 12),
            "general",
            &format!("Chase {own_number} by phone"),
        ),
        renox::db::now(),
    )
    .await
    .unwrap();

    let sara = portal_user(&app, "Sara", tenant_a).await;
    app.acting_as(&sara);

    // The home page has the search box.
    app.get(&format!("/portal/{}", tenant_a.0))
        .await
        .assert_ok()
        .assert_see("wl-portal-home-q");

    // Tenant B's invoice number finds nothing.
    app.get(&search_url(&other_number))
        .await
        .assert_ok()
        .assert_see("Nothing found")
        .assert_dont_see("data-result");

    // Her own invoice number opens it in the portal, also typed loosely.
    app.get(&search_url(&own_number))
        .await
        .assert_redirect(&format!("/portal/{}/invoices", tenant_a.0));
    app.get(&search_url(&own_number.replace('-', " ").to_lowercase()))
        .await
        .assert_redirect(&format!("/portal/{}/invoices", tenant_a.0));

    // The internal ticket mentioning her invoice is never found.
    app.get(&search_url("Chase phone"))
        .await
        .assert_ok()
        .assert_see("Nothing found")
        .assert_dont_see("data-result");
    let hits = wl_portal::search::search_for(app.state(), sara.id, &own_number)
        .await
        .unwrap();
    assert!(
        hits.iter()
            .all(|h| h.subject.subject_type == "invoicing.invoice" && h.party_id == tenant_a.0),
        "{hits:?}"
    );
}

#[renox::test]
async fn a_contact_of_two_companies_finds_the_invoices_of_each_grouped_by_company() {
    let app = booted().await;
    let accountant = person(&app, "Omar", ACCOUNTANT).await;
    let first = company(&app, "Al Noor Trading LLC").await;
    let second = company(&app, "Blue Dune Services LLC").await;
    let stranger = company(&app, "Cedar Hill LLC").await;
    let a = invoice_of(&app, &accountant, first).await;
    let b = invoice_of(&app, &accountant, second).await;
    let c = invoice_of(&app, &accountant, stranger).await;
    let user = portal_user(&app, "Huda", first).await;
    PortalAccess::register(
        app.state(),
        user.id,
        NewGrant::new(first, second, "tenant", &["primary_contact"]),
    )
    .await
    .unwrap();
    app.acting_as(&user);
    let page = app.get(&search_url("INV")).await;
    page.assert_ok()
        .assert_see("Al Noor Trading LLC")
        .assert_see("Blue Dune Services LLC")
        .assert_see(a.number.as_deref().unwrap())
        .assert_see(b.number.as_deref().unwrap())
        .assert_dont_see(c.number.as_deref().unwrap())
        .assert_dont_see("Cedar Hill LLC");
}

#[renox::test]
async fn a_user_without_a_portal_grant_may_not_open_the_portal_search() {
    let app = booted().await;
    let staff = person(&app, "Omar", ACCOUNTANT).await;
    app.acting_as(&staff);
    app.get(&search_url("INV")).await.assert_forbidden();
}
