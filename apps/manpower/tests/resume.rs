//! BL-MPW-12 (#426), 09 §7.1 *Deployment* `resume`, `MPW.RUL.12` (the resume
//! part), `MPW.ACL.14` (a hold does not stop it): a renewed document that is
//! accepted resumes the deployment when no other reason holds it, a manual
//! reason is resumed by staff only, the cover request of the suspension is
//! cancelled with *worker returned* while it is `open` or `partially_filled`
//! and an active cover runs to its own end date. The guard `MPW.ACL.07` of
//! `resume` with the real wl-documents, and the blocked client, are in
//! `host/tests/manpower_resume.rs`.

mod support;

use manpower::suspension::DeploymentSuspension;
use manpower::{
    ClientContract, ContractStatus, Deployment, DeploymentKind, DeploymentStatus, ManpowerRequest,
    RequestKind, RequestStatus, Site,
};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::documents::{
    BlockEntry, BlockMode, DocumentAccepted, DocumentExpired, DocumentId, OverrideMode,
};
use wl_kernel::notify::NotifyRequest;
use wl_kernel::workforce::EmployeeId;
use wl_kernel::{PartyId, Subject};

use support::{Booted, TOWER, boot, date};

/// Amal, a contract that requires cover, the site, worker W (employee 7) and
/// the active D.
struct World {
    t: Booted,
    amal: User,
    contract: ClientContract,
    site: Site,
    d: Deployment,
}

async fn world() -> World {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    ClientContract::query()
        .where_(ClientContract::ID.eq(contract.id))
        .update(t.db(), &[("cover_required", &true)])
        .await
        .unwrap();
    let site = t.site(TOWER, "Tower A").await;
    let w = t.staff.employ(&t.parties, 7, "Ravi Kumar", true);
    let d = t
        .deployment(
            &contract,
            &site,
            w,
            Some(date(2027, 12, 31)),
            DeploymentStatus::Active,
        )
        .await;
    World {
        t,
        amal,
        contract,
        site,
        d,
    }
}

fn blocking(kind: &str) -> Vec<BlockEntry> {
    ["activate", "extend", "resume"]
        .into_iter()
        .map(|transition| BlockEntry {
            kind: kind.into(),
            guarded: format!("deployment.{transition}"),
            mode: BlockMode::Block,
            override_mode: OverrideMode::Never,
            condition: None,
            locked: true,
        })
        .collect()
}

impl World {
    /// `kind` of W expired through 2027-03-19; the night's run suspends D.
    async fn expire(&self, kind: &str) {
        self.t.documents.expire(7, kind);
        self.t
            .state()
            .emit(DocumentExpired {
                document: DocumentId(1),
                subject: Subject::new("hr.employee", 7),
                kind: kind.into(),
                expires_on: date(2027, 3, 19),
                blocks: blocking(kind),
                blocked: Vec::new(),
            })
            .await
            .unwrap();
    }

    /// A renewed `kind` of W is accepted, valid to `until`.
    async fn accept(&self, kind: &str, until: Option<NaiveDate>) {
        self.t.documents.renew(7, kind);
        self.t
            .state()
            .emit(DocumentAccepted {
                document: DocumentId(2),
                subject: Subject::new("hr.employee", 7),
                kind: kind.into(),
                expires_on: until,
            })
            .await
            .unwrap();
    }

    async fn deployment(&self) -> Deployment {
        self.t.reload_deployment(self.d.id).await
    }

    async fn reasons(&self) -> Vec<DeploymentSuspension> {
        DeploymentSuspension::open_for(self.t.db(), self.d.id)
            .await
            .unwrap()
    }

    fn client_notices(&self) -> Vec<NotifyRequest> {
        self.t.notices.of_type("manpower.client_notice")
    }

    /// The cover request of the suspension.
    async fn cover(&self) -> ManpowerRequest {
        ManpowerRequest::query()
            .where_(ManpowerRequest::KIND.eq(RequestKind::Replacement))
            .first(self.t.db())
            .await
            .unwrap()
            .expect("the cover request")
    }

    async fn resume(&self, on: NaiveDate) -> u16 {
        let path = format!("/manpower/deployments/{}/resume", self.d.id);
        self.t
            .on(on, async { self.t.post(&self.amal, &path, &[]).await })
            .await
            .status
            .as_u16()
    }
}

/// AC1: D is suspended for the work permit; the renewed permit (valid to
/// 2028-11-20) is accepted: D is `active`, the reason is cleared, the day
/// and the days suspended are kept, and the client is told the worker is
/// available again.
#[renox::test]
async fn the_renewed_permit_resumes_the_deployment_and_the_client_is_told() {
    let w = world().await;
    w.expire("work_permit").await;
    assert_eq!(w.deployment().await.status, DeploymentStatus::Suspended);

    w.t.on(date(2027, 3, 28), async {
        w.accept("work_permit", Some(date(2028, 11, 20))).await;
    })
    .await;

    let d = w.deployment().await;
    assert_eq!(d.status, DeploymentStatus::Active);
    assert_eq!(d.resumed_on, Some(date(2027, 3, 28)));
    // Suspended from 2027-03-20 to the 27th: 8 calendar days not billed.
    assert_eq!(d.suspended_days, 8);
    assert_eq!(d.days_suspended(date(2027, 4, 30)), 8);
    assert!(w.reasons().await.is_empty());
    let told = w.client_notices();
    assert_eq!(told.len(), 2, "unavailable, then available again");
    let back = told[1].data.to_string();
    assert!(
        back.contains("Ravi Kumar is available again at Tower A from 2027-03-28"),
        "{back}"
    );
}

/// AC2: D suspended for the work permit and the health insurance; accepting the
/// permit leaves D `suspended` with one reason left; accepting a kind that is
/// not expired changes nothing.
#[renox::test]
async fn accepting_one_of_two_kinds_leaves_the_other_reason() {
    let w = world().await;
    w.expire("work_permit").await;
    w.expire("health_insurance").await;
    assert_eq!(w.reasons().await.len(), 2);

    w.accept("passport", Some(date(2030, 1, 1))).await;
    assert_eq!(w.reasons().await.len(), 2, "a kind that is not expired");
    w.accept("work_permit", Some(date(2028, 11, 20))).await;

    assert_eq!(w.deployment().await.status, DeploymentStatus::Suspended);
    let left = w.reasons().await;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].reason, "health_insurance");
    assert_eq!(w.client_notices().len(), 1, "nothing more to the client");
}

/// An acceptance of the kind whose new expiry date is not later than the one
/// that expired removes nothing.
#[renox::test]
async fn an_older_document_does_not_clear_the_reason() {
    let w = world().await;
    w.expire("work_permit").await;
    w.accept("work_permit", Some(date(2027, 3, 1))).await;
    assert_eq!(w.deployment().await.status, DeploymentStatus::Suspended);
    assert_eq!(w.reasons().await.len(), 1);
}

/// AC3: D suspended for *site access withdrawn*: the acceptance of a document
/// does not resume it; account manager A resumes it.
#[renox::test]
async fn a_manual_reason_is_resumed_by_staff_only() {
    let w = world().await;
    w.t.on(date(2027, 3, 1), async {
        w.t.post(
            &w.amal,
            &format!("/manpower/deployments/{}/suspend", w.d.id),
            &[("reason", "site access withdrawn")],
        )
        .await
        .assert_status(303);
    })
    .await;
    w.accept("work_permit", Some(date(2028, 11, 20))).await;
    assert_eq!(w.deployment().await.status, DeploymentStatus::Suspended);
    assert_eq!(w.reasons().await.len(), 1);

    assert_eq!(w.resume(date(2027, 3, 10)).await, 303);
    let d = w.deployment().await;
    assert_eq!(d.status, DeploymentStatus::Active);
    assert_eq!(d.suspended_days, 9);
    assert!(w.reasons().await.is_empty());
}

/// Only an account manager of the client resumes, and only a suspended
/// deployment.
#[renox::test]
async fn resume_needs_the_permission_and_a_suspended_deployment() {
    let w = world().await;
    assert_eq!(w.resume(date(2027, 3, 10)).await, 422, "D is active");
    w.expire("iloe").await;
    let stranger = w.t.account_manager("Sara", PartyId(32)).await;
    let path = format!("/manpower/deployments/{}/resume", w.d.id);
    w.t.post(&stranger, &path, &[]).await.assert_status(403);
    assert_eq!(w.deployment().await.status, DeploymentStatus::Suspended);
}

/// AC4 and AC5: the cover request of the suspension, still `open` (nobody
/// found), is cancelled with *worker returned* when D resumes.
#[renox::test]
async fn an_open_cover_request_is_cancelled_when_the_worker_returns() {
    let w = world().await;
    w.expire("work_permit").await;
    let cover = w.cover().await;
    assert_eq!(cover.status, RequestStatus::Open);

    w.accept("work_permit", Some(date(2028, 11, 20))).await;

    assert_eq!(w.deployment().await.status, DeploymentStatus::Active);
    let cancelled = w.t.reload_request(cover.id).await;
    assert_eq!(cancelled.status, RequestStatus::Cancelled);
}

/// AC4 and AC5: the cover request is `partially_filled` (headcount 2) with an
/// `active` cover deployment C ending 2027-12-15 and a `planned` one: the
/// request is cancelled, the planned cover with it, and C keeps running to its
/// own end date; a `filled` request is left as it is.
#[renox::test]
async fn an_active_cover_runs_to_its_end_date_and_a_filled_request_stays() {
    let w = world().await;
    w.expire("work_permit").await;
    let cover = w.cover().await;
    // The request took one of two workers: C is active, a second is planned.
    let helper = w.t.staff.employ(&w.t.parties, 8, "Omar Said", true);
    let third = w.t.staff.employ(&w.t.parties, 9, "Imran Ali", true);
    let make = |employee: EmployeeId, status: DeploymentStatus| Deployment {
        employee_id: employee.0,
        client_contract_id: w.contract.id,
        client_party_id: TOWER.0,
        site_id: w.site.id,
        request_id: Some(cover.id),
        role: "guard".into(),
        shift_type: "day".into(),
        start_date: date(2027, 3, 21),
        end_date: Some(date(2027, 12, 15)),
        kind: DeploymentKind::Cover,
        covers_deployment_id: Some(w.d.id),
        status,
        ..Deployment::default()
    };
    let mut active = make(helper, DeploymentStatus::Active);
    let mut planned = make(third, DeploymentStatus::Planned);
    let mut tx = w.t.db().begin().await.unwrap();
    active
        .insert_in_state(&mut tx, DeploymentStatus::Active, "fixture")
        .await
        .unwrap();
    planned
        .insert_in_state(&mut tx, DeploymentStatus::Planned, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    ManpowerRequest::query()
        .where_(ManpowerRequest::ID.eq(cover.id))
        .update(
            w.t.db(),
            &[
                ("status", &RequestStatus::PartiallyFilled),
                ("headcount", &2_i64),
                ("filled_count", &1_i64),
            ],
        )
        .await
        .unwrap();

    w.accept("work_permit", Some(date(2028, 11, 20))).await;

    assert_eq!(w.deployment().await.status, DeploymentStatus::Active);
    assert_eq!(
        w.t.reload_request(cover.id).await.status,
        RequestStatus::Cancelled
    );
    let c = w.t.reload_deployment(active.id).await;
    assert_eq!(c.status, DeploymentStatus::Active, "C keeps running");
    assert_eq!(c.end_date, Some(date(2027, 12, 15)));
    assert_eq!(
        w.t.reload_deployment(planned.id).await.status,
        DeploymentStatus::Cancelled
    );

    // A filled request is left as it is.
    let w = world().await;
    w.expire("work_permit").await;
    let cover = w.cover().await;
    ManpowerRequest::query()
        .where_(ManpowerRequest::ID.eq(cover.id))
        .update(
            w.t.db(),
            &[("status", &RequestStatus::Filled), ("filled_count", &1_i64)],
        )
        .await
        .unwrap();
    w.accept("work_permit", Some(date(2028, 11, 20))).await;
    assert_eq!(
        w.t.reload_request(cover.id).await.status,
        RequestStatus::Filled
    );
}

/// BL-MPW-02's gap: a suspended deployment resumes while its contract is on
/// credit hold (`MPW.ACL.14` covers `activate` and `extend` only), by hand and
/// by the renewed document.
#[renox::test]
async fn a_suspended_deployment_resumes_while_the_contract_is_on_hold() {
    let w = world().await;
    w.expire("work_permit").await;
    let second = w.t.staff.employ(&w.t.parties, 8, "Omar Said", true);
    let held =
        w.t.deployment(
            &w.contract,
            &w.site,
            second,
            Some(date(2027, 12, 31)),
            DeploymentStatus::Suspended,
        )
        .await;
    // The contract goes on credit hold: `activate` and `extend` are refused.
    w.t.post(
        &w.amal,
        &format!("/manpower/contracts/{}/hold", w.contract.id),
        &[("reason", "other"), ("text", "Overdue invoices")],
    )
    .await
    .assert_status(303);
    assert_eq!(
        w.t.reload_contract(w.contract.id).await.status,
        ContractStatus::OnHold
    );
    w.t.on(date(2027, 3, 30), async {
        w.t.post(
            &w.amal,
            &format!("/manpower/deployments/{}/extend", w.d.id),
            &[("end_date", "2027-12-31")],
        )
        .await
        .assert_status(422);
    })
    .await;

    // By the renewed document.
    w.accept("work_permit", Some(date(2028, 11, 20))).await;
    assert_eq!(w.deployment().await.status, DeploymentStatus::Active);
    // By hand.
    let path = format!("/manpower/deployments/{}/resume", held.id);
    w.t.on(date(2027, 3, 30), async {
        w.t.post(&w.amal, &path, &[]).await.assert_status(303);
    })
    .await;
    assert_eq!(
        w.t.reload_deployment(held.id).await.status,
        DeploymentStatus::Active
    );
    assert_eq!(
        w.t.reload_contract(w.contract.id).await.status,
        ContractStatus::OnHold,
        "the hold stays"
    );
}

/// A deployment whose end date has passed does not resume (the daily run ends
/// it), and neither does one under a contract that has ended.
#[renox::test]
async fn a_deployment_past_its_end_date_or_under_an_ended_contract_does_not_resume() {
    let w = world().await;
    let short =
        w.t.deployment(
            &w.contract,
            &w.site,
            w.t.staff.employ(&w.t.parties, 8, "Omar Said", true),
            Some(date(2027, 3, 25)),
            DeploymentStatus::Suspended,
        )
        .await;
    let path = format!("/manpower/deployments/{}/resume", short.id);
    w.t.on(date(2027, 3, 26), async {
        let refused = w.t.post(&w.amal, &path, &[]).await;
        refused.assert_status(422);
        assert!(refused.text().contains("end date"), "{}", refused.text());
    })
    .await;
    assert_eq!(
        w.t.reload_deployment(short.id).await.status,
        DeploymentStatus::Suspended
    );

    let ended =
        w.t.contract(TOWER, ContractStatus::Ended, Some(&w.amal))
            .await;
    let under_ended =
        w.t.deployment(
            &ended,
            &w.site,
            w.t.staff.employ(&w.t.parties, 9, "Imran Ali", true),
            Some(date(2027, 12, 31)),
            DeploymentStatus::Suspended,
        )
        .await;
    w.t.on(date(2027, 3, 26), async {
        w.t.post(
            &w.amal,
            &format!("/manpower/deployments/{}/resume", under_ended.id),
            &[],
        )
        .await
        .assert_status(422);
    })
    .await;
}

/// The days of a suspension add up over several suspensions and the list
/// counts the current one.
#[renox::test]
async fn the_days_suspended_add_up_over_several_suspensions() {
    let w = world().await;
    w.t.on(date(2027, 3, 1), async {
        w.t.post(
            &w.amal,
            &format!("/manpower/deployments/{}/suspend", w.d.id),
            &[("reason", "investigation")],
        )
        .await
        .assert_status(303);
    })
    .await;
    assert_eq!(w.resume(date(2027, 3, 4)).await, 303);
    assert_eq!(w.deployment().await.suspended_days, 3);
    w.t.on(date(2027, 3, 10), async {
        w.t.post(
            &w.amal,
            &format!("/manpower/deployments/{}/suspend", w.d.id),
            &[("reason", "site access withdrawn")],
        )
        .await
        .assert_status(303);
    })
    .await;
    let d = w.deployment().await;
    assert_eq!(d.suspended_on, Some(date(2027, 3, 10)));
    // The 10th to the 12th, with the 3 days before.
    assert_eq!(d.days_suspended(date(2027, 3, 12)), 6);
}
