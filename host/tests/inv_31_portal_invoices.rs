//! BL-INV-31 (#465) in the real Property host: the tenant's invoices and
//! statement of account in the portal, with the dispute button.
//!
//! Fixtures: the invoice is issued today and its issue date then set in
//! October 2026; the payment is an allocation of a payment subject whose
//! date is set in October 2026 too, so the statement's month is fixed.

mod support;

use host::roles::{ACCOUNTANT, MEMBER};
use renox::chrono::{NaiveDate, NaiveTime};
use renox::prelude::*;
use renox::testing::TestApp;
use support::*;
use wl_invoicing::{Dispute, Invoice};
use wl_kernel::documents::today;
use wl_kernel::invoicing::VatCategory;
use wl_kernel::payments::PAYMENT_SUBJECT_TYPE;
use wl_kernel::{BusinessCalendar, PartyId, Subject};
use wl_portal::{NewGrant, PortalAccess};

const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-properties/client.toml"
);

fn october(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
}

/// The Property host with the portal disputes switched off.
async fn booted_without_disputes() -> TestApp {
    let config = wl_kernel::ClientConfig::load(CLIENT).expect("the Property client");
    let tuning = host::Tuning::default().invoicing(|m| m.portal_disputes(false));
    let app = TestApp::new(
        host::app_for_tuned(config, tuning)
            .expect("the app")
            .provide(noon_clock()),
    )
    .await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// Sara, a portal user with an `active` tenant grant for `party`.
async fn sara(app: &TestApp, party: PartyId) -> (User, i64) {
    let user = person(app, "Sara", MEMBER).await;
    let grant = PortalAccess::register(
        app.state(),
        user.id,
        NewGrant::new(party, party, "tenant", &["primary_contact"]),
    )
    .await
    .unwrap();
    (user, grant.id)
}

/// Al Noor Trading LLC with an issued invoice of AED 4,500 dated
/// 3 October 2026.
async fn tenant_with_invoice(app: &TestApp) -> (PartyId, Invoice) {
    let party = company(app, "Al Noor Trading LLC").await;
    let accountant = person(app, "Omar", ACCOUNTANT).await;
    let invoice = issued_invoice(
        app,
        &accountant,
        party,
        450_000,
        VatCategory::Exempt,
        today() + renox::chrono::Days::new(30),
    )
    .await;
    Invoice::query()
        .where_(Invoice::ID.eq(invoice.id))
        .update(app.db(), &[("issue_date", &Some(october(3)))])
        .await
        .unwrap();
    let invoice = reload(app, &invoice).await;
    (party, invoice)
}

fn dispute_path(party: PartyId, invoice: &Invoice) -> String {
    format!("/portal/{}/invoices/{}/dispute", party.0, invoice.id)
}

#[renox::test]
async fn sara_sees_the_invoice_and_disputes_part_of_it() {
    let app = booted().await;
    let (party, invoice) = tenant_with_invoice(&app).await;
    assert_eq!(invoice.gross_total, 450_000);
    let number = invoice.number.clone().expect("a number");
    let (sara, _) = sara(&app, party).await;
    app.acting_as(&sara);

    let page = app.get(&format!("/portal/{}/invoices", party.0)).await;
    page.assert_ok()
        .assert_see(&number)
        .assert_see("AED 4,500.00")
        .assert_see("Issued")
        .assert_see("Dispute");

    app.htmx()
        .post(
            &dispute_path(party, &invoice),
            &[("reason", "parking charged twice"), ("amount", "1,500")],
        )
        .await
        .assert_ok();
    let invoice = reload(&app, &invoice).await;
    assert!(invoice.disputed, "the dispute flag is set: reminders stop");
    let dispute = Dispute::find_or_404(app.db(), invoice.dispute_id.expect("a dispute"))
        .await
        .unwrap();
    assert_eq!(dispute.amount, 150_000);
    assert_eq!(dispute.reason, "parking charged twice");
    assert_eq!(dispute.raised_via, wl_invoicing::DisputeVia::Portal);
    assert_eq!(dispute.raised_by_party_id, Some(party.0));
    assert!(
        dispute.ticket_id.is_some(),
        "a billing dispute ticket is opened"
    );
    assert_eq!(
        messages_to_party(&app, wl_invoicing::dispute::ACKNOWLEDGED, party)
            .await
            .len(),
        1,
        "she gets the acknowledgement"
    );
    // The page shows it disputed, without a second Dispute action.
    app.get(&format!("/portal/{}/invoices", party.0))
        .await
        .assert_ok()
        .assert_see("Disputed");
}

#[renox::test]
async fn another_party_sees_nothing_of_the_invoice() {
    let app = booted().await;
    let (party, invoice) = tenant_with_invoice(&app).await;
    let other = company(&app, "Other Tenant LLC").await;
    let (sara, _) = sara(&app, other).await;
    app.acting_as(&sara);
    app.get(&format!("/portal/{}/invoices", party.0))
        .await
        .assert_forbidden();
    app.get(&format!("/portal/{}/invoices", other.0))
        .await
        .assert_ok()
        .assert_dont_see(invoice.number.as_deref().unwrap());
    let refused = app
        .htmx()
        .post(
            &format!("/portal/{}/invoices/{}/dispute", other.0, invoice.id),
            &[("reason", "not mine"), ("amount", "10")],
        )
        .await;
    assert_eq!(refused.status.as_u16(), 404);
}

#[renox::test]
async fn with_the_setting_off_there_is_no_dispute_button_and_a_post_is_refused() {
    let app = booted_without_disputes().await;
    let (party, invoice) = tenant_with_invoice(&app).await;
    let (sara, _) = sara(&app, party).await;
    app.acting_as(&sara);
    app.get(&format!("/portal/{}/invoices", party.0))
        .await
        .assert_ok()
        .assert_see(invoice.number.as_deref().unwrap())
        .assert_dont_see("Send the dispute");
    app.htmx()
        .post(
            &dispute_path(party, &invoice),
            &[("reason", "parking charged twice"), ("amount", "1,500")],
        )
        .await
        .assert_forbidden();
    assert!(!reload(&app, &invoice).await.disputed);
}

#[renox::test]
async fn the_october_statement_runs_from_zero_to_the_balance_left() {
    let app = booted().await;
    let (party, invoice) = tenant_with_invoice(&app).await;
    wl_invoicing::allocate::allocate(
        app.state(),
        &invoice,
        &Subject::new(PAYMENT_SUBJECT_TYPE, 9_001),
        300_000,
        None,
    )
    .await
    .unwrap();
    let paid_at = BusinessCalendar::default()
        .from_local(october(12).and_time(NaiveTime::from_hms_opt(12, 0, 0).unwrap()));
    wl_invoicing::allocation::Allocation::query()
        .where_(wl_invoicing::allocation::Allocation::INVOICE_ID.eq(invoice.id))
        .update(app.db(), &[("allocated_at", &paid_at)])
        .await
        .unwrap();

    let statement = wl_invoicing::portal::statement_of(app.db(), party, october(1), october(31))
        .await
        .unwrap();
    assert_eq!(statement.opening, 0);
    assert_eq!(statement.lines.len(), 2);
    assert_eq!(statement.lines[0].movement.debit, 450_000);
    assert_eq!(statement.lines[1].movement.credit, 300_000);
    assert_eq!(statement.closing, 150_000);

    let (sara, _) = sara(&app, party).await;
    app.acting_as(&sara);
    app.get(&format!(
        "/portal/{}/invoices/statement?from=2026-10-01&to=2026-10-31",
        party.0
    ))
    .await
    .assert_ok()
    .assert_see("Statement of account")
    .assert_see("AED 0.00")
    .assert_see("AED 4,500.00")
    .assert_see("AED 3,000.00")
    .assert_see("AED 1,500.00");
}

#[renox::test]
async fn a_blocked_tenant_can_still_dispute_and_sees_no_block_text() {
    let app = booted().await;
    let (party, invoice) = tenant_with_invoice(&app).await;
    block(&app, party).await;
    let (sara, _) = sara(&app, party).await;
    app.acting_as(&sara);
    let page = app.get(&format!("/portal/{}/invoices", party.0)).await;
    page.assert_ok()
        .assert_dont_see("is blocked")
        .assert_dont_see("non_payment");
    app.htmx()
        .post(
            &dispute_path(party, &invoice),
            &[("reason", "parking charged twice"), ("amount", "1,500")],
        )
        .await
        .assert_ok();
    assert!(reload(&app, &invoice).await.disputed);
}

#[renox::test]
async fn a_read_only_grant_reads_the_invoices_but_cannot_dispute() {
    let app = booted().await;
    let (party, invoice) = tenant_with_invoice(&app).await;
    let (sara, grant) = sara(&app, party).await;
    PortalAccess::end(app.state(), None, grant, Some("lease terminated"))
        .await
        .unwrap();
    app.acting_as(&sara);
    app.get(&format!("/portal/{}/invoices", party.0))
        .await
        .assert_ok()
        .assert_see(invoice.number.as_deref().unwrap())
        .assert_dont_see("Send the dispute");
    app.get(&format!("/portal/{}/invoices/statement", party.0))
        .await
        .assert_ok();
    app.htmx()
        .post(
            &dispute_path(party, &invoice),
            &[("reason", "parking charged twice"), ("amount", "1,500")],
        )
        .await
        .assert_forbidden();
    assert!(!reload(&app, &invoice).await.disputed);
}
