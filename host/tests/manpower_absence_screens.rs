//! BL-MPW-25 (#505), `MPW.RUL.09`, `HRM.RUL.12`: the screens of the absence
//! cases in the demo agency: the list with its table, the case page with the
//! failed contact days, the MoHRE report, the tickets of the case, *Record
//! report* and *Close case*, the link to *Report as absconded* that waits for
//! its guards, and the roles that may not see any of it. Each test opens the
//! page as a role of the demo agency and checks the words on it.

mod manpower_screens_support;
mod support;

use manpower_screens_support::{
    AMAL, HR, browse, cells_of_row, day, demo, employee, fits_a_phone, no_raw_words, page, person,
    post, says, scalar, words,
};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_hr::payroll::{LineStatus, PayrollLine, PayrollRun, PayrollRunStatus};

/// The approver of Gulf Tower, a client user of the portal.
const APPROVER: &str = "approver@gulftower.example";

/// The open absence case of the demo (Kamal Hossain, nine days without
/// attendance).
async fn the_case(app: &TestApp) -> i64 {
    scalar(app, "select min(id) from manpower_absence_cases").await
}

/// The page of case `id`.
fn path(id: i64) -> String {
    format!("/manpower/absence/{id}")
}

/// A failed attempt to reach Kamal on `on`, recorded by HR.
async fn failed_attempt(app: &TestApp, on: renox::chrono::NaiveDate) {
    let who = employee(app, "Kamal Hossain").await;
    let on = on.to_string();
    let response = post(
        app,
        HR,
        &format!("/hr/employees/{who}/contact-attempts"),
        &[
            ("attempted_on", &on),
            ("method", "phone"),
            ("outcome", "no_answer"),
        ],
    )
    .await;
    assert!(
        response.status.is_success() || response.status.is_redirection(),
        "{}",
        response.status
    );
}

/// AC1: HR opens the list and the demo case is a row of an HTML table with the
/// worker, the client, the days absent and *Open*; the JSON answer stays.
#[renox::test]
async fn the_list_shows_the_case_of_the_absent_worker_in_a_table() {
    let app = demo().await;
    let html = page(&app, HR, "/manpower/absence").await;
    says(
        &html,
        &[
            "Absence cases",
            "<table",
            "Kamal Hossain",
            "Al Quoz Logistics Park LLC",
            "Open",
            "0 of 2 failed contact days",
            "Not recorded",
        ],
    );
    assert_eq!(
        cells_of_row(&html, "Kamal Hossain")
            .get(1)
            .map(String::as_str),
        Some("9"),
        "nine days absent, in the cell of the days column"
    );
    no_raw_words(&html);
    // At 390 px the table scrolls inside its box.
    says(&html, &["rx-table-wrap"]);
    fits_a_phone(&html);

    let user = person(&app, HR).await;
    app.acting_as(&user);
    let json = app.request().json().get("/manpower/absence").await;
    json.assert_ok();
    assert!(json.text().starts_with('['), "{}", json.text());
}

/// AC2 and the tickets: with one failed contact day the page says *1 of 2
/// failed contact days*, the link to *Report as absconded* is disabled with its
/// reason in words, and the ticket of the category absence is listed with its
/// status in words.
#[renox::test]
async fn one_failed_contact_day_keeps_the_absconding_link_disabled() {
    let app = demo().await;
    let id = the_case(&app).await;
    failed_attempt(&app, day(-3)).await;
    let html = page(&app, HR, &path(id)).await;
    says(
        &html,
        &[
            "Kamal Hossain",
            "1 of 2 failed contact days",
            &words(day(-3)),
            "Report as absconded",
            "Two failed contact days and the MoHRE number are needed",
            "9 days",
            &words(day(-9)),
            "Tickets",
            "TKT-",
            "Absence: Kamal Hossain",
            "Not recorded",
        ],
    );
    // The ticket row: number, title, and the status word (not the key).
    let ticket = cells_of_row(&html, "TKT-");
    assert!(ticket[0].starts_with("TKT-"), "{ticket:?}");
    assert_eq!(
        ticket.last().map(String::as_str),
        Some("Assigned"),
        "{ticket:?}"
    );
    let employee = employee(&app, "Kamal Hossain").await;
    assert!(
        !html.contains(&format!("href=\"/hr/employees/{employee}\">Report")),
        "no link to report while a guard fails"
    );
    no_raw_words(&html);
    fits_a_phone(&html);
    says(&html, &["rx-sheet", "rx-table-wrap"]);
}

/// AC3: *Record report* with `MOHRE-777` shows the number and the date; an empty
/// number is refused with *Enter the MoHRE report number*, a date in the
/// future with its own words.
#[renox::test]
async fn recording_the_report_shows_the_number_and_refuses_an_empty_one() {
    let app = demo().await;
    let id = the_case(&app).await;
    let today = day(0).to_string();
    let empty = post(
        &app,
        HR,
        &format!("{}/report", path(id)),
        &[("number", ""), ("date", &today)],
    )
    .await;
    empty.assert_status(422);
    assert!(
        empty.text().contains("Enter the MoHRE report number"),
        "{}",
        empty.text()
    );
    let tomorrow = day(1).to_string();
    let future = post(
        &app,
        HR,
        &format!("{}/report", path(id)),
        &[("number", "MOHRE-777"), ("date", &tomorrow)],
    )
    .await;
    future.assert_status(422);
    assert!(
        future.text().contains("cannot be in the future"),
        "{}",
        future.text()
    );

    post(
        &app,
        HR,
        &format!("{}/report", path(id)),
        &[("number", "MOHRE-777"), ("date", &today)],
    )
    .await
    .assert_status(303);
    let html = page(&app, HR, &path(id)).await;
    says(&html, &["MOHRE-777", &words(day(0))]);
    let list = page(&app, HR, "/manpower/absence").await;
    says(&list, &["MOHRE-777"]);
}

/// AC4: with two failed contact days on different days and the report number
/// the link to *Report as absconded* is enabled and goes to the worker's page.
#[renox::test]
async fn two_failed_contact_days_and_the_number_enable_the_absconding_link() {
    let app = demo().await;
    let id = the_case(&app).await;
    failed_attempt(&app, day(-3)).await;
    failed_attempt(&app, day(-2)).await;
    let before = page(&app, HR, &path(id)).await;
    says(
        &before,
        &["2 of 2 failed contact days", "MoHRE number are needed"],
    );

    let today = day(0).to_string();
    post(
        &app,
        HR,
        &format!("{}/report", path(id)),
        &[("number", "MOHRE-777"), ("date", &today)],
    )
    .await
    .assert_status(303);
    let html = page(&app, HR, &path(id)).await;
    let employee = employee(&app, "Kamal Hossain").await;
    assert!(
        html.contains(&format!("href=\"/hr/employees/{employee}\"")),
        "the link is drawn"
    );
    says(
        &html,
        &["Report as absconded", "2 of 2 failed contact days"],
    );
    assert!(
        !html.contains("Two failed contact days and the MoHRE number are needed"),
        "no reason once both are recorded"
    );
    no_raw_words(&html);
}

/// AC5: HR closes the case with the reason *hospitalised*: the page shows
/// *Closed*, the reason, who closed it, and that the held pay is proposed in an
/// off-cycle run, linked; an empty reason is refused; a closed case is
/// read-only.
#[renox::test]
async fn closing_the_case_shows_the_reason_and_the_off_cycle_run() {
    let app = demo().await;
    let id = the_case(&app).await;
    let worker = employee(&app, "Kamal Hossain").await;
    // The worker's pay of September was held for this case and the run was paid.
    let mut run = PayrollRun {
        period: "2026-09".into(),
        run_type: "regular".into(),
        status: PayrollRunStatus::Paid,
        ..PayrollRun::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    run.insert_in_state(&mut tx, PayrollRunStatus::Paid, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    PayrollLine::create(
        app.db(),
        PayrollLine {
            run_id: run.id,
            employee_id: worker,
            fixed_wage: 200_000,
            net_amount: 200_000,
            status: LineStatus::Held,
            held_reason: Some(wl_hr::payroll::HELD_ABSENCE_CASE.to_owned()),
            ..PayrollLine::default()
        },
    )
    .await
    .unwrap();

    let empty = post(&app, HR, &format!("{}/close", path(id)), &[("reason", "")]).await;
    empty.assert_status(422);
    assert!(
        empty.text().contains("Give the reason the case is closed"),
        "{}",
        empty.text()
    );
    let open = page(&app, HR, &path(id)).await;
    says(&open, &["id=\"close-case\"", "id=\"record-report\""]);

    post(
        &app,
        HR,
        &format!("{}/close", path(id)),
        &[("reason", "hospitalised")],
    )
    .await
    .assert_status(303);
    let html = page(&app, HR, &path(id)).await;
    says(
        &html,
        &[
            "Closed",
            "hospitalised",
            "Mona Farouk",
            "proposed in an off-cycle run",
            "Open the payroll run",
            "/hr/payroll/",
        ],
    );
    assert!(
        !html.contains("id=\"close-case\""),
        "a closed case is read-only"
    );
    assert!(!html.contains("id=\"record-report\""));
    no_raw_words(&html);
    let list = page(&app, HR, "/manpower/absence").await;
    says(&list, &["Closed", "Kamal Hossain"]);
    let again = post(
        &app,
        HR,
        &format!("{}/close", path(id)),
        &[("reason", "again")],
    )
    .await;
    again.assert_status(422);
}

/// AC6: the account manager gets 403 with the refusal text on the list and on a
/// case; the client's approver has no link to the cases and no page.
#[renox::test]
async fn only_hr_sees_the_cases_and_a_client_never_does() {
    let app = demo().await;
    let id = the_case(&app).await;
    for path in ["/manpower/absence".to_owned(), path(id)] {
        let refused = browse(&app, AMAL, &path).await;
        refused.assert_status(403);
        assert!(
            refused
                .text()
                .contains("You do not have permission for this action"),
            "{}",
            refused.text()
        );
    }
    let direct = post(
        &app,
        AMAL,
        &format!("{}/close", path(id)),
        &[("reason", "x")],
    )
    .await;
    direct.assert_status(403);

    for own in [
        "/dashboard",
        "/manpower/requests",
        "/portal/manpower/requests",
        "/portal/manpower/deployments",
        "/portal/manpower/timesheets",
    ] {
        let portal = page(&app, APPROVER, own).await;
        assert!(
            !portal.contains("/manpower/absence"),
            "{own}: the client sees no link to the cases"
        );
    }
    browse(&app, APPROVER, "/manpower/absence")
        .await
        .assert_status(403);
}
