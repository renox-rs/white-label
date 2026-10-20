//! BL-CON-17 (#296) and BL-CON-18 (#297): the 02 values for the shared
//! money, procurement, labour and compliance mechanisms run by the real
//! modules of the host in a fresh Construction deployment (`wl:install` and
//! the seeds), not as constants.

use host::construction_seed;
use host::roles::{self, ACCOUNTANT, ADMIN, HR, PROJECT_MANAGER, RECEIVABLES};
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_documents::DocumentKind;
use wl_invoicing::Invoice;
use wl_kernel::approvals::{self};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, InvoiceStatus, VatCategory, invoicer};
use wl_kernel::{ClientConfig, PartyId, PartyKind, Subject};
use wl_parties::{CompanyDetails, NewParty, create};
use wl_sales::LeadSource;
use wl_ticketing::Category;

/// A contracting client with every module the settings reach.
const CLIENT: &str = r#"
[client]
name = "Gulf Build Contracting LLC"
app = "construction"

[modules]
enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "payments", "sales", "hr", "attendance", "inventory"]

[[legal_entities]]
code = "GBC"
name_en = "Gulf Build Contracting LLC"
default = true
"#;

async fn booted() -> TestApp {
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(host::app_for(config).unwrap()).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

async fn person(app: &TestApp, name: &str, role: &str) -> User {
    let mut user = User::register(
        app.db(),
        name,
        &format!("{}@example.com", name.to_lowercase()),
        "secret123",
    )
    .await
    .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    roles::define(app.db()).await.unwrap();
    user.assign_role(app.db(), role).await.unwrap();
    user
}

async fn client_party(app: &TestApp) -> PartyId {
    create(
        app.state(),
        NewParty {
            kind: PartyKind::Company,
            legal_name_en: "Marina Developments LLC".into(),
            company: Some(CompanyDetails::default()),
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .party_id()
}

/// An issued invoice of `fils` to the client of a project, due last month.
async fn issued(app: &TestApp, by: &User, fils: i64) -> Invoice {
    let party = client_party(app).await;
    let due: NaiveDate = today() - Days::new(30);
    let draft = InvoiceDraft::new(party, Subject::new("construction.project", 1))
        .line(DraftLine::new("IPA 1", fils, VatCategory::Exempt))
        .due_date(due);
    let id = invoicer(app.state())
        .draft(app.db(), draft)
        .await
        .unwrap()
        .expect("invoicing is on");
    app.acting_as(by);
    app.htmx()
        .post(&format!("/invoices/{}/issue", id.0), &[])
        .await
        .assert_redirect(&format!("/invoices/{}", id.0));
    Invoice::find_or_404(app.db(), id.0).await.unwrap()
}

/// Item 1 (#296): a write-off of AED 1 creates an approval request: the limit
/// is AED 0, the MD (the company admin) approves, the requester never does,
/// and no dispute is offered to the client in a portal (item 2).
#[renox::test]
async fn a_write_off_of_aed_1_waits_for_the_md() {
    let app = booted().await;
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let md = person(&app, "Mona", ADMIN).await;
    let manager = person(&app, "Pam", PROJECT_MANAGER).await;
    let invoice = issued(&app, &clerk, 100).await;

    app.acting_as(&clerk);
    app.htmx()
        .post(
            &format!("/invoices/{}/write-off", invoice.id),
            &[("reason_code", "insolvent"), ("reason_text", "")],
        )
        .await
        .assert_ok();
    let pending = approvals::pending_for(app.db(), &invoice.subject())
        .await
        .unwrap();
    assert_eq!(pending.len(), 1, "AED 1 is above the limit of AED 0");
    let waiting = |user: &User| {
        let (user, db) = (user.clone(), app.db().clone());
        async move {
            approvals::awaiting(&db, &user)
                .await
                .unwrap()
                .iter()
                .any(|r| r.action == "invoicing.write_off")
        }
    };
    assert!(waiting(&md).await);
    assert!(!waiting(&clerk).await);
    assert!(!waiting(&manager).await, "the MD approves, not the manager");
    let own = approvals::approve(app.state(), pending[0].id, &clerk, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");
    approvals::approve(app.state(), pending[0].id, &md, "agreed")
        .await
        .unwrap();
    let after = Invoice::find_or_404(app.db(), invoice.id).await.unwrap();
    assert_eq!(after.status, InvoiceStatus::WrittenOff);
}

/// The seed stores the ticket categories (`absence` among them, which the
/// absence case needs), none chargeable, turns off the lead sources 02 does
/// not use, creates the five checklists and the SOP library, and a second
/// run changes nothing.
#[renox::test]
async fn the_seed_is_idempotent_and_stores_the_categories_sources_checklists_and_sops() {
    let app = booted().await;
    for key in ["absence", "snag", "defect"] {
        let category = Category::by_key(app.db(), key)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("the category {key}"));
        assert!(!category.chargeable, "{key} is never chargeable");
    }
    for source in LeadSource::ordered(app.db()).await.unwrap() {
        let used = construction::seed::LEAD_SOURCES.contains(&source.code.as_str());
        assert_eq!(source.active, used || !source.seeded, "{}", source.code);
    }
    let again = construction_seed::install(app.state()).await.unwrap();
    assert_eq!(again.total(), 0, "a re-run changes nothing: {again:?}");
}

/// The labour values are in force in the modules: the late-entry window, the
/// overtime caps, the permit kinds, the HR and HSE roles, the line manager
/// and the absconding days, and the sales approval of a cost over budget.
#[renox::test]
async fn the_labour_and_sales_modules_run_with_the_02_values() {
    let app = booted().await;
    let attendance = wl_attendance::config::config_of(app.state());
    assert_eq!(attendance.late_entry_days, 1);
    assert_eq!(attendance.overtime_day_minutes, 2 * 60);
    assert_eq!(attendance.overtime_window_minutes, 144 * 60);
    assert_eq!(attendance.overtime_window_days, 21);
    assert_eq!(attendance.hr_role, HR);
    assert_eq!(attendance.hse_role, roles::HSE_OFFICER);
    assert!(attendance.permit_check);
    assert_eq!(attendance.permit_kinds, ["work_permit", "residence_visa"]);
    let hr = wl_hr::config::config_of(app.state());
    assert_eq!(hr.manager_role, PROJECT_MANAGER);
    assert_eq!(hr.absence_days, 7);
    assert!(wl_sales::config::config_of(app.state()).over_budget_approval);
}

/// The document kinds: work permit and residence visa flag attendance and
/// remind HR, the client's registration and the bond document only remind,
/// and the three registration kinds have the responsible roles of 02.
#[renox::test]
async fn the_document_kinds_flag_remind_and_register_as_in_02() {
    let app = booted().await;
    let kind = |key: &'static str| {
        let db = app.db().clone();
        async move {
            DocumentKind::by_key(&db, key)
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("the kind {key}"))
        }
    };
    for key in ["work_permit", "residence_visa"] {
        let kind = kind(key).await;
        assert_eq!(kind.responsible_role.as_deref(), Some(HR));
        assert_eq!(kind.blocks(app.db()).await.unwrap().len(), 1);
    }
    for key in [
        "client_contractor_registration",
        "performance_bond_document",
    ] {
        assert!(kind(key).await.blocks(app.db()).await.unwrap().is_empty());
    }
    assert_eq!(
        kind("iso_certificate").await.responsible_role.as_deref(),
        Some(roles::HSE_OFFICER)
    );
    for key in ["trademark_registration", "other_certification"] {
        assert_eq!(kind(key).await.responsible_role.as_deref(), Some(ADMIN));
        assert!(kind(key).await.blocks(app.db()).await.unwrap().is_empty());
    }
    assert!(ACCOUNTANT != ADMIN);
}
