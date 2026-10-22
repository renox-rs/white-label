//! The screen map of the demo agency (BL-MPW-22 to BL-MPW-32): every page of
//! the Manpower vertical with its URL, the real ids read from the database the
//! seed made, the person of the demo who owns it, and one line on what it
//! shows. `manpower:demo` prints it; `tests/manpower_demo_screens.rs` opens
//! every URL as its owner.

use renox::prelude::*;

const ADMIN_EMAIL: &str = "admin@desertstar.example";
const MD_EMAIL: &str = "md@desertstar.example";
const HR_EMAIL: &str = "hr@desertstar.example";
const ACCOUNT_MANAGER_EMAIL: &str = "accountmanager@desertstar.example";
const SUPERVISOR_EMAIL: &str = "supervisor@desertstar.example";
const APPROVER_EMAIL: &str = "approver@gulftower.example";

/// One screen of the map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    /// The group it is printed under.
    pub group: &'static str,
    /// The URL, with the query it needs.
    pub url: String,
    /// The login (e-mail) of the role that owns it.
    pub login: &'static str,
    /// What it shows.
    pub what: String,
}

/// The ids of the first column of `query`.
async fn ids(db: &Db, query: &str) -> Result<Vec<i64>> {
    let rows = renox::db::sql(query.to_owned()).fetch_all(db).await?;
    let mut found = Vec::new();
    for row in rows {
        found.push(row.try_get::<i64>("id")?);
    }
    Ok(found)
}

/// The first id of `query`, if any.
async fn first(db: &Db, query: &str) -> Result<Option<i64>> {
    Ok(ids(db, query).await?.into_iter().next())
}

/// The screens of the demo agency.
///
/// # Errors
///
/// The database's errors.
pub async fn map(state: &AppState) -> Result<Vec<Screen>> {
    let db = &state.db;
    let mut screens: Vec<Screen> = Vec::new();
    let mut add = |group: &'static str, login: &'static str, url: String, what: &str| {
        screens.push(Screen {
            group,
            url,
            login,
            what: what.to_owned(),
        });
    };

    let g = "Start";
    for (login, what) in [
        (
            ADMIN_EMAIL,
            "Every tile with a count, each linking to its list",
        ),
        (
            ACCOUNT_MANAGER_EMAIL,
            "The account manager's tiles: his clients only, no payroll or absence",
        ),
        (
            HR_EMAIL,
            "HR's tiles: absence cases, payroll runs and worker documents",
        ),
    ] {
        add(g, login, "/dashboard".to_owned(), what);
    }
    add(
        g,
        ADMIN_EMAIL,
        "/menu".to_owned(),
        "Every link the person may open, for the phone",
    );
    add(
        g,
        MD_EMAIL,
        "/approvals".to_owned(),
        "The approvals inbox: what waits for the MD, and the MD's own requests",
    );

    let g = "Contracts";
    add(
        g,
        ACCOUNT_MANAGER_EMAIL,
        "/manpower/contracts".to_owned(),
        "The client contracts, with the hold and the end date",
    );
    add(
        g,
        ACCOUNT_MANAGER_EMAIL,
        "/manpower/contracts?status=on_hold".to_owned(),
        "The list the tile Contracts on hold opens",
    );
    add(
        g,
        ACCOUNT_MANAGER_EMAIL,
        "/manpower/contracts?ending=60".to_owned(),
        "The list the tile Contracts ending within 60 days opens",
    );
    for (reference, what) in [
        (
            "CC-DEMO-001",
            "A running contract: rate card, requests, deployments, approvals",
        ),
        (
            "CC-DEMO-002",
            "A contract that ends in 45 days: the renewal",
        ),
        (
            "CC-DEMO-003",
            "A contract on credit hold: reasons, state before, release for the MD",
        ),
    ] {
        let q = format!("select id from manpower_client_contracts where reference = '{reference}'");
        if let Some(id) = first(db, &q).await? {
            add(
                g,
                ACCOUNT_MANAGER_EMAIL,
                format!("/manpower/contracts/{id}"),
                what,
            );
        }
    }

    let g = "Rate cards";
    if let Some(id) = first(db, "select id from wl_invoicing_rate_cards order by id").await? {
        add(
            g,
            ACCOUNT_MANAGER_EMAIL,
            format!("/rate-cards/{id}"),
            "A rate card: the versions and who accepted them",
        );
        let q = format!(
            "select id from wl_invoicing_rate_card_versions where rate_card_id = {id} order by id"
        );
        if let Some(version) = first(db, &q).await? {
            add(
                g,
                ACCOUNT_MANAGER_EMAIL,
                format!("/rate-card-versions/{version}"),
                "A rate card version: the lines, the dates, the price preview",
            );
        }
    }

    let g = "Requests";
    add(
        g,
        ACCOUNT_MANAGER_EMAIL,
        "/manpower/requests".to_owned(),
        "The workers the clients asked for",
    );
    add(
        g,
        ACCOUNT_MANAGER_EMAIL,
        "/manpower/requests?status=missing".to_owned(),
        "The list the tile Unfilled requests opens",
    );
    for (status, what) in [
        (
            "partially_filled",
            "A request partly filled: the positions, the deployments, the plan",
        ),
        ("open", "An open request with no worker yet"),
    ] {
        let q = format!("select id from manpower_requests where status = '{status}' order by id");
        if let Some(id) = first(db, &q).await? {
            add(
                g,
                ACCOUNT_MANAGER_EMAIL,
                format!("/manpower/requests/{id}"),
                what,
            );
        }
    }

    let g = "Deployments";
    add(
        g,
        ACCOUNT_MANAGER_EMAIL,
        "/manpower/deployments".to_owned(),
        "The workers on site, with the document flags",
    );
    add(
        g,
        SUPERVISOR_EMAIL,
        "/manpower/deployments".to_owned(),
        "The site supervisor's list: his site only",
    );
    add(
        g,
        ACCOUNT_MANAGER_EMAIL,
        "/manpower/deployments?status=suspended".to_owned(),
        "The list the tile Deployments suspended opens",
    );
    for (status, what) in [
        ("active", "A worker on site: the guards, the end, the cover"),
        (
            "suspended",
            "A suspended worker: the days, the suspension and the resume",
        ),
    ] {
        let q =
            format!("select id from manpower_deployments where status = '{status}' order by id");
        if let Some(id) = first(db, &q).await? {
            add(
                g,
                ACCOUNT_MANAGER_EMAIL,
                format!("/manpower/deployments/{id}"),
                what,
            );
        }
    }

    let g = "Absence";
    add(
        g,
        HR_EMAIL,
        "/manpower/absence".to_owned(),
        "The open absence cases and the MoHRE report",
    );
    if let Some(id) = first(db, "select id from manpower_absence_cases order by id").await? {
        add(
            g,
            HR_EMAIL,
            format!("/manpower/absence/{id}"),
            "An absence case: the days, the report, the cover, the close",
        );
    }

    let g = "Timesheets";
    add(
        g,
        ACCOUNT_MANAGER_EMAIL,
        "/timesheets".to_owned(),
        "The timesheets of the clients, by standing",
    );
    add(
        g,
        ACCOUNT_MANAGER_EMAIL,
        "/timesheets?status=submitted".to_owned(),
        "The list the tile Timesheets waiting for the client opens",
    );
    for (filter, what) in [
        (
            "status = 'submitted'",
            "A timesheet waiting for the client approver",
        ),
        (
            "status = 'locked'",
            "A locked timesheet, billed on an invoice",
        ),
    ] {
        let q = format!("select id from wl_attendance_timesheets where {filter} order by id");
        if let Some(id) = first(db, &q).await? {
            add(g, ACCOUNT_MANAGER_EMAIL, format!("/timesheets/{id}"), what);
        }
    }

    let g = "Payroll";
    add(
        g,
        HR_EMAIL,
        "/hr/payroll".to_owned(),
        "The payroll runs and the start of a new one",
    );
    if let Some(id) = first(db, "select id from wl_hr_payroll_runs order by id").await? {
        add(
            g,
            HR_EMAIL,
            format!("/hr/payroll/{id}"),
            "A payroll run: the lines, the held line, approve and the SIF file",
        );
    }
    add(
        g,
        HR_EMAIL,
        "/hr/settlements".to_owned(),
        "The final settlements of leavers",
    );
    if let Some(id) = first(db, "select id from wl_hr_final_settlements order by id").await? {
        add(
            g,
            HR_EMAIL,
            format!("/hr/settlements/{id}"),
            "A final settlement: the amounts and the payment",
        );
    }
    if let Some(id) = first(db, "select id from wl_hr_pay_accounts order by id").await? {
        add(
            g,
            HR_EMAIL,
            format!("/hr/pay-accounts/{id}"),
            "A pay account of a worker: bank, status, the approval",
        );
    }

    let g = "Workers";
    add(g, HR_EMAIL, "/hr/employees".to_owned(), "The workers");
    if let Some(id) = first(
        db,
        "select employee_id as id from manpower_deployments where status = 'active' order by id",
    )
    .await?
    {
        add(
            g,
            HR_EMAIL,
            format!("/hr/employees/{id}"),
            "A worker: overview, documents, deployments, timesheets, pay accounts",
        );
        add(
            g,
            HR_EMAIL,
            format!("/hr/employees/{id}?reveal=salary"),
            "The salary tab after Reveal (audited)",
        );
        add(
            g,
            ACCOUNT_MANAGER_EMAIL,
            format!("/hr/employees/{id}"),
            "The account manager's read of his client's worker, without salary",
        );
        add(
            g,
            HR_EMAIL,
            format!("/documents/of/hr.employee/{id}"),
            "The documents of a worker, with validity, and the upload",
        );
    }

    let g = "Client portal (Gulf Tower approver)";
    add(
        g,
        APPROVER_EMAIL,
        "/dashboard".to_owned(),
        "The client approver's home: the timesheets waiting for them",
    );
    add(
        g,
        APPROVER_EMAIL,
        "/portal/manpower/requests".to_owned(),
        "The client's requests: write, submit, cancel",
    );
    add(
        g,
        APPROVER_EMAIL,
        "/portal/manpower/deployments".to_owned(),
        "The workers on the client's sites",
    );
    add(
        g,
        APPROVER_EMAIL,
        "/portal/manpower/timesheets".to_owned(),
        "The timesheets to approve",
    );
    if let Some(id) = first(
        db,
        "select id from wl_attendance_timesheets where status = 'submitted' order by id",
    )
    .await?
    {
        add(
            g,
            APPROVER_EMAIL,
            format!("/timesheets/{id}"),
            "A timesheet page for the client approver: approve or reject",
        );
    }

    Ok(screens)
}

/// The map as the command prints it.
///
/// # Errors
///
/// The database's errors.
pub(crate) async fn print(state: &AppState) -> Result<String> {
    let screens = map(state).await?;
    let mut text = format!(
        "\nSCREEN MAP (password {}; the login owning each screen is in brackets)\n",
        super::PASSWORD
    );
    let mut group = "";
    for screen in &screens {
        if screen.group != group {
            group = screen.group;
            text.push_str(&format!("\n{group}\n"));
        }
        text.push_str(&format!(
            "  {:<44} {:<36} {}\n",
            screen.url,
            format!("[{}]", screen.login),
            screen.what
        ));
    }
    Ok(text)
}
