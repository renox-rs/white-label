//! BL-MPW-17 (#431), 09 Manpower §4.7 row *The worker absconds*, §7.2
//! `MPW.RUL.09`: a worker with an active deployment who has not attended for
//! more than 7 calendar days gets an absence case (a ticket for HR), the cover
//! request opens when the contract requires it, the client is told the worker
//! is absent, wl-hr is told (it holds the pay), HR is reminded daily until two
//! failed attempts on different days and the MoHRE report number are recorded,
//! and HR closes the case when the worker returns. The real wl-hr, wl-ticketing
//! and wl-attendance are in `host/tests/manpower_absence.rs`.

mod support;

use std::time::Duration;

use manpower::absence::{self, AbsenceCase};
use manpower::{
    ClientContract, ContractStatus, Deployment, DeploymentStatus, ManpowerRequest, RequestKind,
    RequestStatus, Site,
};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::compliance::ComplianceAlert;
use wl_kernel::notify::NotifyRequest;
use wl_kernel::workforce::{EmployeeId, EmployeeReinstated};

use support::{Booted, TOWER, boot, date, users_of};

/// Amal, HR (Hana), a contract that requires cover, a site and worker W
/// (employee 7) on site since 2027-02-01, active in HR.
struct World {
    t: Booted,
    hana: User,
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
    renox::auth::permissions::define_role(
        t.db(),
        manpower::seed::ABSENCE_ROLE,
        &[manpower::permissions::ABSENCE_MANAGE],
    )
    .await
    .unwrap();
    let hana = t.user_with("Hana", manpower::seed::ABSENCE_ROLE).await;
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
        hana,
        contract,
        site,
        w,
        d,
    }
}

impl World {
    /// The rule at 02:00 on `day`.
    async fn run(&self, day: NaiveDate) -> absence::Run {
        self.t
            .on(day, async { absence::run_on(self.t.state(), day).await })
            .await
            .unwrap()
    }

    fn client(&self) -> Vec<NotifyRequest> {
        self.t
            .notices
            .of_type("manpower.client_notice")
            .into_iter()
            .filter(|n| n.data.to_string().contains("not turned up"))
            .collect()
    }

    /// The reminders to HR.
    fn reminders(&self) -> Vec<NotifyRequest> {
        self.t
            .notices
            .of_type("manpower.notice")
            .into_iter()
            .filter(|n| n.data.to_string().contains("Failed attempts to reach"))
            .collect()
    }

    async fn case(&self) -> AbsenceCase {
        AbsenceCase::open_of(self.t.db(), self.w.0)
            .await
            .unwrap()
            .expect("an open case")
    }

    async fn covers(&self) -> Vec<ManpowerRequest> {
        ManpowerRequest::query()
            .where_(ManpowerRequest::KIND.eq(RequestKind::Replacement))
            .get(self.t.db())
            .await
            .unwrap()
    }
}

/// AC1 and AC2: W last attended on Saturday 2027-03-06 (day 0); at the run of
/// 2027-03-13 (day 7) nothing happens; at the run of 2027-03-14 the case opens:
/// a ticket for HR about W, wl-hr told, a cover request from that day, the
/// client told with no reason, HR reminded, the deployment still `active`. A
/// second run the same day changes nothing.
#[renox::test]
async fn the_case_opens_at_the_first_run_after_the_seventh_day() {
    let w = world().await;
    w.t.history.attended(7, date(2027, 3, 6));

    let early = w.run(date(2027, 3, 13)).await;
    assert!(early.opened.is_empty());
    assert!(w.t.tickets.all().is_empty());

    let run = w.run(date(2027, 3, 14)).await;
    assert_eq!(run.opened.len(), 1);
    let case = w.case().await;
    assert_eq!(case.last_present_on, date(2027, 3, 6));
    assert_eq!(case.opened_on, date(2027, 3, 14));
    assert_eq!(case.deployment_id, w.d.id);

    let tickets = w.t.tickets.all();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].ticket.category_key, "absence");
    assert_eq!(tickets[0].ticket.subject, Subject::new("hr.employee", 7));
    assert_eq!(
        tickets[0].ticket.source,
        Some(Subject::new("manpower.deployment", w.d.id))
    );
    assert!(tickets[0].ticket.title.contains("Ravi Kumar"));
    assert_eq!(case.ticket_id, Some(tickets[0].id));

    let opened = w.t.case_opened.all();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].employee, w.w);
    assert_eq!(opened[0].last_present_on, date(2027, 3, 6));
    assert_eq!(
        opened[0].case,
        Subject::new("ticketing.ticket", tickets[0].id)
    );

    let covers = w.covers().await;
    assert_eq!(covers.len(), 1);
    assert_eq!(covers[0].status, RequestStatus::Open);
    assert_eq!(covers[0].start_date, date(2027, 3, 14));
    assert_eq!(covers[0].end_date, None);
    assert_eq!(covers[0].cause.as_deref(), Some("absence"));

    let client = w.client();
    assert_eq!(client.len(), 1);
    let told = client[0].data.to_string();
    assert!(
        told.contains("Ravi Kumar has not turned up at Tower A"),
        "{told}"
    );
    assert!(!told.to_lowercase().contains("abscond"), "{told}");
    assert_eq!(w.reminders().len(), 1);
    assert_eq!(users_of(&w.reminders()[0]), vec![w.hana.id], "HR is told");
    assert_eq!(
        w.t.reload_deployment(w.d.id).await.status,
        DeploymentStatus::Active,
        "the case does not stop the deployment"
    );

    let again = w.run(date(2027, 3, 14)).await;
    assert!(again.opened.is_empty() && again.reminded.is_empty());
    assert_eq!(w.t.tickets.all().len(), 1);
    assert_eq!(w.t.case_opened.all().len(), 1);
    assert_eq!(w.covers().await.len(), 1);
    assert_eq!(w.client().len(), 1);
}

/// AC3: W's deployment was suspended and resumed on 2027-03-03 and W never
/// attends: day 0 is 03-03, so no case at the runs of 03-08 and 03-10 and the
/// case opens at the run of 03-11.
#[renox::test]
async fn the_count_starts_again_at_a_resume() {
    let w = world().await;
    Deployment::query()
        .where_(Deployment::ID.eq(w.d.id))
        .update(
            w.t.db(),
            &[
                ("activated_on", &Some(date(2027, 2, 1))),
                ("resumed_on", &Some(date(2027, 3, 3))),
            ],
        )
        .await
        .unwrap();
    w.t.history.attended(7, date(2027, 2, 20));
    assert!(w.run(date(2027, 3, 8)).await.opened.is_empty());
    assert!(w.run(date(2027, 3, 10)).await.opened.is_empty());
    let run = w.run(date(2027, 3, 11)).await;
    assert_eq!(run.opened.len(), 1);
    assert_eq!(w.case().await.last_present_on, date(2027, 3, 3));
}

/// A worker on the bench or whose deployment is suspended has no attendance
/// expected, and one who is not `active` in HR is not this rule's.
#[renox::test]
async fn nobody_without_an_active_deployment_gets_a_case() {
    let w = world().await;
    Deployment::query()
        .where_(Deployment::ID.eq(w.d.id))
        .update(w.t.db(), &[("suspended_days", &0_i64)])
        .await
        .unwrap();
    let bench = w.t.staff.employ(&w.t.parties, 8, "Omar Said", true);
    w.t.deployment(
        &w.contract,
        &w.site,
        bench,
        None,
        DeploymentStatus::Suspended,
    )
    .await;
    let gone = w.t.staff.employ(&w.t.parties, 9, "Sami Aziz", false);
    w.t.deployment(&w.contract, &w.site, gone, None, DeploymentStatus::Active)
        .await;
    w.t.history.attended(7, date(2027, 3, 13));
    let run = w.run(date(2027, 3, 14)).await;
    assert!(run.opened.is_empty(), "{:?}", run.opened);
    assert!(w.t.tickets.all().is_empty());
}

/// Approved leave counts as present: the days are counted from the end of the
/// leave (the kernel's leave calendar); here the fake has none, so a worker
/// attending on the 13th is not absent on the 14th.
#[renox::test]
async fn attendance_yesterday_means_no_case() {
    let w = world().await;
    w.t.history.attended(7, date(2027, 3, 13));
    assert!(w.run(date(2027, 3, 14)).await.opened.is_empty());
    assert!(w.run(date(2027, 3, 25)).await.opened.len() == 1);
}

/// AC4: HR is reminded every day until two failed attempts on different days
/// (2027-03-15 by phone, 03-16 by SMS) and the report number "MOHRE-777" are
/// recorded; with one attempt, or no number, the reminders go on. Recording
/// the number needs `absence.manage`, a number and a date not in the future.
#[renox::test]
async fn hr_is_reminded_daily_until_two_attempts_and_the_report_number_are_recorded() {
    let w = world().await;
    w.t.history.attended(7, date(2027, 3, 6));
    w.run(date(2027, 3, 14)).await;
    assert_eq!(w.reminders().len(), 1, "on the day it opens");

    w.t.staff.failed_to_reach(7, date(2027, 3, 15));
    w.run(date(2027, 3, 15)).await;
    assert_eq!(w.reminders().len(), 2);
    assert!(
        w.reminders()[1].data.to_string().contains("1 of 2"),
        "{}",
        w.reminders()[1].data
    );
    // The same day again: once a day.
    w.run(date(2027, 3, 15)).await;
    assert_eq!(w.reminders().len(), 2);

    w.t.staff.failed_to_reach(7, date(2027, 3, 16));
    w.run(date(2027, 3, 16)).await;
    assert_eq!(w.reminders().len(), 3);
    let last = w.reminders()[2].data.to_string();
    assert!(
        last.contains("2 of 2") && last.contains("not recorded"),
        "{last}"
    );

    // The number: the account manager may not, HR needs a number and a past day.
    let path = format!("/manpower/absence/{}/report", w.case().await.id);
    let amal = w.t.user("Nobody").await;
    w.t.post(
        &amal,
        &path,
        &[("number", "MOHRE-777"), ("date", "2027-03-16")],
    )
    .await
    .assert_status(403);
    w.t.on(date(2027, 3, 16), async {
        w.t.post(&w.hana, &path, &[("number", " "), ("date", "2027-03-16")])
            .await
            .assert_status(422);
        w.t.post(
            &w.hana,
            &path,
            &[("number", "MOHRE-777"), ("date", "2027-03-30")],
        )
        .await
        .assert_status(422);
        w.t.post(
            &w.hana,
            &path,
            &[("number", "MOHRE-777"), ("date", "2027-03-16")],
        )
        .await
        .assert_status(303);
    })
    .await;
    let case = w.case().await;
    assert_eq!(case.report_number.as_deref(), Some("MOHRE-777"));
    assert_eq!(case.report_date, Some(date(2027, 3, 16)));
    assert!(w.t.tickets.all()[0].comments[0].contains("MOHRE-777"));

    // Both recorded: no more reminders.
    w.run(date(2027, 3, 17)).await;
    w.run(date(2027, 3, 18)).await;
    assert_eq!(w.reminders().len(), 3);
}

/// AC5 (the system side): W returns on 2027-03-19 and HR closes the case with
/// the reason "hospitalised": the ticket is completed with it, wl-hr is told
/// (`AbsenceCaseClosed`, which makes it release the held pay), the open cover
/// request is cancelled with *worker returned*, the deployment is untouched.
/// Only HR may close, a reason is needed, and a closed case is not closed
/// twice.
#[renox::test]
async fn hr_closes_the_case_when_the_worker_returns() {
    let w = world().await;
    w.t.history.attended(7, date(2027, 3, 6));
    w.run(date(2027, 3, 14)).await;
    let case = w.case().await;
    let path = format!("/manpower/absence/{}/close", case.id);
    let other = w.t.user("Nobody").await;
    w.t.on(date(2027, 3, 19), async {
        w.t.post(&other, &path, &[("reason", "hospitalised")])
            .await
            .assert_status(403);
        w.t.post(&w.hana, &path, &[("reason", " ")])
            .await
            .assert_status(422);
        w.t.post(&w.hana, &path, &[("reason", "hospitalised")])
            .await
            .assert_status(303);
        w.t.post(&w.hana, &path, &[("reason", "again")])
            .await
            .assert_status(422);
    })
    .await;

    assert!(AbsenceCase::open_of(w.t.db(), 7).await.unwrap().is_none());
    let tickets = w.t.tickets.all();
    assert_eq!(tickets[0].completed.as_deref(), Some("hospitalised"));
    let closed = w.t.case_closed.all();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].employee, w.w);
    assert_eq!(closed[0].reason, "hospitalised");
    assert_eq!(closed[0].closed_on, date(2027, 3, 19));
    assert_eq!(
        closed[0].case,
        Subject::new("ticketing.ticket", tickets[0].id)
    );
    let covers = w.covers().await;
    assert_eq!(covers[0].status, RequestStatus::Cancelled);
    assert_eq!(covers[0].cancel_reason.as_deref(), Some("worker returned"));
    assert_eq!(
        w.t.reload_deployment(w.d.id).await.status,
        DeploymentStatus::Active
    );
    // No reminders after the close.
    w.run(date(2027, 3, 20)).await;
    assert_eq!(w.reminders().len(), 1);
}

/// A worker reinstated after the absence report closes the case the same way.
#[renox::test]
async fn a_reinstated_worker_closes_the_case() {
    let w = world().await;
    w.t.history.attended(7, date(2027, 3, 6));
    w.run(date(2027, 3, 14)).await;
    w.t.on(date(2027, 3, 20), async {
        w.t.state()
            .emit(EmployeeReinstated {
                employee: w.w,
                return_date: date(2027, 3, 20),
            })
            .await
            .unwrap();
    })
    .await;
    assert!(AbsenceCase::open_of(w.t.db(), 7).await.unwrap().is_none());
    assert_eq!(w.t.case_closed.all().len(), 1);
    assert_eq!(w.t.case_closed.all()[0].reason, "worker reinstated");
}

/// AC6: the notifier is down, so the follow-up fails: the case row exists, the
/// next run finishes the follow-up once; a run that keeps failing alerts the
/// MD (the owner of the rule) after the fifth attempt.
#[renox::test]
async fn a_failed_run_is_retried_and_then_alerts_the_md() {
    let w = world().await;
    w.t.history.attended(7, date(2027, 3, 6));
    w.t.notices.fail(true);
    let day = date(2027, 3, 14);
    let first =
        w.t.on(day, async { absence::run_on(w.t.state(), day).await })
            .await;
    assert!(first.is_err());
    assert!(w.case().await.announced_at.is_none(), "still to do");

    w.t.notices.fail(false);
    w.t.app.travel(Duration::from_secs(61));
    let next = date(2027, 3, 15);
    w.t.on(next, async { absence::run_on(w.t.state(), next).await })
        .await
        .unwrap();
    assert!(w.case().await.announced_at.is_some());
    assert_eq!(w.t.tickets.all().len(), 1, "one ticket");
    // The event is sent again by the retry (wl-hr keeps one case per ticket).
    let sent = w.t.case_opened.all();
    assert!(!sent.is_empty() && sent.iter().all(|e| e.case == sent[0].case));
    assert_eq!(w.covers().await.len(), 1);

    // Another worker's run keeps failing.
    let w2 = w.t.staff.employ(&w.t.parties, 8, "Omar Said", true);
    w.t.deployment(&w.contract, &w.site, w2, None, DeploymentStatus::Active)
        .await;
    w.t.history.attended(8, date(2027, 3, 6));
    w.t.notices.fail(true);
    for n in 0..5_u64 {
        let day = date(2027, 3, 20) + renox::chrono::Days::new(n);
        w.t.app.travel(Duration::from_secs(2 * 60 * 60));
        let _ =
            w.t.on(day, async { absence::run_on(w.t.state(), day).await })
                .await;
    }
    let alerts = ComplianceAlert::query()
        .where_(ComplianceAlert::RULE.eq("MPW.RUL.09"))
        .get(w.t.db())
        .await
        .unwrap();
    assert_eq!(alerts.len(), 1, "the owner is alerted once: {alerts:?}");
    assert_eq!(alerts[0].owner_role, "md");
}
