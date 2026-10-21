//! BL-HRM-13 (#203): the bank pays some lines of the file and fails others
//! (`record_partial`): the paid lines are confirmed and their payslips
//! published, each failed line gets a task; each failed line is then fixed,
//! held or paid another approved way, `regenerate` makes a SIF with only the
//! fixed lines, and `confirm_paid` ends the run when no line is left `failed`.

mod support;

use renox::prelude::*;
use wl_hr::payroll::{self, FailedLine};
use wl_hr::{
    Employee, EmployeeStatus, LineStatus, PayAccountStatus, PayrollLine, PayrollRun,
    PayrollRunStatus, SifFile, pay_account,
};
use wl_kernel::payments::{Direction, PaymentMethod, PaymentStatus};
use wl_kernel::tickets::Assignee;
use wl_payments::payment::Payment;

use support::{Booted, nov, users};

const NET: i64 = 300_000;

/// A prepares, B approves; `count` employees are paid.
struct Desk {
    t: Booted,
    a: User,
    b: User,
    run: PayrollRun,
    lines: Vec<PayrollLine>,
}

async fn desk(count: usize) -> Desk {
    desk_on(support::boot().await, count).await
}

async fn desk_on(t: Booted, count: usize) -> Desk {
    let a = t.user_with("Pia", "payroll_prep").await;
    let b = t.user_with("Ali", "payroll_appr").await;
    let mut employees: Vec<Employee> = Vec::new();
    for n in 0..count {
        let party = 100 + i64::try_from(n).unwrap();
        employees.push(t.employee_in(party, EmployeeStatus::Active).await);
    }
    let (run, lines) = t.sent_run(&a, &b, "2026-11", &employees, NET).await;
    assert_eq!(run.status, PayrollRunStatus::Submitted);
    assert_eq!(lines.len(), count);
    Desk {
        t,
        a,
        b,
        run,
        lines,
    }
}

async fn files(d: &Desk) -> Vec<SifFile> {
    SifFile::of_run(d.t.db(), d.run.id).await.unwrap()
}

async fn line_ids_of(file: &SifFile, d: &Desk) -> Vec<i64> {
    let mut tx = d.t.db().begin().await.unwrap();
    let mut ids: Vec<i64> = file
        .lines_in(&mut tx)
        .await
        .unwrap()
        .iter()
        .map(|l| l.line_id)
        .collect();
    ids.sort_unstable();
    ids
}

async fn published(d: &Desk) -> usize {
    let mut n = 0;
    for line in &d.lines {
        if d.t.line(line).await.payslip_published_at.is_some() {
            n += 1;
        }
    }
    n
}

/// AC1: 50 lines, the result file shows 47 paid and 3 failed (invalid IBAN):
/// the run is `partially_rejected`, 47 payslips are published, 3 lines are
/// `failed` and there are 3 tasks for the preparer; the approver is told and
/// `PayrollLinesFailed` carries the 3 lines.
#[renox::test]
async fn hrm13_ac1_a_result_file_with_failed_lines_confirms_the_paid_ones_and_opens_tasks() {
    let d = desk(50).await;
    let failed: Vec<(&PayrollLine, &str)> =
        d.lines[..3].iter().map(|l| (l, "invalid IBAN")).collect();

    d.t.partial(&d.a, &d.run, "RESULT-1", &failed)
        .await
        .assert_status(303);

    assert_eq!(
        d.t.run(&d.run).await.status,
        PayrollRunStatus::PartiallyRejected
    );
    let mut paid = 0;
    let mut failed_lines = 0;
    for line in &d.lines {
        match d.t.line(line).await.status {
            LineStatus::Paid => paid += 1,
            LineStatus::Failed => failed_lines += 1,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!((paid, failed_lines), (47, 3));
    assert_eq!(published(&d).await, 47, "47 payslips are published");
    for line in &d.lines[..3] {
        let line = d.t.line(line).await;
        assert_eq!(line.failure_reason.as_deref(), Some("invalid IBAN"));
        assert_eq!(line.payslip_published_at, None);
    }
    // The result file confirms version 1.
    assert_eq!(files(&d).await[0].confirmation.as_deref(), Some("RESULT-1"));

    // One task per failed line, for the preparer.
    let tasks = d.t.tasks.with_key("hr.payroll_line_failed:");
    assert_eq!(tasks.len(), 3);
    assert!(
        tasks
            .iter()
            .all(|t| t.assignee == Some(Assignee::user(d.a.id)))
    );
    // The preparer and the approver are told; the event lists the 3 lines.
    let notices = d.t.notices.of_type("hr.payroll_failed");
    assert_eq!(notices.len(), 1);
    let mut told = vec![d.a.id, d.b.id];
    told.sort_unstable();
    assert_eq!(users(&notices[0]), told);
    let events = d.t.failed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].lines.len(), 3);
    assert!(events[0].lines.iter().all(|l| l.reason == "invalid IBAN"));
}

/// AC2: after 2 lines are fixed (new approved accounts) and 1 is held,
/// `regenerate` builds SIF v2 with the 2 lines only; v1 is kept and the paid
/// lines are not in v2.
#[renox::test]
async fn hrm13_ac2_regenerate_sends_only_the_fixed_lines() {
    let d = desk(50).await;
    let failed: Vec<(&PayrollLine, &str)> =
        d.lines[..3].iter().map(|l| (l, "invalid IBAN")).collect();
    d.t.partial(&d.a, &d.run, "RESULT-1", &failed)
        .await
        .assert_status(303);

    // A failed line is still there: regenerate is refused.
    d.t.transition(&d.a, &d.run, "regenerate", "")
        .await
        .assert_status(422);
    assert_eq!(files(&d).await.len(), 1);

    payroll::fix_line(d.t.db(), d.lines[0].id).await.unwrap();
    payroll::fix_line(d.t.db(), d.lines[1].id).await.unwrap();
    d.t.line_action(&d.a, &d.lines[2], "hold", &[("reason", "account closed")])
        .await
        .assert_status(303);
    let held = d.t.line(&d.lines[2]).await;
    assert_eq!(held.status, LineStatus::Held);
    assert_eq!(held.held_reason.as_deref(), Some("account closed"));

    d.t.transition(&d.a, &d.run, "regenerate", "")
        .await
        .assert_status(303);

    let run = d.t.run(&d.run).await;
    assert_eq!(run.status, PayrollRunStatus::Submitted);
    assert_eq!(run.sif_version, 2);
    let all = files(&d).await;
    assert_eq!(all.len(), 2);
    assert_eq!(all[1].line_count, 2);
    let mut expected = vec![d.lines[0].id, d.lines[1].id];
    expected.sort_unstable();
    assert_eq!(line_ids_of(&all[1], &d).await, expected);
    assert_eq!(line_ids_of(&all[0], &d).await.len(), 50, "v1 is kept");
    assert_eq!(d.t.line(&d.lines[2]).await.sif_version, Some(1));
}

/// AC2, through the real flow: two of three failed lines get a new pay account
/// requested by HR and approved by a second user; the cooling ends and the
/// daily job `hr:pay-accounts` activates both accounts, which fixes the two
/// lines; the third line is held; `regenerate` then builds SIF v2 with exactly
/// those two lines.
#[renox::test]
async fn hrm13_ac2_regenerate_after_two_pay_accounts_were_activated() {
    let d = desk(5).await;
    let hr = d.t.user_with("Hana", "hr_pay").await;
    let approver = d.t.user_with("Basel", "pay_approver").await;
    let failed: Vec<(&PayrollLine, &str)> =
        d.lines[..3].iter().map(|l| (l, "invalid IBAN")).collect();
    d.t.partial(&d.a, &d.run, "RESULT-1", &failed)
        .await
        .assert_status(303);

    let ibans = ["AE550331234567890121234", "AE560331234567890125678"];
    let mut accounts = Vec::new();
    for (n, iban) in ibans.iter().enumerate() {
        let employee = Employee::find_or_404(d.t.db(), d.lines[n].employee_id)
            .await
            .unwrap();
        let name = format!("Worker {n}");
        d.t.parties.name(employee.party_id, &name);
        d.t.parties
            .mobile(employee.party_id, &format!("+97150000010{n}"));
        let form = [
            ("channel", "bank_account"),
            ("account_number", *iban),
            ("routing_code", "123456789"),
            ("holder_name", name.as_str()),
        ];
        d.t.request_account(&hr, &employee, &form)
            .await
            .assert_status(303);
        let account = d.t.accounts_of(&employee).await.pop().unwrap();
        d.t.on(
            nov(2),
            d.t.pay_transition(&approver, &account, "approve", ""),
        )
        .await
        .assert_status(303);
        accounts.push(account);
    }
    d.t.line_action(&d.a, &d.lines[2], "hold", &[("reason", "account closed")])
        .await
        .assert_status(303);

    // Cooling: nothing is fixed yet, regenerate is refused.
    d.t.on(nov(6), async {}).await;
    d.t.app.run_scheduled(pay_account::TASK).await.unwrap();
    assert_eq!(d.t.line(&d.lines[0]).await.status, LineStatus::Failed);
    d.t.transition(&d.a, &d.run, "regenerate", "")
        .await
        .assert_status(422);

    d.t.on(nov(7), async {}).await;
    d.t.app.run_scheduled(pay_account::TASK).await.unwrap();
    for account in &accounts {
        assert_eq!(d.t.account(account).await.status, PayAccountStatus::Active);
    }
    assert_eq!(d.t.line(&d.lines[0]).await.status, LineStatus::Included);
    assert_eq!(d.t.line(&d.lines[1]).await.status, LineStatus::Included);
    assert_eq!(d.t.line(&d.lines[2]).await.status, LineStatus::Held);

    d.t.transition(&d.a, &d.run, "regenerate", "")
        .await
        .assert_status(303);

    let run = d.t.run(&d.run).await;
    assert_eq!(run.status, PayrollRunStatus::Submitted);
    assert_eq!(run.sif_version, 2);
    let all = files(&d).await;
    assert_eq!(all.len(), 2);
    assert_eq!(all[1].line_count, 2);
    let mut expected = vec![d.lines[0].id, d.lines[1].id];
    expected.sort_unstable();
    assert_eq!(line_ids_of(&all[1], &d).await, expected);
    assert_eq!(d.t.line(&d.lines[2]).await.sif_version, Some(1));
}

/// AC3: `confirm_paid` with the confirmation for the 2 lines of v2 makes the
/// run `paid`; the held line waits; the payslips of the paid lines were
/// published once (not again), the 2 new ones now.
#[renox::test]
async fn hrm13_ac3_confirm_paid_ends_the_run_and_the_held_line_waits() {
    let d = desk(50).await;
    let failed: Vec<(&PayrollLine, &str)> =
        d.lines[..3].iter().map(|l| (l, "invalid IBAN")).collect();
    d.t.partial(&d.a, &d.run, "RESULT-1", &failed)
        .await
        .assert_status(303);
    payroll::fix_line(d.t.db(), d.lines[0].id).await.unwrap();
    payroll::fix_line(d.t.db(), d.lines[1].id).await.unwrap();
    d.t.line_action(&d.a, &d.lines[2], "hold", &[("reason", "account closed")])
        .await
        .assert_status(303);
    d.t.transition(&d.a, &d.run, "regenerate", "")
        .await
        .assert_status(303);
    let first_paid = d.t.line(&d.lines[10]).await.payslip_published_at;
    assert!(first_paid.is_some());

    // Without the bank's confirmation of v2 it is refused.
    d.t.confirm(&d.b, &d.run, "").await.assert_status(422);
    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Submitted);

    d.t.confirm(&d.b, &d.run, "BANK-OK-2")
        .await
        .assert_status(303);

    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Paid);
    assert_eq!(
        files(&d).await[1].confirmation.as_deref(),
        Some("BANK-OK-2")
    );
    assert_eq!(d.t.line(&d.lines[0]).await.status, LineStatus::Paid);
    assert_eq!(d.t.line(&d.lines[1]).await.status, LineStatus::Paid);
    assert_eq!(d.t.line(&d.lines[2]).await.status, LineStatus::Held);
    assert_eq!(d.t.line(&d.lines[2]).await.payslip_published_at, None);
    assert_eq!(published(&d).await, 49);
    assert_eq!(
        d.t.line(&d.lines[10]).await.payslip_published_at,
        first_paid,
        "a payslip is published once"
    );
    let events = d.t.paid.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].employees.len(), 49);
    assert_eq!(events[0].total, 49 * NET);
}

/// AC4: `confirm_paid` with a `failed` line left is refused, from either
/// state.
#[renox::test]
async fn hrm13_ac4_confirm_paid_is_refused_while_a_line_is_failed() {
    let d = desk(5).await;
    d.t.partial(&d.a, &d.run, "RESULT-1", &[(&d.lines[0], "invalid IBAN")])
        .await
        .assert_status(303);

    d.t.confirm(&d.b, &d.run, "BANK-OK")
        .await
        .assert_status(422);

    assert_eq!(
        d.t.run(&d.run).await.status,
        PayrollRunStatus::PartiallyRejected
    );
    assert_eq!(d.t.line(&d.lines[0]).await.status, LineStatus::Failed);
    assert!(d.t.paid.all().is_empty());
}

/// AC5: a failed line instead marked paid by an approved transfer "TR-88":
/// `confirm_paid` does not ask for a bank confirmation for it, and the transfer
/// is recorded as an outgoing payment (a voucher) of the line's net to the
/// employee through the kernel `PaymentRecorder` of the real wl-payments
/// (`FIN.PAY.01`). With every failed line moved out `regenerate` would send
/// an empty file and is refused, so the run goes straight to `confirm_paid`
/// from `partially_rejected`.
#[renox::test]
async fn hrm13_ac5_a_line_paid_by_transfer_needs_no_bank_confirmation() {
    let t = support::boot_with_payments().await;
    for n in 0..5 {
        t.parties.name(100 + n, &format!("Worker {n}"));
    }
    let d = desk_on(t, 5).await;
    d.t.partial(&d.a, &d.run, "RESULT-1", &[(&d.lines[0], "invalid IBAN")])
        .await
        .assert_status(303);

    d.t.line_action(
        &d.a,
        &d.lines[0],
        "pay-other",
        &[("method", "transfer"), ("reference", "TR-88")],
    )
    .await
    .assert_status(303);
    let moved = d.t.line(&d.lines[0]).await;
    assert_eq!(moved.status, LineStatus::Paid);
    assert_eq!(moved.paid_method.as_deref(), Some("transfer"));
    assert_eq!(moved.paid_reference.as_deref(), Some("TR-88"));

    let vouchers = Payment::query().get(d.t.db()).await.unwrap();
    assert_eq!(vouchers.len(), 1, "one payment, for the one line moved out");
    let voucher = &vouchers[0];
    assert_eq!(voucher.direction, Direction::Out);
    assert_eq!(voucher.method, PaymentMethod::Transfer);
    assert_eq!(voucher.status, PaymentStatus::Received);
    assert_eq!(voucher.amount, NET);
    assert_eq!(voucher.currency, "AED");
    assert_eq!(voucher.reference.as_deref(), Some("TR-88"));
    let paid_employee = Employee::find_or_404(d.t.db(), moved.employee_id)
        .await
        .unwrap();
    assert_eq!(voucher.party_id, paid_employee.party_id);
    assert_eq!(voucher.recorded_by, Some(d.a.id));
    assert!(voucher.number.is_some(), "a voucher number was allocated");

    // The line is no longer failed: a second call records nothing more.
    d.t.line_action(
        &d.a,
        &d.lines[0],
        "pay-other",
        &[("method", "transfer"), ("reference", "TR-88")],
    )
    .await
    .assert_status(422);
    assert_eq!(Payment::query().get(d.t.db()).await.unwrap().len(), 1);

    d.t.transition(&d.a, &d.run, "regenerate", "")
        .await
        .assert_status(422);
    assert_eq!(files(&d).await.len(), 1);

    d.t.confirm(&d.b, &d.run, "").await.assert_status(303);

    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Paid);
    assert_eq!(
        d.t.line(&d.lines[0]).await.status,
        LineStatus::Paid,
        "the transfer is kept"
    );
    assert!(d.t.line(&d.lines[0]).await.payslip_published_at.is_some());
    assert_eq!(published(&d).await, 5);
}

/// A second partial failure after `regenerate`: `record_partial` again from
/// `submitted`; the lines already `paid` stay paid and the line that fails
/// again gets a new task.
#[renox::test]
async fn hrm13_a_second_partial_failure_leaves_paid_lines_paid_and_opens_a_new_task() {
    let d = desk(5).await;
    d.t.partial(
        &d.a,
        &d.run,
        "RESULT-1",
        &[(&d.lines[0], "invalid IBAN"), (&d.lines[1], "routing code")],
    )
    .await
    .assert_status(303);
    assert_eq!(d.t.tasks.with_key("hr.payroll_line_failed:").len(), 2);
    payroll::fix_line(d.t.db(), d.lines[0].id).await.unwrap();
    payroll::fix_line(d.t.db(), d.lines[1].id).await.unwrap();
    d.t.transition(&d.a, &d.run, "regenerate", "")
        .await
        .assert_status(303);

    d.t.partial(&d.a, &d.run, "RESULT-2", &[(&d.lines[1], "account closed")])
        .await
        .assert_status(303);

    assert_eq!(
        d.t.run(&d.run).await.status,
        PayrollRunStatus::PartiallyRejected
    );
    assert_eq!(d.t.line(&d.lines[0]).await.status, LineStatus::Paid);
    assert_eq!(d.t.line(&d.lines[1]).await.status, LineStatus::Failed);
    for line in &d.lines[2..] {
        assert_eq!(d.t.line(line).await.status, LineStatus::Paid);
    }
    assert_eq!(
        d.t.tasks.with_key("hr.payroll_line_failed:").len(),
        3,
        "one new task for the line that failed again"
    );
    assert_eq!(files(&d).await[1].confirmation.as_deref(), Some("RESULT-2"));
}

/// The result needs the bank's file, a reason for each failed line, at least
/// one failed line and only lines of the file.
#[renox::test]
async fn hrm13_a_result_needs_the_file_the_reasons_and_lines_of_the_file() {
    let d = desk(3).await;
    let other = d.t.employee_in(900, EmployeeStatus::Active).await;
    let (other_run, other_lines) = d.t.sent_run(&d.a, &d.b, "2026-12", &[other], NET).await;

    d.t.partial(&d.a, &d.run, " ", &[(&d.lines[0], "invalid IBAN")])
        .await
        .assert_status(422);
    d.t.partial(&d.a, &d.run, "RESULT-1", &[(&d.lines[0], " ")])
        .await
        .assert_status(422);
    d.t.partial(&d.a, &d.run, "RESULT-1", &[])
        .await
        .assert_status(422);
    d.t.partial(
        &d.a,
        &d.run,
        "RESULT-1",
        &[(&other_lines[0], "invalid IBAN")],
    )
    .await
    .assert_status(422);
    d.t.partial(&d.b, &d.run, "RESULT-1", &[(&d.lines[0], "invalid IBAN")])
        .await
        .assert_status(403);

    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Submitted);
    assert_eq!(
        d.t.run(&other_run).await.status,
        PayrollRunStatus::Submitted
    );
    assert_eq!(published(&d).await, 0);
    assert!(d.t.tasks.opened().is_empty());
    // The system twin takes the same result without a file (an import).
    let twin = payroll::record_partial(
        d.t.state(),
        d.run.id,
        vec![FailedLine {
            line_id: d.lines[0].id,
            reason: "invalid IBAN".into(),
        }],
    )
    .await
    .unwrap();
    assert_eq!(twin.status, PayrollRunStatus::PartiallyRejected);
}

/// A submitted run with no failed line is confirmed with the bank's
/// confirmation of its one file; without it the run is not paid.
#[renox::test]
async fn hrm13_a_run_without_failures_is_confirmed_with_the_confirmation_of_its_file() {
    let d = desk(3).await;

    d.t.confirm(&d.b, &d.run, "").await.assert_status(422);
    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Submitted);

    d.t.confirm(&d.b, &d.run, "BANK-OK-1")
        .await
        .assert_status(303);
    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Paid);
    assert_eq!(published(&d).await, 3);
    // The preparer does not confirm payment.
    let again = d.t.confirm(&d.a, &d.run, "BANK-OK-1").await;
    assert!(again.status.as_u16() == 403 || again.status.as_u16() == 422);
}
