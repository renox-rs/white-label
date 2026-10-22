//! BL-MPW-06 (#420) and BL-MPW-11 (#425), `MPW.RUL.05`, `MPW.ACL.16`,
//! `FIN.USG.01` and `FIN.RAT.01`, with the real wl-attendance, wl-invoicing and
//! wl-hr in the host of a manpower agency: the hours of an approved timesheet
//! become usage records priced by the rate card version of each work date, the
//! billing run drafts the invoice (also for the last timesheet of a contract
//! that has ended), the days a deployment was suspended are not billed (a month
//! rate by the days not suspended, a day rate by attendance), nothing invoiced
//! is repriced by a later rate card version, and a retry bills nothing twice.

mod manpower_support;
mod support;

use host::roles::{ACCOUNTANT, ADMIN};
use manpower::{ClientContract, ContractStatus, Deployment, DeploymentStatus};
use manpower_support::{account, all_valid_except, boot, deployment, document, worker};
use renox::chrono::{Days, NaiveDate, NaiveDateTime};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{day, messages_to, person};
use wl_attendance::timesheet::{ApprovalBasis, ApproverKind};
use wl_attendance::{Timesheet, TimesheetEntry, TimesheetStatus};
use wl_invoicing::Invoice;
use wl_invoicing::rate_card::{RateCardVersion, Unit, VersionStatus};
use wl_invoicing::rate_cards::{self, LineInput};
use wl_invoicing::usage::{self, UsageRecord};
use wl_kernel::approvals;
use wl_kernel::documents::today;
use wl_kernel::workforce::EmployeeId;
use wl_kernel::{BusinessCalendar, PartyId};

fn date(text: &str) -> NaiveDate {
    text.parse().expect("a date")
}

/// Moves the clock to 08:00 of `day` (Asia/Dubai, ahead of the real clock).
fn travel_to(app: &TestApp, day: &str) {
    let local = NaiveDateTime::parse_from_str(&format!("{day} 08:00"), "%Y-%m-%d %H:%M").unwrap();
    let ahead = (BusinessCalendar::default().from_local(local) - renox::db::now()).num_seconds();
    assert!(ahead > 0, "{day} is ahead of the clock");
    app.travel_back();
    app.travel(std::time::Duration::from_secs(ahead.unsigned_abs()));
}

/// The card of the contract with one accepted version from `from` and the
/// lines `lines`, accepted by `by` (the client's signed acceptance).
async fn rate_card(
    app: &TestApp,
    contract: &ClientContract,
    from: NaiveDate,
    lines: Vec<LineInput>,
    by: &User,
) -> (wl_invoicing::rate_card::RateCard, RateCardVersion) {
    let card = rate_cards::create_card(
        app.db(),
        None,
        PartyId(contract.client_party_id),
        &contract.subject(),
    )
    .await
    .unwrap();
    let version = rate_cards::new_version(app.db(), None, card.id, from, false)
        .await
        .unwrap();
    for line in lines {
        rate_cards::add_line(app.db(), version.id, line)
            .await
            .unwrap();
    }
    accept(app, &version, by).await;
    (card, version)
}

async fn accept(app: &TestApp, version: &RateCardVersion, by: &User) {
    app.acting_as(by);
    app.htmx()
        .post(
            &format!("/rate-card-versions/{}/accept", version.id),
            &[("document", "signed-acceptance.pdf")],
        )
        .await
        .assert_status(303);
}

/// A submitted timesheet of `worker` in `deployment` for `from` to `to`,
/// approved by the client approver, with 8 billable hours on each of `days`.
async fn submitted(
    app: &TestApp,
    worker: EmployeeId,
    deployment: &Deployment,
    from: NaiveDate,
    to: NaiveDate,
    days: &[NaiveDate],
) -> Timesheet {
    let mut sheet = Timesheet::regular(worker, from, to).in_context(&deployment.subject());
    sheet.approver_kind = ApproverKind::External;
    sheet.approver_role = Some("client_approver".into());
    sheet.status = TimesheetStatus::Submitted;
    let mut tx = app.db().begin().await.unwrap();
    sheet
        .insert_in_state(&mut tx, TimesheetStatus::Submitted, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    for worked in days {
        TimesheetEntry::create(
            app.db(),
            TimesheetEntry {
                timesheet_id: sheet.id,
                employee_id: worker.0,
                date: *worked,
                normal_minutes: 480,
                billable: true,
                ..TimesheetEntry::default()
            },
        )
        .await
        .unwrap();
    }
    sheet
}

async fn approve(app: &TestApp, carl: &User, sheet: &Timesheet) {
    app.acting_as(carl);
    app.htmx()
        .post(&format!("/timesheets/{}/approve", sheet.id), &[])
        .await
        .assert_ok();
}

async fn usage_of(app: &TestApp, sheet: &Timesheet) -> Vec<UsageRecord> {
    UsageRecord::query()
        .where_(UsageRecord::SOURCE_ID.eq(sheet.id))
        .order_by(UsageRecord::ID)
        .get(app.db())
        .await
        .unwrap()
}

async fn client_invoices(app: &TestApp, client: PartyId) -> Vec<Invoice> {
    Invoice::query()
        .where_(Invoice::PARTY_ID.eq(client.0))
        .order_by(Invoice::ID)
        .get(app.db())
        .await
        .unwrap()
}

async fn client_approver(app: &TestApp) -> User {
    renox::auth::permissions::define_role(app.db(), "client_approver", &[])
        .await
        .unwrap();
    person(app, "Carl", "client_approver").await
}

/// AC2 (#420), (#421 `MPW.ACL.16`) and the retry: contract K ended yesterday;
/// D1's last timesheet is closed and submitted by the 00:05 run, the client
/// approves it after the contract is `ended`, the approval records its 16 hours
/// at AED 25, and the next `MPW.RUL.05` run drafts one invoice of AED 400
/// (the accountant is told) and locks the timesheet. A later past-dated rate
/// card version (AED 30) reprices nothing that is invoiced, the approval event
/// received again records nothing, and a second run drafts no second invoice.
#[renox::test]
async fn the_last_timesheet_of_an_ended_contract_is_billed_as_a_draft_after_its_approval() {
    let app = boot().await;
    let a = account(&app).await;
    let rana = person(&app, "Rana", ADMIN).await;
    let mona = person(&app, "Mona", ADMIN).await;
    let accountant = person(&app, "Aziz", ACCOUNTANT).await;
    let carl = client_approver(&app).await;
    let last_day = today() - Days::new(1);
    ClientContract::query()
        .where_(ClientContract::ID.eq(a.contract.id))
        .update(
            app.db(),
            &[("status", &"terminating"), ("end_date", &last_day)],
        )
        .await
        .unwrap();
    let w1 = worker(&app, "Wasim Khan").await;
    let d1 = deployment(
        &app,
        &a,
        &a.site,
        w1,
        "security_guard",
        DeploymentStatus::Active,
        day(-100),
        Some(last_day),
    )
    .await;
    let (card, _v1) = rate_card(
        &app,
        &a.contract,
        day(-150),
        vec![LineInput::new("security_guard", "day", Unit::Hour, 2_500)],
        &rana,
    )
    .await;
    let worked = [last_day - Days::new(1), last_day];
    let mut new = Timesheet::regular(w1, last_day - Days::new(6), today() + Days::new(2))
        .in_context(&d1.subject());
    new.approver_kind = ApproverKind::External;
    new.approver_role = Some("client_approver".into());
    let open = Timesheet::create(app.db(), new).await.unwrap();
    app.run_scheduled(manpower::CONTRACT_END_TASK)
        .await
        .unwrap();
    assert_eq!(
        ClientContract::find_or_404(app.db(), a.contract.id)
            .await
            .unwrap()
            .status,
        ContractStatus::Ended
    );
    let closed = Timesheet::find_or_404(app.db(), open.id).await.unwrap();
    assert_eq!(closed.status, TimesheetStatus::Submitted);
    // The hours allocated to the days, entered for the client to approve (the
    // submit check compares them with attendance, which this test does not
    // record).
    for worked in worked {
        TimesheetEntry::create(
            app.db(),
            TimesheetEntry {
                timesheet_id: closed.id,
                employee_id: w1.0,
                date: worked,
                normal_minutes: 480,
                billable: true,
                ..TimesheetEntry::default()
            },
        )
        .await
        .unwrap();
    }

    assert!(
        usage_of(&app, &closed).await.is_empty(),
        "nothing before the approval"
    );
    app.run_scheduled(manpower::BILLING_TASK).await.unwrap();
    assert!(
        client_invoices(&app, a.client).await.is_empty(),
        "an unapproved timesheet is not billed"
    );

    approve(&app, &carl, &closed).await;
    let approved = Timesheet::find_or_404(app.db(), open.id).await.unwrap();
    assert_eq!(approved.approval_basis, Some(ApprovalBasis::Approved));
    let records = usage_of(&app, &approved).await;
    let prices: Vec<_> = records
        .iter()
        .map(|r| (r.rate_date, r.draft_line().net()))
        .collect();
    assert_eq!(
        prices,
        [(Some(worked[0]), 20_000), (Some(worked[1]), 20_000)]
    );
    assert!(
        client_invoices(&app, a.client).await.is_empty(),
        "the invoice waits for the billing run"
    );

    app.run_scheduled(manpower::BILLING_TASK).await.unwrap();
    let invoices = client_invoices(&app, a.client).await;
    assert_eq!(invoices.len(), 1, "one draft invoice");
    assert_eq!(invoices[0].net_total, 40_000);
    assert_eq!(invoices[0].party_id, a.client.0);
    assert_eq!(
        usage::of_invoice(app.db(), invoices[0].id)
            .await
            .unwrap()
            .len(),
        2
    );
    let locked = Timesheet::find_or_404(app.db(), open.id).await.unwrap();
    assert_eq!(locked.status, TimesheetStatus::Locked, "billing used it");
    assert_eq!(
        locked.billed_by.as_deref(),
        Some(format!("invoicing.invoice:{}", invoices[0].id).as_str())
    );
    let told = messages_to(&app, "manpower.notice", &accountant).await;
    assert!(
        told.iter()
            .any(|m| m.data.0.to_string().contains("AED 400")),
        "the accountant is told of the draft"
    );

    // MPW.ACL.16: V2 (AED 30) dated in the past, accepted and approved, leaves
    // the invoiced hours at AED 25.
    let v2 = rate_cards::new_version(app.db(), None, card.id, last_day, false)
        .await
        .unwrap();
    rate_cards::add_line(
        app.db(),
        v2.id,
        LineInput::new("security_guard", "day", Unit::Hour, 3_000),
    )
    .await
    .unwrap();
    accept(&app, &v2, &rana).await;
    let waiting = RateCardVersion::find_or_404(app.db(), v2.id).await.unwrap();
    assert_eq!(waiting.status, VersionStatus::AwaitingApproval);
    let request = approvals::pending_for(app.db(), &waiting.subject())
        .await
        .unwrap()
        .remove(0);
    approvals::approve(app.state(), request.id, &mona, "")
        .await
        .unwrap();
    let after: Vec<_> = usage_of(&app, &approved)
        .await
        .iter()
        .map(|r| r.draft_line().net())
        .collect();
    assert_eq!(after, [20_000, 20_000], "invoiced hours keep their price");

    // The same approval received again, and a second run: nothing twice.
    let again = wl_attendance::approval::approved_event(app.state(), &locked)
        .await
        .unwrap();
    app.state().emit(again).await.unwrap();
    assert_eq!(usage_of(&app, &locked).await.len(), 2);
    app.run_scheduled(manpower::BILLING_TASK).await.unwrap();
    assert_eq!(client_invoices(&app, a.client).await.len(), 1);
}

/// AC2 (#425): W1 (a monthly rate of AED 3,000) and W2 (a day rate of AED 120)
/// have the work permit expire on 20 November; the nightly run of 21 November
/// suspends both. On 1 December the client approves November's timesheets and
/// the billing run drafts one invoice: W1 at 3,000 x 20/30 = AED 2,000, and W2
/// for the three days with attendance up to 20 November only (AED 360); the
/// hours recorded for 22 November are not billed.
#[renox::test]
async fn novembers_invoice_bills_the_suspended_deployments_only_to_the_twentieth() {
    let app = boot().await;
    let a = account(&app).await;
    let rana = person(&app, "Rana", ADMIN).await;
    let accountant = person(&app, "Aziz", ACCOUNTANT).await;
    let carl = client_approver(&app).await;
    let w1 = worker(&app, "Wasim Khan").await;
    let w2 = worker(&app, "Wajid Ali").await;
    for w in [w1, w2] {
        all_valid_except(&app, w, &["work_permit"]).await;
        document(&app, w, "work_permit", date("2026-11-20")).await;
    }
    let d1 = deployment(
        &app,
        &a,
        &a.site,
        w1,
        "security_guard",
        DeploymentStatus::Active,
        date("2026-10-01"),
        Some(date("2027-06-30")),
    )
    .await;
    let d2 = deployment(
        &app,
        &a,
        &a.site,
        w2,
        "cleaner",
        DeploymentStatus::Active,
        date("2026-10-01"),
        Some(date("2027-06-30")),
    )
    .await;
    // The contract and the deployments are fixtures dated before the clock.
    ClientContract::query()
        .where_(ClientContract::ID.eq(a.contract.id))
        .update(
            app.db(),
            &[
                ("start_date", &date("2026-10-01")),
                ("end_date", &date("2027-06-30")),
            ],
        )
        .await
        .unwrap();
    rate_card(
        &app,
        &a.contract,
        date("2026-10-01"),
        vec![
            LineInput::new("security_guard", "day", Unit::Month, 300_000),
            LineInput::new("cleaner", "day", Unit::Day, 12_000),
        ],
        &rana,
    )
    .await;

    travel_to(&app, "2026-11-21");
    app.at_travelled_time(async {
        wl_documents::expiry::run(app.state(), date("2026-11-21"))
            .await
            .unwrap();
    })
    .await;
    for id in [d1.id, d2.id] {
        let suspended = Deployment::find_or_404(app.db(), id).await.unwrap();
        assert_eq!(suspended.status, DeploymentStatus::Suspended);
        assert_eq!(suspended.suspended_on, Some(date("2026-11-21")));
    }

    travel_to(&app, "2026-12-01");
    app.at_travelled_time(async {
        let t1 = submitted(&app, w1, &d1, date("2026-11-01"), date("2026-11-30"), &[]).await;
        let attendance = [
            date("2026-11-18"),
            date("2026-11-19"),
            date("2026-11-20"),
            date("2026-11-22"),
        ];
        let t2 = submitted(
            &app,
            w2,
            &d2,
            date("2026-11-01"),
            date("2026-11-30"),
            &attendance,
        )
        .await;
        approve(&app, &carl, &t1).await;
        approve(&app, &carl, &t2).await;
        let monthly = usage_of(&app, &t1).await;
        assert_eq!(monthly.len(), 1);
        assert_eq!(monthly[0].draft_line().net(), 200_000, "3,000 x 20/30");
        let daily = usage_of(&app, &t2).await;
        let days: Vec<_> = daily.iter().map(|r| r.rate_date.unwrap()).collect();
        assert_eq!(
            days,
            [date("2026-11-18"), date("2026-11-19"), date("2026-11-20")]
        );

        app.run_scheduled(manpower::BILLING_TASK).await.unwrap();
        let invoices = client_invoices(&app, a.client).await;
        let seen: Vec<_> = invoices
            .iter()
            .map(|i| (i.id, i.party_id, i.net_total))
            .collect();
        assert_eq!(invoices.len(), 1, "{seen:?}");
        assert_eq!(invoices[0].net_total, 200_000 + 36_000);
        assert_eq!(
            usage::of_invoice(app.db(), invoices[0].id)
                .await
                .unwrap()
                .len(),
            4
        );
        let told = messages_to(&app, "manpower.notice", &accountant).await;
        assert!(
            told.iter()
                .any(|m| m.data.0.to_string().contains("AED 2,360"))
        );
    })
    .await;
    app.travel_back();
}

/// A worker who was suspended and resumed: the span of the suspension is
/// closed by the `resume`, so a month rate leaves out exactly those days. D
/// ran from 1 November, was suspended on 11 and back on 16 (days 11 to 15 are
/// not billed): 3,000 x 25/30 = AED 2,500.
#[renox::test]
async fn a_resumed_deployment_is_billed_except_for_the_days_it_was_suspended() {
    let app = boot().await;
    let a = account(&app).await;
    let rana = person(&app, "Rana", ADMIN).await;
    let carl = client_approver(&app).await;
    let w = worker(&app, "Wasim Khan").await;
    let d = deployment(
        &app,
        &a,
        &a.site,
        w,
        "security_guard",
        DeploymentStatus::Active,
        date("2026-10-01"),
        Some(date("2027-06-30")),
    )
    .await;
    rate_card(
        &app,
        &a.contract,
        date("2026-10-01"),
        vec![LineInput::new(
            "security_guard",
            "day",
            Unit::Month,
            300_000,
        )],
        &rana,
    )
    .await;
    // `suspend` and `resume` as the system runs them, on the given days.
    for (name, on) in [
        ("auto_suspend", "2026-11-11"),
        ("auto_resume", "2026-11-16"),
    ] {
        let fresh = Deployment::find_or_404(app.db(), d.id).await.unwrap();
        let input = manpower::deployment::DeploymentInput {
            on: Some(date(on)),
            ..Default::default()
        };
        let outcome = fresh
            .transition(name)
            .by_system()
            .reason("work permit")
            .input(&input)
            .run(app.state())
            .await
            .map_err(|e| e.into_error())
            .unwrap();
        outcome.emit(app.state()).await.unwrap();
    }
    travel_to(&app, "2026-12-01");
    app.at_travelled_time(async {
        let sheet = submitted(&app, w, &d, date("2026-11-01"), date("2026-11-30"), &[]).await;
        approve(&app, &carl, &sheet).await;
        let records = usage_of(&app, &sheet).await;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].draft_line().net(), 250_000);
    })
    .await;
    app.travel_back();
}
