//! The documents of one record (`/documents/of/{type}/{id}`): what is on
//! file, and the form that adds a document for review. It is how a person
//! gives a tenant their Emirates ID before a lease is submitted.

mod support;

use renox::prelude::*;
use wl_documents::{Document, DocumentStatus, permissions as perm};

use support::{boot, residence_visa};

const PDF: &[u8] = b"%PDF-1.7 visa";

#[renox::test]
async fn a_document_is_added_to_a_record_and_waits_for_review() {
    let t = boot().await;
    t.install(&[residence_visa()]).await;
    let agent = t.user("Ali", &[perm::VIEW, perm::UPLOAD]).await;
    t.app.acting_as(&agent);

    t.app
        .get("/documents/of/hr.employee/7")
        .await
        .assert_ok()
        .assert_see("No documents yet")
        .assert_see("Residence visa")
        .assert_see("Upload for review");

    t.app
        .post_multipart(
            "/documents/of/hr.employee/7",
            &[
                ("kind", "residence_visa"),
                ("number", "V-77"),
                ("expiry_date", "2031-01-31"),
            ],
            &[("files", "visa.pdf", PDF)],
        )
        .await
        .assert_redirect("/documents/of/hr.employee/7");

    let stored = Document::query()
        .where_(Document::SUBJECT_ID.eq(7))
        .first(t.db())
        .await
        .unwrap()
        .expect("the document is attached");
    assert_eq!(stored.subject_type, "hr.employee");
    assert_eq!(stored.status, DocumentStatus::PendingReview);
    assert_eq!(stored.uploaded_by, Some(agent.id));
    assert_eq!(stored.number.as_deref(), Some("V-77"));

    t.app
        .get("/documents/of/hr.employee/7")
        .await
        .assert_ok()
        .assert_see("V-77")
        .assert_see("Waiting for review");
    // Another record has none of it.
    t.app
        .get("/documents/of/hr.employee/8")
        .await
        .assert_see("No documents yet");
}

#[renox::test]
async fn the_form_says_what_is_wrong_under_its_field() {
    let t = boot().await;
    t.install(&[residence_visa()]).await;
    let agent = t.user("Ali", &[perm::VIEW, perm::UPLOAD]).await;
    t.app.acting_as(&agent);
    let url = "/documents/of/hr.employee/7";

    // No kind chosen.
    t.app
        .htmx()
        .post_multipart(url, &[("kind", "")], &[("files", "visa.pdf", PDF)])
        .await
        .assert_invalid("kind");
    // No file.
    t.app
        .htmx()
        .post_multipart(url, &[("kind", "residence_visa")], &[])
        .await
        .assert_invalid("files");
    // A kind that does not take this record.
    t.app
        .htmx()
        .post_multipart(
            "/documents/of/fleet.vehicle/7",
            &[("kind", "residence_visa")],
            &[("files", "visa.pdf", PDF)],
        )
        .await
        .assert_invalid("kind");
    // A date that is not one.
    t.app
        .htmx()
        .post_multipart(
            url,
            &[("kind", "residence_visa"), ("expiry_date", "next year")],
            &[("files", "visa.pdf", PDF)],
        )
        .await
        .assert_invalid("expiry_date");
    assert_eq!(Document::query().count(t.db()).await.unwrap(), 0);
}

#[renox::test]
async fn only_who_may_see_and_upload_get_the_page_and_the_form() {
    let t = boot().await;
    t.install(&[residence_visa()]).await;
    let reader = t.user("Rita", &[perm::VIEW]).await;
    t.app.acting_as(&reader);
    t.app
        .get("/documents/of/hr.employee/7")
        .await
        .assert_ok()
        .assert_dont_see("Upload for review");
    t.app
        .post_multipart(
            "/documents/of/hr.employee/7",
            &[("kind", "residence_visa")],
            &[("files", "visa.pdf", PDF)],
        )
        .await
        .assert_forbidden();

    let nobody = t.user("Nora", &[]).await;
    t.app.acting_as(&nobody);
    t.app
        .get("/documents/of/hr.employee/7")
        .await
        .assert_forbidden();
    assert_eq!(Document::query().count(t.db()).await.unwrap(), 0);
}
