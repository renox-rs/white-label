//! Acceptance checks of #52 (part 6/6) for the pages (AC12 and the review
//! state of AC7): who sees the library and the details, which actions they
//! see, and how a passed review date reads. The page flows themselves are in
//! `sop_library_pages.rs`.

mod support;

use renox::chrono::Duration;
use renox::prelude::*;
use wl_documents::sops::kind_seeds;
use wl_documents::{Document, Sop, permissions as perm};
use wl_kernel::documents::today;

use support::boot;

const PDF: &[u8] = b"%PDF-1.7 sop";

#[renox::test]
async fn the_pages_hide_what_the_viewer_cannot_use_and_read_overdue_not_expired() {
    let t = boot().await;
    t.install(&kind_seeds()).await;
    t.roles(&["hse_officer"]).await;
    let hana = t.user("Hana", &[perm::SOPS_MANAGE]).await;
    hana.assign_role(t.db(), "hse_officer").await.unwrap();
    let vera = t.user("Vera", &[perm::VIEW]).await;
    let dan = t.user("Dan", &[]).await;

    t.app.acting_as(&hana);
    t.app
        .htmx()
        .post(
            "/documents/sops",
            &[
                ("code", "SOP-HSE-01"),
                ("title_en", "Working at height"),
                ("owner_role", "hse_officer"),
                ("review_interval_months", "12"),
                ("ack_due_days", "7"),
            ],
        )
        .await
        .assert_status(303);
    let sop = Sop::query()
        .where_(Sop::CODE.eq("SOP-HSE-01"))
        .first(t.db())
        .await
        .unwrap()
        .expect("the SOP exists");
    let details = format!("/documents/sops/{}", sop.id);
    t.app
        .htmx()
        .post_multipart(
            &format!("{details}/versions"),
            &[],
            &[("files", "working-at-height.pdf", PDF)],
        )
        .await
        .assert_ok();
    let v1 = Document::query()
        .where_(Document::SUBJECT_ID.eq(sop.id))
        .first(t.db())
        .await
        .unwrap()
        .expect("the version is stored");
    t.app
        .htmx()
        .post(&format!("/documents/sop-versions/{}/accept", v1.id), &[])
        .await
        .assert_ok();

    // `sops.manage` sees the actions; `documents.view` sees the data only.
    t.app.get("/documents/sops").await.assert_see("New SOP");
    t.app.acting_as(&vera);
    t.app
        .get("/documents/sops")
        .await
        .assert_ok()
        .assert_see("SOP-HSE-01")
        .assert_dont_see("New SOP");
    t.app
        .get(&details)
        .await
        .assert_ok()
        .assert_see("SOP-HSE-01")
        .assert_dont_see("edit-readers")
        .assert_dont_see("upload-version")
        .assert_dont_see("withdraw-sop");

    // Others get 403 on every page.
    t.app.acting_as(&dan);
    for path in ["/documents/sops", "/documents/sops/new", details.as_str()] {
        t.app.get(path).await.assert_forbidden();
    }

    // A passed review date reads "Review overdue", never "Expired", and the
    // SOP stays active.
    let yesterday = today() - Duration::days(1);
    Document::query()
        .where_(Document::ID.eq(v1.id))
        .update(t.db(), &[("expiry_date", &yesterday)])
        .await
        .unwrap();
    t.app.acting_as(&vera);
    for path in ["/documents/sops", details.as_str()] {
        t.app
            .get(path)
            .await
            .assert_ok()
            .assert_see("Review overdue")
            .assert_see("Active")
            .assert_dont_see("Expired");
    }
}
