//! Blocking kinds and the separation of duties (`DOC.ACL.03`, #51 part 1):
//! no `record`, and the uploader cannot accept their own upload.

mod support;

use renox::prelude::*;

use wl_documents::permissions as perm;
use wl_documents::{Document, DocumentKind, DocumentStatus, KindSeed};

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
