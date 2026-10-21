//! BL-MPW-14 (#428), 09 Manpower §4.1 rows *The client stops paying* and *The
//! client rejects a deployed worker*, §7.1 *Deployment* `end`: the account
//! manager ends a deployment for the client's rejection (with the client's
//! note, the replacement request when the contract requires cover, the worker
//! back on the bench), the MD alone withdraws a worker for client non-payment
//! (the notice period, or a waiver), and a deployment entered in error is
//! ended. The conduct ticket and the rejected attendance, with the real
//! wl-ticketing and wl-attendance, are in `host/tests/manpower_end_reasons.rs`.

mod support;

use manpower::{
    ClientContract, ContractStatus, Deployment, DeploymentStatus, ManpowerRequest, RequestKind,
    RequestStatus, Site,
};
use renox::prelude::*;
use wl_kernel::workforce::EmployeeId;

use support::{Booted, TOWER, boot, date};

/// Amal, the MD, a contract with a 30-day notice period and the cover
/// obligation `cover`, a site and worker W (employee 7) on site until
/// 2027-07-31.
struct World {
    t: Booted,
    amal: User,
    md: User,
    contract: ClientContract,
    d: Deployment,
    w: EmployeeId,
}

async fn world(cover: bool) -> World {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    // The MD also manages the deployments of the client (the permission is
    // needed besides the role).
    let md = t.md("Basel").await;
    md.assign_role_in(
        t.db(),
        manpower::permissions::ACCOUNT_MANAGER_ROLE,
        &manpower::permissions::client_scope(TOWER.0),
    )
    .await
    .unwrap();
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    ClientContract::query()
        .where_(ClientContract::ID.eq(contract.id))
        .update(t.db(), &[("cover_required", &cover)])
        .await
        .unwrap();
    let site: Site = t.site(TOWER, "Tower A").await;
    let w = t.staff.employ(&t.parties, 7, "Ravi Kumar", true);
    let d = t
        .deployment(
            &contract,
            &site,
            w,
            Some(date(2027, 7, 31)),
            DeploymentStatus::Active,
        )
        .await;
    World {
        t,
        amal,
        md,
        contract,
        d,
        w,
    }
}

impl World {
    fn path(&self) -> String {
        format!("/manpower/deployments/{}/end", self.d.id)
    }

    async fn covers(&self) -> Vec<ManpowerRequest> {
        ManpowerRequest::query()
            .where_(ManpowerRequest::KIND.eq(RequestKind::Replacement))
            .get(self.t.db())
            .await
            .unwrap()
    }

    async fn deployment(&self) -> Deployment {
        self.t.reload_deployment(self.d.id).await
    }
}

/// AC1: D of W at client C (cover on); account manager A ends it with the
/// reason *client rejected* and the note "late three times": D is `ended`
/// with the note, a replacement request for one worker from tomorrow to D's
/// end date is `open`, W is on the bench and the client is notified. Without
/// the note it is refused.
#[renox::test]
async fn a_rejected_worker_is_ended_with_the_note_and_the_replacement_opens() {
    let w = world(true).await;
    let today = date(2027, 3, 10);
    w.t.on(today, async {
        w.t.post(&w.amal, &w.path(), &[("reason", "client_rejected")])
            .await
            .assert_status(422);
        assert_eq!(w.deployment().await.status, DeploymentStatus::Active);
        w.t.post(
            &w.amal,
            &w.path(),
            &[("reason", "client_rejected"), ("note", "late three times")],
        )
        .await
        .assert_status(303);
    })
    .await;

    let d = w.deployment().await;
    assert_eq!(d.status, DeploymentStatus::Ended);
    assert_eq!(d.end_reason.as_deref(), Some("client_rejected"));
    assert_eq!(d.end_note.as_deref(), Some("late three times"));
    assert_eq!(d.ended_on, Some(today));

    let covers = w.covers().await;
    assert_eq!(covers.len(), 1);
    assert_eq!(covers[0].status, RequestStatus::Open);
    assert_eq!(covers[0].headcount, 1);
    assert_eq!(covers[0].start_date, date(2027, 3, 11), "from tomorrow");
    assert_eq!(covers[0].end_date, Some(date(2027, 7, 31)));
    assert_eq!(covers[0].role, "guard");
    assert_eq!(covers[0].site_id, w.d.site_id);
    assert_eq!(covers[0].cause.as_deref(), Some("rejection"));
    assert_eq!(w.t.covers.all().len(), 1);

    let bench = manpower::bench::of(w.t.state(), &[w.w]).await.unwrap();
    assert_eq!(bench, vec![w.w], "W is back on the bench");
    assert_eq!(w.t.notices.of_type("manpower.client_notice").len(), 1);
    assert_eq!(w.t.ended.all().len(), 1);
    assert_eq!(w.t.ended.all()[0].reason, "client_rejected");
}

/// AC3 (the Must not): with the cover obligation off, no request is created.
#[renox::test]
async fn a_rejection_opens_no_replacement_when_the_contract_requires_no_cover() {
    let w = world(false).await;
    w.t.on(date(2027, 3, 10), async {
        w.t.post(
            &w.amal,
            &w.path(),
            &[("reason", "client_rejected"), ("note", "rude to staff")],
        )
        .await
        .assert_status(303);
    })
    .await;
    assert_eq!(w.deployment().await.status, DeploymentStatus::Ended);
    assert!(w.covers().await.is_empty());
    assert_eq!(w.t.covers.all().len(), 0);
}

/// A deployment that ends today or in the past needs no cover.
#[renox::test]
async fn a_rejection_on_the_last_day_needs_no_replacement() {
    let w = world(true).await;
    w.t.on(date(2027, 7, 31), async {
        w.t.post(
            &w.amal,
            &w.path(),
            &[("reason", "client_rejected"), ("note", "no show")],
        )
        .await
        .assert_status(303);
    })
    .await;
    assert!(w.covers().await.is_empty());
}

/// AC4: a user who is not the MD cannot choose *client non-payment*, not
/// even the account manager of the client. The MD with an end date 10 days
/// ahead on a 30-day notice contract is refused without a waiver and the
/// refusal says the earliest day; a past date is refused.
#[renox::test]
async fn only_the_md_withdraws_a_worker_and_the_notice_period_is_kept() {
    let w = world(true).await;
    let today = date(2027, 3, 10);
    w.t.on(today, async {
        w.t.post(
            &w.amal,
            &w.path(),
            &[("reason", "client_non_payment"), ("end_date", "2027-04-20")],
        )
        .await
        .assert_status(403);

        let refused =
            w.t.post(
                &w.md,
                &w.path(),
                &[("reason", "client_non_payment"), ("end_date", "2027-03-20")],
            )
            .await;
        refused.assert_status(422);
        assert!(refused.text().contains("2027-04-09"), "{}", refused.text());
        w.t.post(
            &w.md,
            &w.path(),
            &[("reason", "client_non_payment"), ("end_date", "2027-03-01")],
        )
        .await
        .assert_status(422);
    })
    .await;
    let d = w.deployment().await;
    assert_eq!(d.status, DeploymentStatus::Active);
    assert_eq!(d.end_date, Some(date(2027, 7, 31)));
    assert_eq!(d.planned_end_reason, None);
}

/// The MD gives the notice: the worker stays until the end date (the
/// deployment is `active` with the new end date and the client is told), and
/// the daily run ends it on the day after with the reason *client
/// non-payment*.
#[renox::test]
async fn the_withdrawal_for_non_payment_ends_the_deployment_after_the_notice_period() {
    let w = world(true).await;
    w.t.on(date(2027, 3, 10), async {
        w.t.post(
            &w.md,
            &w.path(),
            &[
                ("reason", "client_non_payment"),
                ("end_date", "2027-04-09"),
                ("note", "invoices 3 months overdue"),
            ],
        )
        .await
        .assert_status(303);
    })
    .await;
    let scheduled = w.deployment().await;
    assert_eq!(scheduled.status, DeploymentStatus::Active);
    assert_eq!(scheduled.end_date, Some(date(2027, 4, 9)));
    assert_eq!(
        scheduled.planned_end_reason.as_deref(),
        Some("client_non_payment")
    );
    let told = w.t.notices.of_type("manpower.client_notice");
    assert_eq!(told.len(), 1);
    assert!(
        told[0].data.to_string().contains("2027-04-09"),
        "{}",
        told[0].data
    );

    w.t.on(date(2027, 4, 10), async {
        manpower::deployments::daily(w.t.state()).await.unwrap();
    })
    .await;
    let ended = w.deployment().await;
    assert_eq!(ended.status, DeploymentStatus::Ended);
    assert_eq!(ended.end_reason.as_deref(), Some("client_non_payment"));
    assert_eq!(ended.end_date, Some(date(2027, 4, 9)));
    assert!(w.covers().await.is_empty(), "non-payment opens no cover");
}

/// A waiver attached by the MD allows a short notice: with the end date today
/// the deployment ends at once, the waiver is kept.
#[renox::test]
async fn a_waiver_lets_the_md_withdraw_the_worker_now() {
    let w = world(true).await;
    w.t.on(date(2027, 3, 10), async {
        w.t.post(
            &w.md,
            &w.path(),
            &[("reason", "client_non_payment"), ("waiver", "WAIVER-7.pdf")],
        )
        .await
        .assert_status(303);
    })
    .await;
    let d = w.deployment().await;
    assert_eq!(d.status, DeploymentStatus::Ended);
    assert_eq!(d.end_reason.as_deref(), Some("client_non_payment"));
    assert_eq!(d.end_waiver.as_deref(), Some("WAIVER-7.pdf"));
    assert_eq!(d.ended_on, Some(date(2027, 3, 10)));
    let _ = w.contract.id;
}

/// AC5: a deployment entered in error is ended with that reason and the
/// worker is back on the bench; the attendance is rejected in the host test.
#[renox::test]
async fn a_deployment_entered_in_error_is_ended_with_that_reason() {
    let w = world(true).await;
    w.t.on(date(2027, 3, 10), async {
        w.t.post(&w.amal, &w.path(), &[("reason", "entered_in_error")])
            .await
            .assert_status(303);
    })
    .await;
    let d = w.deployment().await;
    assert_eq!(d.status, DeploymentStatus::Ended);
    assert_eq!(d.end_reason.as_deref(), Some("entered_in_error"));
    assert!(w.covers().await.is_empty(), "a mistake needs no cover");
}
