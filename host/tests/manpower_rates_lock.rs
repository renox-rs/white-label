//! BL-MPW-07 (#421), `MPW.ACL.16`, with the real wl-attendance in the host of
//! a manpower agency: a rate card version dated in the past, accepted and
//! approved by the MD, reprices the unlocked hours of its period and never the
//! hours of a timesheet that is `locked`. The lock is the real timesheet's
//! (wl-attendance's `Timesheets` provider answers `is_locked`).

mod support;

use host::roles::ADMIN;
use renox::chrono::{NaiveDate, NaiveDateTime};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{noon_clock, person, tenant};
use wl_attendance::timesheet::Consumer;
use wl_attendance::{Timesheet, TimesheetStatus};
use wl_hr::{EmployeeStatus, NewEmployee, employees};
use wl_invoicing::rate_card::{RateCardVersion, Unit, VersionStatus};
use wl_invoicing::rate_cards::{self, LineInput};
use wl_invoicing::rate_pricing::{self, Work};
use wl_invoicing::usage::UsageRecord;
use wl_kernel::approvals;
use wl_kernel::documents::today;
use wl_kernel::timesheets::timesheets;
use wl_kernel::{BusinessCalendar, ClientConfig, PartyId};

/// A manpower agency: the vertical on, with the shared modules it reaches.
const CLIENT: &str = r#"
[client]
name = "Gulf Manpower Services LLC"
app = "manpower"

[modules]
enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "payments", "hr", "attendance", "portal"]

[[legal_entities]]
code = "GMS"
name_en = "Gulf Manpower Services LLC"
default = true
"#;

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

/// The MD's staff (a second admin: whoever accepts cannot approve).
async fn admin(app: &TestApp, name: &str) -> User {
    person(app, name, ADMIN).await
}

/// 8 hours of guard on the day shift on `day`, priced by `contract`'s card.
fn shift(contract: &wl_kernel::Subject, day: &str) -> Work {
    Work::new(contract.clone(), "guard", "day", date(day), "normal", 8_000)
}

/// An `approved` timesheet of `owner` for `from` to `to`, then `locked` by
/// wl-attendance's `lock` (what billing or payroll does).
async fn timesheet(
    app: &TestApp,
    owner: wl_kernel::workforce::EmployeeId,
    from: &str,
    to: &str,
    locked: bool,
) -> Timesheet {
    let mut sheet = Timesheet::regular(owner, date(from), date(to));
    sheet.status = TimesheetStatus::Approved;
    let mut tx = app.db().begin().await.unwrap();
    sheet
        .insert_in_state(&mut tx, TimesheetStatus::Approved, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    if locked {
        wl_attendance::timesheet::lock(app.state(), sheet.id, Consumer::Billing, "usage-invoice:1")
            .await
            .unwrap();
    }
    Timesheet::find_or_404(app.db(), sheet.id).await.unwrap()
}

/// AC4: V1 (guard AED 25) is accepted; hours of 1 and 2 December on a
/// timesheet locked on 10 December and of 8 December on an open one are priced
/// at 25. V2 (AED 27, effective 2026-12-01, made on 10 December) is accepted
/// by staff and approved by the MD: the open hours are repriced at 27, the
/// hours of the real locked timesheet keep AED 25.
#[renox::test]
async fn a_locked_timesheet_keeps_its_price_when_the_md_approves_a_past_version() {
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(host::app_for(config).unwrap().provide(noon_clock())).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    let rana = admin(&app, "Rana").await;
    let mona = admin(&app, "Mona").await;
    let client = PartyId(3);
    let contract = manpower::ClientContract::create(
        app.db(),
        manpower::ClientContract {
            reference: "CC-0001".into(),
            client_party_id: client.0,
            start_date: date("2026-11-01"),
            end_date: date("2027-12-31"),
            payment_terms_days: 30,
            notice_period_days: 30,
            ..manpower::ClientContract::default()
        },
    )
    .await
    .unwrap();
    let source = contract.subject();
    let worker_party = tenant(&app, "Wasim Khan").await;
    let worker = employees::create(
        app.state(),
        NewEmployee::new(worker_party, "security_guard", today()).status(EmployeeStatus::Active),
    )
    .await
    .unwrap()
    .employee_id();

    travel_to(&app, "2026-12-10");
    app.at_travelled_time(async {
        let card = rate_cards::create_card(app.db(), None, client, &source)
            .await
            .unwrap();
        let guard = |rate: i64| LineInput::new("guard", "day", Unit::Hour, rate * 100);
        let v1 = rate_cards::new_version(app.db(), None, card.id, date("2026-11-01"), false)
            .await
            .unwrap();
        rate_cards::add_line(app.db(), v1.id, guard(25))
            .await
            .unwrap();
        let accept = |version: &RateCardVersion| {
            let path = format!("/rate-card-versions/{}/accept", version.id);
            let app = &app;
            let rana = &rana;
            async move {
                app.acting_as(rana);
                app.htmx()
                    .post(&path, &[("document", "signed-acceptance.pdf")])
                    .await
            }
        };
        accept(&v1).await.assert_status(303);

        // Hours of two timesheets are priced at V1; the first is locked.
        let locked = timesheet(&app, worker, "2026-12-01", "2026-12-07", true).await;
        let open = timesheet(&app, worker, "2026-12-08", "2026-12-14", false).await;
        assert_eq!(locked.status, TimesheetStatus::Locked);
        let provider = timesheets(app.state());
        assert!(
            provider
                .is_locked(app.state(), &locked.subject())
                .await
                .unwrap(),
            "the real provider says locked"
        );
        assert!(
            !provider
                .is_locked(app.state(), &open.subject())
                .await
                .unwrap()
        );
        let record = |sheet: &Timesheet, day: &'static str| {
            let (db, source) = (app.db().clone(), source.clone());
            let subject = sheet.subject();
            async move {
                rate_pricing::record_hours(&db, &subject, &shift(&source, day), None)
                    .await
                    .unwrap()
                    .unwrap()
            }
        };
        let locked_one = record(&locked, "2026-12-01").await;
        let locked_two = record(&locked, "2026-12-02").await;
        let unlocked = record(&open, "2026-12-08").await;

        // V2 is dated in the past: accepted, it waits for the MD.
        let v2 = rate_cards::new_version(app.db(), None, card.id, date("2026-12-01"), false)
            .await
            .unwrap();
        rate_cards::add_line(app.db(), v2.id, guard(27))
            .await
            .unwrap();
        accept(&v2).await.assert_status(303);
        let waiting = RateCardVersion::find_or_404(app.db(), v2.id).await.unwrap();
        assert_eq!(waiting.status, VersionStatus::AwaitingApproval);
        let request = approvals::pending_for(app.db(), &waiting.subject())
            .await
            .unwrap()
            .remove(0);
        approvals::approve(app.state(), request.id, &mona, "")
            .await
            .unwrap();
        let accepted = RateCardVersion::find_or_404(app.db(), v2.id).await.unwrap();
        assert_eq!(accepted.status, VersionStatus::Accepted);

        let price = |record: &UsageRecord| {
            let (db, id) = (app.db().clone(), record.id);
            async move { UsageRecord::find_or_404(&db, id).await.unwrap().unit_price }
        };
        assert_eq!(price(&unlocked).await, 2_700, "the open timesheet");
        assert_eq!(price(&locked_one).await, 2_500, "the locked timesheet");
        assert_eq!(price(&locked_two).await, 2_500, "the locked timesheet");
    })
    .await;
    app.travel_back();
}
