//! SOP translations and the review (#52, part 4/6): translation upload and
//! accept without new events, supersede with the version, the owner-role
//! reminders of `DOC.RUL.01` and the review state. AC7 and AC8 of #52.

mod support;

use renox::chrono::Duration;
use renox::prelude::*;
use renox::testing::TestResponse;
use wl_documents::expiry;
use wl_documents::sops::review::{ReviewState, review_state};
use wl_documents::{
    Document, DocumentStatus, Sop, SopStatus, Sops, permissions as perm, sops::kind_seeds,
};
use wl_kernel::documents::{
    DocumentAccepted, DocumentExpired, DocumentExpiring, DocumentRejected, DocumentSuperseded,
    SopPublished, today,
};

use support::{Booted, boot};

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

impl World {
    async fn translate(&self, by: &User, version: &Document, language: &str) -> TestResponse {
        self.t.app.acting_as(by);
        self.t
            .app
            .post(
                &format!("/test/sop-versions/{}/translations", version.id),
                &[("language", language)],
            )
            .await
    }

    async fn translation(&self, by: &User, version: &Document, language: &str) -> Document {
        let response = self.translate(by, version, language).await;
        response.assert_ok();
        Document::find_or_404(self.t.db(), response.text().parse().unwrap())
            .await
            .unwrap()
    }

    async fn accept_translation(&self, by: &User, translation: &Document) -> TestResponse {
        self.t.app.acting_as(by);
        self.t
            .app
            .post(
                &format!("/test/sop-translations/{}/accept", translation.id),
                &[],
            )
            .await
    }

    /// Uploads and accepts a version as Hana.
    async fn published(&self) -> Document {
        let version = self.upload(&self.hana).await;
        self.accept(&self.hana, &version).await.assert_ok();
        self.reload(&version).await
    }

    async fn review(&self, on: renox::chrono::NaiveDate) -> ReviewState {
        Sops::review_state(self.t.db(), &self.sop().await, on)
            .await
            .unwrap()
    }
}

/// AC8: an Arabic translation accepted after publication emits no
/// `SopPublished`, only `DocumentAccepted`; a new version supersedes it.
#[renox::test]
async fn a_translation_is_accepted_without_a_new_publication_and_follows_its_version() {
    let w = world().await;
    let v1 = w.published().await;
    assert_eq!(w.t.app.emitted::<SopPublished>().len(), 1);

    let ar = w.translation(&w.mona, &v1, " AR ").await;
    assert_eq!(ar.status, DocumentStatus::PendingReview);
    assert_eq!(ar.subject_type, "documents.sop");
    let link = Sops::translation_of(w.t.db(), ar.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (link.language.as_str(), link.version_document_id),
        ("ar", v1.id)
    );
    assert_eq!(Sops::translations(w.t.db(), v1.id).await.unwrap().len(), 1);

    w.accept_translation(&w.hana, &ar).await.assert_ok();
    let ar = w.reload(&ar).await;
    assert_eq!(ar.status, DocumentStatus::Accepted);
    assert_eq!(ar.expiry_date, None);
    assert_eq!(
        w.t.app.emitted::<SopPublished>().len(),
        1,
        "no new publication"
    );
    w.t.app.assert_emitted::<DocumentAccepted>(|e| {
        e.document == ar.document_id() && e.kind == "sop_translation"
    });
    // The translation is not the SOP's current version.
    assert_eq!(w.sop().await.current_document_id, Some(v1.id));
    assert_eq!(w.status().await, SopStatus::Active);
    assert_eq!(w.reload(&v1).await.status, DocumentStatus::Accepted);

    // Version 2 supersedes version 1 and its translation.
    let v2 = w.upload(&w.hana).await;
    w.accept(&w.hana, &v2).await.assert_ok();
    assert_eq!(w.reload(&v1).await.status, DocumentStatus::Superseded);
    let ar = w.reload(&ar).await;
    assert_eq!(ar.status, DocumentStatus::Superseded);
    assert_eq!(ar.superseded_by, Some(v2.id));
    w.t.app
        .assert_emitted::<DocumentSuperseded>(|e| e.document == ar.document_id());
    assert_eq!(w.t.app.emitted::<SopPublished>().len(), 2);
}

/// Another language's translation survives; a newer translation in the same
/// language replaces the old one.
#[renox::test]
async fn a_translation_replaces_only_the_same_language() {
    let w = world().await;
    let v1 = w.published().await;
    let ar1 = w.translation(&w.hana, &v1, "ar").await;
    let ur = w.translation(&w.hana, &v1, "ur").await;
    for t in [&ar1, &ur] {
        w.accept_translation(&w.hana, t).await.assert_ok();
    }
    let ar2 = w.translation(&w.hana, &v1, "ar").await;
    w.accept_translation(&w.hana, &ar2).await.assert_ok();
    assert_eq!(w.reload(&ar1).await.status, DocumentStatus::Superseded);
    assert_eq!(w.reload(&ar1).await.superseded_by, Some(ar2.id));
    assert_eq!(w.reload(&ur).await.status, DocumentStatus::Accepted);
    assert_eq!(w.reload(&ar2).await.status, DocumentStatus::Accepted);
    // The version itself is untouched.
    assert_eq!(w.reload(&v1).await.status, DocumentStatus::Accepted);
}

/// Owner role and `sops.manage` as for versions; a translation of a version
/// that is not the current one is not accepted; a blank language and a
/// non-version target are refused.
#[renox::test]
async fn translations_follow_the_rules_of_versions() {
    let w = world().await;
    let v1 = w.upload(&w.hana).await;
    // Pending version: the translation can be uploaded, not accepted yet.
    let ar = w.translation(&w.hana, &v1, "ar").await;
    assert!(!w.accept_translation(&w.hana, &ar).await.status.is_success());
    w.accept(&w.hana, &v1).await.assert_ok();

    assert!(!w.accept_translation(&w.mona, &ar).await.status.is_success());
    let rita = w.t.reviewer().await;
    w.accept_translation(&rita, &ar).await.assert_status(403);
    w.translate(&rita, &v1, "ar").await.assert_status(403);
    assert_eq!(w.reload(&ar).await.status, DocumentStatus::PendingReview);

    w.translate(&w.hana, &v1, " ").await.assert_status(422);
    // A translation is no target for a translation.
    w.translate(&w.hana, &ar, "ur").await.assert_status(422);

    // Rejecting: owner role and a reason needed; DocumentRejected only.
    let reject = format!("/test/sop-translations/{}/reject", ar.id);
    w.t.app.acting_as(&w.mona);
    assert!(
        !w.t.app
            .post(&reject, &[("reason", "x")])
            .await
            .status
            .is_success()
    );
    w.t.app.acting_as(&w.hana);
    w.t.app
        .post(&reject, &[("reason", "")])
        .await
        .assert_status(422);
    w.t.app
        .post(&reject, &[("reason", "Wrong text")])
        .await
        .assert_ok();
    assert_eq!(w.reload(&ar).await.status, DocumentStatus::Rejected);
    w.t.app.assert_emitted::<DocumentRejected>(|e| {
        e.kind == "sop_translation" && e.reason == "Wrong text"
    });
    assert_eq!(w.t.app.emitted::<SopPublished>().len(), 1);

    // After version 2, a translation of the old version is not accepted.
    let ur = w.translation(&w.hana, &v1, "ur").await;
    let v2 = w.upload(&w.hana).await;
    w.accept(&w.hana, &v2).await.assert_ok();
    assert!(!w.accept_translation(&w.hana, &ur).await.status.is_success());
    assert_eq!(w.reload(&ur).await.status, DocumentStatus::PendingReview);
    // The superseded version takes no new translation.
    w.translate(&w.hana, &v1, "ar").await.assert_status(422);
}

/// AC7: 30 days before the review date the HSE officers get the reminder;
/// the day after it the SOP reads review overdue, stays active, and nothing
/// expires or blocks.
#[renox::test]
async fn the_owner_role_is_reminded_and_a_passed_review_date_only_reads_overdue() {
    let w = world().await;
    let v1 = w.published().await;
    let review = v1.expiry_date.unwrap();
    let ola = w.t.staff("Ola", "hse_officer", None).await;
    // Not the owner role: a coordinator hears nothing.
    w.t.roles(&["coordinator"]).await;
    let cora = w.t.staff("Cora", "coordinator", None).await;
    w.t.notices.clear();

    let before = review - Duration::days(31);
    let report = expiry::run(w.t.state(), before).await.unwrap();
    assert_eq!(report.messages, 0, "{report:?}");
    assert_eq!(w.review(before).await, ReviewState::Ok);

    let due = review - Duration::days(30);
    let report = expiry::run(w.t.state(), due).await.unwrap();
    assert_eq!(
        (report.events, report.messages, report.failed),
        (1, 2, 0),
        "{report:?}"
    );
    assert_eq!(w.t.notices.to_user(w.hana.id).len(), 1);
    assert_eq!(w.t.notices.to_user(ola.id).len(), 1);
    assert!(w.t.notices.to_user(cora.id).is_empty());
    assert!(w.t.notices.to_user(w.mona.id).is_empty());
    assert_eq!(
        w.t.notices.to_user(w.hana.id)[0].type_key,
        expiry::run::NOTIFICATION_TYPE
    );
    assert_eq!(w.t.app.emitted::<DocumentExpiring>().len(), 1);
    assert_eq!(w.review(due).await, ReviewState::Due);

    // The review date itself is still due; the day after is overdue.
    expiry::run(w.t.state(), review).await.unwrap();
    assert_eq!(w.review(review).await, ReviewState::Due);
    let after = review + Duration::days(1);
    let report = expiry::run(w.t.state(), after).await.unwrap();
    assert_eq!(
        (report.expired, report.expired_events),
        (0, 0),
        "{report:?}"
    );
    assert!(w.t.app.emitted::<DocumentExpired>().is_empty());
    assert_eq!(w.review(after).await, ReviewState::Overdue);
    assert_eq!(review_state(Some(review), after), ReviewState::Overdue);
    assert_eq!(w.reload(&v1).await.status, DocumentStatus::Accepted);
    assert_eq!(w.status().await, SopStatus::Active);
}

/// The review state of a draft or withdrawn SOP is "no review date".
#[renox::test]
async fn a_draft_or_withdrawn_sop_has_no_review_date() {
    let w = world().await;
    let day = today();
    assert_eq!(w.review(day).await, ReviewState::NoReviewDate);
    w.published().await;
    assert_eq!(w.review(day).await, ReviewState::Ok);
    w.withdraw(&w.mona).await.assert_ok();
    assert_eq!(w.review(day).await, ReviewState::NoReviewDate);
}
