//! The edit and correction pages (#51 A7): the read-only number and expiry
//! date of a blocking kind, the correction form, the queue badge and the
//! permission checks.

mod support;

use renox::prelude::*;
use wl_documents::{Document, DocumentStatus, KindSeed, permissions as perm};

use support::{Booted, boot, day, ravi, work_permit};

const PDF: &[u8] = b"%PDF-1.7 corrected";

fn trade_licence() -> KindSeed {
    KindSeed::new("trade_licence", "Trade licence")
        .subject_type("hr.employee")
        .expires()
}

struct World {
    t: Booted,
    /// Uploads, manages and reviews.
    ann: User,
    /// Reviews.
    rita: User,
    /// Holds nothing.
    dan: User,
    /// The accepted work permit (blocking kind).
    permit: Document,
    /// The accepted trade licence (not blocking).
    licence: Document,
}

async fn world() -> World {
    let t = boot().await;
    t.install(&[work_permit(), trade_licence()]).await;
    let ann = t
        .user(
            "Ann",
            &[perm::VIEW, perm::UPLOAD, perm::MANAGE, perm::REVIEW],
        )
        .await;
    let rita = t.reviewer().await;
    let dan = t.user("Dan", &[]).await;
    let permit = t
        .accepted(&rita, ravi(), "work_permit", Some(day(30)))
        .await;
    let licence = t
        .accepted(&rita, ravi(), "trade_licence", Some(day(60)))
        .await;
    World {
        t,
        ann,
        rita,
        dan,
        permit,
        licence,
    }
}

impl World {
    async fn correct(
        &self,
        by: &User,
        document: &Document,
        expiry: &str,
    ) -> renox::testing::TestResponse {
        self.t.app.acting_as(by);
        self.t
            .app
            .htmx()
            .post_multipart(
                &format!("/documents/{}/correction", document.id),
                &[("number", "WP-2"), ("expiry_date", expiry)],
                &[("files", "permit.pdf", PDF)],
            )
            .await
    }
}

#[renox::test]
async fn the_edit_page_shows_a_blocking_number_and_expiry_read_only() {
    let w = world().await;
    let t = &w.t;

    t.app.acting_as(&w.ann);
    t.app
        .get(&format!("/documents/{}/edit", w.permit.id))
        .await
        .assert_ok()
        .assert_see("name=\"title\"")
        .assert_see(&format!("/documents/{}/correction", w.permit.id))
        .assert_dont_see("name=\"expiry_date\"")
        .assert_dont_see("name=\"number\"");
    t.app
        .get(&format!("/documents/{}/edit", w.licence.id))
        .await
        .assert_ok()
        .assert_see("name=\"expiry_date\"")
        .assert_see("name=\"number\"");

    // The permit's expiry and number cannot change, for anyone.
    t.app
        .htmx()
        .post(
            &format!("/documents/{}", w.permit.id),
            &[
                ("title", "Work permit"),
                ("expiry_date", &day(400).to_string()),
            ],
        )
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(
            &format!("/documents/{}", w.permit.id),
            &[("title", "Work permit"), ("number", "X-1")],
        )
        .await
        .assert_status(422);
    assert_eq!(t.reload(&w.permit).await.expiry_date, w.permit.expiry_date);

    // The authority can.
    t.app
        .htmx()
        .post(
            &format!("/documents/{}", w.permit.id),
            &[("title", "Work permit"), ("issuing_authority", "MOHRE")],
        )
        .await
        .assert_ok();
    let saved = t.reload(&w.permit).await;
    assert_eq!(saved.issuing_authority.as_deref(), Some("MOHRE"));
    assert_eq!(saved.expiry_date, w.permit.expiry_date);

    // A kind that blocks nothing is edited in place, a bad date is a 422.
    let expiry = day(90).to_string();
    t.app
        .htmx()
        .post(
            &format!("/documents/{}", w.licence.id),
            &[("title", "Trade licence"), ("expiry_date", "soon")],
        )
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(
            &format!("/documents/{}", w.licence.id),
            &[
                ("title", "Trade licence"),
                ("number", "TL-9"),
                ("expiry_date", &expiry),
            ],
        )
        .await
        .assert_ok();
    let saved = t.reload(&w.licence).await;
    assert_eq!(saved.number.as_deref(), Some("TL-9"));
    assert_eq!(saved.expiry_date, Some(day(90)));
}

#[renox::test]
async fn a_correction_waits_for_another_user_and_shows_a_badge() {
    let w = world().await;
    let t = &w.t;

    t.app.acting_as(&w.ann);
    t.app
        .get(&format!("/documents/{}/correction", w.permit.id))
        .await
        .assert_ok()
        .assert_see("name=\"files\"");

    // No file is a 422; so is a bad date.
    t.app
        .htmx()
        .post_multipart(
            &format!("/documents/{}/correction", w.permit.id),
            &[("expiry_date", &day(400).to_string())],
            &[],
        )
        .await
        .assert_status(422);
    w.correct(&w.ann, &w.permit, "later")
        .await
        .assert_status(422);

    let expiry = day(400).to_string();
    w.correct(&w.ann, &w.permit, &expiry).await.assert_ok();
    let new = Document::query()
        .where_(Document::STATUS.eq(DocumentStatus::PendingReview))
        .first(t.db())
        .await
        .unwrap()
        .expect("the correction is pending");
    assert_eq!(new.uploaded_by, Some(w.ann.id));
    assert_eq!(new.expiry_date, Some(day(400)));
    assert_eq!(t.reload(&w.permit).await.status, DocumentStatus::Accepted);

    // A second open correction is a 422.
    w.correct(&w.ann, &w.permit, &expiry)
        .await
        .assert_status(422);

    // The queue marks it; the uploader cannot accept it, another user can.
    t.app.acting_as(&w.rita);
    t.app
        .get("/documents/review")
        .await
        .assert_ok()
        .assert_see("Correction");
    t.app.acting_as(&w.ann);
    t.app
        .htmx()
        .post(&format!("/documents/{}/accept", new.id), &[])
        .await
        .assert_status(422);
    assert_eq!(t.reload(&new).await.status, DocumentStatus::PendingReview);
    let accepted = t.accept(&w.rita, &new).await;
    assert_eq!(accepted.status, DocumentStatus::Accepted);
    assert_eq!(t.reload(&w.permit).await.status, DocumentStatus::Superseded);

    // An accepted document is not corrected again from the old one.
    w.correct(&w.ann, &w.permit, &expiry)
        .await
        .assert_status(422);
}

#[renox::test]
async fn the_pages_check_their_permissions() {
    let w = world().await;
    let t = &w.t;
    let edit = format!("/documents/{}/edit", w.permit.id);
    let correction = format!("/documents/{}/correction", w.permit.id);

    t.app.acting_as(&w.dan);
    t.app.get(&edit).await.assert_forbidden();
    t.app.get(&correction).await.assert_forbidden();
    t.app
        .htmx()
        .post(
            &format!("/documents/{}", w.permit.id),
            &[("title", "Work permit")],
        )
        .await
        .assert_forbidden();
    w.correct(&w.dan, &w.permit, &day(400).to_string())
        .await
        .assert_forbidden();

    // Reviewing alone neither edits nor corrects.
    t.app.acting_as(&w.rita);
    t.app.get(&edit).await.assert_forbidden();
    t.app.get(&correction).await.assert_forbidden();
}
