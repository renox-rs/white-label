//! BL-MPW-01 (#415) point 4 and 7 and BL-MPW-03 (#417) point 2, `MPW.RUL.10`,
//! with the real wl-invoicing and wl-payments in the host of a manpower
//! agency: the `FIN.RUL.01` reminder still goes to the client after the credit
//! hold while manpower sends the client company nothing; a real cheque bounce
//! holds the contract, paying the invoice leaves it held, and the real
//! resolution of the cheque (a replacement) releases it.

mod support;

use host::roles::{ACCOUNTANT, RECEIVABLES};
use manpower::{ClientContract, ContractHold, ContractStatus, HoldReason};
use renox::chrono::Days;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{company, entity, messages_to_party, noon_clock, person};
use wl_invoicing::Invoice;
use wl_invoicing::reminders::AFTER_DUE;
use wl_kernel::cheques::{ChequeStatus, RegisterCheque, cheques};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, InvoiceId, VatCategory, invoicer};
use wl_kernel::{ClientConfig, PartyId, Subject};
use wl_notify::Message;
use wl_payments::Cheque;

/// A manpower agency: the vertical on, with the shared modules it reaches.
const CLIENT: &str = r#"
[client]
name = "Gulf Manpower Services LLC"
app = "manpower"

[modules]
enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "payments", "hr", "attendance", "portal"]

[[legal_entities]]
code = "GMS"
name_en = "Gulf Manpower Services LLC"
default = true
"#;

async fn booted() -> TestApp {
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(host::app_for(config).unwrap().provide(noon_clock())).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// An active contract of `client`, inserted as a fixture.
async fn active_contract(app: &TestApp, client: PartyId) -> ClientContract {
    let mut contract = ClientContract {
        reference: "CC-0001".into(),
        client_party_id: client.0,
        start_date: today().checked_sub_days(Days::new(90)).unwrap(),
        end_date: today().checked_add_days(Days::new(300)).unwrap(),
        payment_terms_days: 30,
        notice_period_days: 30,
        status: ContractStatus::Active,
        ..ClientContract::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    contract
        .insert_in_state(&mut tx, ContractStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    contract
}

/// INV-1 of the contract's hours, AED 42,000, issued by `clerk` and due 31
/// days ago, then marked overdue by wl-invoicing's own run.
async fn overdue_invoice(app: &TestApp, clerk: &User, contract: &ClientContract) -> Invoice {
    let draft = InvoiceDraft::new(PartyId(contract.client_party_id), contract.subject())
        .line(
            DraftLine::new("Guards, September", 4_200_000, VatCategory::Exempt)
                .origin(contract.subject()),
        )
        .due_date(today().checked_sub_days(Days::new(31)).unwrap());
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
    wl_invoicing::overdue::run(app.state()).await.unwrap();
    Invoice::find_or_404(app.db(), id.0).await.unwrap()
}

async fn pay(app: &TestApp, invoice: &Invoice) {
    invoicer(app.state())
        .allocate(
            app.state(),
            InvoiceId(invoice.id),
            Subject::new("payments.payment", 1),
            invoice.balance,
        )
        .await
        .unwrap();
}

/// MPW-01 AC7: after the credit hold the client still gets the `FIN.RUL.01`
/// reminder for INV-1 (wl-invoicing's, 14 days after the due date), and not one
/// message of the manpower vertical goes to the client company.
#[renox::test]
async fn the_reminder_for_the_overdue_invoice_still_goes_to_the_client_after_the_hold() {
    let app = booted().await;
    let clerk = person(&app, "Wendy", RECEIVABLES).await;
    let client = company(&app, "Gulf Tower LLC").await;
    let contract = active_contract(&app, client).await;
    overdue_invoice(&app, &clerk, &contract).await;

    app.run_scheduled(manpower::CREDIT_HOLD_TASK).await.unwrap();
    assert_eq!(
        ClientContract::find_or_404(app.db(), contract.id)
            .await
            .unwrap()
            .status,
        ContractStatus::OnHold
    );

    let sent = wl_invoicing::reminders::run(app.state(), today())
        .await
        .unwrap();
    assert_eq!(sent, 1, "the reminder goes on as before the hold");
    assert_eq!(
        messages_to_party(&app, AFTER_DUE, client).await.len(),
        1,
        "the client has the reminder"
    );
    let to_client = Message::query()
        .where_(Message::PARTY_ID.eq(client.0))
        .get(app.db())
        .await
        .unwrap();
    assert!(
        to_client
            .iter()
            .all(|message| !message.type_key.starts_with("manpower.")),
        "manpower told the client company nothing: {:?}",
        to_client.iter().map(|m| &m.type_key).collect::<Vec<_>>()
    );
}

/// MPW-01 AC4 and MPW-03 AC2: a received cheque of the client bounces through
/// the cheque route and holds the contract (on top of INV-1); paying INV-1
/// leaves it held for the cheque; the replacement that covers the cheque
/// resolves it for real (`ChequeBounceResolved`) and the contract goes back to
/// `active`.
#[renox::test]
async fn a_real_bounced_cheque_holds_and_its_real_resolution_releases() {
    let app = booted().await;
    let clerk = person(&app, "Wendy", RECEIVABLES).await;
    let accountant = person(&app, "Asha", ACCOUNTANT).await;
    let client = company(&app, "Gulf Tower LLC").await;
    let contract = active_contract(&app, client).await;
    let invoice = overdue_invoice(&app, &clerk, &contract).await;
    app.run_scheduled(manpower::CREDIT_HOLD_TASK).await.unwrap();

    let gms = entity(&app, "GMS").await;
    let register =
        |number: &str| RegisterCheque::new(client, gms.id, "ENBD", number, today(), 1_500_000);
    let summary = cheques(app.state())
        .register(app.state(), register("CHQ-9"), Some(accountant.id))
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
                ("certificate", "CERT-9"),
                ("honoured", ""),
            ],
        )
        .await
        .assert_ok();
    assert_eq!(
        Cheque::find_or_404(app.db(), cheque.id)
            .await
            .unwrap()
            .status,
        ChequeStatus::Bounced
    );
    let held = ClientContract::find_or_404(app.db(), contract.id)
        .await
        .unwrap();
    assert_eq!(held.status, ContractStatus::OnHold);
    assert_eq!(held.status_before_hold, Some(ContractStatus::Active));
    let reasons = ContractHold::open_of(app.db(), contract.id).await.unwrap();
    assert_eq!(reasons.len(), 2, "INV-1 and the cheque");
    let bounce = reasons
        .iter()
        .find(|row| row.reason == HoldReason::BouncedCheque)
        .expect("the cheque is a reason");
    assert_eq!(bounce.cheque_id, Some(cheque.id));
    assert_eq!(bounce.amount, 1_500_000);

    // INV-1 paid: the cheque still holds the contract.
    pay(&app, &invoice).await;
    assert_eq!(
        ClientContract::find_or_404(app.db(), contract.id)
            .await
            .unwrap()
            .status,
        ContractStatus::OnHold
    );
    let left = ContractHold::open_of(app.db(), contract.id).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].reason, HoldReason::BouncedCheque);

    // A replacement cheque covers it: the cheque is replaced, resolved.
    let replacement = cheques(app.state())
        .register(app.state(), register("CHQ-10"), Some(accountant.id))
        .await
        .expect("the replacement is registered");
    app.htmx()
        .post(
            &format!("/cheques/{}/replacements", cheque.id),
            &[
                ("kind", "cheque"),
                ("ref_id", &replacement.id.0.to_string()),
            ],
        )
        .await
        .assert_ok();
    app.htmx()
        .post(&format!("/cheques/{}/replace", cheque.id), &[])
        .await
        .assert_ok();

    assert_eq!(
        Cheque::find_or_404(app.db(), cheque.id)
            .await
            .unwrap()
            .status,
        ChequeStatus::Replaced
    );
    let released = ClientContract::find_or_404(app.db(), contract.id)
        .await
        .unwrap();
    assert_eq!(released.status, ContractStatus::Active);
    assert!(
        ContractHold::open_of(app.db(), contract.id)
            .await
            .unwrap()
            .is_empty()
    );
}
