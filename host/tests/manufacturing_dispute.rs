//! BL-MFG-13 (#394) with the real invoicing, parties, tickets and portal: the
//! customer's buyer disputes an invoice in the portal; a quality dispute
//! raises an NCR on each lot of the ticked lines (every line when none is
//! ticked), links the dispute and its ticket and tells QC; a price dispute
//! raises none; the buyer sees the status of the dispute and never the NCR.

mod manufacturing_support;

use host::roles::{ADMIN, QC_INSPECTOR};
use manufacturing::Ncr;
use manufacturing::disputes::links_of;
use manufacturing_support::{Floor, boot, browse, floor, person, post};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_inventory::{Lot, LotStatus, NewDispatch, NewDispatchLine, create_dispatch};
use wl_invoicing::Invoice;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory, invoicer};
use wl_kernel::{PartyId, PartyKind, Subject};
use wl_parties::{CompanyDetails, NewParty, create};
use wl_portal::{AccessGrant, GrantStatus};

async fn customer(app: &TestApp) -> PartyId {
    create(
        app.state(),
        NewParty {
            kind: PartyKind::Company,
            legal_name_en: "Al Noor Trading LLC".into(),
            company: Some(CompanyDetails::default()),
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .party_id()
}

/// The buyer: a user with an active portal grant on the customer.
async fn buyer(app: &TestApp, company: PartyId) -> User {
    let mut user = User::register(app.db(), "Bilal", "bilal@alnoor.example", "secret123")
        .await
        .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    user.assign_role(app.db(), host::roles::MEMBER)
        .await
        .unwrap();
    let mut grant = AccessGrant {
        user_id: Some(user.id),
        person_party_id: company.0,
        party_id: company.0,
        audience: "tenant".into(),
        roles: "owner".into(),
        source: "invitation".into(),
        valid_from: renox::chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(),
        status: GrantStatus::Active,
        ..AccessGrant::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    grant
        .insert_in_state(&mut tx, GrantStatus::Active, "test")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    user
}

async fn lot(app: &TestApp, f: &Floor, number: &str) -> Lot {
    let mut lot = Lot {
        item_id: f.resin.id,
        number: number.to_owned(),
        status: LotStatus::Released,
        ..Lot::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    lot.insert_in_state(&mut tx, LotStatus::Released, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    lot
}

/// A dispatch of one line per lot to the customer and an issued invoice with
/// a line per dispatch line.
async fn invoiced(
    app: &TestApp,
    f: &Floor,
    company: PartyId,
    accountant: &User,
    lots: &[&Lot],
) -> Invoice {
    let dispatch = create_dispatch(
        app.db(),
        None,
        NewDispatch {
            source: Subject::new("sales.sales_order", 50),
            recipient: Some(company),
            recipient_name: "Al Noor Trading LLC".into(),
            address: "Warehouse 4".into(),
            location_id: f.finished.id,
            lines: lots
                .iter()
                .map(|lot| NewDispatchLine::of(f.resin.id, 10_000).lot(lot.id))
                .collect(),
        },
    )
    .await
    .unwrap();
    let lines = wl_inventory::dispatch::lines_of(app.db(), dispatch.id)
        .await
        .unwrap();
    let mut draft = InvoiceDraft::new(company, dispatch.subject());
    for line in &lines {
        draft = draft
            .line(DraftLine::new("Resin", 100_000, VatCategory::ZeroRated).origin(line.subject()));
    }
    let id = invoicer(app.state())
        .draft(app.db(), draft)
        .await
        .unwrap()
        .expect("invoicing is on");
    let issued = post(app, accountant, &format!("/invoices/{}/issue", id.0), &[]).await;
    assert_eq!(issued.status.as_u16(), 303, "{}", issued.text());
    Invoice::find_or_404(app.db(), id.0).await.unwrap()
}

async fn ncrs(app: &TestApp) -> Vec<Ncr> {
    Ncr::query().order_by(Ncr::ID).get(app.db()).await.unwrap()
}

/// INV with line 1 (lot F4) and line 2 (lot F6): the buyer ticks line 1 and
/// disputes for quality ("colour off"): the invoice is flagged, a ticket is
/// linked, an open NCR exists on F4 only, with the text, and QC has the notice.
/// With no line ticked on a second invoice every lot gets one. The buyer's
/// page shows *Under review* and no NCR.
#[renox::test]
async fn a_quality_dispute_from_the_portal_raises_ncrs_on_the_ticked_lots() {
    let app = boot().await;
    let f = floor(&app).await;
    let company = customer(&app).await;
    let bilal = buyer(&app, company).await;
    let accountant = person(&app, "Alia", ADMIN, None).await;
    let qc = person(&app, "Ines", QC_INSPECTOR, Some(f.plant.scope())).await;
    let (f4, f6) = (lot(&app, &f, "F4").await, lot(&app, &f, "F6").await);
    let first = invoiced(&app, &f, company, &accountant, &[&f4, &f6]).await;
    let path = format!("/portal/manufacturing/invoices/{}/dispute", first.id);

    post(
        &app,
        &bilal,
        &path,
        &[
            ("kind", "quality"),
            ("text", "colour off"),
            ("line_1", "on"),
        ],
    )
    .await
    .assert_status(303);

    let flagged = Invoice::find_or_404(app.db(), first.id).await.unwrap();
    assert!(flagged.disputed, "the invoice is flagged");
    let found = ncrs(&app).await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].lot_id, Some(f4.id));
    assert_eq!(found[0].defect, "colour off");
    let links = links_of(app.db(), found[0].id).await.unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].invoice_id, first.id);
    assert!(links[0].ticket_id.is_some(), "the ticket is linked");
    let page = browse(&app, &bilal, "/portal/manufacturing/invoices").await;
    page.assert_ok();
    assert!(page.text().contains("Under review"), "{}", page.text());
    assert!(!page.text().contains(&found[0].number));
    assert!(!page.text().contains("NCR"));
    let record = browse(&app, &qc, &format!("/manufacturing/ncrs/{}", found[0].id)).await;
    assert!(
        record.text().contains("Customer dispute"),
        "{}",
        record.text()
    );

    let second = invoiced(&app, &f, company, &accountant, &[&f4, &f6]).await;
    post(
        &app,
        &bilal,
        &format!("/portal/manufacturing/invoices/{}/dispute", second.id),
        &[("kind", "quality"), ("text", "brittle")],
    )
    .await
    .assert_status(303);
    let lots: Vec<_> = ncrs(&app).await.iter().map(|n| n.lot_id).collect();
    assert_eq!(
        lots,
        [Some(f4.id), Some(f6.id)],
        "F4 is linked again; none is raised twice"
    );
}

/// A dispute about the price flags the invoice and raises no NCR, even when
/// lines are sent; a person with no portal grant is refused; another
/// customer's invoice answers 404.
#[renox::test]
async fn a_price_dispute_raises_no_ncr_and_only_the_buyer_may_dispute() {
    let app = boot().await;
    let f = floor(&app).await;
    let company = customer(&app).await;
    let bilal = buyer(&app, company).await;
    let accountant = person(&app, "Alia", ADMIN, None).await;
    let outsider = person(&app, "Olga", QC_INSPECTOR, Some(f.plant.scope())).await;
    let f4 = lot(&app, &f, "F4").await;
    let invoice = invoiced(&app, &f, company, &accountant, &[&f4]).await;
    let path = format!("/portal/manufacturing/invoices/{}/dispute", invoice.id);

    let denied = post(&app, &outsider, &path, &[("kind", "price"), ("text", "x")]).await;
    post(
        &app,
        &bilal,
        &path,
        &[("kind", "price"), ("text", "too dear"), ("line_1", "on")],
    )
    .await
    .assert_status(303);

    denied.assert_status(403);
    assert!(
        Invoice::find_or_404(app.db(), invoice.id)
            .await
            .unwrap()
            .disputed
    );
    assert!(ncrs(&app).await.is_empty());
    let other = post(
        &app,
        &bilal,
        "/portal/manufacturing/invoices/999/dispute",
        &[("kind", "price"), ("text", "x")],
    )
    .await;
    other.assert_status(404);
}

/// BL-MFG-14 *Disputes* and BL-MFG-13 AC3 (#394, #395): the buyer's portal
/// offers "Dispute this invoice"; the choice of lines is part of the form only
/// for the kind *quality* (it sits in a block the page shows when the kind is
/// quality), and a price dispute ignores the lines it is sent: no NCR.
#[renox::test]
async fn the_portal_offers_dispute_this_invoice_and_the_lines_only_for_a_quality_dispute() {
    let app = boot().await;
    let f = floor(&app).await;
    let company = customer(&app).await;
    let bilal = buyer(&app, company).await;
    let accountant = person(&app, "Alia", ADMIN, None).await;
    let f4 = lot(&app, &f, "F4").await;
    let invoice = invoiced(&app, &f, company, &accountant, &[&f4]).await;

    let page = browse(&app, &bilal, "/portal/manufacturing/invoices").await;
    page.assert_ok();
    page.assert_see("Dispute this invoice");
    let html = page.text();
    let gate = html
        .find("data-rx-show-when")
        .unwrap_or_else(|| panic!("a block shown by the kind: {html}"));
    let checkbox = html.find("line_1").expect("the line to tick");
    assert!(
        gate < checkbox,
        "the lines come after the gate that shows them"
    );
    let block = &html[gate..checkbox];
    assert!(
        block.contains("quality") && !block.contains("price"),
        "the block is shown for quality only: {block}"
    );

    post(
        &app,
        &bilal,
        &format!("/portal/manufacturing/invoices/{}/dispute", invoice.id),
        &[("kind", "price"), ("text", "too dear"), ("line_1", "on")],
    )
    .await
    .assert_status(303);
    assert!(ncrs(&app).await.is_empty());
}
