//! Requesting a block override and using it once in the guard (#50 A4, A6,
//! A7). The full acceptance checks AC1–AC9 come with the last part; these
//! cover `request_override`, `check_as` and the claim, give back and report
//! steps with a test `BlockSubjects` for `deployment.activate`.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use renox::prelude::*;
use wl_documents::{BlockOverride, BlockOverrideStatus, Documents, KindSeed, permissions as perm};
use wl_kernel::approvals;
use wl_kernel::documents::{self, BlockMode, DocumentBlockOverridden};

use support::{Booted, Options, RAVI, boot_options, day, ravi, residence_visa};

const ACTIVATE: &str = "deployment.activate";

fn health_card() -> KindSeed {
    KindSeed::new("health_card", "Health card")
        .subject_type("hr.employee")
        .expires()
        .blocks(ACTIVATE, BlockMode::Block)
}

/// Ravi's health card and residence visa are expired; users A (requester),
/// B (approver) and C (also an approver).
struct World {
    t: Booted,
    rita: User,
    a: User,
    b: User,
    c: User,
}

async fn world(kinds: &[KindSeed]) -> World {
    let t = boot_options(Options {
        kernel: true,
        ..Options::default()
    })
    .await;
    t.install(kinds).await;
    let rita = t.reviewer().await;
    for kind in ["health_card", "residence_visa"] {
        if kinds.iter().any(|k| k.key == kind) {
            t.accepted(&rita, ravi(), kind, Some(day(-3))).await;
        }
    }
    let a = t.user("Alice", &[]).await;
    let b = t.user("Bea", &[perm::BLOCK_OVERRIDE]).await;
    let c = t.user("Cora", &[perm::BLOCK_OVERRIDE]).await;
    World { t, rita, a, b, c }
}

impl World {
    async fn request(&self, kind: &str) -> Result<Vec<BlockOverride>> {
        Documents::request_override(
            self.t.state(),
            &self.a,
            kind,
            ACTIVATE,
            RAVI,
            "renewal booked",
        )
        .await
    }

    /// A requests and B approves.
    async fn approved(&self) -> BlockOverride {
        let mut rows = self.request("health_card").await.unwrap();
        assert_eq!(rows.len(), 1);
        let row = rows.remove(0);
        let id = row.approval_request_id.expect("an approval request");
        approvals::approve(self.t.state(), id, &self.b, "")
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

    async fn covered(&self, actor: Option<i64>) -> Option<usize> {
        documents::guard(self.t.state())
            .check_as(self.t.state(), ACTIVATE, RAVI, day(0), actor)
            .await
            .unwrap()
            .ok()
            .map(|outcome| outcome.overrides.len())
    }
}

fn status_422(result: Result<Vec<BlockOverride>>, text: &str) {
    match result {
        Err(Error::Status(code, message)) => {
            assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
            assert!(message.contains(text), "{message}");
        }
        other => panic!("expected a 422 with `{text}`, got {other:?}"),
    }
}

#[renox::test]
async fn a_request_is_stored_with_a_pending_approval_and_refuses_the_open_duplicate() {
    let w = world(&[health_card()]).await;
    let rows = w.request("health_card").await.unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.status, BlockOverrideStatus::Requested);
    assert_eq!(row.requested_by, w.a.id);
    assert_eq!(row.subject(), ravi());
    assert_eq!(row.guarded, ACTIVATE);
    assert_eq!(row.record_id, RAVI);
    assert!(row.scope().is_global());
    let request = approvals::ApprovalRequest::find(w.t.db(), row.approval_request_id.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.status, approvals::ApprovalStatus::Pending);
    assert_eq!(request.subject_id, row.id);
    let announced = w.t.app.emitted::<approvals::ApprovalRequested>();
    assert_eq!(announced.len(), 1, "announced after the commit");
    assert!(announced[0].step_approvers.contains(&w.b.id));
    assert!(!announced[0].step_approvers.contains(&w.a.id));

    status_422(w.request("health_card").await, "already open");
}

#[renox::test]
async fn a_request_is_refused_for_a_blank_reason_a_legal_block_and_nothing_to_override() {
    let w = world(&[health_card(), residence_visa()]).await;
    let blank =
        Documents::request_override(w.t.state(), &w.a, "health_card", ACTIVATE, RAVI, " ").await;
    status_422(blank, "Say why");
    status_422(
        w.request("residence_visa").await,
        "Residence visa cannot be overridden",
    );
}

#[renox::test]
async fn a_request_for_a_record_the_guard_does_not_refuse_is_refused() {
    let w = world(&[health_card()]).await;
    w.t.accepted(&w.rita, ravi(), "health_card", Some(day(30)))
        .await;
    status_422(w.request("health_card").await, "nothing to override");
    assert_eq!(BlockOverride::query().count(w.t.db()).await.unwrap(), 0);
}

#[renox::test]
async fn only_the_requester_is_covered_and_only_once_approved() {
    let w = world(&[health_card()]).await;
    let row = w.request("health_card").await.unwrap().remove(0);
    assert_eq!(w.covered(Some(w.a.id)).await, None, "not approved yet");

    let approver = row.approval_request_id.unwrap();
    approvals::approve(w.t.state(), approver, &w.b, "")
        .await
        .unwrap();
    let approved = w.reload(&row).await;
    assert_eq!(approved.status, BlockOverrideStatus::Approved);
    assert_eq!(approved.approved_by, Some(w.b.id));
    assert!(approved.valid_until.is_some());

    assert_eq!(w.covered(Some(w.a.id)).await, Some(1));
    assert_eq!(w.covered(Some(w.b.id)).await, None, "the approver");
    assert_eq!(w.covered(Some(w.c.id)).await, None, "another holder");
    assert_eq!(w.covered(None).await, None, "no actor");
}

#[renox::test]
async fn a_legal_block_still_refuses_beside_a_covered_one() {
    let w = world(&[health_card(), residence_visa()]).await;
    let row = w.request("health_card").await.unwrap().remove(0);
    approvals::approve(w.t.state(), row.approval_request_id.unwrap(), &w.b, "")
        .await
        .unwrap();
    let refusal = documents::guard(w.t.state())
        .check_as(w.t.state(), ACTIVATE, RAVI, day(0), Some(w.a.id))
        .await
        .unwrap()
        .expect_err("the visa still blocks");
    let message = refusal.message();
    assert!(message.contains("Residence visa"), "{message}");
    assert!(!message.contains("Health card"), "{message}");
}

#[renox::test]
async fn an_approved_override_runs_the_closure_once_and_emits_the_event() {
    let w = world(&[health_card()]).await;
    let row = w.approved().await;
    let runs = AtomicUsize::new(0);
    let run = || async {
        runs.fetch_add(1, Ordering::SeqCst);
        Ok(())
    };
    documents::guarded_transition_as(w.t.state(), ACTIVATE, RAVI, day(0), Some(w.a.id), run)
        .await
        .unwrap();
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    let used = w.reload(&row).await;
    assert_eq!(used.status, BlockOverrideStatus::Used);
    assert!(used.used_at.is_some());
    let events = w.t.app.emitted::<DocumentBlockOverridden>();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].override_id.0, row.id);
    assert_eq!(events[0].kind, "health_card");
    assert_eq!(events[0].requested_by, w.a.id);
    assert_eq!(events[0].approved_by, w.b.id);
    assert_eq!(events[0].reason, "renewal booked");

    // A second attempt is refused and runs nothing.
    let again = documents::guarded_transition_as(
        w.t.state(),
        ACTIVATE,
        RAVI,
        day(0),
        Some(w.a.id),
        || async {
            runs.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    )
    .await;
    assert!(again.is_err());
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert_eq!(w.t.app.emitted::<DocumentBlockOverridden>().len(), 1);
}

#[renox::test]
async fn a_failing_closure_gives_the_override_back() {
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
    assert!(w.t.app.emitted::<DocumentBlockOverridden>().is_empty());
    assert_eq!(w.covered(Some(w.a.id)).await, Some(1));
}

#[renox::test]
async fn an_unused_override_runs_out_after_72_hours() {
    let w = world(&[health_card()]).await;
    w.approved().await;
    w.t.app.travel(Duration::from_secs(71 * 3600));
    let near = w.t.app.at_travelled_time(w.covered(Some(w.a.id))).await;
    assert_eq!(near, Some(1));
    w.t.app.travel(Duration::from_secs(2 * 3600));
    let late = w.t.app.at_travelled_time(w.covered(Some(w.a.id))).await;
    assert_eq!(late, None);
}
