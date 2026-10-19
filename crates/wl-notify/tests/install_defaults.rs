//! `install` fills the default give-up reaction of unedited, empty rows
//! (#60 AC8).

mod support;

use renox::db::Json;
use renox::prelude::Model;
use wl_kernel::notify::GiveUpReaction;
use wl_notify::NotificationType;

use support::{PAUSE_NOTICE, RECEIPT, REMINDER, boot};

async fn stored(t: &support::Booted, key: &str) -> NotificationType {
    NotificationType::by_key(t.db(), key)
        .await
        .unwrap()
        .unwrap()
}

/// Replaces the stored row of `key` with one that has no reactions, as
/// rows stored before the defaults existed.
async fn blank(t: &support::Booted, key: &str) {
    let mut row = stored(t, key).await;
    row.delete(t.db()).await.unwrap();
    row.id = 0;
    row.give_up = Json(Vec::new());
    row.staff_role = None;
    row.created_at = None;
    row.updated_at = None;
    NotificationType::create(t.db(), row).await.unwrap();
}

#[renox::test]
async fn install_fills_an_unedited_empty_row_and_keeps_the_others() {
    let t = boot().await;
    blank(&t, PAUSE_NOTICE).await;
    blank(&t, RECEIPT).await;
    let mut edited = stored(&t, RECEIPT).await;
    edited.save(t.db()).await.unwrap();
    assert!(!stored(&t, RECEIPT).await.is_unedited());
    let own = stored(&t, REMINDER).await;

    wl_notify::types::install(t.db()).await.unwrap();

    let filled = stored(&t, PAUSE_NOTICE).await;
    assert_eq!(filled.give_up.0, [GiveUpReaction::StaffTask]);
    assert!(filled.is_unedited());
    assert!(stored(&t, RECEIPT).await.give_up.0.is_empty());
    assert_eq!(stored(&t, REMINDER).await.give_up.0, own.give_up.0);

    wl_notify::types::install(t.db()).await.unwrap();
    assert_eq!(stored(&t, PAUSE_NOTICE).await.give_up.0, filled.give_up.0);
    assert!(stored(&t, RECEIPT).await.give_up.0.is_empty());
}
