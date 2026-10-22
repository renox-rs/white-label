//! BL-MPW-30 (#510), `HRM.ACL.04`, `HRM.WPS.01`: the payroll run and
//! settlement lists in the demo agency: the list of runs with the start of a
//! run, the list of final settlements, and who may open them.

mod manpower_screens_support;
mod support;

use manpower_screens_support::{
    ADMIN, HR, MD, SUPERVISOR, browse, day, demo, fits_a_phone, landed, no_raw_words, page, post,
    says, scalar, words,
};

/// AC1: HR opens `/hr/payroll` and sees the runs with period, status in words
/// and the total in AED; a viewer of the list without `payroll.prepare` has no
/// *New run*; the page fits a phone.
#[renox::test]
async fn hr_sees_the_payroll_runs_with_period_status_and_total_in_aed() {
    let app = demo().await;
    let html = page(&app, HR, "/hr/payroll").await;
    let month = day(0).format("%Y-%m").to_string();
    says(&html, &["Payroll runs", &month, "Draft", "AED ", "New run"]);
    no_raw_words(&html);
    fits_a_phone(&html);
    assert!(
        html.contains("rx-table-wrap"),
        "the table scrolls in its box"
    );
    let md = page(&app, MD, "/hr/payroll").await;
    assert!(!md.contains("New run"), "the approver does not start a run");
}

/// AC2: *New run* for another month makes a run and calculates it; a second
/// run for the same month is refused in words.
#[renox::test]
async fn new_run_makes_and_calculates_the_run_and_a_second_for_the_month_is_refused() {
    let app = demo().await;
    let next = day(40).format("%Y-%m").to_string();
    let made = post(&app, HR, "/hr/payroll", &[("period", &next)]).await;
    made.assert_status(303);
    let id = landed(&made);
    let html = page(&app, HR, &format!("/hr/payroll/{id}")).await;
    says(&html, &[&next, "Calculated"]);
    let again = post(&app, HR, "/hr/payroll", &[("period", &next)]).await;
    again.assert_status(422);
    assert!(
        again
            .text()
            .contains(&format!("A regular run for {next} already exists")),
        "{}",
        again.text()
    );
    let bad = post(&app, HR, "/hr/payroll", &[("period", "October")]).await;
    bad.assert_status(422);
    assert!(bad.text().contains("Give the month as year and month"));
    let md = post(&app, MD, "/hr/payroll", &[("period", "2031-01")]).await;
    md.assert_status(403);
    assert_eq!(
        scalar(
            &app,
            "select count(*) from wl_hr_payroll_runs where period = '2031-01'"
        )
        .await,
        0
    );
}

/// AC7 and AC8: `/hr/settlements` lists the settlements with employee, status
/// in words, amounts in AED and the 14-day deadline; the supervisor gets 403
/// on both lists.
#[renox::test]
async fn the_settlement_list_shows_status_amounts_and_deadline_and_the_supervisor_is_refused() {
    let app = demo().await;
    let html = page(&app, HR, "/hr/settlements").await;
    says(
        &html,
        &[
            "Final settlements",
            "Dawit",
            "Resignation",
            "Draft",
            "AED 3,850.00",
            &words(day(24)),
        ],
    );
    no_raw_words(&html);
    fits_a_phone(&html);
    page(&app, ADMIN, "/hr/settlements").await;
    for path in ["/hr/settlements", "/hr/payroll"] {
        browse(&app, SUPERVISOR, path).await.assert_status(403);
    }
}
