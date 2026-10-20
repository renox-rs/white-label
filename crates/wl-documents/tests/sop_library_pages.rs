//! The SOP back-office pages (#52, part 5/6): the library, the create form
//! and the details page with versions, translations, readers and actions,
//! with their permission checks (AC12).

mod support;

use renox::prelude::*;
use wl_documents::sops::kind_seeds;
use wl_documents::{Document, DocumentStatus, Sop, SopStatus, Sops, permissions as perm};

use support::{Booted, boot};

const PDF: &[u8] = b"%PDF-1.7 sop";

struct World {
    t: Booted,
    /// Manages SOPs and holds the owner role.
    hana: User,
    /// Manages SOPs without the owner role.
    mona: User,
    /// Only views documents.
    vera: User,
    /// Holds nothing.
    dan: User,
    /// Holds `documents.review`.
    rita: User,
}

async fn world() -> World {
    let t = boot().await;
    t.install(&kind_seeds()).await;
    t.roles(&["hse_officer"]).await;
    let hana = t.user("Hana", &[perm::SOPS_MANAGE]).await;
    hana.assign_role(t.db(), "hse_officer").await.unwrap();
    let mona = t.user("Mona", &[perm::SOPS_MANAGE]).await;
    let vera = t.user("Vera", &[perm::VIEW]).await;
    let dan = t.user("Dan", &[]).await;
    let rita = t.reviewer().await;
    World {
        t,
        hana,
        mona,
        vera,
        dan,
        rita,
    }
}

impl World {
    async fn create(&self, by: &User, code: &str) -> renox::testing::TestResponse {
        self.t.app.acting_as(by);
        self.t
            .app
            .htmx()
            .post(
                "/documents/sops",
                &[
                    ("code", code),
                    ("title_en", "Working at height"),
                    ("title_ar", "العمل على ارتفاع"),
                    ("category", "hse"),
                    ("owner_role", "hse_officer"),
                    ("legal_entity_type", ""),
                    ("legal_entity_id", ""),
                    ("review_interval_months", "12"),
                    ("ack_due_days", "7"),
                ],
            )
            .await
    }

    async fn sop(&self, code: &str) -> Sop {
        Sop::query()
            .where_(Sop::CODE.eq(code))
            .first(self.t.db())
            .await
            .unwrap()
            .expect("the SOP exists")
    }

    async fn upload(&self, by: &User, sop: &Sop) -> Document {
        self.t.app.acting_as(by);
        self.t
            .app
            .htmx()
            .post_multipart(
                &format!("/documents/sops/{}/versions", sop.id),
                &[("note", "first issue")],
                &[("files", "working-at-height.pdf", PDF)],
            )
            .await
            .assert_ok();
        Document::query()
            .where_(Document::SUBJECT_ID.eq(sop.id))
            .order_by_desc(Document::ID)
            .first(self.t.db())
            .await
            .unwrap()
            .expect("the version is stored")
    }
}

#[renox::test]
async fn the_pages_create_list_show_and_guard_an_sop() {
    let w = world().await;
    let t = &w.t;

    // The form is for `sops.manage` only.
    t.app.acting_as(&w.vera);
    t.app.get("/documents/sops/new").await.assert_forbidden();
    t.app.acting_as(&w.mona);
    t.app
        .get("/documents/sops/new")
        .await
        .assert_ok()
        .assert_see("owner_role");

    // Creating stores the SOP; a second one with the same code is a 422.
    w.create(&w.mona, "SOP-HSE-01").await.assert_status(303);
    let sop = w.sop("SOP-HSE-01").await;
    assert_eq!(sop.owner_role, "hse_officer");
    assert_eq!(sop.review_interval_months, 12);
    w.create(&w.mona, "SOP-HSE-01").await.assert_status(422);
    t.app
        .htmx()
        .post("/documents/sops", &[("code", "SOP-X")])
        .await
        .assert_status(422);
    assert_eq!(Sop::query().count(t.db()).await.unwrap(), 1);
    t.app.acting_as(&w.vera);
    t.app
        .htmx()
        .post(
            "/documents/sops",
            &[
                ("code", "SOP-V"),
                ("title_en", "t"),
                ("owner_role", "hse_officer"),
            ],
        )
        .await
        .assert_forbidden();

    // The library: `sops.manage` and `documents.view` see it, others do not.
    for viewer in [&w.mona, &w.vera] {
        t.app.acting_as(viewer);
        t.app
            .get("/documents/sops")
            .await
            .assert_ok()
            .assert_see("SOP-HSE-01")
            .assert_see("Working at height")
            .assert_see("Draft");
    }
    t.app.acting_as(&w.dan);
    t.app.get("/documents/sops").await.assert_forbidden();
    t.app
        .get(&format!("/documents/sops/{}", sop.id))
        .await
        .assert_forbidden();

    // Readers: one line each; the details page lists them.
    t.app.acting_as(&w.mona);
    t.app
        .htmx()
        .post(
            &format!("/documents/sops/{}/readers", sop.id),
            &[("readers", "site_worker@property.site:1\nall")],
        )
        .await
        .assert_ok();
    let readers = Sops::readers(t.db(), sop.id).await.unwrap();
    assert_eq!(readers.len(), 2);
    t.app
        .htmx()
        .post(
            &format!("/documents/sops/{}/readers", sop.id),
            &[("readers", "site_worker@property.site")],
        )
        .await
        .assert_status(422);
    t.app.acting_as(&w.vera);
    t.app
        .htmx()
        .post(
            &format!("/documents/sops/{}/readers", sop.id),
            &[("readers", "all")],
        )
        .await
        .assert_forbidden();
    t.app
        .get(&format!("/documents/sops/{}", sop.id))
        .await
        .assert_ok()
        .assert_see("All staff")
        .assert_see("site_worker (property.site #1)")
        .assert_dont_see("edit-readers")
        .assert_dont_see("upload-version");

    // A version is uploaded by a manager; a viewer cannot upload.
    t.app.acting_as(&w.vera);
    t.app
        .htmx()
        .post_multipart(
            &format!("/documents/sops/{}/versions", sop.id),
            &[],
            &[("files", "v.pdf", PDF)],
        )
        .await
        .assert_forbidden();
    t.app.acting_as(&w.mona);
    t.app
        .htmx()
        .post_multipart(&format!("/documents/sops/{}/versions", sop.id), &[], &[])
        .await
        .assert_status(422);
    let v1 = w.upload(&w.mona, &sop).await;
    assert_eq!(v1.status, DocumentStatus::PendingReview);

    // Pending: the manager without the owner role sees no decision buttons,
    // the viewer sees neither those nor the upload; the owner sees them.
    t.app.acting_as(&w.mona);
    t.app
        .get(&format!("/documents/sops/{}", sop.id))
        .await
        .assert_ok()
        .assert_see("v1")
        .assert_see("Pending review")
        .assert_see("upload-version")
        .assert_see("edit-readers")
        .assert_dont_see("accept-version-");
    t.app.acting_as(&w.vera);
    t.app
        .get(&format!("/documents/sops/{}", sop.id))
        .await
        .assert_ok()
        .assert_see("v1")
        .assert_dont_see("accept-version-")
        .assert_dont_see("upload-version");
    t.app.acting_as(&w.hana);
    t.app
        .get(&format!("/documents/sops/{}", sop.id))
        .await
        .assert_ok()
        .assert_see(&format!("accept-version-{}", v1.id))
        .assert_see(&format!("reject-version-{}", v1.id));

    // The review queue does not list it; the generic accept would refuse.
    t.app.acting_as(&w.rita);
    t.app
        .get("/documents/review")
        .await
        .assert_ok()
        .assert_dont_see("SOP-HSE-01");

    // The manager without the owner role is refused (422), the version
    // stays pending; a reject needs a reason.
    t.app.acting_as(&w.mona);
    t.app
        .htmx()
        .post(&format!("/documents/sop-versions/{}/accept", v1.id), &[])
        .await
        .assert_status(422);
    t.app.acting_as(&w.hana);
    t.app
        .htmx()
        .post(
            &format!("/documents/sop-versions/{}/reject", v1.id),
            &[("reason", " ")],
        )
        .await
        .assert_status(422);
    assert_eq!(
        Document::find_or_404(t.db(), v1.id).await.unwrap().status,
        DocumentStatus::PendingReview
    );

    // The owner accepts: the SOP is active with its review date.
    t.app
        .htmx()
        .post(&format!("/documents/sop-versions/{}/accept", v1.id), &[])
        .await
        .assert_ok();
    let sop = w.sop("SOP-HSE-01").await;
    assert_eq!(Sops::status(t.db(), &sop).await.unwrap(), SopStatus::Active);
    let accepted = Document::find_or_404(t.db(), v1.id).await.unwrap();
    let review = accepted.expiry_date.expect("a review date").to_string();
    t.app
        .get("/documents/sops?status=active")
        .await
        .assert_ok()
        .assert_see("SOP-HSE-01")
        .assert_see("Active")
        .assert_see(&review);
    t.app
        .get("/documents/sops?status=draft")
        .await
        .assert_ok()
        .assert_dont_see("SOP-HSE-01");

    // Its file opens for a viewer; a document of another subject does not.
    t.app.acting_as(&w.vera);
    t.app
        .get(&format!("/documents/sop-files/{}", v1.id))
        .await
        .assert_ok();
    t.app.acting_as(&w.dan);
    t.app
        .get(&format!("/documents/sop-files/{}", v1.id))
        .await
        .assert_forbidden();

    // A translation of the current version: uploaded by the manager,
    // decided by the owner; the page lists it.
    t.app.acting_as(&w.mona);
    t.app
        .htmx()
        .post_multipart(
            &format!("/documents/sop-versions/{}/translations", v1.id),
            &[("language", "ar")],
            &[("files", "ar.pdf", PDF)],
        )
        .await
        .assert_ok();
    let translation = Document::query()
        .where_(Document::SUBJECT_ID.eq(sop.id))
        .where_(Document::STATUS.eq(DocumentStatus::PendingReview))
        .first(t.db())
        .await
        .unwrap()
        .expect("the translation is stored");
    t.app.acting_as(&w.hana);
    t.app
        .get(&format!("/documents/sops/{}", sop.id))
        .await
        .assert_ok()
        .assert_see(&format!("accept-translation-{}", translation.id));
    t.app
        .htmx()
        .post(
            &format!("/documents/sop-translations/{}/accept", translation.id),
            &[],
        )
        .await
        .assert_ok();
    assert_eq!(
        Document::find_or_404(t.db(), translation.id)
            .await
            .unwrap()
            .status,
        DocumentStatus::Accepted
    );

    // Withdrawing needs `sops.manage`; the owner role is not needed.
    t.app.acting_as(&w.vera);
    t.app
        .htmx()
        .post(&format!("/documents/sops/{}/withdraw", sop.id), &[])
        .await
        .assert_forbidden();
    t.app.acting_as(&w.mona);
    t.app
        .get(&format!("/documents/sops/{}", sop.id))
        .await
        .assert_ok()
        .assert_see("withdraw-sop");
    t.app
        .htmx()
        .post(
            &format!("/documents/sops/{}/withdraw", sop.id),
            &[("reason", "replaced by a new procedure")],
        )
        .await
        .assert_ok();
    let sop = w.sop("SOP-HSE-01").await;
    assert_eq!(
        Sops::status(t.db(), &sop).await.unwrap(),
        SopStatus::Withdrawn
    );
    t.app
        .get("/documents/sops?status=withdrawn")
        .await
        .assert_ok()
        .assert_see("SOP-HSE-01");
    t.app
        .get(&format!("/documents/sops/{}", sop.id))
        .await
        .assert_ok()
        .assert_dont_see("withdraw-sop")
        .assert_dont_see("edit-readers");
}
