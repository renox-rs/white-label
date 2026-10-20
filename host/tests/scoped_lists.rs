//! The module-wide lists for a user who holds the permission in some
//! buildings only (docs/ACCESS-CONTROL.md §3, layer 3): they show the records
//! of those buildings, never another building's, and answer 403 only when the
//! permission is held nowhere. The sidebar links lead to pages that open.

mod support;

use host::roles::{ACCOUNTANT, MEMBER, PROPERTY_MANAGER, RECEIVABLES};
use property::UnitUse;
use renox::prelude::*;
use renox::testing::TestApp;
use support::*;
use wl_kernel::PartyId;
use wl_kernel::cheques::{RegisterCheque, cheques};
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory, invoicer};

/// What one building has: its unit, a draft lease for its own tenant, an
/// issued invoice billed for the building, a cheque and a deposit of the
/// lease, a ticket about the building, a management agreement with the unit
/// and an enquiry about it.
struct Tower {
    building: property::Building,
    tenant: PartyId,
    invoice: wl_invoicing::Invoice,
    cheque: wl_payments::Cheque,
    deposit: wl_payments::Deposit,
    ticket: wl_ticketing::Ticket,
    agreement: property::agreement::ManagementAgreement,
}

async fn tower(app: &TestApp, code: &str, tenant_name: &str, owner_name: &str) -> Tower {
    let accountant = person(app, &format!("Amal{code}"), ACCOUNTANT).await;
    let clerk = person(app, &format!("Clara{code}"), RECEIVABLES).await;
    let unit = unit_of(app, code, "101", UnitUse::Residential).await;
    let building = property::Building::find_or_404(app.db(), unit.building_id)
        .await
        .unwrap();
    let tenant = tenant(app, tenant_name).await;
    let lease = draft_lease(app, &unit, tenant, -30).await;

    // An invoice billed for the building.
    let draft = InvoiceDraft::new(tenant, lease.subject())
        .scope(building.subject())
        .line(DraftLine::new("Rent", 100_000, VatCategory::Exempt))
        .due_date(day(0));
    let id = invoicer(app.state())
        .draft(app.db(), draft)
        .await
        .unwrap()
        .expect("invoicing is on");
    app.acting_as(&clerk);
    app.htmx()
        .post(&format!("/invoices/{}/issue", id.0), &[])
        .await
        .assert_redirect(&format!("/invoices/{}", id.0));
    let invoice = wl_invoicing::Invoice::find_or_404(app.db(), id.0)
        .await
        .unwrap();

    // A cheque received with the lease.
    let entity = entity(app, "PGP").await;
    let request = RegisterCheque::new(
        tenant,
        entity.id,
        "ENBD",
        &format!("{code}-1"),
        day(30),
        100_000,
    )
    .agreement(lease.subject());
    let summary = cheques(app.state())
        .register(app.state(), request, Some(accountant.id))
        .await
        .expect("the cheque is registered");
    let cheque = wl_payments::Cheque::find_or_404(app.db(), summary.id.0)
        .await
        .unwrap();

    let deposit = settling_deposit(app, &accountant, tenant, &lease, 500_000).await;

    let ticket = vendor_ticket(
        app,
        &building.subject(),
        "general",
        company(app, &format!("Vendor {code}")).await,
        None,
    )
    .await;

    let owner = self::tenant(app, owner_name).await;
    let agreement = agreement_of(app, owner).await;
    property::agreement::add_unit(app.db(), &agreement, unit.id, day(-100))
        .await
        .unwrap();
    property::enquiry::record(
        app.db(),
        property::enquiry::NewEnquiry {
            prospect_name: format!("Prospect {code}"),
            contact: "050 000 0000".into(),
            unit_id: Some(unit.id),
            source: "walk_in".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    Tower {
        building,
        tenant,
        invoice,
        cheque,
        deposit,
        ticket,
        agreement,
    }
}

#[renox::test]
async fn a_manager_of_one_building_sees_the_lists_of_that_building() {
    let app = booted().await;
    let a = tower(&app, "SL-A", "Tara Alpha", "Olga Alpha").await;
    let b = tower(&app, "SL-B", "Tariq Beta", "Omar Beta").await;
    let manager = scoped_person(&app, "Pam", PROPERTY_MANAGER, &a.building.scope()).await;
    app.acting_as(&manager);

    // Every list opens, with this building's records and not the other's.
    let rows = [
        (
            "/invoices",
            format!("/invoices/{}", a.invoice.id),
            format!("/invoices/{}", b.invoice.id),
        ),
        (
            "/deposits",
            format!("/deposits/{}", a.deposit.id),
            format!("/deposits/{}", b.deposit.id),
        ),
        (
            "/cheques",
            format!("/cheques/{}", a.cheque.id),
            format!("/cheques/{}", b.cheque.id),
        ),
        (
            "/tickets",
            format!("/tickets/{}", a.ticket.id),
            format!("/tickets/{}", b.ticket.id),
        ),
        (
            "/parties",
            format!("/parties/{}", a.tenant.0),
            format!("/parties/{}", b.tenant.0),
        ),
        (
            "/property/agreements",
            format!("/property/agreements/{}", a.agreement.id),
            format!("/property/agreements/{}", b.agreement.id),
        ),
    ];
    for (list, mine, theirs) in &rows {
        app.get(list)
            .await
            .assert_ok()
            .assert_see(&format!("href=\"{mine}\""))
            .assert_dont_see(&format!("href=\"{theirs}\""));
        // The page of a record of this building opens, the other building's
        // does not.
        if !list.starts_with("/property") {
            app.get(mine).await.assert_ok();
            app.get(theirs).await.assert_forbidden();
        }
    }
    app.get(&format!("/property/agreements/{}", a.agreement.id))
        .await
        .assert_ok();
    app.get(&format!("/property/agreements/{}", b.agreement.id))
        .await
        .assert_forbidden();

    // The deposit's payment is the building's payment.
    let paid = a.deposit.received_payment_id.expect("the deposit was paid");
    let other = b.deposit.received_payment_id.expect("the deposit was paid");
    app.get("/payments")
        .await
        .assert_ok()
        .assert_see(&format!("href=\"/payments/{paid}\""))
        .assert_dont_see(&format!("href=\"/payments/{other}\""));
    app.get(&format!("/payments/{paid}")).await.assert_ok();
    app.get(&format!("/payments/{other}"))
        .await
        .assert_forbidden();

    // The enquiries about this building's units; the other building's are not.
    app.get("/property/enquiries")
        .await
        .assert_ok()
        .assert_see("Prospect SL-A")
        .assert_dont_see("Prospect SL-B");

    // The other lists open (they may be empty).
    for path in [
        "/credit-notes",
        "/refunds",
        "/compliance-flags",
        "/documents/flags",
        "/documents/sops",
    ] {
        app.get(path).await.assert_ok();
    }
}

#[renox::test]
async fn the_sidebar_lists_what_the_scoped_manager_can_open() {
    let app = booted().await;
    let a = tower(&app, "SL-A", "Tara Alpha", "Olga Alpha").await;
    let manager = scoped_person(&app, "Pam", PROPERTY_MANAGER, &a.building.scope()).await;
    app.acting_as(&manager);
    let dashboard = app.get("/dashboard").await;
    dashboard.assert_ok();
    // Every link of the sidebar opens for the person who sees it.
    let html = dashboard.text();
    let mut links: Vec<String> = html
        .split("class=\"rx-sidebar__link\" href=\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next().map(str::to_owned))
        .collect();
    links.sort();
    links.dedup();
    assert!(links.len() > 8, "{links:?}");
    for link in ["/invoices", "/deposits", "/cheques", "/tickets", "/parties"] {
        assert!(links.iter().any(|l| l == link), "{link} in {links:?}");
    }
    for link in links {
        let page = app.get(&link).await;
        assert!(
            page.status.is_success(),
            "the sidebar shows {link} and it answers {}",
            page.status
        );
    }
}

#[renox::test]
async fn a_company_wide_role_sees_every_building_and_a_member_none() {
    let app = booted().await;
    let a = tower(&app, "SL-A", "Tara Alpha", "Olga Alpha").await;
    let b = tower(&app, "SL-B", "Tariq Beta", "Omar Beta").await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    app.acting_as(&accountant);
    app.get("/invoices")
        .await
        .assert_ok()
        .assert_see(&format!("href=\"/invoices/{}\"", a.invoice.id))
        .assert_see(&format!("href=\"/invoices/{}\"", b.invoice.id));
    app.get("/cheques")
        .await
        .assert_ok()
        .assert_see(&format!("href=\"/cheques/{}\"", a.cheque.id))
        .assert_see(&format!("href=\"/cheques/{}\"", b.cheque.id));

    // Nobody who holds the permission nowhere gets a list.
    let member = person(&app, "Mia", MEMBER).await;
    app.acting_as(&member);
    for path in [
        "/invoices",
        "/credit-notes",
        "/payments",
        "/refunds",
        "/deposits",
        "/cheques",
        "/tickets",
        "/parties",
        "/property/agreements",
        "/property/enquiries",
        "/documents/flags",
        "/documents/sops",
        "/compliance-flags",
    ] {
        app.get(path).await.assert_forbidden();
    }
    // A manager of tower B only gets nothing of tower A.
    let manager_b = scoped_person(&app, "Paul", PROPERTY_MANAGER, &b.building.scope()).await;
    app.acting_as(&manager_b);
    app.get(&format!("/invoices/{}", a.invoice.id))
        .await
        .assert_forbidden();
}
