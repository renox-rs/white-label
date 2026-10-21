//! BL-MPW-11 (#425), 09 §7.1 *Deployment* `suspend`, `MPW.RUL.12` (the expiry
//! part), `MPW.DEP.02`: the account manager suspends a deployment by hand with
//! a reason; the expiry of a document or licence the deployment needs
//! suspends it the day after the expiry date, tells the client with a neutral
//! reason and the account manager with the real one, opens the cover request,
//! adds a further kind to the reasons without a second message, leaves a
//! `planned` deployment alone, and a failing run is retried and then alerts
//! the owner. The real nightly chain with wl-documents is in
//! `host/tests/manpower_suspend.rs`.

mod support;

use std::time::Duration;

use manpower::suspension::{self, DeploymentSuspension, SuspensionKind};
use manpower::{
    ClientContract, ContractStatus, Deployment, DeploymentStatus, ManpowerRequest, RequestKind,
    RequestStatus, Site,
};
use renox::prelude::*;
use wl_kernel::compliance::ComplianceAlert;
use wl_kernel::documents::{BlockEntry, BlockMode, DocumentExpired, DocumentId, OverrideMode};
use wl_kernel::notify::NotifyRequest;
use wl_kernel::workforce::EmployeeId;
use wl_kernel::{PartyId, Subject};

use support::{Booted, TOWER, boot, date, users_of};

/// What the tests share: Amal, a contract that requires cover, the site, and
/// worker W (employee 7) active at it until the end of the year.
struct World {
    t: Booted,
    amal: User,
    contract: ClientContract,
    site: Site,
    w: EmployeeId,
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
        w,
        d,
    }
}

/// `kind` of employee 7 expired through 2027-03-19, as the nightly run of
/// wl-documents reports it: the kind blocks every deployment transition.
fn expired(kind: &str, blocks: Vec<BlockEntry>) -> DocumentExpired {
    DocumentExpired {
        document: DocumentId(1),
        subject: Subject::new("hr.employee", 7),
        kind: kind.into(),
        expires_on: date(2027, 3, 19),
        blocks,
        blocked: Vec::new(),
    }
}

fn blocking(kind: &str, mode: BlockMode) -> Vec<BlockEntry> {
    ["activate", "extend", "resume"]
        .into_iter()
        .map(|transition| BlockEntry {
            kind: kind.into(),
            guarded: format!("deployment.{transition}"),
            mode,
            override_mode: OverrideMode::Never,
            condition: None,
            locked: true,
        })
        .collect()
}

impl World {
    /// The night after the permit's last day: the kind expired.
    async fn expire(&self, kind: &str) {
        self.t.documents.expire(7, kind);
        self.t
            .state()
            .emit(expired(kind, blocking(kind, BlockMode::Block)))
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

    /// The notices to the client about the suspension: the one that tells it
    /// the cover is not found yet (`MPW.RUL.13`, a same-day suspension) is
    /// BL-MPW-16's and has its own tests.
    fn client_notices(&self) -> Vec<NotifyRequest> {
        self.t
            .notices
            .of_type("manpower.client_notice")
            .into_iter()
            .filter(|request| !text(request).contains("Cover not found yet"))
            .collect()
    }

    /// The notices to staff about the suspension (the cover request has its
    /// own).
    fn staff_notices(&self) -> Vec<NotifyRequest> {
        self.t
            .notices
            .of_type("manpower.notice")
            .into_iter()
            .filter(|request| text(request).contains("suspended"))
            .collect()
    }

    async fn covers(&self) -> Vec<ManpowerRequest> {
        ManpowerRequest::query()
            .where_(ManpowerRequest::KIND.eq(RequestKind::Replacement))
            .get(self.t.db())
            .await
            .unwrap()
    }
}

/// The text of a notice, title and body.
fn text(request: &NotifyRequest) -> String {
    request.data.to_string()
}

/// AC5: account manager A suspends the deployment with the reason *site
/// access withdrawn*; without a reason it is refused. The deployment is
/// `suspended` from the day, A is told, the client is told it is temporarily
/// unavailable without the reason, and the cover the contract requires is
/// open from the day, open-ended. A further reason is added with no second
/// message.
#[renox::test]
async fn the_account_manager_suspends_by_hand_and_a_reason_is_needed() {
    let w = world().await;
    let path = format!("/manpower/deployments/{}/suspend", w.d.id);
    let day = date(2027, 3, 1);
    w.t.on(day, async {
        w.t.post(&w.amal, &path, &[("reason", "  ")])
            .await
            .assert_status(422);
        assert_eq!(w.deployment().await.status, DeploymentStatus::Active);

        w.t.post(&w.amal, &path, &[("reason", "site access withdrawn")])
            .await
            .assert_status(303);
    })
    .await;

    let d = w.deployment().await;
    assert_eq!(d.status, DeploymentStatus::Suspended);
    assert_eq!(d.suspended_on, Some(day));
    let reasons = w.reasons().await;
    assert_eq!(reasons.len(), 1);
    assert_eq!(reasons[0].kind, SuspensionKind::Manual);
    assert_eq!(reasons[0].reason, "site access withdrawn");

    let client = w.client_notices();
    assert_eq!(client.len(), 1, "the client is told once");
    let told = text(&client[0]);
    assert!(
        told.contains("Ravi Kumar is temporarily unavailable"),
        "{told}"
    );
    assert!(told.contains("We will confirm the return date"), "{told}");
    assert!(
        !told.contains("site access"),
        "no reason for the client: {told}"
    );
    let staff = w.staff_notices();
    assert_eq!(staff.len(), 1);
    assert!(text(&staff[0]).contains("site access withdrawn"));
    assert_eq!(users_of(&staff[0]), vec![w.amal.id]);

    let covers = w.covers().await;
    assert_eq!(covers.len(), 1, "the contract requires cover");
    assert_eq!(covers[0].status, RequestStatus::Open);
    assert_eq!(covers[0].start_date, day);
    assert_eq!(covers[0].end_date, None);
    assert_eq!(covers[0].headcount, 1);
    assert_eq!(covers[0].role, "guard");
    assert_eq!(covers[0].site_id, w.site.id);
    assert_eq!(covers[0].replaces_deployment_id, Some(w.d.id));

    // Another reason on the suspended deployment: added, nothing sent.
    w.t.on(day, async {
        w.t.post(&w.amal, &path, &[("reason", "investigation")])
            .await
            .assert_status(303);
    })
    .await;
    assert_eq!(w.reasons().await.len(), 2);
    assert_eq!(w.client_notices().len(), 1);
    assert_eq!(w.staff_notices().len(), 1);
    assert_eq!(w.covers().await.len(), 1);
}

/// Only an `active` or `suspended` deployment is suspended, and only by the
/// account manager of its client.
#[renox::test]
async fn a_planned_deployment_is_not_suspended_by_hand_and_a_stranger_cannot() {
    let w = world().await;
    let planned =
        w.t.deployment(
            &w.contract,
            &w.site,
            w.w,
            Some(date(2027, 12, 31)),
            DeploymentStatus::Planned,
        )
        .await;
    w.t.post(
        &w.amal,
        &format!("/manpower/deployments/{}/suspend", planned.id),
        &[("reason", "investigation")],
    )
    .await
    .assert_status(422);
    let stranger = w.t.account_manager("Sara", PartyId(32)).await;
    w.t.post(
        &stranger,
        &format!("/manpower/deployments/{}/suspend", w.d.id),
        &[("reason", "investigation")],
    )
    .await
    .assert_status(403);
    assert_eq!(w.deployment().await.status, DeploymentStatus::Active);
}

/// AC1: the work permit of W expired on 2027-03-19 and the nightly run reports
/// it: D is `suspended` from 2027-03-20, the reason is the permit, the client
/// is told with the neutral text (no word of a permit), the account manager is
/// told which kind, and the replacement request is `open` from 2027-03-20.
#[renox::test]
async fn an_expired_work_permit_suspends_the_deployment_the_day_after() {
    let w = world().await;
    w.expire("work_permit").await;

    let d = w.deployment().await;
    assert_eq!(d.status, DeploymentStatus::Suspended);
    assert_eq!(d.suspended_on, Some(date(2027, 3, 20)));
    assert!(d.suspension_announced_at.is_some());
    let reasons = w.reasons().await;
    assert_eq!(reasons.len(), 1);
    assert_eq!(reasons[0].kind, SuspensionKind::Document);
    assert_eq!(reasons[0].reason, "work_permit");
    assert_eq!(reasons[0].expired_on, Some(date(2027, 3, 19)));

    let client = w.client_notices();
    assert_eq!(client.len(), 1);
    let told = text(&client[0]);
    assert!(
        told.contains("temporarily unavailable at Tower A from 2027-03-20"),
        "{told}"
    );
    assert!(!told.to_lowercase().contains("permit"), "{told}");
    let staff = w.staff_notices();
    assert!(
        staff.iter().any(|n| text(n).contains("work permit")),
        "the account manager is told the kind"
    );
    let covers = w.covers().await;
    assert_eq!(covers.len(), 1);
    assert_eq!(covers[0].status, RequestStatus::Open);
    assert_eq!(covers[0].start_date, date(2027, 3, 20));
    assert_eq!(covers[0].cause.as_deref(), Some("suspension"));
}

/// Idempotency: the same `DocumentExpired` twice suspends once, with one
/// message to the client and one cover request.
#[renox::test]
async fn the_same_expiry_twice_suspends_once() {
    let w = world().await;
    w.expire("work_permit").await;
    w.expire("work_permit").await;
    assert_eq!(w.reasons().await.len(), 1);
    assert_eq!(w.client_notices().len(), 1);
    assert_eq!(w.covers().await.len(), 1);
}

/// AC3: W's planned deployment D8 stays `planned` when the permit expires, and
/// nothing is sent for it; a kind the deployment does not need (a `flag` mode
/// entry, or no entry) suspends nothing.
#[renox::test]
async fn a_planned_deployment_and_a_kind_not_needed_are_left_alone() {
    let w = world().await;
    let w2 = w.t.staff.employ(&w.t.parties, 8, "Omar Said", true);
    let d8 =
        w.t.deployment(
            &w.contract,
            &w.site,
            w2,
            Some(date(2027, 12, 31)),
            DeploymentStatus::Planned,
        )
        .await;
    w.t.documents.expire(8, "work_permit");
    let mut event = expired("work_permit", blocking("work_permit", BlockMode::Block));
    event.subject = Subject::new("hr.employee", 8);
    w.t.state().emit(event).await.unwrap();
    assert_eq!(
        w.t.reload_deployment(d8.id).await.status,
        DeploymentStatus::Planned
    );
    assert!(w.t.notices.all().is_empty(), "nothing is sent for D8");

    // The attendance flag is not a block: D stays active.
    w.t.documents.expire(7, "iloe");
    w.t.state()
        .emit(expired("iloe", blocking("iloe", BlockMode::Flag)))
        .await
        .unwrap();
    w.t.state()
        .emit(expired("passport", Vec::new()))
        .await
        .unwrap();
    assert_eq!(w.deployment().await.status, DeploymentStatus::Active);
    assert!(w.t.notices.all().is_empty());
}

/// AC4: D is already suspended for the work permit; the health insurance
/// expiring adds a second reason and sends no second message to the client.
#[renox::test]
async fn a_further_expired_kind_is_added_without_a_second_message() {
    let w = world().await;
    w.expire("work_permit").await;
    w.expire("health_insurance").await;

    let reasons = w.reasons().await;
    let kinds: Vec<&str> = reasons.iter().map(|r| r.reason.as_str()).collect();
    assert_eq!(kinds, ["work_permit", "health_insurance"]);
    assert_eq!(w.client_notices().len(), 1, "the client is told once");
    assert_eq!(w.staff_notices().len(), 1);
    assert_eq!(w.covers().await.len(), 1);
    assert_eq!(w.deployment().await.status, DeploymentStatus::Suspended);
}

/// A renewal accepted before the night's run: the kind is valid again, so
/// nothing is suspended.
#[renox::test]
async fn a_kind_that_is_valid_again_suspends_nothing() {
    let w = world().await;
    w.t.documents.renew(7, "work_permit");
    w.t.state()
        .emit(expired(
            "work_permit",
            blocking("work_permit", BlockMode::Block),
        ))
        .await
        .unwrap();
    assert_eq!(w.deployment().await.status, DeploymentStatus::Active);
}

/// `extend` of a suspended deployment keeps it suspended.
#[renox::test]
async fn extend_of_a_suspended_deployment_keeps_it_suspended() {
    let w = world().await;
    w.expire("iloe").await;
    assert_eq!(w.deployment().await.status, DeploymentStatus::Suspended);
    w.t.on(date(2027, 3, 22), async {
        w.t.post(
            &w.amal,
            &format!("/manpower/deployments/{}/extend", w.d.id),
            &[("end_date", "2027-12-31")],
        )
        .await
        .assert_status(422);
    })
    .await;
    let short =
        w.t.deployment(
            &w.contract,
            &w.site,
            w.t.staff.employ(&w.t.parties, 9, "Imran Ali", true),
            Some(date(2027, 6, 30)),
            DeploymentStatus::Suspended,
        )
        .await;
    w.t.on(date(2027, 3, 22), async {
        w.t.post(
            &w.amal,
            &format!("/manpower/deployments/{}/extend", short.id),
            &[("end_date", "2027-09-30")],
        )
        .await
        .assert_status(303);
    })
    .await;
    let extended = w.t.reload_deployment(short.id).await;
    assert_eq!(extended.status, DeploymentStatus::Suspended);
    assert_eq!(extended.end_date, Some(date(2027, 9, 30)));
}

/// The deployments list shows staff why a deployment is suspended and for how
/// many days; a client user sees only the status.
#[renox::test]
async fn the_deployments_list_shows_the_days_suspended() {
    let w = world().await;
    w.expire("work_permit").await;
    // Suspended from 2027-03-20: on the 25th it is the sixth day.
    let rows =
        w.t.on(date(2027, 3, 25), async {
            w.t.get(&w.amal, "/manpower/deployments").await
        })
        .await;
    rows.assert_ok();
    let json: renox::serde_json::Value = rows.json();
    let row = &json[0];
    assert_eq!(row["status"], "Suspended");
    assert_eq!(row["days_suspended"], 6);
    let note = row["suspension"][0].as_str().unwrap();
    assert!(note.contains("work permit"), "{note}");
}

/// AC6: the notifier is down, so the follow-up of the run fails: the run
/// retries (the deployment is suspended already, the follow-up still to do).
/// When the notifier works the 02:00 run finishes it; a run that keeps failing
/// alerts the owner (the MD) after the fifth attempt.
#[renox::test]
async fn a_failing_run_is_retried_and_then_alerts_the_owner() {
    let w = world().await;
    w.t.notices.fail(true);
    w.expire("work_permit").await;
    let d = w.deployment().await;
    assert_eq!(d.status, DeploymentStatus::Suspended);
    assert!(
        d.suspension_announced_at.is_none(),
        "the follow-up is still to do"
    );
    assert!(w.client_notices().is_empty());

    // The notifier works again: the next 02:00 run finishes the follow-up once.
    w.t.notices.fail(false);
    w.t.app.travel(Duration::from_secs(61));
    w.t.app
        .at_travelled_time(suspension::retry(w.t.state()))
        .await
        .unwrap();
    assert!(w.deployment().await.suspension_announced_at.is_some());
    assert_eq!(w.client_notices().len(), 1);
    assert_eq!(w.covers().await.len(), 1);
    w.t.app
        .at_travelled_time(suspension::retry(w.t.state()))
        .await
        .unwrap();
    assert_eq!(w.client_notices().len(), 1, "never twice");
    assert!(
        ComplianceAlert::query()
            .where_(ComplianceAlert::RULE.eq("MPW.RUL.12"))
            .get(w.t.db())
            .await
            .unwrap()
            .is_empty()
    );

    // Another worker's run keeps failing: five attempts, then the alert.
    let md = w.t.md("Bader").await;
    renox::auth::permissions::define_role(w.t.db(), wl_kernel::COMPANY_ADMIN_ROLE, &[])
        .await
        .unwrap();
    let admin = w.t.user_with("Aisha", wl_kernel::COMPANY_ADMIN_ROLE).await;
    let w2 = w.t.staff.employ(&w.t.parties, 8, "Omar Said", true);
    let d2 =
        w.t.deployment(
            &w.contract,
            &w.site,
            w2,
            Some(date(2027, 12, 31)),
            DeploymentStatus::Active,
        )
        .await;
    w.t.notices.fail(true);
    w.t.notices
        .always_works(wl_kernel::compliance::service::NOTIFICATION_TYPE);
    w.t.documents.expire(8, "work_permit");
    let mut event = expired("work_permit", blocking("work_permit", BlockMode::Block));
    event.subject = Subject::new("hr.employee", 8);
    w.t.state().emit(event).await.unwrap();
    for attempt in 1..=5 {
        w.t.app.travel(Duration::from_secs(2 * 60 * 60));
        // A follow-up that fails is kept for the next attempt and the fifth
        // gives up with the alert: the run answers `Ok`, so an `Err` would be
        // a fault of the rule and must not hide behind the alert below.
        w.t.app
            .at_travelled_time(suspension::retry(w.t.state()))
            .await
            .unwrap_or_else(|error| panic!("attempt {attempt} ended in an error: {error}"));
    }
    let alerts = ComplianceAlert::query()
        .where_(ComplianceAlert::RULE.eq("MPW.RUL.12"))
        .get(w.t.db())
        .await
        .unwrap();
    assert_eq!(alerts.len(), 1, "the owner is alerted once");
    assert_eq!(alerts[0].subject_id, d2.id);
    assert_eq!(alerts[0].owner_role, "md");
    // The kernel tells the MD and the company admin (`PLT.RUL.04`), once.
    let told =
        w.t.notices
            .of_type(wl_kernel::compliance::service::NOTIFICATION_TYPE);
    assert_eq!(told.len(), 1);
    let mut recipients = users_of(&told[0]);
    recipients.sort_unstable();
    let mut wanted = vec![md.id, admin.id];
    wanted.sort_unstable();
    assert_eq!(recipients, wanted, "the MD and the company admin");
}
