//! The HR lists on the standard grid (#527): the employees, the payroll runs,
//! the final settlements, the leave queue and my leave are `renox::grid`s that
//! are direct children of their pages; they search (also the employee's name,
//! kept by the parties directory), filter (role, year, status), sort and page
//! through the query string, and export only for the roles that may.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_hr::{EmployeeStatus, LeaveRequest, LeaveStatus, PayrollRunStatus, SettlementStatus};

use support::Booted;

/// The footer of the grid in `html`: first row, last row and total
/// (`1–10 of 18`); `(0, 0, 0)` when the grid shows no footer (no rows).
fn range(html: &str) -> (i64, i64, i64) {
    let marker = "rx-grid__range\">";
    let Some(at) = html.find(marker) else {
        return (0, 0, 0);
    };
    let rest = &html[at + marker.len()..];
    let text = &rest[..rest.find("</p>").unwrap()];
    let numbers: Vec<i64> = text
        .split(|c: char| !c.is_ascii_digit() && c != ',')
        .filter(|part| !part.replace(',', "").is_empty())
        .map(|part| part.replace(',', "").parse().unwrap())
        .collect();
    (numbers[0], numbers[1], numbers[2])
}

/// The page at `path` as `user`, as a browser asks for it.
async fn page(t: &Booted, user: &User, path: &str) -> String {
    t.app.acting_as(user);
    let response = t
        .app
        .request()
        .header("accept", "text/html,application/xhtml+xml,*/*;q=0.8")
        .get(path)
        .await;
    assert_eq!(response.status.as_u16(), 200, "{path}: {}", response.text());
    response.text()
}

/// How many rows `query` counts.
async fn count(t: &Booted, query: &str) -> i64 {
    renox::db::sql(query.to_owned())
        .scalar::<i64>(t.db())
        .await
        .unwrap()
}

/// The template `name` of the crate puts its `<rx-grid>` directly in
/// `<rx-page>`, after the header, with nothing but dialogs after it.
fn grid_is_a_child_of_the_page(name: &str) {
    let path = format!("{}/views/wl-hr/{name}", env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{path}"));
    let lines: Vec<&str> = source.lines().collect();
    let at = lines
        .iter()
        .position(|line| line.trim_start().starts_with("<rx-grid"))
        .unwrap_or_else(|| panic!("{name} has no grid"));
    assert!(
        lines[at].starts_with("  <rx-grid"),
        "{name}: the grid is a direct child of <rx-page>"
    );
    let child = |line: &&&str| line.starts_with("  <") && !line.starts_with("   ");
    for line in lines[..at].iter().filter(child) {
        assert!(
            ["  <rx-page-header", "  <rx-include", "  </rx-page-header"]
                .iter()
                .any(|ok| line.starts_with(ok)),
            "{name}: `{line}` wraps or comes before the grid"
        );
    }
    for line in lines[at + 1..].iter().filter(child) {
        assert!(
            [
                "  </rx-grid",
                "  <rx-action-sheet",
                "  </rx-action-sheet",
                "  <template",
                "  </template",
            ]
            .iter()
            .any(|ok| line.starts_with(ok)),
            "{name}: `{line}` is below the grid"
        );
    }
    assert!(
        !source.contains("<rx-table"),
        "{name}: no table beside the grid"
    );
}

/// Every list of the issue is a grid that is a direct child of its page.
#[test]
fn every_hr_list_is_a_grid_that_fills_the_page() {
    for name in [
        "employees.html",
        "payroll_runs.html",
        "settlements.html",
        "leave_queue.html",
        "my_leave.html",
    ] {
        grid_is_a_child_of_the_page(name);
    }
}

/// Three employees with names in the directory; Zainab is a security guard.
async fn three_employees(t: &Booted) -> Vec<wl_hr::Employee> {
    let mut employees = Vec::new();
    for (party, name) in [
        (61, "Zainab Rahman"),
        (62, "Omar Saleh"),
        (63, "Priya Nair"),
    ] {
        t.parties.name(party, name);
        employees.push(t.employee_in(party, EmployeeStatus::Active).await);
    }
    renox::db::sql(format!(
        "update wl_hr_employees set job_role = 'security_guard' where id = {}",
        employees[0].id
    ))
    .execute(t.db())
    .await
    .unwrap();
    employees
}

/// The employees: page size and pages, sort, the name found through the
/// directory, the role a filter (in words), and the export for HR only.
#[renox::test]
async fn the_employees_page_sort_filter_by_role_and_find_the_name() {
    let t = support::boot().await;
    three_employees(&t).await;
    let hana = t.user_with("Hana", "hr").await;
    let total = count(&t, "select count(*) from wl_hr_employees").await;
    assert!(total >= 3);

    let first = page(&t, &hana, "/hr/employees?per_page=10").await;
    assert_eq!(range(&first), (1, total.min(10), total));
    assert!(first.contains("Zainab Rahman"), "the name is shown");
    assert!(first.contains("Security guard"), "the role in words");
    let second = page(&t, &hana, "/hr/employees?per_page=10&page=2").await;
    assert_eq!(range(&second).2, total);
    let sorted = page(&t, &hana, "/hr/employees?sort=-hire_date").await;
    assert_eq!(range(&sorted).2, total);

    let found = page(&t, &hana, "/hr/employees?search=zainab").await;
    assert_eq!(range(&found).2, 1, "the name through the directory");
    assert!(found.contains("Zainab Rahman"));
    assert!(!found.contains("Omar Saleh"));
    let nobody = page(&t, &hana, "/hr/employees?search=nobody-has-this").await;
    assert_eq!(range(&nobody).2, 0);

    let guards = page(&t, &hana, "/hr/employees?in.job_role=security_guard").await;
    assert_eq!(
        range(&guards).2,
        count(
            &t,
            "select count(*) from wl_hr_employees where job_role = 'security_guard'"
        )
        .await
    );
    assert!(guards.contains("Zainab Rahman"));
    assert!(!guards.contains("Priya Nair"));

    t.app.acting_as(&hana);
    let file = t.app.get("/hr/employees?export=csv").await;
    file.assert_ok();
    assert!(
        file.text().contains("Security guard"),
        "the export in words"
    );
    let manager = t.user_with("Mona", "manager").await;
    t.app.acting_as(&manager);
    t.app
        .get("/hr/employees?export=csv")
        .await
        .assert_forbidden();
}

/// The payroll runs: filter by year and status, sort, export for who
/// prepares and refused to who only reads.
#[renox::test]
async fn the_payroll_runs_filter_by_year_and_status() {
    let t = support::boot().await;
    t.payroll_run("2025-12", PayrollRunStatus::Paid).await;
    t.payroll_run("2026-10", PayrollRunStatus::Calculated).await;
    t.payroll_run("2026-11", PayrollRunStatus::Draft).await;
    let prep = t.user_with("Pia", "payroll_prep").await;

    let total = count(&t, "select count(*) from wl_hr_payroll_runs").await;
    let all = page(&t, &prep, "/hr/payroll?per_page=10&sort=period").await;
    assert_eq!(range(&all), (1, total.min(10), total));
    let last_year = page(&t, &prep, "/hr/payroll?in.year=2025").await;
    assert_eq!(
        range(&last_year).2,
        count(
            &t,
            "select count(*) from wl_hr_payroll_runs where period like '2025-%'"
        )
        .await
    );
    assert!(last_year.contains("2025-12"));
    let drafts = page(&t, &prep, "/hr/payroll?in.status=draft").await;
    assert_eq!(
        range(&drafts).2,
        count(
            &t,
            "select count(*) from wl_hr_payroll_runs where status = 'draft'"
        )
        .await
    );
    let found = page(&t, &prep, "/hr/payroll?search=2026-1").await;
    assert_eq!(
        range(&found).2,
        count(
            &t,
            "select count(*) from wl_hr_payroll_runs where period like '%2026-1%'"
        )
        .await
    );

    t.app.acting_as(&prep);
    t.app.get("/hr/payroll?export=csv").await.assert_ok();
    let reader = t.user_with("Ali", "accountant").await;
    t.app.acting_as(&reader);
    t.app.get("/hr/payroll?export=csv").await.assert_forbidden();
}

/// The settlements: the employee found by name, filter by status, sort.
#[renox::test]
async fn the_settlements_find_the_employee_and_filter_by_status() {
    let t = support::boot().await;
    let employees = three_employees(&t).await;
    t.settlement_in(&employees[0], SettlementStatus::Draft, |_| {})
        .await;
    t.settlement_in(&employees[1], SettlementStatus::Paid, |_| {})
        .await;
    let prep = t.user_with("Sana", "settle_prep").await;

    let all = page(&t, &prep, "/hr/settlements?per_page=10&sort=-net").await;
    assert_eq!(range(&all), (1, 2, 2));
    assert!(all.contains("Zainab Rahman") && all.contains("Omar Saleh"));
    let found = page(&t, &prep, "/hr/settlements?search=omar").await;
    assert_eq!(range(&found).2, 1);
    assert!(found.contains("Omar Saleh") && !found.contains("Zainab Rahman"));
    let drafts = page(&t, &prep, "/hr/settlements?in.status=draft").await;
    assert_eq!(range(&drafts).2, 1);
    assert!(drafts.contains("Zainab Rahman"));
}

/// The leave queue: the employee found by name, sort and pages, the row
/// actions kept.
#[renox::test]
async fn the_leave_queue_finds_the_employee_and_keeps_its_actions() {
    let t = support::boot().await;
    let employees = three_employees(&t).await;
    for (employee, from) in [(&employees[0], 1), (&employees[1], 8)] {
        let mut request = LeaveRequest {
            employee_id: employee.id,
            leave_type: "annual".into(),
            from_date: NaiveDate::from_ymd_opt(2026, 12, from).unwrap(),
            to_date: NaiveDate::from_ymd_opt(2026, 12, from + 2).unwrap(),
            status: LeaveStatus::Requested,
            ..LeaveRequest::default()
        };
        let mut tx = t.db().begin().await.unwrap();
        request
            .insert_in_state(&mut tx, LeaveStatus::Requested, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    let hana = t.user_in_branch("Hana", "hr").await;

    let all = page(&t, &hana, "/hr/leave/queue?per_page=10&sort=-from_date").await;
    assert_eq!(range(&all), (1, 2, 2));
    assert!(all.contains("Approve") && all.contains("Reject"));
    let found = page(&t, &hana, "/hr/leave/queue?search=zainab").await;
    assert_eq!(range(&found).2, 1);
    assert!(found.contains("Zainab Rahman") && !found.contains("Omar Saleh"));
    let requested = page(&t, &hana, "/hr/leave/queue?in.status=requested").await;
    assert_eq!(range(&requested).2, 2);
}
