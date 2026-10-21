//! The Construction money flows with the real wl-invoicing, wl-payments and
//! wl-parties in the host (BL-CON-06 #285, BL-CON-08 #287, BL-CON-02 #281):
//! invoicing a certified IPA makes the tax invoice, the credit note for the
//! accepted liquidated damages, the dispute for the contested line and the
//! supplier bill from the client; the bill is approved and set off against
//! the invoice (the balance drops by AED 5,250.00, the bill is paid, no bank
//! account), `reverse` reopens both, the dispute holds the reminders, a
//! blocked client waits for the exception; a retention invoice disputed for
//! a deduction resolves `credited` as a credit note; the final account
//! invoice written off after the approval lets the project close.

mod support;

use construction::{
    DeductionKind, DeductionStatus, Ipa, IpaDeduction, IpaKind, IpaStatus, Project, ProjectStatus,
    Retention, RetentionPart, RetentionStatus,
};
use host::roles::{ACCOUNTANT, ADMIN, FINANCE_MANAGER, PROJECT_MANAGER, RECEIVABLES};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{block, company, day, entity, noon_clock, person};
use wl_invoicing::bill_machine::approve as approve_bill;
use wl_invoicing::bill_submit::submit;
use wl_invoicing::credit_notes::By;
use wl_invoicing::{CreditNote, Dispute, Invoice, SupplierBill, SupplierBillStatus};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::invoicing::InvoiceStatus;
use wl_kernel::payments::{PaymentMethod, PaymentStatus};
use wl_kernel::{BLOCK_EXCEPTION_ACTION, ClientConfig, PartyId};
use wl_payments::Payment;

const CLIENT: &str = r#"
[client]
name = "Gulf Build Contracting LLC"
app = "construction"

[modules]
enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "payments", "hr", "attendance", "inventory"]

[[legal_entities]]
code = "GBC"
name_en = "Gulf Build Contracting LLC"
trn = "100000000000003"
default = true
"#;

/// AED in fils.
const fn aed(amount: i64) -> i64 {
    amount * 100
}

async fn boot() -> TestApp {
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(host::app_for(config).unwrap().provide(noon_clock())).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// The accountant of a Construction client: the accountant role and the
/// receivables role, which issues, credits and disputes invoices.
async fn accountant(app: &TestApp, name: &str) -> User {
    let user = person(app, name, ACCOUNTANT).await;
    user.assign_role(app.db(), RECEIVABLES).await.unwrap();
    user
}

async fn post(app: &TestApp, by: &User, path: &str, form: &[(&str, &str)]) -> TestResponse {
    app.acting_as(by);
    app.htmx().post(path, form).await
}

use renox::testing::TestResponse;

/// Panics with the message of the page when the answer is not 200.
#[track_caller]
fn ok(response: TestResponse) {
    if response.status.as_u16() != 200 {
        let text = response.text();
        let message = text
            .split("app-empty__text\">")
            .nth(1)
            .and_then(|rest| rest.split('<').next())
            .unwrap_or(&text);
        panic!("{}: {message}", response.status);
    }
}

/// A project of `client` in `status`, managed by `manager`.
async fn project(
    app: &TestApp,
    code: &str,
    client: PartyId,
    status: ProjectStatus,
    manager: &User,
) -> Project {
    let mut project = Project::new(code, "Marina Tower fit-out", client).with_budget(50_000_000);
    project.manager_user_id = Some(manager.id);
    project.status = status;
    let mut tx = app.db().begin().await.unwrap();
    project
        .insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    project
}

async fn ipa(
    app: &TestApp,
    project: &Project,
    kind: IpaKind,
    status: IpaStatus,
    certified: i64,
) -> Ipa {
    let mut ipa = Ipa::new(project.id);
    ipa.number = 6;
    ipa.kind = kind;
    ipa.status = status;
    ipa.claimed_amount = certified + aed(20_000);
    ipa.certified_amount = Some(certified);
    ipa.valuation_on = Some(day(0));
    let mut tx = app.db().begin().await.unwrap();
    ipa.insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    ipa
}

async fn deduction(
    app: &TestApp,
    ipa: &Ipa,
    kind: DeductionKind,
    reference: &str,
    amount: i64,
    status: DeductionStatus,
) {
    let mut line = IpaDeduction {
        project_id: ipa.project_id,
        ipa_id: ipa.id,
        kind,
        reference: reference.to_owned(),
        amount,
        status,
        recorded_by: 1,
        ..IpaDeduction::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    line.insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

/// `POST /payments`: a set-off of `amount` dirhams.
async fn set_off(
    app: &TestApp,
    by: &User,
    party_code: &str,
    invoice: &Invoice,
    bill: &SupplierBill,
    amount: &str,
) -> TestResponse {
    let entity_id = entity(app, "GBC").await.id.to_string();
    let invoice_id = invoice.id.to_string();
    let bill_id = bill.id.to_string();
    post(
        app,
        by,
        "/payments",
        &[
            ("legal_entity_id", entity_id.as_str()),
            ("party_code", party_code),
            ("method", "set_off"),
            ("amount", amount),
            ("set_off_invoice_id", invoice_id.as_str()),
            ("set_off_bill_id", bill_id.as_str()),
        ],
    )
    .await
}

async fn invoice_now(app: &TestApp, id: i64) -> Invoice {
    Invoice::find_or_404(app.db(), id).await.unwrap()
}

async fn bill_now(app: &TestApp, id: i64) -> SupplierBill {
    SupplierBill::find_or_404(app.db(), id).await.unwrap()
}

/// IPA 6 certified for AED 180,000 with BC-12 (AED 5,000) and LD-1 (AED
/// 30,000) accepted and CC-3 (AED 12,000) contested, invoiced by the
/// accountant: the real tax invoice, credit note, dispute and supplier bill.
/// The bill is approved and set off; the set-off is reversed; the client is
/// blocked and the set-off waits for the exception.
#[renox::test]
async fn invoicing_a_certified_ipa_and_setting_off_the_back_charge_with_the_real_modules() {
    let app = boot().await;
    let pm = person(&app, "Amal", PROJECT_MANAGER).await;
    let accountant = accountant(&app, "Aisha").await;
    let finance = person(&app, "Fiona", FINANCE_MANAGER).await;
    let client = company(&app, "Palm Developments LLC").await;
    let code = wl_parties::Party::find_or_404(app.db(), client.0)
        .await
        .unwrap()
        .code;
    let project = project(&app, "TWR-01", client, ProjectStatus::InProgress, &pm).await;
    Project::query()
        .where_(Project::ID.eq(project.id))
        .update(app.db(), &[("retention_bps", &1_000_i64)])
        .await
        .unwrap();
    let certified = ipa(
        &app,
        &project,
        IpaKind::Interim,
        IpaStatus::Certified,
        aed(180_000),
    )
    .await;
    deduction(
        &app,
        &certified,
        DeductionKind::BackCharge,
        "BC-12",
        aed(5_000),
        DeductionStatus::Accepted,
    )
    .await;
    deduction(
        &app,
        &certified,
        DeductionKind::LiquidatedDamages,
        "LD-1",
        aed(30_000),
        DeductionStatus::Accepted,
    )
    .await;
    deduction(
        &app,
        &certified,
        DeductionKind::ContraCharge,
        "CC-3",
        aed(12_000),
        DeductionStatus::Contested,
    )
    .await;

    ok(post(
        &app,
        &accountant,
        &format!("/construction/ipas/{}/invoice", certified.id),
        &[],
    )
    .await);

    let done = Ipa::find_or_404(app.db(), certified.id).await.unwrap();
    assert_eq!(done.status, IpaStatus::Invoiced);

    // The tax invoice: AED 180,000 + 9,000 VAT.
    let invoice = invoice_now(&app, done.invoice_id.expect("the invoice")).await;
    assert_eq!(invoice.net_total, aed(180_000));
    assert_eq!(invoice.vat_total, aed(9_000));
    assert_eq!(invoice.gross_total, aed(189_000));
    assert_eq!(invoice.party_id, client.0);

    // The credit note for the accepted liquidated damages: AED 30,000 plus
    // the AED 1,500 VAT of the invoice line.
    let note = CreditNote::find_or_404(app.db(), done.credit_note_id.expect("the credit note"))
        .await
        .unwrap();
    assert_eq!(note.net_total, aed(30_000));
    assert_eq!(note.vat_total, aed(1_500));
    assert_eq!(note.gross_total, aed(31_500));

    // The contested part is disputed: AED 12,000 + 600 VAT, and the
    // reminders do not run for the invoice.
    let dispute = Dispute::find(app.db(), done.dispute_id.expect("the dispute"))
        .await
        .unwrap()
        .expect("the dispute row");
    assert_eq!(dispute.amount, aed(12_600));
    assert!(invoice.disputed);
    let sent = wl_invoicing::reminders::run(app.state(), day(120))
        .await
        .unwrap();
    assert_eq!(sent, 0, "a disputed invoice is not chased");

    // The accepted back-charge is the client's bill: AED 5,000 + 250 VAT.
    let bill = bill_now(&app, done.supplier_bill_id.expect("the bill")).await;
    assert_eq!(bill.supplier(), client);
    assert_eq!(bill.net_total, aed(5_000));
    assert_eq!(bill.gross_total, aed(5_250));
    if bill.status == SupplierBillStatus::Draft {
        submit(app.state(), By::System(Some(accountant.id)), bill.id)
            .await
            .unwrap();
    }
    approve_bill(app.state(), By::System(Some(finance.id)), bill.id, "")
        .await
        .unwrap();
    let bill = bill_now(&app, bill.id).await;
    assert_eq!(bill.status, SupplierBillStatus::Approved);

    // Set off AED 5,250.00 against the invoice: its balance drops by that,
    // the bill is paid, and no bank account is touched.
    let before = invoice_now(&app, invoice.id).await.balance;
    set_off(&app, &accountant, &code, &invoice, &bill, "5250.00")
        .await
        .assert_status(303);
    assert_eq!(
        invoice_now(&app, invoice.id).await.balance,
        before - aed(5_250)
    );
    let paid = bill_now(&app, bill.id).await;
    assert_eq!(paid.status, SupplierBillStatus::Paid);
    assert_eq!(paid.balance, 0);
    let payment = Payment::query()
        .where_(Payment::METHOD.eq(PaymentMethod::SetOff))
        .first(app.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(payment.method, PaymentMethod::SetOff);
    assert_eq!(payment.status, PaymentStatus::Received);
    assert_eq!(payment.bank_account, None);

    // Reversing it reopens both.
    ok(post(
        &app,
        &accountant,
        &format!("/payments/{}/reverse", payment.id),
        &[("reason", "entered_in_error"), ("note", "Wrong bill")],
    )
    .await);
    assert_eq!(invoice_now(&app, invoice.id).await.balance, before);
    let reopened = bill_now(&app, bill.id).await;
    assert_eq!(reopened.status, SupplierBillStatus::Approved);
    assert_eq!(reopened.balance, aed(5_250));

    // A blocked client: the set-off waits for the exception, then runs.
    block(&app, client).await;
    set_off(&app, &accountant, &code, &invoice, &reopened, "5250.00")
        .await
        .assert_status(303);
    assert_eq!(invoice_now(&app, invoice.id).await.balance, before);
    assert_eq!(
        bill_now(&app, bill.id).await.status,
        SupplierBillStatus::Approved
    );
    let requests = ApprovalRequest::query()
        .where_(ApprovalRequest::ACTION.eq(BLOCK_EXCEPTION_ACTION))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    approvals::approve(app.state(), requests[0].id, &finance, "approved")
        .await
        .unwrap();
    assert_eq!(
        invoice_now(&app, invoice.id).await.balance,
        before - aed(5_250)
    );
    assert_eq!(
        bill_now(&app, bill.id).await.status,
        SupplierBillStatus::Paid
    );
}

/// The invoiced final retention part (AED 50,000 + 2,500 VAT) is disputed by
/// the accountant for a client deduction of AED 8,000 (+ 400 VAT); the dispute
/// resolved `credited` for AED 8,400 is a credit note of AED 8,000 + 400.
#[renox::test]
async fn a_disputed_retention_invoice_resolves_credited_as_a_real_credit_note() {
    let app = boot().await;
    let pm = person(&app, "Amal", PROJECT_MANAGER).await;
    let accountant = accountant(&app, "Aisha").await;
    let md = person(&app, "Omar", ADMIN).await;
    let client = company(&app, "Palm Developments LLC").await;
    let project = project(
        &app,
        "TWR-02",
        client,
        ProjectStatus::PracticalCompletion,
        &pm,
    )
    .await;
    for (part, status) in [
        (RetentionPart::First, RetentionStatus::Invoiced),
        (RetentionPart::Final, RetentionStatus::Due),
    ] {
        let mut row = Retention::new(project.id, part, aed(50_000));
        row.status = status;
        let mut tx = app.db().begin().await.unwrap();
        row.insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    let part = Retention::query()
        .where_(Retention::PROJECT_ID.eq(project.id))
        .where_(Retention::PART.eq(RetentionPart::Final))
        .first(app.db())
        .await
        .unwrap()
        .unwrap();

    ok(post(
        &app,
        &accountant,
        &format!("/construction/retentions/{}/invoice", part.id),
        &[],
    )
    .await);
    let invoiced = Retention::find_or_404(app.db(), part.id).await.unwrap();
    assert_eq!(invoiced.status, RetentionStatus::Invoiced);
    let invoice = invoice_now(&app, invoiced.invoice_id.expect("the invoice")).await;
    assert_eq!(invoice.net_total, aed(50_000));
    assert_eq!(invoice.vat_total, aed(2_500));

    ok(post(
        &app,
        &accountant,
        &format!("/construction/retentions/{}/dispute", part.id),
        &[("amount", "800000"), ("reason", "Defects")],
    )
    .await);
    let disputed = invoice_now(&app, invoice.id).await;
    assert!(disputed.disputed);
    let dispute = Dispute::find(app.db(), disputed.dispute_id.expect("the dispute"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(dispute.amount, aed(8_400), "AED 8,000 + VAT 400");

    ok(post(
        &app,
        &accountant,
        &format!("/invoices/{}/dispute/resolve", invoice.id),
        &[
            ("outcome", "credited"),
            ("note", "Defects accepted"),
            ("agreed_amount", "8400.00"),
        ],
    )
    .await);

    // A credit above the limit waits for a second person (`FIN.ACL.07`); the
    // request is about the credit note.
    let waiting = ApprovalRequest::query()
        .where_(ApprovalRequest::STATUS.eq(wl_kernel::approvals::ApprovalStatus::Pending))
        .get(app.db())
        .await
        .unwrap();
    for request in &waiting {
        approvals::approve(app.state(), request.id, &md, "agreed")
            .await
            .unwrap();
    }
    let resolved = invoice_now(&app, invoice.id).await;
    assert!(!resolved.disputed, "pending: {waiting:?}");
    let notes = CreditNote::query()
        .where_(CreditNote::INVOICE_ID.eq(invoice.id))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].net_total, aed(8_000));
    assert_eq!(notes[0].vat_total, aed(400));
    assert_eq!(notes[0].gross_total, aed(8_400));
}

/// The final account invoice of a terminated project is written off once
/// another user approves it, and the project still closes with it.
#[renox::test]
async fn a_final_account_invoice_written_off_after_approval_lets_the_project_close() {
    let app = boot().await;
    let pm = person(&app, "Amal", PROJECT_MANAGER).await;
    let accountant = accountant(&app, "Aisha").await;
    let md = person(&app, "Omar", ADMIN).await;
    let client = company(&app, "Palm Developments LLC").await;
    let project = project(&app, "TWR-03", client, ProjectStatus::Terminated, &pm).await;
    let account = ipa(
        &app,
        &project,
        IpaKind::FinalAccount,
        IpaStatus::Certified,
        aed(85_000),
    )
    .await;
    ok(post(
        &app,
        &accountant,
        &format!("/construction/ipas/{}/invoice", account.id),
        &[],
    )
    .await);
    let invoice_id = Ipa::find_or_404(app.db(), account.id)
        .await
        .unwrap()
        .invoice_id
        .expect("the final account invoice");
    let settle = format!("/construction/projects/{}/settle", project.id);

    ok(post(
        &app,
        &accountant,
        &format!("/invoices/{invoice_id}/write-off"),
        &[("reason_code", "insolvent"), ("reason_text", "")],
    )
    .await);
    let pending =
        ApprovalRequest::pending_about(app.db(), &invoice_now(&app, invoice_id).await.subject())
            .await
            .unwrap();
    assert_eq!(pending.len(), 1);

    approvals::approve(app.state(), pending[0].id, &md, "bad debt")
        .await
        .unwrap();
    assert_eq!(
        invoice_now(&app, invoice_id).await.status,
        InvoiceStatus::WrittenOff
    );

    post(&app, &pm, &settle, &[]).await.assert_ok();
    let closed = Project::find_or_404(app.db(), project.id).await.unwrap();
    assert_eq!(closed.status, ProjectStatus::Closed);
}
