//! The screen map of the demo firm: every page of the Accounting vertical,
//! and the shared pages it uses, with its URL, the real ids read from the
//! database the seed made, the person of the demo who owns it and one line on
//! what it shows. `accounting:demo` prints it;
//! `tests/accounting_demo_screens.rs` opens every URL as its owner.

use renox::prelude::*;

use super::{
    ACCOUNTANT_EMAIL, ADMIN_EMAIL, BILLING_EMAIL, CLIENT_FINANCE_EMAIL, CLIENT_OWNER_EMAIL,
    MANAGER_EMAIL, MLRO_EMAIL, PARTNER_EMAIL, ROSE_OWNER_EMAIL, TAX_EMAIL,
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

/// The party id of the client company named `name`.
async fn client(db: &Db, name: &str) -> Result<Option<i64>> {
    first(
        db,
        &format!(
            "select id from wl_parties_parties where legal_name_en = '{}'",
            name.replace('\'', "''")
        ),
    )
    .await
}

/// The engagement of the client named `name`.
async fn engagement(db: &Db, name: &str) -> Result<Option<i64>> {
    let Some(party) = client(db, name).await? else {
        return Ok(None);
    };
    first(
        db,
        &format!(
            "select id from accounting_engagements where client_party_id = {party} order by id"
        ),
    )
    .await
}

/// The screens of the demo firm.
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
            PARTNER_EMAIL,
            "The partner's dashboard: the tax profile change to decide, overdue retainers",
        ),
        (MANAGER_EMAIL, "The engagement manager's dashboard"),
        (ACCOUNTANT_EMAIL, "The accountant's dashboard"),
        (MLRO_EMAIL, "The MLRO's dashboard"),
        (BILLING_EMAIL, "The billing admin's dashboard: receivables"),
    ] {
        add(g, login, "/dashboard".to_owned(), what);
    }
    add(
        g,
        PARTNER_EMAIL,
        "/menu".to_owned(),
        "Every link the person may open, for the phone",
    );
    add(
        g,
        PARTNER_EMAIL,
        "/approvals".to_owned(),
        "The approvals inbox: Gulf Horizon's move to monthly VAT periods",
    );

    let g = "Engagements";
    add(
        g,
        MANAGER_EMAIL,
        "/accounting/engagements".to_owned(),
        "Every engagement: active, onboarding, on hold, ended, declined",
    );
    for (name, login, what) in [
        (
            "Marina Bay Trading LLC",
            MANAGER_EMAIL,
            "Active: letter signed in the portal, calendar, retainer, returns, request list received",
        ),
        (
            "Oasis Fresh Foods LLC",
            MANAGER_EMAIL,
            "Active (monthly VAT): an overdue retainer invoice and a return due in five days",
        ),
        (
            "Palm Tech Solutions LLC",
            MANAGER_EMAIL,
            "Onboarding: the CDD file waits for the MLRO, the letter is out for signature",
        ),
        (
            "Sand Dune Contracting LLC",
            PARTNER_EMAIL,
            "On hold for non-payment: tasks held, the deadline notice sent, two invoices overdue",
        ),
        (
            "Coral Reef Restaurants LLC",
            PARTNER_EMAIL,
            "Ended at the client's request: nothing left to bill",
        ),
        (
            "Pearl Fashion House LLC",
            MANAGER_EMAIL,
            "Declined: the letter was not signed",
        ),
    ] {
        if let Some(id) = engagement(db, name).await? {
            add(g, login, format!("/accounting/engagements/{id}"), what);
        }
    }
    if let Some(party) = client(db, "Marina Bay Trading LLC").await? {
        add(
            g,
            ACCOUNTANT_EMAIL,
            format!("/accounting/clients/{party}/tax-profile"),
            "Marina Bay's tax profile: TRN, quarterly VAT, corporate tax",
        );
    }
    if let Some(party) = client(db, "Gulf Horizon Logistics LLC").await? {
        add(
            g,
            PARTNER_EMAIL,
            format!("/accounting/clients/{party}/tax-profile"),
            "Gulf Horizon's tax profile with the change that waits",
        );
    }
    add(
        g,
        MANAGER_EMAIL,
        "/deadline-tasks".to_owned(),
        "The compliance calendar: VAT and corporate tax deadlines",
    );

    let g = "CDD and AML";
    for (name, what) in [
        (
            "Palm Tech Solutions LLC",
            "Pending approval: the MLRO approves or rejects",
        ),
        (
            "Al Waha General Trading FZE",
            "High risk with enhanced due diligence, due for its periodic review",
        ),
        ("Marina Bay Trading LLC", "Approved, low risk"),
    ] {
        if let Some(party) = client(db, name).await?
            && let Some(id) = first(
                db,
                &format!(
                    "select id from accounting_cdd_files where client_party_id = {party} order by id desc"
                ),
            )
            .await?
        {
            add(g, MLRO_EMAIL, format!("/accounting/cdd/{id}"), what);
        }
    }
    add(
        g,
        MLRO_EMAIL,
        "/accounting/aml-cases".to_owned(),
        "The AML cases, the MLRO's alone",
    );
    if let Some(id) = first(
        db,
        "select id from accounting_aml_cases where status = 'under_review' order by id",
    )
    .await?
    {
        add(
            g,
            MLRO_EMAIL,
            format!("/accounting/aml-cases/{id}"),
            "Al Waha's split cash deposits, under review: report or close",
        );
    }
    if let Some(id) = first(
        db,
        "select id from accounting_aml_cases where status = 'closed_no_action' order by id",
    )
    .await?
    {
        add(
            g,
            MLRO_EMAIL,
            format!("/accounting/aml-cases/{id}"),
            "A concern closed with no action, with its reason",
        );
    }
    add(
        g,
        MLRO_EMAIL,
        "/parties/screening".to_owned(),
        "The screenings: the re-screening hit on Al Waha's UBO",
    );

    let g = "Returns";
    add(
        g,
        TAX_EMAIL,
        "/accounting/returns".to_owned(),
        "Every return: preparing, in review, with the client, filed, corrected, cancelled",
    );
    for (status, login, what) in [
        (
            "client_approval",
            TAX_EMAIL,
            "Marina Bay's quarter, with the client for approval",
        ),
        (
            "in_review",
            TAX_EMAIL,
            "Desert Rose's quarter, with the reviewer",
        ),
        (
            "filed",
            TAX_EMAIL,
            "Filed by the client's staff, with the FTA acknowledgement",
        ),
        (
            "superseded",
            ACCOUNTANT_EMAIL,
            "Filed and corrected by a voluntary disclosure",
        ),
        (
            "cancelled",
            ACCOUNTANT_EMAIL,
            "Cancelled: the client files it itself",
        ),
    ] {
        if let Some(id) = first(
            db,
            &format!("select id from accounting_returns where status = '{status}' order by id"),
        )
        .await?
        {
            add(g, login, format!("/accounting/returns/{id}"), what);
        }
    }
    if let Some(id) = first(
        db,
        "select id from accounting_returns where status = 'preparing' and workpapers_attached = 0 order by due_date",
    )
    .await?
    {
        add(
            g,
            ACCOUNTANT_EMAIL,
            format!("/accounting/returns/{id}"),
            "Oasis Fresh Foods: due in five days, workpapers missing",
        );
    }

    let g = "KPI packs";
    add(
        g,
        MANAGER_EMAIL,
        "/accounting/kpi-packs".to_owned(),
        "The KPI packs of the fractional CFO service",
    );
    for (status, what) in [
        (
            "published",
            "Marina Bay's month, published with the commentary",
        ),
        ("draft", "Gulf Horizon's month in draft: add the commentary"),
    ] {
        if let Some(id) = first(
            db,
            &format!("select id from accounting_kpi_packs where status = '{status}' order by id"),
        )
        .await?
        {
            add(
                g,
                MANAGER_EMAIL,
                format!("/accounting/kpi-packs/{id}"),
                what,
            );
        }
    }

    let g = "Money";
    add(
        g,
        BILLING_EMAIL,
        "/invoices".to_owned(),
        "The retainer invoices of the month",
    );
    add(
        g,
        BILLING_EMAIL,
        "/invoices?in.status=overdue".to_owned(),
        "The overdue retainers: Oasis Fresh Foods and Sand Dune",
    );
    if let Some(id) = first(
        db,
        "select id from wl_invoicing_invoices where status = 'overdue' order by id",
    )
    .await?
    {
        add(
            g,
            BILLING_EMAIL,
            format!("/invoices/{id}"),
            "An overdue retainer invoice",
        );
    }
    add(
        g,
        BILLING_EMAIL,
        "/receivables/ageing".to_owned(),
        "Receivables by age",
    );
    add(
        g,
        BILLING_EMAIL,
        "/subscriptions".to_owned(),
        "The retainers as subscriptions: active, paused while on hold",
    );
    add(
        g,
        PARTNER_EMAIL,
        "/cash-forecast".to_owned(),
        "The cash-flow forecast",
    );

    let g = "People and documents";
    add(
        g,
        ADMIN_EMAIL,
        "/parties".to_owned(),
        "The client companies, their UBOs and directors",
    );
    if let Some(party) = client(db, "Al Waha General Trading FZE").await? {
        add(
            g,
            MLRO_EMAIL,
            format!("/parties/{party}"),
            "A high-risk client company",
        );
    }
    add(
        g,
        MLRO_EMAIL,
        "/documents/flags".to_owned(),
        "The document flags: Desert Rose's trade licence expires in three weeks",
    );
    add(
        g,
        ADMIN_EMAIL,
        "/users".to_owned(),
        "The firm's people and the client users",
    );

    let g = "Client portal (Marina Bay owner)";
    add(
        g,
        CLIENT_OWNER_EMAIL,
        "/dashboard".to_owned(),
        "What waits for the client: the return to approve, the KPI pack, the sections",
    );
    if let Some(id) = first(
        db,
        "select id from accounting_returns where status = 'client_approval' order by id",
    )
    .await?
    {
        add(
            g,
            CLIENT_OWNER_EMAIL,
            format!("/accounting/portal/returns/{id}"),
            "The return to approve for filing (e-signature) or send back",
        );
    }
    if let Some(id) = first(
        db,
        "select id from accounting_kpi_packs where status = 'published' order by id",
    )
    .await?
    {
        add(
            g,
            CLIENT_OWNER_EMAIL,
            format!("/accounting/portal/kpi-packs/{id}"),
            "The published KPI pack with the manager's commentary",
        );
    }
    if let Some(id) = engagement(db, "Marina Bay Trading LLC").await? {
        add(
            g,
            CLIENT_OWNER_EMAIL,
            format!("/accounting/portal/engagements/{id}/letter"),
            "The engagement letter the owner signed",
        );
    }
    if let Some(party) = client(db, "Marina Bay Trading LLC").await? {
        for (section, what) in [
            ("engagements", "The company's engagements and their status"),
            ("returns", "The company's returns"),
            ("kpi_packs", "The company's KPI packs"),
        ] {
            add(
                g,
                CLIENT_OWNER_EMAIL,
                format!("/portal/{party}/{section}"),
                what,
            );
        }
        add(
            g,
            CLIENT_FINANCE_EMAIL,
            format!("/portal/{party}/returns"),
            "The finance contact reads the returns (no approval)",
        );
    }
    if let Some(party) = client(db, "Desert Rose Interiors LLC").await? {
        add(
            g,
            ROSE_OWNER_EMAIL,
            format!("/portal/{party}/returns"),
            "Desert Rose's owner: the filed and corrected returns",
        );
    }
    Ok(screens)
}

/// The map as `accounting:demo` prints it.
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
