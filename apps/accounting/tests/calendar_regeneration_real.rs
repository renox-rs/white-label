//! #339 (BL-ACC-15, `ACC.RUL.10`) with the real wl-ticketing: the compliance
//! calendar is rebuilt from the effective date of an approved tax profile
//! change. Upcoming tasks that no longer apply are waived with *tax profile
//! changed*, a task whose period stays gets the new due date (audited), the
//! new tasks are created, and started or overdue tasks that no longer match
//! and returns in progress for affected periods are listed for the manager.
//! The whole chain runs: the accountant's request, the manager's approval,
//! the listener, the rule.

mod support;

use accounting::calendar::{self, VAT_RETURN};
use accounting::regeneration::{self, RULE, list_key};
use accounting::returns::{self, NewReturn};
use accounting::{Engagement, EngagementStatus, ReturnType, TaxProfile, suspension};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::approvals;
use wl_kernel::automation::{RuleRun, RunOutcome};
use wl_kernel::deadlines::{
    DeadlineTaskId, DeadlineTaskStatus, DeadlineTaskSummary, NewDeadlineTask, TaskRecurrence,
    deadline_tasks,
};
use wl_kernel::tickets::task_creator;

use support::{Booted, Client, MANAGER, boot_real};

fn day(text: &str) -> NaiveDate {
    text.parse().unwrap()
}

/// A client with its quarters ending in January, an `active` engagement and
/// the calendar the old profile generates on 2026-11-02: the VAT return for
/// the quarter ending 2027-01-31 and the corporate tax return for 2026.
async fn january_calendar(t: &Booted) -> (Client, Engagement) {
    t.set_clock(2, 10, 0);
    let client = t.client().await;
    t.tax_profile_staggered(&client, day("2026-01-31")).await;
    let engagement = t.active_engagement(&client).await;
    let made = calendar::generate(t.state(), &engagement, day("2026-11-02"))
        .await
        .unwrap();
    assert_eq!(made.len(), 2, "a VAT and a corporate tax task");
    (client, engagement)
}

/// A VAT task of `engagement` for the quarter ending `end`, due `due`.
async fn vat(t: &Booted, engagement: &Engagement, end: &str, due: &str) -> DeadlineTaskId {
    let end = day(end);
    deadline_tasks(t.state())
        .create(
            t.state(),
            NewDeadlineTask::new(
                engagement.subject(),
                VAT_RETURN,
                &format!("VAT return, period ending {end}"),
                day(due),
            )
            .party(engagement.client())
            .lead_days(21)
            .owner_role("accountant")
            .reviewer_role(MANAGER)
            .evidence_required()
            .recurrence(TaskRecurrence::Quarterly)
            .period_end(end),
        )
        .await
        .unwrap()
        .expect("ticketing is on")
}

/// The accountant asks for the quarters ending `first_end`, effective
/// `effective`, with QFZP `qfzp`; a manager approves.
async fn change_and_approve(
    t: &Booted,
    client: &Client,
    first_end: &str,
    qfzp: &str,
    effective: &str,
) -> i64 {
    let accountant = t.accountant().await;
    let manager = t.manager_named("Bashir").await;
    t.post(
        &accountant,
        &format!(
            "/accounting/clients/{}/tax-profile/change",
            client.company.0
        ),
        &[
            ("trn", "100000000000003"),
            ("vat_registered", "1"),
            ("vat_period", "quarterly"),
            ("vat_first_period_end", first_end),
            ("financial_year_end", "2026-12-31"),
            ("ct_registration_no", "CT-100200300"),
            ("qfzp", qfzp),
            ("effective_date", effective),
        ],
    )
    .await
    .assert_ok();
    let change = accounting::tax_profile_change::TaxProfileChange::pending(t.db(), client.company)
        .await
        .unwrap()
        .expect("pending");
    let request = approvals::pending_for(t.db(), &change.subject())
        .await
        .unwrap()
        .remove(0);
    t.run(approvals::approve(t.state(), request.id, &manager, "ok"))
        .await
        .unwrap();
    change.id
}

/// The tasks of `engagement` now, by due date.
async fn tasks_of(t: &Booted, engagement: &Engagement) -> Vec<DeadlineTaskSummary> {
    deadline_tasks(t.state())
        .of_subject(t.db(), &engagement.subject())
        .await
        .unwrap()
}

/// The VAT task for the period ending `end` that is not waived, if any.
fn live(tasks: &[DeadlineTaskSummary], end: &str) -> Option<DeadlineTaskSummary> {
    tasks
        .iter()
        .find(|task| {
            task.task_type == VAT_RETURN
                && task.period_end == Some(day(end))
                && task.status != DeadlineTaskStatus::NotRequired
        })
        .cloned()
}

/// The description of the manager's list of `engagement` after the change
/// that created the profile version `profile`.
async fn manager_list(t: &Booted, engagement: &Engagement, profile: i64) -> Option<String> {
    let key = list_key(engagement.id, profile);
    wl_ticketing::Ticket::query()
        .where_(wl_ticketing::Ticket::SOURCE_KEY.eq(key.as_str()))
        .first(t.db())
        .await
        .unwrap()
        .map(|ticket| ticket.description)
}

/// The id of the newest profile version of `client`.
async fn newest(t: &Booted, client: &Client) -> TaxProfile {
    TaxProfile::latest(t.db(), client.company)
        .await
        .unwrap()
        .unwrap()
}

/// Quarterly VAT tasks for the quarters ending 2027-01-31 and 2027-04-30
/// (`upcoming`) and a change to the quarters ending in February effective
/// 2027-02-01: the January task is kept, the April one is waived with *tax
/// profile changed* (and no next occurrence), and tasks are created for the
/// first new period (to 2027-02-28) and the quarter ending 2027-05-31. The
/// corporate tax task is untouched; running again changes nothing.
#[renox::test]
async fn the_upcoming_tasks_that_no_longer_apply_are_waived_and_the_new_ones_created() {
    let t = boot_real().await;
    let (client, engagement) = january_calendar(&t).await;
    let january = live(&tasks_of(&t, &engagement).await, "2027-01-31").unwrap();
    assert_eq!(january.due_date, day("2027-02-28"));
    let april = vat(&t, &engagement, "2027-04-30", "2027-05-28").await;
    let before = tasks_of(&t, &engagement).await;

    let change = change_and_approve(&t, &client, "2026-02-28", "", "2027-02-01").await;

    let after = tasks_of(&t, &engagement).await;
    // The period that ended before the effective date is not compared.
    let kept = live(&after, "2027-01-31").expect("the January task is kept");
    assert_eq!(kept.id, january.id);
    assert_eq!(kept.due_date, day("2027-02-28"));
    assert_eq!(kept.status, DeadlineTaskStatus::Upcoming);
    // The April task no longer applies: waived with the reason, no next one.
    let waived = t.task_now(april).await;
    assert_eq!(waived.status, DeadlineTaskStatus::NotRequired);
    assert_eq!(waived.closed_reason.as_deref(), Some("tax profile changed"));
    assert_eq!(waived.next, None, "the regeneration creates the new tasks");
    // The new tasks: the transitional period and the quarter after it.
    let first = live(&after, "2027-02-28").expect("the first new period");
    assert_eq!(first.due_date, day("2027-03-28"));
    assert_eq!(first.status, DeadlineTaskStatus::Upcoming);
    let quarter = live(&after, "2027-05-31").expect("the quarter ending 2027-05-31");
    assert_eq!(quarter.due_date, day("2027-06-28"));
    assert_eq!(
        after.len(),
        before.len() + 2,
        "two created, nothing deleted or duplicated"
    );
    // The corporate tax return is not touched.
    let ct = |tasks: &[DeadlineTaskSummary]| {
        tasks
            .iter()
            .filter(|task| task.task_type == calendar::CT_RETURN)
            .map(|task| (task.id, task.status))
            .collect::<Vec<_>>()
    };
    assert_eq!(ct(&after), ct(&before));
    // Nothing was left for the manager.
    let profile = newest(&t, &client).await;
    assert!(manager_list(&t, &engagement, profile.id).await.is_none());

    // Running the rule again creates and changes nothing.
    let again = t.run(regeneration::apply(t.state(), change)).await.unwrap();
    assert_eq!(again.len(), 1);
    assert!(again[0].created.is_empty() && again[0].waived.is_empty());
    assert!(again[0].rescheduled.is_empty() && again[0].listed.is_empty());
    assert_eq!(tasks_of(&t, &engagement).await.len(), after.len());
    let rerun = t
        .run(regeneration::run_for_change(t.state(), change))
        .await
        .unwrap();
    assert_eq!(rerun, RunOutcome::Skipped, "one rule run per change");
    let subject = accounting::tax_profile_change::TaxProfileChange::find_or_404(t.db(), change)
        .await
        .unwrap()
        .subject();
    let run = RuleRun::find_occurrence(t.db(), RULE, &subject, &format!("{RULE}:{subject}"))
        .await
        .unwrap();
    assert!(run.is_some());
}

/// The April task already `in_progress` and no longer matching is listed for
/// the manager and not waived; the January task, whatever its state, is not
/// listed. A VAT return in `preparing` for the affected period is on the list
/// too.
#[renox::test]
async fn a_started_task_and_a_return_in_progress_are_listed_for_the_manager() {
    let t = boot_real().await;
    let manager = t.manager().await;
    let (client, engagement) = january_calendar(&t).await;
    let january = live(&tasks_of(&t, &engagement).await, "2027-01-31").unwrap();
    let april = vat(&t, &engagement, "2027-04-30", "2027-05-28").await;
    t.task_act(&manager, april, "start", &[]).await.assert_ok();
    t.task_act(&manager, january.id, "start", &[])
        .await
        .assert_ok();
    let ret = returns::open(
        t.state(),
        None,
        NewReturn::new(
            engagement.id,
            ReturnType::Vat,
            day("2027-04-30"),
            day("2027-05-28"),
        ),
    )
    .await
    .unwrap();
    assert_eq!(ret.task_id, Some(april.0), "linked to the April task");

    change_and_approve(&t, &client, "2026-02-28", "", "2027-02-01").await;

    // Not waived.
    assert_eq!(
        t.task_now(april).await.status,
        DeadlineTaskStatus::InProgress
    );
    assert_eq!(
        t.task_now(january.id).await.status,
        DeadlineTaskStatus::InProgress
    );
    // The manager's list names the April task and the return, not January.
    let profile = newest(&t, &client).await;
    let list = manager_list(&t, &engagement, profile.id)
        .await
        .expect("the manager has a list");
    assert!(
        list.contains("VAT return, period ending 2027-04-30"),
        "{list}"
    );
    assert!(list.contains("in_progress"), "{list}");
    assert!(list.contains(&ret.number()), "{list}");
    assert!(!list.contains("2027-01-31"), "{list}");
    let opened = task_creator(t.state())
        .find_open(t.state(), &list_key(engagement.id, profile.id))
        .await
        .unwrap();
    assert!(opened.is_some());
    // The new tasks are still created.
    let after = tasks_of(&t, &engagement).await;
    assert!(live(&after, "2027-02-28").is_some());
    assert!(live(&after, "2027-05-31").is_some());
}

/// An `upcoming` task whose period stays but whose due date moves from
/// 2027-05-28 to 2027-06-28 keeps its id with the new due date (audited) and
/// is not waived; a started one with another due date is listed instead.
#[renox::test]
async fn a_task_whose_period_stays_gets_the_new_due_date_and_keeps_its_id() {
    let t = boot_real().await;
    t.set_clock(2, 10, 0);
    let client = t.client().await;
    t.tax_profile_staggered(&client, day("2026-02-28")).await;
    let engagement = t.active_engagement(&client).await;
    let february = vat(&t, &engagement, "2027-02-28", "2027-03-28").await;
    let may = vat(&t, &engagement, "2027-05-31", "2027-05-28").await;
    let august = vat(&t, &engagement, "2027-08-31", "2027-09-30").await;
    t.task_act(&t.manager().await, august, "start", &[])
        .await
        .assert_ok();

    // The stagger stays; QFZP status changes from 2027-02-01.
    change_and_approve(&t, &client, "2026-02-28", "1", "2027-02-01").await;

    let moved = t.task_now(may).await;
    assert_eq!(moved.status, DeadlineTaskStatus::Upcoming);
    assert_eq!(moved.due_date, day("2027-06-28"));
    assert_eq!(moved.start_date, day("2027-06-07"), "the lead time follows");
    let unchanged = t.task_now(february).await;
    assert_eq!(unchanged.due_date, day("2027-03-28"));
    assert_eq!(unchanged.status, DeadlineTaskStatus::Upcoming);
    // The move is audited with both dates and the reason.
    let audit = renox::audit::for_subject(t.db(), wl_ticketing::DeadlineTask::TABLE, may.0, 10)
        .await
        .unwrap();
    let entry = audit
        .iter()
        .find(|entry| entry.action == "task.rescheduled")
        .expect("the new due date is audited");
    let data = entry.data.clone();
    assert!(data.to_string().contains("2027-05-28"), "{data}");
    assert!(data.to_string().contains("2027-06-28"), "{data}");
    assert!(data.to_string().contains("tax profile changed"), "{data}");
    // The started one is not moved: it is listed for the manager.
    assert_eq!(t.task_now(august).await.due_date, day("2027-09-30"));
    let profile = newest(&t, &client).await;
    let list = manager_list(&t, &engagement, profile.id)
        .await
        .expect("the started task is listed");
    assert!(list.contains("2027-08-31"), "{list}");
    assert!(!list.contains("2027-05-31"), "{list}");
    // Nothing was waived or created twice.
    let all = tasks_of(&t, &engagement).await;
    assert!(
        all.iter()
            .all(|task| task.status != DeadlineTaskStatus::NotRequired),
        "nothing waived"
    );
    assert_eq!(
        all.iter()
            .filter(|task| task.task_type == VAT_RETURN)
            .count(),
        3
    );
}

/// An effective date in the past: an `upcoming` task for a period ending on
/// or after it, due before today, that no longer applies is waived; an
/// overdue one is listed and never waived. A suspended engagement gets the
/// new tasks held.
#[renox::test]
async fn a_past_effective_date_waives_the_upcoming_and_lists_the_overdue() {
    let t = boot_real().await;
    t.set_clock(2, 10, 0);
    let client = t.client().await;
    t.tax_profile_staggered(&client, day("2026-01-31")).await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Suspended, Some("client_request"))
        .await;
    let october = vat(&t, &engagement, "2026-10-31", "2026-10-28").await;
    let december = vat(&t, &engagement, "2026-12-31", "2026-10-15").await;
    // The overdue run of 2026-10-20 marks the December task (due 10-15) and
    // not yet the October one (due 10-28), which is `upcoming` with a due
    // date before today (2026-11-02).
    t.run(wl_ticketing::deadlines::mark_overdue(
        t.state(),
        day("2026-10-20"),
    ))
    .await
    .unwrap();
    assert_eq!(
        t.task_now(december).await.status,
        DeadlineTaskStatus::Overdue
    );
    assert_eq!(
        t.task_now(october).await.status,
        DeadlineTaskStatus::Upcoming
    );
    t.run(suspension::hold(t.state(), &engagement, day("2026-11-02")))
        .await
        .unwrap();

    change_and_approve(&t, &client, "2026-02-28", "", "2026-10-01").await;

    assert_eq!(
        t.task_now(october).await.status,
        DeadlineTaskStatus::NotRequired,
        "the upcoming task that no longer applies is waived"
    );
    assert_eq!(
        t.task_now(december).await.status,
        DeadlineTaskStatus::Overdue,
        "an overdue task is never waived"
    );
    let profile = newest(&t, &client).await;
    let list = manager_list(&t, &engagement, profile.id)
        .await
        .expect("the overdue task is listed");
    assert!(list.contains("2026-12-31"), "{list}");
    let all = tasks_of(&t, &engagement).await;
    let created = live(&all, "2026-11-30").expect("the first new period");
    assert!(created.held, "created held for a suspended engagement");
}
