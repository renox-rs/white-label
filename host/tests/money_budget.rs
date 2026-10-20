//! BL-PRP-26 (#276) items 26 and 27 with the real modules of the host: the
//! budget categories are owned by the accountant; a monthly budget prepared
//! by the accountant needs the company admin and not its preparer; an actual
//! reaching 100% of its month's line warns the category's owner role and the
//! accountant once (`FIN.RUL.24`); an expense with category *owner drawings*
//! is left out of the operating cost, and the user who records it cannot
//! approve it (`FIN.ACL.21`), the company admin does.

mod support;

use host::roles::{ACCOUNTANT, ADMIN, FINANCE_MANAGER, MARKETING_MANAGER};
use renox::chrono::Datelike;
use renox::prelude::*;
use renox::testing::TestApp;
use support::*;
use wl_invoicing::budget_lines::{self, LineInput, NewBudget};
use wl_invoicing::budget_report::{self, ReportQuery};
use wl_invoicing::{
    BudgetCategory, BudgetStatus, CompanyBudget, DrawingApproval, DrawingStatus, StartChoice,
    SupplierBill, SupplierBillStatus,
};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::documents::today;

const VARIANCE: &str = "invoicing.budget_variance";

/// A draft budget of PGP for this year with `amount` fils for the category
/// `code` in the current month, prepared by `by`.
async fn draft_budget(app: &TestApp, by: &User, code: &str, amount: i64) -> CompanyBudget {
    let entity = entity(app, "PGP").await;
    let budget = budget_lines::create(
        app.state(),
        NewBudget {
            legal_entity_id: entity.id,
            fiscal_year: i64::from(today().year()),
            name: String::new(),
            start: StartChoice::Empty,
            revision_reason: None,
        },
        Some(by.id),
    )
    .await
    .expect("the draft is made");
    let category = BudgetCategory::by_code(app.db(), code)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the category {code}"));
    let month = budget.months()[today().month0() as usize];
    budget_lines::set_lines(
        app.db(),
        budget.id,
        &[LineInput {
            category_id: category.id,
            month,
            amount,
            note: None,
        }],
    )
    .await
    .expect("the line is set");
    budget
}

async fn pending(app: &TestApp, budget: &CompanyBudget) -> ApprovalRequest {
    let mut pending = approvals::pending_for(app.db(), &budget.subject())
        .await
        .unwrap();
    assert_eq!(pending.len(), 1, "one pending request");
    pending.remove(0)
}

/// Submits `budget` as `by` and has the company admin approve it.
async fn approved_budget(
    app: &TestApp,
    by: &User,
    admin: &User,
    code: &str,
    amount: i64,
) -> CompanyBudget {
    let budget = draft_budget(app, by, code, amount).await;
    app.acting_as(by);
    app.htmx()
        .post(&format!("/budgets/{}/submit", budget.id), &[])
        .await
        .assert_redirect(&format!("/budgets/{}", budget.id));
    let request = pending(app, &budget).await;
    approvals::approve(app.state(), request.id, admin, "")
        .await
        .unwrap()
        .expect("the approval runs");
    CompanyBudget::find_or_404(app.db(), budget.id)
        .await
        .unwrap()
}

/// Item 26: the categories are owned by the accountant (the marketing line
/// keeps its marketing manager); the budget the accountant prepares waits for
/// the company admin, and the accountant cannot approve it.
#[renox::test]
async fn the_accountant_owns_the_categories_and_the_company_admin_approves_the_budget() {
    let app = booted().await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let admin = person(&app, "Alia", ADMIN).await;

    for code in [
        "rent_utilities",
        "professional_fees",
        "bank_charges",
        "sales_services",
    ] {
        let category = BudgetCategory::by_code(app.db(), code)
            .await
            .unwrap()
            .expect(code);
        assert_eq!(category.owner_role.as_deref(), Some(ACCOUNTANT), "{code}");
    }
    let marketing = BudgetCategory::by_code(app.db(), "marketing")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(marketing.owner_role.as_deref(), Some(MARKETING_MANAGER));

    let budget = draft_budget(&app, &accountant, "rent_utilities", 500_000).await;
    app.acting_as(&accountant);
    app.htmx()
        .post(&format!("/budgets/{}/submit", budget.id), &[])
        .await
        .assert_redirect(&format!("/budgets/{}", budget.id));
    let submitted = CompanyBudget::find_or_404(app.db(), budget.id)
        .await
        .unwrap();
    assert_eq!(submitted.status, BudgetStatus::Submitted);
    let request = pending(&app, &budget).await;
    assert_eq!(request.requester_id, accountant.id);
    let approvers = approvals::notify::step_approvers(app.db(), &request)
        .await
        .unwrap();
    assert!(approvers.iter().any(|u| u.id == admin.id));
    assert!(!approvers.iter().any(|u| u.id == accountant.id));
    let own = approvals::approve(app.state(), request.id, &accountant, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");

    approvals::approve(app.state(), request.id, &admin, "")
        .await
        .unwrap();
    let approved = CompanyBudget::find_or_404(app.db(), budget.id)
        .await
        .unwrap();
    assert_eq!(approved.status, BudgetStatus::Approved);
    assert_eq!(approved.approved_by, Some(admin.id));
}

/// Item 26, `FIN.RUL.24`: an approved bill that takes the month's actual of
/// *Rent and utilities* to 100% of its line warns the category's owner role
/// (the accountant) and the accountant, once; the finance manager is not the
/// 01 variance role and is not told; a later bill does not warn again.
#[renox::test]
async fn a_month_reaching_its_line_warns_the_accountants_once() {
    let app = booted().await;
    let amal = person(&app, "Amal", ACCOUNTANT).await;
    let bilal = person(&app, "Bilal", ACCOUNTANT).await;
    let finance = person(&app, "Fiona", FINANCE_MANAGER).await;
    let admin = person(&app, "Alia", ADMIN).await;
    let vendor = company(&app, "Dubai Utilities LLC").await;
    let code = wl_parties::Party::find_or_404(app.db(), vendor.0)
        .await
        .unwrap()
        .code;
    // The line of this month is AED 5,000.
    approved_budget(&app, &amal, &admin, "rent_utilities", 500_000).await;

    // A bill of AED 4,000 is under the line.
    let first = bill(&app, &amal, &code, "UT-1", "4000", "utilities").await;
    approve_bill(&app, &bilal, &first).await;
    assert_eq!(count_messages(&app, VARIANCE).await, 0);

    // AED 1,000 more makes AED 5,000 net, 100% of the line.
    let second = bill(&app, &amal, &code, "UT-2", "1000", "utilities").await;
    approve_bill(&app, &bilal, &second).await;
    assert_eq!(messages_to(&app, VARIANCE, &amal).await.len(), 1);
    assert_eq!(messages_to(&app, VARIANCE, &bilal).await.len(), 1);
    assert!(messages_to(&app, VARIANCE, &finance).await.is_empty());

    // Once: a third bill does not warn again.
    let third = bill(&app, &amal, &code, "UT-3", "300", "utilities").await;
    approve_bill(&app, &bilal, &third).await;
    assert_eq!(messages_to(&app, VARIANCE, &amal).await.len(), 1);
}

/// A supplier bill of one line of `price` dirhams coded `cost_code`, created
/// and submitted by `by`.
async fn bill(
    app: &TestApp,
    by: &User,
    supplier: &str,
    number: &str,
    price: &str,
    cost_code: &str,
) -> SupplierBill {
    app.acting_as(by);
    app.htmx()
        .post(
            "/supplier-bills",
            &[
                ("supplier", supplier),
                ("supplier_invoice_number", number),
                ("bill_date", day(0).to_string().as_str()),
                ("currency", "AED"),
                ("line_1_description", "Charge"),
                ("line_1_quantity", "1"),
                ("line_1_unit_price", price),
                ("line_1_vat", "standard"),
                ("line_1_cost_code", cost_code),
            ],
        )
        .await
        .assert_status(303);
    let bill = SupplierBill::query()
        .order_by_desc(SupplierBill::ID)
        .first(app.db())
        .await
        .unwrap()
        .expect("the bill");
    app.htmx()
        .post(&format!("/supplier-bills/{}/submit", bill.id), &[])
        .await
        .assert_status(303);
    SupplierBill::find_or_404(app.db(), bill.id).await.unwrap()
}

async fn approve_bill(app: &TestApp, by: &User, bill: &SupplierBill) {
    app.acting_as(by);
    app.htmx()
        .post(
            &format!("/supplier-bills/{}/transitions/approve", bill.id),
            &[("reason", "")],
        )
        .await
        .assert_status(303);
    assert_eq!(
        SupplierBill::find_or_404(app.db(), bill.id)
            .await
            .unwrap()
            .status,
        SupplierBillStatus::Approved
    );
}

/// Item 27: a bill line coded *owner drawings* is a drawing: the accountant
/// who records it cannot approve it, the company admin does; it is left out
/// of the operating cost of the budget report and shown on its own line.
#[renox::test]
async fn an_owner_drawing_is_approved_by_the_company_admin_and_is_not_operating_cost() {
    let app = booted().await;
    let amal = person(&app, "Amal", ACCOUNTANT).await;
    let admin = person(&app, "Alia", ADMIN).await;
    let vendor = company(&app, "Villa Decor LLC").await;
    let code = wl_parties::Party::find_or_404(app.db(), vendor.0)
        .await
        .unwrap()
        .code;
    approved_budget(&app, &amal, &admin, "rent_utilities", 500_000).await;

    let drawing = bill(&app, &amal, &code, "VD-1", "2000", "owner_drawings").await;
    let ordinary = bill(&app, &amal, &code, "VD-2", "1000", "other").await;
    approve_bill(&app, &amal, &ordinary).await;

    // The drawing waits for the company admin; the requester cannot approve.
    let mut rows = DrawingApproval::for_source(app.db(), &drawing.subject())
        .await
        .unwrap();
    let row = rows.remove(0);
    assert_eq!(row.status, DrawingStatus::Requested);
    assert_eq!(row.requested_by, Some(amal.id));
    let request_id = row.approval_request_id.expect("an approval request");
    app.acting_as(&amal);
    app.htmx()
        .post(
            &format!("/supplier-bills/{}/transitions/approve", drawing.id),
            &[("reason", "")],
        )
        .await
        .assert_status(422);
    let own = approvals::approve(app.state(), request_id, &amal, "").await;
    assert!(own.is_err(), "the requester cannot approve their drawing");

    approvals::approve(app.state(), request_id, &admin, "")
        .await
        .unwrap()
        .expect("the approval runs");
    let approved = DrawingApproval::for_source(app.db(), &drawing.subject())
        .await
        .unwrap()
        .remove(0);
    assert_eq!(approved.status, DrawingStatus::Approved);
    assert_eq!(approved.approved_by, Some(admin.id));
    approve_bill(&app, &amal, &drawing).await;

    // Operating cost has the ordinary bill only; the drawing is on its own.
    let entity = entity(&app, "PGP").await;
    let config = wl_invoicing::WlInvoicing::new().config().clone();
    let report = budget_report::build(
        app.db(),
        &config,
        &ReportQuery {
            entities: vec![entity.id],
            month: day(0),
            ytd: false,
            drawings_lines: true,
            uncategorised: "Uncategorised".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(report.drawings_total().1, 210_000);
    // Net of VAT: AED 1,000. The drawing's VAT is not recovered, so its
    // AED 2,100 is gross.
    assert_eq!(report.expense_total().1, 100_000);
}
