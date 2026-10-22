//! BL-MPW-13 (#427), 09 Manpower §4.2, §4.7, §7.1 *Deployment* `end` and
//! `cancel`, `MPW.RUL.12` (the exit part): the deployments follow the
//! non-standard exits of the worker. wl-hr's events are emitted by the test as
//! wl-hr emits them (the chain with the real wl-hr is in
//! `host/tests/manpower_exits.rs`).

mod support;

use std::time::Duration;

use manpower::{
    ClientContract, ContractStatus, Deployment, DeploymentStatus, ManpowerRequest, RequestStatus,
    Site, exits,
};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::notify::NotifyRequest;
use wl_kernel::workforce::{
    EmployeeAbsconded, EmployeeDeceased, EmployeeId, EmployeeNoticeGiven, EmployeeNoticeWithdrawn,
    EmployeeOnboardingCancelled, EmployeeReinstated, VisaCaseRejected,
};

use support::{Booted, TOWER, boot, date, users_of};

/// What the tests share: Amal, the MD, a contract, a site, a request for two
/// guards and worker W (employee 7).
struct World {
    t: Booted,
    amal: User,
    md: User,
    contract: ClientContract,
    site: Site,
    request: ManpowerRequest,
    w: EmployeeId,
}

async fn world() -> World {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let md = t.md("Basel").await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let site = t.site(TOWER, "Tower A").await;
    let request = t.request(&contract, &site, 2, RequestStatus::Open).await;
    let w = t.staff.employ(&t.parties, 7, "Ravi Kumar", true);
    World {
        t,
        amal,
        md,
        contract,
        site,
        request,
        w,
    }
}

impl World {
    /// D1: W is on site until the end of the contract, filling the request.
    async fn on_site(&self) -> Deployment {
        let d = self
            .t
            .deployment(
                &self.contract,
                &self.site,
                self.w,
                Some(date(2027, 12, 31)),
                DeploymentStatus::Active,
            )
            .await;
        self.t.fills(&d, &self.request).await;
        self.t.reload_deployment(d.id).await
    }

    /// A deployment of W in `status` at the site, ending `end`.
    async fn deployment(&self, status: DeploymentStatus, end: Option<NaiveDate>) -> Deployment {
        self.t
            .deployment(&self.contract, &self.site, self.w, end, status)
            .await
    }

    async fn reload(&self, d: &Deployment) -> Deployment {
        self.t.reload_deployment(d.id).await
    }

    fn client_notices(&self) -> Vec<NotifyRequest> {
        self.t.notices.of_type("manpower.client_notice")
    }

    fn staff_notices(&self) -> Vec<NotifyRequest> {
        self.t.notices.of_type("manpower.notice")
    }
}

fn text(request: &NotifyRequest) -> String {
    request.data.to_string()
}

fn absconded(on: NaiveDate) -> EmployeeAbsconded {
    EmployeeAbsconded {
        employee: EmployeeId(7),
        report_number: "MOHRE-777".into(),
        report_date: on,
    }
}

/// AC1: W has an active D1 and a planned D2; `EmployeeAbsconded` with the
/// report date 2027-03-09 ends D1 with the reason *absconded* and that last
/// day, cancels D2, closes the last timesheet, tells the client only that W is
/// no longer available and tells the account manager the real reason. The same
/// event again changes nothing and sends nothing.
#[renox::test]
async fn an_absconding_worker_ends_the_active_and_cancels_the_planned_deployment() {
    let w = world().await;
    let d1 = w.on_site().await;
    let d2 = w.deployment(DeploymentStatus::Planned, None).await;
    w.t.fills(&d2, &w.request).await;

    let day = date(2027, 3, 9);
    w.t.on(day, async {
        w.t.state().emit(absconded(day)).await.unwrap();
    })
    .await;

    let ended = w.reload(&d1).await;
    assert_eq!(ended.status, DeploymentStatus::Ended);
    assert_eq!(ended.end_reason.as_deref(), Some("absconded"));
    assert_eq!(ended.end_date, Some(day));
    assert_eq!(ended.ended_on, Some(day));
    let cancelled = w.reload(&d2).await;
    assert_eq!(cancelled.status, DeploymentStatus::Cancelled);
    assert_eq!(cancelled.cancel_reason.as_deref(), Some("worker absconded"));
    assert_eq!(w.t.ended.all().len(), 1);
    assert_eq!(w.t.ended.all()[0].reason, "absconded");
    assert_eq!(w.t.cancelled.all().len(), 1);
    assert_eq!(w.t.closings.all().len(), 1, "the last timesheet is closed");
    assert_eq!(w.t.closings.all()[0].2, day);

    // The client: no longer available, never the reason; one message for each
    // deployment the client knows (both fill the client's request).
    let client = w.client_notices();
    assert_eq!(client.len(), 2, "{client:?}");
    for told in &client {
        let body = text(told);
        assert!(body.contains("Ravi Kumar is no longer available"), "{body}");
        for word in ["abscond", "MOHRE", "left"] {
            assert!(!body.to_lowercase().contains(word), "{body}");
        }
    }
    // The account manager: the real reason, the ended and cancelled
    // deployments and the request that does not reopen.
    let staff = w.staff_notices();
    assert_eq!(staff.len(), 1, "{staff:?}");
    assert_eq!(users_of(&staff[0]), vec![w.amal.id]);
    let told = text(&staff[0]);
    assert!(told.contains("worker absconded"), "{told}");
    assert!(told.contains("Deployment ended at Tower A"), "{told}");
    assert!(told.contains("Planned deployment at Tower A"), "{told}");
    assert!(
        told.contains(&format!("Request #{}", w.request.id)),
        "{told}"
    );
    assert_eq!(
        w.t.reload_request(w.request.id).await.status,
        RequestStatus::Open,
        "the request does not reopen or close"
    );

    // A repeated event changes nothing.
    w.t.on(day, async {
        w.t.state().emit(absconded(day)).await.unwrap();
    })
    .await;
    assert_eq!(w.t.ended.all().len(), 1);
    assert_eq!(w.client_notices().len(), 2);
    assert_eq!(w.staff_notices().len(), 1);
    assert_eq!(w.t.closings.all().len(), 1);
}

/// AC2: `EmployeeDeceased` ends D1 with the reason *deceased* on the date of
/// death; the client gets the same neutral text, with no mention of death.
#[renox::test]
async fn a_death_ends_the_deployment_with_the_reason_deceased() {
    let w = world().await;
    let d1 = w.on_site().await;
    let day = date(2027, 4, 2);
    w.t.on(day, async {
        w.t.state()
            .emit(EmployeeDeceased {
                employee: w.w,
                date: day - renox::chrono::Days::new(1),
            })
            .await
            .unwrap();
    })
    .await;
    let ended = w.reload(&d1).await;
    assert_eq!(ended.status, DeploymentStatus::Ended);
    assert_eq!(ended.end_reason.as_deref(), Some("deceased"));
    assert_eq!(ended.end_date, Some(date(2027, 4, 1)));
    let client = w.client_notices();
    assert_eq!(client.len(), 1);
    let body = text(&client[0]);
    assert!(body.contains("no longer available"), "{body}");
    assert!(
        !body.to_lowercase().contains("death") && !body.to_lowercase().contains("died"),
        "{body}"
    );
    let staff = w.staff_notices();
    assert!(text(&staff[0]).contains("worker deceased"));
}

/// A suspended deployment goes with the worker, too.
#[renox::test]
async fn a_suspended_deployment_ends_on_an_absconding_report() {
    let w = world().await;
    let d = w.deployment(DeploymentStatus::Suspended, None).await;
    let day = date(2027, 3, 9);
    w.t.on(day, async {
        w.t.state().emit(absconded(day)).await.unwrap();
    })
    .await;
    let ended = w.reload(&d).await;
    assert_eq!(ended.status, DeploymentStatus::Ended);
    assert_eq!(ended.end_date, Some(day), "an open-ended deployment stops");
}

/// AC3: `EmployeeOnboardingCancelled` cancels the planned D3 whose start was
/// communicated (it fills the client's request), tells the client, and tells
/// the account manager that request R stays unfilled; a planned deployment
/// that fills no request is cancelled without a word to the client.
#[renox::test]
async fn a_cancelled_onboarding_cancels_the_planned_deployments() {
    let w = world().await;
    let d3 = w.deployment(DeploymentStatus::Planned, None).await;
    w.t.fills(&d3, &w.request).await;
    let d3b = w.deployment(DeploymentStatus::Planned, None).await;
    let active = w.on_site().await;

    w.t.on(date(2027, 1, 20), async {
        w.t.state()
            .emit(EmployeeOnboardingCancelled {
                employee: w.w,
                reason: "visa refused".into(),
            })
            .await
            .unwrap();
    })
    .await;

    assert_eq!(w.reload(&d3).await.status, DeploymentStatus::Cancelled);
    assert_eq!(w.reload(&d3b).await.status, DeploymentStatus::Cancelled);
    assert_eq!(
        w.reload(&d3).await.cancel_reason.as_deref(),
        Some("onboarding cancelled")
    );
    assert_eq!(
        w.reload(&active).await.status,
        DeploymentStatus::Active,
        "only the planned ones go"
    );
    assert_eq!(
        w.client_notices().len(),
        1,
        "only D3's start was communicated"
    );
    let staff = w.staff_notices();
    assert_eq!(staff.len(), 1);
    assert!(
        text(&staff[0]).contains(&format!("Request #{} stays unfilled", w.request.id)),
        "{}",
        text(&staff[0])
    );
    assert_eq!(
        w.t.reload_request(w.request.id).await.status,
        RequestStatus::Open
    );
}

/// AC4: D4 ended on 2027-09-30 before `give_notice` set the last working day
/// 2027-06-15; the withdrawn notice restores 2027-09-30. D4' had no end date:
/// it is empty again. A deployment extended meanwhile keeps its date. The
/// client and the account manager are told; a repeated event changes nothing.
#[renox::test]
async fn a_withdrawn_notice_restores_the_end_date_it_replaced() {
    let w = world().await;
    let d4 = w
        .deployment(DeploymentStatus::Active, Some(date(2027, 9, 30)))
        .await;
    let d4b = w.deployment(DeploymentStatus::Planned, None).await;
    let early = w
        .deployment(DeploymentStatus::Active, Some(date(2027, 5, 31)))
        .await;
    let notice = EmployeeNoticeGiven {
        employee: w.w,
        last_working_day: date(2027, 6, 15),
    };
    w.t.on(date(2027, 5, 1), async {
        w.t.state().emit(notice.clone()).await.unwrap();
        // A second notice event keeps the first end date.
        w.t.state().emit(notice).await.unwrap();
    })
    .await;
    assert_eq!(w.reload(&d4).await.end_date, Some(date(2027, 6, 15)));
    assert_eq!(w.reload(&d4b).await.end_date, Some(date(2027, 6, 15)));
    assert_eq!(
        w.reload(&early).await.end_date,
        Some(date(2027, 5, 31)),
        "an earlier end date stays"
    );
    assert!(
        w.client_notices().is_empty(),
        "the notice tells the client nothing"
    );

    w.t.on(date(2027, 5, 10), async {
        w.t.state()
            .emit(EmployeeNoticeWithdrawn { employee: w.w })
            .await
            .unwrap();
    })
    .await;
    assert_eq!(w.reload(&d4).await.end_date, Some(date(2027, 9, 30)));
    assert_eq!(w.reload(&d4b).await.end_date, None);
    assert_eq!(w.reload(&early).await.end_date, Some(date(2027, 5, 31)));
    assert_eq!(w.reload(&d4).await.status, DeploymentStatus::Active);
    assert_eq!(w.client_notices().len(), 2, "one per deployment restored");
    let staff = w.staff_notices();
    assert_eq!(staff.len(), 1);
    assert!(
        text(&staff[0]).contains("2027-06-15"),
        "{}",
        text(&staff[0])
    );

    // Again: nothing to restore, nothing sent.
    w.t.on(date(2027, 5, 10), async {
        exits::restore_after_notice(w.t.state(), w.w).await.unwrap();
    })
    .await;
    assert_eq!(w.client_notices().len(), 2);
    assert_eq!(w.staff_notices().len(), 1);
}

/// A deployment extended after the notice keeps the date it was extended to.
#[renox::test]
async fn a_deployment_extended_since_the_notice_keeps_its_end_date() {
    let w = world().await;
    let d = w
        .deployment(DeploymentStatus::Active, Some(date(2027, 9, 30)))
        .await;
    w.t.on(date(2027, 5, 1), async {
        w.t.state()
            .emit(EmployeeNoticeGiven {
                employee: w.w,
                last_working_day: date(2027, 6, 15),
            })
            .await
            .unwrap();
    })
    .await;
    Deployment::query()
        .where_(Deployment::ID.eq(d.id))
        .update(w.t.db(), &[("end_date", &Some(date(2027, 8, 1)))])
        .await
        .unwrap();
    w.t.on(date(2027, 5, 2), async {
        w.t.state()
            .emit(EmployeeNoticeWithdrawn { employee: w.w })
            .await
            .unwrap();
    })
    .await;
    let after = w.reload(&d).await;
    assert_eq!(after.end_date, Some(date(2027, 8, 1)));
    assert_eq!(after.notice_last_day, None);
}

/// AC5 (the Must not): after `EmployeeReinstated`, D1 stays `ended` and D2
/// `cancelled`; the account manager plans a new deployment.
#[renox::test]
async fn a_reinstated_worker_gets_no_deployment_back() {
    let w = world().await;
    let d1 = w.on_site().await;
    let d2 = w.deployment(DeploymentStatus::Planned, None).await;
    let day = date(2027, 3, 9);
    w.t.on(day, async {
        w.t.state().emit(absconded(day)).await.unwrap();
        w.t.state()
            .emit(EmployeeReinstated {
                employee: w.w,
                return_date: day,
            })
            .await
            .unwrap();
    })
    .await;
    assert_eq!(w.reload(&d1).await.status, DeploymentStatus::Ended);
    assert_eq!(w.reload(&d2).await.status, DeploymentStatus::Cancelled);
}

/// AC6: W's residence visa expires 2027-08-31; D5 runs to 2027-12-31 and D6
/// to 2027-08-15. `VisaCaseRejected` (renewal) tells D5's account manager and
/// the MD about D5 only; D5 stays `active`, the client is told nothing. A new
/// or cancellation case changes nothing.
#[renox::test]
async fn a_rejected_renewal_lists_the_deployments_past_the_visa_to_the_staff() {
    let w = world().await;
    let d5 = w
        .deployment(DeploymentStatus::Active, Some(date(2027, 12, 31)))
        .await;
    let d6 = w
        .deployment(DeploymentStatus::Active, Some(date(2027, 8, 15)))
        .await;
    let rejected = |case_type: &str| VisaCaseRejected {
        case: Subject::new("hr.visa_case", 7),
        case_type: case_type.into(),
        employee: w.w,
        deadline: Some(date(2027, 8, 1)),
        visa_expiry: (case_type == "renewal").then(|| date(2027, 8, 31)),
    };
    for ignored in ["new", "cancellation"] {
        w.t.state().emit(rejected(ignored)).await.unwrap();
    }
    assert!(w.staff_notices().is_empty());

    w.t.state().emit(rejected("renewal")).await.unwrap();
    let staff = w.staff_notices();
    assert_eq!(staff.len(), 2, "the account manager and the MD: {staff:?}");
    let mut to: Vec<i64> = staff.iter().flat_map(users_of).collect();
    to.sort_unstable();
    let mut wanted = vec![w.amal.id, w.md.id];
    wanted.sort_unstable();
    assert_eq!(to, wanted);
    for told in &staff {
        let body = text(told);
        assert!(body.contains("2027-08-31"), "{body}");
        assert!(body.contains("ends 2027-12-31"), "{body}");
        assert!(
            !body.contains("2027-08-15"),
            "D6 ends before the visa: {body}"
        );
    }
    assert!(w.client_notices().is_empty());
    assert_eq!(w.reload(&d5).await.status, DeploymentStatus::Active);
    assert_eq!(w.reload(&d6).await.status, DeploymentStatus::Active);
}

/// The follow-up of an exit fails (the notifier is down): the deployment is
/// ended already, the 02:00 run finishes the client and account manager
/// notices once and never twice.
#[renox::test]
async fn a_failed_follow_up_is_finished_by_the_nightly_retry_once() {
    let w = world().await;
    let d1 = w.on_site().await;
    let day = date(2027, 3, 9);
    w.t.notices.fail(true);
    w.t.on(day, async {
        w.t.state().emit(absconded(day)).await.unwrap();
    })
    .await;
    assert_eq!(w.reload(&d1).await.status, DeploymentStatus::Ended);
    assert!(w.client_notices().is_empty());

    w.t.notices.fail(false);
    w.t.app.travel(Duration::from_secs(61));
    for _ in 0..2 {
        w.t.app
            .at_travelled_time(manpower::deployments::daily(w.t.state()))
            .await
            .unwrap();
    }
    assert_eq!(w.client_notices().len(), 1);
    assert_eq!(w.staff_notices().len(), 1);
}
