//! Acceptance checks AC1–AC9 of #50, *Approve a one-use override of a
//! document block*: the test `BlockSubjects` for `deployment.activate` (with
//! its scope), the kinds *Health card* (`with_approval`), *SIRA licence* and
//! *Work permit* (legal blocks), users A, B (md) and C, and clock travel.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use renox::auth::permissions;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_documents::{
    BlockOverride, BlockOverrideStatus, Documents, KindSeed, kinds, permissions as perm,
};
use wl_kernel::approvals::{self, ApprovalRequested, ApprovalStatus};
use wl_kernel::documents::{self, BlockMode, DocumentBlockOverridden, OverrideMode};

use support::{Booted, Options, RAVI, boot_options, day, ravi, site, work_permit};

const ACTIVATE: &str = "deployment.activate";

fn health_card() -> KindSeed {
    KindSeed::new("health_card", "Health card")
        .subject_type("hr.employee")
        .expires()
        .blocks(ACTIVATE, BlockMode::Block)
}

fn sira_licence() -> KindSeed {
    KindSeed::new("sira_licence", "SIRA licence")
        .subject_type("hr.employee")
        .expires()
        .legal_block()
        .blocks(ACTIVATE, BlockMode::Block)
}

/// Ravi's documents are all expired; A asks, B and C (md in the
/// deployment's site) decide, D is md in another site.
struct World {
    t: Booted,
    rita: User,
    a: User,
    b: User,
    c: User,
    d: User,
}

async fn world(kinds: &[KindSeed]) -> World {
    world_with(kinds, true).await
}

/// [`world`]; without `fake_events` the listeners run (the refusal of an
/// override by a rejection is one).
async fn world_with(kinds: &[KindSeed], fake_events: bool) -> World {
    let t = boot_options(Options {
        kernel: true,
        fake_events,
        ..Options::default()
    })
    .await;
    t.install(kinds).await;
    let rita = t.reviewer().await;
    for kind in kinds {
        t.accepted(&rita, ravi(), &kind.key, Some(day(-3))).await;
    }
    permissions::define_role(t.db(), "md", &[perm::BLOCK_OVERRIDE])
        .await
        .unwrap();
    let a = t.staff("Alice", "md", Some(&support::plant1())).await;
    let b = t.staff("Bea", "md", Some(&site())).await;
    let c = t.staff("Cora", "md", Some(&site())).await;
    let d = t.staff("Dan", "md", Some(&support::plant2())).await;
    World {
        t,
        rita,
        a,
        b,
        c,
        d,
    }
}

impl World {
    async fn request(&self, kind: &str) -> Result<Vec<BlockOverride>> {
        Documents::request_override(
            self.t.state(),
            &self.a,
            kind,
            ACTIVATE,
            RAVI,
            "renewal booked 2026-12-03",
        )
        .await
    }

    async fn requested(&self) -> BlockOverride {
        self.request("health_card").await.unwrap().remove(0)
    }

    async fn approved(&self) -> BlockOverride {
        let row = self.requested().await;
        approvals::approve(
            self.t.state(),
            row.approval_request_id.unwrap(),
            &self.b,
            "",
        )
        .await
        .unwrap();
        self.reload(&row).await
    }

    async fn reload(&self, row: &BlockOverride) -> BlockOverride {
        BlockOverride::find(self.t.db(), row.id)
            .await
            .unwrap()
            .unwrap()
    }

    /// `actor` activates; the closure counts its runs.
    async fn activate(&self, actor: &User, runs: &AtomicUsize) -> Result<()> {
        documents::guarded_transition_as(
            self.t.state(),
            ACTIVATE,
            RAVI,
            day(0),
            Some(actor.id),
            || async {
                runs.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
    }

    async fn refusal(&self, actor: &User) -> documents::BlockRefusal {
        documents::guard(self.t.state())
            .check_as(self.t.state(), ACTIVATE, RAVI, day(0), Some(actor.id))
            .await
            .unwrap()
            .expect_err("the guard refuses")
    }
}

fn text(error: &Error) -> String {
    match error {
        Error::Status(_, message) => message.clone(),
        other => format!("{other:?}"),
    }
}

fn is_forbidden(error: &Error) -> bool {
    matches!(
        error,
        Error::Forbidden | Error::Status(StatusCode::FORBIDDEN, _)
    )
}

#[renox::test]
async fn ac1_ravis_expired_health_card_is_overridden_once_by_a_with_the_approval_of_b() {
    let w = world(&[health_card()]).await;
    let refusal = w.refusal(&w.a).await;
    let message = refusal.message();
    assert!(message.contains("Health card expired"), "{message}");
    assert!(message.contains("override: with approval"), "{message}");
    let links = refusal.override_links();
    assert_eq!(links.len(), 1);
    assert!(links[0].starts_with("/documents/overrides/new?kind=health_card"));

    let row = w.requested().await;
    assert_eq!(row.status, BlockOverrideStatus::Requested);
    assert_eq!(row.reason, "renewal booked 2026-12-03");
    assert_eq!(row.scope(), site());
    let request = approvals::ApprovalRequest::find(w.t.db(), row.approval_request_id.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.status, ApprovalStatus::Pending);
    let announced = w.t.app.emitted::<ApprovalRequested>();
    assert!(announced[0].step_approvers.contains(&w.b.id));
    assert!(announced[0].step_approvers.contains(&w.c.id));
    assert!(!announced[0].step_approvers.contains(&w.a.id));
    assert!(!announced[0].step_approvers.contains(&w.d.id), "other site");

    // B sees it waiting.
    w.t.app.acting_as(&w.b);
    w.t.app
        .get("/documents/overrides")
        .await
        .assert_ok()
        .assert_see("Waiting for your approval")
        .assert_see(&format!("/documents/overrides/{}", row.id));
    assert_eq!(approvals::awaiting(w.t.db(), &w.b).await.unwrap().len(), 1);
    assert!(
        approvals::awaiting(w.t.db(), &w.a)
            .await
            .unwrap()
            .is_empty()
    );

    approvals::approve(w.t.state(), request.id, &w.b, "")
        .await
        .unwrap();
    let approved = w.reload(&row).await;
    assert_eq!(approved.status, BlockOverrideStatus::Approved);
    assert_eq!(approved.approved_by, Some(w.b.id));
    let at = approved.approved_at.expect("approved at");
    assert_eq!(
        approved.valid_until,
        Some(at + renox::chrono::Duration::hours(72))
    );

    let runs = AtomicUsize::new(0);
    w.activate(&w.a, &runs).await.unwrap();
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    let used = w.reload(&row).await;
    assert_eq!(used.status, BlockOverrideStatus::Used);
    assert!(used.used_at.is_some());
    let events = w.t.app.emitted::<DocumentBlockOverridden>();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.override_id.0, row.id);
    assert_eq!(event.kind, "health_card");
    assert_eq!(event.subject, ravi());
    assert_eq!(event.guarded, ACTIVATE);
    assert_eq!(event.record_id, RAVI);
    assert_eq!(event.reason, "renewal booked 2026-12-03");
    assert_eq!(event.requested_by, w.a.id);
    assert_eq!(event.approved_by, w.b.id);

    // AC2: a second attempt is refused, runs nothing, emits nothing.
    let again = w.activate(&w.a, &runs).await.expect_err("used up");
    assert!(text(&again).contains("Health card"), "{}", text(&again));
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert_eq!(w.t.app.emitted::<DocumentBlockOverridden>().len(), 1);
}

#[renox::test]
async fn ac3_neither_the_approver_nor_another_holder_can_use_it_and_a_still_can() {
    let w = world(&[health_card()]).await;
    let row = w.approved().await;
    let runs = AtomicUsize::new(0);
    for other in [&w.b, &w.c] {
        let refused = w
            .activate(other, &runs)
            .await
            .expect_err("not the requester");
        assert!(text(&refused).contains("Health card"));
    }
    assert_eq!(runs.load(Ordering::SeqCst), 0);
    assert!(w.reload(&row).await.used_at.is_none());
    w.activate(&w.a, &runs).await.unwrap();
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert_eq!(w.reload(&row).await.status, BlockOverrideStatus::Used);
}

#[renox::test]
async fn ac4_an_unused_override_is_good_for_72_hours_and_then_shows_as_expired() {
    let w = world(&[health_card()]).await;
    let row = w.approved().await;
    let runs = AtomicUsize::new(0);

    w.t.app.travel(Duration::from_secs(72 * 3600 - 60));
    let inside = w.t.app.at_travelled_time(w.refusal_or_ok(&w.a)).await;
    assert!(inside, "still accepted a minute before the 72 hours end");

    w.t.app.travel(Duration::from_secs(120));
    let outside = w.t.app.at_travelled_time(w.refusal_or_ok(&w.a)).await;
    assert!(!outside, "refused a minute after");
    let refused =
        w.t.app
            .at_travelled_time(w.activate(&w.a, &runs))
            .await
            .expect_err("expired");
    assert!(text(&refused).contains("Health card"));
    assert_eq!(runs.load(Ordering::SeqCst), 0);

    w.t.app.acting_as(&w.a);
    w.t.app
        .get(&format!("/documents/overrides/{}", row.id))
        .await
        .assert_ok()
        .assert_see("Expired");
}

impl World {
    /// Whether the guard lets `actor` through now.
    async fn refusal_or_ok(&self, actor: &User) -> bool {
        documents::guard(self.t.state())
            .check_as(self.t.state(), ACTIVATE, RAVI, day(0), Some(actor.id))
            .await
            .unwrap()
            .is_ok()
    }
}

#[renox::test]
async fn ac5_a_legal_block_offers_no_override_and_still_refuses_beside_an_approved_one() {
    let w = world(&[health_card(), sira_licence()]).await;
    let refusal = w.refusal(&w.a).await;
    let message = refusal.message();
    assert!(message.contains("override: not allowed"), "{message}");
    // Only the health card is offered.
    let links = refusal.override_links();
    assert_eq!(links.len(), 1);
    assert!(links[0].contains("kind=health_card"));

    match w.request("sira_licence").await {
        Err(Error::Status(code, message)) => {
            assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
            assert!(
                message.contains("SIRA licence cannot be overridden"),
                "{message}"
            );
        }
        other => panic!("expected a 422, got {other:?}"),
    }

    w.approved().await;
    let runs = AtomicUsize::new(0);
    let refused = w
        .activate(&w.a, &runs)
        .await
        .expect_err("the licence blocks");
    let message = text(&refused);
    assert!(message.contains("SIRA licence"), "{message}");
    assert!(!message.contains("Health card"), "{message}");
    assert_eq!(runs.load(Ordering::SeqCst), 0);
}

#[renox::test]
async fn ac6_a_legal_block_stays_never_and_an_ordinary_kind_can_be_switched_to_never() {
    let w = world(&[health_card(), work_permit()]).await;
    let locked = kinds::set_override_mode(w.t.db(), "work_permit", OverrideMode::WithApproval)
        .await
        .unwrap();
    assert!(locked.is_err(), "a legal block cannot be loosened");
    w.t.install(&[work_permit()]).await;
    assert_eq!(
        w.t.kind("work_permit").await.override_mode,
        OverrideMode::Never
    );
    let admin = w.t.kind_admin().await;
    w.t.app.acting_as(&admin);
    w.t.app
        .get("/document-kinds/work_permit")
        .await
        .assert_ok()
        .assert_dont_see("/document-kinds/work_permit/override-mode");

    kinds::set_override_mode(w.t.db(), "health_card", OverrideMode::Never)
        .await
        .unwrap()
        .unwrap();
    let refusal = w.refusal(&w.a).await;
    assert!(refusal.message().contains("override: not allowed"));
    assert!(refusal.override_links().is_empty());
    assert!(w.request("health_card").await.is_err());
}

/// A vertical may fix the override mode to `never` without making the kind a
/// legal block (05 practitioner licence): the client cannot loosen it, a
/// stored kind is put back on install, and the page offers no change.
#[renox::test]
async fn ac6b_a_fixed_override_mode_stays_never_and_the_kind_is_not_a_legal_block() {
    let fixed = KindSeed::new("fixed_licence", "Fixed licence")
        .subject_type("hr.employee")
        .expires()
        .override_fixed()
        .blocks(ACTIVATE, BlockMode::Block);
    kinds::register_kind(fixed.clone());
    let w = world(std::slice::from_ref(&fixed)).await;
    let kind = w.t.kind("fixed_licence").await;
    assert_eq!(kind.override_mode, OverrideMode::Never);
    assert!(!kind.legal_block && !kind.compliance_critical);

    let refused = kinds::set_override_mode(w.t.db(), "fixed_licence", OverrideMode::WithApproval)
        .await
        .unwrap()
        .expect_err("a fixed override mode cannot be loosened");
    assert_eq!(refused.reason, kinds::RefusalReason::OverrideFixed);

    // A row an older build stored as `with_approval` is put back.
    let mut stored = w.t.kind("fixed_licence").await;
    stored.override_mode = OverrideMode::WithApproval;
    stored.save(w.t.db()).await.unwrap();
    w.t.install(std::slice::from_ref(&fixed)).await;
    assert_eq!(
        w.t.kind("fixed_licence").await.override_mode,
        OverrideMode::Never
    );

    let admin = w.t.kind_admin().await;
    w.t.app.acting_as(&admin);
    w.t.app
        .get("/document-kinds/fixed_licence")
        .await
        .assert_ok()
        .assert_dont_see("/document-kinds/fixed_licence/override-mode");
}

#[renox::test]
async fn ac7_a_cannot_approve_their_own_request_and_a_rejection_refuses_the_override() {
    let w = world_with(&[health_card()], false).await;
    let row = w.requested().await;
    let id = row.approval_request_id.unwrap();
    let own = approvals::approve(w.t.state(), id, &w.a, "")
        .await
        .expect_err("separation of duties");
    assert!(is_forbidden(&own), "{own:?}");
    let request = approvals::ApprovalRequest::find(w.t.db(), id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.status, ApprovalStatus::Pending);
    let outsider = approvals::approve(w.t.state(), id, &w.d, "")
        .await
        .expect_err("not an approver in this site");
    assert!(is_forbidden(&outsider), "{outsider:?}");

    approvals::reject(w.t.state(), id, &w.b, "no renewal proof")
        .await
        .unwrap();
    assert_eq!(w.reload(&row).await.status, BlockOverrideStatus::Refused);
    let runs = AtomicUsize::new(0);
    assert!(w.activate(&w.a, &runs).await.is_err());
    assert_eq!(runs.load(Ordering::SeqCst), 0);
}

#[renox::test]
async fn ac8_a_failing_closure_gives_it_back_and_of_two_concurrent_attempts_one_runs() {
    let w = world(&[health_card()]).await;
    let row = w.approved().await;
    let failed = documents::guarded_transition_as(
        w.t.state(),
        ACTIVATE,
        RAVI,
        day(0),
        Some(w.a.id),
        || async { Err::<(), _>(Error::Status(StatusCode::CONFLICT, "boom".into())) },
    )
    .await;
    assert!(failed.is_err());
    let back = w.reload(&row).await;
    assert_eq!(back.status, BlockOverrideStatus::Approved);
    assert!(back.used_at.is_none());

    let runs = AtomicUsize::new(0);
    let (first, second) = renox::tokio::join!(w.activate(&w.a, &runs), w.activate(&w.a, &runs));
    assert_eq!(
        [first.is_ok(), second.is_ok()]
            .iter()
            .filter(|ok| **ok)
            .count(),
        1,
        "{first:?} {second:?}"
    );
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    let used = w.reload(&row).await;
    assert_eq!(used.status, BlockOverrideStatus::Used);
    assert!(used.used_at.is_some());
    assert_eq!(w.t.app.emitted::<DocumentBlockOverridden>().len(), 1);
}

#[renox::test]
async fn ac9_a_request_needs_a_reason_a_refusal_and_no_open_duplicate() {
    let w = world(&[health_card()]).await;
    let blank =
        Documents::request_override(w.t.state(), &w.a, "health_card", ACTIVATE, RAVI, "  ").await;
    assert!(blank.is_err());
    w.requested().await;
    assert!(w.request("health_card").await.is_err(), "already open");
    assert_eq!(BlockOverride::query().count(w.t.db()).await.unwrap(), 1);
}

#[renox::test]
async fn ac9_a_record_the_guard_does_not_refuse_needs_no_override() {
    let w = world(&[health_card()]).await;
    w.t.accepted(&w.rita, ravi(), "health_card", Some(day(30)))
        .await;
    assert!(
        w.request("health_card").await.is_err(),
        "nothing to override"
    );
    assert_eq!(BlockOverride::query().count(w.t.db()).await.unwrap(), 0);
}

/// Without wl-documents the guard covers nothing and the closure just runs.
#[renox::test]
async fn ac9_without_the_module_the_closure_just_runs() {
    let app = TestApp::new(renox::App::new()).await;
    let runs = AtomicUsize::new(0);
    let outcome = documents::guard(app.state())
        .check_as(app.state(), ACTIVATE, RAVI, day(0), Some(1))
        .await
        .unwrap()
        .expect("nothing refuses");
    assert!(outcome.overrides.is_empty());
    documents::guarded_transition_as(app.state(), ACTIVATE, RAVI, day(0), Some(1), || async {
        runs.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}
