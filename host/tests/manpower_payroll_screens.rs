//! BL-MPW-30 (#510), `HRM.ACL.04`, `HRM.WPS.01`: the payroll run and
//! settlement lists in the demo agency: the list of runs with the start of a
//! run, the list of final settlements, and who may open them.

mod manpower_screens_support;
mod support;

use manpower_screens_support::{
    ADMIN, HR, MD, SUPERVISOR, browse, cells_of_row, day, demo, fits_a_phone, landed, no_raw_words,
    page, post, says, scalar, words,
};

/// AC1: HR opens `/hr/payroll` and sees the runs with period, status in words
/// and the total in AED; a viewer of the list without `payroll.prepare` has no
/// *New run*; the page fits a phone.
#[renox::test]
async fn hr_sees_the_payroll_runs_with_period_status_and_total_in_aed() {
    let app = demo().await;
    let html = page(&app, HR, "/hr/payroll").await;
    let month = day(0).format("%Y-%m").to_string();
    // The grid shows bare amounts under a column label that ends with (AED).
    says(
        &html,
        &["Payroll runs", &month, "Draft", "Total net (AED)", "New run"],
    );
    no_raw_words(&html);
    fits_a_phone(&html);
    assert!(
        html.contains("rx-grid__scroll"),
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
            "Dues (AED)",
            "3,850.00",
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

/// Starts and calculates the regular run of the month `n` days away as `email`;
/// the id of the run.
/// Every cell of every table of `html` that is a bare amount such as `2550.00`.
fn bare_amounts(html: &str) -> Vec<String> {
    html.split("<td")
        .skip(1)
        .filter_map(|cell| {
            let cell = cell.split_once('>')?.1.split("</td>").next()?;
            let cell = cell.trim();
            let digits = cell.trim_start_matches('-');
            let (whole, fraction) = digits.split_once('.')?;
            (!whole.is_empty()
                && whole.chars().all(|c| c.is_ascii_digit())
                && fraction.len() == 2
                && fraction.chars().all(|c| c.is_ascii_digit()))
            .then(|| cell.to_owned())
        })
        .collect()
}

/// The run page shows its amounts as `AED 2,550.00`, not as bare `2550.00`.
#[renox::test]
async fn the_run_page_shows_amounts_in_aed_with_a_thousands_comma() {
    let app = demo().await;
    let id = started(&app, HR, 40).await;
    let html = page(&app, HR, &format!("/hr/payroll/{id}")).await;
    assert_eq!(bare_amounts(&html), Vec::<String>::new(), "bare amounts");
    let ravi = cells_of_row(&html, "Ravi Kumar");
    assert!(
        ravi.iter().any(|cell| cell.starts_with("AED ")),
        "an amount in AED: {ravi:?}"
    );
    assert!(
        ravi.iter().any(|cell| cell.contains(',')),
        "a thousands comma: {ravi:?}"
    );
}

/// The documents page of a run goes back to the run, and the one of a worker back
/// to the worker.
#[renox::test]
async fn the_documents_page_of_a_run_has_a_back_link_to_the_run() {
    let app = demo().await;
    let id = started(&app, HR, 40).await;
    let docs = page(&app, HR, &format!("/documents/of/hr.payroll_run/{id}")).await;
    says(&docs, &[&format!("href=\"/hr/payroll/{id}\"")]);
    let worker = manpower_screens_support::employee(&app, "Wasim Khan").await;
    let docs = page(&app, HR, &format!("/documents/of/hr.employee/{worker}")).await;
    says(&docs, &[&format!("href=\"/hr/employees/{worker}\"")]);
}

async fn started(app: &renox::testing::TestApp, email: &str, n: i64) -> i64 {
    let month = day(n).format("%Y-%m").to_string();
    let made = post(app, email, "/hr/payroll", &[("period", &month)]).await;
    made.assert_status(303);
    landed(&made)
}

/// AC3, AC4, AC6: the preparer who opens the run as approver sees *Approve*
/// disabled with the reason and is refused when posting it; the MD approves;
/// then HR sees *Submit* and submits, and a calculated run offers none; the SIF
/// of a run never submitted is refused, the one of a submitted run downloads
/// and the download is audited.
#[renox::test]
async fn the_preparer_cannot_approve_and_an_approved_run_is_submitted_and_its_sif_downloads() {
    let app = demo().await;
    let id = started(&app, ADMIN, 40).await;
    let path = format!("/hr/payroll/{id}");
    let own = page(&app, ADMIN, &path).await;
    says(
        &own,
        &[
            "Calculated",
            "Approve",
            "prepared by one user and approved by another",
        ],
    );
    let refused = post(
        &app,
        ADMIN,
        &format!("{path}/transitions/approve"),
        &[("reason", "")],
    )
    .await;
    // The issue says 403; the guard of the transition answers 422 with its
    // text and the tests of BL-HRM-11 pin that, so the service is unchanged.
    refused.assert_status(422);
    assert!(
        refused
            .text()
            .contains("prepared by one user and approved by another"),
        "{}",
        refused.text()
    );
    let calculated = page(&app, HR, &path).await;
    assert!(
        !calculated.contains("transitions/submit"),
        "a calculated run offers no submit"
    );
    let early = browse(&app, HR, &format!("{path}/sif/1")).await;
    early.assert_status(422);
    assert!(early.text().contains("no salary file yet"));

    post(
        &app,
        MD,
        &format!("{path}/transitions/approve"),
        &[("reason", "")],
    )
    .await
    .assert_status(303);
    // The demo has a worker whose pay account is cooling, one waiting for a
    // second user and one without: the button is disabled with their numbers.
    let blocked = page(&app, HR, &path).await;
    says(
        &blocked,
        &["Approved", "Submit", "No active WPS pay account for: DSM-"],
    );
    no_raw_words(&blocked);
    // Their accounts are made active (the precondition, not the test).
    renox::db::sql("update wl_hr_pay_accounts set status = 'active'".to_owned())
        .execute(app.db())
        .await
        .unwrap();
    let approved = page(&app, HR, &path).await;
    assert!(
        !approved.contains("No active WPS pay account"),
        "{}",
        approved
            .split("No active WPS pay account")
            .nth(1)
            .unwrap_or_default()
            .chars()
            .take(80)
            .collect::<String>()
    );
    let sent = post(
        &app,
        HR,
        &format!("{path}/transitions/submit"),
        &[("reason", "")],
    )
    .await;
    sent.assert_status(303);
    let html = page(&app, HR, &path).await;
    says(
        &html,
        &[
            "Submitted",
            "Download SIF",
            &format!("{path}/sif/1"),
            "Upload bank result",
        ],
    );
    fits_a_phone(&html);
    let file = browse(&app, HR, &format!("{path}/sif/1")).await;
    file.assert_status(200);
    assert!(file.text().starts_with("SCR,"), "{}", file.text());
    assert!(
        file.header("content-disposition")
            .unwrap_or_default()
            .contains(".sif")
    );
    assert_eq!(
        scalar(
            &app,
            "select count(*) from audit_logs where action = 'hr.payroll.sif_downloaded'"
        )
        .await,
        1
    );
}

/// A run of the month `n` days away, calculated by HR, approved by the MD and
/// submitted by HR (the pay accounts made active first); its id.
async fn submitted(app: &renox::testing::TestApp, n: i64) -> i64 {
    let id = started(app, HR, n).await;
    let path = format!("/hr/payroll/{id}");
    post(
        app,
        MD,
        &format!("{path}/transitions/approve"),
        &[("reason", "")],
    )
    .await
    .assert_status(303);
    renox::db::sql("update wl_hr_pay_accounts set status = 'active'".to_owned())
        .execute(app.db())
        .await
        .unwrap();
    post(
        app,
        HR,
        &format!("{path}/transitions/submit"),
        &[("reason", "")],
    )
    .await
    .assert_status(303);
    id
}

/// HR uploads the bank's result file on the run's documents page; the number
/// of the document.
async fn uploaded(app: &renox::testing::TestApp, id: i64, number: &str) {
    let hr = manpower_screens_support::person(app, HR).await;
    app.acting_as(&hr);
    let response = app
        .htmx()
        .post_multipart(
            &format!("/documents/of/hr.payroll_run/{id}"),
            &[("kind", "bank_result"), ("number", number)],
            &[("files", "result.csv", b"line,status\n1,ok\n")],
        )
        .await;
    assert!(
        response.status.is_success() || response.status.is_redirection(),
        "{}",
        response.text()
    );
}

/// AC5: the bank's result file is uploaded on the run's documents page (the
/// page names the run, not `hr.payroll_run`); recording it with one rejected
/// line makes the run partly rejected, the page lists the rejected line with
/// the reason and the paid ones, and offers *Regenerate* (refused while the
/// failed line is left).
#[renox::test]
async fn a_bank_result_with_one_rejected_line_is_listed_with_its_reason_and_regenerate() {
    let app = demo().await;
    let id = submitted(&app, 40).await;
    let path = format!("/hr/payroll/{id}");
    uploaded(&app, id, "BANK-RES-1").await;
    let documents = page(&app, HR, &format!("/documents/of/hr.payroll_run/{id}")).await;
    says(&documents, &["Payroll ", "Bank result file", "BANK-RES-1"]);
    assert!(
        !documents.contains("hr.payroll_run #"),
        "the run has a name"
    );

    let failing = scalar(
        &app,
        &format!("select min(id) from wl_hr_payroll_lines where run_id = {id}"),
    )
    .await;
    let none = post(
        &app,
        HR,
        &format!("{path}/partial"),
        &[
            ("result_file", ""),
            ("failed", &format!("{failing}=Invalid IBAN")),
        ],
    )
    .await;
    none.assert_status(422);
    post(
        &app,
        HR,
        &format!("{path}/partial"),
        &[
            ("result_file", "BANK-RES-1"),
            ("failed", &format!("{failing}=Invalid IBAN")),
        ],
    )
    .await
    .assert_status(303);
    let html = page(&app, HR, &path).await;
    says(
        &html,
        &[
            "Partly rejected",
            "Failed",
            "Invalid IBAN",
            "rejected 1 line(s)",
            "Regenerate",
            "Fix each rejected line or move it out of the run",
            "transitions/regenerate",
            "BANK-RES-1",
        ],
    );
    no_raw_words(&html);
    fits_a_phone(&html);
    // Regenerate is refused while the failed line is left.
    let early = post(
        &app,
        HR,
        &format!("{path}/transitions/regenerate"),
        &[("reason", "")],
    )
    .await;
    early.assert_status(422);
    assert!(
        early
            .text()
            .contains("Fix each failed line or move it out of the run")
    );
}

/// AC5 (whole file): a bank that rejects the whole file is recorded with its
/// reason in words, and the page offers *Resubmit*.
#[renox::test]
async fn a_whole_file_rejection_is_recorded_and_resubmit_is_offered() {
    let app = demo().await;
    let id = submitted(&app, 40).await;
    let path = format!("/hr/payroll/{id}");
    post(
        &app,
        HR,
        &format!("{path}/rejection"),
        &[("reason", "establishment_id"), ("notice", "BANK-NOTICE-7")],
    )
    .await
    .assert_status(303);
    let html = page(&app, HR, &path).await;
    says(&html, &["Rejected", "Resubmit"]);
    no_raw_words(&html);
}

/// The sidebar: HR and the MD reach the payroll runs from it, the accountant
/// reaches the settlements; the supervisor sees neither link.
#[renox::test]
async fn the_sidebar_links_the_lists_to_the_roles_that_may_open_them() {
    let app = demo().await;
    let hr = page(&app, HR, "/dashboard").await;
    says(&hr, &["href=\"/hr/payroll\"", "href=\"/hr/settlements\""]);
    // The links read in words, not as their text keys.
    says(&hr, &["Payroll runs", "Final settlements"]);
    assert!(!hr.contains("nav.payroll") && !hr.contains("nav.settlements"));
    let accountant = page(&app, manpower_screens_support::ACCOUNTANT, "/dashboard").await;
    says(
        &accountant,
        &["href=\"/hr/payroll\"", "href=\"/hr/settlements\""],
    );
    let supervisor = page(&app, SUPERVISOR, "/dashboard").await;
    assert!(!supervisor.contains("href=\"/hr/payroll\""));
    assert!(!supervisor.contains("href=\"/hr/settlements\""));
    page(
        &app,
        manpower_screens_support::ACCOUNTANT,
        "/hr/settlements",
    )
    .await;
    page(&app, manpower_screens_support::ACCOUNTANT, "/hr/payroll").await;
}

/// The bank's result form: a row that cannot be read is refused in words with
/// the row, not with a bare key; the row is read by the employee number the
/// page shows (no line id is needed).
#[renox::test]
async fn a_failed_row_is_read_by_employee_number_and_a_bad_row_is_refused_in_words() {
    let app = demo().await;
    let id = submitted(&app, 41).await;
    let path = format!("/hr/payroll/{id}");
    uploaded(&app, id, "BANK-RES-2").await;
    let bad = post(
        &app,
        HR,
        &format!("{path}/partial"),
        &[
            ("result_file", "BANK-RES-2"),
            ("failed", "DSM-9999=wrong IBAN"),
        ],
    )
    .await;
    bad.assert_status(422);
    let text = bad.text();
    assert!(text.contains("Cannot read the row"), "{text}");
    assert!(text.contains("DSM-9999=wrong IBAN"), "{text}");
    post(
        &app,
        HR,
        &format!("{path}/partial"),
        &[
            ("result_file", "BANK-RES-2"),
            ("failed", "DSM-0001=wrong IBAN"),
        ],
    )
    .await
    .assert_status(303);
    let html = page(&app, HR, &path).await;
    says(&html, &["Partly rejected", "wrong IBAN"]);
}
