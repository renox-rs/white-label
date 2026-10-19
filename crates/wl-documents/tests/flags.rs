//! Flagging a record instead of refusing it for a kind in flag mode: the
//! acceptance checks AC1-AC7 of #46.
//!
//! No module guards a transition yet, so the checks call
//! `guarded_transition` the way a guarded module will, with the
//! test-registered `BlockSubjects` provider of `support`.

mod support;

use std::sync::atomic::{AtomicBool, Ordering};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_documents::DocumentFlag;
use wl_kernel::Subject;
use wl_kernel::documents::{self, FlagReason, RecordFlag};

use support::{
    Booted, MONA_ATTENDANCE, RAVI_ATTENDANCE, boot, flagged_work_permit, medical_certificate, ravi,
};

const ACCEPT: &str = "attendance_record.accept";
const LABEL: &str = "Work permit expired 2026-11-30";

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// Runs the guarded accept of `record` on `on_date`; whether the closure ran.
async fn accept(t: &Booted, record: i64, on_date: NaiveDate) -> Result<bool> {
    let ran = AtomicBool::new(false);
    documents::guarded_transition(t.state(), ACCEPT, record, on_date, || async {
        ran.store(true, Ordering::SeqCst);
        Ok(())
    })
    .await?;
    Ok(ran.load(Ordering::SeqCst))
}

async fn flags(t: &Booted, record: i64) -> Vec<RecordFlag> {
    documents::guard(t.state())
        .flags_for(t.db(), "attendance_record", record)
        .await
        .unwrap()
}

/// Ravi's work permit expired on 2026-11-30; his attendance is flagged.
async fn expired_permit(t: &Booted) -> User {
    t.install(&[flagged_work_permit()]).await;
    let rita = t.reviewer().await;
    t.accepted(&rita, ravi(), "work_permit", Some(date(2026, 11, 30)))
        .await;
    rita
}

/// AC1: the transition runs and the record gets one unreviewed flag.
#[renox::test]
async fn an_expired_permit_flags_the_record_and_the_transition_runs() {
    let t = boot().await;
    expired_permit(&t).await;

    assert!(
        accept(&t, RAVI_ATTENDANCE, date(2026, 12, 2))
            .await
            .unwrap()
    );
    let flags = flags(&t, RAVI_ATTENDANCE).await;
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].label, LABEL);
    assert_eq!(flags[0].reason, FlagReason::Expired);
    assert_eq!(flags[0].subject, ravi());
    assert_eq!(flags[0].on_date, date(2026, 12, 2));
    assert!(!flags[0].is_reviewed());
}

/// AC2: judged on 2026-11-29 the permit was valid: no flag.
#[renox::test]
async fn a_late_entry_is_judged_on_its_own_day() {
    let t = boot().await;
    expired_permit(&t).await;

    assert!(
        accept(&t, RAVI_ATTENDANCE, date(2026, 11, 29))
            .await
            .unwrap()
    );
    assert!(flags(&t, RAVI_ATTENDANCE).await.is_empty());
}

/// AC3: review needs a note, keeps the label, and happens once.
#[renox::test]
async fn a_flag_is_reviewed_with_a_note_once_and_keeps_its_label() {
    let t = boot().await;
    let rita = expired_permit(&t).await;
    accept(&t, RAVI_ATTENDANCE, date(2026, 12, 2))
        .await
        .unwrap();
    let id = flags(&t, RAVI_ATTENDANCE).await[0].id;
    let guard = documents::guard(t.state());

    assert!(guard.review_flag(t.db(), id, rita.id, "  ").await.is_err());
    guard
        .review_flag(t.db(), id, rita.id, "Permit renewal in progress")
        .await
        .unwrap();
    assert!(
        guard
            .review_flag(t.db(), id, rita.id, "again")
            .await
            .is_err()
    );

    let flags = flags(&t, RAVI_ATTENDANCE).await;
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].label, LABEL);
    assert!(flags[0].is_reviewed());
    assert_eq!(flags[0].reviewer_name.as_deref(), Some("Rita"));
    assert_eq!(
        flags[0].review_note.as_deref(),
        Some("Permit renewal in progress")
    );

    // The flags page shows it too.
    t.app.acting_as(&rita);
    t.app
        .get("/documents/flags")
        .await
        .assert_ok()
        .assert_see(LABEL)
        .assert_see("Rita")
        .assert_see("Permit renewal in progress");
}

/// AC4: a renewed permit leaves the flag as it was.
#[renox::test]
async fn a_renewed_permit_leaves_the_flag() {
    let t = boot().await;
    let rita = expired_permit(&t).await;
    accept(&t, RAVI_ATTENDANCE, date(2026, 12, 2))
        .await
        .unwrap();
    let before = flags(&t, RAVI_ATTENDANCE).await;

    t.accepted(&rita, ravi(), "work_permit", Some(date(2027, 11, 30)))
        .await;

    assert_eq!(flags(&t, RAVI_ATTENDANCE).await, before);
    assert_eq!(before[0].label, LABEL);
}

/// AC5: a failing block-mode kind refuses; the closure does not run and
/// nothing is flagged.
#[renox::test]
async fn a_failing_block_kind_wins_over_the_flag() {
    let t = boot().await;
    t.install(&[flagged_work_permit(), medical_certificate()])
        .await;
    let rita = t.reviewer().await;
    t.accepted(&rita, ravi(), "work_permit", Some(date(2026, 11, 30)))
        .await;
    t.accepted(
        &rita,
        ravi(),
        "medical_certificate",
        Some(date(2026, 11, 1)),
    )
    .await;

    let refused = accept(&t, RAVI_ATTENDANCE, date(2026, 12, 2))
        .await
        .expect_err("the transition is refused");
    match refused {
        Error::Status(code, message) => {
            assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
            assert!(message.contains("Medical certificate"), "{message}");
            assert!(!message.contains("Work permit"), "{message}");
        }
        other => panic!("unexpected error {other:?}"),
    }
    assert!(flags(&t, RAVI_ATTENDANCE).await.is_empty());
    assert_eq!(DocumentFlag::query().count(t.db()).await.unwrap(), 0);
}

/// AC6: a failing closure writes no flag; repeating the accept adds none.
#[renox::test]
async fn a_failed_transition_writes_no_flag_and_a_repeat_adds_none() {
    let t = boot().await;
    expired_permit(&t).await;

    let failed: Result<()> = documents::guarded_transition(
        t.state(),
        ACCEPT,
        RAVI_ATTENDANCE,
        date(2026, 12, 2),
        || async { Err(Error::Status(StatusCode::CONFLICT, "boom".into())) },
    )
    .await;
    assert!(failed.is_err());
    assert!(flags(&t, RAVI_ATTENDANCE).await.is_empty());

    accept(&t, RAVI_ATTENDANCE, date(2026, 12, 2))
        .await
        .unwrap();
    accept(&t, RAVI_ATTENDANCE, date(2026, 12, 2))
        .await
        .unwrap();
    assert_eq!(flags(&t, RAVI_ATTENDANCE).await.len(), 1);
}

/// AC7: list badges count the unreviewed flags per record. (Without
/// wl-documents, `guarded_transition` and `flags_for` are covered by the
/// unit tests of `wl_kernel::documents`.)
#[renox::test]
async fn open_flag_counts_count_the_unreviewed_flags() {
    let t = boot().await;
    let rita = expired_permit(&t).await;
    t.accepted(
        &rita,
        Subject::new("hr.employee", MONA_ATTENDANCE),
        "work_permit",
        Some(date(2027, 12, 31)),
    )
    .await;
    accept(&t, RAVI_ATTENDANCE, date(2026, 12, 2))
        .await
        .unwrap();
    accept(&t, MONA_ATTENDANCE, date(2026, 12, 2))
        .await
        .unwrap();

    let ids = [RAVI_ATTENDANCE, MONA_ATTENDANCE, 99];
    let counts = documents::guard(t.state())
        .open_flag_counts(t.db(), "attendance_record", &ids)
        .await
        .unwrap();
    let count = |id: i64| counts.get(&id).copied().unwrap_or(0);
    assert_eq!(count(RAVI_ATTENDANCE), 1);
    assert_eq!(count(MONA_ATTENDANCE), 0);
    assert_eq!(count(99), 0);
}
