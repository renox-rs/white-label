//! BL-PAY-14 (#107): `FIN.RUL.18` chases a bounced cheque. Three days after
//! the bounce the party (open amount and bounce fee, not while blocked) and
//! the responsible staff are reminded; fourteen days after it the accountant
//! and the manager get a task to re-present, refer to legal or write off. It
//! stops once the cheque leaves `bounced`, and counts again from a new bounce.
//!
//! The rule is run with the day it is to act on (the job passes today in
//! Asia/Dubai), so a test names the day instead of waiting for it.

mod support;

use renox::chrono::{NaiveDate, TimeDelta};
use wl_kernel::cheques::ChequeStatus;
use wl_kernel::documents::today;
use wl_kernel::notify::NotifyRecipient;
use wl_payments::cheque_chase;
use wl_payments::followup::{FollowupKind, FollowupStatus};
use wl_payments::gateway::FakeGateway;
use wl_payments::notices::{CHEQUE_REMINDER, CHEQUE_REMINDER_STAFF, CHEQUE_TASK};
use wl_payments::{Cheque, FollowupTask, WlPayments};

use support::{BLUE_DUNES, Booted, DUSK_HOLDINGS, boot, boot_with};

fn after(days: i64) -> NaiveDate {
    today() + TimeDelta::days(days)
}

fn user_ids(recipients: &[NotifyRecipient]) -> Vec<i64> {
    recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect()
}

async fn decisions(t: &Booted, cheque: &Cheque) -> Vec<FollowupTask> {
    FollowupTask::of_cheque(t.db(), cheque.id)
        .await
        .unwrap()
        .into_iter()
        .filter(|task| task.kind == FollowupKind::DecideCheque)
        .collect()
}

/// A cheque of AED 20,000.00 on which the bank paid AED 6,000.00 and that
/// bounced today for the rest, AED 14,000.00, with the bounce fee AED 500.00.
async fn bounced_with_fee(t: &Booted, staff: &renox::auth::User, number: &str) -> Cheque {
    let rent = t.invoices.add(BLUE_DUNES, &format!("INV-{number}"), 2_000_000);
    t.bounced_cheque(staff, BLUE_DUNES, number, 2_000_000, Some(rent), "6000.00")
        .await
}

async fn boot_with_fee() -> Booted {
    boot_with(
        WlPayments::new().bounce_fee("bounce_fee", 50_000),
        FakeGateway::new(),
    )
    .await
}

/// AC1: a cheque that bounced with AED 14,000.00 open and a bounce fee of AED
/// 500.00: nothing on day 2; the party notice on day 3 shows both amounts, the
/// responsible staff are told; the tasks are made on day 14 for the accountant
/// and the manager, not before.
#[renox::test]
async fn the_party_and_staff_are_reminded_at_three_days_and_the_decision_at_fourteen() {
    let t = boot_with_fee().await;
    let a = t.cheque_staff("Amal").await;
    let accountant = t.accountant("Cara").await;
    let manager = t.finance_manager("Mona").await;
    let cheque = bounced_with_fee(&t, &a, "6001").await;
    assert_eq!(cheque.open_amount, 1_400_000);
    assert_eq!(cheque.bounce_fee_total, 50_000);
    assert_eq!(cheque.bounced_on, Some(today()));

    // Day 2: nothing yet.
    let report = cheque_chase::run(t.state(), after(2)).await.unwrap();
    assert_eq!((report.reminded, report.decisions), (0, 0));
    assert!(t.notices.of_type(CHEQUE_REMINDER).is_empty());

    // Day 3: the party and the staff.
    let report = cheque_chase::run(t.state(), after(3)).await.unwrap();
    assert_eq!((report.reminded, report.decisions), (1, 0));
    let party = t.notices.of_type(CHEQUE_REMINDER);
    assert_eq!(party.len(), 1);
    assert!(matches!(
        party[0].recipients[0],
        NotifyRecipient::Party { id } if id == BLUE_DUNES
    ));
    let data = party[0].data.to_string();
    for part in ["6001", "AED 14,000.00", "AED 500.00"] {
        assert!(data.contains(part), "{part} in {data}");
    }
    let staff = t.notices.of_type(CHEQUE_REMINDER_STAFF);
    assert_eq!(staff.len(), 1);
    assert!(user_ids(&staff[0].recipients).contains(&accountant.id));
    assert!(decisions(&t, &cheque).await.is_empty());

    // The same day again, and the next days: no second reminder.
    cheque_chase::run(t.state(), after(3)).await.unwrap();
    cheque_chase::run(t.state(), after(13)).await.unwrap();
    assert_eq!(t.notices.of_type(CHEQUE_REMINDER).len(), 1);
    assert!(decisions(&t, &cheque).await.is_empty());

    // Day 14: the accountant's and the manager's tasks.
    let report = cheque_chase::run(t.state(), after(14)).await.unwrap();
    assert_eq!((report.reminded, report.decisions), (0, 1));
    let tasks = decisions(&t, &cheque).await;
    assert_eq!(tasks.len(), 2);
    let mut roles: Vec<&str> = tasks.iter().map(|task| task.role.as_str()).collect();
    roles.sort_unstable();
    assert_eq!(roles, ["accountant", "finance_manager"]);
    assert!(tasks.iter().all(|task| task.status == FollowupStatus::Open));
    assert!(tasks[0].text.contains("re-present, refer to legal or write off"));
    assert_eq!(tasks[0].amount, 1_400_000);
    let managers = FollowupTask::open_for(t.db(), "finance_manager").await.unwrap();
    assert!(managers.iter().any(|task| task.kind == FollowupKind::DecideCheque));
    let told: Vec<i64> = t
        .notices
        .of_type(CHEQUE_TASK)
        .iter()
        .flat_map(|r| user_ids(&r.recipients))
        .collect();
    assert!(told.contains(&manager.id));
    assert!(told.contains(&accountant.id));

    // Never twice.
    cheque_chase::run(t.state(), after(15)).await.unwrap();
    assert_eq!(decisions(&t, &cheque).await.len(), 2);
}

/// The staff told are those who collected the cheque when one is known.
#[renox::test]
async fn the_user_who_collected_the_cheque_is_told() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    let collector = t.user("Colin", &[]).await;
    let entity = t.entity("PGP").await;
    let request = wl_kernel::cheques::RegisterCheque::new(
        BLUE_DUNES,
        entity.id,
        "ENBD",
        "6002",
        today(),
        1_000_000,
    )
    .collected_by(collector.id, false);
    let cheque = t.register_cheque(request).await;
    let cheque = t.deposit_cheque(&a, &cheque).await;
    t.bounce_cheque(&a, &cheque, "insufficient_funds", "CERT-1", "")
        .await
        .assert_ok();

    cheque_chase::run(t.state(), after(3)).await.unwrap();

    let staff = t.notices.of_type(CHEQUE_REMINDER_STAFF);
    assert_eq!(user_ids(&staff[0].recipients), vec![collector.id]);
}

/// AC2: a referral on day 9 means no task on day 14, and nothing more once
/// the cheque is a legal case.
#[renox::test]
async fn a_referral_before_the_decision_day_stops_the_chase() {
    let t = boot_with_fee().await;
    let a = t.cheque_staff("Amal").await;
    let cheque = bounced_with_fee(&t, &a, "6003").await;
    cheque_chase::run(t.state(), after(3)).await.unwrap();
    assert_eq!(t.notices.of_type(CHEQUE_REMINDER).len(), 1);

    t.refer_cheque(&a, &cheque, "civil_execution", None)
        .await
        .assert_ok();
    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::LegalCase);

    let report = cheque_chase::run(t.state(), after(14)).await.unwrap();
    assert_eq!((report.reminded, report.decisions), (0, 0));
    assert!(decisions(&t, &cheque).await.is_empty());
    assert_eq!(t.notices.of_type(CHEQUE_REMINDER).len(), 1);
}

/// A cheque referred before the first reminder is not reminded.
#[renox::test]
async fn no_reminder_goes_for_a_cheque_that_is_no_longer_bounced() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    let inv = t.invoices.add(BLUE_DUNES, "INV-9", 1_000_000);
    let cheque = t
        .bounced_cheque(&a, BLUE_DUNES, "6004", 1_000_000, Some(inv), "")
        .await;
    // Written off at once (no approval needed).
    t.write_off_cheque(&a, &cheque, "untraceable").await.assert_ok();
    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::WrittenOff);

    let report = cheque_chase::run(t.state(), after(14)).await.unwrap();
    assert_eq!((report.reminded, report.decisions), (0, 0));
    assert!(t.notices.of_type(CHEQUE_REMINDER).is_empty());
    assert!(t.notices.of_type(CHEQUE_REMINDER_STAFF).is_empty());
}

/// A cheque that is replaced is not chased, and the decision task made before
/// is closed.
#[renox::test]
async fn the_decision_task_is_closed_when_the_cheque_is_replaced() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    let cheque = t
        .bounced_cheque(&a, BLUE_DUNES, "6005", 1_000_000, None, "")
        .await;
    cheque_chase::run(t.state(), after(14)).await.unwrap();
    assert_eq!(decisions(&t, &cheque).await.len(), 2);

    // A new cheque for the full open amount replaces it.
    let new = t.received_cheque(BLUE_DUNES, "6010", 1_000_000, None).await;
    t.link_replacement(&a, &cheque, "cheque", new.id)
        .await
        .assert_ok();
    t.replace_cheque(&a, &cheque).await.assert_ok();

    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::Replaced);
    assert!(decisions(&t, &cheque)
        .await
        .iter()
        .all(|task| task.status == FollowupStatus::Done));
}

/// Edge: a blocked party gets no reminder; the staff still do, and the tasks
/// are made as usual.
#[renox::test]
async fn a_blocked_party_is_not_reminded_but_the_staff_are() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    t.accountant("Cara").await;
    let cheque = t
        .bounced_cheque(&a, DUSK_HOLDINGS, "6006", 1_000_000, None, "")
        .await;

    cheque_chase::run(t.state(), after(3)).await.unwrap();

    assert!(t.notices.of_type(CHEQUE_REMINDER).is_empty());
    assert_eq!(t.notices.of_type(CHEQUE_REMINDER_STAFF).len(), 1);
    // The reminder counts as done: it is not sent when the party is unblocked
    // later in the same bounce.
    cheque_chase::run(t.state(), after(4)).await.unwrap();
    assert_eq!(t.notices.of_type(CHEQUE_REMINDER_STAFF).len(), 1);
    cheque_chase::run(t.state(), after(14)).await.unwrap();
    assert_eq!(decisions(&t, &cheque).await.len(), 2);
}

/// Edge: a part payment leaves the cheque bounced and the reminder shows the
/// new open amount.
#[renox::test]
async fn the_reminder_shows_the_open_amount_after_a_part_payment() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    let inv = t.invoices.add(BLUE_DUNES, "INV-10", 2_000_000);
    let cheque = t
        .bounced_cheque(&a, BLUE_DUNES, "6007", 2_000_000, Some(inv), "")
        .await;
    t.settle_cheque(&a, &cheque, "cash", "5000.00", "")
        .await
        .assert_ok();
    assert_eq!(t.cheque(cheque.id).await.open_amount, 1_500_000);

    cheque_chase::run(t.state(), after(3)).await.unwrap();

    let data = t.notices.of_type(CHEQUE_REMINDER)[0].data.to_string();
    assert!(data.contains("AED 15,000.00"), "{data}");
}

/// Edge: re-presented and bounced again, the count starts from the new bounce.
#[renox::test]
async fn a_second_bounce_restarts_the_count() {
    let t = boot().await;
    t.travel_to("2030-03-01", 8, 0);
    let a = t.cheque_staff("Amal").await;
    let cheque = t
        .bounced_cheque(&a, BLUE_DUNES, "6008", 1_000_000, None, "")
        .await;
    let first = NaiveDate::from_ymd_opt(2030, 3, 1).unwrap();
    assert_eq!(cheque.bounced_on, Some(first));
    cheque_chase::run(t.state(), first + TimeDelta::days(3)).await.unwrap();
    assert_eq!(t.notices.of_type(CHEQUE_REMINDER).len(), 1);

    // Presented again and returned again on the 6th.
    t.travel_to("2030-03-06", 8, 0);
    t.represent_cheque(&a, &cheque, "2030-03-06", true, "the party confirmed funds")
        .await
        .assert_ok();
    t.bounce_cheque(&a, &cheque, "insufficient_funds", "CERT-2", "")
        .await
        .assert_ok();
    let again = t.cheque(cheque.id).await;
    assert_eq!(again.status, ChequeStatus::Bounced);
    let second = NaiveDate::from_ymd_opt(2030, 3, 6).unwrap();
    assert_eq!(again.bounced_on, Some(second));

    // Three days after the first bounce's reminder is not enough...
    cheque_chase::run(t.state(), second + TimeDelta::days(2)).await.unwrap();
    assert_eq!(t.notices.of_type(CHEQUE_REMINDER).len(), 1);
    // ...three days after the second bounce is.
    cheque_chase::run(t.state(), second + TimeDelta::days(3)).await.unwrap();
    assert_eq!(t.notices.of_type(CHEQUE_REMINDER).len(), 2);
    cheque_chase::run(t.state(), second + TimeDelta::days(14)).await.unwrap();
    assert_eq!(decisions(&t, &cheque).await.len(), 2);
}

/// The days are a setting.
#[renox::test]
async fn the_reminder_days_are_a_setting() {
    let t = boot_with(WlPayments::new().cheque_chase(5, 20), FakeGateway::new()).await;
    let a = t.cheque_staff("Amal").await;
    let cheque = t
        .bounced_cheque(&a, BLUE_DUNES, "6009", 1_000_000, None, "")
        .await;
    cheque_chase::run(t.state(), after(4)).await.unwrap();
    assert!(t.notices.of_type(CHEQUE_REMINDER).is_empty());
    cheque_chase::run(t.state(), after(5)).await.unwrap();
    assert_eq!(t.notices.of_type(CHEQUE_REMINDER).len(), 1);
    cheque_chase::run(t.state(), after(19)).await.unwrap();
    assert!(decisions(&t, &cheque).await.is_empty());
    cheque_chase::run(t.state(), after(20)).await.unwrap();
    assert_eq!(decisions(&t, &cheque).await.len(), 2);
}
