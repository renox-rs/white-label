//! The document lifecycle (wl-documents §4.1) and its files: the acceptance
//! check AC9 of #45.

mod support;

use renox::prelude::*;
use renox::state_machine;
use wl_documents::documents::sha256_hex;
use wl_documents::{AttachReason, Document, DocumentStatus, Documents, KindSeed, NewDocument};
use wl_kernel::documents::{
    DocumentAccepted, DocumentRejected, DocumentSuperseded, DocumentUploaded,
};

use support::{boot, day, pdf, ravi, residence_visa};

/// A kind that needs no review, so `record` accepts at once.
fn site_photo() -> KindSeed {
    KindSeed::new("site_photo", "Site photo")
        .subject_type("hr.employee")
        .without_review()
        .file_types(&["jpg", "pdf"])
        .max_size(64 * 1024)
}

/// The transitions of `document`, oldest first.
async fn transitions(db: &Db, document: &Document) -> Vec<String> {
    state_machine::history(db, Document::TABLE, document.id)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.transition)
        .collect()
}

/// `submit` gives `pending_review`; `accept` supersedes the previous
/// accepted document of the kind and subject.
#[renox::test]
async fn submit_and_accept_supersede_the_previous_document() {
    let t = boot().await;
    t.install(&[residence_visa()]).await;
    let rita = t.reviewer().await;

    let first = t.attach(ravi(), "residence_visa", Some(day(20))).await;
    assert_eq!(first.status, DocumentStatus::PendingReview);
    t.app
        .assert_emitted::<DocumentUploaded>(|e| e.document == first.document_id());
    // Pending is not current.
    assert!(
        Documents::current(t.db(), &ravi(), "residence_visa")
            .await
            .unwrap()
            .is_none()
    );

    let first = t.accept(&rita, &first).await;
    assert_eq!(first.status, DocumentStatus::Accepted);
    assert!(first.accepted_at.is_some());
    let current = Documents::current(t.db(), &ravi(), "residence_visa")
        .await
        .unwrap();
    assert_eq!(current.unwrap().id, first.id);

    let second = t.attach(ravi(), "residence_visa", Some(day(400))).await;
    let second = t.accept(&rita, &second).await;
    assert_eq!(second.status, DocumentStatus::Accepted);

    let first = t.reload(&first).await;
    assert_eq!(first.status, DocumentStatus::Superseded);
    assert_eq!(first.superseded_by, Some(second.id));
    assert_eq!(
        transitions(t.db(), &first).await,
        ["submit", "accept", "supersede"]
    );
    let current = Documents::current(t.db(), &ravi(), "residence_visa")
        .await
        .unwrap();
    assert_eq!(current.unwrap().id, second.id);
    t.app
        .assert_emitted::<DocumentAccepted>(|e| e.document == second.document_id());
    t.app.assert_emitted::<DocumentSuperseded>(|e| {
        e.document == first.document_id() && e.replaced_by == second.document_id()
    });
}

/// `reject` needs a reason; `resubmit` adds a new version and sends the
/// document back to review. Every version keeps its storage key and SHA-256.
#[renox::test]
async fn reject_needs_a_reason_and_resubmit_adds_a_version() {
    let t = boot().await;
    t.install(&[residence_visa()]).await;
    let rita = t.reviewer().await;
    let uploader = t.user("Ula", &[]).await;
    let document = t.attach(ravi(), "residence_visa", Some(day(20))).await;
    let page = format!("/documents/{}/reject", document.id);

    t.app.acting_as(&rita);
    t.app
        .htmx()
        .post(&page, &[("reason", "  ")])
        .await
        .assert_status(422);
    t.app.htmx().post(&page, &[]).await.assert_status(422);
    assert_eq!(
        t.reload(&document).await.status,
        DocumentStatus::PendingReview
    );

    t.app
        .htmx()
        .post(&page, &[("reason", "Photo is cut off")])
        .await
        .assert_ok();
    let rejected = t.reload(&document).await;
    assert_eq!(rejected.status, DocumentStatus::Rejected);
    assert_eq!(
        rejected.rejection_reason.as_deref(),
        Some("Photo is cut off")
    );
    t.app
        .assert_emitted::<DocumentRejected>(|e| e.reason == "Photo is cut off");

    t.app.acting_as(&uploader);
    let resubmit = format!("/test/documents/{}/resubmit", document.id);
    let response = t
        .app
        .htmx()
        .post(
            &resubmit,
            &[("file_name", "visa-full.pdf"), ("size", "4096")],
        )
        .await;
    response.assert_ok();
    assert_eq!(response.text(), "2");
    let resubmitted = t.reload(&document).await;
    assert_eq!(resubmitted.status, DocumentStatus::PendingReview);
    assert_eq!(resubmitted.current_version, 2);
    assert!(resubmitted.rejection_reason.is_none());
    assert_eq!(
        transitions(t.db(), &resubmitted).await,
        ["submit", "reject", "resubmit"]
    );

    let versions = resubmitted.versions(t.db()).await.unwrap();
    assert_eq!(versions.len(), 2);
    for version in &versions {
        assert!(!version.storage_key.is_empty());
        assert_eq!(version.checksum.len(), 64);
        let stored = version
            .read(t.state())
            .await
            .unwrap()
            .expect("the file is stored");
        assert_eq!(sha256_hex(&stored), version.checksum);
        assert_eq!(i64::try_from(stored.len()).unwrap(), version.size_bytes);
    }
    assert_ne!(versions[0].storage_key, versions[1].storage_key);
    assert_eq!(versions[0].file_name, "scan.pdf");
    assert_eq!(versions[1].file_name, "visa-full.pdf");
    assert_eq!(versions[1].size_bytes, 4096);

    // A document that is not rejected cannot be resubmitted.
    t.app
        .htmx()
        .post(&resubmit, &[("file_name", "again.pdf"), ("size", "10")])
        .await
        .assert_status(422);
}

/// `record` accepts at once for a kind without review (and needs
/// `documents.upload`); `archive` needs `documents.manage`.
#[renox::test]
async fn record_accepts_at_once_and_archive_works() {
    let t = boot().await;
    t.install(&[site_photo(), residence_visa()]).await;
    let clerk = t.user("Clara", &[wl_documents::permissions::UPLOAD]).await;
    let manager = t.user("Mona", &[wl_documents::permissions::MANAGE]).await;
    let nobody = t.user("Nora", &[]).await;
    let record = [
        ("subject_id", "7"),
        ("kind", "site_photo"),
        ("file_name", "site.pdf"),
        ("size", "100"),
    ];

    t.app.acting_as(&nobody);
    t.app
        .htmx()
        .post("/test/documents/record", &record)
        .await
        .assert_forbidden();

    t.app.acting_as(&clerk);
    let response = t.app.htmx().post("/test/documents/record", &record).await;
    response.assert_ok();
    let id: i64 = response.text().parse().unwrap();
    let recorded = Document::find(t.db(), id).await.unwrap().unwrap();
    assert_eq!(recorded.status, DocumentStatus::Accepted);
    assert_eq!(transitions(t.db(), &recorded).await, ["record"]);
    let current = Documents::current(t.db(), &ravi(), "site_photo")
        .await
        .unwrap();
    assert_eq!(current.unwrap().id, id);

    // A kind that needs review cannot be recorded.
    let visa = [
        ("subject_id", "7"),
        ("kind", "residence_visa"),
        ("file_name", "v.pdf"),
        ("size", "100"),
    ];
    t.app
        .htmx()
        .post("/test/documents/record", &visa)
        .await
        .assert_status(422);

    let archive = format!("/test/documents/{id}/archive");
    t.app.acting_as(&clerk);
    t.app.htmx().post(&archive, &[]).await.assert_forbidden();
    t.app.acting_as(&manager);
    let response = t
        .app
        .htmx()
        .post(&archive, &[("reason", "Not needed")])
        .await;
    response.assert_ok();
    assert_eq!(response.text(), "archived");
    let archived = t.reload(&recorded).await;
    assert_eq!(archived.status, DocumentStatus::Archived);
    assert_eq!(transitions(t.db(), &archived).await, ["record", "archive"]);
    assert!(
        Documents::current(t.db(), &ravi(), "site_photo")
            .await
            .unwrap()
            .is_none()
    );
}

/// A disallowed file type, a file that is too large, an empty one and a
/// subject the kind does not attach to are refused, and nothing is stored.
#[renox::test]
async fn a_disallowed_or_oversized_file_is_refused() {
    let t = boot().await;
    t.install(&[site_photo()]).await;
    let attach = |subject, file| {
        Documents::attach(
            t.state(),
            None,
            NewDocument::new(subject, "site_photo", file),
        )
    };

    let refused = attach(ravi(), pdf("site.exe")).await.unwrap().unwrap_err();
    assert_eq!(refused.reason, AttachReason::FileType);
    assert_eq!(
        refused.to_string(),
        "Site photo does not accept .exe files (allowed: jpg, pdf)"
    );

    let big = Upload::new("site.pdf", "application/pdf", vec![b'x'; 65 * 1024]);
    let refused = attach(ravi(), big).await.unwrap().unwrap_err();
    assert_eq!(refused.reason, AttachReason::TooLarge);
    assert!(refused.to_string().contains("larger than"), "{refused}");

    let empty = Upload::new("site.pdf", "application/pdf", Vec::new());
    let refused = attach(ravi(), empty).await.unwrap().unwrap_err();
    assert_eq!(refused.reason, AttachReason::Empty);

    let vehicle = wl_kernel::Subject::new("fleet.vehicle", 3);
    let refused = attach(vehicle.clone(), pdf("site.pdf"))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.reason, AttachReason::SubjectType);
    match refused.into_error() {
        Error::Status(code, _) => assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY),
        other => panic!("unexpected error {other:?}"),
    }

    assert!(
        Documents::of_subject(t.db(), &ravi())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        Documents::of_subject(t.db(), &vehicle)
            .await
            .unwrap()
            .is_empty()
    );

    // A file that fits is stored privately, with its checksum.
    let file = pdf("site.pdf");
    let document = attach(ravi(), file.clone()).await.unwrap().unwrap();
    let versions = document.versions(t.db()).await.unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].checksum, sha256_hex(file.bytes()));
    assert!(
        versions[0]
            .storage_key
            .starts_with(&format!("documents/hr.employee/7/{}/v1/", document.id))
    );
}
