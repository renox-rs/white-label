//! The global search and the Related panel wired in the host (BL-AGT-02,
//! BL-AGT-03): the box in the top bar of every signed-in page (`mod+k`,
//! turned off with the reason for a role that may search nothing), the
//! search page opening an invoice by its number, and the invoice page
//! loading its Related panel.
//!
//! The invoice type is registered for search here as its module will
//! (BL-AGT-04); its relations are the ones wl-invoicing declares.

use renox::prelude::*;
use renox::testing::TestApp;
use wl_invoicing::Invoice;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory, invoicer};
use wl_kernel::search::{self, SearchDocument, Searchable, kinds};
use wl_kernel::{PartyId, Subject};
use wl_parties::Party;

use host::roles::{self, ACCOUNTANT, MEMBER, RECEIVABLES};

/// A verified user with `role`.
async fn person(app: &TestApp, name: &str, role: &str) -> User {
    let mut user = User::register(
        app.db(),
        name,
        &format!("{}@search.example", name.to_lowercase()),
        "secret123",
    )
    .await
    .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    roles::define(app.db()).await.unwrap();
    user.assign_role(app.db(), role).await.unwrap();
    user
}

#[renox::test]
async fn the_top_bar_has_a_search_box_turned_off_for_a_role_that_may_search_nothing() {
    let app = TestApp::new(host::app()).await;
    let member = person(&app, "Maya", MEMBER).await;
    app.acting_as(&member);
    let page = app.get("/dashboard").await;
    page.assert_ok()
        .assert_see("id=\"wl-search-q\"")
        .assert_see("action=\"/search\"")
        .assert_see("role=\"search\"")
        .assert_see("Your role gives access to no record that can be searched")
        .assert_see("search.js");
    let page = app.get("/search?q=INV").await;
    page.assert_ok()
        .assert_see("Your role gives access to no record that can be searched.");
}

#[renox::test]
async fn an_invoice_number_opens_the_invoice_and_its_page_loads_the_related_panel() {
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    // As wl-invoicing will declare it (BL-AGT-04).
    Searchable::of::<Invoice>("invoicing.invoice")
        .permission(wl_invoicing::permissions::VIEW)
        .label("Invoices")
        .link("/invoices/{id}")
        .document(|invoice: &Invoice| {
            let number = invoice.number.clone().unwrap_or_default();
            SearchDocument::new(number.clone())
                .identifier(kinds::NUMBER, number)
                .status(invoice.status.to_string())
        })
        .register();

    let party = Party::query()
        .first(app.db())
        .await
        .unwrap()
        .expect("a party");
    let draft = InvoiceDraft::new(PartyId(party.id), Subject::new("property.lease", 11))
        .line(DraftLine::new("Rent", 100_000, VatCategory::Standard));
    let id = invoicer(app.state())
        .draft(app.db(), draft)
        .await
        .unwrap()
        .expect("invoicing is on");
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    app.acting_as(&clerk);
    app.htmx()
        .post(&format!("/invoices/{}/issue", id.0), &[])
        .await
        .assert_redirect(&format!("/invoices/{}", id.0));
    search::rebuild_type(app.db(), "invoicing.invoice")
        .await
        .unwrap();
    let invoice = Invoice::find_or_404(app.db(), id.0).await.unwrap();
    let number = invoice.number.clone().expect("an issued invoice has a number");

    let accountant = person(&app, "Adam", ACCOUNTANT).await;
    app.acting_as(&accountant);
    // The box is on for a role that may search invoices.
    let page = app.get("/dashboard").await;
    page.assert_ok()
        .assert_see("Search by number, name or words")
        .assert_dont_see("Your role gives access to no record");
    // The number, typed in lower case with spaces, opens the invoice.
    let typed = number.to_lowercase().replace('-', "+");
    app.get(&format!("/search?q={typed}"))
        .await
        .assert_redirect(&format!("/invoices/{}", invoice.id));
    // The invoice page loads its Related panel, which lists its line.
    app.get(&format!("/invoices/{}", invoice.id))
        .await
        .assert_ok()
        .assert_see(&format!(
            "hx-get=\"/related/invoicing.invoice:{}/panel\"",
            invoice.id
        ));
    app.get(&format!("/related/invoicing.invoice:{}/panel", invoice.id))
        .await
        .assert_ok()
        .assert_see("data-group=\"lines\" data-count=\"1\"")
        .assert_see("data-group=\"credit_notes\" data-count=\"0\"");

    // A member finds nothing and cannot open the panel.
    let member = person(&app, "Maya", MEMBER).await;
    app.acting_as(&member);
    let page = app.get(&format!("/search?q={typed}")).await;
    page.assert_ok().assert_dont_see(&number);
    app.get(&format!("/related/invoicing.invoice:{}/panel", invoice.id))
        .await
        .assert_not_found();
}
