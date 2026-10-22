//! BL-MPW-01 (#415) and BL-MPW-03 (#417), `MPW.RUL.10`, through the real host of
//! a manpower agency: an invoice of wl-invoicing issued under the contract and
//! overdue more than 30 days puts the contract on credit hold at the 02:00
//! run; the invoice paid through the kernel's `Invoicer` (wl-invoicing emits
//! `InvoicePaid`) releases it to the state it had.

use manpower::{ClientContract, ContractHold, ContractStatus, HoldReason};
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_invoicing::Invoice;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory, invoicer};
use wl_kernel::{ClientConfig, PartyId, Subject};
use wl_parties::Party;

use host::roles::{self, RECEIVABLES};

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

/// A contract of `client` that is `active`, inserted as a fixture.
async fn active_contract(app: &TestApp, client: i64) -> ClientContract {
    let mut contract = ClientContract {
        reference: "CC-0001".into(),
        client_party_id: client,
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

/// An invoice of the contract's hours, due on `due`, issued by the clerk.
async fn issued_under(
    app: &TestApp,
    clerk: &User,
    contract: &ClientContract,
    due: NaiveDate,
) -> Invoice {
    let draft = InvoiceDraft::new(PartyId(contract.client_party_id), contract.subject())
        .line(
            DraftLine::new("Guards, October", 4_200_000, VatCategory::Exempt)
                .origin(contract.subject()),
        )
        .due_date(due);
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

/// The invoice is overdue more than 30 days: the 02:00 run holds the contract
/// for it, with its number and balance; a disputed one would not. Paid in
/// full through the `Invoicer`, the contract is released to `active`.
#[renox::test]
async fn an_overdue_wl_invoicing_invoice_holds_the_contract_and_payment_releases_it() {
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(host::app_for(config).unwrap()).await;
    app.kernel().seed().await.unwrap();
    let clerk = person(&app, "Wendy", RECEIVABLES).await;
    let party = Party::query()
        .first(app.db())
        .await
        .unwrap()
        .expect("a seeded party");
    let contract = active_contract(&app, party.id).await;
    // Due 29 days ago: not yet overdue more than 30 days.
    let invoice = issued_under(
        &app,
        &clerk,
        &contract,
        today().checked_sub_days(Days::new(29)).unwrap(),
    )
    .await;
    wl_invoicing::overdue::run(app.state()).await.unwrap();
    app.run_scheduled(manpower::CREDIT_HOLD_TASK).await.unwrap();
    assert_eq!(
        ClientContract::find_or_404(app.db(), contract.id)
            .await
            .unwrap()
            .status,
        ContractStatus::Active
    );

    // Two days later it is 31 days.
    app.travel(std::time::Duration::from_secs(2 * 24 * 3600));
    app.at_travelled_time(async {
        app.run_scheduled(manpower::CREDIT_HOLD_TASK).await.unwrap();
    })
    .await;
    let held = ClientContract::find_or_404(app.db(), contract.id)
        .await
        .unwrap();
    assert_eq!(held.status, ContractStatus::OnHold);
    assert_eq!(held.status_before_hold, Some(ContractStatus::Active));
    let reasons = ContractHold::open_of(app.db(), contract.id).await.unwrap();
    assert_eq!(reasons.len(), 1);
    assert_eq!(reasons[0].reason, HoldReason::Overdue);
    assert_eq!(reasons[0].invoice_id, Some(invoice.id));
    assert_eq!(reasons[0].amount, invoice.balance);
    assert_eq!(reasons[0].label, invoice.number);

    // The client pays: wl-invoicing allocates and emits `InvoicePaid`.
    let invoice = Invoice::find_or_404(app.db(), invoice.id).await.unwrap();
    invoicer(app.state())
        .allocate(
            app.state(),
            wl_kernel::invoicing::InvoiceId(invoice.id),
            Subject::new("payments.payment", 1),
            invoice.balance,
        )
        .await
        .unwrap();
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
