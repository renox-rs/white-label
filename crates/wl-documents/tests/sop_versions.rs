//! The SOP versions (#52, part 3/6): upload, accept with the review date and
//! supersede, reject, withdraw, the owner-role guard and the generic
//! transitions refusing SOP kinds. The acceptance checks AC1 to AC6 of #52.

mod support;

use renox::prelude::*;
use renox::testing::TestResponse;
use wl_documents::sops::versions::review_date;
use wl_documents::{
    Document, DocumentStatus, Sop, SopStatus, Sops, permissions as perm, sops::kind_seeds,
};
use wl_kernel::documents::{
    DocumentRejected, DocumentSuperseded, SopId, SopPublished, SopWithdrawn, today,
};

use support::{Booted, boot, residence_visa};

/// Boots with the SOP kinds, the owner role and the SOP "SOP-HSE-01" read by
/// "site_worker" in all sites.
struct World {
    t: Booted,
    /// Manages SOPs and holds the owner role (the HSE officer A).
    hana: User,
    /// Manages SOPs without the owner role.
    mona: User,
    sop: Sop,
}

async fn world() -> World {
    let t = boot().await;
    t.install(&kind_seeds()).await;
    t.roles(&["hse_officer"]).await;
    let hana = t.user("Hana", &[perm::SOPS_MANAGE]).await;
    hana.assign_role(t.db(), "hse_officer").await.unwrap();
    let mona = t.user("Mona", &[perm::SOPS_MANAGE]).await;
    t.app.acting_as(&mona);
    let created = t
        .app
        .post(
            "/test/sops",
            &[
                ("code", "SOP-HSE-01"),
                ("title", "Working at height"),
                ("owner_role", "hse_officer"),
            ],
        )
        .await;
    created.assert_ok();
    let sop = Sop::find_or_404(t.db(), created.text().parse().unwrap())
        .await
        .unwrap();
    t.app
        .post(
            &format!("/test/sops/{}/readers", sop.id),
            &[("readers", "site_worker@property.site:1")],
        )
        .await
        .assert_ok();
    World { t, hana, mona, sop }
}

impl World {
    /// `by` uploads a version; its document.
    async fn upload(&self, by: &User) -> Document {
        self.t.app.acting_as(by);
        let response = self
            .t
            .app
            .post(
                &format!("/test/sops/{}/versions", self.sop.id),
                &[("file_name", "working-at-height.pdf")],
            )
            .await;
        response.assert_ok();
        Document::find_or_404(self.t.db(), response.text().parse().unwrap())
            .await
            .unwrap()
    }

    async fn accept(&self, by: &User, version: &Document) -> TestResponse {
        self.t.app.acting_as(by);
        self.t
            .app
            .post(&format!("/test/sop-versions/{}/accept", version.id), &[])
            .await
    }

    async fn reject(&self, by: &User, version: &Document, reason: &str) -> TestResponse {
        self.t.app.acting_as(by);
        self.t
            .app
            .post(
                &format!("/test/sop-versions/{}/reject", version.id),
                &[("reason", reason)],
            )
            .await
    }

    async fn withdraw(&self, by: &User) -> TestResponse {
        self.t.app.acting_as(by);
        self.t
            .app
            .post(&format!("/test/sops/{}/withdraw", self.sop.id), &[])
            .await
    }

    async fn reload(&self, version: &Document) -> Document {
        self.t.reload(version).await
    }

    async fn sop(&self) -> Sop {
        Sop::find_or_404(self.t.db(), self.sop.id).await.unwrap()
    }

    async fn status(&self) -> SopStatus {
        Sops::status(self.t.db(), &self.sop().await).await.unwrap()
    }
}

/// AC1: accepting version 1 makes the SOP active, with the review date and
/// `SopPublished`; the uploader accepts their own version (AC4).
#[renox::test]
async fn the_owner_accepts_a_version_and_the_sop_is_published() {
    let w = world().await;
    assert_eq!(w.status().await, SopStatus::Draft);
    let v1 = w.upload(&w.hana).await;
    assert_eq!(v1.status, DocumentStatus::PendingReview);
    assert_eq!(v1.subject_type, "documents.sop");
    assert_eq!(Sops::version_number(w.t.db(), &v1).await.unwrap(), 1);
    assert_eq!(w.status().await, SopStatus::Draft);

    w.accept(&w.hana, &v1).await.assert_ok();
    let v1 = w.reload(&v1).await;
    assert_eq!(v1.status, DocumentStatus::Accepted);
    assert_eq!(v1.expiry_date, Some(review_date(today(), 12)));
    assert_eq!(w.sop().await.current_document_id, Some(v1.id));
    assert_eq!(w.status().await, SopStatus::Active);

    let published = w.t.app.emitted::<SopPublished>();
    assert_eq!(published.len(), 1);
    assert_eq!(published[0].sop, SopId(w.sop.id));
    assert_eq!(published[0].version, 1);
    assert_eq!(published[0].ack_due_days, 7);
    assert_eq!(published[0].readers.len(), 1);
    assert_eq!(published[0].readers[0].role, "site_worker");
    assert_eq!(published[0].document, v1.document_id());
}

/// The review date follows the SOP's own interval.
#[renox::test]
async fn the_review_date_uses_the_interval_of_the_sop() {
    let w = world().await;
    let mut sop = w.sop().await;
    sop.review_interval_months = 6;
    sop.save(w.t.db()).await.unwrap();
    let v1 = w.upload(&w.hana).await;
    w.accept(&w.hana, &v1).await.assert_ok();
    assert_eq!(
        w.reload(&v1).await.expiry_date,
        Some(review_date(today(), 6))
    );
}

/// AC2 and AC4: version 2 supersedes version 1 and is published again.
#[renox::test]
async fn a_new_version_supersedes_the_previous_one() {
    let w = world().await;
    let v1 = w.upload(&w.mona).await;
    w.accept(&w.hana, &v1).await.assert_ok();
    // Hana uploads and accepts her own version 2.
    let v2 = w.upload(&w.hana).await;
    assert_eq!(v2.uploaded_by, Some(w.hana.id));
    assert_eq!(Sops::version_number(w.t.db(), &v2).await.unwrap(), 2);
    w.accept(&w.hana, &v2).await.assert_ok();

    let v1 = w.reload(&v1).await;
    assert_eq!(v1.status, DocumentStatus::Superseded);
    assert_eq!(v1.superseded_by, Some(v2.id));
    assert_eq!(w.reload(&v2).await.status, DocumentStatus::Accepted);
    assert_eq!(w.sop().await.current_document_id, Some(v2.id));
    assert_eq!(w.status().await, SopStatus::Active);
    let published = w.t.app.emitted::<SopPublished>();
    assert_eq!(
        published.iter().map(|p| p.version).collect::<Vec<_>>(),
        [1, 2]
    );
    w.t.app
        .assert_emitted::<DocumentSuperseded>(|e| e.document == v1.document_id());
}

/// AC3: without the owner role, or with only `documents.review`, nobody
/// accepts or rejects a version; it stays pending.
#[renox::test]
async fn only_the_owner_role_accepts_or_rejects() {
    let w = world().await;
    let v1 = w.upload(&w.mona).await;
    let rita = w.t.reviewer().await;

    // sops.manage without the owner role: refused by the guard.
    let refused = w.accept(&w.mona, &v1).await;
    assert!(!refused.status.is_success());
    assert!(!w.reject(&w.mona, &v1, "no").await.status.is_success());
    // documents.review only: no sops.manage.
    w.accept(&rita, &v1).await.assert_status(403);
    w.reject(&rita, &v1, "no").await.assert_status(403);
    // The generic route of the review queue refuses an SOP too.
    w.t.app.acting_as(&rita);
    let queue =
        w.t.app
            .htmx()
            .post(&format!("/documents/{}/accept", v1.id), &[])
            .await;
    assert!(!queue.status.is_success());

    assert_eq!(w.reload(&v1).await.status, DocumentStatus::PendingReview);
    assert_eq!(w.status().await, SopStatus::Draft);
    w.t.app.assert_not_emitted::<SopPublished>();

    // A holder of the owner role without sops.manage is refused as well.
    let ola = w.t.user("Ola", &[perm::VIEW, perm::REVIEW]).await;
    ola.assign_role(w.t.db(), "hse_officer").await.unwrap();
    w.accept(&ola, &v1).await.assert_status(403);
    assert_eq!(w.reload(&v1).await.status, DocumentStatus::PendingReview);
}

/// The generic transitions refuse the SOP kinds even for `documents.manage`
/// and `sops.manage`; the SOP transitions refuse other documents.
#[renox::test]
async fn the_generic_transitions_refuse_sop_kinds() {
    let w = world().await;
    let v1 = w.upload(&w.hana).await;
    w.accept(&w.hana, &v1).await.assert_ok();
    let omar =
        w.t.user("Omar", &[perm::VIEW, perm::MANAGE, perm::REVIEW])
            .await;
    for user in [&omar, &w.hana] {
        w.t.app.acting_as(user);
        let archived =
            w.t.app
                .post(&format!("/test/documents/{}/archive", v1.id), &[])
                .await;
        assert!(!archived.status.is_success());
    }
    assert_eq!(w.reload(&v1).await.status, DocumentStatus::Accepted);
    assert_eq!(w.status().await, SopStatus::Active);

    // Directly through the service, too.
    let v2 = w.upload(&w.hana).await;
    for user in [&w.hana, &w.mona] {
        w.t.app.acting_as(user);
        let path = format!("/test/documents/{}", v2.id);
        let accepted = w.t.app.post(&format!("{path}/accept"), &[]).await;
        assert!(!accepted.status.is_success());
        let rejected =
            w.t.app
                .post(&format!("{path}/reject"), &[("reason", "x")])
                .await;
        assert!(!rejected.status.is_success());
    }
    assert_eq!(w.reload(&v2).await.status, DocumentStatus::PendingReview);

    // An ordinary document is not an SOP version.
    w.t.install(&[residence_visa()]).await;
    let visa =
        w.t.attach(support::ravi(), "residence_visa", Some(support::day(300)))
            .await;
    let result = w.accept(&w.hana, &visa).await;
    assert!(!result.status.is_success());
    assert_eq!(w.reload(&visa).await.status, DocumentStatus::PendingReview);
}

/// AC5: only `sops.manage` withdraws; the SOP reads `withdrawn`, emits
/// `SopWithdrawn` and is active again once a new version is accepted.
#[renox::test]
async fn withdrawing_archives_the_current_version() {
    let w = world().await;
    let v1 = w.upload(&w.hana).await;
    w.accept(&w.hana, &v1).await.assert_ok();
    let v2 = w.upload(&w.hana).await;
    w.accept(&w.hana, &v2).await.assert_ok();

    let omar = w.t.user("Omar", &[perm::VIEW, perm::MANAGE]).await;
    w.withdraw(&omar).await.assert_status(403);
    assert_eq!(w.status().await, SopStatus::Active);
    w.t.app.assert_not_emitted::<SopWithdrawn>();

    // The owner role is not needed to withdraw.
    w.withdraw(&w.mona).await.assert_ok();
    assert_eq!(w.reload(&v2).await.status, DocumentStatus::Archived);
    assert_eq!(w.sop().await.current_document_id, Some(v2.id));
    assert_eq!(w.status().await, SopStatus::Withdrawn);
    let withdrawn = w.t.app.emitted::<SopWithdrawn>();
    assert_eq!(withdrawn.len(), 1);
    assert_eq!(withdrawn[0].sop, SopId(w.sop.id));
    assert_eq!(withdrawn[0].version, 2);
    assert!(withdrawn[0].cancelled_acknowledgements.is_empty());

    // Nothing left to withdraw.
    assert!(!w.withdraw(&w.mona).await.status.is_success());
    assert_eq!(w.t.app.emitted::<SopWithdrawn>().len(), 1);

    // A new accepted version makes it active again.
    let v3 = w.upload(&w.mona).await;
    w.accept(&w.hana, &v3).await.assert_ok();
    assert_eq!(w.status().await, SopStatus::Active);
    assert_eq!(w.sop().await.current_document_id, Some(v3.id));
}

/// A draft SOP has nothing to withdraw.
#[renox::test]
async fn a_draft_sop_cannot_be_withdrawn() {
    let w = world().await;
    assert!(!w.withdraw(&w.mona).await.status.is_success());
    w.upload(&w.hana).await;
    assert!(!w.withdraw(&w.mona).await.status.is_success());
    w.t.app.assert_not_emitted::<SopWithdrawn>();
}

/// AC6: a rejected version leaves the SOP where it was.
#[renox::test]
async fn a_rejected_version_leaves_the_current_version() {
    let w = world().await;
    let v1 = w.upload(&w.hana).await;
    w.reject(&w.hana, &v1, "").await.assert_status(422);
    w.reject(&w.hana, &v1, "Wrong template").await.assert_ok();
    let v1 = w.reload(&v1).await;
    assert_eq!(v1.status, DocumentStatus::Rejected);
    assert_eq!(v1.rejection_reason.as_deref(), Some("Wrong template"));
    assert_eq!(w.status().await, SopStatus::Draft);
    assert!(w.sop().await.current_document_id.is_none());
    w.t.app
        .assert_emitted::<DocumentRejected>(|e| e.reason == "Wrong template");

    // With a current version: a rejected version 3 changes nothing.
    let v2 = w.upload(&w.hana).await;
    w.accept(&w.hana, &v2).await.assert_ok();
    let v3 = w.upload(&w.hana).await;
    w.reject(&w.hana, &v3, "Outdated").await.assert_ok();
    assert_eq!(w.reload(&v2).await.status, DocumentStatus::Accepted);
    assert_eq!(w.sop().await.current_document_id, Some(v2.id));
    assert_eq!(w.status().await, SopStatus::Active);
    assert_eq!(w.t.app.emitted::<SopPublished>().len(), 1);
}

/// Uploading a version needs `sops.manage` and the SOP kinds.
#[renox::test]
async fn only_a_sops_manager_uploads_a_version() {
    let w = world().await;
    let omar =
        w.t.user("Omar", &[perm::VIEW, perm::UPLOAD, perm::MANAGE])
            .await;
    w.t.app.acting_as(&omar);
    w.t.app
        .post(
            &format!("/test/sops/{}/versions", w.sop.id),
            &[("file_name", "a.pdf")],
        )
        .await
        .assert_status(403);
    assert_eq!(
        Document::query().count(w.t.db()).await.unwrap(),
        0,
        "nothing is stored"
    );
}
