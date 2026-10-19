//! The broadcast record and its machine (#61 part 2): the transitions, the
//! approval above a size and the one message per party and broadcast.

mod support;

use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::approvals::{self, ApprovalPolicy, PolicyDefault};
use wl_kernel::notify::NotifyChannel;
use wl_notify::broadcast_machine::{
    ACTION, CANCEL_SYSTEM, COMPLETE, CompleteInput, EXPIRE, REJECT, REQUEST, REQUEST_OVER_LIMIT,
    RequestInput, START,
};
use wl_notify::permissions::MARKETING_SEND;
use wl_notify::{Broadcast, BroadcastStatus, Message};

use support::{Booted, boot};

/// A draft broadcast of `estimate` recipients asked for by user `by`.
async fn draft(t: &Booted, estimate: i64, by: i64) -> Broadcast {
    Broadcast::create(
        t.db(),
        Broadcast {
            type_key: "campaign.offer".into(),
            requester_type: "sales.campaign".into(),
            requester_id: 7,
            requested_by: by,
            channel: NotifyChannel::Sms,
            segment_id: 3,
            send_at: renox::db::now(),
            estimate,
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

/// Runs `name` on broadcast `id` as the system.
async fn run(
    t: &Booted,
    id: i64,
    name: &str,
    reason: Option<&str>,
    input: renox::serde_json::Value,
) -> Result<Broadcast, TransitionError> {
    let broadcast = Broadcast::find_or_404(t.db(), id).await?;
    let mut run = broadcast.transition(name).by_system().input(&input);
    if let Some(reason) = reason {
        run = run.reason(reason);
    }
    let _outcome = run.run(t.app.state()).await?;
    Ok(Broadcast::find_or_404(t.db(), id).await?)
}

fn limit() -> renox::serde_json::Value {
    json!(RequestInput { limit: 200 })
}

#[renox::test]
async fn a_request_within_the_limit_is_scheduled_without_approval() {
    let t = boot().await;
    let b = draft(&t, 180, 1).await;
    let over = run(&t, b.id, REQUEST_OVER_LIMIT, None, limit()).await;
    assert!(matches!(over, Err(TransitionError::Guard(_))));
    let b = run(&t, b.id, REQUEST, None, limit()).await.unwrap();
    assert_eq!(b.status, BroadcastStatus::Scheduled);
    assert_eq!(b.approval_request_id, None);
    assert_eq!(b.approved_by, None);
}

#[renox::test]
async fn above_the_limit_without_a_policy_nothing_is_requested() {
    let t = boot().await;
    // Another test registers the policy for the whole process; this app has
    // none stored.
    renox::db::sql(format!("DELETE FROM {}", ApprovalPolicy::TABLE))
        .execute(t.db())
        .await
        .unwrap();
    let b = draft(&t, 350, 1).await;
    let within = run(&t, b.id, REQUEST, None, limit()).await;
    assert!(matches!(within, Err(TransitionError::Guard(_))));
    let none = run(&t, b.id, REQUEST_OVER_LIMIT, None, limit()).await;
    assert!(none.is_err());
    let b = Broadcast::find_or_404(t.db(), b.id).await.unwrap();
    assert_eq!(b.status, BroadcastStatus::Draft, "rolled back");
}

#[renox::test]
async fn above_the_limit_a_second_holder_approves_and_it_is_scheduled() {
    let t = boot().await;
    PolicyDefault::new(ACTION)
        .step_permission(MARKETING_SEND)
        .register();
    approvals::install(t.db()).await.unwrap();
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let b_user = t.user("Bilal", &[MARKETING_SEND]).await;
    let c = t.user("Chen", &["notifications.preferences.manage"]).await;

    let b = draft(&t, 350, a.id).await;
    let b = run(&t, b.id, REQUEST_OVER_LIMIT, None, limit())
        .await
        .unwrap();
    assert_eq!(b.status, BroadcastStatus::PendingApproval);
    let request = b.approval_request_id.expect("an approval request");

    // The requester and a holder of another permission may not approve.
    assert!(
        approvals::approve(t.app.state(), request, &a, "")
            .await
            .is_err()
    );
    assert!(
        approvals::approve(t.app.state(), request, &c, "")
            .await
            .is_err()
    );
    approvals::approve(t.app.state(), request, &b_user, "")
        .await
        .unwrap();
    let b = Broadcast::find_or_404(t.db(), b.id).await.unwrap();
    assert_eq!(b.status, BroadcastStatus::Scheduled);
    assert_eq!(b.approved_by, Some(b_user.id));
}

#[renox::test]
async fn reject_and_expire_cancel_with_their_reasons() {
    let t = boot().await;
    PolicyDefault::new(ACTION)
        .step_permission(MARKETING_SEND)
        .register();
    approvals::install(t.db()).await.unwrap();
    for (name, reason, stored) in [
        (REJECT, Some("wrong audience"), "wrong audience"),
        (REJECT, None, "rejected"),
        (EXPIRE, None, "approval_expired"),
    ] {
        let b = draft(&t, 350, 1).await;
        run(&t, b.id, REQUEST_OVER_LIMIT, None, limit())
            .await
            .unwrap();
        let b = run(&t, b.id, name, reason, json!({})).await.unwrap();
        assert_eq!(b.status, BroadcastStatus::Cancelled);
        assert_eq!(b.cancel_reason.as_deref(), Some(stored));
    }
}

#[renox::test]
async fn start_and_complete_store_the_counts() {
    let t = boot().await;
    let b = draft(&t, 100, 1).await;
    run(&t, b.id, REQUEST, None, limit()).await.unwrap();
    let b = run(&t, b.id, START, None, json!({})).await.unwrap();
    assert_eq!(b.status, BroadcastStatus::Sending);
    assert!(b.started_at.is_some());

    let mut input = CompleteInput {
        recipients_resolved: 100,
        ..Default::default()
    };
    input.excluded.add("no_consent");
    input.results.sent = 100;
    let b = run(&t, b.id, COMPLETE, None, json!(input)).await.unwrap();
    assert_eq!(b.status, BroadcastStatus::Completed);
    assert_eq!(b.recipients_resolved, Some(100));
    assert_eq!(b.excluded.0.of("no_consent"), 1);
    assert_eq!(b.results.0.sent, 100);
    assert!(b.completed_at.is_some());
    // A completed broadcast cannot be cancelled.
    assert!(run(&t, b.id, CANCEL_SYSTEM, None, json!({})).await.is_err());
}

#[renox::test]
async fn a_scheduled_broadcast_is_cancelled_by_the_system_with_a_reason() {
    let t = boot().await;
    let b = draft(&t, 10, 1).await;
    run(&t, b.id, REQUEST, None, limit()).await.unwrap();
    let b = run(
        &t,
        b.id,
        CANCEL_SYSTEM,
        Some("requester_guard_failed"),
        json!({}),
    )
    .await
    .unwrap();
    assert_eq!(b.status, BroadcastStatus::Cancelled);
    assert_eq!(b.cancel_reason.as_deref(), Some("requester_guard_failed"));
}

#[renox::test]
async fn a_party_gets_one_message_per_broadcast() {
    let t = boot().await;
    let b = draft(&t, 10, 1).await;
    let message = |party: i64, broadcast: Option<i64>| Message {
        type_key: "campaign.offer".into(),
        subject_type: "notify.broadcast".into(),
        subject_id: b.id,
        party_id: Some(party),
        broadcast_id: broadcast,
        rendered_body: "Offer".into(),
        ..Default::default()
    };
    Message::create(t.db(), message(5, Some(b.id)))
        .await
        .unwrap();
    // The same party again in the same broadcast: refused.
    assert!(
        Message::create(t.db(), message(5, Some(b.id)))
            .await
            .is_err()
    );
    // Another party, and messages of no broadcast, are fine.
    Message::create(t.db(), message(6, Some(b.id)))
        .await
        .unwrap();
    Message::create(t.db(), message(5, None)).await.unwrap();
    Message::create(t.db(), message(5, None)).await.unwrap();
}
