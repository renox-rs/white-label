//! The reads the cash-flow forecast needs from this module (BL-PAY-25, #118,
//! `FIN.CFL.01`): the cheques not cleared with their open amounts and
//! documents, the latest closing balance of each bank account, the payments
//! of an account after a date and the refunds approved or failed but not
//! paid. The reads change nothing.

mod support;

use renox::chrono::{NaiveDate, TimeDelta};
use renox::prelude::*;
use wl_kernel::bank_statements::bank_statements;
use wl_kernel::cheques::{ChequeStatus, RegisterCheque, cheques as cheque_contract};
use wl_kernel::documents::today;
use wl_kernel::payments::{Direction, PaymentMethod, RecordPayment, RefundSource, payment_recorder};
use wl_kernel::Subject;
use wl_payments::bank_statement::{BankStatementImport, ImportStatus};
use wl_payments::refund::RefundStatus;
use wl_payments::{Cheque, Payment, Refund, permissions as perm};

use support::{BLUE_DUNES, Booted, boot};

fn in_days(days: i64) -> NaiveDate {
    today() + TimeDelta::days(days)
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

async fn cheque_on(t: &Booted, number: &str, on: NaiveDate, amount: i64) -> Cheque {
    let entity = t.entity("PGP").await;
    t.register_cheque(RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", number, on, amount))
        .await
}

async fn open(t: &Booted, direction: Direction) -> Vec<wl_kernel::cheques::OpenCheque> {
    let entity = t.entity("PGP").await;
    cheque_contract(t.state())
        .not_cleared(t.state(), entity.id, direction)
        .await
        .unwrap()
}

/// Received, due-for-deposit and deposited cheques are open, in cheque date
/// order; cleared, bounced and returned ones, issued ones, and another
/// entity's are not.
#[renox::test]
async fn the_cheques_not_cleared_are_listed_by_date() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let later = cheque_on(&t, "3002", in_days(60), 700_000).await;
    let soon = cheque_on(&t, "3001", in_days(30), 500_000).await;
    let deposited = cheque_on(&t, "3003", today(), 300_000).await;
    let deposited = t.deposit_cheque(&staff, &deposited).await;
    assert_eq!(deposited.status, ChequeStatus::Deposited);

    let cleared = cheque_on(&t, "3004", today(), 100_000).await;
    let cleared = t.deposit_cheque(&staff, &cleared).await;
    t.cheque_action(&staff, &cleared, "clear", &[]).await.assert_ok();
    let bounced = t
        .bounced_cheque(&staff, BLUE_DUNES, "3005", 200_000, None, "")
        .await;
    assert_eq!(bounced.status, ChequeStatus::Bounced);

    // An issued cheque is money out; another entity's is not ours.
    let pgp = t.entity("PGP").await;
    let other = t.entity("PGFZ").await;
    t.register_cheque(
        RegisterCheque::new(BLUE_DUNES, pgp.id, "ENBD", "3006", in_days(10), 900_000).issued(),
    )
    .await;
    t.register_cheque(RegisterCheque::new(
        BLUE_DUNES, other.id, "ENBD", "3007", in_days(10), 800_000,
    ))
    .await;

    let received = open(&t, Direction::In).await;
    let numbers: Vec<&str> = received.iter().map(|c| c.cheque.number.as_str()).collect();
    assert_eq!(numbers, ["3003", "3001", "3002"]);
    assert_eq!(received[1].cheque.id.0, soon.id);
    assert_eq!(received[2].cheque.id.0, later.id);
    assert_eq!(received[0].cheque.status, ChequeStatus::Deposited);
    assert_eq!(received[1].cheque.open_amount, 500_000);
    assert_eq!(received[1].cheque.date, in_days(30));

    let issued = open(&t, Direction::Out).await;
    assert_eq!(issued.len(), 1);
    assert_eq!(issued[0].cheque.number, "3006");
}

/// A cheque that bounced is not open; once it is re-presented and deposited
/// it is open again, for what is still owed.
#[renox::test]
async fn a_re_presented_cheque_is_open_again() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let bounced = t
        .bounced_cheque(&staff, BLUE_DUNES, "3101", 2_000_000, None, "")
        .await;
    assert!(open(&t, Direction::In).await.is_empty());
    let on = in_days(3).format("%Y-%m-%d").to_string();
    t.represent_cheque(&staff, &bounced, &on, true, "")
        .await
        .assert_ok();
    let read = open(&t, Direction::In).await;
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].cheque.status, ChequeStatus::Deposited);
    assert_eq!(read[0].cheque.open_amount, 2_000_000);
}

/// Each linked document comes with the part of the cheque applied to it: an
/// invoice up to its balance, the next document with what is left.
#[renox::test]
async fn the_documents_a_cheque_pays_come_with_the_amount_applied() {
    let t = boot().await;
    let entity = t.entity("PGP").await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-1", 400_000);
    let charge = Subject::new("invoicing.charge", 5);
    t.register_cheque(
        RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "3201", in_days(20), 1_000_000)
            .link(rent.subject())
            .link(charge.clone()),
    )
    .await;
    let read = open(&t, Direction::In).await;
    let documents = &read[0].documents;
    assert_eq!(documents.len(), 2);
    assert_eq!(documents[0].document, rent.subject());
    assert_eq!(documents[0].amount, 400_000);
    assert_eq!(documents[1].document, charge);
    assert_eq!(documents[1].amount, 600_000);
    // A cheque without a link pays nothing named.
    cheque_on(&t, "3202", in_days(25), 100_000).await;
    assert!(open(&t, Direction::In).await[1].documents.is_empty());
}

async fn import(
    t: &Booted,
    account: &str,
    end: NaiveDate,
    balance: Option<i64>,
    status: ImportStatus,
    completed_after_secs: i64,
) -> BankStatementImport {
    let entity = t.entity("PGP").await;
    let mut row = BankStatementImport {
        legal_entity_id: entity.id,
        bank_account: account.to_owned(),
        file_name: "statement.csv".into(),
        format: "csv".into(),
        currency: "AED".into(),
        period_start: end - TimeDelta::days(30),
        period_end: end,
        closing_balance: balance,
        status,
        completed_at: Some(renox::db::now() + TimeDelta::seconds(completed_after_secs)),
        ..Default::default()
    };
    row.insert(t.db()).await.unwrap();
    row
}

async fn balances(t: &Booted) -> Vec<wl_kernel::bank_statements::BankBalance> {
    let entity = t.entity("PGP").await;
    bank_statements(t.state())
        .latest_closing_balances(t.db(), entity.id)
        .await
        .unwrap()
}

/// The balance is dated the last day of the period; the import whose period
/// ends last wins, not the one imported last.
#[renox::test]
async fn an_older_period_imported_later_does_not_replace_the_balance() {
    let t = boot().await;
    let january = import(&t, "ENBD-1", date(2027, 1, 31), Some(900_000), ImportStatus::Completed, 0).await;
    import(&t, "ENBD-1", date(2026, 12, 31), Some(100_000), ImportStatus::Completed, 10).await;
    let read = balances(&t).await;
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].amount, 900_000);
    assert_eq!(read[0].as_of, date(2027, 1, 31));
    assert_eq!(read[0].import_id, january.id);
    assert_eq!(read[0].bank_account, "ENBD-1");
}

/// An import without a balance, a reverted one and one not completed are
/// never used; the latest one with a balance is.
#[renox::test]
async fn only_a_completed_import_with_a_balance_counts() {
    let t = boot().await;
    let good = import(&t, "ENBD-1", date(2027, 1, 31), Some(-250_000), ImportStatus::Completed, 0).await;
    import(&t, "ENBD-1", date(2027, 2, 28), None, ImportStatus::Completed, 5).await;
    import(&t, "ENBD-1", date(2027, 3, 31), Some(1), ImportStatus::Reverted, 6).await;
    import(&t, "ENBD-1", date(2027, 4, 30), Some(2), ImportStatus::Uploaded, 7).await;
    import(&t, "ENBD-1", date(2027, 5, 31), Some(3), ImportStatus::Failed, 8).await;
    let read = balances(&t).await;
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].import_id, good.id);
    // A balance below 0 is a balance.
    assert_eq!(read[0].amount, -250_000);

    let t = boot().await;
    import(&t, "ENBD-1", date(2027, 3, 31), None, ImportStatus::Completed, 0).await;
    assert!(balances(&t).await.is_empty());
}

/// On the same period end the import completed last wins; accounts are kept
/// apart and ordered by name.
#[renox::test]
async fn the_import_completed_last_wins_on_the_same_period_end() {
    let t = boot().await;
    import(&t, "ENBD-1", date(2027, 1, 31), Some(100_000), ImportStatus::Completed, 0).await;
    let corrected = import(&t, "ENBD-1", date(2027, 1, 31), Some(110_000), ImportStatus::Completed, 30).await;
    import(&t, "MASHREQ-2", date(2027, 1, 15), Some(5_000), ImportStatus::Completed, 0).await;
    let read = balances(&t).await;
    assert_eq!(read.len(), 2);
    assert_eq!(read[0].bank_account, "ENBD-1");
    assert_eq!(read[0].import_id, corrected.id);
    assert_eq!(read[0].amount, 110_000);
    assert_eq!(read[1].bank_account, "MASHREQ-2");

    // Another entity's accounts are not returned.
    let other = t.entity("PGFZ").await;
    let none = bank_statements(t.state())
        .latest_closing_balances(t.db(), other.id)
        .await
        .unwrap();
    assert!(none.is_empty());
}

async fn upload(t: &Booted, pairs: Vec<(String, String)>) -> renox::testing::TestResponse {
    let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    t.app.htmx().post("/bank-statements", &refs).await
}

/// The accountant records an upload with the closing balance; the period must
/// not end before it starts, and the page is for `bank.reconcile`.
#[renox::test]
async fn the_accountant_enters_the_closing_balance_at_upload() {
    let t = boot().await;
    let entity = t.entity("PGP").await;
    let accountant = t.user("Dina", &[perm::VIEW, perm::BANK_RECONCILE]).await;
    let clerk = t.user("Omar", &[perm::VIEW]).await;
    let id = entity.id.to_string();
    let form = |end: &str, balance: &str| -> Vec<(String, String)> {
        [
            ("legal_entity_id", id.as_str()),
            ("bank_account", "ENBD-1"),
            ("file_name", "jan.csv"),
            ("format", "csv"),
            ("period_start", "2027-01-01"),
            ("period_end", end),
            ("closing_balance", balance),
        ]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
    };

    t.app.acting_as(&clerk);
    upload(&t, form("2027-01-31", "1,250.50")).await.assert_forbidden();

    t.app.acting_as(&accountant);
    upload(&t, form("2026-12-01", "10")).await.assert_status(422);
    upload(&t, form("2027-01-31", "ten")).await.assert_status(422);
    assert_eq!(BankStatementImport::query().count(t.db()).await.unwrap(), 0);

    upload(&t, form("2027-01-31", "1,250.50")).await.assert_ok();
    let rows = BankStatementImport::query().get(t.db()).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].closing_balance, Some(125_050));
    assert_eq!(rows[0].period_end, date(2027, 1, 31));
    assert_eq!(rows[0].status, ImportStatus::Uploaded);
    assert_eq!(rows[0].imported_by, Some(accountant.id));
    // An upload is not a completed import: no balance is read from it yet.
    assert!(balances(&t).await.is_empty());

    // The balance may be left out: the file carries it.
    upload(&t, form("2027-02-28", "")).await.assert_ok();
    let rows = BankStatementImport::query().get(t.db()).await.unwrap();
    assert_eq!(rows[1].closing_balance, None);

    t.app.get("/bank-statements").await.assert_ok().assert_see("ENBD-1");
}

async fn pay(
    t: &Booted,
    direction: Direction,
    amount: i64,
    on: NaiveDate,
    account: Option<&str>,
    method: PaymentMethod,
    reference: &str,
) -> Payment {
    let entity = t.entity("PGP").await;
    let mut request = RecordPayment::new(BLUE_DUNES, entity.id, method, amount)
        .date(on)
        .reference(reference);
    if direction == Direction::Out {
        request = request.outgoing();
    }
    if let Some(account) = account {
        request = request.bank_account(account);
    }
    t.receive_request(request).await
}

async fn after(t: &Booted, account: &str, on: NaiveDate) -> Vec<wl_kernel::payments::BankPayment> {
    let entity = t.entity("PGP").await;
    payment_recorder(t.state())
        .payments_after(t.db(), entity.id, account, on)
        .await
        .unwrap()
}

/// Only payments dated strictly after the day are returned, by the payment's
/// date; the reference (a VAT summary) and the direction come with them.
#[renox::test]
async fn the_payments_after_a_date_are_those_dated_after_it() {
    let t = boot().await;
    let on = date(2027, 1, 31);
    pay(&t, Direction::In, 100_000, on, Some("ENBD-1"), PaymentMethod::Transfer, "R-1").await;
    let next = pay(&t, Direction::In, 200_000, date(2027, 2, 1), Some("ENBD-1"), PaymentMethod::Transfer, "R-2").await;
    // Recorded now but dated long ago: its date decides.
    pay(&t, Direction::In, 300_000, date(2026, 6, 1), Some("ENBD-1"), PaymentMethod::Transfer, "R-3").await;
    let vat = pay(&t, Direction::Out, 400_000, date(2027, 2, 28), Some("ENBD-1"), PaymentMethod::Transfer, "VAT-2027-Q1").await;
    // Another account, and none.
    pay(&t, Direction::In, 500_000, date(2027, 2, 2), Some("MASHREQ-2"), PaymentMethod::Transfer, "R-5").await;
    pay(&t, Direction::In, 600_000, date(2027, 2, 3), None, PaymentMethod::Cash, "R-6").await;

    let read = after(&t, "ENBD-1", on).await;
    let numbers: Vec<i64> = read.iter().map(|p| p.payment.id.0).collect();
    assert_eq!(numbers, [next.id, vat.id]);
    assert_eq!(read[0].payment.amount, 200_000);
    assert_eq!(read[0].payment.direction, Direction::In);
    assert_eq!(read[0].reference.as_deref(), Some("R-2"));
    assert_eq!(read[1].payment.direction, Direction::Out);
    assert_eq!(read[1].reference.as_deref(), Some("VAT-2027-Q1"));
    assert_eq!(read[1].bank_account, "ENBD-1");
    assert!(after(&t, "ENBD-1", date(2027, 2, 28)).await.is_empty());
}

/// A reversed payment is never returned.
#[renox::test]
async fn a_reversed_payment_is_not_returned() {
    let t = boot().await;
    let staff = t.user("Rana", &[perm::VIEW, perm::RECORD, perm::REVERSE]).await;
    let kept = pay(&t, Direction::In, 100_000, date(2027, 2, 1), Some("ENBD-1"), PaymentMethod::Transfer, "K").await;
    let gone = pay(&t, Direction::In, 200_000, date(2027, 2, 2), Some("ENBD-1"), PaymentMethod::Transfer, "G").await;
    t.reverse(&staff, &gone, "bank_return", "returned by the bank")
        .await
        .assert_ok();
    let read = after(&t, "ENBD-1", date(2027, 1, 31)).await;
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].payment.id.0, kept.id);
}

fn credit_note() -> RefundSource {
    RefundSource::CreditNote(Subject::new("invoicing.credit_note", 7))
}

async fn unpaid(t: &Booted) -> Vec<wl_kernel::payments::UnpaidRefund> {
    let entity = t.entity("PGP").await;
    payment_recorder(t.state())
        .refunds_unpaid(t.db(), entity.id)
        .await
        .unwrap()
}

/// A refund waiting for approval is not returned; an approved one is; a paid
/// one is not; one the bank returned (`failed`) is again.
#[renox::test]
async fn the_refunds_approved_or_failed_are_returned() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let requested = t
        .request_for_source(&a, BLUE_DUNES, &credit_note(), 100_000, "transfer", "")
        .await;
    assert!(unpaid(&t).await.is_empty());

    t.refund_action(&b, &requested, "approve", &[]).await.assert_ok();
    let read = unpaid(&t).await;
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].id.0, requested.id);
    assert_eq!(read[0].amount, 100_000);
    assert_eq!(read[0].party, BLUE_DUNES);
    assert!(!read[0].failed);
    assert_eq!(read[0].approved_on, Some(today()));
    assert_eq!(read[0].source, Subject::new("invoicing.credit_note", 7));

    t.refund_action(&a, &requested, "pay", &[("method", "transfer"), ("reference", "BANK-9")])
        .await
        .assert_ok();
    assert_eq!(t.refund(requested.id).await.status, RefundStatus::Paid);
    assert!(unpaid(&t).await.is_empty());

    t.fail_refund(&a, &t.refund(requested.id).await, "bank_return", "account closed")
        .await
        .assert_ok();
    assert_eq!(t.refund(requested.id).await.status, RefundStatus::Failed);
    let read = unpaid(&t).await;
    assert_eq!(read.len(), 1);
    assert!(read[0].failed);
}

async fn counts(t: &Booted) -> (u64, u64, u64, u64) {
    let db = t.db();
    (
        Cheque::query().count(db).await.unwrap(),
        Payment::query().count(db).await.unwrap(),
        Refund::query().count(db).await.unwrap(),
        BankStatementImport::query().count(db).await.unwrap(),
    )
}

/// The reads are side-effect free.
#[renox::test]
async fn the_reads_change_nothing() {
    let t = boot().await;
    cheque_on(&t, "3301", in_days(10), 100_000).await;
    pay(&t, Direction::In, 100_000, date(2027, 2, 1), Some("ENBD-1"), PaymentMethod::Transfer, "X").await;
    import(&t, "ENBD-1", date(2027, 1, 31), Some(1), ImportStatus::Completed, 0).await;
    let before = counts(&t).await;
    open(&t, Direction::In).await;
    balances(&t).await;
    after(&t, "ENBD-1", date(2027, 1, 1)).await;
    unpaid(&t).await;
    assert_eq!(counts(&t).await, before);
}

/// The received cheques that bounced are listed with what they cover, for the
/// forecast's memo line; a cleared one is not.
#[renox::test]
async fn the_bounced_cheques_are_listed_with_their_documents() {
    let t = boot().await;
    let staff = t.cheque_staff("Amal").await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-5", 700_000);
    let bounced = t
        .bounced_cheque(&staff, BLUE_DUNES, "3401", 700_000, Some(rent), "")
        .await;
    let fine = t.received_cheque(BLUE_DUNES, "3402", 100_000, None).await;
    let fine = t.deposit_cheque(&staff, &fine).await;
    t.cheque_action(&staff, &fine, "clear", &[]).await.assert_ok();

    let entity = t.entity("PGP").await;
    let read = cheque_contract(t.state())
        .bounced(t.state(), entity.id)
        .await
        .unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].cheque.id.0, bounced.id);
    assert_eq!(read[0].cheque.status, ChequeStatus::Bounced);
    assert_eq!(read[0].documents.len(), 1);
    assert_eq!(read[0].documents[0].document, rent.subject());
    assert_eq!(read[0].documents[0].amount, 700_000);
}
