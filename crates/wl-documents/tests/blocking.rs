//! Blocking kinds and the separation of duties (`DOC.ACL.03`, #51 part 1):
//! no `record`, and the uploader cannot accept their own upload.

mod support;

use renox::prelude::*;

use wl_documents::permissions as perm;
use wl_documents::{Document, DocumentKind, DocumentStatus, KindSeed, ReminderLog};

use support::{boot, driving_licence, residence_visa};

/// A kind with an expiry date that has no `blocks`: the uploader may accept it.
fn site_pass() -> KindSeed {
    KindSeed::new("site_pass", "Site pass")
        .subject_type("hr.employee")
        .expires()
}

fn form<'a>(kind: &'a str, expiry: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        ("subject_id", "7"),
        ("kind", kind),
        ("file_name", "scan.pdf"),
        ("size", "100"),
        ("expires_on", expiry),
    ]
}

#[renox::test]
async fn is_blocking_means_at_least_one_blocks_entry() {
    let t = boot().await;
    t.install(&[driving_licence(), site_pass()]).await;
    let licence = t.kind("driving_licence").await;
    assert!(licence.is_blocking(t.db()).await.unwrap());
    let pass = t.kind("site_pass").await;
    assert!(!pass.is_blocking(t.db()).await.unwrap());
}

/// AC5: `record` is refused for a blocking kind, even if the stored
/// `needs_review` were false.
#[renox::test]
async fn record_is_refused_for_a_blocking_kind_even_without_needs_review() {
    let t = boot().await;
    t.install(&[driving_licence()]).await;
    let clerk = t.user("Clara", &[perm::UPLOAD]).await;
    let kind = t.kind("driving_licence").await;
    assert!(kind.needs_review);
    DocumentKind::query()
        .where_(DocumentKind::ID.eq(kind.id))
        .update(t.db(), &[("needs_review", &false)])
        .await
        .unwrap();
    assert!(!t.kind("driving_licence").await.needs_review);

    t.app.acting_as(&clerk);
    t.app
        .htmx()
        .post(
            "/test/documents/record",
            &form("driving_licence", "2027-11-30"),
        )
        .await
        .assert_status(422);
    assert!(Document::query().get(t.db()).await.unwrap().is_empty());
}

/// AC5 and AC7: the uploader cannot accept their own blocking upload,
/// someone else can; a non-blocking kind still accepts its uploader.
#[renox::test]
async fn the_uploader_cannot_accept_their_own_blocking_upload() {
    let t = boot().await;
    t.install(&[residence_visa(), site_pass()]).await;
    let a = t.user("Aya", &[perm::UPLOAD, perm::REVIEW]).await;
    let b = t.reviewer().await;

    t.app.acting_as(&a);
    let response = t
        .app
        .htmx()
        .post(
            "/test/documents/attach",
            &form("residence_visa", "2027-11-30"),
        )
        .await;
    response.assert_ok();
    let id: i64 = response.text().parse().unwrap();
    let visa = Document::find(t.db(), id).await.unwrap().unwrap();
    assert_eq!(visa.status, DocumentStatus::PendingReview);
    assert_eq!(visa.uploaded_by, Some(a.id));

    let accept = format!("/test/documents/{id}/accept");
    t.app.htmx().post(&accept, &[]).await.assert_status(422);
    assert_eq!(t.reload(&visa).await.status, DocumentStatus::PendingReview);

    t.app.acting_as(&b);
    t.app.htmx().post(&accept, &[]).await.assert_ok();
    assert_eq!(t.reload(&visa).await.status, DocumentStatus::Accepted);

    t.app.acting_as(&a);
    let response = t
        .app
        .htmx()
        .post("/test/documents/attach", &form("site_pass", "2027-11-30"))
        .await;
    response.assert_ok();
    let own: i64 = response.text().parse().unwrap();
    t.app
        .htmx()
        .post(&format!("/test/documents/{own}/accept"), &[])
        .await
        .assert_ok();
    let own = Document::find(t.db(), own).await.unwrap().unwrap();
    assert_eq!(own.status, DocumentStatus::Accepted);
}

/// Sub-task 2/5: a correction is a new pending document, one at a time,
/// accepted by someone else; both steps are audited.
#[renox::test]
async fn a_correction_replaces_the_document_once_someone_else_accepts_it() {
    let t = boot().await;
    t.install(&[residence_visa()]).await;
    let a = t.user("Aya", &[perm::UPLOAD, perm::REVIEW]).await;
    let b = t.reviewer().await;
    let viewer = t.user("Vera", &[perm::VIEW]).await;

    t.app.acting_as(&a);
    let response = t
        .app
        .htmx()
        .post(
            "/test/documents/attach",
            &form("residence_visa", "2026-11-30"),
        )
        .await;
    let first: i64 = response.text().parse().unwrap();
    let old = Document::find(t.db(), first).await.unwrap().unwrap();
    let old = t.accept(&b, &old).await;
    assert_eq!(old.status, DocumentStatus::Accepted);

    // Without `documents.upload` the correction is forbidden.
    t.app.acting_as(&viewer);
    let correct = format!("/test/documents/{first}/correct");
    t.app
        .htmx()
        .post(&correct, &form("residence_visa", "2027-11-30"))
        .await
        .assert_status(403);

    t.app.acting_as(&a);
    let response = t
        .app
        .htmx()
        .post(&correct, &form("residence_visa", "2027-11-30"))
        .await;
    response.assert_ok();
    let second: i64 = response.text().parse().unwrap();
    let new = Document::find(t.db(), second).await.unwrap().unwrap();
    assert_eq!(new.status, DocumentStatus::PendingReview);
    assert_eq!(new.uploaded_by, Some(a.id));
    assert_eq!(new.kind_id, old.kind_id);
    assert_eq!(new.subject(), old.subject());
    assert_eq!(new.current_version, 1);
    assert_eq!(t.reload(&old).await.status, DocumentStatus::Accepted);

    // One open correction at a time.
    t.app
        .htmx()
        .post(&correct, &form("residence_visa", "2028-11-30"))
        .await
        .assert_status(422);

    // The uploader cannot accept; someone else can.
    let accept = format!("/test/documents/{second}/accept");
    t.app.htmx().post(&accept, &[]).await.assert_status(422);
    t.app.acting_as(&b);
    t.app.htmx().post(&accept, &[]).await.assert_ok();
    assert_eq!(t.reload(&new).await.status, DocumentStatus::Accepted);
    assert_eq!(t.reload(&old).await.status, DocumentStatus::Superseded);

    let entries = renox::audit::for_subject(t.db(), Document::TABLE, second, 10)
        .await
        .unwrap();
    let submitted = entries
        .iter()
        .find(|e| e.action == "document.correction_submitted")
        .expect("the submission is audited");
    assert_eq!(submitted.user_id, Some(a.id));
    assert_eq!(submitted.data["old_document"], first);
    assert_eq!(submitted.data["new_expiry_date"], "2027-11-30");
    let corrected = entries
        .iter()
        .find(|e| e.action == "document.corrected")
        .expect("the correction is audited");
    assert_eq!(corrected.user_id, Some(b.id));
    assert_eq!(corrected.data["old_document"], first);
    assert_eq!(corrected.data["new_document"], second);
    assert_eq!(corrected.data["old_expiry_date"], "2026-11-30");
    assert_eq!(corrected.data["new_expiry_date"], "2027-11-30");
    assert_eq!(corrected.data["uploaded_by"], a.id);
    assert_eq!(corrected.data["accepted_by"], b.id);

    // The old document is no longer accepted, so it cannot be corrected.
    t.app.acting_as(&a);
    t.app
        .htmx()
        .post(&correct, &form("residence_visa", "2029-11-30"))
        .await
        .assert_status(422);
}

/// Sub-task 3/5: number and expiry date of a blocking kind cannot be edited
/// in place, by anyone; title and authority can, and it is audited.
#[renox::test]
async fn a_blocking_kind_refuses_in_place_edits_of_number_and_expiry() {
    let t = boot().await;
    t.install(&[residence_visa()]).await;
    let a = t.user("Aya", &[perm::UPLOAD]).await;
    let b = t.reviewer().await;
    let manager = t.user("Omar", &[perm::MANAGE]).await;
    let viewer = t.user("Vera", &[perm::VIEW]).await;

    t.app.acting_as(&a);
    let response = t
        .app
        .htmx()
        .post(
            "/test/documents/attach",
            &form("residence_visa", "2026-11-30"),
        )
        .await;
    let id: i64 = response.text().parse().unwrap();
    let visa = Document::find(t.db(), id).await.unwrap().unwrap();
    let visa = t.accept(&b, &visa).await;
    let update = format!("/test/documents/{id}/update");

    t.app.acting_as(&viewer);
    t.app
        .htmx()
        .post(&update, &[("title", "X")])
        .await
        .assert_status(403);

    t.app.acting_as(&manager);
    t.app
        .htmx()
        .post(&update, &[("expires_on", "2030-01-01")])
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(&update, &[("number", "N-1")])
        .await
        .assert_status(422);
    let same = t.reload(&visa).await;
    assert_eq!(same.expiry_date, visa.expiry_date);
    assert_eq!(same.number, visa.number);

    t.app
        .htmx()
        .post(&update, &[("title", "Visa of Aya"), ("authority", "GDRFA")])
        .await
        .assert_ok();
    let edited = t.reload(&visa).await;
    assert_eq!(edited.title, "Visa of Aya");
    assert_eq!(edited.issuing_authority.as_deref(), Some("GDRFA"));
    assert_eq!(edited.expiry_date, visa.expiry_date);
    let entries = renox::audit::for_subject(t.db(), Document::TABLE, id, 10)
        .await
        .unwrap();
    let updated = entries
        .iter()
        .find(|e| e.action == "document.updated")
        .expect("the edit is audited");
    assert_eq!(updated.user_id, Some(manager.id));
    assert_eq!(updated.data["new"]["title"], "Visa of Aya");
    assert_eq!(updated.data["old"]["title"], visa.title);
}

/// Sub-task 3/5: a non-blocking kind edits its expiry date in place, audited
/// with old and new values, and the reminders sent for the old date are
/// forgotten.
#[renox::test]
async fn a_non_blocking_kind_edits_its_expiry_and_reschedules_reminders() {
    let t = boot().await;
    t.install(&[site_pass()]).await;
    let a = t.user("Aya", &[perm::UPLOAD, perm::REVIEW]).await;
    let manager = t.user("Omar", &[perm::MANAGE]).await;

    t.app.acting_as(&a);
    let response = t
        .app
        .htmx()
        .post("/test/documents/attach", &form("site_pass", "2026-11-30"))
        .await;
    let id: i64 = response.text().parse().unwrap();
    let pass = Document::find(t.db(), id).await.unwrap().unwrap();
    let pass = t.accept(&a, &pass).await;
    let date = renox::chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
    ReminderLog::insert_new(t.db(), id, 30, date).await.unwrap();

    t.app.acting_as(&manager);
    let update = format!("/test/documents/{id}/update");
    // A title-only edit keeps the reminders.
    t.app
        .htmx()
        .post(&update, &[("title", "Pass")])
        .await
        .assert_ok();
    assert_eq!(ReminderLog::offsets_for(t.db(), id).await.unwrap(), [30]);

    t.app
        .htmx()
        .post(&update, &[("expires_on", "2027-06-30"), ("number", "P-9")])
        .await
        .assert_ok();
    let edited = t.reload(&pass).await;
    assert_eq!(edited.expiry_date.unwrap().to_string(), "2027-06-30");
    assert_eq!(edited.number.as_deref(), Some("P-9"));
    assert!(
        ReminderLog::offsets_for(t.db(), id)
            .await
            .unwrap()
            .is_empty()
    );

    let entries = renox::audit::for_subject(t.db(), Document::TABLE, id, 10)
        .await
        .unwrap();
    let last = entries
        .iter()
        .find(|e| e.data["new"]["number"] == "P-9")
        .expect("the edit is audited");
    assert_eq!(last.action, "document.updated");
    assert_eq!(last.data["old"]["expiry_date"], "2026-11-30");
    assert_eq!(last.data["new"]["expiry_date"], "2027-06-30");
}
