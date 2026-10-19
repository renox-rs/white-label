//! What `notice_status` and `undelivered_notices` answer from the stored
//! messages and statutory-notice records (#59 A2, A11).

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::{DateTime, Model};
use renox::state_machine::StateMachine;
use wl_kernel::Subject;
use wl_kernel::notify::{self, NoticeStatus, ProofMethod};
use wl_notify::message::MessageStatus;
use wl_notify::{Message, NotificationType, StatutoryNotice};

use support::{NOTICE, boot};

const LEASE: &str = "lease.changed_terms_notice";
const OTHER: &str = "lease.renewal_notice";

fn subject() -> Subject {
    Subject::new("property.lease", 7)
}

/// A message of `type_key` about the lease, first of its chain unless
/// `chain` says otherwise.
async fn message(
    t: &support::Booted,
    type_key: &str,
    status: MessageStatus,
    chain: Option<i64>,
    delivered_at: Option<DateTime>,
) -> Message {
    let mut m = Message {
        type_key: type_key.into(),
        status,
        ..Default::default()
    };
    m.set_subject(&subject());
    m.chain_id = chain;
    m.delivered_at = delivered_at;
    let mut tx = t.db().begin().await.unwrap();
    m.insert_in_state(&mut tx, status, "fixture").await.unwrap();
    m.chain_id = Some(chain.unwrap_or(m.id));
    m.save(&mut tx).await.unwrap();
    tx.commit().await.unwrap();
    m
}

async fn status(t: &support::Booted, type_key: &str) -> NoticeStatus {
    let state = t.app.state();
    notify::notifier(state)
        .notice_status(state, &subject(), type_key)
        .await
        .unwrap()
}

async fn undelivered(t: &support::Booted) -> Vec<i64> {
    let state = t.app.state();
    notify::notifier(state)
        .undelivered_notices(state, &subject())
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.notice)
        .collect()
}

async fn record(t: &support::Booted, m: &Message) -> StatutoryNotice {
    StatutoryNotice::create(t.db(), StatutoryNotice::for_message(m, renox::db::now()))
        .await
        .unwrap()
}

#[renox::test]
async fn a_notice_in_flight_or_only_sent_is_not_delivered() {
    let t = boot().await;
    assert_eq!(status(&t, LEASE).await, NoticeStatus::NotDelivered);
    message(&t, LEASE, MessageStatus::Queued, None, None).await;
    assert_eq!(status(&t, LEASE).await, NoticeStatus::NotDelivered);
    message(&t, LEASE, MessageStatus::Sent, None, None).await;
    assert_eq!(status(&t, LEASE).await, NoticeStatus::NotDelivered);
}

#[renox::test]
async fn a_delivered_notice_answers_with_the_provider_time() {
    let t = boot().await;
    let at = renox::db::now();
    message(&t, LEASE, MessageStatus::Delivered, None, Some(at)).await;
    assert_eq!(status(&t, LEASE).await, NoticeStatus::Delivered { at });
    message(&t, OTHER, MessageStatus::Read, None, Some(at)).await;
    assert_eq!(status(&t, OTHER).await, NoticeStatus::Delivered { at });
}

#[renox::test]
async fn a_recorded_proof_answers_with_its_delivery_date() {
    let t = boot().await;
    let m = message(&t, LEASE, MessageStatus::Abandoned, None, None).await;
    let mut notice = record(&t, &m).await;
    assert_eq!(status(&t, LEASE).await, NoticeStatus::NotDelivered);
    assert_eq!(undelivered(&t).await, [notice.id]);

    let on = NaiveDate::from_ymd_opt(2026, 9, 3).unwrap();
    notice.proof_method = Some(ProofMethod::HandDelivery);
    notice.delivered_on = Some(on);
    notice.save(t.db()).await.unwrap();
    assert_eq!(
        status(&t, LEASE).await,
        NoticeStatus::ProofRecorded { delivered_on: on }
    );
    assert!(undelivered(&t).await.is_empty());
}

#[renox::test]
async fn each_type_keeps_its_own_status() {
    let t = boot().await;
    let a = message(&t, LEASE, MessageStatus::Abandoned, None, None).await;
    let b = message(&t, OTHER, MessageStatus::Abandoned, None, None).await;
    let mut first = record(&t, &a).await;
    let second = record(&t, &b).await;
    first.delivered_on = NaiveDate::from_ymd_opt(2026, 9, 3);
    first.save(t.db()).await.unwrap();
    assert!(matches!(
        status(&t, LEASE).await,
        NoticeStatus::ProofRecorded { .. }
    ));
    assert_eq!(status(&t, OTHER).await, NoticeStatus::NotDelivered);
    assert_eq!(undelivered(&t).await, [second.id]);
}

#[renox::test]
async fn a_later_chain_that_is_delivered_wins() {
    let t = boot().await;
    let old = message(&t, LEASE, MessageStatus::Abandoned, None, None).await;
    let notice = record(&t, &old).await;
    message(&t, LEASE, MessageStatus::Queued, None, None).await;
    assert_eq!(status(&t, LEASE).await, NoticeStatus::NotDelivered);
    assert_eq!(undelivered(&t).await, [notice.id]);
    let at = Some(renox::db::now());
    message(&t, LEASE, MessageStatus::Delivered, None, at).await;
    assert!(matches!(
        status(&t, LEASE).await,
        NoticeStatus::Delivered { .. }
    ));
    assert!(undelivered(&t).await.is_empty());
}

#[renox::test]
async fn the_last_message_of_the_chain_decides() {
    let t = boot().await;
    let first = message(&t, LEASE, MessageStatus::Failed, None, None).await;
    assert_eq!(status(&t, LEASE).await, NoticeStatus::NotDelivered);
    let at = Some(renox::db::now());
    message(&t, LEASE, MessageStatus::Delivered, Some(first.id), at).await;
    assert!(matches!(
        status(&t, LEASE).await,
        NoticeStatus::Delivered { .. }
    ));
}

#[renox::test]
async fn install_keeps_the_proof_methods_a_client_stored() {
    let t = boot().await;
    let mut row = NotificationType::by_key(t.db(), NOTICE)
        .await
        .unwrap()
        .expect("the type is installed");
    row.proof_methods = renox::db::Json(vec![ProofMethod::Courier]);
    row.save(t.db()).await.unwrap();
    wl_notify::types::install(t.db()).await.unwrap();
    let again = NotificationType::by_key(t.db(), NOTICE)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(again.to_def().proof_methods, [ProofMethod::Courier]);
}
