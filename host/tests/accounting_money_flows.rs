//! Class A gaps of the batch 3 audit for #348, through the real host of the 04
//! Accounting client and its real routes: a cheque that bounces, the
//! write-off notice the client gets and who may approve a write-off, and the
//! client portal after the engagement ends (read-only for 90 days, then
//! closed).

mod support;

use accounting::{Engagement, EngagementStatus, Return, ReturnStatus, ReturnType};
use host::roles::{ACCOUNTANT, BILLING, PARTNER};
use renox::chrono::{NaiveDate, NaiveDateTime};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::*;
use wl_kernel::approvals;
use wl_kernel::cheques::{ChequeStatus, RegisterCheque, cheques};
use wl_kernel::invoicing::{InvoiceStatus, VatCategory};
use wl_kernel::{BusinessCalendar, PartyId};
use wl_payments::cheque::ChequeReturn;
use wl_payments::followup::FollowupKind;
use wl_payments::notices::{CHEQUE_BOUNCED, CHEQUE_BOUNCED_STAFF};
use wl_payments::{Cheque, FollowupTask};

/// How many lines invoice `id` has.
async fn line_count(app: &TestApp, id: i64) -> u64 {
    wl_invoicing::InvoiceLine::query()
        .where_(wl_invoicing::InvoiceLine::INVOICE_ID.eq(id))
        .count(app.db())
        .await
        .unwrap()
}

fn date(text: &str) -> NaiveDate {
    text.parse().expect("a date")
}

/// A bounced cheque in the 04 host: the accountant deposits it and records the
/// return through the cheque routes. No fee line is added to the client's
/// invoice (none is set), the client is told, billing (the cheque manager) is
/// told, and the accountant has the follow-up task.
#[renox::test]
async fn a_bounced_cheque_adds_no_fee_and_billing_and_the_accountant_are_told() {
    let app = accounting_booted().await;
    let billing = person(&app, "Bella", BILLING).await;
    let accountant = person(&app, "Asha", ACCOUNTANT).await;
    let client = company(&app, "Gulf Trading A LLC").await;
    let invoice = issued_invoice(
        &app,
        &billing,
        client,
        2_000_000,
        VatCategory::Exempt,
        day(10),
    )
    .await;
    let lines_before = line_count(&app, invoice.id).await;
    let entity = entity(&app, "FLA").await;
    let request = RegisterCheque::new(client, entity.id, "ENBD", "1042", day(0), 2_000_000)
        .link(invoice.subject());
    let summary = cheques(app.state())
        .register(app.state(), request, Some(accountant.id))
        .await
        .expect("the cheque is registered");
    let cheque = Cheque::find_or_404(app.db(), summary.id.0).await.unwrap();

    app.acting_as(&accountant);
    app.htmx()
        .post(&format!("/cheques/{}/deposit", cheque.id), &[])
        .await
        .assert_ok();
    app.htmx()
        .post(
            &format!("/cheques/{}/bounce", cheque.id),
            &[
                ("reason", "insufficient_funds"),
                ("certificate", "CERT-77"),
                ("honoured", ""),
            ],
        )
        .await
        .assert_ok();

    let bounced = Cheque::find_or_404(app.db(), cheque.id).await.unwrap();
    assert_eq!(bounced.status, ChequeStatus::Bounced);
    assert_eq!(bounced.bounce_fee_total, 0);
    let returns = ChequeReturn::of_cheque(app.db(), cheque.id).await.unwrap();
    assert_eq!(returns.len(), 1);
    assert_eq!(returns[0].fee_amount, 0);
    assert_eq!(returns[0].fee_subject_id, None, "no fee charge was created");
    assert_eq!(
        line_count(&app, invoice.id).await,
        lines_before,
        "no fee line on the invoice"
    );
    assert_eq!(
        messages_to_party(&app, CHEQUE_BOUNCED, client).await.len(),
        1,
        "the client is told"
    );
    assert_eq!(
        messages_to(&app, CHEQUE_BOUNCED_STAFF, &billing)
            .await
            .len(),
        1,
        "billing, the cheque manager, is told"
    );
    let follow_up = FollowupTask::open_of_cheque(app.db(), cheque.id)
        .await
        .unwrap();
    assert_eq!(follow_up.len(), 1);
    assert_eq!(follow_up[0].kind, FollowupKind::FollowUpCheque);
    assert_eq!(follow_up[0].role, "accountant");
}

/// A written-off invoice sends the client the notice by e-mail (the real
/// mail, not the column), and billing never approves a write-off: it asks,
/// a partner approves, billing is refused.
#[renox::test]
async fn the_client_gets_the_write_off_notice_and_billing_cannot_approve() {
    let app = accounting_booted().await;
    let billing = person(&app, "Bella", BILLING).await;
    let other_billing = person(&app, "Basma", BILLING).await;
    let partner = person(&app, "Pia", PARTNER).await;
    let client = company(&app, "Gulf Trading A LLC").await;
    let invoice = issued_invoice(&app, &billing, client, 100, VatCategory::Exempt, day(-30)).await;
    let number = invoice.number.clone().expect("a number");
    app.acting_as(&billing);
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
    let approve = format!("/approvals/{}/approve", pending[0].id);

    // Billing, the requester or not, may not approve.
    for user in [&billing, &other_billing] {
        app.acting_as(user);
        app.htmx().post(&approve, &[]).await.assert_forbidden();
    }
    assert_eq!(reload(&app, &invoice).await.status, InvoiceStatus::Issued);
    assert!(
        app.sent_mail()
            .iter()
            .all(|mail| !mail.subject.contains("written off")),
        "nothing is sent before the approval"
    );

    app.acting_as(&partner);
    app.htmx()
        .post(&approve, &[("comment", "agreed")])
        .await
        .assert_ok();
    assert_eq!(
        reload(&app, &invoice).await.status,
        InvoiceStatus::WrittenOff
    );
    app.run_all_jobs().await;
    let notices: Vec<_> = app
        .sent_mail()
        .into_iter()
        .filter(|mail| mail.subject == format!("Invoice {number} written off as a bad debt"))
        .collect();
    assert_eq!(notices.len(), 1, "{:?}", app.sent_mail());
    assert!(
        notices[0].text.contains("Amount written off AED 1.00"),
        "{}",
        notices[0].text
    );
    assert_eq!(notices[0].to.len(), 1);
}

/// Moves the clock to `at` (Asia/Dubai, ahead of the real clock).
fn travel_to(app: &TestApp, at: &str) {
    let local = NaiveDateTime::parse_from_str(at, "%Y-%m-%d %H:%M").unwrap();
    let ahead = (BusinessCalendar::default().from_local(local) - renox::db::now()).num_seconds();
    assert!(ahead > 0, "{at} is ahead of the clock");
    app.travel_back();
    app.travel(std::time::Duration::from_secs(ahead.unsigned_abs()));
}

/// An engagement of `company` that ended on 2026-11-01.
async fn ended_engagement(app: &TestApp, company: PartyId) -> Engagement {
    let mut engagement = Engagement {
        client_party_id: company.0,
        title: "Bookkeeping".to_owned(),
        letter_signed: true,
        ended_on: Some(date("2026-11-01")),
        status: EngagementStatus::Ended,
        ..Engagement::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    engagement
        .insert_in_state(&mut tx, EngagementStatus::Ended, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    Engagement::find_or_404(app.db(), engagement.id)
        .await
        .unwrap()
}

/// `POR.ACL.01` through the portal routes as a client user: the engagement
/// ended 2026-11-01, so until 2027-01-30 the portal reads (the engagement is
/// listed, the return page says nothing can be changed) and refuses every
/// change; from 2027-01-31 the section is empty and the return page is gone.
#[renox::test]
async fn the_client_portal_reads_for_90_days_after_the_end_then_closes() {
    let app = accounting_booted().await;
    let company = company(&app, "Gulf Trading A LLC").await;
    let omar = tenant(&app, "Omar Haddad").await;
    let engagement = ended_engagement(&app, company).await;
    let mut ret = Return {
        client_party_id: company.0,
        engagement_id: engagement.id,
        return_type: ReturnType::Vat,
        period_end: date("2026-09-30"),
        due_date: date("2026-10-28"),
        version: 1,
        figures: Some("Output VAT 12,000; input VAT 4,000".to_owned()),
        workpapers_attached: true,
        status: ReturnStatus::ClientApproval,
        ..Return::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    ret.insert_in_state(&mut tx, ReturnStatus::ClientApproval, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let client_user = person(&app, "Omar", host::roles::MEMBER).await;
    wl_portal::PortalAccess::register(
        app.state(),
        client_user.id,
        wl_portal::NewGrant::new(omar, company, "client", &["owner"]),
    )
    .await
    .unwrap();
    let section = format!("/portal/{}/engagements", company.0);
    let page = format!("/accounting/portal/returns/{}", ret.id);

    // The last day it reads.
    travel_to(&app, "2027-01-30 12:00");
    app.acting_as(&client_user);
    app.get(&section)
        .await
        .assert_ok()
        .assert_see(&engagement.number());
    app.get(&page)
        .await
        .assert_ok()
        .assert_see("nothing can be changed");
    for action in ["approve", "send-back", "withdraw"] {
        app.htmx()
            .post(
                &format!("/accounting/portal/returns/{}/{action}", ret.id),
                &[("comment", "ok")],
            )
            .await
            .assert_forbidden();
    }
    assert_eq!(
        Return::find_or_404(app.db(), ret.id).await.unwrap().status,
        ReturnStatus::ClientApproval,
        "nothing changed while read-only"
    );

    // The day after, the portal is closed.
    travel_to(&app, "2027-01-31 12:00");
    app.acting_as(&client_user);
    let closed = app.get(&section).await;
    closed.assert_ok();
    assert!(
        !closed.text().contains(&engagement.number()),
        "the closed portal lists nothing"
    );
    app.get(&page).await.assert_not_found();
    app.htmx()
        .post(
            &format!("/accounting/portal/returns/{}/approve", ret.id),
            &[],
        )
        .await
        .assert_not_found();
    app.travel_back();
}
