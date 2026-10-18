//! The receivables ageing page renders for a user with `receivables.view`
//! and is closed to everyone else (#67 part 5).

use renox::chrono::Days;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_invoicing::{Dispute, Invoice};
use wl_kernel::documents::today;
use wl_kernel::invoicing::InvoiceStatus;
use wl_parties::Party;

use host::roles::{self, MEMBER, RECEIVABLES};

/// A verified user with `role`.
async fn person(app: &TestApp, email: &str, role: &str) -> User {
    let mut user = User::register(app.db(), "Rita", email, "secret123")
        .await
        .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    roles::define(app.db()).await.unwrap();
    user.assign_role(app.db(), role).await.unwrap();
    user
}

#[renox::test]
async fn the_ageing_page_shows_the_buckets_and_the_disputed_column() {
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();

    let member = person(&app, "ageing.member@test.example", MEMBER).await;
    app.acting_as(&member);
    app.get("/receivables/ageing").await.assert_forbidden();

    let clerk = person(&app, "ageing.clerk@test.example", RECEIVABLES).await;
    app.acting_as(&clerk);
    app.get("/receivables/ageing")
        .await
        .assert_ok()
        .assert_see("Receivables ageing")
        .assert_see("0-30, 31-60, 61-90, 90+");

    // An invoice of AED 5,000.00, 40 days past due, of which AED 1,200.00
    // is disputed.
    let party = Party::query()
        .first(app.db())
        .await
        .unwrap()
        .expect("a party");
    let mut invoice = Invoice {
        party_id: party.id,
        currency: "AED".into(),
        due_date: today().checked_sub_days(Days::new(40)),
        gross_total: 500_000,
        balance: 500_000,
        status: InvoiceStatus::Issued,
        disputed: true,
        ..Default::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    invoice
        .insert_in_state(&mut tx, InvoiceStatus::Issued, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut dispute = Dispute {
        invoice_id: invoice.id,
        reason: "wrong hours".into(),
        amount: 120_000,
        ..Default::default()
    };
    dispute.insert(app.db()).await.unwrap();
    invoice.dispute_id = Some(dispute.id);
    invoice.save_only(app.db(), &["dispute_id"]).await.unwrap();

    app.get("/receivables/ageing")
        .await
        .assert_ok()
        .assert_see("31-60")
        .assert_see("Disputed")
        .assert_see("AED 3,800.00")
        .assert_see("AED 1,200.00")
        .assert_see("AED 5,000.00");
    // Another entity has none of it.
    app.get("/receivables/ageing?entity=999999")
        .await
        .assert_ok()
        .assert_dont_see("AED 5,000.00");
}
