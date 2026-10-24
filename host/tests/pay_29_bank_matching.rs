//! BL-PAY-29 (#468) with the real modules of the Property host: the bank
//! statement lines are suggested for the tenant's open invoices, confirmed or
//! matched by hand, the payments are recorded at `complete`, a deposited
//! cheque is cleared, and the reversal of a confirmed match waits for a
//! property manager (`PRP.ACL.16`).

mod support;

use host::roles::{ACCOUNTANT, PROPERTY_MANAGER, RECEIVABLES};
use property::UnitUse;
use renox::prelude::*;
use renox::testing::TestApp;
use support::*;
use wl_invoicing::Invoice;
use wl_kernel::PartyId;
use wl_kernel::approvals;
use wl_kernel::cheques::{ChequeStatus, RegisterCheque, cheques};
use wl_kernel::invoicing::{InvoiceStatus, VatCategory};
use wl_kernel::payments::PaymentStatus;
use wl_payments::bank_match;
use wl_payments::bank_statement::{BankStatementImport, ImportStatus};
use wl_payments::bank_statement_line::{BankStatementLine, LineStatus};
use wl_payments::{Allocation, Cheque, Payment};

/// Uploads a CSV statement of ENBD-1 for the last ten days with `rows`
/// (`date,description,amount,reference,counterparty`) and reads it.
async fn imported(app: &TestApp, accountant: &User, rows: &[String]) -> BankStatementImport {
    let entity = entity(app, "PGP").await.id.to_string();
    let mut content = String::from("date,description,amount,reference,counterparty\n");
    for row in rows {
        content.push_str(row);
        content.push('\n');
    }
    let start = day(-10).to_string();
    let end = day(0).to_string();
    app.acting_as(accountant);
    app.htmx()
        .post(
            "/bank-statements",
            &[
                ("legal_entity_id", entity.as_str()),
                ("bank_account", "ENBD-1"),
                ("file_name", "statement.csv"),
                ("format", "csv"),
                ("period_start", start.as_str()),
                ("period_end", end.as_str()),
                ("closing_balance", "100000"),
                ("content", content.as_str()),
            ],
        )
        .await
        .assert_redirect("/bank-statements");
    let import = BankStatementImport::query()
        .order_by_desc(BankStatementImport::ID)
        .first(app.db())
        .await
        .unwrap()
        .unwrap();
    app.htmx()
        .post(&format!("/bank-statements/{}/parse", import.id), &[])
        .await
        .assert_ok();
    BankStatementImport::find_or_404(app.db(), import.id)
        .await
        .unwrap()
}

async fn lines(app: &TestApp, import: &BankStatementImport) -> Vec<BankStatementLine> {
    BankStatementLine::of_import(app.db(), import.id)
        .await
        .unwrap()
}

async fn post(app: &TestApp, path: &str, pairs: &[(&str, &str)]) -> u16 {
    app.htmx().post(path, pairs).await.status.as_u16()
}

struct Scene {
    app: TestApp,
    accountant: User,
    clerk: User,
    tenant: PartyId,
}

async fn scene() -> Scene {
    let app = booted().await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let tenant = company(&app, "Al Noor Trading LLC").await;
    Scene {
        app,
        accountant,
        clerk,
        tenant,
    }
}

impl Scene {
    async fn invoice(&self, fils: i64) -> Invoice {
        issued_invoice(
            &self.app,
            &self.clerk,
            self.tenant,
            fils,
            VatCategory::Exempt,
            day(20),
        )
        .await
    }
}

#[renox::test]
async fn a_line_with_the_invoice_number_is_suggested_and_paid_at_complete() {
    let s = scene().await;
    let invoice = s.invoice(450_000).await;
    let number = invoice.number.clone().unwrap();
    let import = imported(
        &s.app,
        &s.accountant,
        &[format!(
            "{},Transfer,4500.00,{number},AL NOOR TRADING",
            day(-1)
        )],
    )
    .await;
    assert_eq!(import.status, ImportStatus::InReview);
    let line = lines(&s.app, &import).await.remove(0);
    assert_eq!(line.status, LineStatus::Suggested);
    assert_eq!(
        line.suggestion_kind.as_deref(),
        Some(bank_match::KIND_INVOICES)
    );
    assert_eq!(line.suggestion_reason.as_deref(), Some("reference"));
    assert_eq!(line.suggestion_party_id, Some(s.tenant.0));
    assert_eq!(
        line.suggestion_targets.clone().unwrap().0,
        vec![(invoice.id, 450_000)]
    );
    // The page shows the suggestion.
    s.app
        .get(&format!("/bank-statements/{}", import.id))
        .await
        .assert_ok()
        .assert_see(&number)
        .assert_see("Confirm");

    assert_eq!(
        post(
            &s.app,
            &format!("/bank-statement-lines/{}/confirm", line.id),
            &[]
        )
        .await,
        200
    );
    // Nothing is recorded before complete.
    assert_eq!(
        Payment::query()
            .where_(Payment::STATEMENT_LINE_ID.eq(line.id))
            .count(s.app.db())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        post(
            &s.app,
            &format!("/bank-statements/{}/complete", import.id),
            &[]
        )
        .await,
        200
    );
    let payments = Payment::query()
        .where_(Payment::STATEMENT_LINE_ID.eq(line.id))
        .get(s.app.db())
        .await
        .unwrap();
    assert_eq!(payments.len(), 1, "one transfer");
    assert_eq!(payments[0].amount, 450_000);
    assert_eq!(
        payments[0].method,
        wl_kernel::payments::PaymentMethod::Transfer
    );
    let rows = Allocation::all_of(s.app.db(), payments[0].id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(reload(&s.app, &invoice).await.status, InvoiceStatus::Paid);
}

#[renox::test]
async fn a_line_matched_by_hand_to_two_invoices_records_one_payment_with_two_allocations() {
    let s = scene().await;
    let first = s.invoice(450_000).await;
    let second = s.invoice(450_000).await;
    let import = imported(
        &s.app,
        &s.accountant,
        &[format!("{},Rent,9000.00,TRF-778,SOMEONE ELSE", day(-1))],
    )
    .await;
    let line = lines(&s.app, &import).await.remove(0);
    let allocations = format!(
        "{} | 4500\n{} | 4500",
        first.number.clone().unwrap(),
        second.number.clone().unwrap()
    );
    assert_eq!(
        post(
            &s.app,
            &format!("/bank-statement-lines/{}/match", line.id),
            &[("allocations", allocations.as_str())],
        )
        .await,
        200
    );
    assert_eq!(
        post(
            &s.app,
            &format!("/bank-statements/{}/complete", import.id),
            &[]
        )
        .await,
        200
    );
    let payments = Payment::query()
        .where_(Payment::STATEMENT_LINE_ID.eq(line.id))
        .get(s.app.db())
        .await
        .unwrap();
    assert_eq!(payments.len(), 1);
    assert_eq!(payments[0].amount, 900_000);
    assert_eq!(
        Allocation::all_of(s.app.db(), payments[0].id)
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(reload(&s.app, &first).await.status, InvoiceStatus::Paid);
    assert_eq!(reload(&s.app, &second).await.status, InvoiceStatus::Paid);
}

#[renox::test]
async fn a_line_for_a_deposited_cheque_clears_it_without_another_payment() {
    let s = scene().await;
    let unit = unit_of(&s.app, "TWR-B", "501", UnitUse::Residential).await;
    let lease = draft_lease(&s.app, &unit, s.tenant, -30).await;
    let rent = s.invoice(2_500_000).await;
    let entity = entity(&s.app, "PGP").await;
    let summary = cheques(s.app.state())
        .register(
            s.app.state(),
            RegisterCheque::new(s.tenant, entity.id, "ENBD", "000451", day(0), 2_500_000)
                .link(rent.subject())
                .agreement(lease.subject()),
            Some(s.accountant.id),
        )
        .await
        .expect("the cheque is registered");
    s.app.acting_as(&s.accountant);
    assert_eq!(
        post(&s.app, &format!("/cheques/{}/deposit", summary.id.0), &[]).await,
        200
    );
    let import = imported(
        &s.app,
        &s.accountant,
        &[format!("{},Cheque clearing,25000.00,CHQ 000451,", day(0))],
    )
    .await;
    let line = lines(&s.app, &import).await.remove(0);
    assert_eq!(
        line.suggestion_kind.as_deref(),
        Some(bank_match::KIND_CHEQUE)
    );
    assert_eq!(
        post(
            &s.app,
            &format!("/bank-statement-lines/{}/confirm", line.id),
            &[]
        )
        .await,
        200
    );
    assert_eq!(
        post(
            &s.app,
            &format!("/bank-statements/{}/complete", import.id),
            &[]
        )
        .await,
        200
    );
    let cheque = Cheque::find_or_404(s.app.db(), summary.id.0).await.unwrap();
    assert_eq!(cheque.status, ChequeStatus::Cleared);
    assert_eq!(
        Payment::query()
            .where_(Payment::PARTY_ID.eq(s.tenant.0))
            .count(s.app.db())
            .await
            .unwrap(),
        1,
        "only the cheque's own payment"
    );
    assert_eq!(
        Payment::query()
            .where_(Payment::STATEMENT_LINE_ID.eq(line.id))
            .count(s.app.db())
            .await
            .unwrap(),
        0
    );
}

#[renox::test]
async fn an_accountant_cannot_reverse_a_confirmed_match_a_property_manager_approves_it() {
    let s = scene().await;
    let manager = person(&s.app, "Pam", PROPERTY_MANAGER).await;
    let first = s.invoice(450_000).await;
    let second = s.invoice(450_000).await;
    let import = imported(
        &s.app,
        &s.accountant,
        &[format!("{},Rent,9000.00,TRF-900,", day(-1))],
    )
    .await;
    let line = lines(&s.app, &import).await.remove(0);
    let allocations = format!(
        "{}\n{}",
        first.number.clone().unwrap(),
        second.number.clone().unwrap()
    );
    post(
        &s.app,
        &format!("/bank-statement-lines/{}/match", line.id),
        &[("allocations", allocations.as_str())],
    )
    .await;
    assert_eq!(
        post(
            &s.app,
            &format!("/bank-statements/{}/complete", import.id),
            &[]
        )
        .await,
        200
    );
    let payment = bank_match::payment_of_line(s.app.db(), line.id)
        .await
        .unwrap()
        .expect("the payment of the line");

    // The accountant asks: a request, nothing reversed.
    assert_eq!(
        post(
            &s.app,
            &format!("/bank-statement-lines/{}/reverse", line.id),
            &[]
        )
        .await,
        200
    );
    assert_eq!(
        Payment::find_or_404(s.app.db(), payment.id)
            .await
            .unwrap()
            .status,
        PaymentStatus::Received
    );
    assert_eq!(reload(&s.app, &first).await.status, InvoiceStatus::Paid);
    let mut pending = approvals::pending_for(s.app.db(), &payment.subject())
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    let request = pending.remove(0);
    let own = approvals::approve(s.app.state(), request.id, &s.accountant, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");

    approvals::approve(s.app.state(), request.id, &manager, "wrong tenant")
        .await
        .unwrap()
        .expect("the approval runs");
    let reversed = Payment::find_or_404(s.app.db(), payment.id).await.unwrap();
    assert_eq!(reversed.status, PaymentStatus::Reversed);
    assert_eq!(
        reversed.reversal_note.as_deref(),
        Some(bank_match::REVERSE_NOTE)
    );
    for invoice in [&first, &second] {
        let open = reload(&s.app, invoice).await;
        assert_eq!(open.status, InvoiceStatus::Issued, "open again");
        assert!(!open.disputed, "reminders resume");
        assert_eq!(open.balance, 450_000);
    }
}

#[renox::test]
async fn revert_of_an_import_in_review_removes_its_lines_and_records_nothing() {
    let s = scene().await;
    let invoice = s.invoice(450_000).await;
    let number = invoice.number.clone().unwrap();
    let import = imported(
        &s.app,
        &s.accountant,
        &[format!("{},Transfer,4500.00,{number},", day(-1))],
    )
    .await;
    let line = lines(&s.app, &import).await.remove(0);
    post(
        &s.app,
        &format!("/bank-statement-lines/{}/confirm", line.id),
        &[],
    )
    .await;
    assert_eq!(
        post(
            &s.app,
            &format!("/bank-statements/{}/revert", import.id),
            &[]
        )
        .await,
        200
    );
    assert!(lines(&s.app, &import).await.is_empty());
    assert_eq!(
        Payment::query()
            .where_(Payment::PARTY_ID.eq(s.tenant.0))
            .count(s.app.db())
            .await
            .unwrap(),
        0
    );
    assert_eq!(reload(&s.app, &invoice).await.status, InvoiceStatus::Issued);
}

#[renox::test]
async fn a_line_with_no_candidate_stays_unmatched_and_blocks_complete() {
    let s = scene().await;
    s.invoice(450_000).await;
    let import = imported(
        &s.app,
        &s.accountant,
        &[format!("{},Unknown transfer,1234.00,XYZ-1,NOBODY", day(-1))],
    )
    .await;
    let line = lines(&s.app, &import).await.remove(0);
    assert_eq!(line.status, LineStatus::Unmatched);
    assert!(line.suggestion_kind.is_none());
    assert_eq!(
        post(
            &s.app,
            &format!("/bank-statements/{}/complete", import.id),
            &[]
        )
        .await,
        422
    );
    assert_eq!(
        BankStatementImport::find_or_404(s.app.db(), import.id)
            .await
            .unwrap()
            .status,
        ImportStatus::InReview
    );
}
