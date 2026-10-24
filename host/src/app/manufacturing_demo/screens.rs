//! The screen map of the demo company (BL-MFG-14): every page of the
//! Manufacturing vertical, and the shared pages it uses, with its URL, the
//! real ids read from the database the seed made, the person of the demo who
//! owns it and one line on what it shows. `manufacturing:demo` prints it;
//! `tests/manufacturing_demo_screens.rs` opens every URL as its owner.

use renox::prelude::*;

use super::{
    ACCOUNTANT_EMAIL, ADMIN_EMAIL, BUYER_EMAIL, MANAGER_EMAIL, OWNER_EMAIL, PLANNER_EMAIL,
    QC_EMAIL, SALES_EMAIL, SALES_MANAGER_EMAIL, STORE_EMAIL, SUPERVISOR_EMAIL,
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
            "The owner's tiles: work, quality, receivables and cash",
        ),
        (
            MANAGER_EMAIL,
            "The production manager's tiles: work orders, NCRs, returns",
        ),
        (
            PLANNER_EMAIL,
            "The planner's tiles: orders to release and on hold",
        ),
        (
            SALES_EMAIL,
            "Sales' tiles: returned goods to decide and orders on hold",
        ),
        (QC_EMAIL, "QC's tiles: NCRs and returned lots to inspect"),
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
        "The approvals inbox: write-offs, late entries, aborts and the owner's own requests",
    );

    let g = "Work orders";
    add(
        g,
        PLANNER_EMAIL,
        "/manufacturing/work-orders".to_owned(),
        "The work orders of the plant, with the status and the source",
    );
    add(
        g,
        PLANNER_EMAIL,
        "/manufacturing/work-orders?status=open".to_owned(),
        "The list the tile Work orders in progress opens",
    );
    for (number_like, login, what) in [
        (
            "noor",
            SUPERVISOR_EMAIL,
            "Running for Al Noor's order: issued material, reports (one from a paper job card with the late-entry flag), the order's changes",
        ),
        (
            "done",
            MANAGER_EMAIL,
            "Completed and closed: the actual cost and the loss",
        ),
        (
            "abort",
            MANAGER_EMAIL,
            "The abort that waits for the production manager",
        ),
        ("stock", PLANNER_EMAIL, "Held for a machine, for stock"),
        (
            "fresh",
            PLANNER_EMAIL,
            "Planned for Gulf Fresh Foods: release it",
        ),
    ] {
        let q = match number_like {
            "noor" => {
                "select w.id as id from manufacturing_work_orders w where w.sales_order_id is not null and w.status = 'in_progress' order by w.id"
            }
            "done" => {
                "select id from manufacturing_work_orders where status = 'closed' order by id"
            }
            "abort" => {
                "select id from manufacturing_work_orders where abort_reason is not null order by id"
            }
            "stock" => {
                "select id from manufacturing_work_orders where status = 'on_hold' order by id"
            }
            _ => "select id from manufacturing_work_orders where status = 'planned' order by id",
        };
        if let Some(id) = first(db, q).await? {
            add(g, login, format!("/manufacturing/work-orders/{id}"), what);
        }
    }

    let g = "Bills of materials";
    add(
        g,
        MANAGER_EMAIL,
        "/boms".to_owned(),
        "The bills of materials with their versions",
    );
    if let Some(id) = first(db, "select id from wl_inventory_boms order by id").await? {
        add(
            g,
            MANAGER_EMAIL,
            format!("/boms/{id}"),
            "A released version: components, hours, release",
        );
    }

    let g = "Quality";
    add(
        g,
        QC_EMAIL,
        "/manufacturing/ncrs".to_owned(),
        "The NCRs: open, dispositioned, closed",
    );
    if let Some(id) = first(
        db,
        "select id from manufacturing_ncrs where status = 'open' order by id",
    )
    .await?
    {
        add(
            g,
            QC_EMAIL,
            format!("/manufacturing/ncrs/{id}"),
            "An open NCR on a quarantined liner lot: disposition",
        );
    }
    if let Some(id) = first(
        db,
        "select id from manufacturing_ncrs where status != 'open' order by id",
    )
    .await?
    {
        add(
            g,
            QC_EMAIL,
            format!("/manufacturing/ncrs/{id}"),
            "A dispositioned NCR: scrap, with its history",
        );
    }
    add(
        g,
        QC_EMAIL,
        "/manufacturing/inspections".to_owned(),
        "The returned lots waiting for QC: pass or fail",
    );
    add(
        g,
        MANAGER_EMAIL,
        "/manufacturing/conformity".to_owned(),
        "The flags of the items (regulated, export only, customer specific) and their certificates",
    );
    add(
        g,
        ADMIN_EMAIL,
        "/settings/compliance".to_owned(),
        "The 07 values for documents, notices, late entries, onboarding and SOPs",
    );

    let g = "Sales";
    add(
        g,
        SALES_EMAIL,
        "/sales-orders".to_owned(),
        "The orders, with the delivery progress of each line",
    );
    for (key, login, what) in [
        (
            "noor",
            SALES_EMAIL,
            "Al Noor's order: lines picked, out for delivery and delivered, the work started",
        ),
        (
            "sahara",
            SALES_MANAGER_EMAIL,
            "Sahara Retail's order on credit hold: release it",
        ),
        (
            "eph",
            SALES_EMAIL,
            "The sleeves made for Emirates Print House: the cancel sheet proposes scrap",
        ),
    ] {
        let q = match key {
            "noor" => {
                "select o.id from wl_sales_sales_orders o join wl_parties_parties p on p.id = o.party_id where p.legal_name_en like 'Al Noor%' order by o.id"
            }
            "sahara" => {
                "select id from wl_sales_sales_orders where status = 'credit_hold' order by id"
            }
            _ => {
                "select o.id from wl_sales_sales_orders o join wl_parties_parties p on p.id = o.party_id where p.legal_name_en like 'Emirates Print%' order by o.id"
            }
        };
        if let Some(id) = first(db, q).await? {
            add(g, login, format!("/sales-orders/{id}"), what);
        }
    }
    add(
        g,
        SALES_MANAGER_EMAIL,
        "/credit/accounts".to_owned(),
        "The credit accounts with limit, exposure and hold",
    );
    add(
        g,
        SALES_EMAIL,
        "/manufacturing/returns".to_owned(),
        "The returned goods sales decides on: deliver again or cancel",
    );

    let g = "Stock and shipping";
    add(
        g,
        STORE_EMAIL,
        "/stock".to_owned(),
        "The stock by item and location",
    );
    add(
        g,
        STORE_EMAIL,
        "/dispatches".to_owned(),
        "The dispatches: a draft, one picked, one out, two exports",
    );
    if let Some(id) = first(
        db,
        "select id from wl_inventory_dispatches where status = 'draft' order by id",
    )
    .await?
    {
        add(
            g,
            STORE_EMAIL,
            format!("/dispatches/{id}"),
            "A draft dispatch: pick it",
        );
    }
    add(
        g,
        STORE_EMAIL,
        "/stock/holds".to_owned(),
        "The stock holds, including the recall's",
    );
    add(g, OWNER_EMAIL, "/recalls".to_owned(), "The recalls");
    if let Some(id) = first(db, "select id from wl_inventory_recalls order by id").await? {
        add(
            g,
            OWNER_EMAIL,
            format!("/recalls/{id}"),
            "The liner recall: lots, hold, trace",
        );
        add(
            g,
            OWNER_EMAIL,
            format!("/manufacturing/recalls/{id}"),
            "The customers the recall reached",
        );
    }
    add(
        g,
        ACCOUNTANT_EMAIL,
        "/manufacturing/exports".to_owned(),
        "The export evidence: the dispatch 65 days out without it",
    );

    let g = "Money";
    add(
        g,
        ACCOUNTANT_EMAIL,
        "/invoices".to_owned(),
        "The invoices, the overdue one of Sahara Retail among them",
    );
    add(
        g,
        ACCOUNTANT_EMAIL,
        "/receivables/ageing".to_owned(),
        "Who owes what and for how long",
    );
    add(
        g,
        OWNER_EMAIL,
        "/cash-forecast".to_owned(),
        "The 13-week cash forecast and its AED 200,000 alert line",
    );
    add(
        g,
        ADMIN_EMAIL,
        "/settings/negative-flow".to_owned(),
        "The 07 values for sales, stock, receivables, cash and budget",
    );
    add(
        g,
        OWNER_EMAIL,
        "/budgets".to_owned(),
        "The budgets, approved by the owner",
    );

    let g = "People and documents";
    add(
        g,
        ADMIN_EMAIL,
        "/hr/onboarding-templates".to_owned(),
        "The five onboarding checklists",
    );
    add(
        g,
        ADMIN_EMAIL,
        "/documents/sops".to_owned(),
        "The SOP library with owners and readers",
    );
    add(
        g,
        ADMIN_EMAIL,
        "/document-kinds".to_owned(),
        "The document kinds: conformity certificates, licences, exit evidence",
    );
    add(g, QC_EMAIL, "/tickets".to_owned(), "The tasks and tickets");

    let g = "Customer portal (Al Noor buyer)";
    add(
        g,
        BUYER_EMAIL,
        "/portal/manufacturing/invoices".to_owned(),
        "The customer's invoices, with Dispute this invoice",
    );
    add(
        g,
        BUYER_EMAIL,
        "/portal/manufacturing/orders".to_owned(),
        "Upload a customer PO: a duplicate is refused",
    );

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
            "  {:<44} {:<44} {}\n",
            screen.url,
            format!("[{}]", screen.login),
            screen.what
        ));
    }
    Ok(text)
}
