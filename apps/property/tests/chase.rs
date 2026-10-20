//! BL-PRP-02 (#252): chase a lease waiting for signature (`PRP.RUL.12`). After
//! 14 days the leasing agent and the property manager are reminded once;
//! after 30 days the property manager gets one task to extend or cancel;
//! extending closes the task and a lease still waiting on the new date gets a
//! new one. The rule never cancels and never writes to the tenant.

mod support;

use property::chase::{self, MAX_EXTENSION_DAYS, run};
use property::notices::LEASE_UNSIGNED;
use property::{Lease, LeaseStatus};
use renox::chrono::Duration;
use renox::prelude::*;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::tickets::{TicketId, task_creator};

use support::{AGENT, Booted, MANAGER, boot, d, day};

/// A lease waiting for signature since `since`, with its agent and manager.
async fn waiting(t: &Booted, since: renox::chrono::NaiveDate) -> (Lease, User, User) {
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    // Submitted on `since` by the real transition: the clock is that day.
    let lease = t.at(since, t.pending(&agent, &unit, 0)).await;
    assert_eq!(lease.submitted_on, Some(since));
    (lease, agent, manager)
}

/// The users a request went to.
fn users(request: &wl_kernel::notify::NotifyRequest) -> Vec<i64> {
    request
        .recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect()
}

/// AC1: on day 14 the leasing agent and the property manager are notified,
/// once; nothing earlier, nothing again, nothing to the tenant.
#[renox::test]
async fn bl_prp_02_the_agent_and_the_manager_are_reminded_once_at_fourteen_days() {
    let t = boot().await;
    let since = d(3, 1);
    let (lease, agent, manager) = waiting(&t, since).await;

    let early = run(t.state(), since + Duration::days(13)).await.unwrap();
    assert_eq!((early.reminded, early.tasked), (0, 0));
    assert!(t.notices.of_type(LEASE_UNSIGNED).is_empty());

    let at_14 = run(t.state(), since + Duration::days(14)).await.unwrap();
    assert_eq!(at_14.reminded, 1);
    let sent = t.notices.of_type(LEASE_UNSIGNED);
    assert_eq!(sent.len(), 1);
    let mut to = users(&sent[0]);
    to.sort_unstable();
    let mut expected = vec![agent.id, manager.id];
    expected.sort_unstable();
    assert_eq!(to, expected);
    assert!(
        sent[0]
            .recipients
            .iter()
            .all(|r| matches!(r, NotifyRecipient::User { .. }))
    );
    assert_eq!(sent[0].subject, lease.subject());
    assert_eq!(sent[0].data["days"], 14);

    let again = run(t.state(), since + Duration::days(15)).await.unwrap();
    assert_eq!(again.reminded, 0);
    assert_eq!(t.notices.of_type(LEASE_UNSIGNED).len(), 1);
    // Nothing yet for the manager's list.
    assert!(t.tasks.with_key("property.chase:").is_empty());
}

/// The leasing agent is the user who drafted the lease when it is known.
#[renox::test]
async fn bl_prp_02_the_agent_who_drafted_the_lease_is_the_one_reminded() {
    let t = boot().await;
    let (lease, _agent, manager) = waiting(&t, d(3, 1)).await;
    let drafter = t.user_in("Dina", AGENT, lease.building_id).await;
    t.set(&lease, &[("created_by", &Some(drafter.id))]).await;
    run(t.state(), d(3, 21)).await.unwrap();
    let sent = t.notices.of_type(LEASE_UNSIGNED);
    let mut to = users(&sent[0]);
    to.sort_unstable();
    let mut expected = vec![drafter.id, manager.id];
    expected.sort_unstable();
    assert_eq!(to, expected);
}

/// AC2: on day 30 the property manager has one task to extend or cancel; a
/// second run adds none and the lease is not cancelled.
#[renox::test]
async fn bl_prp_02_the_manager_gets_one_extend_or_cancel_task_at_thirty_days() {
    let t = boot().await;
    let since = d(3, 1);
    let (lease, _agent, manager) = waiting(&t, since).await;
    let before = run(t.state(), since + Duration::days(29)).await.unwrap();
    assert_eq!(before.tasked, 0);

    let at_30 = run(t.state(), since + Duration::days(30)).await.unwrap();
    assert_eq!(at_30.tasked, 1);
    let tasks = t.tasks.with_key("property.chase:");
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].open);
    assert_eq!(tasks[0].subject, lease.subject());
    assert_eq!(
        tasks[0].assignee,
        Some(wl_kernel::tickets::Assignee::user(manager.id))
    );
    assert!(
        tasks[0].title.contains(&lease.number()),
        "{}",
        tasks[0].title
    );

    let again = run(t.state(), since + Duration::days(31)).await.unwrap();
    assert_eq!(again.tasked, 0);
    assert_eq!(t.tasks.with_key("property.chase:").len(), 1);
    // The rule never cancels.
    assert_eq!(t.status(&lease).await, LeaseStatus::PendingSignature);
}

/// AC3: extending closes the task; on the extended date a lease still
/// waiting gets a new one; an extended date not reached has none.
#[renox::test]
async fn bl_prp_02_extending_closes_the_task_and_a_new_one_comes_on_the_new_date() {
    let t = boot().await;
    let since = d(3, 1);
    let (lease, _agent, manager) = waiting(&t, since).await;
    run(t.state(), since + Duration::days(30)).await.unwrap();
    let path = format!("/property/leases/{}/extend", lease.id);

    // On 2027-03-31: at most 30 days ahead, and after today.
    let task_day = d(3, 31);
    let far = (task_day + Duration::days(MAX_EXTENSION_DAYS + 1)).to_string();
    t.at(task_day, t.post(&manager, &path, &[("until", &far)]))
        .await
        .assert_status(422);
    let today = task_day.to_string();
    t.at(task_day, t.post(&manager, &path, &[("until", &today)]))
        .await
        .assert_status(422);
    assert!(t.tasks.with_key("property.chase:")[0].open);

    // Extending to 2027-04-15 closes the task.
    let until = d(4, 15);
    t.at(
        task_day,
        t.post(&manager, &path, &[("until", &until.to_string())]),
    )
    .await
    .assert_ok();
    let extended = t.reload(&lease).await;
    assert_eq!(extended.follow_up_until, Some(until));
    assert_eq!(extended.status, LeaseStatus::PendingSignature);
    let tasks = t.tasks.with_key("property.chase:");
    assert!(!tasks[0].open);
    assert!(
        tasks[0].note.contains(&until.to_string()),
        "{}",
        tasks[0].note
    );

    // Before the new date: nothing.
    let before = run(t.state(), until - Duration::days(1)).await.unwrap();
    assert_eq!(before.tasked, 0);
    // On it: a new task, once.
    let on = run(t.state(), until).await.unwrap();
    assert_eq!(on.tasked, 1);
    let tasks = t.tasks.with_key("property.chase:");
    assert_eq!(tasks.len(), 2);
    assert!(tasks[1].open);
    assert!(
        tasks[1]
            .key
            .as_deref()
            .unwrap()
            .ends_with(&until.to_string())
    );
    assert_eq!(run(t.state(), until).await.unwrap().tasked, 0);
}

/// A task closed by hand without extending is not opened again for the same
/// date.
#[renox::test]
async fn bl_prp_02_a_closed_task_is_not_opened_again() {
    let t = boot().await;
    let since = d(3, 1);
    let (_lease, _agent, _manager) = waiting(&t, since).await;
    run(t.state(), since + Duration::days(30)).await.unwrap();
    let task = &t.tasks.with_key("property.chase:")[0];
    task_creator(t.state())
        .complete(t.state(), TicketId(task.id), "handled by phone")
        .await
        .unwrap();
    let after = run(t.state(), since + Duration::days(35)).await.unwrap();
    assert_eq!(after.tasked, 0);
    assert_eq!(t.tasks.with_key("property.chase:").len(), 1);
}

/// AC4: a lease activated meanwhile gets no reminder and no task; a cancelled
/// one neither.
#[renox::test]
async fn bl_prp_02_a_lease_that_left_pending_signature_is_not_chased() {
    let t = boot().await;
    let since = d(3, 1);
    let (lease, _agent, manager) = waiting(&t, since).await;
    // Activated on 2027-03-10, before the reminder of the 15th.
    t.at(d(3, 10), t.waive(&manager, &lease)).await;
    t.at(d(3, 10), t.activate(&manager, &lease))
        .await
        .assert_ok();
    assert_eq!(t.status(&lease).await, LeaseStatus::Active);
    for day in [d(3, 15), d(3, 31), since + Duration::days(40)] {
        let report = run(t.state(), day).await.unwrap();
        assert_eq!((report.reminded, report.tasked, report.failed), (0, 0, 0));
    }
    assert!(t.notices.of_type(LEASE_UNSIGNED).is_empty());
    assert!(t.tasks.with_key("property.chase:").is_empty());
}

/// Only a property manager of the building extends, and only a lease waiting
/// for signature has a follow-up.
#[renox::test]
async fn bl_prp_02_the_permission_and_the_state_are_checked() {
    let t = boot().await;
    let (lease, agent, manager) = waiting(&t, d(3, 1)).await;
    let path = format!("/property/leases/{}/extend", lease.id);
    let until = day(5).to_string();
    t.post(&agent, &path, &[("until", &until)])
        .await
        .assert_forbidden();
    t.post(&manager, &path, &[("until", "soon")])
        .await
        .assert_status(422);
    t.waive(&manager, &lease).await;
    t.activate(&manager, &lease).await.assert_ok();
    t.post(&manager, &path, &[("until", &until)])
        .await
        .assert_status(422);
    assert_eq!(
        chase::task_due_on(&t.reload(&lease).await),
        Some(d(3, 1) + Duration::days(30))
    );
}
