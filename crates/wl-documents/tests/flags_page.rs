//! The back-office flags page (#46, part 4): `/documents/flags` lists the
//! flags, unreviewed first, and "Mark reviewed" needs a note.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_documents::{DocumentFlag, permissions as perm};
use wl_kernel::documents::FlagReason;

use support::boot;

/// Stores a flag labelled `label` on record `record_id` of `record_type`.
async fn flag(
    db: &Db,
    record_type: &str,
    record_id: i64,
    label: &str,
    reviewed: bool,
) -> DocumentFlag {
    let mut flag = DocumentFlag {
        record_type: record_type.into(),
        record_id,
        guarded: "attendance_record.accept".into(),
        kind: "work_permit".into(),
        subject_type: "hr.employee".into(),
        subject_id: 7,
        reason: FlagReason::Expired,
        on_date: NaiveDate::from_ymd_opt(2026, 12, 2).unwrap(),
        label: label.into(),
        ..DocumentFlag::default()
    };
    flag.save(db).await.unwrap();
    if reviewed {
        flag.reviewed_at = Some(renox::db::now());
        flag.reviewed_by = Some(1);
        flag.review_note = Some("Permit renewal in progress".into());
        flag.save(db).await.unwrap();
    }
    flag
}

#[renox::test]
async fn the_flags_page_lists_unreviewed_first_and_marks_reviewed_with_a_note() {
    let t = boot().await;
    let db = t.db();
    // The older flag is reviewed, the newer one is not: the open one is first.
    let done = flag(db, "ticket", 1, "Insurance expired 2026-10-01", true).await;
    let open = flag(
        db,
        "attendance_record",
        2,
        "Work permit expired 2026-11-30",
        false,
    )
    .await;

    let viewer = t.user("Vera", &[perm::VIEW]).await;
    t.app.acting_as(&viewer);
    let response = t.app.get("/documents/flags").await;
    let page = response
        .assert_ok()
        .assert_see("Work permit expired 2026-11-30")
        .assert_see("Insurance expired 2026-10-01")
        .assert_see("Permit renewal in progress");
    let body = page.text();
    assert!(
        body.find("Work permit expired").unwrap() < body.find("Insurance expired").unwrap(),
        "unreviewed flags come first"
    );
    // Filters by state and record type.
    t.app
        .get("/documents/flags?in.state=open")
        .await
        .assert_ok()
        .assert_see("Work permit expired 2026-11-30")
        .assert_dont_see("Insurance expired 2026-10-01");
    t.app
        .get("/documents/flags?in.record_type=ticket")
        .await
        .assert_ok()
        .assert_dont_see("Work permit expired 2026-11-30");
    // Without documents.review the viewer cannot review.
    t.app
        .htmx()
        .post(
            &format!("/documents/flags/{}/review", open.id),
            &[("note", "ok")],
        )
        .await
        .assert_forbidden();

    let reviewer = t.reviewer().await;
    t.app.acting_as(&reviewer);
    t.app
        .htmx()
        .post(
            &format!("/documents/flags/{}/review", open.id),
            &[("note", "  ")],
        )
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(
            &format!("/documents/flags/{}/review", open.id),
            &[("note", "Permit renewal in progress")],
        )
        .await
        .assert_ok();
    let stored = DocumentFlag::find(db, open.id).await.unwrap().unwrap();
    assert_eq!(stored.reviewed_by, Some(reviewer.id));
    assert_eq!(
        stored.review_note.as_deref(),
        Some("Permit renewal in progress")
    );
    // A second review is refused.
    t.app
        .htmx()
        .post(
            &format!("/documents/flags/{}/review", done.id),
            &[("note", "again")],
        )
        .await
        .assert_status(422);
}
