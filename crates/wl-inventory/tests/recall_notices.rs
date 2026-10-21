//! The notices of a recall, a notice that could not be delivered and the
//! close with its report (wl-inventory §4 *Recall* `send_notices`, `close`,
//! §5.7 steps 5 and 6, issue #235): every point of "Done when" through the
//! module's routes, the real stock hold decisions, the kernel's approvals and
//! the `MessageGaveUp` event of wl-notify.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_inventory::movement::MovementType;
use wl_inventory::recall_notice::{notice_key, notice_types, task_key};
use wl_inventory::{HoldStatus, LotStatus, NoticeStatus, Recall, RecallLine, RecallStatus};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::notify::{NotifyChannel, NotifyRecipient};
use wl_kernel::{PartyId, Subject};

use support::{Booted, boot};

fn at(day: u32) -> DateTime {
    NaiveDate::from_ymd_opt(2026, 10, day)
        .unwrap()
        .and_hms_opt(8, 0, 0)
        .unwrap()
        .and_utc()
}

fn visit(n: i64) -> Subject {
    Subject::new("clinics.visit", n)
}

/// A recall on lot L9 (5 kg left at Branch 1) after visits 1 to `visits` used
/// it, patient `10 + n` behind visit `n`; the recall is `traced`.
async fn traced_recall(t: &Booted, visits: i64) -> (Recall, wl_inventory::Item, wl_inventory::Lot) {
    let serum = t.tracked_item("SERUM").await;
    let l9 = t.lot(&serum, "L9", LotStatus::Released).await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.stock_in(&serum, &branch, Some(&l9), 5_000 + visits * 1_000, 1_000)
        .await;
    for n in 1..=visits {
        let used = visit(n);
        t.movement(
            MovementType::Issue,
            &serum,
            Some(&l9),
            Some(&branch),
            None,
            1_000,
            &used,
            Some(&used),
            at(u32::try_from(n).unwrap() + 1),
            false,
        )
        .await;
        t.trace_parties.set(&used, PartyId(10 + n));
    }
    let manager = t.recaller("Rita").await;
    let item_id = serum.id.to_string();
    t.recall_as(
        &manager,
        &[
            ("source", "supplier"),
            ("reason", "Contamination found by the supplier"),
            ("items", item_id.as_str()),
            ("lot_numbers", "L9"),
        ],
    )
    .await
    .assert_ok();
    let recall = t.all_recalls().await.pop().expect("a recall");
    assert_eq!(recall.status, RecallStatus::Traced);
    (recall, serum, l9)
}

/// The users and parties each notice of `kind` went to.
fn told_parties(t: &Booted, kind: &str) -> Vec<i64> {
    let mut parties: Vec<i64> = t
        .notices
        .all()
        .iter()
        .filter(|request| request.type_key == kind)
        .flat_map(|request| request.recipients.clone())
        .filter_map(|recipient| match recipient {
            NotifyRecipient::Party { id } => Some(id.0),
            _ => None,
        })
        .collect();
    parties.sort_unstable();
    parties
}

/// Done when 1: 3 affected lines and the *patient* template: the lines are
/// `sent`, each party is told through wl-notify, and the three templates have
/// an English and an Arabic text.
#[renox::test]
async fn notices_with_the_patient_template_mark_the_lines_sent() {
    let t = boot().await;
    let (recall, _, _) = traced_recall(&t, 3).await;
    let manager = t.recaller("Rita").await;
    let lines = t.recall_lines(&recall).await;
    assert_eq!(lines.len(), 3);

    let all: Vec<&RecallLine> = lines.iter().collect();
    t.notices_as(&manager, &recall, "patient", &all)
        .await
        .assert_ok();

    let now = t.recall_lines(&recall).await;
    assert!(now.iter().all(|l| l.notice_status == NoticeStatus::Sent));
    assert!(now.iter().all(|l| l.template.as_deref() == Some("patient")));
    assert!(now.iter().all(|l| l.sent_by == Some(manager.id)));
    assert_eq!(told_parties(&t, &notice_key("patient")), vec![11, 12, 13]);
    let sent = t.notices.all();
    let first = sent
        .iter()
        .find(|r| r.type_key == notice_key("patient"))
        .unwrap();
    assert_eq!(first.subject, lines[0].subject());
    assert_eq!(first.data["lot"], "L9");

    for def in notice_types() {
        assert!(def.template_for(NotifyChannel::Email, "en").is_some());
        let arabic = def.template_for(NotifyChannel::Email, "ar").unwrap();
        assert_eq!(arabic.language, "ar");
    }

    // A line that was told is not sent again, and only a holder of
    // `recalls.manage` sends.
    t.notices_as(&manager, &recall, "patient", &[&now[0]])
        .await
        .assert_status(422);
    let keeper = t.storekeeper("Amal").await;
    t.notices_as(&keeper, &recall, "patient", &[&now[0]])
        .await
        .assert_forbidden();
    assert_eq!(t.notices.all().len(), sent.len());
}

/// The template and the lines are guards: none chosen, a template that does
/// not exist, no line, and a line with no party each refuse the notice.
#[renox::test]
async fn a_notice_needs_a_template_lines_and_a_party() {
    let t = boot().await;
    let (recall, serum, l9) = traced_recall(&t, 1).await;
    let manager = t.recaller("Rita").await;
    // A job card whose patient the vertical does not know.
    let branch = t.named_location("BRANCH-2", "Branch 2").await;
    t.stock_in(&serum, &branch, Some(&l9), 1_000, 1_000).await;
    let job = Subject::new("clinics.job", 4);
    t.movement(
        MovementType::Issue,
        &serum,
        Some(&l9),
        Some(&branch),
        None,
        500,
        &job,
        Some(&job),
        at(9),
        false,
    )
    .await;
    t.trace_as(&manager, &recall).await.assert_ok();
    let lines = t.recall_lines(&recall).await;
    assert_eq!(lines.len(), 2);
    let unknown = lines.iter().find(|l| l.party_id.is_none()).unwrap();
    let known = lines.iter().find(|l| l.party_id.is_some()).unwrap();

    t.notices_as(&manager, &recall, "", &[known])
        .await
        .assert_status(422);
    t.notices_as(&manager, &recall, "press", &[known])
        .await
        .assert_status(422);
    t.notices_as(&manager, &recall, "patient", &[])
        .await
        .assert_status(422);
    t.notices_as(&manager, &recall, "patient", &[unknown])
        .await
        .assert_status(422);
    assert!(t.notices.all().iter().all(|r| !r.type_key.starts_with("inventory.recall_notice_")));
    assert!(
        t.recall_lines(&recall)
            .await
            .iter()
            .all(|l| l.notice_status == NoticeStatus::Pending)
    );
}

/// Done when 2: `MessageGaveUp` for line 2 returns it to `pending` and the
/// recall owner gets one task for it; it can be sent again.
#[renox::test]
async fn a_notice_that_could_not_be_delivered_returns_the_line_and_opens_a_task() {
    let t = boot().await;
    let (recall, _, _) = traced_recall(&t, 3).await;
    let manager = t.recaller("Rita").await;
    let lines = t.recall_lines(&recall).await;
    let all: Vec<&RecallLine> = lines.iter().collect();
    t.notices_as(&manager, &recall, "patient", &all)
        .await
        .assert_ok();

    t.gave_up(&lines[1], "patient").await;

    let now = t.recall_lines(&recall).await;
    assert_eq!(now[0].notice_status, NoticeStatus::Sent);
    assert_eq!(now[1].notice_status, NoticeStatus::Pending);
    assert_eq!(now[2].notice_status, NoticeStatus::Sent);
    let tasks = t.tickets.opened();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].subject, recall.subject());
    assert_eq!(
        tasks[0].source_key.as_deref(),
        Some(task_key(lines[1].id).as_str())
    );
    assert!(tasks[0].assignee.is_some());

    // Another line given up gets a task of its own.
    t.gave_up(&lines[0], "patient").await;
    assert_eq!(t.tickets.opened().len(), 2);
    assert_eq!(
        t.recall_lines(&recall).await[0].notice_status,
        NoticeStatus::Pending
    );

    // Sent again, with another template.
    t.notices_as(&manager, &recall, "customer", &[&now[1]])
        .await
        .assert_ok();
    let again = t.recall_lines(&recall).await;
    assert_eq!(again[1].notice_status, NoticeStatus::Sent);
    assert_eq!(again[1].template.as_deref(), Some("customer"));
}

/// Done when 3: line 2 waived with a reason and the recall's hold `closed`:
/// `close` succeeds, a report PDF is kept, `RecallClosed` is emitted. Until
/// then a pending line refuses it, and a waiver needs a reason.
#[renox::test]
async fn a_recall_closes_with_its_report_once_every_line_is_told_or_waived() {
    let t = boot().await;
    let (recall, _, l9) = traced_recall(&t, 3).await;
    let manager = t.recaller("Rita").await;
    let releaser = t.releaser("Ann").await;
    let lines = t.recall_lines(&recall).await;
    t.notices_as(&manager, &recall, "patient", &[&lines[0], &lines[2]])
        .await
        .assert_ok();

    // Line 2 is pending and the hold is open: refused, nothing changes.
    t.close_recall_as(&manager, &recall).await.assert_status(422);
    assert_eq!(t.recall_now(&recall).await.status, RecallStatus::Traced);

    t.waive_as(&manager, &lines[1], "  ").await.assert_status(422);
    t.waive_as(&manager, &lines[1], "Told in person")
        .await
        .assert_ok();
    let waived = t.recall_lines(&recall).await;
    assert_eq!(waived[1].notice_status, NoticeStatus::Waived);
    assert_eq!(
        waived[1].waive_reason.as_deref(),
        Some("Told in person")
    );
    assert_eq!(waived[1].waived_by, Some(manager.id));
    // A line already decided cannot be waived again.
    t.waive_as(&manager, &lines[0], "again").await.assert_status(422);

    // Every line is decided, the hold is not: still refused.
    t.close_recall_as(&manager, &recall).await.assert_status(422);
    assert!(t.recall_closed.all().is_empty());

    // The held stock is discarded; the hold closes by itself.
    let hold = t.all_holds().await.pop().unwrap();
    assert_eq!(hold.status, HoldStatus::Open);
    let held = t.hold_lines(&hold).await.remove(0);
    t.decide_as(&releaser, &held, "discard").await.assert_ok();
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Closed);
    assert_eq!(t.lot_now(&l9).await.status, LotStatus::Rejected);

    let keeper = t.storekeeper("Amal").await;
    t.close_recall_as(&keeper, &recall).await.assert_forbidden();
    t.close_recall_as(&manager, &recall).await.assert_ok();

    let closed = t.recall_now(&recall).await;
    assert_eq!(closed.status, RecallStatus::Closed);
    assert_eq!(closed.closed_by, Some(manager.id));
    let report = closed.report.clone().expect("a report");
    assert!(report.starts_with("%PDF"), "{report}");
    assert!(report.contains(&recall.number.clone().unwrap()));
    assert!(report.contains("Told in person"));
    t.app.acting_as(&manager);
    t.app
        .get(&format!("/recalls/{}/report", recall.id))
        .await
        .assert_ok();

    let events = t.recall_closed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].recall, recall.subject());
    assert_eq!(events[0].lines.len(), 3);
    assert_eq!(events[0].outcomes.len(), 1);
    assert_eq!(events[0].lots, vec![Subject::new("inventory.lot", l9.id)]);

    // A closed recall is final.
    t.close_recall_as(&manager, &recall).await.assert_status(422);
    t.trace_as(&manager, &recall).await.assert_status(422);
    assert_eq!(t.recall_closed.all().len(), 1);
}

/// Done when 4: a `release` of the recalled lot still waiting for its
/// `STK.ACL.05` approval leaves the hold `open` and `close` is refused; after
/// the second person approves, it closes.
#[renox::test]
async fn a_release_waiting_for_the_second_person_keeps_the_recall_open() {
    let t = boot().await;
    let (recall, _, l9) = traced_recall(&t, 0).await;
    let manager = t.recaller("Rita").await;
    let ann = t.releaser("Ann").await;
    let ben = t.releaser("Ben").await;
    assert!(t.recall_lines(&recall).await.is_empty());
    let hold = t.all_holds().await.pop().unwrap();

    // An undecided line of the hold refuses the close.
    t.close_recall_as(&manager, &recall).await.assert_status(422);

    let line = t.hold_lines(&hold).await.remove(0);
    t.decide_as(&ann, &line, "release").await.assert_ok();
    let requests = ApprovalRequest::pending_about(t.db(), &line.subject())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Open);
    t.close_recall_as(&manager, &recall).await.assert_status(422);
    assert_eq!(t.recall_now(&recall).await.status, RecallStatus::Traced);
    assert!(t.recall_closed.all().is_empty());

    approvals::approve(t.state(), requests[0].id, &ben, "checked")
        .await
        .unwrap();
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Closed);
    assert_eq!(t.lot_now(&l9).await.status, LotStatus::Released);

    t.close_recall_as(&manager, &recall).await.assert_ok();
    assert_eq!(t.recall_now(&recall).await.status, RecallStatus::Closed);
    assert_eq!(t.recall_closed.all().len(), 1);
}
