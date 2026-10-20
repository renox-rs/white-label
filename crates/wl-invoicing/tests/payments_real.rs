//! Owner drawings and bank statements with the real wl-payments module
//! (wl-payments #118, #119; wl-invoicing §5.14, §5.16): no stub payment source
//! and no scripted recorder. An accountant saves a drawing on `POST /payments`,
//! wl-invoicing's drawings approval asks the owner, the decision makes
//! wl-payments record the payment, and the drawings line of budget vs actual and
//! the cash-flow forecast read it from wl-payments.

mod support;

use renox::auth::{Permissions, permissions};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_invoicing::budget_report::{self, ReportQuery};
use wl_invoicing::cash_forecast::{self, Warning};
use wl_invoicing::cash_forecast_load;
use wl_invoicing::{
    BudgetCategory, DrawingApproval, DrawingStatus, WlInvoicing, permissions as perm,
};
use wl_kernel::approvals;
use wl_kernel::drawings::DRAWING_REQUEST_SUBJECT_TYPE;
use wl_kernel::payments::{Direction, payment_recorder};
use wl_kernel::{ClientConfig, LegalEntity, Subject};
use wl_payments::drawings::{DrawingStatus as PaymentDrawingStatus, PaymentDrawing};
use wl_payments::{Payment, WlPayments, permissions as pay};

use support::{QUIET_SANDS, Recording, TestDirectory};

const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

const CLIENT: &str = r#"
[client]
name = "Palm Grove Properties LLC"
app = "property"

[modules]
enabled = ["parties", "invoicing", "payments"]

[[legal_entities]]
code = "PGP"
name_en = "Palm Grove Properties LLC"
trn = "100000000000003"
default = true
"#;

/// The owner party's code in the test directory.
const OWNER_CODE: &str = "P-00002";

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

struct Real {
    app: TestApp,
    _views: tempfile::TempDir,
}

/// wl-invoicing and wl-payments in one app, the owner party set on the
/// seeded drawings category.
async fn boot() -> Real {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let directory: std::sync::Arc<dyn wl_kernel::parties::PartyDirectory> =
        std::sync::Arc::new(TestDirectory::default());
    let notifier: std::sync::Arc<dyn wl_kernel::notify::Notifier> =
        std::sync::Arc::new(Recording::default());
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(WlInvoicing::new())
        .module(WlPayments::new())
        .provide(directory)
        .provide(notifier);
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    wl_invoicing::budget_category::install(app.db())
        .await
        .expect("the budget categories are seeded");
    let mut category = BudgetCategory::by_code(app.db(), "owner_drawings")
        .await
        .unwrap()
        .expect("the drawings category");
    category.owner_party_id = Some(QUIET_SANDS.0);
    category.save(app.db()).await.unwrap();
    Real { app, _views: views }
}

async fn user(t: &Real, name: &str, grants: &[&str]) -> User {
    let email = format!("{}@example.com", name.to_lowercase());
    let user = User::register(t.app.db(), name, &email, "password123")
        .await
        .unwrap();
    let role = format!("role_{}", name.to_lowercase());
    permissions::define_role(t.app.db(), &role, grants)
        .await
        .unwrap();
    user.assign_role(t.app.db(), &role).await.unwrap();
    user
}

/// `POST /payments` as `by`: a drawing (`out`) or a repayment (`in`).
async fn post_drawing(
    t: &Real,
    by: &User,
    direction: &str,
    amount: &str,
    date: &str,
) -> renox::testing::TestResponse {
    let entity = LegalEntity::query()
        .where_(LegalEntity::CODE.eq("PGP"))
        .first(t.app.db())
        .await
        .unwrap()
        .unwrap()
        .id
        .to_string();
    t.app.acting_as(by);
    t.app
        .htmx()
        .post(
            "/payments",
            &[
                ("legal_entity_id", entity.as_str()),
                ("party_code", OWNER_CODE),
                ("method", "transfer"),
                ("amount", amount),
                ("reference", "TRX-DRAW"),
                ("payment_date", date),
                ("bank_account", "ENBD-1"),
                ("purpose", "owner_drawings"),
                ("drawing_category", "owner_drawings"),
                ("drawing_direction", direction),
            ],
        )
        .await
}

/// The owner approves the newest drawing request through wl-invoicing's real
/// approval.
async fn approve_latest(t: &Real, owner: &User) -> PaymentDrawing {
    let drawing = PaymentDrawing::query()
        .order_by_desc(PaymentDrawing::ID)
        .first(t.app.db())
        .await
        .unwrap()
        .expect("a drawing request");
    let approval = DrawingApproval::for_source(
        t.app.db(),
        &Subject::new(DRAWING_REQUEST_SUBJECT_TYPE, drawing.id),
    )
    .await
    .unwrap()
    .remove(0);
    assert_eq!(approval.status, DrawingStatus::Requested);
    approvals::approve(
        t.app.state(),
        approval.approval_request_id.expect("an approval request"),
        owner,
        "",
    )
    .await
    .unwrap()
    .expect("the approval runs");
    PaymentDrawing::find_or_404(t.app.db(), drawing.id)
        .await
        .unwrap()
}

/// A drawing of AED 15,000 and a repayment of AED 5,000 in March are the
/// owner drawings line of March: AED 10,000, read by wl-invoicing from the
/// payments wl-payments really recorded after the owner's approval. The
/// requester cannot approve it.
#[renox::test]
async fn the_drawings_wl_payments_recorded_are_on_the_drawings_line_of_march() {
    let t = boot().await;
    let a = user(&t, "Amal", &[pay::VIEW, pay::RECORD, perm::VIEW]).await;
    let b = user(
        &t,
        "Bilal",
        &[perm::SUPPLIER_BILLS_VIEW, perm::DRAWINGS_APPROVE],
    )
    .await;
    post_drawing(&t, &a, "out", "15000.00", "2027-03-10")
        .await
        .assert_status(303);
    assert_eq!(Payment::query().count(t.app.db()).await.unwrap(), 0);

    // The requester is refused by the approval itself.
    let request = DrawingApproval::query()
        .first(t.app.db())
        .await
        .unwrap()
        .unwrap();
    assert!(
        approvals::approve(t.app.state(), request.approval_request_id.unwrap(), &a, "")
            .await
            .is_err()
    );
    assert_eq!(Payment::query().count(t.app.db()).await.unwrap(), 0);

    let recorded = approve_latest(&t, &b).await;
    assert_eq!(recorded.status, PaymentDrawingStatus::Recorded);
    post_drawing(&t, &a, "in", "5000.00", "2027-03-20")
        .await
        .assert_status(303);

    let entity = LegalEntity::query()
        .where_(LegalEntity::CODE.eq("PGP"))
        .first(t.app.db())
        .await
        .unwrap()
        .unwrap();
    let config = t
        .app
        .state()
        .provided::<wl_invoicing::InvoicingConfig>()
        .unwrap();
    let march = budget_report::build(
        t.app.db(),
        &config,
        &ReportQuery {
            entities: vec![entity.id],
            month: day(2027, 3, 1),
            ytd: false,
            drawings_lines: true,
            uncategorised: "Uncategorised".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(march.drawings_total().1, 1_000_000);
    assert_eq!(march.expense_total().1, 0);
    assert_eq!(march.drawings_items.len(), 2);
    assert_eq!(march.drawings_items[0].amount, 1_500_000);
    assert_eq!(march.drawings_items[1].amount, -500_000);
    let payment = Payment::find_or_404(t.app.db(), recorded.payment_id.unwrap())
        .await
        .unwrap();
    assert_eq!(march.drawings_items[0].source, payment.subject());
}

/// A completed statement import gives the balance of the account; a drawing
/// dated after it, in the future, is in the cash-flow forecast: the opening of
/// the account is the balance less the drawing.
#[renox::test]
async fn a_future_dated_drawing_is_in_the_cash_flow_forecast() {
    let t = boot().await;
    let a = user(
        &t,
        "Amal",
        &[pay::VIEW, pay::RECORD, pay::BANK_RECONCILE, perm::VIEW],
    )
    .await;
    let b = user(
        &t,
        "Bilal",
        &[perm::SUPPLIER_BILLS_VIEW, perm::DRAWINGS_APPROVE],
    )
    .await;
    let entity = LegalEntity::query()
        .where_(LegalEntity::CODE.eq("PGP"))
        .first(t.app.db())
        .await
        .unwrap()
        .unwrap();

    // The statement: AED 100,000.00 as of 2027-01-10.
    t.app.acting_as(&a);
    let id = entity.id.to_string();
    t.app
        .htmx()
        .post(
            "/bank-statements",
            &[
                ("legal_entity_id", id.as_str()),
                ("bank_account", "ENBD-1"),
                ("file_name", "jan.mt940"),
                ("format", "mt940"),
                ("period_start", "2027-01-01"),
                ("period_end", "2027-01-10"),
                ("closing_balance", ""),
                ("content", ":25:ENBD-1\n:62F:C270110AED100000,00\n"),
            ],
        )
        .await
        .assert_status(303);
    let import = wl_payments::bank_statement::BankStatementImport::query()
        .first(t.app.db())
        .await
        .unwrap()
        .unwrap();
    for action in ["parse", "complete"] {
        t.app
            .htmx()
            .post(&format!("/bank-statements/{}/{action}", import.id), &[])
            .await
            .assert_ok();
    }

    // The drawing, dated ahead of today, once approved.
    let ahead_of_today = renox::db::now().date_naive() + renox::chrono::Duration::days(30);
    let ahead = ahead_of_today
        .max(day(2027, 2, 1))
        .format("%Y-%m-%d")
        .to_string();
    post_drawing(&t, &a, "out", "15000.00", &ahead)
        .await
        .assert_status(303);
    let recorded = approve_latest(&t, &b).await;
    assert_eq!(recorded.status, PaymentDrawingStatus::Recorded);

    // The recorder (wl-payments) returns it as a payment after the balance day.
    let after = payment_recorder(t.app.state())
        .payments_after(t.app.db(), entity.id, "ENBD-1", day(2027, 1, 10))
        .await
        .unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].payment.direction, Direction::Out);
    assert_eq!(after[0].payment.amount, 1_500_000);

    let config = t
        .app
        .state()
        .provided::<wl_invoicing::InvoicingConfig>()
        .unwrap();
    let inputs = cash_forecast_load::load(
        t.app.state(),
        &config,
        std::slice::from_ref(&entity),
        wl_kernel::documents::today(),
    )
    .await
    .unwrap();
    let forecast = cash_forecast::assemble(&inputs);
    assert_eq!(forecast.openings.len(), 1);
    assert_eq!(forecast.openings[0].account, "ENBD-1");
    assert_eq!(
        forecast.openings[0].amount, 8_500_000,
        "AED 100,000.00 less the drawing of AED 15,000.00"
    );
    assert!(
        !forecast
            .warnings
            .iter()
            .any(|w| matches!(w, Warning::NoBalance { .. }))
    );
}
