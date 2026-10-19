//! Acceptance checks AC1-AC7 of #51 (`DOC.ACL.03`): a blocking document is
//! corrected only through a new version that another user accepts. The page
//! and service details are in `blocking.rs` and `document_pages.rs`; here
//! each criterion is followed from start to end.

mod support;

use renox::prelude::*;
use wl_documents::{
    Document, DocumentKind, DocumentStatus, Documents, KindSeed, permissions as perm,
};

use support::{Booted, boot, day, driving_licence, ravi, work_permit};

const PDF: &[u8] = b"%PDF-1.7 corrected";

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

async fn entries(t: &Booted, id: i64) -> Vec<renox::audit::AuditLog> {
    renox::audit::for_subject(t.db(), Document::TABLE, id, 20)
        .await
        .unwrap()
}

/// `by` submits a correction of `old` on the page.
async fn correct(
    t: &Booted,
    by: &User,
    old: &Document,
    expiry: &str,
) -> renox::testing::TestResponse {
    t.app.acting_as(by);
    t.app
        .htmx()
        .post_multipart(
            &format!("/documents/{}/correction", old.id),
            &[("number", "WP-2"), ("expiry_date", expiry)],
            &[("files", "permit.pdf", PDF)],
        )
        .await
}

async fn pending(t: &Booted, kind: &str) -> Option<Document> {
    let kind = t.kind(kind).await;
    Document::query()
        .where_(Document::KIND_ID.eq(kind.id))
        .where_(Document::STATUS.eq(DocumentStatus::PendingReview))
        .first(t.db())
        .await
        .unwrap()
}

/// AC1: number and expiry of an accepted blocking document are read-only and
/// refused in place, for a manager and for a user holding every permission;
/// title and authority change and are audited.
#[renox::test]
async fn ac1_number_and_expiry_of_a_blocking_document_are_not_editable() {
    let t = boot().await;
    t.install(&[work_permit()]).await;
    let rita = t.reviewer().await;
    let manager = t.user("Omar", &[perm::VIEW, perm::MANAGE]).await;
    let admin = t
        .user(
            "Adel",
            &[perm::VIEW, perm::UPLOAD, perm::MANAGE, perm::REVIEW],
        )
        .await;
    let permit = t
        .accepted(&rita, ravi(), "work_permit", Some(day(30)))
        .await;
    let edit = format!("/documents/{}/edit", permit.id);
    let save = format!("/documents/{}", permit.id);

    for user in [&manager, &admin] {
        t.app.acting_as(user);
        let page = t.app.get(&edit).await;
        page.assert_ok()
            .assert_dont_see("name=\"expiry_date\"")
            .assert_dont_see("name=\"number\"");
        // The link to the correction form is for who may upload.
        if user.id == admin.id {
            page.assert_see(&format!("/documents/{}/correction", permit.id));
        } else {
            page.assert_dont_see(&format!("/documents/{}/correction", permit.id));
        }
        let later = day(400).to_string();
        t.app
            .htmx()
            .post(&save, &[("title", "Work permit"), ("expiry_date", &later)])
            .await
            .assert_status(422);
        t.app
            .htmx()
            .post(&save, &[("title", "Work permit"), ("number", "WP-9")])
            .await
            .assert_status(422);
        let same = t.reload(&permit).await;
        assert_eq!(same.expiry_date, permit.expiry_date);
        assert_eq!(same.number, permit.number);
    }

    t.app.acting_as(&manager);
    t.app
        .htmx()
        .post(
            &save,
            &[("title", "Permit of Ravi"), ("issuing_authority", "MOHRE")],
        )
        .await
        .assert_ok();
    let saved = t.reload(&permit).await;
    assert_eq!(saved.title, "Permit of Ravi");
    assert_eq!(saved.issuing_authority.as_deref(), Some("MOHRE"));
    assert_eq!(saved.expiry_date, permit.expiry_date);
    let updated = entries(&t, permit.id)
        .await
        .into_iter()
        .find(|e| e.action == "document.updated")
        .expect("the edit is audited");
    assert_eq!(updated.user_id, Some(manager.id));
    assert_eq!(updated.data["old"]["title"], permit.title);
    assert_eq!(updated.data["new"]["title"], "Permit of Ravi");
    assert_eq!(updated.data["new"]["issuing_authority"], "MOHRE");
}

/// AC2 and AC3: A submits a correction, cannot accept it, B accepts it; the
/// expiry in force changes and both steps are audited with old and new values.
#[renox::test]
async fn ac2_ac3_a_correction_takes_effect_when_another_user_accepts_it() {
    let t = boot().await;
    t.install(&[work_permit()]).await;
    let rita = t.reviewer().await;
    let a = t
        .user("Ann", &[perm::VIEW, perm::UPLOAD, perm::REVIEW])
        .await;
    let old = t
        .accepted(&rita, ravi(), "work_permit", Some(day(30)))
        .await;

    let new_expiry = day(400);
    correct(&t, &a, &old, &new_expiry.to_string())
        .await
        .assert_ok();
    let new = pending(&t, "work_permit")
        .await
        .expect("the correction is pending");
    assert_ne!(new.id, old.id);
    assert_eq!(new.uploaded_by, Some(a.id));
    assert_eq!(new.kind_id, old.kind_id);
    assert_eq!(new.subject(), old.subject());
    assert_eq!(new.current_version, 1);
    assert_eq!(t.reload(&old).await.status, DocumentStatus::Accepted);
    // Until it is accepted the old document is in force.
    let current = Documents::current(t.db(), &ravi(), "work_permit")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.id, old.id);

    // A cannot accept it.
    t.app.acting_as(&a);
    t.app
        .htmx()
        .post(&format!("/documents/{}/accept", new.id), &[])
        .await
        .assert_status(422);
    assert_eq!(t.reload(&new).await.status, DocumentStatus::PendingReview);

    let accepted = t.accept(&rita, &new).await;
    assert_eq!(accepted.status, DocumentStatus::Accepted);
    assert_eq!(t.reload(&old).await.status, DocumentStatus::Superseded);
    let current = Documents::current(t.db(), &ravi(), "work_permit")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.id, new.id);
    assert_eq!(current.expiry_date, Some(new_expiry));

    let trail = entries(&t, new.id).await;
    let submitted = trail
        .iter()
        .find(|e| e.action == "document.correction_submitted")
        .expect("the submission is audited");
    assert_eq!(submitted.user_id, Some(a.id));
    let corrected = trail
        .iter()
        .find(|e| e.action == "document.corrected")
        .expect("the correction is audited");
    assert_eq!(corrected.user_id, Some(rita.id));
    assert_eq!(corrected.data["old_document"], old.id);
    assert_eq!(corrected.data["new_document"], new.id);
    assert_eq!(
        corrected.data["old_expiry_date"],
        day(30).to_string().as_str()
    );
    assert_eq!(
        corrected.data["new_expiry_date"],
        new_expiry.to_string().as_str()
    );
    assert_eq!(
        corrected.data["old_number"],
        renox::serde_json::json!(t.reload(&old).await.number)
    );
    assert_eq!(corrected.data["new_number"], "WP-2");
    assert_eq!(corrected.data["uploaded_by"], a.id);
    assert_eq!(corrected.data["accepted_by"], rita.id);
}

/// AC4: a kind that blocks nothing is edited in place, audited with old and
/// new values.
#[renox::test]
async fn ac4_a_non_blocking_kind_is_edited_in_place_and_audited() {
    let t = boot().await;
    t.install(&[site_pass()]).await;
    let rita = t.reviewer().await;
    let manager = t.user("Omar", &[perm::VIEW, perm::MANAGE]).await;
    let pass = t.accepted(&rita, ravi(), "site_pass", Some(day(30))).await;

    t.app.acting_as(&manager);
    t.app
        .htmx()
        .post(
            &format!("/documents/{}", pass.id),
            &[
                ("title", pass.title.as_str()),
                ("number", "SP-9"),
                ("expiry_date", &day(200).to_string()),
            ],
        )
        .await
        .assert_ok();
    let edited = t.reload(&pass).await;
    assert_eq!(edited.number.as_deref(), Some("SP-9"));
    assert_eq!(edited.expiry_date, Some(day(200)));
    assert_eq!(edited.status, DocumentStatus::Accepted);
    let updated = entries(&t, pass.id)
        .await
        .into_iter()
        .find(|e| e.action == "document.updated")
        .expect("the edit is audited");
    assert_eq!(updated.user_id, Some(manager.id));
    assert_eq!(
        updated.data["old"]["expiry_date"],
        day(30).to_string().as_str()
    );
    assert_eq!(
        updated.data["new"]["expiry_date"],
        day(200).to_string().as_str()
    );
    assert_eq!(updated.data["new"]["number"], "SP-9");
}

/// AC5: `record` is refused for a blocking kind, also when the stored
/// `needs_review` is false; the upload waits for review and its uploader
/// cannot accept it, another user can.
#[renox::test]
async fn ac5_a_blocking_kind_cannot_be_recorded_and_is_reviewed_by_someone_else() {
    let t = boot().await;
    t.install(&[driving_licence()]).await;
    let a = t
        .user("Ann", &[perm::VIEW, perm::UPLOAD, perm::REVIEW])
        .await;
    let b = t.reviewer().await;
    let kind = t.kind("driving_licence").await;
    DocumentKind::query()
        .where_(DocumentKind::ID.eq(kind.id))
        .update(t.db(), &[("needs_review", &false)])
        .await
        .unwrap();

    t.app.acting_as(&a);
    t.app
        .htmx()
        .post(
            "/test/documents/record",
            &form("driving_licence", &day(300).to_string()),
        )
        .await
        .assert_status(422);
    assert!(Document::query().get(t.db()).await.unwrap().is_empty());

    let response = t
        .app
        .htmx()
        .post(
            "/test/documents/attach",
            &form("driving_licence", &day(300).to_string()),
        )
        .await;
    response.assert_ok();
    let id: i64 = response.text().parse().unwrap();
    let licence = Document::find(t.db(), id).await.unwrap().unwrap();
    assert_eq!(licence.status, DocumentStatus::PendingReview);
    assert_eq!(licence.uploaded_by, Some(a.id));

    t.app
        .htmx()
        .post(&format!("/documents/{id}/accept"), &[])
        .await
        .assert_status(422);
    assert_eq!(
        t.reload(&licence).await.status,
        DocumentStatus::PendingReview
    );
    let accepted = t.accept(&b, &licence).await;
    assert_eq!(accepted.status, DocumentStatus::Accepted);
}

/// AC6: one open correction at a time, only an accepted document is
/// corrected, and the routes check their permissions.
#[renox::test]
async fn ac6_corrections_are_limited_and_routes_check_permissions() {
    let t = boot().await;
    t.install(&[work_permit()]).await;
    let rita = t.reviewer().await;
    let a = t.user("Ann", &[perm::VIEW, perm::UPLOAD]).await;
    let nobody = t.user("Dan", &[]).await;
    let old = t
        .accepted(&rita, ravi(), "work_permit", Some(day(30)))
        .await;
    let later = day(400).to_string();

    correct(&t, &a, &old, &later).await.assert_ok();
    correct(&t, &a, &old, &later).await.assert_status(422);
    let open = pending(&t, "work_permit").await.unwrap();

    // A document that is not accepted is not corrected.
    correct(&t, &a, &open, &later).await.assert_status(422);
    assert_eq!(
        Document::query()
            .where_(Document::STATUS.eq(DocumentStatus::PendingReview))
            .get(t.db())
            .await
            .unwrap()
            .len(),
        1
    );

    // Without the permissions the routes are 403.
    t.app.acting_as(&nobody);
    t.app
        .get(&format!("/documents/{}/correction", old.id))
        .await
        .assert_forbidden();
    t.app
        .get(&format!("/documents/{}/edit", old.id))
        .await
        .assert_forbidden();
    correct(&t, &nobody, &old, &later).await.assert_forbidden();

    // Uploading alone does not edit.
    t.app.acting_as(&a);
    t.app
        .get(&format!("/documents/{}/edit", old.id))
        .await
        .assert_forbidden();
    t.app
        .htmx()
        .post(&format!("/documents/{}", old.id), &[("title", "X")])
        .await
        .assert_forbidden();
    assert_eq!(t.reload(&old).await.title, old.title);
}

/// AC7: the uploader still accepts their own upload of a kind that blocks
/// nothing.
#[renox::test]
async fn ac7_the_uploader_accepts_a_non_blocking_upload() {
    let t = boot().await;
    t.install(&[site_pass()]).await;
    let a = t
        .user("Ann", &[perm::VIEW, perm::UPLOAD, perm::REVIEW])
        .await;
    t.app.acting_as(&a);
    let response = t
        .app
        .htmx()
        .post(
            "/test/documents/attach",
            &form("site_pass", &day(60).to_string()),
        )
        .await;
    response.assert_ok();
    let id: i64 = response.text().parse().unwrap();
    let pass = Document::find(t.db(), id).await.unwrap().unwrap();
    assert_eq!(pass.uploaded_by, Some(a.id));
    let accepted = t.accept(&a, &pass).await;
    assert_eq!(accepted.status, DocumentStatus::Accepted);
}
