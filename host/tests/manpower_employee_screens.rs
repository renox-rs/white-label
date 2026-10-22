//! BL-MPW-31 (#511), `HRM.ACL.03`, `MPW.ACL.03`: the employee page of the demo
//! agency with its tabs: the documents with their validity for the PRO, the
//! deployments (and the bench), the timesheets, the salary behind *Reveal* with
//! its audit entry, and the pay accounts; what the account manager does not see.

mod manpower_screens_support;
mod support;

use manpower_screens_support::{
    AMAL, HR, MD, PRO, SUPERVISOR, browse, cells_of_row, day, demo, employee, fits_a_phone,
    no_raw_words, page, says, scalar, words,
};
use renox::testing::TestApp;

/// The path of the page of the worker called `name`.
async fn path(app: &TestApp, name: &str) -> String {
    format!("/hr/employees/{}", employee(app, name).await)
}

/// The id of an active worker with no deployment.
async fn bench_worker(app: &TestApp) -> i64 {
    scalar(
        app,
        "select min(e.id) from wl_hr_employees e where e.status = 'active' and not exists \
         (select 1 from manpower_deployments d where d.employee_id = e.id)",
    )
    .await
}

/// AC1 and AC2: the PRO opens a worker and *Documents* lists the kinds with
/// *Valid to* dates and a badge, an expired visa says *Expired*; the documents
/// page of the worker names the worker, not `hr.employee #2`; the PRO sees no
/// salary and no pay accounts.
#[renox::test]
async fn the_pro_sees_the_documents_with_validity_and_the_documents_page_names_the_worker() {
    let app = demo().await;
    let id = employee(&app, "Wasim Khan").await;
    let ended = day(-3).to_string();
    renox::db::sql(format!(
        "update wl_documents_documents set expiry_date = '{ended}' where subject_id = {id} \
         and subject_type = 'hr.employee' and kind_id = \
         (select id from wl_documents_kinds where key = 'residence_visa')"
    ))
    .execute(app.db())
    .await
    .unwrap();
    let html = page(&app, PRO, &format!("/hr/employees/{id}")).await;
    says(
        &html,
        &[
            "Documents",
            "Residence visa",
            "Valid to",
            &words(day(-3)),
            "Expired",
            "Valid",
            "PRO",
        ],
    );
    no_raw_words(&html);
    fits_a_phone(&html);
    assert!(!html.contains("Reveal"), "the PRO has no salary tab");
    assert!(!html.contains("employee-tabs-salary"));
    assert!(!html.contains("Pay accounts"));

    let docs = page(&app, PRO, &format!("/documents/of/hr.employee/{id}")).await;
    says(&docs, &["Wasim Khan"]);
    assert!(
        docs.contains("<p class=\"rx-subtitle\">Wasim Khan"),
        "the subtitle of the page holds the worker's name"
    );
    assert!(!docs.contains("hr.employee #"), "the page names the worker");
}

/// The hire date of the employee page and of the list reads `01 Nov 2026`.
#[renox::test]
async fn the_hire_date_reads_as_a_day_month_year_in_words() {
    let app = demo().await;
    let id = employee(&app, "Wasim Khan").await;
    let hired = manpower_screens_support::text(
        &app,
        &format!("select cast(hire_date as text) from wl_hr_employees where id = {id}"),
    )
    .await;
    let hired = renox::chrono::NaiveDate::parse_from_str(&hired[..10], "%Y-%m-%d").unwrap();
    let on_page = page(&app, HR, &format!("/hr/employees/{id}")).await;
    says(&on_page, &["Hire date", &words(hired)]);
    assert!(!on_page.contains(&hired.to_string()), "no ISO date");
    let list = page(&app, HR, "/hr/employees").await;
    says(&list, &[&words(hired)]);
    assert!(!list.contains(&format!(">{hired}<")), "no ISO date");
}

/// *Start an onboarding* is not the main button of an active worker's page.
#[renox::test]
async fn start_an_onboarding_is_not_the_primary_button_of_an_active_worker() {
    let app = demo().await;
    let html = page(&app, HR, &path(&app, "Wasim Khan").await).await;
    says(&html, &["Start an onboarding"]);
    assert!(
        html.contains("rx-button--secondary\" type=\"button\" data-rx-open=\"start-onboarding\""),
        "the trigger of the onboarding sheet is a secondary button"
    );
}

/// AC3: a deployed worker's *Deployments* lists the client, site and status in
/// words; a worker without a deployment says *On the bench*.
#[renox::test]
async fn deployments_list_the_client_and_status_and_a_bench_worker_says_so() {
    let app = demo().await;
    let html = page(&app, HR, &path(&app, "Wasim Khan").await).await;
    says(
        &html,
        &[
            "Deployments",
            "Gulf Tower Facilities LLC",
            "Active",
            "Security guard",
        ],
    );
    no_raw_words(&html);
    let bench = bench_worker(&app).await;
    let html = page(&app, HR, &format!("/hr/employees/{bench}")).await;
    says(&html, &["On the bench"]);
    no_raw_words(&html);
}

/// AC4: HR sees *Timesheets* with the month, the hours and the status in words.
#[renox::test]
async fn hr_sees_the_timesheets_with_hours_and_status() {
    let app = demo().await;
    let html = page(&app, HR, &path(&app, "Ravi Kumar").await).await;
    says(&html, &["Timesheets", "Month", "Hours"]);
    // Ravi's sheet: the week it covers, its hours and its status in words.
    let row = cells_of_row(&html, "Regular");
    assert_eq!(
        row,
        [
            format!("{} - {}", words(day(-6)), words(day(0))),
            "58 h".to_owned(),
            "Regular".to_owned(),
            "Submitted".to_owned(),
        ],
        "the timesheet row"
    );
    no_raw_words(&html);
}

/// AC5: without the permission the Salary tab is absent; with it the amounts
/// are hidden until *Reveal*, which shows them in AED and writes an audit entry.
#[renox::test]
async fn salary_is_hidden_until_reveal_and_each_reveal_is_audited() {
    let app = demo().await;
    let page_path = path(&app, "Wasim Khan").await;
    let before = page(&app, HR, &page_path).await;
    says(&before, &["Salary", "Reveal", "recorded in the audit log"]);
    assert!(
        !before.contains("Basic wage"),
        "nothing is shown before Reveal"
    );
    let audited = "select count(*) from audit_logs where action = 'hr.employee.salary_viewed'";
    assert_eq!(scalar(&app, audited).await, 0);

    let shown = page(&app, HR, &format!("{page_path}?reveal=salary")).await;
    says(
        &shown,
        &["Basic wage", "AED 1,800.00", "Fixed wage a month"],
    );
    no_raw_words(&shown);
    assert_eq!(scalar(&app, audited).await, 1);

    // The MD holds the permission too; the PRO and the account manager do not
    // get the amounts even by asking for them.
    page(&app, MD, &format!("{page_path}?reveal=salary")).await;
    assert_eq!(scalar(&app, audited).await, 2);
    for who in [PRO, AMAL] {
        let html = page(&app, who, &format!("{page_path}?reveal=salary")).await;
        assert!(!html.contains("Basic wage"), "{who} sees no amounts");
    }
    assert_eq!(scalar(&app, audited).await, 2);
}

/// AC6: *Pay accounts* shows the bank, the masked number, the status and the
/// day the cooling ends.
#[renox::test]
async fn pay_accounts_show_bank_masked_number_status_and_cooling_end() {
    let app = demo().await;
    let html = page(&app, HR, &path(&app, "Ravi Kumar").await).await;
    says(
        &html,
        &[
            "Pay accounts",
            "Bank account",
            "033100101",
            "…",
            "Cooling",
            &words(day(3)),
        ],
    );
    assert!(!html.contains("AE07033"), "the full number is never shown");
    no_raw_words(&html);
}

/// AC7: the account manager opens a worker of the client he looks after and
/// sees the deployments and timesheets, no salary and no pay accounts; a worker
/// without a deployment in his clients is refused; the supervisor is refused.
#[renox::test]
async fn the_account_manager_sees_no_salary_and_no_pay_accounts() {
    let app = demo().await;
    let html = page(&app, AMAL, &path(&app, "Wasim Khan").await).await;
    says(
        &html,
        &["Deployments", "Timesheets", "Gulf Tower Facilities LLC"],
    );
    for hidden in ["Reveal", "Pay accounts", "Salary", "Documents", "Dawit"] {
        assert!(
            !html.contains(hidden),
            "the account manager sees no `{hidden}`"
        );
    }
    no_raw_words(&html);
    let bench = bench_worker(&app).await;
    browse(&app, AMAL, &format!("/hr/employees/{bench}"))
        .await
        .assert_status(403);
    browse(&app, SUPERVISOR, &path(&app, "Wasim Khan").await)
        .await
        .assert_status(403);
}
