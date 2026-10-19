//! Acceptance checks of #52 (part 6/6): one walk through the life of the SOP
//! "SOP-HSE-01 Working at height" (AC1 to AC9) and the pinned absence of
//! blocks (AC7). The parts have their own focused files (`sop_service.rs`,
//! `sop_versions.rs`, `sop_translations.rs`); this one checks that they work
//! together.

mod support;

use renox::chrono::Duration;
use renox::prelude::*;
use renox::testing::TestResponse;
use wl_documents::kinds::KindBlock;
use wl_documents::sops::review::ReviewState;
use wl_documents::sops::versions::review_date;
use wl_documents::{
    Document, DocumentKind, DocumentStatus, Sop, SopStatus, Sops, expiry, permissions as perm,
    sops::kind_seeds,
};
use wl_kernel::documents::{
    DocumentExpired, SopId, SopPublished, SopReadersChanged, SopWithdrawn, today,
};

use support::{Booted, boot};

struct World {
    t: Booted,
    /// Manages SOPs and holds the owner role (HSE officer A).
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
            &[("readers", "site_worker@sites:1")],
        )
        .await
        .assert_ok();
    World { t, hana, mona, sop }
}

impl World {
    async fn post(&self, by: &User, path: &str, form: &[(&str, &str)]) -> TestResponse {
        self.t.app.acting_as(by);
        self.t.app.post(path, form).await
    }

    async fn upload(&self, by: &User) -> Document {
        let response = self
            .post(
                by,
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
        self.post(
            by,
            &format!("/test/sop-versions/{}/accept", version.id),
            &[],
        )
        .await
    }

    async fn reject(&self, by: &User, version: &Document, reason: &str) -> TestResponse {
        self.post(
            by,
            &format!("/test/sop-versions/{}/reject", version.id),
            &[("reason", reason)],
        )
        .await
    }

    async fn status(&self) -> SopStatus {
        let sop = Sop::find_or_404(self.t.db(), self.sop.id).await.unwrap();
        Sops::status(self.t.db(), &sop).await.unwrap()
    }

    async fn current(&self) -> Option<i64> {
        Sop::find_or_404(self.t.db(), self.sop.id)
            .await
            .unwrap()
            .current_document_id
    }
}

/// AC1, AC2, AC5, AC6 and AC8 in the order a real SOP lives them.
#[renox::test]
async fn an_sop_lives_through_versions_withdrawal_and_republication() {
    let w = world().await;
    assert_eq!(w.status().await, SopStatus::Draft);

    // A rejected first version: still a draft, nothing published (AC6).
    let first = w.upload(&w.mona).await;
    w.reject(&w.hana, &first, "Wrong template")
        .await
        .assert_ok();
    assert_eq!(w.status().await, SopStatus::Draft);
    assert_eq!(w.current().await, None);
    w.t.app.assert_not_emitted::<SopPublished>();

    // The next version is accepted by the owner role (AC1).
    let v1 = w.upload(&w.hana).await;
    w.accept(&w.hana, &v1).await.assert_ok();
    let v1 = w.t.reload(&v1).await;
    assert_eq!(v1.expiry_date, Some(review_date(today(), 12)));
    assert_eq!(w.status().await, SopStatus::Active);
    let published = w.t.app.emitted::<SopPublished>();
    assert_eq!(published.len(), 1);
    assert_eq!(published[0].readers[0].role, "site_worker");
    assert_eq!(published[0].readers[0].scope_type, "sites");
    assert_eq!(published[0].readers[0].scope_id, "1");

    // An Arabic translation accepted afterwards publishes nothing (AC8).
    let ar = w
        .post(
            &w.mona,
            &format!("/test/sop-versions/{}/translations", v1.id),
            &[("language", "ar")],
        )
        .await;
    ar.assert_ok();
    let ar: i64 = ar.text().parse().unwrap();
    w.post(&w.hana, &format!("/test/sop-translations/{ar}/accept"), &[])
        .await
        .assert_ok();
    assert_eq!(w.t.app.emitted::<SopPublished>().len(), 1);

    // The next version supersedes the version and its translation (AC2, AC8).
    let v2 = w.upload(&w.mona).await;
    w.accept(&w.hana, &v2).await.assert_ok();
    assert_eq!(w.t.reload(&v1).await.status, DocumentStatus::Superseded);
    let ar = Document::find_or_404(w.t.db(), ar).await.unwrap();
    assert_eq!(ar.status, DocumentStatus::Superseded);
    assert_eq!(w.current().await, Some(v2.id));
    assert_eq!(w.t.app.emitted::<SopPublished>().len(), 2);

    // A rejected later version changes nothing (AC6).
    let v3 = w.upload(&w.hana).await;
    w.reject(&w.hana, &v3, "Outdated").await.assert_ok();
    assert_eq!(w.current().await, Some(v2.id));
    assert_eq!(w.status().await, SopStatus::Active);

    // Withdrawing needs `sops.manage` (AC5).
    let omar = w.t.user("Omar", &[perm::VIEW, perm::MANAGE]).await;
    w.post(&omar, &format!("/test/sops/{}/withdraw", w.sop.id), &[])
        .await
        .assert_status(403);
    assert_eq!(w.status().await, SopStatus::Active);
    w.post(&w.mona, &format!("/test/sops/{}/withdraw", w.sop.id), &[])
        .await
        .assert_ok();
    assert_eq!(w.status().await, SopStatus::Withdrawn);
    let withdrawn = w.t.app.emitted::<SopWithdrawn>();
    assert_eq!(withdrawn.len(), 1);
    assert_eq!(withdrawn[0].version, 3);

    // A withdrawn SOP refuses reader changes; a new version reactivates it.
    w.post(
        &w.mona,
        &format!("/test/sops/{}/readers", w.sop.id),
        &[("readers", "driver")],
    )
    .await
    .assert_status(422);
    let v4 = w.upload(&w.hana).await;
    w.accept(&w.hana, &v4).await.assert_ok();
    assert_eq!(w.status().await, SopStatus::Active);
    assert_eq!(w.t.app.emitted::<SopPublished>().len(), 3);
}

/// AC3 and AC4 side by side: the version stays pending for everyone but the
/// owner role, and the uploader may decide on their own version.
#[renox::test]
async fn only_the_owner_role_decides_and_the_uploader_may() {
    let w = world().await;
    let rita = w.t.reviewer().await;
    let version = w.upload(&w.hana).await;
    assert!(!w.accept(&w.mona, &version).await.status.is_success());
    assert!(!w.reject(&w.mona, &version, "no").await.status.is_success());
    w.accept(&rita, &version).await.assert_status(403);
    w.reject(&rita, &version, "no").await.assert_status(403);
    assert_eq!(
        w.t.reload(&version).await.status,
        DocumentStatus::PendingReview
    );
    // Hana uploaded it and holds the owner role.
    w.accept(&w.hana, &version).await.assert_ok();
    assert_eq!(w.t.reload(&version).await.status, DocumentStatus::Accepted);
}

/// AC9 on the walk: the difference is announced for an active SOP only.
#[renox::test]
async fn removing_a_reader_of_an_active_sop_is_announced() {
    let w = world().await;
    let readers = format!("/test/sops/{}/readers", w.sop.id);
    // Draft: stored, nothing announced.
    w.post(&w.mona, &readers, &[("readers", "driver")])
        .await
        .assert_ok();
    w.post(&w.mona, &readers, &[("readers", "site_worker@sites:1")])
        .await
        .assert_ok();
    w.t.app.assert_not_emitted::<SopReadersChanged>();
    let v1 = w.upload(&w.hana).await;
    w.accept(&w.hana, &v1).await.assert_ok();

    // The same readers: nothing; removing "site worker": announced.
    w.post(&w.mona, &readers, &[("readers", "site_worker@sites:1")])
        .await
        .assert_ok();
    w.t.app.assert_not_emitted::<SopReadersChanged>();
    w.post(&w.mona, &readers, &[("readers", "driver")])
        .await
        .assert_ok();
    let changed = w.t.app.emitted::<SopReadersChanged>();
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].sop, SopId(w.sop.id));
    assert_eq!(changed[0].removed[0].role, "site_worker");
    assert_eq!(changed[0].added[0].role, "driver");
}

/// AC7: the review date reads overdue; no SOP kind blocks, so nothing can
/// expire or refuse because a review date passed.
#[renox::test]
async fn a_passed_review_date_is_overdue_and_blocks_nothing() {
    let w = world().await;
    for key in ["sop", "sop_translation"] {
        let kind = DocumentKind::by_key(w.t.db(), key)
            .await
            .unwrap()
            .expect("the kind is installed");
        assert!(kind.blocks(w.t.db()).await.unwrap().is_empty());
        assert!(!kind.legal_block && !kind.compliance_critical);
    }
    assert_eq!(KindBlock::query().count(w.t.db()).await.unwrap(), 0);

    let v1 = w.upload(&w.hana).await;
    w.accept(&w.hana, &v1).await.assert_ok();
    let review = w.t.reload(&v1).await.expiry_date.unwrap();
    let after = review + Duration::days(1);
    expiry::run(w.t.state(), after).await.unwrap();
    assert!(w.t.app.emitted::<DocumentExpired>().is_empty());
    let sop = Sop::find_or_404(w.t.db(), w.sop.id).await.unwrap();
    assert_eq!(
        Sops::review_state(w.t.db(), &sop, after).await.unwrap(),
        ReviewState::Overdue
    );
    assert_eq!(w.status().await, SopStatus::Active);
}
