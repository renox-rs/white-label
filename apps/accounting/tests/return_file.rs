//! #343 (BL-ACC-19, `ACC.ACL.15`) with the real wl-ticketing: a return is
//! filed only by a staff member whose FTA tax agent registration is valid on
//! the filing date (the date on the acknowledgement) and linked to the firm's
//! tax agency, or recorded as filed by the client through its own EmaraTax
//! user. An expired client KYC document flags the return for the MLRO but
//! does not stop it. For a blocked client the filing waits for the MLRO's
//! approval. Filing completes the linked task and sends the payment
//! reminder.
//!
//! wl-hr has no Qualification records yet (BL-HRM-01): the registrations are
//! the tests' fake `Qualifications` (Class B).

mod support;

use accounting::permissions as perm;
use accounting::return_effects::PAYMENT_REMINDER_NOTICE;
use accounting::return_machine::FILE;
use accounting::{Engagement, Return, ReturnStatus};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::TestResponse;
use wl_kernel::approvals::{self, ApprovalStatus};
use wl_kernel::deadlines::{DeadlineTaskId, DeadlineTaskStatus};

use support::{Booted, Client, Crew, boot_real, d};

fn day(text: &str) -> NaiveDate {
    text.parse().unwrap()
}

/// What a filing test starts from: a client with an `active` engagement, its
/// VAT task due 2026-11-28 (overdue on 2026-12-05) and the `approved` return
/// that tracks it, and the clock on 2026-12-05.
struct Setup {
    client: Client,
    engagement: Engagement,
    crew: Crew,
    task: DeadlineTaskId,
    ret: Return,
}

async fn setup(t: &Booted) -> Setup {
    t.set_clock_on(d(11, 5), 10, 0);
    let client = t.client().await;
    let engagement = t.active_engagement(&client).await;
    let crew = t.crew(&client).await;
    let task = t.vat_task(&engagement, d(11, 28)).await;
    let ret = t.open_return(&engagement, d(11, 28)).await;
    assert_eq!(ret.task_id, Some(task.0));
    let ret = t.return_at(&ret, &crew, ReturnStatus::Approved).await;
    t.set_clock_on(d(12, 5), 10, 0);
    Setup {
        client,
        engagement,
        crew,
        task,
        ret,
    }
}

/// Tariq, who files returns (`returns.file`).
async fn filer(t: &Booted) -> User {
    t.user_with_grants("Tariq", &[perm::RETURNS_VIEW, perm::RETURNS_FILE])
        .await
}

/// `user` files `ret` with the acknowledgement dated `date`.
async fn file_on(t: &Booted, user: &User, ret: &Return, date: &str) -> TestResponse {
    t.ret_act(
        user,
        ret,
        "file",
        &[("acknowledgement", "FTA-ACK-1188"), ("filed_on", date)],
    )
    .await
}

/// Tax specialist T with a registration expiring 2026-11-30: an
/// acknowledgement dated 2026-11-28 files the return; one dated 2026-12-01 is
/// refused naming the registration, and so is a registration of another
/// firm's agency, no registration, and a filer without an employee record.
#[renox::test]
async fn the_registration_must_be_valid_on_the_date_of_the_acknowledgement() {
    let t = boot_real().await;
    let s = setup(&t).await;
    let tariq = filer(&t).await;
    t.qualifications.register(tariq.id, "TA-1001", d(11, 30));

    file_on(&t, &tariq, &s.ret, "2026-12-01")
        .await
        .assert_status(422)
        .assert_see("TA-1001")
        .assert_see("2026-11-30");
    assert_eq!(t.return_status(&s.ret).await, ReturnStatus::Approved);

    file_on(&t, &tariq, &s.ret, "2026-11-28").await.assert_ok();
    let filed = t.return_now(&s.ret).await;
    assert_eq!(filed.status, ReturnStatus::Filed);
    assert_eq!(filed.filed_on, Some(day("2026-11-28")));
    assert_eq!(filed.filed_by_user_id, Some(tariq.id));
    assert_eq!(filed.tax_agent_registration.as_deref(), Some("TA-1001"));
    assert!(!filed.filed_by_client);
    assert_eq!(filed.acknowledgement.as_deref(), Some("FTA-ACK-1188"));

    // Not linked to the firm's agency, missing, or not on the staff.
    let other = t.open_return(&s.engagement, d(12, 28)).await;
    let other = t.return_at(&other, &s.crew, ReturnStatus::Approved).await;
    let elsewhere = t
        .user_with_grants("Eva", &[perm::RETURNS_VIEW, perm::RETURNS_FILE])
        .await;
    t.qualifications
        .register_elsewhere(elsewhere.id, "TA-2002", d(12, 31));
    let unregistered = t
        .user_with_grants("Uma", &[perm::RETURNS_VIEW, perm::RETURNS_FILE])
        .await;
    t.qualifications.staff(unregistered.id);
    let outsider = t
        .user_with_grants("Omar B", &[perm::RETURNS_VIEW, perm::RETURNS_FILE])
        .await;
    for user in [&elsewhere, &unregistered, &outsider] {
        file_on(&t, user, &other, "2026-12-05")
            .await
            .assert_status(422);
        assert_eq!(t.return_status(&other).await, ReturnStatus::Approved);
    }
    // The staff member named as the filer is checked, not the user who
    // records it: Tariq's registration is expired on 2026-12-05.
    t.ret_act(
        &s.crew.partner,
        &other,
        "file",
        &[
            ("acknowledgement", "FTA-ACK-2"),
            ("filed_on", "2026-12-05"),
            ("filer", &tariq.id.to_string()),
        ],
    )
    .await
    .assert_status(422);
}

/// Filed by the client through its own EmaraTax user: no tax agent check, the
/// user "falcon.finance" is recorded as filer; without the user name it is
/// refused. An acknowledgement dated in the future is refused too.
#[renox::test]
async fn a_return_filed_by_the_client_records_its_emaratax_user() {
    let t = boot_real().await;
    let s = setup(&t).await;
    let tariq = filer(&t).await;

    t.ret_act(
        &tariq,
        &s.ret,
        "file",
        &[
            ("acknowledgement", "FTA-ACK-77"),
            ("filed_on", "2026-12-04"),
            ("by_client", "1"),
        ],
    )
    .await
    .assert_status(422);
    t.ret_act(
        &tariq,
        &s.ret,
        "file",
        &[
            ("acknowledgement", "FTA-ACK-77"),
            ("filed_on", "2026-12-06"),
            ("by_client", "1"),
            ("client_user", "falcon.finance"),
        ],
    )
    .await
    .assert_status(422);
    assert_eq!(t.return_status(&s.ret).await, ReturnStatus::Approved);

    t.ret_act(
        &tariq,
        &s.ret,
        "file",
        &[
            ("acknowledgement", "FTA-ACK-77"),
            ("filed_on", "2026-12-04"),
            ("by_client", "1"),
            ("client_user", "falcon.finance"),
        ],
    )
    .await
    .assert_ok();
    let filed = t.return_now(&s.ret).await;
    assert_eq!(filed.status, ReturnStatus::Filed);
    assert!(filed.filed_by_client);
    assert_eq!(
        filed.client_emaratax_user.as_deref(),
        Some("falcon.finance")
    );
    assert_eq!(filed.filed_by_user_id, None);
    assert_eq!(filed.tax_agent_registration, None);
    t.get(&tariq, &format!("/accounting/returns/{}", s.ret.id))
        .await
        .assert_ok()
        .assert_see("falcon.finance");
}

/// The client's trade licence expired: `file` succeeds and the return is
/// flagged for the MLRO, who sees the flag; the payment reminder still goes.
#[renox::test]
async fn an_expired_client_kyc_document_flags_the_return_for_the_mlro() {
    let t = boot_real().await;
    t.set_clock_on(d(11, 5), 10, 0);
    let client = t.client().await;
    let reviewer = t.user_with("Rita", support::REVIEWER).await;
    t.kyc(
        &reviewer,
        client.company,
        accounting::seed::TRADE_LICENCE,
        d(11, 30),
    )
    .await;
    let engagement = t.active_engagement(&client).await;
    let crew = t.crew(&client).await;
    let ret = t.open_return(&engagement, d(11, 28)).await;
    let ret = t.return_at(&ret, &crew, ReturnStatus::Approved).await;
    let mlro = t.mlro().await;
    let tariq = filer(&t).await;
    t.qualifications.register(tariq.id, "TA-1001", d(12, 31));

    // Filed while the licence is valid: no flag.
    t.set_clock_on(d(12, 5), 10, 0);
    t.ret_act(
        &tariq,
        &ret,
        "file",
        &[
            ("acknowledgement", "FTA-ACK-9"),
            ("filed_on", "2026-11-20"),
            ("tax_payable", "1250.00"),
        ],
    )
    .await
    .assert_ok();
    assert!(
        accounting::documents::flags_of(t.state(), ret.id)
            .await
            .unwrap()
            .is_empty()
    );

    // The next one is filed after the licence expired.
    let next = t.open_return(&engagement, d(12, 28)).await;
    let next = t.return_at(&next, &crew, ReturnStatus::Approved).await;
    t.ret_act(
        &tariq,
        &next,
        "file",
        &[
            ("acknowledgement", "FTA-ACK-10"),
            ("filed_on", "2026-12-05"),
            ("tax_payable", "300.00"),
        ],
    )
    .await
    .assert_ok();
    assert_eq!(t.return_status(&next).await, ReturnStatus::Filed);
    let flags = accounting::documents::flags_of(t.state(), next.id)
        .await
        .unwrap();
    assert_eq!(flags.len(), 1);
    assert!(flags[0].label.contains("2026-11-30"), "{}", flags[0].label);
    t.get(&mlro, &format!("/accounting/returns/{}", next.id))
        .await
        .assert_ok()
        .assert_see("KYC document expired");
    t.get(&tariq, &format!("/accounting/returns/{}", next.id))
        .await
        .assert_ok()
        .assert_dont_see("KYC document expired");
    // The flag did not stop the payment reminder.
    assert_eq!(t.notices.of_type(PAYMENT_REMINDER_NOTICE).len(), 2);
}

/// The client is blocked: `file` waits for the MLRO's approval and the
/// return stays `approved`; the filer is told only that the client is on hold
/// and the MLRO's approval is needed. A rejection leaves the return
/// `approved`; the approval of a new request files it and completes the task.
#[renox::test]
async fn a_blocked_clients_return_is_filed_on_the_mlros_approval() {
    let t = boot_real().await;
    let s = setup(&t).await;
    let mlro = t.mlro().await;
    let tariq = filer(&t).await;
    t.qualifications.register(tariq.id, "TA-1001", d(12, 31));
    t.block(s.client.company, "non_payment").await;

    file_on(&t, &tariq, &s.ret, "2026-12-05").await.assert_ok();
    // The filer sees that the MLRO's approval is needed, and never why.
    let page = t
        .get(&tariq, &format!("/accounting/returns/{}", s.ret.id))
        .await;
    page.assert_ok()
        .assert_see("On hold — MLRO approval needed")
        .assert_dont_see("non_payment");
    assert_eq!(t.return_status(&s.ret).await, ReturnStatus::Approved);
    assert_ne!(
        t.task_now(s.task).await.status,
        DeadlineTaskStatus::Completed,
        "nothing is completed before the MLRO approves"
    );
    let pending = approvals::pending_for(t.db(), &s.ret.subject())
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].transition, FILE);
    assert_eq!(pending[0].requester_id, tariq.id);

    // The filer cannot approve; a rejection keeps the return approved.
    assert!(
        t.run(approvals::approve(t.state(), pending[0].id, &tariq, "ok"))
            .await
            .is_err()
    );
    t.run(approvals::reject(
        t.state(),
        pending[0].id,
        &mlro,
        "Wait for the unblock",
    ))
    .await
    .unwrap();
    assert_eq!(t.return_status(&s.ret).await, ReturnStatus::Approved);

    // Asked again, the MLRO approves: filed, and the task completes.
    file_on(&t, &tariq, &s.ret, "2026-12-05").await.assert_ok();
    let again = approvals::pending_for(t.db(), &s.ret.subject())
        .await
        .unwrap();
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].status, ApprovalStatus::Pending);
    t.run(approvals::approve(t.state(), again[0].id, &mlro, "Cleared"))
        .await
        .unwrap();
    let filed = t.return_now(&s.ret).await;
    assert_eq!(filed.status, ReturnStatus::Filed);
    assert_eq!(filed.filed_by_user_id, Some(tariq.id));
    assert_eq!(filed.tax_agent_registration.as_deref(), Some("TA-1001"));
    assert_eq!(filed.filed_on, Some(day("2026-12-05")));
    assert_eq!(
        t.task_now(s.task).await.status,
        DeadlineTaskStatus::Completed
    );
}

/// Filing completes the linked task with the acknowledgement as evidence; a
/// task that was overdue stays *late*. A return with tax payable sends the
/// client approvers the payment reminder; one without payable sends none.
#[renox::test]
async fn filing_completes_the_linked_task_and_reminds_the_client_to_pay() {
    let t = boot_real().await;
    let s = setup(&t).await;
    let tariq = filer(&t).await;
    t.qualifications.register(tariq.id, "TA-1001", d(12, 31));
    t.run(wl_ticketing::deadlines::mark_overdue(t.state(), d(12, 5)))
        .await
        .unwrap();
    assert_eq!(t.task_now(s.task).await.status, DeadlineTaskStatus::Overdue);

    t.ret_act(
        &tariq,
        &s.ret,
        "file",
        &[
            ("acknowledgement", "FTA-ACK-3000"),
            ("filed_on", "2026-12-05"),
            ("tax_payable", "8000.00"),
        ],
    )
    .await
    .assert_ok();
    assert_eq!(t.return_status(&s.ret).await, ReturnStatus::Filed);
    let task = t.task_now(s.task).await;
    assert_eq!(task.status, DeadlineTaskStatus::Completed);
    assert!(task.late, "completed after it went overdue");
    let filed = t.return_now(&s.ret).await;
    assert_eq!(filed.tax_payable, Some(800_000));

    let reminders = t.notices.of_type(PAYMENT_REMINDER_NOTICE);
    assert_eq!(reminders.len(), 1);
    assert_eq!(reminders[0].data["amount"], "AED 8,000.00");
    assert_eq!(reminders[0].data["due"], "2026-11-28");

    // Nothing payable: no reminder.
    let other = t.open_return(&s.engagement, d(12, 28)).await;
    let other = t.return_at(&other, &s.crew, ReturnStatus::Approved).await;
    t.ret_act(
        &tariq,
        &other,
        "file",
        &[
            ("acknowledgement", "FTA-ACK-3001"),
            ("filed_on", "2026-12-05"),
        ],
    )
    .await
    .assert_ok();
    assert_eq!(t.notices.of_type(PAYMENT_REMINDER_NOTICE).len(), 1);
}
