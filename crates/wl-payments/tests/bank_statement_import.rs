//! The bank statement import (wl-payments §4 *Bank statement import*, §5.7,
//! issue #118): upload, `parse` (a CSV or MT940 file into lines), `complete`
//! and `revert`, and the closing balance the cash-flow forecast reads from a
//! completed import. Everything goes through the page routes.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::bank_statements::bank_statements;
use wl_kernel::notify::NotifyRecipient;
use wl_payments::bank_statement::{BankStatementImport, ImportStatus};
use wl_payments::bank_statement_line::{BankStatementLine, LineStatus};
use wl_payments::notices::BANK_IMPORT_FAILED;
use wl_payments::permissions as perm;

use support::{Booted, boot};

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

async fn accountant(t: &Booted) -> User {
    t.user("Dina", &[perm::VIEW, perm::BANK_RECONCILE]).await
}

/// `POST /bank-statements` as the acting user; the id of the new import.
async fn upload(
    t: &Booted,
    format: &str,
    start: &str,
    end: &str,
    balance: &str,
    content: &str,
) -> BankStatementImport {
    let entity = t.entity("PGP").await.id.to_string();
    t.app
        .htmx()
        .post(
            "/bank-statements",
            &[
                ("legal_entity_id", entity.as_str()),
                ("bank_account", "ENBD-1"),
                ("file_name", "statement"),
                ("format", format),
                ("period_start", start),
                ("period_end", end),
                ("closing_balance", balance),
                ("content", content),
            ],
        )
        .await
        .assert_redirect("/bank-statements");
    BankStatementImport::query()
        .order_by_desc(BankStatementImport::ID)
        .first(t.db())
        .await
        .unwrap()
        .unwrap()
}

async fn act(
    t: &Booted,
    import: &BankStatementImport,
    action: &str,
) -> renox::testing::TestResponse {
    t.app
        .htmx()
        .post(&format!("/bank-statements/{}/{action}", import.id), &[])
        .await
}

async fn reload(t: &Booted, import: &BankStatementImport) -> BankStatementImport {
    BankStatementImport::find_or_404(t.db(), import.id)
        .await
        .unwrap()
}

async fn balances(t: &Booted) -> Vec<wl_kernel::bank_statements::BankBalance> {
    let entity = t.entity("PGP").await;
    bank_statements(t.state())
        .latest_closing_balances(t.db(), entity.id)
        .await
        .unwrap()
}

const MT940: &str =
    ":20:STMT1\n:25:ENBD-1\n:28C:1/1\n:60F:C270101AED10000,00\n:62F:C270110AED50000,00\n";

/// Given an MT940 file with a closing balance of AED 50,000 for 2027-01-01 to
/// 2027-01-10: nothing is read from the upload or from the import in review;
/// once completed it returns AED 50,000 as of 2027-01-10.
#[renox::test]
async fn an_mt940_file_gives_its_closing_balance_once_completed() {
    let t = boot().await;
    let dina = accountant(&t).await;
    t.app.acting_as(&dina);
    let import = upload(&t, "mt940", "2027-01-01", "2027-01-10", "", MT940).await;
    assert_eq!(import.status, ImportStatus::Uploaded);
    assert_eq!(import.closing_balance, None, "the file is not read yet");
    assert!(balances(&t).await.is_empty());

    act(&t, &import, "parse").await.assert_ok();
    let read = reload(&t, &import).await;
    assert_eq!(read.status, ImportStatus::InReview);
    assert_eq!(read.closing_balance, Some(5_000_000), "from the :62F: tag");
    assert_eq!(read.line_count, 0);
    assert!(balances(&t).await.is_empty(), "not completed yet");

    act(&t, &import, "complete").await.assert_ok();
    let done = reload(&t, &import).await;
    assert_eq!(done.status, ImportStatus::Completed);
    assert!(done.completed_at.is_some());
    let balances = balances(&t).await;
    assert_eq!(balances.len(), 1);
    assert_eq!(balances[0].amount, 5_000_000);
    assert_eq!(balances[0].as_of, date(2027, 1, 10));
    assert_eq!(balances[0].import_id, import.id);
    // A completed import is final.
    act(&t, &import, "revert").await.assert_status(422);
    act(&t, &import, "complete").await.assert_status(422);
}

/// A CSV import with the closing balance entered at upload (AED 48,000) is
/// read, completed and returns that amount.
#[renox::test]
async fn a_csv_import_with_the_balance_entered_at_upload_returns_it() {
    let t = boot().await;
    let dina = accountant(&t).await;
    t.app.acting_as(&dina);
    let import = upload(&t, "csv", "2027-01-01", "2027-01-31", "48,000", "").await;
    assert_eq!(import.closing_balance, Some(4_800_000));
    assert!(balances(&t).await.is_empty());

    act(&t, &import, "parse").await.assert_ok();
    act(&t, &import, "complete").await.assert_ok();

    let balances = balances(&t).await;
    assert_eq!(balances.len(), 1);
    assert_eq!(balances[0].amount, 4_800_000);
    assert_eq!(balances[0].as_of, date(2027, 1, 31));
}

/// A reverted import's balance is not returned, and its lines are removed.
#[renox::test]
async fn a_reverted_import_returns_no_balance_and_loses_its_lines() {
    let t = boot().await;
    let dina = accountant(&t).await;
    t.app.acting_as(&dina);
    let csv = "date,description,reference,amount\n2027-01-05,Bank charges,CHG-1,-25.00\n";
    let import = upload(&t, "csv", "2027-01-01", "2027-01-31", "1,000", csv).await;
    act(&t, &import, "parse").await.assert_ok();
    assert_eq!(
        BankStatementLine::of_import(t.db(), import.id)
            .await
            .unwrap()
            .len(),
        1
    );

    act(&t, &import, "revert").await.assert_ok();
    let reverted = reload(&t, &import).await;
    assert_eq!(reverted.status, ImportStatus::Reverted);
    assert!(balances(&t).await.is_empty());
    assert!(
        BankStatementLine::of_import(t.db(), import.id)
            .await
            .unwrap()
            .is_empty()
    );
    // Terminal.
    act(&t, &import, "complete").await.assert_status(422);
}

/// After the January import (balance as of 2027-01-31), completing a
/// December import uploaded later does not change the returned balance.
#[renox::test]
async fn a_december_import_completed_after_january_does_not_change_the_balance() {
    let t = boot().await;
    let dina = accountant(&t).await;
    t.app.acting_as(&dina);
    let january = upload(&t, "csv", "2027-01-01", "2027-01-31", "9,000", "").await;
    act(&t, &january, "parse").await.assert_ok();
    act(&t, &january, "complete").await.assert_ok();
    let before = balances(&t).await;
    assert_eq!(before[0].amount, 900_000);

    let december = upload(&t, "csv", "2026-12-01", "2026-12-31", "1,000", "").await;
    act(&t, &december, "parse").await.assert_ok();
    act(&t, &december, "complete").await.assert_ok();
    assert_eq!(reload(&t, &december).await.status, ImportStatus::Completed);

    let after = balances(&t).await;
    assert_eq!(after, before);
    assert_eq!(after[0].as_of, date(2027, 1, 31));
    assert_eq!(after[0].import_id, january.id);
}

/// On the same period end the import completed last wins, even when it was
/// uploaded first.
#[renox::test]
async fn the_import_completed_last_wins_whatever_the_upload_order() {
    let t = boot().await;
    let dina = accountant(&t).await;
    t.app.acting_as(&dina);
    let first = upload(&t, "csv", "2027-01-01", "2027-01-31", "400", "").await;
    let second = upload(&t, "csv", "2027-01-01", "2027-01-31", "415", "").await;
    for import in [&first, &second] {
        act(&t, import, "parse").await.assert_ok();
    }
    // The second upload is completed first, the first upload last.
    act(&t, &second, "complete").await.assert_ok();
    act(&t, &first, "complete").await.assert_ok();
    let read = balances(&t).await;
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].import_id, first.id);
    assert_eq!(read[0].amount, 40_000);
}

/// The lines are read, an import cannot complete while one is unmatched, a
/// line is ignored only with a reason, and a line already imported for the
/// account is skipped by a later import.
#[renox::test]
async fn an_import_completes_when_every_line_is_ignored_with_a_reason() {
    let t = boot().await;
    let dina = accountant(&t).await;
    t.app.acting_as(&dina);
    let csv = "date,description,reference,amount\n\
               2027-01-05,Bank charges,CHG-1,-25.00\n\
               2027-01-06,\"Transfer, internal\",INT-1,-500.00\n";
    let import = upload(&t, "csv", "2027-01-01", "2027-01-31", "2,000", csv).await;
    act(&t, &import, "parse").await.assert_ok();
    let read = reload(&t, &import).await;
    assert_eq!((read.status, read.line_count), (ImportStatus::InReview, 2));
    let lines = BankStatementLine::of_import(t.db(), import.id)
        .await
        .unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].amount, -2_500);
    assert_eq!(lines[1].description, "Transfer, internal");
    assert!(lines.iter().all(|l| l.status == LineStatus::Unmatched));

    act(&t, &import, "complete").await.assert_status(422);
    assert_eq!(reload(&t, &import).await.status, ImportStatus::InReview);

    let ignore = |id: i64, reason: &'static str| {
        let t = &t;
        async move {
            t.app
                .htmx()
                .post(
                    &format!("/bank-statement-lines/{id}/ignore"),
                    &[("reason", reason)],
                )
                .await
        }
    };
    ignore(lines[0].id, "  ").await.assert_status(422);
    ignore(lines[0].id, "bank charges").await.assert_ok();
    act(&t, &import, "complete").await.assert_status(422);
    ignore(lines[1].id, "internal transfer").await.assert_ok();
    let ignored = BankStatementLine::find_or_404(t.db(), lines[0].id)
        .await
        .unwrap();
    assert_eq!(ignored.status, LineStatus::Ignored);
    assert_eq!(ignored.ignore_reason.as_deref(), Some("bank charges"));

    act(&t, &import, "complete").await.assert_ok();
    assert_eq!(balances(&t).await[0].amount, 200_000);

    // The same file again for the same account: its lines are skipped.
    let again = upload(&t, "csv", "2027-01-01", "2027-01-31", "2,000", csv).await;
    act(&t, &again, "parse").await.assert_ok();
    let read = reload(&t, &again).await;
    assert_eq!((read.status, read.line_count), (ImportStatus::InReview, 0));
}

/// A file that is not recognised fails the import and tells the uploader the
/// reason: Excel, a file of another account, a transaction outside the
/// period, a closing balance not dated the last day of the period.
#[renox::test]
async fn a_file_that_is_not_recognised_fails_the_import_and_tells_the_uploader() {
    let t = boot().await;
    let dina = accountant(&t).await;
    t.app.acting_as(&dina);
    let other_account = ":25:MASHREQ-9\n:62F:C270110AED50000,00\n";
    let late_line = ":25:ENBD-1\n:61:2701200120D10,00NMSCX\n:62F:C270110AED1,00\n";
    let wrong_day = ":25:ENBD-1\n:62F:C270109AED1,00\n";
    let cases = [
        ("excel", "anything", "cannot be read yet"),
        ("mt940", other_account, "MASHREQ-9"),
        ("mt940", late_line, "outside the period"),
        ("mt940", wrong_day, "not the last day"),
        ("csv", "date,amount\n2027-01-02,5", "needs the columns"),
        ("mt940", "", "missing"),
    ];
    for (n, (format, content, reason)) in cases.into_iter().enumerate() {
        let import = upload(&t, format, "2027-01-01", "2027-01-10", "", content).await;
        act(&t, &import, "parse").await.assert_ok();
        let failed = reload(&t, &import).await;
        assert_eq!(failed.status, ImportStatus::Failed, "case {n}");
        let told = t.notices.of_type(BANK_IMPORT_FAILED);
        assert_eq!(told.len(), n + 1);
        assert!(
            told[n].data.to_string().contains(reason),
            "{}: {}",
            reason,
            told[n].data
        );
        assert!(matches!(
            told[n].recipients[0],
            NotifyRecipient::User { id } if id == dina.id
        ));
    }
    assert!(balances(&t).await.is_empty());
}

/// The statement page lists the lines with the actions, for `bank.reconcile`.
#[renox::test]
async fn the_statement_page_shows_the_lines_and_needs_the_permission() {
    let t = boot().await;
    let dina = accountant(&t).await;
    let clerk = t.user("Omar", &[perm::VIEW]).await;
    t.app.acting_as(&dina);
    let csv = "date,description,reference,amount\n2027-01-05,ATM Marina,ATM-1,-1200.00\n";
    let import = upload(&t, "csv", "2027-01-01", "2027-01-31", "", csv).await;
    t.app
        .get(&format!("/bank-statements/{}", import.id))
        .await
        .assert_ok()
        .assert_see(&format!("parse-import-{}", import.id));
    act(&t, &import, "parse").await.assert_ok();
    t.app
        .get(&format!("/bank-statements/{}", import.id))
        .await
        .assert_ok()
        .assert_see("ATM Marina")
        .assert_see("1,200.00")
        .assert_see(&format!("complete-import-{}", import.id));
    t.app
        .get("/bank-statements")
        .await
        .assert_ok()
        .assert_see("statement");

    t.app.acting_as(&clerk);
    t.app
        .get(&format!("/bank-statements/{}", import.id))
        .await
        .assert_forbidden();
    act(&t, &import, "complete").await.assert_forbidden();
}
