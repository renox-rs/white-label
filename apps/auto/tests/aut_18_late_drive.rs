//! BL-AUT-18 (#414), *Integrations and late entry*: a test drive written up
//! after the fact keeps its business time. Up to 30 minutes behind it is on
//! time; later it is flagged and needs a reason; entered 30 hours late (the
//! limit is 24) it creates an approval request for a holder of
//! `late_entries.approve` and nothing is written until it is approved.

mod support;

use auto::late_times;
use auto::test_drive::TestDrive;
use auto::test_drives::NewDrive;
use auto::JobCardStatus;
use renox::chrono::Duration;
use renox::db::now;
use support::boot;

#[renox::test]
async fn aut_18_a_test_drive_entered_30_hours_late_waits_for_approval() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    let advisor = t.advisor().await;
    let left = now() - Duration::hours(30);

    // A late time needs a reason first.
    let refused = late_times::enter(
        t.state(),
        &advisor,
        card.id,
        NewDrive::out(52_210).left_at(left),
        None,
    )
    .await
    .unwrap_err();
    assert!(refused.to_string().len() > 3, "{refused}");

    let entered = late_times::enter(
        t.state(),
        &advisor,
        card.id,
        NewDrive::out(52_210).left_at(left),
        Some("The system was down on Saturday"),
    )
    .await
    .unwrap();
    assert!(entered.is_pending(), "{entered:?}");
    // Nothing is written until the approval.
    assert!(
        TestDrive::of_job_card(t.db(), card.id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[renox::test]
async fn aut_18_a_test_drive_entered_two_hours_late_is_recorded_with_the_business_time() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    let advisor = t.advisor().await;
    let left = now() - Duration::hours(2);

    let entered = late_times::enter(
        t.state(),
        &advisor,
        card.id,
        NewDrive::out(52_210).left_at(left),
        Some("Paper sheet"),
    )
    .await
    .unwrap();
    assert!(!entered.is_pending(), "{entered:?}");
    let drives = TestDrive::of_job_card(t.db(), card.id).await.unwrap();
    assert_eq!(drives.len(), 1);
    assert_eq!(drives[0].out_at.timestamp(), left.timestamp());
    assert_eq!(drives[0].driver_user_id, advisor.id);
}

#[renox::test]
async fn aut_18_a_drive_in_the_future_or_without_a_time_is_refused() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    let advisor = t.advisor().await;
    for new in [
        NewDrive::out(52_210),
        NewDrive::out(52_210).left_at(now() + Duration::hours(2)),
    ] {
        let error = late_times::enter(t.state(), &advisor, card.id, new, Some("x"))
            .await
            .unwrap_err();
        let text = error.to_string();
        assert!(text.contains("future") || text.contains("Give the time"), "{text}");
    }
}
