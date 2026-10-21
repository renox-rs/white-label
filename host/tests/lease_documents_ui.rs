//! The lease path as people walk it in the browser, where it used to stop:
//! the documents a lease waits for are added on the record's documents page
//! (there was no form), the refusal says what is missing in words, the
//! deposit received is recorded from the lease, and the review queue names
//! whose document it is.

mod support;

use host::roles::{ACCOUNTANT, COMPLIANCE, LEASING_AGENT, MEMBER, PROPERTY_MANAGER};
use property::UnitUse;
use renox::prelude::*;
use renox::testing::TestApp;
use support::*;
use wl_documents::{Document, DocumentStatus};
use wl_kernel::PartyId;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

/// `by` adds a document of `kind` to `subject` on the documents page.
async fn upload(app: &TestApp, by: &User, subject: &str, kind: &str, expiry: Option<&str>) {
    app.acting_as(by);
    let mut fields = vec![("kind", kind), ("number", "N-1")];
    if let Some(expiry) = expiry {
        fields.push(("expiry_date", expiry));
    }
    app.post_multipart(
        &format!("/documents/of/{subject}"),
        &fields,
        &[("files", "scan.png", PNG)],
    )
    .await
    .assert_redirect(&format!("/documents/of/{subject}"));
}

/// `by` accepts every document waiting for review.
async fn accept_all(app: &TestApp, by: &User) {
    let waiting = Document::query()
        .where_(Document::STATUS.eq(DocumentStatus::PendingReview))
        .get(app.db())
        .await
        .unwrap();
    app.acting_as(by);
    for document in waiting {
        app.htmx()
            .post(&format!("/documents/{}/accept", document.id), &[])
            .await
            .assert_ok();
    }
}

struct Scene {
    app: TestApp,
    tenant: PartyId,
    lease: property::Lease,
    agent: User,
    manager: User,
    accountant: User,
    officer: User,
}

async fn scene() -> Scene {
    let app = booted().await;
    let unit = unit_of(&app, "TWR-D", "1204", UnitUse::Residential).await;
    let scope = property::Building::scope_of(unit.building_id);
    let agent = scoped_person(&app, "Ali", LEASING_AGENT, &scope).await;
    let manager = scoped_person(&app, "Mona", PROPERTY_MANAGER, &scope).await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let officer = person(&app, "Comply", COMPLIANCE).await;
    let tenant = tenant(&app, "Karim Haddad").await;
    let new =
        property::NewLease::new(unit.id, tenant, day(0), day(364), 9_600_000, 4).deposit(480_000);
    let lease = property::leases::create_draft(app.state(), None, new)
        .await
        .unwrap();
    Scene {
        app,
        tenant,
        lease,
        agent,
        manager,
        accountant,
        officer,
    }
}

#[renox::test]
async fn the_refusal_says_in_words_what_the_tenant_needs_and_the_page_links_to_it() {
    let s = scene().await;
    let submit = format!("/property/leases/{}/submit", s.lease.id);
    s.app.acting_as(&s.agent);
    let refused = s.app.htmx().post(&submit, &[]).await;
    refused.assert_status(422);
    let body = refused.text();
    assert!(
        body.contains("The tenant needs a valid Emirates ID, passport and residence visa on file."),
        "{body}"
    );
    // The guard's own line (the renewing role, the override mode) is for the log.
    assert!(
        !body.contains("override:") && !body.contains("renew:"),
        "{body}"
    );

    // The lease page tells it before the button is pressed, with the link.
    let tenant_documents = format!("/documents/of/parties.party/{}", s.tenant.0);
    s.app
        .get(&format!("/property/leases/{}", s.lease.id))
        .await
        .assert_ok()
        .assert_see("Documents needed")
        .assert_see("needs a valid Emirates ID, passport and residence visa")
        .assert_see(&format!("href=\"{tenant_documents}\""));

    // A document that is there but not valid is named for what is wrong.
    upload(
        &s.app,
        &s.agent,
        &format!("parties.party/{}", s.tenant.0),
        "passport",
        Some("2031-01-01"),
    )
    .await;
    upload(
        &s.app,
        &s.agent,
        &format!("parties.party/{}", s.tenant.0),
        "emirates_id",
        Some("2031-01-01"),
    )
    .await;
    upload(
        &s.app,
        &s.agent,
        &format!("parties.party/{}", s.tenant.0),
        "residence_visa",
        Some("2031-01-01"),
    )
    .await;
    s.app.acting_as(&s.agent);
    let waiting = s.app.htmx().post(&submit, &[]).await;
    waiting.assert_status(422);
    assert!(
        waiting.text().contains("not accepted yet"),
        "{}",
        waiting.text()
    );
}

#[renox::test]
async fn the_lease_goes_from_documents_to_active_through_the_pages() {
    let s = scene().await;
    let tenant_docs = format!("parties.party/{}", s.tenant.0);
    let lease_docs = format!("property.lease/{}", s.lease.id);
    let id = s.lease.id;

    // The agent adds the three documents; compliance accepts them (a
    // document is never accepted by the person who added it).
    for kind in ["emirates_id", "passport", "residence_visa"] {
        upload(&s.app, &s.agent, &tenant_docs, kind, Some("2031-01-01")).await;
    }
    s.app.acting_as(&s.agent);
    s.app
        .htmx()
        .post(&format!("/documents/{}/accept", 1), &[])
        .await
        .assert_forbidden();
    // The queue says whose documents they are, with the link.
    s.app.acting_as(&s.officer);
    s.app
        .get("/documents/review")
        .await
        .assert_ok()
        .assert_see("Karim Haddad")
        .assert_see(&format!("href=\"/documents/of/{tenant_docs}\""));
    accept_all(&s.app, &s.officer).await;

    s.app.acting_as(&s.agent);
    s.app
        .htmx()
        .post(&format!("/property/leases/{id}/submit"), &[])
        .await
        .assert_ok();

    // The manager adds the signed contract to the lease, scoped to their
    // building; the lease is named in the queue.
    upload(
        &s.app,
        &s.manager,
        &lease_docs,
        "signed_tenancy_contract",
        None,
    )
    .await;
    s.app.acting_as(&s.officer);
    s.app
        .get("/documents/review")
        .await
        .assert_see(&s.lease.number_with_version());
    accept_all(&s.app, &s.officer).await;

    // The deposit: refused to a member and to a manager of another
    // building, recorded by the accountant, then held.
    let deposit = format!("/property/leases/{id}/deposit");
    let member = person(&s.app, "Mia", MEMBER).await;
    s.app.acting_as(&member);
    s.app
        .htmx()
        .post(&deposit, &[("method", "transfer")])
        .await
        .assert_forbidden();
    let other = unit_of(&s.app, "TWR-E", "1", UnitUse::Residential).await;
    let stranger = scoped_person(
        &s.app,
        "Paul",
        PROPERTY_MANAGER,
        &property::Building::scope_of(other.building_id),
    )
    .await;
    s.app.acting_as(&stranger);
    s.app
        .htmx()
        .post(&deposit, &[("method", "transfer")])
        .await
        .assert_forbidden();

    // Without the deposit the activation waits, and says so.
    s.app.acting_as(&s.manager);
    s.app
        .htmx()
        .post(&format!("/property/leases/{id}/activate"), &[])
        .await
        .assert_status(422);
    s.app.acting_as(&s.accountant);
    s.app
        .get(&format!("/property/leases/{id}"))
        .await
        .assert_see("Record the deposit received");
    s.app
        .htmx()
        .post(&deposit, &[("method", "wire")])
        .await
        .assert_invalid("method");
    s.app
        .htmx()
        .post(
            &deposit,
            &[("method", "transfer"), ("reference", "TRF-KH-1")],
        )
        .await
        .assert_ok();
    let held = wl_kernel::deposits::deposits(s.app.state())
        .held_for(s.app.db(), &s.lease.agreement())
        .await
        .unwrap()
        .expect("the lease holds the deposit");
    assert_eq!((held.required, held.held), (480_000, 480_000));
    // Once, and the page no longer offers it.
    s.app
        .htmx()
        .post(&deposit, &[("method", "transfer")])
        .await
        .assert_status(422);
    s.app
        .get(&format!("/property/leases/{id}"))
        .await
        .assert_dont_see("Record the deposit received");

    s.app.acting_as(&s.manager);
    s.app
        .htmx()
        .post(&format!("/property/leases/{id}/activate"), &[])
        .await
        .assert_ok();
    let active = property::Lease::find_or_404(s.app.db(), id).await.unwrap();
    assert_eq!(active.status, property::LeaseStatus::Active);

    // The Ejari number waits for the certificate, and the refusal names it.
    let ejari = format!("/property/leases/{id}/ejari");
    let fields = [
        ("number", "EJ-1"),
        ("registered_on", "2026-10-20"),
        ("expires_on", "2027-10-19"),
    ];
    let refused = s.app.htmx().post(&ejari, &fields).await;
    refused.assert_status(422);
    assert!(
        refused
            .text()
            .contains("needs a valid Ejari / Tawtheeq certificate on file"),
        "{}",
        refused.text()
    );
    s.app
        .get(&format!("/property/leases/{id}"))
        .await
        .assert_see("Documents needed");
    upload(
        &s.app,
        &s.manager,
        &lease_docs,
        "ejari_certificate",
        Some("2027-10-19"),
    )
    .await;
    accept_all(&s.app, &s.officer).await;
    s.app.acting_as(&s.manager);
    s.app.htmx().post(&ejari, &fields).await.assert_ok();
}

#[renox::test]
async fn the_party_page_links_to_its_documents() {
    let s = scene().await;
    s.app.acting_as(&s.officer);
    s.app
        .get(&format!("/parties/{}", s.tenant.0))
        .await
        .assert_ok()
        .assert_see(&format!(
            "href=\"/documents/of/parties.party/{}\"",
            s.tenant.0
        ));
    // And the agent of the tenant's building reaches the same page.
    s.app.acting_as(&s.agent);
    s.app
        .get(&format!("/documents/of/parties.party/{}", s.tenant.0))
        .await
        .assert_ok()
        .assert_see("Karim Haddad");
}
