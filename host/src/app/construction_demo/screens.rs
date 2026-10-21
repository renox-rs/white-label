//! The screen map: the URLs, with real ids, of every F2 page and of the
//! screens of the shared modules that have data, read from the database the
//! seed made. The command prints it; the test of the demo opens every URL.

use renox::chrono::{Datelike, Days, NaiveDate, Weekday};
use renox::prelude::*;

use super::people::{self, LOGINS};
use super::roles::ADMIN;
use super::seed_hr::{STAFF, TWR};

/// One screen of the map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    /// The group it is printed under.
    pub group: &'static str,
    /// The URL, with the query it needs.
    pub url: String,
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

/// The last working day before `today`.
fn last_working_day(today: NaiveDate) -> NaiveDate {
    let mut day = today.checked_sub_days(Days::new(1)).unwrap_or(today);
    while matches!(day.weekday(), Weekday::Sat | Weekday::Sun) {
        day = day.checked_sub_days(Days::new(1)).unwrap_or(day);
    }
    day
}

/// The screens of the demo company.
///
/// # Errors
///
/// The database's errors.
pub async fn map(state: &AppState) -> Result<Vec<Screen>> {
    let db = &state.db;
    let mut screens: Vec<Screen> = Vec::new();
    let mut add = |group: &'static str, url: String, what: &str| {
        screens.push(Screen {
            group,
            url,
            what: what.to_owned(),
        });
    };
    let ipa = |filter: &str| {
        format!(
            "select i.id as id from construction_ipas i join construction_projects p on p.id = i.project_id where {filter} order by i.id"
        )
    };

    // Projects and IPAs.
    let g = "Construction";
    add(
        g,
        "/construction/projects".to_owned(),
        "The projects: status, contract value, IPAs and what waits on each",
    );
    for (filter, what) in [
        (
            "p.code = 'TWR-01' and i.status = 'submitted'",
            "IPA submitted more than 14 days ago, not certified; a deduction waits for the MD",
        ),
        (
            "p.code = 'TWR-01' and i.status = 'superseded'",
            "IPA superseded by the next one",
        ),
        (
            "p.code = 'TWR-01' and i.status = 'invoiced' and i.number = 2",
            "IPA invoiced with deductions (credit note, dispute, back-charge set off)",
        ),
        (
            "p.code = 'TRM-07' and i.status = 'certified'",
            "Final account of the terminated project, certified",
        ),
        (
            "p.code = 'MALL-02' and i.status = 'draft'",
            "IPA in draft, with progress",
        ),
        (
            "p.code = 'CLN-03' and i.number = 2",
            "IPA invoiced, unpaid and overdue",
        ),
    ] {
        if let Some(id) = first(db, &ipa(filter)).await? {
            add(g, format!("/construction/ipas/{id}"), what);
        }
    }
    for (code, what) in [
        ("TRM-07", "Closing screen of the terminated project"),
        (
            "OFC-06",
            "Closing screen of the completed project in its defects liability period",
        ),
        ("TWR-01", "Closing screen of a project in progress"),
    ] {
        let q = format!("select id from construction_projects where code = '{code}'");
        if let Some(id) = first(db, &q).await? {
            add(g, format!("/construction/projects/{id}/closing"), what);
        }
    }
    for (code, what) in [
        (
            "TWR-01",
            "Variation report: every state, the gap between price and approval",
        ),
        (
            "MALL-02",
            "Variation report: approved, waiting, extension of time",
        ),
        ("CLN-03", "Variation report: a rejected variation"),
    ] {
        let q = format!("select id from construction_projects where code = '{code}'");
        if let Some(id) = first(db, &q).await? {
            add(
                g,
                format!("/construction/projects/{id}/variation-report"),
                what,
            );
        }
    }

    // People and attendance.
    let g = "People and attendance";
    add(
        g,
        "/attendance/crew".to_owned(),
        "The form that opens a crew list: location, day and crew",
    );
    let site = first(
        db,
        "select w.id as id from wl_attendance_work_locations w join construction_projects p on p.id = w.scope_id where p.code = 'TWR-01' order by w.id",
    )
    .await?;
    if let Some(site) = site {
        let crew: Vec<String> = ids(
            db,
            "select e.id as id from wl_hr_employees e join wl_hr_employee_scopes s on s.employee_id = e.id join construction_projects p on p.id = cast(s.scope_id as integer) where p.code = 'TWR-01' order by e.id",
        )
        .await?
        .into_iter()
        .map(|id| id.to_string())
        .collect();
        let date = last_working_day(super::super::demo_data::real_today());
        add(
            g,
            format!(
                "/attendance/crew?location_id={site}&date={date}&employees={}",
                crew.join(",")
            ),
            "Crew list of the tower site on the last working day",
        );
    }
    add(g, "/hr/employees".to_owned(), "Employees");
    for (key, what) in [
        ("karim", "Employee with an overtime flag on a record"),
        ("sher", "Worker with an open absence case"),
        ("dennis", "Worker whose work permit expired"),
        ("imran", "Worker whose work permit expires in 20 days"),
        ("anitha", "New hire in onboarding"),
    ] {
        let Some(member) = STAFF.iter().find(|member| member.key == key) else {
            continue;
        };
        let q = format!(
            "select e.id as id from wl_hr_employees e join wl_parties_parties p on p.id = e.party_id where p.legal_name_en = '{}'",
            member.en
        );
        if let Some(id) = first(db, &q).await? {
            add(g, format!("/hr/employees/{id}"), what);
        }
    }
    add(
        g,
        "/hr/onboarding-templates".to_owned(),
        "Onboarding checklists",
    );
    add(g, "/tickets".to_owned(), "Tickets, with the absence case");
    if let Some(id) = first(
        db,
        "select t.id as id from wl_ticketing_tickets t join wl_ticketing_categories c on c.id = t.category_id where c.key = 'absence' order by t.id",
    )
    .await?
    {
        add(g, format!("/tickets/{id}"), "The absence case ticket for HR");
    }
    add(g, "/deadline-tasks".to_owned(), "Deadline tasks");

    // Documents and parties.
    let g = "Parties and documents";
    add(
        g,
        "/parties".to_owned(),
        "Clients, suppliers and subcontractors",
    );
    for (name, what) in [
        (people::AL_NOOR, "The client blocked by compliance"),
        (people::MEP, "Dubai subcontractor, registration valid"),
        (people::GLASS, "Dubai subcontractor, registration expired"),
    ] {
        let q = format!("select id from wl_parties_parties where legal_name_en = '{name}'");
        if let Some(id) = first(db, &q).await? {
            add(g, format!("/parties/{id}"), what);
            if name != people::AL_NOOR {
                add(
                    g,
                    format!("/documents/of/parties.party/{id}"),
                    "Documents of the subcontractor",
                );
            }
        }
    }
    add(
        g,
        "/documents/review".to_owned(),
        "Documents waiting for review",
    );
    add(g, "/documents/flags".to_owned(), "Document flags");
    add(g, "/documents/sops".to_owned(), "SOP library");
    add(g, "/compliance-alerts".to_owned(), "Compliance alerts");

    // Stock and procurement.
    let g = "Stock and procurement";
    for (code, what) in [
        (
            "CABLE-25",
            "Suppliers of an item; a change of preferred supplier is requested",
        ),
        ("GYP-12", "Suppliers of an item, with the last prices"),
    ] {
        let q = format!("select id from wl_inventory_items where code = '{code}'");
        if let Some(id) = first(db, &q).await? {
            add(g, format!("/items/{id}/suppliers"), what);
        }
    }
    if let Some(id) = first(
        db,
        "select id from wl_inventory_reorder_suggestions order by id",
    )
    .await?
    {
        add(
            g,
            format!("/reorder-suggestions/{id}"),
            "Reorder suggestion",
        );
    }
    add(
        g,
        "/preferred-requests".to_owned(),
        "Preferred-supplier requests",
    );
    if let Some(id) = first(
        db,
        "select id from wl_inventory_preferred_supplier_requests where status = 'pending' order by id",
    )
    .await?
    {
        add(
            g,
            format!("/preferred-requests/{id}"),
            "A request to change the preferred supplier, waiting",
        );
    }
    for name in [people::GYPSUM, people::ELECTRICAL] {
        let q = format!("select id from wl_parties_parties where legal_name_en = '{name}'");
        if let Some(id) = first(db, &q).await? {
            add(g, format!("/suppliers/{id}/score"), "Supplier score");
        }
    }
    if let Some(id) = first(
        db,
        "select id from wl_inventory_purchase_orders where status = 'issued' order by id",
    )
    .await?
    {
        add(
            g,
            format!("/purchase-orders/{id}/revisions/1/document"),
            "Purchase order document",
        );
    }

    // Money.
    let g = "Money";
    add(g, "/invoices".to_owned(), "Invoices");
    for (filter, what) in [
        (
            "status = 'issued' and source_type = 'construction.ipa'",
            "Invoice issued, not yet due",
        ),
        (
            "status = 'overdue' and disputed = 0 and source_type = 'construction.ipa'",
            "Invoice overdue",
        ),
        (
            "disputed = 1 and source_type = 'construction.ipa'",
            "Invoice disputed (contested deduction)",
        ),
        (
            "status = 'paid' and source_type = 'construction.ipa'",
            "Invoice paid",
        ),
        (
            "source_type = 'construction.retention'",
            "Retention invoice",
        ),
    ] {
        let q = format!("select id from wl_invoicing_invoices where {filter} order by id desc");
        if let Some(id) = first(db, &q).await? {
            add(g, format!("/invoices/{id}"), what);
        }
    }
    add(g, "/credit-notes".to_owned(), "Credit notes");
    if let Some(id) = first(db, "select id from wl_invoicing_credit_notes order by id").await? {
        add(g, format!("/credit-notes/{id}"), "Credit note");
    }
    add(g, "/payments".to_owned(), "Payments, with the set-off");
    if let Some(id) = first(
        db,
        "select id from wl_payments_payments where method = 'set_off' order by id",
    )
    .await?
    {
        add(
            g,
            format!("/payments/{id}"),
            "Set-off of a back-charge against an invoice",
        );
    }
    add(g, "/supplier-bills".to_owned(), "Supplier bills");
    if let Some(id) = first(
        db,
        "select id from wl_invoicing_supplier_bills where status = 'partially_paid' order by id",
    )
    .await?
    {
        add(
            g,
            format!("/supplier-bills/{id}"),
            "Supplier bill paid in part",
        );
    }
    add(g, "/budgets".to_owned(), "Budgets");
    if let Some(id) = first(
        db,
        "select id from wl_invoicing_company_budgets order by id desc",
    )
    .await?
    {
        add(g, format!("/budgets/{id}"), "The company budget");
    }
    add(g, "/budget-categories".to_owned(), "Budget categories");
    add(
        g,
        "/budget-report".to_owned(),
        "Budget against actual, the materials cost code at 92%",
    );
    add(g, "/cash-forecast".to_owned(), "Cash forecast");
    add(g, "/receivables/ageing".to_owned(), "Receivables ageing");
    add(g, "/cheques".to_owned(), "Cheques");
    if let Some(id) = first(
        db,
        "select id from wl_payments_cheques where status = 'bounced' order by id",
    )
    .await?
    {
        add(g, format!("/cheques/{id}"), "The bounced cheque");
    }
    add(
        g,
        "/refunds".to_owned(),
        "Refunds: requested, approved, rejected",
    );
    if let Some(id) = first(
        db,
        "select id from wl_payments_refunds where status = 'requested' order by id",
    )
    .await?
    {
        add(g, format!("/refunds/{id}"), "A refund waiting for approval");
    }

    // Settings and administration.
    let g = "Settings";
    add(
        g,
        "/settings/negative-flow".to_owned(),
        "Settings of the shared modules for this client",
    );
    add(g, "/settings/compliance".to_owned(), "Compliance settings");
    add(g, "/legal-entities".to_owned(), "Legal entities");
    add(g, "/users".to_owned(), "Users");
    add(g, "/activity".to_owned(), "Activity log");
    add(g, "/notifications".to_owned(), "Notifications");
    let _ = (ADMIN, LOGINS, TWR);
    Ok(screens)
}

/// The map as the command prints it.
///
/// # Errors
///
/// The database's errors.
pub(crate) async fn print(state: &AppState) -> Result<String> {
    let screens = map(state).await?;
    let mut text =
        String::from("\nSCREEN MAP (sign in as admin@alwaha.example, password password123)\n");
    let mut group = "";
    for screen in &screens {
        if screen.group != group {
            group = screen.group;
            text.push_str(&format!("\n{group}\n"));
        }
        text.push_str(&format!("  {:<62} {}\n", screen.url, screen.what));
    }
    Ok(text)
}
