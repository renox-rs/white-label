//! The screen map of the demo company: every page of the Auto vertical, and
//! the shared pages it uses, with its URL, the real ids read from the
//! database the seed made, the person of the demo who owns it and one line on
//! what it shows. `auto:demo` prints it; `tests/auto_demo_screens.rs` opens
//! every URL as its owner.

use renox::prelude::*;

use super::{
    ACCOUNTANT_EMAIL, ADMIN_EMAIL, ADVISOR_EMAIL, BRANCH_EMAIL, COUNTER_EMAIL, FLEET_EMAIL,
    INSURANCE_EMAIL, OWNER_EMAIL, PURCHASER_EMAIL, STORE_EMAIL, TECH_EMAIL, WORKSHOP_EMAIL,
};

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

/// The first id of `query`, if any.
async fn first(db: &Db, query: &str) -> Result<Option<i64>> {
    let rows = renox::db::sql(query.to_owned()).fetch_all(db).await?;
    match rows.first() {
        Some(row) => Ok(Some(row.try_get::<i64>("id")?)),
        None => Ok(None),
    }
}

/// The job card in `status` (the first one).
async fn job(db: &Db, status: &str) -> Result<Option<i64>> {
    first(
        db,
        &format!("select id from auto_job_cards where status = '{status}' order by id"),
    )
    .await
}

/// The screens of the demo company.
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
            OWNER_EMAIL,
            "The owner's dashboard: receivables and approvals",
        ),
        (BRANCH_EMAIL, "The branch manager's dashboard"),
        (ADVISOR_EMAIL, "The service advisor's dashboard"),
        (WORKSHOP_EMAIL, "The workshop manager's dashboard"),
        (ACCOUNTANT_EMAIL, "The accountant's dashboard"),
    ] {
        add(g, login, "/dashboard".to_owned(), what);
    }
    add(
        g,
        OWNER_EMAIL,
        "/menu".to_owned(),
        "Every link the person may open, for the phone",
    );
    add(
        g,
        OWNER_EMAIL,
        "/approvals".to_owned(),
        "The approvals inbox",
    );

    let g = "Workshop";
    add(
        g,
        ADVISOR_EMAIL,
        "/auto/job-cards".to_owned(),
        "Every job card with its status: checked in to delivered",
    );
    add(
        g,
        ADVISOR_EMAIL,
        "/scheduling/bookings".to_owned(),
        "The bookings of the bay, each checked in to its job card",
    );
    for (status, login, what) in [
        (
            "delivered",
            ADVISOR_EMAIL,
            "Delivered and invoiced: parts from the store, the customer's approval on the link",
        ),
        (
            "ready",
            ADVISOR_EMAIL,
            "Passed the QC, ready for collection",
        ),
        (
            "quality_check",
            WORKSHOP_EMAIL,
            "The work is done: the workshop manager's QC checklist",
        ),
        (
            "in_progress",
            ADVISOR_EMAIL,
            "Work running: parts issued, labour part done",
        ),
        (
            "awaiting_parts",
            ADVISOR_EMAIL,
            "Waiting for an alternator on special order, with its expected day",
        ),
        (
            "awaiting_approval",
            ADVISOR_EMAIL,
            "The estimate is with the customer",
        ),
        (
            "inspecting",
            ADVISOR_EMAIL,
            "The technician is inspecting: findings so far",
        ),
        (
            "checked_in",
            ADVISOR_EMAIL,
            "Just checked in: assign a technician",
        ),
        (
            "declined",
            ADVISOR_EMAIL,
            "The customer declined the clutch estimate",
        ),
    ] {
        if let Some(id) = job(db, status).await? {
            add(g, login, format!("/auto/job-cards/{id}"), what);
        }
    }
    if let Some(id) = job(db, "delivered").await? {
        add(
            g,
            ADVISOR_EMAIL,
            format!("/auto/job-cards/{id}/estimate"),
            "The estimate sheet: labour, parts, VAT, the promised day and the warranty",
        );
    }
    add(
        g,
        TECH_EMAIL,
        "/auto/technician".to_owned(),
        "The technician's own job cards (no contact details)",
    );
    add(
        g,
        WORKSHOP_EMAIL,
        "/auto/labour-menu".to_owned(),
        "The labour menu: flat-rate and hourly operations",
    );

    let g = "Vehicles";
    add(
        g,
        ADVISOR_EMAIL,
        "/auto/vehicles".to_owned(),
        "The customers' vehicles by plate",
    );
    if let Some(id) = first(
        db,
        "select vehicle_id as id from auto_job_cards where status = 'delivered' order by id",
    )
    .await?
    {
        add(
            g,
            ADVISOR_EMAIL,
            format!("/auto/vehicles/{id}"),
            "A vehicle with its service history and next service",
        );
    }

    let g = "Insurance";
    add(
        g,
        INSURANCE_EMAIL,
        "/auto/claims".to_owned(),
        "The insurance claims by where they stand",
    );
    for (status, what) in [
        (
            "approved",
            "Approved by the insurer line by line, with the excess the owner pays",
        ),
        ("estimate_submitted", "The estimate is with the insurer"),
    ] {
        if let Some(id) = first(
            db,
            &format!("select id from auto_insurance_claims where status = '{status}' order by id"),
        )
        .await?
        {
            add(g, INSURANCE_EMAIL, format!("/auto/claims/{id}"), what);
        }
    }
    if let Some(id) = first(
        db,
        "select job_card_id as id from auto_insurance_claims where status = 'approved' order by id",
    )
    .await?
    {
        add(
            g,
            ADVISOR_EMAIL,
            format!("/auto/job-cards/{id}"),
            "The insurance repair's job card: insurer and owner each billed their share",
        );
    }
    add(
        g,
        INSURANCE_EMAIL,
        "/auto/insurers".to_owned(),
        "The insurers with their agreed rates",
    );
    if let Some(id) = first(db, "select id from auto_insurers order by id").await? {
        add(
            g,
            INSURANCE_EMAIL,
            format!("/auto/insurers/{id}"),
            "An insurer: surveyors, payment terms, labour and storage rates",
        );
    }

    let g = "Fleet";
    add(
        g,
        FLEET_EMAIL,
        "/auto/fleet".to_owned(),
        "The fleet contracts",
    );
    for (status, what) in [
        (
            "active",
            "Active: two vans with their service plans and the contract's labour rate",
        ),
        ("draft", "In draft: waiting for the signed copy"),
    ] {
        if let Some(id) = first(
            db,
            &format!("select id from auto_fleet_contracts where status = '{status}' order by id"),
        )
        .await?
        {
            add(g, FLEET_EMAIL, format!("/auto/fleet/{id}"), what);
        }
    }

    let g = "Parts and sales";
    add(
        g,
        STORE_EMAIL,
        "/stock".to_owned(),
        "The stock of the workshop store and the Deira counter",
    );
    add(
        g,
        COUNTER_EMAIL,
        "/counter-sales".to_owned(),
        "The walk-in sales of the counter",
    );
    if let Some(id) = first(db, "select id from wl_sales_counter_sales order by id").await? {
        add(
            g,
            COUNTER_EMAIL,
            format!("/counter-sales/{id}"),
            "A counter sale paid in cash, with its VAT receipt",
        );
    }
    add(
        g,
        ADMIN_EMAIL,
        "/credit/accounts".to_owned(),
        "The B2B credit accounts: Quick Fix Garage",
    );
    add(
        g,
        ACCOUNTANT_EMAIL,
        "/auto/export-sales".to_owned(),
        "The zero-rated export waiting for its exit evidence",
    );
    if let Some(id) = first(db, "select id from wl_inventory_recalls order by id").await? {
        add(
            g,
            OWNER_EMAIL,
            format!("/recalls/{id}"),
            "The supplier's recall of the Toyota brake pads",
        );
        add(
            g,
            OWNER_EMAIL,
            format!("/auto/recalls/{id}"),
            "The customers who got the recalled pads, to tell",
        );
    }
    add(
        g,
        PURCHASER_EMAIL,
        "/auto/supplier-claims".to_owned(),
        "The warranty claims on suppliers",
    );

    let g = "Money";
    add(
        g,
        ACCOUNTANT_EMAIL,
        "/invoices".to_owned(),
        "The invoices: the delivered job, the B2B account, the export",
    );
    add(
        g,
        ACCOUNTANT_EMAIL,
        "/invoices?in.status=overdue".to_owned(),
        "Quick Fix Garage's overdue invoice",
    );
    if let Some(id) = first(
        db,
        "select invoice_id as id from auto_job_cards where invoice_id is not null order by id",
    )
    .await?
    {
        add(
            g,
            ACCOUNTANT_EMAIL,
            format!("/invoices/{id}"),
            "The invoice of the delivered job card",
        );
    }
    add(
        g,
        ACCOUNTANT_EMAIL,
        "/receivables/ageing".to_owned(),
        "Receivables by age",
    );

    let g = "People";
    add(
        g,
        ADMIN_EMAIL,
        "/parties".to_owned(),
        "The customers, insurers, suppliers and the fleet company",
    );
    add(
        g,
        ADMIN_EMAIL,
        "/users".to_owned(),
        "The people of the company and their roles",
    );
    Ok(screens)
}

/// The map as `auto:demo` prints it.
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
            "  {:<44} {:<44} {}\n",
            screen.url,
            format!("[{}]", screen.login),
            screen.what
        ));
    }
    Ok(text)
}
