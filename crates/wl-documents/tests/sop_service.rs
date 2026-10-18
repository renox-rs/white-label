//! The SOP service (#52, part 2/6): unique codes, required readers with
//! `SopReadersChanged`, `Sops::required_for` and the derived status. The
//! acceptance checks AC9 to AC11 of #52.

mod support;

use renox::auth::permissions::Scope;
use renox::prelude::*;
use wl_documents::{
    Document, DocumentStatus, Documents, Sop, SopReader, SopStatus, Sops, permissions as perm,
    sops::kind_seeds,
};
use wl_kernel::documents::{SopId, SopReaderRef, SopReadersChanged};

use support::{Booted, boot, day};

/// The manager of SOPs.
async fn manager(t: &Booted) -> User {
    t.user("Mona", &[perm::SOPS_MANAGE]).await
}

/// Creates the SOP `code` through the test route as `user`.
async fn create(t: &Booted, user: &User, code: &str, entity: i64) -> renox::testing::TestResponse {
    t.app.acting_as(user);
    t.app
        .post(
            "/test/sops",
            &[
                ("code", code),
                ("title", "Working at height"),
                ("owner_role", "hse_officer"),
                ("entity_id", &entity.to_string()),
            ],
        )
        .await
}

/// Sets the readers of `sop` through the test route as `user`.
async fn set_readers(
    t: &Booted,
    user: &User,
    sop: i64,
    readers: &str,
) -> renox::testing::TestResponse {
    t.app.acting_as(user);
    t.app
        .post(
            &format!("/test/sops/{sop}/readers"),
            &[("readers", readers)],
        )
        .await
}

/// A new SOP as a row, `draft`.
async fn new_sop(t: &Booted, mona: &User, code: &str) -> Sop {
    let id = create(t, mona, code, 0).await;
    id.assert_ok();
    Sop::find_or_404(t.db(), id.text().parse().unwrap())
        .await
        .unwrap()
}

/// Makes `sop` active: a version document accepted and current.
async fn publish(t: &Booted, rita: &User, sop: &Sop) -> Document {
    t.install(&kind_seeds()).await;
    let version = t.attach(sop.subject(), "sop", Some(day(365))).await;
    let version = t.accept(rita, &version).await;
    assert_eq!(version.status, DocumentStatus::Accepted);
    let mut row = Sop::find_or_404(t.db(), sop.id).await.unwrap();
    row.current_document_id = Some(version.id);
    row.save(t.db()).await.unwrap();
    version
}

fn reader(role: &str, scope: Option<(&str, &str)>) -> SopReaderRef {
    SopReaderRef {
        all_staff: false,
        role: role.into(),
        scope_type: scope.map_or("", |s| s.0).into(),
        scope_id: scope.map_or("", |s| s.1).into(),
    }
}

async fn status(t: &Booted, sop: &Sop) -> SopStatus {
    let row = Sop::find_or_404(t.db(), sop.id).await.unwrap();
    Sops::status(t.db(), &row).await.unwrap()
}

/// AC10: the code is unique per legal entity; the pair decides.
#[renox::test]
async fn the_code_is_unique_per_legal_entity() {
    let t = boot().await;
    let mona = manager(&t).await;
    create(&t, &mona, "SOP-HSE-01", 0).await.assert_ok();
    create(&t, &mona, "SOP-HSE-01", 0).await.assert_status(422);
    create(&t, &mona, "SOP-HSE-01", 1).await.assert_ok();
    create(&t, &mona, "SOP-HSE-01", 2).await.assert_ok();
    create(&t, &mona, "SOP-HSE-01", 1).await.assert_status(422);
    assert_eq!(Sop::query().count(t.db()).await.unwrap(), 3);
}

/// Creating, updating and the readers need `sops.manage`.
#[renox::test]
async fn only_a_sops_manager_creates_updates_and_sets_readers() {
    let t = boot().await;
    let mona = manager(&t).await;
    let other = t
        .user("Omar", &[perm::VIEW, perm::MANAGE, perm::REVIEW])
        .await;
    create(&t, &other, "SOP-1", 0).await.assert_status(403);
    let sop = new_sop(&t, &mona, "SOP-1").await;
    assert_eq!(sop.owner_role, "hse_officer");
    assert_eq!((sop.review_interval_months, sop.ack_due_days), (12, 7));
    assert!(sop.current_document_id.is_none());

    t.app.acting_as(&other);
    t.app
        .post(
            &format!("/test/sops/{}", sop.id),
            &[("code", "SOP-2"), ("title", "T"), ("owner_role", "x")],
        )
        .await
        .assert_status(403);
    set_readers(&t, &other, sop.id, "all")
        .await
        .assert_status(403);

    t.app.acting_as(&mona);
    t.app
        .post(
            &format!("/test/sops/{}", sop.id),
            &[("code", "SOP-2"), ("title", "T"), ("owner_role", "x")],
        )
        .await
        .assert_ok();
    let row = Sop::find_or_404(t.db(), sop.id).await.unwrap();
    assert_eq!((row.code.as_str(), row.owner_role.as_str()), ("SOP-2", "x"));
    // Keeping its own code is not a clash; another SOP's code is.
    let second = new_sop(&t, &mona, "SOP-3").await;
    t.app
        .post(
            &format!("/test/sops/{}", second.id),
            &[("code", "SOP-2"), ("title", "T"), ("owner_role", "x")],
        )
        .await
        .assert_status(422);
    t.app
        .post(
            &format!("/test/sops/{}", second.id),
            &[("code", "SOP-3"), ("title", "T2"), ("owner_role", "x")],
        )
        .await
        .assert_ok();
}

/// A blank code is refused.
#[renox::test]
async fn a_blank_code_is_refused() {
    let t = boot().await;
    create(&t, &manager(&t).await, "  ", 0)
        .await
        .assert_status(422);
}

/// The status is derived: draft, active, withdrawn.
#[renox::test]
async fn the_status_is_derived_from_the_current_version() {
    let t = boot().await;
    let mona = manager(&t).await;
    let sop = new_sop(&t, &mona, "SOP-HSE-01").await;
    assert_eq!(status(&t, &sop).await, SopStatus::Draft);

    let rita = t.reviewer().await;
    let version = publish(&t, &rita, &sop).await;
    assert_eq!(status(&t, &sop).await, SopStatus::Active);

    let omar = t.user("Omar", &[perm::MANAGE]).await;
    t.app.acting_as(&omar);
    t.app
        .post(&format!("/test/documents/{}/archive", version.id), &[])
        .await
        .assert_ok();
    assert_eq!(status(&t, &sop).await, SopStatus::Withdrawn);
    let listed = Sops::list(t.db(), Some(SopStatus::Withdrawn))
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert!(
        Sops::list(t.db(), Some(SopStatus::Active))
            .await
            .unwrap()
            .is_empty()
    );
}

/// AC9: removing a reader of an active SOP is announced; saving the same
/// readers is not; a draft announces nothing; a withdrawn SOP refuses.
#[renox::test]
async fn changing_readers_emits_the_difference_for_an_active_sop_only() {
    let t = boot().await;
    let mona = manager(&t).await;
    let sop = new_sop(&t, &mona, "SOP-HSE-01").await;

    // Draft: stored, nothing emitted.
    set_readers(&t, &mona, sop.id, "site_worker@sites:1;driver")
        .await
        .assert_ok();
    t.app.assert_not_emitted::<SopReadersChanged>();
    assert_eq!(SopReader::query().count(t.db()).await.unwrap(), 2);

    let rita = t.reviewer().await;
    let version = publish(&t, &rita, &sop).await;

    // The same readers (in another order, with a duplicate): nothing.
    set_readers(&t, &mona, sop.id, "driver; site_worker@sites:1;driver")
        .await
        .assert_ok();
    t.app.assert_not_emitted::<SopReadersChanged>();

    // Removing "driver" and adding "all": one event.
    set_readers(&t, &mona, sop.id, "site_worker@sites:1;all")
        .await
        .assert_ok();
    let all = SopReaderRef {
        all_staff: true,
        role: String::new(),
        scope_type: String::new(),
        scope_id: String::new(),
    };
    t.app.assert_emitted::<SopReadersChanged>(|e| {
        e.sop == SopId(sop.id)
            && e.removed == vec![reader("driver", None)]
            && e.added == vec![all.clone()]
    });
    assert_eq!(t.app.emitted::<SopReadersChanged>().len(), 1);

    // A changed scope is removed plus added.
    set_readers(&t, &mona, sop.id, "site_worker@sites:2;all")
        .await
        .assert_ok();
    let events = t.app.emitted::<SopReadersChanged>();
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[1].removed,
        vec![reader("site_worker", Some(("sites", "1")))]
    );
    assert_eq!(
        events[1].added,
        vec![reader("site_worker", Some(("sites", "2")))]
    );
    assert_eq!(
        Sops::readers(t.db(), sop.id).await.unwrap().len(),
        2,
        "the stored rows follow the set"
    );

    // A bad reader is refused and changes nothing.
    set_readers(&t, &mona, sop.id, "@sites:1")
        .await
        .assert_status(422);
    set_readers(&t, &mona, sop.id, "worker@sites")
        .await
        .assert_status(422);
    assert_eq!(Sops::readers(t.db(), sop.id).await.unwrap().len(), 2);

    // Withdrawn: refused.
    let omar = t.user("Omar", &[perm::MANAGE]).await;
    t.app.acting_as(&omar);
    t.app
        .post(&format!("/test/documents/{}/archive", version.id), &[])
        .await
        .assert_ok();
    set_readers(&t, &mona, sop.id, "driver")
        .await
        .assert_status(422);
    assert_eq!(t.app.emitted::<SopReadersChanged>().len(), 2);
}

/// AC11: `required_for` returns the active SOPs that list the role in that
/// scope, and the all-staff ones.
#[renox::test]
async fn required_for_lists_the_active_sops_of_a_role_in_a_scope() {
    let t = boot().await;
    let mona = manager(&t).await;
    let site1 = Scope::new("sites", 1);
    let site2 = Scope::new("sites", 2);

    let height = new_sop(&t, &mona, "SOP-HSE-01").await;
    let fire = new_sop(&t, &mona, "SOP-HSE-02").await;
    let everyone = new_sop(&t, &mona, "SOP-HR-01").await;
    let draft = new_sop(&t, &mona, "SOP-HSE-03").await;
    set_readers(&t, &mona, height.id, "site_worker@sites:1")
        .await
        .assert_ok();
    set_readers(&t, &mona, fire.id, "site_worker")
        .await
        .assert_ok();
    set_readers(&t, &mona, everyone.id, "all").await.assert_ok();
    set_readers(&t, &mona, draft.id, "site_worker")
        .await
        .assert_ok();

    // Nothing is required while every SOP is a draft.
    assert!(
        Sops::required_for(t.db(), "site_worker", &site1)
            .await
            .unwrap()
            .is_empty()
    );

    let rita = t.reviewer().await;
    for sop in [&height, &fire, &everyone] {
        publish(&t, &rita, sop).await;
    }
    let codes = |sops: Vec<Sop>| -> Vec<String> { sops.into_iter().map(|s| s.code).collect() };
    assert_eq!(
        codes(
            Sops::required_for(t.db(), "site_worker", &site1)
                .await
                .unwrap()
        ),
        ["SOP-HR-01", "SOP-HSE-01", "SOP-HSE-02"]
    );
    assert_eq!(
        codes(
            Sops::required_for(t.db(), "site_worker", &site2)
                .await
                .unwrap()
        ),
        ["SOP-HR-01", "SOP-HSE-02"]
    );
    assert_eq!(
        codes(Sops::required_for(t.db(), "driver", &site1).await.unwrap()),
        ["SOP-HR-01"]
    );
}

/// The kinds' documents stay documents: a version attached to the SOP is a
/// `pending_review` document of subject `documents.sop`.
#[renox::test]
async fn a_version_is_a_document_of_the_sop() {
    let t = boot().await;
    let mona = manager(&t).await;
    let sop = new_sop(&t, &mona, "SOP-HSE-01").await;
    t.install(&kind_seeds()).await;
    let version = t.attach(sop.subject(), "sop", None).await;
    assert_eq!(version.status, DocumentStatus::PendingReview);
    let listed = Documents::of_subject(t.db(), &sop.subject()).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(status(&t, &sop).await, SopStatus::Draft);
}
