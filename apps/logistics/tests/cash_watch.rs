//! BL-LOG-13 (#311), 03 §4.1 row *Driver does not hand over the cash, or
//! disappears*, §7.1 *Cash handover*, §7.2 `LOG.RUL.13`, `FIN.RUL.22`,
//! `FIN.ACL.06`, `FIN.ACL.19` (open item L-20): 12 hours after the run's
//! return to the depot (or the end of the driver's shift for a run still
//! out) the operations manager gets a task to call the driver; after 48 hours
//! the company admin is told, the manager is asked to count what was
//! received and the driver's HR record gets an absence note unless the driver
//! checked in; the client's remittance is never reduced. The delivery module
//! is the real wl-delivery and the cash is the real wl-payments custody.
//!
//! Instants: a run returned at "2026-11-03 20:00" is modelled by the instant
//! the run really returned ([`returned_at`]); the other instants are
//! relative to it, so each boundary is exact.

mod support;

use logistics::cash_watch::{self, CASH_ESCALATED, CashWatch, Watched};
use renox::auth::permissions;
use renox::chrono::Duration;
use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::tickets::{Assignee, OpenTicket};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::custody::{
    CashCollection, HandoverPeriod, HandoverStatus, HandoverSummary, custody,
};
use wl_kernel::delivery::RunId;
use wl_kernel::documents::today;
use wl_payments::cash_escalation::{self, Escalated};
use wl_payments::cash_notices::CASH_TASK;
use wl_payments::permissions as pperm;

use logistics::permissions as perm;
use support::roster::employee_of;
use support::{SOUQ, World, boot_with_payments};

/// The operations manager: holds the role and counts and writes off cash.
async fn manager(w: &World, name: &str) -> User {
    permissions::define_role(w.db(), perm::OPERATIONS_MANAGER_ROLE, &[perm::SHIPMENTS_VIEW])
        .await
        .unwrap();
    let user = w
        .user_with(
            name,
            &[
                perm::SHIPMENTS_VIEW,
                pperm::CASH_VIEW,
                pperm::CASH_COUNT,
                pperm::CASH_WRITE_OFF,
            ],
        )
        .await;
    user.assign_role(w.db(), perm::OPERATIONS_MANAGER_ROLE)
        .await
        .unwrap();
    user
}

/// The company admin.
async fn admin(w: &World) -> User {
    permissions::define_role(w.db(), wl_kernel::COMPANY_ADMIN_ROLE, &[])
        .await
        .unwrap();
    let user = w.user_with("Amira", &[perm::SHIPMENTS_VIEW]).await;
    user.assign_role(w.db(), wl_kernel::COMPANY_ADMIN_ROLE)
        .await
        .unwrap();
    user
}

/// A COD shipment of AED 1,800 delivered by the driver on a run that is
/// still out: the cash is in the driver's open handover. Returns the run.
async fn out_with_cash(w: &World, reference: &str) -> i64 {
    let shipment = w
        .book_with(&w.dispatcher, SOUQ, reference, &[("cod", "1800")])
        .await;
    let (_out, delivery) = w.out_for_delivery(&shipment).await;
    w.deliver_stop(&delivery, "1800.00")
        .await
        .assert_status(303);
    assert_eq!(w.reload(&shipment).await.status, logistics::ShipmentStatus::Delivered);
    w.delivery_of(&shipment).await.run_id.expect("on a run")
}

/// The driver's handover of `run`.
async fn handover(w: &World, run: i64) -> HandoverSummary {
    custody(w.state())
        .handover_of(w.db(), w.driver.id, &RunId(run).subject())
        .await
        .unwrap()
        .expect("the driver has a handover for the run")
}

/// The driver returns the run to the depot, with the cash not handed over.
async fn returned_at(w: &World, run: i64) -> DateTime {
    w.post(&w.driver, &format!("/driver/runs/{run}/return"), &[])
        .await
        .assert_ok();
    handover(w, run)
        .await
        .ended_at
        .expect("the return ended the period")
}

fn hours(n: i64) -> Duration {
    Duration::hours(n)
}

fn minutes(n: i64) -> Duration {
    Duration::minutes(n)
}

fn call_tickets(w: &World) -> Vec<OpenTicket> {
    w.tickets
        .opened()
        .into_iter()
        .filter(|t| {
            t.source_key
                .as_deref()
                .is_some_and(|key| key.starts_with("logistics.cash_call:"))
        })
        .collect()
}

fn count_tickets(w: &World) -> usize {
    w.tickets
        .opened()
        .iter()
        .filter(|t| {
            t.source_key
                .as_deref()
                .is_some_and(|key| key.starts_with("logistics.cash_count:"))
        })
        .count()
}

/// AC1: given a run returned to the depot with the handover `open` (AED
/// 1,800 expected), the operations manager gets a task 12 hours later, not a
/// minute before, and only once.
#[renox::test]
async fn the_manager_gets_a_task_12_hours_after_the_return() {
    let w = boot_with_payments().await;
    let mona = manager(&w, "Mona").await;
    let run = out_with_cash(&w, "ORD-1").await;
    let start = returned_at(&w, run).await;
    let open = handover(&w, run).await;
    assert_eq!(open.status, HandoverStatus::Open);
    assert_eq!(open.expected, 180_000);
    assert!(!open.escalate, "the vertical rule replaces FIN.RUL.22");

    let early = cash_watch::run(w.state(), start + hours(12) - minutes(1))
        .await
        .unwrap();
    assert_eq!(early, Watched::default());
    assert!(call_tickets(&w).is_empty());

    let done = cash_watch::run(w.state(), start + hours(12)).await.unwrap();
    assert_eq!(done, Watched { tasks: 1, ..Watched::default() });
    let tasks = call_tickets(&w);
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].title.contains("Call Dev"));
    assert!(tasks[0].title.contains("AED 1800.00"));
    assert_eq!(tasks[0].assignee, Some(Assignee::user(mona.id)));
    assert_eq!(tasks[0].subject, open.id.subject());
    assert!(!tasks[0].description.contains("abort"));
    let watch = CashWatch::query().first(w.db()).await.unwrap().unwrap();
    assert_eq!(watch.handover_id, open.id.0);
    assert_eq!(watch.started_at, start);
    assert!(watch.task_at.is_some() && watch.admin_at.is_none());

    // Not repeated, and nothing is told to the admin yet.
    let again = cash_watch::run(w.state(), start + hours(13)).await.unwrap();
    assert_eq!(again, Watched::default());
    assert_eq!(call_tickets(&w).len(), 1);
    assert!(w.notices.of_type(CASH_ESCALATED).is_empty());
}

/// AC2: given a run still out and the driver's shift ending 18:00, the task
/// 12 hours later asks to call the driver and abort the run; aborting it for
/// *driver unreachable* opens the count.
#[renox::test]
async fn for_a_run_still_out_the_clock_starts_at_the_end_of_the_shift_and_asks_to_abort() {
    let w = boot_with_payments().await;
    manager(&w, "Mona").await;
    let run = out_with_cash(&w, "ORD-1").await;
    // The shift ended 18:00 Dubai (14:00 UTC) on the run date.
    let shift_end: DateTime = format!("{}T14:00:00Z", today()).parse().unwrap();
    w.shifts.set(w.driver.id, today(), shift_end);
    assert!(handover(&w, run).await.ended_at.is_none());

    let early = cash_watch::run(w.state(), shift_end + hours(12) - minutes(1))
        .await
        .unwrap();
    assert_eq!(early, Watched::default());
    let done = cash_watch::run(w.state(), shift_end + hours(12)).await.unwrap();
    assert_eq!(done.tasks, 1);
    let tasks = call_tickets(&w);
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].description.contains("abort the run"));
    assert!(tasks[0].description.contains("driver unreachable"));
    let watch = CashWatch::query().first(w.db()).await.unwrap().unwrap();
    assert_eq!(watch.started_at, shift_end);
    assert!(watch.run_out);

    // The dispatcher aborts the run for *driver unreachable*: the handover
    // can be counted.
    w.post(
        &w.dispatcher,
        &format!("/runs/{run}/abort"),
        &[("reason", "driver_unreachable")],
    )
    .await
    .assert_ok();
    assert!(handover(&w, run).await.ended_at.is_some());
}

/// Without a recorded shift a run still out starts its clock at 23:59 of the
/// run date.
#[renox::test]
async fn without_a_shift_a_run_still_out_starts_at_23_59_of_the_run_date() {
    let w = boot_with_payments().await;
    manager(&w, "Mona").await;
    let run = out_with_cash(&w, "ORD-1").await;
    // 23:59 Dubai is 19:59 UTC.
    let start: DateTime = format!("{}T19:59:00Z", today()).parse().unwrap();

    let open = handover(&w, run).await;
    assert_eq!(cash_watch::clock_start(w.state(), &open).await.unwrap(), start);
    assert_eq!(
        cash_watch::run(w.state(), start + hours(12) - minutes(1)).await.unwrap(),
        Watched::default()
    );
    assert_eq!(
        cash_watch::run(w.state(), start + hours(12)).await.unwrap().tasks,
        1
    );
}

/// AC3: 48 hours after the start the company admin is told, the manager is
/// asked to count and the driver's HR record gets an absence note, once;
/// counting AED 0 makes the handover `short` AED 1,800.
#[renox::test]
async fn after_48_hours_the_admin_is_told_the_manager_counts_and_the_driver_has_an_absence_note() {
    let w = boot_with_payments().await;
    let mona = manager(&w, "Mona").await;
    let boss = admin(&w).await;
    let run = out_with_cash(&w, "ORD-1").await;
    let start = returned_at(&w, run).await;
    cash_watch::run(w.state(), start + hours(12)).await.unwrap();

    let early = cash_watch::run(w.state(), start + hours(48) - minutes(1))
        .await
        .unwrap();
    assert_eq!(early, Watched::default());
    assert!(w.notices.of_type(CASH_ESCALATED).is_empty());
    assert!(w.absences.all().is_empty());

    let done = cash_watch::run(w.state(), start + hours(48)).await.unwrap();
    assert_eq!(
        done,
        Watched {
            admin_notices: 1,
            absence_notes: 1,
            ..Watched::default()
        }
    );
    let notices = w.notices.users_of_type(CASH_ESCALATED);
    assert_eq!(notices, vec![boss.id], "the company admin is told, once");
    assert_eq!(count_tickets(&w), 1, "the manager is asked to count");
    let absences = w.absences.all();
    assert_eq!(absences.len(), 1);
    assert_eq!(absences[0].employee, employee_of(w.driver.id));
    assert_eq!(absences[0].case, RunId(run).subject());
    assert_eq!(absences[0].opened_on, today());
    assert_eq!(absences[0].last_present_on, today());

    // Counted at AED 0 by the manager: short by the whole amount.
    let open = handover(&w, run).await;
    w.post(&mona, &format!("/cash/{}/count", open.id.0), &[("counted", "0")])
        .await
        .assert_ok();
    let short = handover(&w, run).await;
    assert_eq!(short.status, HandoverStatus::Short);
    assert_eq!(short.difference, 180_000);
    assert_eq!(short.counted, 0);
    // The tasks close with the count, and nothing is sent twice.
    assert!(!w.tickets.completed().is_empty());
    let after = cash_watch::run(w.state(), start + hours(72)).await.unwrap();
    assert_eq!(after.admin_notices, 0);
    assert_eq!(w.absences.all().len(), 1);
    assert_eq!(w.notices.of_type(CASH_ESCALATED).len(), 1);
}

/// AC4: the client's next remittance still includes the AED 1,800 of COD on
/// the run's delivered shipments, whatever became of the cash.
#[renox::test]
async fn the_clients_remittance_still_includes_the_cod_of_the_missing_cash() {
    let w = boot_with_payments().await;
    let mona = manager(&w, "Mona").await;
    admin(&w).await;
    let run = out_with_cash(&w, "ORD-1").await;
    let start = returned_at(&w, run).await;
    cash_watch::run(w.state(), start + hours(48)).await.unwrap();
    let open = handover(&w, run).await;
    w.post(&mona, &format!("/cash/{}/count", open.id.0), &[("counted", "0")])
        .await
        .assert_ok();
    assert_eq!(handover(&w, run).await.status, HandoverStatus::Short);

    w.rule().await;

    let remittance = w.the_remittance_of(SOUQ).await;
    assert_eq!(remittance.cod_total, 180_000);
}

/// AC5: with `FIN.ACL.19` off a salary deduction is refused, so recovery is in
/// cash only; `write_off` of AED 1,800 (limit AED 50) needs another
/// manager's approval.
#[renox::test]
async fn recovery_is_in_cash_only_and_the_write_off_needs_approval() {
    let w = boot_with_payments().await;
    let mona = manager(&w, "Mona").await;
    let nadia = manager(&w, "Nadia").await;
    admin(&w).await;
    let run = out_with_cash(&w, "ORD-1").await;
    let start = returned_at(&w, run).await;
    cash_watch::run(w.state(), start + hours(48)).await.unwrap();
    let open = handover(&w, run).await;
    let path = format!("/cash/{}", open.id.0);
    w.post(&mona, &format!("{path}/count"), &[("counted", "0")])
        .await
        .assert_ok();

    // No salary deduction while the setting is off.
    w.post(&mona, &format!("{path}/deductions"), &[("amount", "1800")])
        .await
        .assert_status(422);
    // A cash recovery is recorded as usual.
    w.post(&mona, &format!("{path}/recoveries"), &[("amount", "20")])
        .await
        .assert_ok();
    assert_eq!(handover(&w, run).await.recovered, 2_000);

    // The write-off of the rest waits for the second manager.
    w.post(&mona, &format!("{path}/write-off"), &[("reason", "Driver unreachable")])
        .await
        .assert_ok();
    assert_eq!(handover(&w, run).await.status, HandoverStatus::Short);
    let request = ApprovalRequest::pending_about(w.db(), &open.id.subject())
        .await
        .unwrap()
        .pop()
        .expect("a request waits");
    assert_eq!(request.payload.0["amount"], 178_000);
    assert!(approvals::approve(w.state(), request.id, &mona, "mine").await.is_err());
    approvals::approve(w.state(), request.id, &nadia, "agreed")
        .await
        .unwrap();
    assert_eq!(handover(&w, run).await.status, HandoverStatus::WrittenOff);
}

/// AC6: a handover counted before the task (a driver who came back) produces
/// no task, no admin notice and no absence note.
#[renox::test]
async fn a_handover_counted_in_time_produces_no_task() {
    let w = boot_with_payments().await;
    let mona = manager(&w, "Mona").await;
    admin(&w).await;
    let run = out_with_cash(&w, "ORD-1").await;
    let start = returned_at(&w, run).await;
    let open = handover(&w, run).await;

    w.post(&mona, &format!("/cash/{}/count", open.id.0), &[("counted", "1800")])
        .await
        .assert_ok();
    assert_eq!(handover(&w, run).await.status, HandoverStatus::Counted);

    for later in [12, 13, 48, 49] {
        let done = cash_watch::run(w.state(), start + hours(later)).await.unwrap();
        assert_eq!(done, Watched::default(), "{later} hours");
    }
    assert!(call_tickets(&w).is_empty());
    assert_eq!(count_tickets(&w), 0);
    assert!(w.notices.of_type(CASH_ESCALATED).is_empty());
    assert!(w.absences.all().is_empty());
}

/// A driver who comes back after the task but before M is counted as usual:
/// the task closes and no admin notice or absence note follows.
#[renox::test]
async fn a_driver_who_hands_over_after_the_task_closes_it() {
    let w = boot_with_payments().await;
    let mona = manager(&w, "Mona").await;
    admin(&w).await;
    let run = out_with_cash(&w, "ORD-1").await;
    let start = returned_at(&w, run).await;
    cash_watch::run(w.state(), start + hours(12)).await.unwrap();
    assert_eq!(call_tickets(&w).len(), 1);

    let open = handover(&w, run).await;
    w.post(&mona, &format!("/cash/{}/count", open.id.0), &[("counted", "1800")])
        .await
        .assert_ok();

    assert!(!w.tickets.completed().is_empty(), "the task closes");
    let watch = CashWatch::query().first(w.db()).await.unwrap().unwrap();
    assert!(watch.closed_at.is_some());
    let done = cash_watch::run(w.state(), start + hours(48)).await.unwrap();
    assert_eq!(done, Watched::default());
    assert!(w.absences.all().is_empty());
    assert!(w.notices.of_type(CASH_ESCALATED).is_empty());
}

/// AC7: given the driver checked in at the depot after the start but the
/// handover is still `open` at M, the admin notice and the count request are
/// sent and no absence note is written.
#[renox::test]
async fn a_driver_who_checked_in_gets_no_absence_note() {
    let w = boot_with_payments().await;
    manager(&w, "Mona").await;
    let boss = admin(&w).await;
    let run = out_with_cash(&w, "ORD-1").await;
    let start = returned_at(&w, run).await;
    w.attendance
        .check_in(employee_of(w.driver.id), start + hours(12) + minutes(30));

    let done = cash_watch::run(w.state(), start + hours(48)).await.unwrap();

    assert_eq!(
        done,
        Watched {
            tasks: 1,
            admin_notices: 1,
            absence_notes: 0,
            closed: 0
        }
    );
    assert_eq!(w.notices.users_of_type(CASH_ESCALATED), vec![boss.id]);
    assert_eq!(count_tickets(&w), 1);
    assert!(w.absences.all().is_empty());
    let watch = CashWatch::query().first(w.db()).await.unwrap().unwrap();
    assert_eq!(watch.absence_skipped.as_deref(), Some("checked_in"));
    assert!(watch.absence_noted_at.is_none());
}

/// AC8: for the driver's handover `FIN.RUL.22` creates no task and no admin
/// notice (only `LOG.RUL.13` runs); a counter-staff handover left `open`
/// still gets the `FIN.RUL.22` task.
#[renox::test]
async fn the_module_rule_skips_the_drivers_handover_and_keeps_the_counter_staffs() {
    let w = boot_with_payments().await;
    manager(&w, "Mona").await;
    admin(&w).await;
    let run = out_with_cash(&w, "ORD-1").await;
    let start = returned_at(&w, run).await;
    // Counter staff collected AED 500 in a day period, with the module rule.
    let counter = w.user_with("Cora", &[perm::SHIPMENTS_VIEW]).await;
    let day = HandoverPeriod::day(today());
    custody(w.state())
        .collect(
            w.state(),
            CashCollection::new(counter.id, day, 50_000, Subject::new("sales.order", 9)),
        )
        .await
        .unwrap()
        .expect("a handover");

    // 48 hours after the driver's start: past both clocks of both handovers.
    let late = start + hours(72);
    let escalated = cash_escalation::run(w.state(), late).await.unwrap();
    assert_eq!(
        escalated,
        Escalated {
            tasks: 1,
            admin_notices: 1
        },
        "only the counter staff's handover is escalated by FIN.RUL.22"
    );
    let notices = w.notices.of_type(CASH_TASK);
    assert!(
        notices
            .iter()
            .all(|n| !n.data["text"].as_str().unwrap_or_default().contains("Dev")),
        "nothing about the driver"
    );

    // The vertical rule acts for the driver only.
    let done = cash_watch::run(w.state(), late).await.unwrap();
    assert_eq!(done.tasks, 1);
    assert_eq!(done.admin_notices, 1);
}
