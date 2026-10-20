//! The approvers of a decision about an invoice are looked for in the scope
//! of the invoice: the building it was billed for. A property manager of that
//! building decides a write-off, a property manager of another building does
//! not; before, only the holders of a global role were asked, which left the
//! building-scoped property managers of the Property client out of every
//! invoice approval.

mod support;

use host::roles::{PROPERTY_MANAGER, RECEIVABLES};
use renox::prelude::*;
use support::*;
use wl_kernel::approvals;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, InvoiceStatus, VatCategory, invoicer};
use wl_kernel::{PartyId, Subject};

/// An issued invoice of AED 1,000 to `party` for the building `building`.
async fn invoice_for_building(
    app: &renox::testing::TestApp,
    clerk: &User,
    party: PartyId,
    building: i64,
) -> wl_invoicing::Invoice {
    let draft = InvoiceDraft::new(party, Subject::new("property.lease", 9))
        .scope(Subject::new(
            property::building::BUILDING_SUBJECT_TYPE,
            building,
        ))
        .line(DraftLine::new("Rent", 100_000, VatCategory::Exempt))
        .due_date(day(-30));
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
    reload_by_id(app, id.0).await
}

async fn reload_by_id(app: &renox::testing::TestApp, id: i64) -> wl_invoicing::Invoice {
    wl_invoicing::Invoice::find_or_404(app.db(), id)
        .await
        .unwrap()
}

#[renox::test]
async fn a_write_off_is_decided_by_a_property_manager_of_the_invoices_building() {
    let app = booted().await;
    let (tower_a, _) = building(&app, "SC-A").await;
    let (tower_b, _) = building(&app, "SC-B").await;
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let manager_a = scoped_person(&app, "Pam", PROPERTY_MANAGER, &tower_a.scope()).await;
    let manager_b = scoped_person(&app, "Paul", PROPERTY_MANAGER, &tower_b.scope()).await;
    let party = tenant(&app, "Tara Nasser").await;

    let invoice = invoice_for_building(&app, &clerk, party, tower_a.id).await;
    assert_eq!(invoice.scope_id, Some(tower_a.id));
    app.acting_as(&clerk);
    app.htmx()
        .post(
            &format!("/invoices/{}/write-off", invoice.id),
            &[("reason_code", "insolvent"), ("reason_text", "")],
        )
        .await
        .assert_ok();
    let pending = approvals::pending_for(app.db(), &invoice.subject())
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);

    let waiting = |user: &User| {
        let user = user.clone();
        let db = app.db().clone();
        async move {
            approvals::awaiting(&db, &user)
                .await
                .unwrap()
                .iter()
                .any(|r| r.action == "invoicing.write_off")
        }
    };
    assert!(waiting(&manager_a).await, "the building's manager is asked");
    assert!(
        !waiting(&manager_b).await,
        "the manager of another building is not"
    );
    let refused = approvals::approve(app.state(), pending[0].id, &manager_b, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    approvals::approve(app.state(), pending[0].id, &manager_a, "agreed")
        .await
        .unwrap()
        .expect("the approval runs");
    assert_eq!(
        reload_by_id(&app, invoice.id).await.status,
        InvoiceStatus::WrittenOff
    );
}
