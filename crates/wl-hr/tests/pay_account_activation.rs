//! BL-HRM-20 (#210), `HRM.ACL.05`: when the cooling ends the new pay account
//! becomes `active` and the previous one `retired`; a payroll run uses the
//! account that is active when it is submitted, never a cooling one; a line
//! whose account is closed while the new one cools is held and paid off-cycle.

mod support;

use renox::prelude::*;
use wl_hr::pay_account;
use wl_hr::{
    Employee, EmployeeStatus, LineStatus, PayAccount, PayAccountStatus, PayrollLine, PayrollRun,
    SifFile, SifLine,
};

use support::{Booted, nov};

const NEW_IBAN: &str = "AE550331234567890121234";
const OLD_IBAN: &str = "AE100261234567890124321";
const NET: i64 = 300_000;

/// HR user A and approver B ask for and approve the new account; P prepares
/// payroll. E, Ravi Kumar, is paid into the old account, ending 4321.
struct World {
    t: Booted,
    a: User,
    b: User,
    prep: User,
    e: Employee,
    old: PayAccount,
}

async fn world() -> World {
    let t = support::boot().await;
    let a = t.user_with("Hana", "hr_pay").await;
    let b = t.user_with("Basel", "pay_approver").await;
    let prep = t.user_with("Pia", "payroll_prep").await;
    let e = t.employee_in(61, EmployeeStatus::Active).await;
    t.parties.name(61, "Ravi Kumar");
    t.parties.mobile(61, "+971500000061");
    let old = t
        .pay_account_in(&e, PayAccountStatus::Active, OLD_IBAN)
        .await;
    World {
        t,
        a,
        b,
        prep,
        e,
        old,
    }
}

/// A asks for `NEW_IBAN` and B approves it on 2026-11-02: it cools until the
/// 7th.
async fn cooling(w: &World) -> PayAccount {
    let form = [
        ("channel", "bank_account"),
        ("account_number", NEW_IBAN),
        ("routing_code", "123456789"),
        ("holder_name", "Ravi Kumar"),
    ];
    w.t.request_account(&w.a, &w.e, &form)
        .await
        .assert_status(303);
    let account = w.t.accounts_of(&w.e).await.pop().unwrap();
    w.t.on(nov(2), w.t.pay_transition(&w.b, &account, "approve", ""))
        .await
        .assert_status(303);
    w.t.account(&account).await
}

/// The clock to `day`, then the daily job at 00:00 Asia/Dubai.
async fn job_on(w: &World, day: renox::chrono::NaiveDate) {
    w.t.on(day, async {}).await;
    w.t.app.run_scheduled(pay_account::TASK).await.unwrap();
}

/// The line of the file of `run` that carries `line`.
async fn sent_line(t: &Booted, line: &PayrollLine) -> SifLine {
    SifLine::query()
        .where_(SifLine::LINE_ID.eq(line.id))
        .order_by_desc(SifLine::ID)
        .first(t.db())
        .await
        .unwrap()
        .expect("the line is in a file")
}

/// AC1: cooling ends 2026-11-07: that day the new account is `active`, the old
/// one `retired` and `PayAccountChanged` is emitted, once however often the
/// job runs; the day before nothing happens.
#[renox::test]
async fn hrm20_ac1_the_account_activates_on_its_end_date_and_the_old_one_retires() {
    let w = world().await;
    let new = cooling(&w).await;

    job_on(&w, nov(6)).await;
    assert_eq!(w.t.account(&new).await.status, PayAccountStatus::Cooling);
    assert!(w.t.pay_changed.all().is_empty());

    job_on(&w, nov(7)).await;
    job_on(&w, nov(7)).await;

    assert_eq!(w.t.account(&new).await.status, PayAccountStatus::Active);
    assert_eq!(w.t.account(&w.old).await.status, PayAccountStatus::Retired);
    let events = w.t.pay_changed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].employee.0, w.e.id);
    assert_eq!(events[0].last4, "1234");
    assert_eq!(events[0].approver, Some(w.b.id));
}

/// AC2: a run submitted on 2026-11-06 uses the old account; one that was
/// approved before the account activated, and submitted on 2026-11-07, the new
/// one. The file keeps the account it was sent to.
#[renox::test]
async fn hrm20_ac2_a_run_uses_the_account_active_when_it_is_submitted() {
    let w = world().await;
    let new = cooling(&w).await;
    let later =
        w.t.payroll_run("2026-11", wl_hr::PayrollRunStatus::Approved)
            .await;
    let later_line = w.t.payroll_line(&later, &w.e, NET, NET).await;

    let earlier =
        w.t.payroll_run("2026-10", wl_hr::PayrollRunStatus::Approved)
            .await;
    let lines = [w.t.payroll_line(&earlier, &w.e, NET, NET).await];

    // Day 6: the job finds nothing to activate and the run is submitted.
    job_on(&w, nov(6)).await;
    w.t.on(nov(6), wl_hr::payroll::submit(w.t.state(), earlier.id))
        .await
        .unwrap();
    assert_eq!(w.t.account(&new).await.status, PayAccountStatus::Cooling);
    let early = sent_line(&w.t, &lines[0]).await;
    assert_eq!(early.pay_account_id, Some(w.old.id));
    assert_eq!(early.account_last4.as_deref(), Some("4321"));

    // Day 7: the job activates the account, then the other run is submitted.
    job_on(&w, nov(7)).await;
    w.t.on(nov(7), wl_hr::payroll::submit(w.t.state(), later.id))
        .await
        .unwrap();

    let used = sent_line(&w.t, &later_line).await;
    assert_eq!(used.pay_account_id, Some(new.id));
    assert_eq!(used.account_last4.as_deref(), Some("1234"));
    // The first file still names the old account.
    let early = sent_line(&w.t, &lines[0]).await;
    assert_eq!(early.pay_account_id, Some(w.old.id));
}

/// AC3: the old account was reported closed and the new one still cools: the
/// next run holds the line with the reason "account closed" and makes no file;
/// the line the bank failed is fixed when the new account activates
/// ([`wl_hr::release::line_fixed`]) and the held one is released.
#[renox::test]
async fn hrm20_ac3_a_closed_account_holds_the_line_until_the_new_one_is_active() {
    let w = world().await;
    let (first, lines) = w.t.submitted_run("2026-10", &[&w.e], NET).await;
    w.t.partial(
        &w.prep,
        &first,
        "RESULT-1",
        &[(&lines[0], "account closed")],
    )
    .await
    .assert_status(303);
    assert_eq!(w.t.line(&lines[0]).await.status, LineStatus::Failed);
    assert!(w.t.account(&w.old).await.closed_reported_on.is_some());
    let new = cooling(&w).await;

    let next: PayrollRun =
        w.t.payroll_run("2026-11", wl_hr::PayrollRunStatus::Approved)
            .await;
    let held = w.t.payroll_line(&next, &w.e, NET, NET).await;
    wl_hr::payroll::submit(w.t.state(), next.id).await.unwrap();

    let held_line = w.t.line(&held).await;
    assert_eq!(held_line.status, LineStatus::Held);
    assert_eq!(held_line.held_reason.as_deref(), Some("account closed"));
    assert!(SifFile::of_run(w.t.db(), next.id).await.unwrap().is_empty());

    job_on(&w, nov(7)).await;

    assert_eq!(w.t.account(&new).await.status, PayAccountStatus::Active);
    assert_eq!(w.t.line(&lines[0]).await.status, LineStatus::Included);
    assert!(w.t.line(&held).await.hold_cleared_on.is_some());
}

/// AC4: with *notice undelivered* set on the account, on the end date it stays
/// `cooling` and no event is emitted.
#[renox::test]
async fn hrm20_ac4_an_undelivered_notice_keeps_the_account_cooling_on_its_end_date() {
    let w = world().await;
    let new = cooling(&w).await;
    PayAccount::query()
        .where_(PayAccount::ID.eq(new.id))
        .update(
            w.t.db(),
            &[("notice_undelivered_at", &Some(renox::db::now()))],
        )
        .await
        .unwrap();

    job_on(&w, nov(7)).await;
    job_on(&w, nov(9)).await;

    assert_eq!(w.t.account(&new).await.status, PayAccountStatus::Cooling);
    assert_eq!(w.t.account(&w.old).await.status, PayAccountStatus::Active);
    assert!(w.t.pay_changed.all().is_empty());
}

/// A cooling account is never used, and the first account of an employee
/// activates without a previous one to retire.
#[renox::test]
async fn hrm20_the_first_account_of_a_new_employee_is_used_once_active() {
    let w = world().await;
    let f = w.t.employee_in(62, EmployeeStatus::Active).await;
    w.t.parties.name(62, "Mona Said");
    w.t.parties.mobile(62, "+971500000062");
    let form = [
        ("channel", "bank_account"),
        ("account_number", NEW_IBAN),
        ("routing_code", "123456789"),
        ("holder_name", "Mona Said"),
    ];
    w.t.request_account(&w.a, &f, &form)
        .await
        .assert_status(303);
    let first = w.t.accounts_of(&f).await.pop().unwrap();
    w.t.on(nov(2), w.t.pay_transition(&w.b, &first, "approve", ""))
        .await
        .assert_status(303);

    let (_, early) = w.t.submitted_run("2026-10", &[&f], NET).await;
    assert_eq!(sent_line(&w.t, &early[0]).await.pay_account_id, None);

    job_on(&w, nov(7)).await;
    assert_eq!(w.t.account(&first).await.status, PayAccountStatus::Active);
    let (_, later) = w.t.submitted_run("2026-11", &[&f], NET).await;
    assert_eq!(
        sent_line(&w.t, &later[0]).await.pay_account_id,
        Some(first.id)
    );
}
