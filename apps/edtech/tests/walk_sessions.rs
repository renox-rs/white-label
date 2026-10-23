//! Found by walking the session screens: a session that has not been taught
//! does not move to a time that has passed.

mod support;

use edtech::sessions;
use renox::prelude::*;
use support::{boot, day, next_weekday};

#[renox::test]
async fn a_scheduled_session_cannot_be_moved_to_the_past() {
    let t = boot().await;
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let batch = t.open_batch(&tutor, &room, next_weekday(day(7), 1)).await;
    let planned = t.sessions_of(&batch).await;
    let amal = t.coordinator().await;

    let error = sessions::reschedule(t.state(), &amal, planned[0].id, day(-3), "17:00")
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("already passed"),
        "the refusal says why: {error}"
    );
    let same = edtech::Session::find_or_404(t.db(), planned[0].id)
        .await
        .unwrap();
    assert_eq!(same.starts_at, planned[0].starts_at, "the session stays");
}
