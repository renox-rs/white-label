//! BL-HRM-03 (#193): the PRO rejects a visa case with the authority's reference
//! and the reason; HR gets the follow-up task (cancel onboarding or open another
//! case for a new visa; a deadline before the current visa expires for a
//! renewal), the active deployments are told through `VisaCaseRejected` and the
//! MD is notified. Fees stay posted and the employee does not change state.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_hr::{EmployeeStatus, VisaCaseStatus};
use wl_kernel::documents::dubai_date;

use support::Booted;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// The PRO posts the rejection of `case`.
async fn reject(
    t: &Booted,
    pro: &User,
    case: &wl_hr::VisaCase,
    fields: &[(&str, &str)],
) -> renox::testing::TestResponse {
    t.app.acting_as(pro);
    t.app
        .post(&format!("/hr/visa-cases/{}/reject", case.id), fields)
        .await
}

/// AC1: a renewal `in_progress` for A, whose visa expires 2026-12-31, rejected
/// with "GDRFA-123" and a reason: the case is `rejected`, `VisaCaseRejected`
/// has deadline 2026-12-01 and visa expiry 2026-12-31, HR has a task due
/// 2026-12-01 and the MD is notified.
#[renox::test]
async fn hrm03_ac1_a_rejected_renewal_gives_the_deadline_task_and_tells_the_md() {
    let t = support::boot().await;
    let pro = t.user_with("Pia", "pro").await;
    let md = t.user_with("Basel", "owner").await;
    let a = t.employee_in(60, EmployeeStatus::Active).await;
    let case = t
        .visa_in(&a, "renewal", VisaCaseStatus::InProgress, |c| {
            c.current_expiry = Some(day(2026, 12, 31));
        })
        .await;

    reject(
        &t,
        &pro,
        &case,
        &[
            ("reference", "GDRFA-123"),
            ("reason", "Passport copy unreadable"),
        ],
    )
    .await
    .assert_status(303);

    let after = t.case(&case).await;
    assert_eq!(after.status, VisaCaseStatus::Rejected);
    assert_eq!(after.rejection_reference.as_deref(), Some("GDRFA-123"));
    assert_eq!(
        after.rejection_reason.as_deref(),
        Some("Passport copy unreadable")
    );
    let events = t.visa_rejected.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].case_type, "renewal");
    assert_eq!(events[0].employee, a.employee_id());
    assert_eq!(events[0].deadline, Some(day(2026, 12, 1)));
    assert_eq!(events[0].visa_expiry, Some(day(2026, 12, 31)));
    let tasks = t.tasks.with_key("hr.visa_rejected:");
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        dubai_date(tasks[0].due_at.expect("a due time")),
        day(2026, 12, 1)
    );
    assert!(
        tasks[0].title.contains("renewal refused"),
        "{}",
        tasks[0].title
    );
    let told = t.notices.of_type("hr.visa_rejected");
    assert_eq!(told.len(), 1);
    assert_eq!(support::users(&told[0]), vec![md.id]);
    // HR decides: the employee's state does not change.
    assert_eq!(t.employee(&a).await.status, EmployeeStatus::Active);
}

/// A visa that expires in less than 30 days: the task is due today.
#[renox::test]
async fn hrm03_a_deadline_that_has_passed_is_due_today() {
    let t = support::boot().await;
    let pro = t.user_with("Pia", "pro").await;
    let a = t.employee_in(60, EmployeeStatus::Active).await;
    let case = t
        .visa_in(&a, "renewal", VisaCaseStatus::OnHold, |c| {
            c.current_expiry = Some(day(2026, 12, 31));
        })
        .await;

    t.on(day(2026, 12, 20), async {
        reject(
            &t,
            &pro,
            &case,
            &[("reference", "ICP-9"), ("reason", "Refused")],
        )
        .await
        .assert_status(303);
    })
    .await;

    let events = t.visa_rejected.all();
    assert_eq!(events[0].deadline, Some(day(2026, 12, 20)));
    assert_eq!(events[0].visa_expiry, Some(day(2026, 12, 31)));
    let tasks = t.tasks.with_key("hr.visa_rejected:");
    assert_eq!(
        dubai_date(tasks[0].due_at.expect("a due time")),
        day(2026, 12, 20)
    );
}

/// AC2: a new case gives HR the task "cancel onboarding or open another case",
/// and the event carries no deadline and no expiry.
#[renox::test]
async fn hrm03_ac2_a_rejected_new_case_gives_hr_the_cancel_or_reopen_task() {
    let t = support::boot().await;
    let pro = t.user_with("Pia", "pro").await;
    let hr = t.user_with("Hana", "hr").await;
    let e = t.employee_in(61, EmployeeStatus::Onboarding).await;
    let case = t
        .visa_in(&e, "new", VisaCaseStatus::InProgress, |_| {})
        .await;

    reject(
        &t,
        &pro,
        &case,
        &[("reference", "MOHRE-5"), ("reason", "Refused")],
    )
    .await
    .assert_status(303);

    let tasks = t.tasks.with_key("hr.visa_rejected:");
    assert_eq!(tasks.len(), 1);
    assert!(
        tasks[0]
            .title
            .contains("cancel onboarding or open another case"),
        "{}",
        tasks[0].title
    );
    assert_eq!(tasks[0].due_at, None);
    assert_eq!(
        tasks[0].assignee,
        Some(wl_kernel::tickets::Assignee::user(hr.id))
    );
    let events = t.visa_rejected.all();
    assert_eq!(events[0].case_type, "new");
    assert_eq!((events[0].deadline, events[0].visa_expiry), (None, None));
    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Onboarding);
}

/// AC3: without an authority reference (or without a reason) `reject` is
/// refused and nothing changes.
#[renox::test]
async fn hrm03_ac3_without_an_authority_reference_reject_is_refused() {
    let t = support::boot().await;
    let pro = t.user_with("Pia", "pro").await;
    let e = t.employee_in(61, EmployeeStatus::Active).await;
    let case = t
        .visa_in(&e, "renewal", VisaCaseStatus::InProgress, |_| {})
        .await;

    reject(
        &t,
        &pro,
        &case,
        &[("reference", "  "), ("reason", "Refused")],
    )
    .await
    .assert_status(422);
    reject(&t, &pro, &case, &[("reference", "GDRFA-1"), ("reason", "")])
        .await
        .assert_status(422);

    assert_eq!(t.case(&case).await.status, VisaCaseStatus::InProgress);
    assert!(t.visa_rejected.all().is_empty());
    assert!(t.tasks.opened().is_empty());
}

/// AC4: from `open`, `reject` is not offered; a user without `visas.manage`
/// cannot reject either.
#[renox::test]
async fn hrm03_ac4_from_open_reject_is_not_offered() {
    let t = support::boot().await;
    let pro = t.user_with("Pia", "pro").await;
    let hr = t.user_with("Hana", "hr").await;
    let e = t.employee_in(61, EmployeeStatus::Active).await;
    let open = t.visa_in(&e, "renewal", VisaCaseStatus::Open, |_| {}).await;
    let live = t
        .visa_in(&e, "renewal", VisaCaseStatus::InProgress, |_| {})
        .await;

    reject(
        &t,
        &pro,
        &open,
        &[("reference", "GDRFA-1"), ("reason", "Refused")],
    )
    .await
    .assert_status(422);
    assert_eq!(t.case(&open).await.status, VisaCaseStatus::Open);
    // The machine offers `reject` from the two live states only.
    let machine = wl_hr::VisaCase::machine();
    let from = machine.find("reject").unwrap().from_states();
    assert!(!from.iter().any(|state| state == "open"), "{from:?}");
    assert!(from.iter().any(|state| state == "in_progress"));
    reject(
        &t,
        &hr,
        &live,
        &[("reference", "GDRFA-1"), ("reason", "Refused")],
    )
    .await
    .assert_forbidden();
    assert_eq!(t.case(&live).await.status, VisaCaseStatus::InProgress);
}

/// Fees already posted stay posted, and a refused cancellation or transfer
/// case gets the event only: no task.
#[renox::test]
async fn hrm03_other_types_get_the_event_and_no_task() {
    let t = support::boot().await;
    let pro = t.user_with("Pia", "pro").await;
    let e = t.employee_in(61, EmployeeStatus::Notice).await;
    let case = t
        .visa_in(&e, "cancellation", VisaCaseStatus::InProgress, |_| {})
        .await;

    reject(
        &t,
        &pro,
        &case,
        &[("reference", "GDRFA-2"), ("reason", "Refused")],
    )
    .await
    .assert_status(303);

    assert_eq!(t.visa_rejected.all().len(), 1);
    assert!(t.tasks.opened().is_empty());
    assert!(t.costs.posted().is_empty(), "no fee is reversed or posted");
}
