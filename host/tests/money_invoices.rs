//! BL-PRP-26 (#276), the invoice settings of the Property client run by the
//! real modules of the host (a fresh 01 deployment, `wl:install` and the
//! seeds): the write-off approval (items 1 and 4), the invoice dispute of the
//! party and its escalation (items 2 and 3), the chargeable ticket invoiced on
//! close and issued at once (items 7 and 7a), the reminders and the 01 task
//! paused by a dispute (item 4), the bad-debt relief (item 5) and the
//! e-invoice transmission (item 6).

mod support;

use host::roles::{ACCOUNTANT, ADMIN, PROPERTY_MANAGER, RECEIVABLES, TECHNICIAN};
use renox::prelude::*;
use support::*;
use wl_invoicing::dispute::{self, Dispute};
use wl_invoicing::escalation;
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest, ApprovalStatus};
use wl_kernel::invoicing::{
    DisputeBy, DisputeRequest, InvoiceId, InvoiceStatus, VatCategory, invoicer,
};
use wl_kernel::tickets::{Assignee, OpenTicket};
use wl_ticketing::{Ticket, TicketStatus, Tickets};

/// Item 1: in a fresh 01 deployment a write-off of AED 1 creates an approval
/// request (the limit is AED 0), the approvers are the property managers (and
/// the company admin), and the requester never approves it: a company admin
/// who asks cannot approve their own request, a property manager can.
#[renox::test]
async fn a_write_off_of_aed_1_waits_for_a_property_manager_who_did_not_ask() {
    let app = booted().await;
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let admin = person(&app, "Alia", ADMIN).await;
    let manager = person(&app, "Pam", PROPERTY_MANAGER).await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let party = tenant(&app, "Tara Nasser").await;

    // AED 1.00, exempt, due last month.
    let invoice = issued_invoice(&app, &clerk, party, 100, VatCategory::Exempt, day(-30)).await;
    assert_eq!(invoice.balance, 100);

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
    assert_eq!(pending.len(), 1, "AED 1 is above the limit of AED 0");
    assert_eq!(pending[0].requester_id, clerk.id);
    assert_eq!(reload(&app, &invoice).await.status, InvoiceStatus::Issued);

    // The property manager and the admin are asked; the requester, the
    // accountant and a technician are not.
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
    assert!(waiting(&manager).await);
    assert!(waiting(&admin).await);
    assert!(!waiting(&clerk).await);
    assert!(!waiting(&accountant).await);

    // A second approver is not the requester: the clerk cannot approve, the
    // property manager can, and the invoice is written off.
    let own = approvals::approve(app.state(), pending[0].id, &clerk, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");
    approvals::approve(app.state(), pending[0].id, &manager, "agreed")
        .await
        .unwrap()
        .expect("the approval runs");
    assert_eq!(
        reload(&app, &invoice).await.status,
        InvoiceStatus::WrittenOff
    );

    // The company admin asks for another one: not the admin, the manager.
    let second = issued_invoice(&app, &clerk, party, 100, VatCategory::Exempt, day(-30)).await;
    app.acting_as(&admin);
    app.htmx()
        .post(
            &format!("/invoices/{}/write-off", second.id),
            &[("reason_code", "insolvent"), ("reason_text", "")],
        )
        .await
        .assert_ok();
    let pending = approvals::pending_for(app.db(), &second.subject())
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    let own = approvals::approve(app.state(), pending[0].id, &admin, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");
    approvals::approve(app.state(), pending[0].id, &manager, "")
        .await
        .unwrap();
    assert_eq!(
        reload(&app, &second).await.status,
        InvoiceStatus::WrittenOff
    );
    let stored = ApprovalRequest::find(app.db(), pending[0].id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, ApprovalStatus::Approved);
}

/// Item 2: the party disputes in the portal (`portal_disputes` is on), only
/// its own invoice; staff can also dispute.
#[renox::test]
async fn the_tenant_may_dispute_its_own_issued_invoice_and_no_one_elses() {
    let app = booted().await;
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let tara = tenant(&app, "Tara Nasser").await;
    let other = tenant(&app, "Omar Hassan").await;
    let invoice = issued_invoice(&app, &clerk, tara, 100_000, VatCategory::Standard, day(10)).await;

    let foreign = DisputeRequest::new(
        InvoiceId(invoice.id),
        "not my flat",
        DisputeBy::Party { id: other },
    )
    .amount(1_000);
    let refused = dispute::raise(app.state(), foreign).await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    assert!(!reload(&app, &invoice).await.disputed);

    let own = DisputeRequest::new(
        InvoiceId(invoice.id),
        "AC was out of order",
        DisputeBy::Party { id: tara },
    )
    .amount(10_000);
    let id = dispute::raise(app.state(), own).await.unwrap();
    let flagged = reload(&app, &invoice).await;
    assert!(flagged.disputed);
    let stored = Dispute::find(app.db(), id).await.unwrap().unwrap();
    assert_eq!(stored.raised_by_party_id, Some(tara.0));
    assert_eq!(stored.amount, 10_000);

    // Staff can also dispute another invoice.
    let second = issued_invoice(&app, &clerk, tara, 50_000, VatCategory::Standard, day(10)).await;
    app.acting_as(&clerk);
    app.htmx()
        .post(
            &format!("/invoices/{}/dispute", second.id),
            &[("reason", "Wrong hours"), ("amount", "100.00")],
        )
        .await
        .assert_ok();
    assert!(reload(&app, &second).await.disputed);
}

/// Item 3: a dispute open 14 days reminds the invoice owner, and 14 days
/// later the property manager too (`FIN.RUL.15`); not before the 14th day.
#[renox::test]
async fn a_dispute_open_fourteen_days_reminds_the_owner_and_then_the_property_manager() {
    let app = booted().await;
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let manager = person(&app, "Pam", PROPERTY_MANAGER).await;
    let tara = tenant(&app, "Tara Nasser").await;
    let invoice = issued_invoice(&app, &clerk, tara, 100_000, VatCategory::Standard, day(30)).await;

    let request = DisputeRequest::new(
        InvoiceId(invoice.id),
        "wrong hours",
        DisputeBy::User { id: clerk.id },
    )
    .amount(20_000);
    let id = dispute::raise(app.state(), request).await.unwrap();
    // Raised 14 days ago.
    let mut row = Dispute::find(app.db(), id).await.unwrap().unwrap();
    let raised = day(-14).and_hms_opt(9, 0, 0).unwrap().and_utc();
    row.raised_at = raised;
    row.save(app.db()).await.unwrap();

    // The 13th day: nothing.
    let sent = escalation::run(app.state(), day(-1)).await.unwrap();
    assert_eq!(sent, 0);
    assert!(
        messages_to(&app, escalation::ESCALATED, &clerk)
            .await
            .is_empty()
    );

    // The 14th day: the invoice owner is reminded, the manager is not told.
    let sent = escalation::run(app.state(), day(0)).await.unwrap();
    assert_eq!(sent, 1);
    assert_eq!(
        messages_to(&app, escalation::ESCALATED, &clerk).await.len(),
        1
    );
    assert!(
        messages_to(&app, escalation::ESCALATED, &manager)
            .await
            .is_empty()
    );
    // The same day again: once.
    assert_eq!(escalation::run(app.state(), day(0)).await.unwrap(), 0);

    // The 28th day: the property manager is told as well.
    escalation::run(app.state(), day(14)).await.unwrap();
    assert_eq!(
        messages_to(&app, escalation::ESCALATED, &manager)
            .await
            .len(),
        1
    );
    assert_eq!(
        messages_to(&app, escalation::ESCALATED, &clerk).await.len(),
        2
    );
}

/// Items 7 and 7a: the Property category `tenant_repair` is chargeable and
/// invoiced on close. Resolving creates no invoice (the tenant is shown the
/// amount); on `confirm` the invoice with the charge lines is drafted through
/// the `Invoicer` and issued at once (the ticket source is auto-issued), AED
/// 350 with VAT and an invoice number.
#[renox::test]
async fn a_chargeable_ticket_is_invoiced_and_issued_when_the_tenant_confirms() {
    let app = booted().await;
    let technician = person(&app, "Tim", TECHNICIAN).await;
    // The tenant's portal user: the requester of the ticket, who may see it.
    renox::auth::permissions::define_role(app.db(), "tenant", &[wl_ticketing::permissions::VIEW])
        .await
        .unwrap();
    let tenant_user = person(&app, "Tara", "tenant").await;
    let party = tenant(&app, "Tara Nasser").await;

    let opened = Tickets::open_at(
        app.state(),
        OpenTicket::new(
            Subject::new("property.unit", 12),
            property::seed::TENANT_REPAIR,
            "AC capacitor",
        )
        .requester_user(tenant_user.id)
        .bill_to(party)
        .assign_to(Assignee::User { id: technician.id }),
        renox::db::now(),
    )
    .await
    .unwrap();
    let id = opened.id.0;
    let ticket = Ticket::find_or_404(app.db(), id).await.unwrap();
    assert!(ticket.chargeable, "the Property category is chargeable");

    app.acting_as(&technician);
    app.htmx()
        .post(&format!("/tickets/{id}/actions/start"), &[])
        .await
        .assert_status(303);
    // Resolving needs at least one charge line.
    app.htmx()
        .post(&format!("/tickets/{id}/actions/resolve"), &[])
        .await
        .assert_status(422);
    app.htmx()
        .post(
            &format!("/tickets/{id}/charges"),
            &[
                ("description", "Replace AC capacitor"),
                ("quantity", "1"),
                ("unit_price", "333.33"),
                ("vat_category", "standard"),
            ],
        )
        .await
        .assert_ok();
    app.htmx()
        .post(&format!("/tickets/{id}/actions/resolve"), &[])
        .await
        .assert_status(303);
    let resolved = Ticket::find_or_404(app.db(), id).await.unwrap();
    assert_eq!(resolved.status, TicketStatus::Resolved);
    assert_eq!(resolved.invoice_id, None, "no invoice on resolve");

    app.acting_as(&tenant_user);
    app.htmx()
        .post(&format!("/tickets/{id}/actions/confirm"), &[])
        .await
        .assert_status(303);
    let closed = Ticket::find_or_404(app.db(), id).await.unwrap();
    assert_eq!(closed.status, TicketStatus::Closed);
    let invoice_id = closed.invoice_id.expect("the invoice on confirm");
    let invoice = wl_invoicing::Invoice::find_or_404(app.db(), invoice_id)
        .await
        .unwrap();
    assert_eq!(
        invoice.status,
        InvoiceStatus::Issued,
        "issued without review"
    );
    assert!(invoice.number.is_some());
    assert_eq!(invoice.party_id, party.0);
    assert_eq!(invoice.gross_total, 35_000);
    let summary = invoicer(app.state())
        .find(app.db(), InvoiceId(invoice_id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.status, InvoiceStatus::Issued);
}

/// Item 4: an issued invoice that is disputed gets no `FIN.RUL.01` reminder
/// and no `PRP.RUL.10` demand task, while the same invoice undisputed does:
/// two leases, one invoice each, ten and thirty-one days overdue.
#[renox::test]
async fn a_disputed_invoice_gets_no_reminder_and_no_formal_demand() {
    use property::UnitUse;
    use property::arrears::{self, ArrearsDemand};
    use wl_invoicing::reminders;

    let app = booted().await;
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let tara = tenant(&app, "Tara Nasser").await;
    let omar = tenant(&app, "Omar Hassan").await;
    let tara_unit = unit_of(&app, "TWR-A", "1204", UnitUse::Residential).await;
    let omar_unit = unit_of(&app, "TWR-B", "803", UnitUse::Residential).await;
    let tara_lease = draft_lease(&app, &tara_unit, tara, -300).await;
    let omar_lease = draft_lease(&app, &omar_unit, omar, -300).await;

    // Both invoices fell due 31 days ago and are unpaid.
    let kept = invoiced_charge(
        &app,
        &clerk,
        &tara_lease,
        1_000_000,
        VatCategory::Exempt,
        day(-31),
        1,
    )
    .await;
    let disputed = invoiced_charge(
        &app,
        &clerk,
        &omar_lease,
        1_000_000,
        VatCategory::Exempt,
        day(-31),
        1,
    )
    .await;
    app.acting_as(&clerk);
    app.htmx()
        .post(
            &format!("/invoices/{}/dispute", disputed.id),
            &[("reason", "Wrong amount"), ("amount", "100.00")],
        )
        .await
        .assert_ok();
    assert!(reload(&app, &disputed).await.disputed);
    assert!(!reload(&app, &kept).await.disputed);

    // FIN.RUL.01: one reminder, for the invoice that is not disputed.
    let sent = reminders::run(app.state(), day(0)).await.unwrap();
    assert_eq!(sent, 1);
    let logged = wl_invoicing::ReminderLog::query()
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].invoice_id, kept.id);

    // PRP.RUL.10: one demand, for the lease without the dispute.
    let report = arrears::run(app.state(), day(0)).await.unwrap();
    assert_eq!(report.demands, 1);
    assert_eq!(
        ArrearsDemand::of_root(app.db(), tara_lease.root_id)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        ArrearsDemand::of_root(app.db(), omar_lease.root_id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Item 5: a written-off commercial-rent invoice (standard-rated) that meets
/// the relief conditions gets a `FIN.RUL.14` VAT adjustment proposal on the
/// open VAT summary and the accountant is told; a written-off residential
/// rent invoice (exempt) gets none.
#[renox::test]
async fn bad_debt_relief_is_proposed_for_commercial_rent_and_not_for_residential_rent() {
    use wl_invoicing::bad_debt_relief::{self, PROPOSED};
    use wl_invoicing::{AdjustmentStatus, Invoice, VatAdjustment, VatSummary, vat_summary};

    let app = booted().await;
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let manager = person(&app, "Pam", PROPERTY_MANAGER).await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let admin = person(&app, "Alia", ADMIN).await;
    let shop = company(&app, "Palm Retail LLC").await;
    let tara = tenant(&app, "Tara Nasser").await;

    // AED 1,000 plus 5% VAT for the shop, AED 1,000 exempt for the flat.
    let commercial =
        issued_invoice(&app, &clerk, shop, 100_000, VatCategory::Standard, day(-60)).await;
    let residential =
        issued_invoice(&app, &clerk, tara, 100_000, VatCategory::Exempt, day(-60)).await;
    for invoice in [&commercial, &residential] {
        app.acting_as(&clerk);
        app.htmx()
            .post(
                &format!("/invoices/{}/write-off", invoice.id),
                &[("reason_code", "insolvent"), ("reason_text", "")],
            )
            .await
            .assert_ok();
        let request = approvals::pending_for(app.db(), &invoice.subject())
            .await
            .unwrap()
            .remove(0);
        approvals::approve(app.state(), request.id, &manager, "")
            .await
            .unwrap()
            .expect("the approval runs");
        assert_eq!(
            reload(&app, invoice).await.status,
            InvoiceStatus::WrittenOff
        );
    }
    let relief = VatAdjustment::relief_of(app.db(), &commercial)
        .await
        .unwrap()
        .expect("the commercial write-off made the relief");
    assert_eq!(relief.amount, 5_000);
    assert_eq!(relief.status, AdjustmentStatus::Pending);
    let exempt = VatAdjustment::relief_of(app.db(), &residential)
        .await
        .unwrap();
    assert_eq!(
        exempt.as_ref().map_or(0, |a| a.amount),
        0,
        "residential rent is exempt: nothing to relieve"
    );

    // The conditions: the supply was more than six months ago, the notice of
    // the write-off was given, and the VAT summary of the supply period is
    // locked; the relief goes on the open summary of today.
    let supply = day(-250);
    Invoice::query()
        .where_(Invoice::ID.eq(commercial.id))
        .update(
            app.db(),
            &[
                ("date_of_supply", &Some(supply)),
                ("write_off_notice_on", &Some(day(-30))),
            ],
        )
        .await
        .unwrap();
    let config = wl_invoicing::WlInvoicing::new().config().clone();
    let entity = entity(&app, "PGP").await;
    vat_summary::ensure_open(app.db(), &config, &entity, supply)
        .await
        .unwrap();
    let past = VatSummary::containing(app.db(), &config, &entity, supply)
        .await
        .unwrap()
        .expect("the summary of the supply period");
    for (step, by) in [
        ("prepare", &accountant),
        ("review", &admin),
        ("lock", &admin),
    ] {
        app.acting_as(by);
        app.htmx()
            .post(
                &format!("/vat-summaries/{}/transitions/{step}", past.id),
                &[("reason", "")],
            )
            .await
            .assert_status(303);
    }
    vat_summary::ensure_open(app.db(), &config, &entity, day(0))
        .await
        .unwrap();
    let open = VatSummary::containing(app.db(), &config, &entity, day(0))
        .await
        .unwrap()
        .expect("the open summary of today");

    let ran = bad_debt_relief::run(app.state(), day(0)).await.unwrap();
    assert_eq!(ran.proposed, 1);
    let proposed = VatAdjustment::find(app.db(), relief.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(proposed.status, AdjustmentStatus::Proposed);
    assert_eq!(proposed.summary_id, Some(open.id));
    assert_eq!(messages_to(&app, PROPOSED, &accountant).await.len(), 1);
    // Nothing was proposed for the residential invoice, even with the same
    // conditions.
    if let Some(exempt) = exempt {
        let still = VatAdjustment::find(app.db(), exempt.id)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(still.status, AdjustmentStatus::Proposed);
    }
    // The invoice stays as written off.
    assert_eq!(
        reload(&app, &commercial).await.status,
        InvoiceStatus::WrittenOff
    );
}
