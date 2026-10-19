//! The dispute sheets, the dispute panel and the "Disputed" badge render and
//! post for a user with `invoices.dispute`, `credited` also needs
//! `invoices.credit`, and the pages show nothing to others (#67 part 6).

use renox::auth::permissions;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_invoicing::permissions as perm;
use wl_invoicing::{Dispute, Invoice};
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory, invoicer};
use wl_kernel::{PartyId, Subject};
use wl_parties::Party;

use host::roles::{self, MEMBER, RECEIVABLES};

/// A verified user with `role`.
async fn person(app: &TestApp, name: &str, role: &str) -> User {
    let mut user = User::register(
        app.db(),
        name,
        &format!("{}@test.example", name.to_lowercase()),
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

/// An issued invoice of two lines: AED 1,050.00 and AED 525.00.
async fn issued(app: &TestApp, party: PartyId, clerk: &User) -> Invoice {
    let draft = InvoiceDraft::new(party, Subject::new("property.lease", 31))
        .line(DraftLine::new("Rent", 100_000, VatCategory::Standard))
        .line(DraftLine::new("Parking", 50_000, VatCategory::Standard));
    let id = invoicer(app.state())
        .draft(app.db(), draft)
        .await
        .unwrap()
        .expect("invoicing is on");
    app.acting_as(clerk);
    app.htmx()
        .post(&format!("/invoices/{}/issue", id.0), &[])
        .await
        .assert_redirect(&format!("/invoices/{}", id.0));
    Invoice::find_or_404(app.db(), id.0).await.unwrap()
}

#[renox::test]
async fn the_dispute_sheets_panel_and_badge_follow_the_dispute_permission() {
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    let party = Party::query()
        .first(app.db())
        .await
        .unwrap()
        .expect("a party");
    let party = PartyId(party.id);
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let invoice = issued(&app, party, &clerk).await;
    let page = format!("/invoices/{}", invoice.id);
    let dispute_url = format!("{page}/dispute");
    let resolve_url = format!("{page}/dispute/resolve");

    // Members cannot post; a viewer sees no dispute action.
    let member = person(&app, "Maya", MEMBER).await;
    app.acting_as(&member);
    app.htmx()
        .post(&dispute_url, &[("reason", "No")])
        .await
        .assert_forbidden();
    app.htmx()
        .post(&resolve_url, &[("outcome", "upheld"), ("note", "No")])
        .await
        .assert_forbidden();
    let viewer = User::register(app.db(), "Vera", "vera@test.example", "secret123")
        .await
        .unwrap();
    permissions::define_role(app.db(), "dispute_viewer", &[perm::VIEW])
        .await
        .unwrap();
    viewer
        .assign_role(app.db(), "dispute_viewer")
        .await
        .unwrap();
    app.acting_as(&viewer);
    app.get(&page)
        .await
        .assert_ok()
        .assert_dont_see("Dispute the invoice");
    app.htmx()
        .post(&dispute_url, &[("reason", "No")])
        .await
        .assert_forbidden();

    // The clerk sees the sheet; refusals answer 422.
    app.acting_as(&clerk);
    app.get(&page)
        .await
        .assert_ok()
        .assert_see("Dispute the invoice")
        .assert_dont_see("Resolve the dispute");
    app.htmx()
        .post(&dispute_url, &[("amount", "10.00"), ("reason", " ")])
        .await
        .assert_status(422);
    app.htmx()
        .post(&dispute_url, &[("reason", "Hours")])
        .await
        .assert_status(422);
    app.htmx()
        .post(&dispute_url, &[("reason", "Hours"), ("amount", "ten")])
        .await
        .assert_status(422);
    app.htmx()
        .post(&dispute_url, &[("reason", "Hours"), ("line_9", "1")])
        .await
        .assert_status(422);

    // Ticking the second line disputes its gross, AED 525.00.
    app.htmx()
        .post(
            &dispute_url,
            &[("reason", "Wrong parking"), ("line_2", "on")],
        )
        .await
        .assert_ok();
    let flagged = Invoice::find_or_404(app.db(), invoice.id).await.unwrap();
    assert!(flagged.disputed);
    let dispute = Dispute::find(app.db(), flagged.dispute_id.expect("dispute id"))
        .await
        .unwrap()
        .expect("the dispute");
    assert_eq!(dispute.amount, 52_500);
    app.get(&page)
        .await
        .assert_ok()
        .assert_see("Wrong parking")
        .assert_see("AED 525.00")
        .assert_see("Disputed")
        .assert_see("TKT-")
        .assert_see("Resolve the dispute")
        .assert_dont_see("Dispute the invoice");
    app.get("/invoices")
        .await
        .assert_ok()
        .assert_see("Disputed");
    // Already disputed.
    app.htmx()
        .post(&dispute_url, &[("reason", "Again"), ("amount", "1.00")])
        .await
        .assert_status(422);

    // A user who may dispute but not credit: `credited` is refused with 403.
    permissions::define_role(app.db(), "dispute_only", &[perm::VIEW, perm::DISPUTE])
        .await
        .unwrap();
    let agent = User::register(app.db(), "Dina", "dina@test.example", "secret123")
        .await
        .unwrap();
    agent.assign_role(app.db(), "dispute_only").await.unwrap();
    app.acting_as(&agent);
    app.get(&page)
        .await
        .assert_ok()
        .assert_see("Resolve the dispute")
        .assert_dont_see("Credited: issue a credit note");
    app.htmx()
        .post(
            &resolve_url,
            &[
                ("outcome", "credited"),
                ("agreed_amount", "100.00"),
                ("note", "Agreed"),
            ],
        )
        .await
        .assert_forbidden();
    app.htmx()
        .post(&resolve_url, &[("outcome", "upheld"), ("note", " ")])
        .await
        .assert_status(422);
    app.htmx()
        .post(&resolve_url, &[("outcome", "maybe"), ("note", "x")])
        .await
        .assert_status(422);
    app.htmx()
        .post(
            &resolve_url,
            &[("outcome", "upheld"), ("note", "The hours are right")],
        )
        .await
        .assert_ok();
    let cleared = Invoice::find_or_404(app.db(), invoice.id).await.unwrap();
    assert!(!cleared.disputed);
    app.get(&page)
        .await
        .assert_ok()
        .assert_dont_see("Resolve the dispute")
        .assert_see("Dispute the invoice");
}
