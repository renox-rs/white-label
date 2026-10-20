//! BL-PAY-12 (#105): a bounced received cheque is referred to a legal case:
//! civil execution by default, a criminal complaint only for the return
//! reasons on the configured list. The case owner gets a task with the
//! evidence pack, the party a formal notice, and the manager is told. The
//! platform records the case; it files nothing.
//!
//! It also checks BL-PAY-11 from `legal_case` (a part payment, and replacing
//! with the legal case task closed).

mod support;

use renox::chrono::NaiveDate;
use wl_kernel::cheques::{ChequeId, ChequeOutcome, ChequeStatus, RegisterCheque};
use wl_kernel::documents::today;
use wl_kernel::notify::NotifyRecipient;
use wl_payments::cheque::CaseType;
use wl_payments::followup::{FollowupKind, FollowupStatus};
use wl_payments::gateway::FakeGateway;
use wl_payments::notices::{CHEQUE_NOTICE, CHEQUE_REFERRED, CHEQUE_TASK};
use wl_payments::{Cheque, FollowupTask, WlPayments};

use support::{BLUE_DUNES, Booted, DUSK_HOLDINGS, boot, boot_with};

async fn tasks_of(t: &Booted, cheque: &Cheque, kind: FollowupKind) -> Vec<FollowupTask> {
    FollowupTask::of_cheque(t.db(), cheque.id)
        .await
        .unwrap()
        .into_iter()
        .filter(|task| task.kind == kind)
        .collect()
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

/// A bounced cheque of `amount` fils for rent, returned for `reason` (a code
/// of the bounce sheet).
async fn bounced_for(
    t: &Booted,
    staff: &renox::auth::User,
    number: &str,
    amount: i64,
    reason: &str,
) -> Cheque {
    let rent = t.invoices.add(BLUE_DUNES, &format!("INV-{number}"), amount);
    let cheque = t
        .received_cheque(BLUE_DUNES, number, amount, Some(rent))
        .await;
    let cheque = t.deposit_cheque(staff, &cheque).await;
    t.bounce_cheque(staff, &cheque, reason, "CERT-77", "")
        .await
        .assert_ok();
    t.cheque(cheque.id).await
}

/// AC1: a bounced cheque (insufficient funds, AED 10,000.00 open), referred
/// by user A as a civil execution with no owner named: `legal_case`, owner A,
/// A's task with the evidence pack, the formal notice with the cheque number,
/// the reason, the open amount and the 7-day demand, and the manager told.
#[renox::test]
async fn a_referral_gives_the_owner_a_task_the_party_a_notice_and_tells_the_manager() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    let accountant = t.accountant("Cara").await;
    let manager = t.finance_manager("Mona").await;
    let cheque = bounced_for(&t, &a, "4001", 1_000_000, "insufficient_funds").await;
    assert_eq!(cheque.open_amount, 1_000_000);

    t.refer_cheque(&a, &cheque, "civil_execution", None)
        .await
        .assert_ok();

    let case = t.cheque(cheque.id).await;
    assert_eq!(case.status, ChequeStatus::LegalCase);
    assert_eq!(case.case_type, Some(CaseType::CivilExecution));
    assert_eq!(case.case_owner_id, Some(a.id));
    assert_eq!(case.referred_on, Some(today()));
    assert_eq!(case.open_amount, 1_000_000);

    // The case owner's task, with the evidence pack.
    let tasks = tasks_of(&t, &case, FollowupKind::LegalCase).await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee_id, Some(a.id));
    assert_eq!(tasks[0].status, FollowupStatus::Open);
    assert_eq!(tasks[0].amount, 1_000_000);
    let pack = &tasks[0].evidence.0;
    assert_eq!(pack["cheque"]["number"], "4001");
    assert_eq!(pack["cheque"]["open"], 1_000_000);
    assert_eq!(pack["return_certificate"], "CERT-77");
    assert_eq!(pack["return_reason"], "insufficient funds");
    assert_eq!(pack["invoices"].as_array().unwrap().len(), 1);
    assert_eq!(pack["notices_sent"].as_array().unwrap().len(), 1);
    assert_eq!(pack["returns"].as_array().unwrap().len(), 1);
    // The reminders and the bounce follow-up give way.
    let follow_up = tasks_of(&t, &case, FollowupKind::FollowUpCheque).await;
    assert_eq!(follow_up[0].status, FollowupStatus::Done);
    // The owner is told of the task.
    let told_owner = t.notices.of_type(CHEQUE_TASK);
    let last = told_owner.last().expect("a task notice");
    assert!(user_ids(&last.recipients).contains(&a.id));
    assert!(user_ids(&last.recipients).contains(&accountant.id));

    // The formal notice to the party.
    let notices = t.notices.of_type(CHEQUE_NOTICE);
    assert_eq!(notices.len(), 1);
    assert!(matches!(
        notices[0].recipients[0],
        NotifyRecipient::Party { id } if id == BLUE_DUNES
    ));
    let data = notices[0].data.to_string();
    for part in [
        "4001",
        "insufficient funds",
        "AED 10,000.00",
        "Blue Dunes LLC",
        "Palm Grove Properties LLC",
        "civil execution",
    ] {
        assert!(data.contains(part), "{part} in {data}");
    }
    assert_eq!(notices[0].data["days"], 7);

    // The vertical hears of the referral.
    let heard = t.cheque_referred.all();
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].cheque.0, cheque.id);
    assert_eq!(heard[0].party, BLUE_DUNES);
    assert_eq!(heard[0].open_amount, 1_000_000);
    assert_eq!(heard[0].case_owner, a.id);

    // The manager is told.
    let referred = t.notices.of_type(CHEQUE_REFERRED);
    assert_eq!(referred.len(), 1);
    assert_eq!(user_ids(&referred[0].recipients), vec![manager.id]);
    assert!(referred[0].data.to_string().contains("Amal"));
    assert!(
        referred[0]
            .data
            .to_string()
            .contains(&format!("/cheques/{}", cheque.id))
    );
}

/// AC2: naming user C as the case owner gives the task to C; a user that does
/// not exist is refused.
#[renox::test]
async fn the_case_owner_named_gets_the_task() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    let c = t.user("Carl", &[]).await;
    let cheque = bounced_for(&t, &a, "4002", 1_000_000, "insufficient_funds").await;

    t.refer_cheque(&a, &cheque, "civil_execution", Some(9_999))
        .await
        .assert_status(422);
    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::Bounced);

    t.refer_cheque(&a, &cheque, "civil_execution", Some(c.id))
        .await
        .assert_ok();
    let case = t.cheque(cheque.id).await;
    assert_eq!(case.case_owner_id, Some(c.id));
    let tasks = tasks_of(&t, &case, FollowupKind::LegalCase).await;
    assert_eq!(tasks[0].assignee_id, Some(c.id));
    let told = t.notices.of_type(CHEQUE_TASK);
    assert!(user_ids(&told.last().unwrap().recipients).contains(&c.id));
    // The case owner sees the task on the tasks page (the accountants do too).
    let mine = FollowupTask::open_for_user(t.db(), "accountant", c.id)
        .await
        .unwrap();
    assert!(mine.iter().any(|task| task.kind == FollowupKind::LegalCase));
}

/// AC3: a referral one day after the bounce, before the 3-day reminder, is
/// allowed: reminders are practice, not a guard.
#[renox::test]
async fn a_referral_one_day_after_the_bounce_is_allowed() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    t.travel_to("2030-01-10", 8, 0);
    let cheque = bounced_for(&t, &a, "4003", 1_000_000, "insufficient_funds").await;
    assert_eq!(
        cheque.bounced_on,
        Some(NaiveDate::from_ymd_opt(2030, 1, 10).unwrap())
    );
    t.travel_to("2030-01-11", 8, 0);
    t.refer_cheque(&a, &cheque, "civil_execution", None)
        .await
        .assert_ok();
    let case = t.cheque(cheque.id).await;
    assert_eq!(case.status, ChequeStatus::LegalCase);
    assert_eq!(
        case.referred_on,
        Some(NaiveDate::from_ymd_opt(2030, 1, 11).unwrap())
    );
}

/// AC4: with the default criminal-grounds list, a criminal complaint for
/// "insufficient funds" is refused; for "account closed" it is allowed.
#[renox::test]
async fn a_criminal_complaint_needs_a_return_reason_on_the_list() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    let funds = bounced_for(&t, &a, "4004", 1_000_000, "insufficient_funds").await;
    t.refer_cheque(&a, &funds, "criminal_complaint", None)
        .await
        .assert_status(422);
    let still = t.cheque(funds.id).await;
    assert_eq!(still.status, ChequeStatus::Bounced);
    assert_eq!(still.case_type, None);
    assert!(
        tasks_of(&t, &still, FollowupKind::LegalCase)
            .await
            .is_empty()
    );
    assert!(t.notices.of_type(CHEQUE_NOTICE).is_empty());

    let closed = bounced_for(&t, &a, "4005", 1_000_000, "account_closed").await;
    t.refer_cheque(&a, &closed, "criminal_complaint", None)
        .await
        .assert_ok();
    let case = t.cheque(closed.id).await;
    assert_eq!(case.status, ChequeStatus::LegalCase);
    assert_eq!(case.case_type, Some(CaseType::CriminalComplaint));
    let data = t.notices.of_type(CHEQUE_NOTICE)[0].data.to_string();
    assert!(data.contains("criminal complaint"), "{data}");

    // The other two default grounds.
    for (number, reason) in [("4006", "payment_stopped"), ("4007", "signature_irregular")] {
        let cheque = bounced_for(&t, &a, number, 1_000_000, reason).await;
        t.refer_cheque(&a, &cheque, "criminal_complaint", None)
            .await
            .assert_ok();
    }
    // "Other" is not a ground, and an unknown case type is refused.
    let other = bounced_for(&t, &a, "4008", 1_000_000, "other").await;
    t.refer_cheque(&a, &other, "criminal_complaint", None)
        .await
        .assert_status(422);
    t.refer_cheque(&a, &other, "arbitration", None)
        .await
        .assert_status(422);
    // No case type at all is the default: civil execution.
    t.refer_cheque(&a, &other, "", None).await.assert_ok();
    assert_eq!(
        t.cheque(other.id).await.case_type,
        Some(CaseType::CivilExecution)
    );
}

/// The criminal-grounds list is a setting.
#[renox::test]
async fn the_criminal_grounds_are_a_setting() {
    let t = boot_with(
        WlPayments::new().criminal_grounds(&["Insufficient Funds"]),
        FakeGateway::new(),
    )
    .await;
    let a = t.cheque_staff("Amal").await;
    let funds = bounced_for(&t, &a, "4010", 1_000_000, "insufficient_funds").await;
    t.refer_cheque(&a, &funds, "criminal_complaint", None)
        .await
        .assert_ok();
    let closed = bounced_for(&t, &a, "4011", 1_000_000, "account_closed").await;
    t.refer_cheque(&a, &closed, "criminal_complaint", None)
        .await
        .assert_status(422);
}

/// The minimum open amount for a legal case is a setting; the default (0)
/// needs only something open.
#[renox::test]
async fn the_open_amount_must_be_above_the_minimum() {
    let t = boot_with(
        WlPayments::new().legal_minimum(1_000_000),
        FakeGateway::new(),
    )
    .await;
    let a = t.cheque_staff("Amal").await;
    let small = bounced_for(&t, &a, "4020", 1_000_000, "insufficient_funds").await;
    t.refer_cheque(&a, &small, "civil_execution", None)
        .await
        .assert_status(422);
    let big = bounced_for(&t, &a, "4021", 1_000_001, "insufficient_funds").await;
    t.refer_cheque(&a, &big, "civil_execution", None)
        .await
        .assert_ok();

    let plain = boot().await;
    let a = plain.cheque_staff("Amal").await;
    let tiny = bounced_for(&plain, &a, "4022", 1, "insufficient_funds").await;
    plain
        .refer_cheque(&a, &tiny, "civil_execution", None)
        .await
        .assert_ok();
}

/// AC5: a user without `cheques.legal` cannot refer; only a bounced received
/// cheque is referred.
#[renox::test]
async fn only_cheques_legal_refers_and_only_a_bounced_received_cheque() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    let manager = t
        .user(
            "Mira",
            &[
                wl_payments::permissions::CHEQUES_VIEW,
                wl_payments::permissions::CHEQUES_DEPOSIT,
                wl_payments::permissions::CHEQUES_MANAGE,
            ],
        )
        .await;
    let cheque = bounced_for(&t, &a, "4030", 1_000_000, "insufficient_funds").await;
    t.refer_cheque(&manager, &cheque, "civil_execution", None)
        .await
        .assert_forbidden();
    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::Bounced);

    // Not deposited yet, or already a case.
    let held = t.received_cheque(BLUE_DUNES, "4031", 1_000_000, None).await;
    t.refer_cheque(&a, &held, "civil_execution", None)
        .await
        .assert_status(422);
    t.refer_cheque(&a, &cheque, "civil_execution", None)
        .await
        .assert_ok();
    let case = t.cheque(cheque.id).await;
    t.refer_cheque(&a, &case, "civil_execution", None)
        .await
        .assert_status(422);

    // An issued cheque is replaced, not referred.
    let entity = t.entity("PGP").await;
    let issued = t
        .register_cheque(
            RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "4032", today(), 1_050_000).issued(),
        )
        .await;
    let issued = t.deposit_cheque(&a, &issued).await;
    t.bounce_cheque(&a, &issued, "insufficient_funds", "", "")
        .await
        .assert_ok();
    let issued = t.cheque(issued.id).await;
    t.refer_cheque(&a, &issued, "civil_execution", None)
        .await
        .assert_status(422);
}

/// The formal notice goes to a blocked party too: it is a legal step.
#[renox::test]
async fn the_formal_notice_goes_to_a_blocked_party_too() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    let entity = t.entity("PGP").await;
    let cheque = t
        .register_cheque(RegisterCheque::new(
            DUSK_HOLDINGS,
            entity.id,
            "ENBD",
            "4040",
            today(),
            500_000,
        ))
        .await;
    let cheque = t.deposit_cheque(&a, &cheque).await;
    t.bounce_cheque(&a, &cheque, "account_closed", "CERT-1", "")
        .await
        .assert_ok();
    t.refer_cheque(&a, &cheque, "civil_execution", None)
        .await
        .assert_ok();
    assert_eq!(t.notices.of_type(CHEQUE_NOTICE).len(), 1);
}

/// BL-PAY-11 from `legal_case`: a part payment keeps the cheque a case; a
/// replacement that covers the open amount closes the legal task, emits
/// `ChequeBounceResolved` (replaced) and replaces it.
#[renox::test]
async fn a_legal_case_takes_a_part_payment_and_is_replaced_with_its_task_closed() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    t.accountant("Cara").await;
    let rent = t.invoices.add(BLUE_DUNES, "INV-1", 2_000_000);
    let cheque = t
        .received_cheque(BLUE_DUNES, "4050", 2_000_000, Some(rent))
        .await;
    let cheque = t.deposit_cheque(&a, &cheque).await;
    t.bounce_cheque(&a, &cheque, "account_closed", "CERT-77", "6000")
        .await
        .assert_ok();
    let cheque = t.cheque(cheque.id).await;
    t.refer_cheque(&a, &cheque, "civil_execution", None)
        .await
        .assert_ok();
    let case = t.cheque(cheque.id).await;
    assert_eq!(case.open_amount, 1_400_000);
    assert_eq!(
        tasks_of(&t, &case, FollowupKind::LegalCase).await[0].status,
        FollowupStatus::Open
    );

    // A part payment: still a case, with less open.
    t.settle_cheque(&a, &case, "cash", "4000.00", "")
        .await
        .assert_ok();
    let case = t.cheque(cheque.id).await;
    assert_eq!(case.status, ChequeStatus::LegalCase);
    assert_eq!(
        (case.settled_amount, case.open_amount),
        (400_000, 1_000_000)
    );
    // Not the whole of it.
    t.settle_cheque(&a, &case, "cash", "10000.00", "")
        .await
        .assert_status(422);

    // A new cheque of AED 10,000.00 replaces it.
    let new = t
        .received_cheque(BLUE_DUNES, "4051", 1_000_000, Some(rent))
        .await;
    t.link_replacement(&a, &case, "cheque", new.id)
        .await
        .assert_ok();
    t.replace_cheque(&a, &case).await.assert_ok();
    let replaced = t.cheque(cheque.id).await;
    assert_eq!(replaced.status, ChequeStatus::Replaced);
    assert_eq!(
        tasks_of(&t, &replaced, FollowupKind::LegalCase).await[0].status,
        FollowupStatus::Done
    );
    let resolved = t.cheque_resolved.all();
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].cheque, ChequeId(cheque.id));
    assert_eq!(resolved[0].outcome, ChequeOutcome::Replaced);

    // When the new cheque clears, the original is handed back with a receipt.
    let new = t.deposit_cheque(&a, &new).await;
    t.cheque_action(&a, &new, "clear", &[]).await.assert_ok();
    let hand_back = tasks_of(&t, &replaced, FollowupKind::HandBack).await;
    assert_eq!(hand_back.len(), 1);
    assert!(hand_back[0].evidence.0["receipt"].as_str().is_some());
}
