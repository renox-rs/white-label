//! BL-FNB-16 (#380), 06 §5, §7.2, `FIN.BUD.01`, `FIN.RUL.24`, `FIN.ACL.21`:
//! the budget mechanisms run with the values of a fresh 06 deployment
//! (`fnb_booted`): the accountant prepares the monthly budget and the
//! operations director approves it, never its preparer; an actual reaching
//! 100% of its month's line warns the accountants once; an owner drawing is
//! approved by the operations director and never by the person who recorded
//! it, and it is left out of the operating cost of the budget report.

mod support;

use host::roles::{ACCOUNTANT, ADMIN, OPERATIONS_DIRECTOR};
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

/// A draft budget of ZCD for this year with `amount` fils for the category
/// `code` in the current month, prepared by `by`.
async fn draft_budget(app: &TestApp, by: &User, code: &str, amount: i64) -> CompanyBudget {
    let entity = entity(app, "ZCD").await;
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

/// Submits `budget` as `by` and has `approver` approve it.
async fn approved_budget(
    app: &TestApp,
    by: &User,
    approver: &User,
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
    approvals::approve(app.state(), request.id, approver, "")
        .await
        .unwrap()
        .expect("the approval runs");
    CompanyBudget::find_or_404(app.db(), budget.id)
        .await
        .unwrap()
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

/// AC (Cash-flow 2): the budget the accountant prepares is approved by the
/// operations director, and the accountant cannot approve it.
#[renox::test]
async fn the_accountants_budget_waits_for_the_operations_director() {
    let app = fnb_booted().await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let director = person(&app, "Omar", OPERATIONS_DIRECTOR).await;

    let budget = draft_budget(&app, &accountant, "rent_utilities", 500_000).await;
    app.acting_as(&accountant);
    app.htmx()
        .post(&format!("/budgets/{}/submit", budget.id), &[])
        .await
        .assert_redirect(&format!("/budgets/{}", budget.id));
    assert_eq!(
        CompanyBudget::find_or_404(app.db(), budget.id)
            .await
            .unwrap()
            .status,
        BudgetStatus::Submitted
    );
    let request = pending(&app, &budget).await;
    assert_eq!(request.requester_id, accountant.id);
    let approvers = approvals::notify::step_approvers(app.db(), &request)
        .await
        .unwrap();
    assert!(approvers.iter().any(|u| u.id == director.id));
    assert!(!approvers.iter().any(|u| u.id == accountant.id));
    let own = approvals::approve(app.state(), request.id, &accountant, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");

    approvals::approve(app.state(), request.id, &director, "")
        .await
        .unwrap();
    let approved = CompanyBudget::find_or_404(app.db(), budget.id)
        .await
        .unwrap();
    assert_eq!(approved.status, BudgetStatus::Approved);
    assert_eq!(approved.approved_by, Some(director.id));
}

/// `FIN.RUL.24`: an approved bill that takes the month's actual of *Rent and
/// utilities* to 100% of its line warns the accountants, once; the operations
/// director is not told; a later bill does not warn again.
#[renox::test]
async fn a_month_reaching_its_line_warns_the_accountants_once() {
    let app = fnb_booted().await;
    let amal = person(&app, "Amal", ACCOUNTANT).await;
    let bilal = person(&app, "Bilal", ACCOUNTANT).await;
    let director = person(&app, "Omar", OPERATIONS_DIRECTOR).await;
    let vendor = company(&app, "Dubai Utilities LLC").await;
    let code = wl_parties::Party::find_or_404(app.db(), vendor.0)
        .await
        .unwrap()
        .code;
    approved_budget(&app, &amal, &director, "rent_utilities", 500_000).await;

    let first = bill(&app, &amal, &code, "UT-1", "4000", "utilities").await;
    approve_bill(&app, &bilal, &first).await;
    assert_eq!(count_messages(&app, VARIANCE).await, 0);

    let second = bill(&app, &amal, &code, "UT-2", "1000", "utilities").await;
    approve_bill(&app, &bilal, &second).await;
    assert_eq!(messages_to(&app, VARIANCE, &amal).await.len(), 1);
    assert_eq!(messages_to(&app, VARIANCE, &bilal).await.len(), 1);
    assert!(messages_to(&app, VARIANCE, &director).await.is_empty());

    let third = bill(&app, &amal, &code, "UT-3", "300", "utilities").await;
    approve_bill(&app, &bilal, &third).await;
    assert_eq!(messages_to(&app, VARIANCE, &amal).await.len(), 1);
}

/// `FIN.ACL.21`: a bill line coded *owner drawings* is a drawing: the
/// accountant who records it cannot approve it, the operations director
/// does; it is left out of the operating cost of the budget report.
#[renox::test]
async fn an_owner_drawing_is_approved_by_the_operations_director_and_is_not_operating_cost() {
    let app = fnb_booted().await;
    let amal = person(&app, "Amal", ACCOUNTANT).await;
    let director = person(&app, "Omar", OPERATIONS_DIRECTOR).await;
    let admin = person(&app, "Alia", ADMIN).await;
    let vendor = company(&app, "Villa Decor LLC").await;
    let code = wl_parties::Party::find_or_404(app.db(), vendor.0)
        .await
        .unwrap()
        .code;
    approved_budget(&app, &amal, &director, "rent_utilities", 500_000).await;

    let drawing = bill(&app, &amal, &code, "VD-1", "2000", "owner_drawings").await;
    let ordinary = bill(&app, &amal, &code, "VD-2", "1000", "other").await;
    approve_bill(&app, &amal, &ordinary).await;

    let row = DrawingApproval::for_source(app.db(), &drawing.subject())
        .await
        .unwrap()
        .remove(0);
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

    approvals::approve(app.state(), request_id, &director, "")
        .await
        .unwrap()
        .expect("the approval runs");
    let approved = DrawingApproval::for_source(app.db(), &drawing.subject())
        .await
        .unwrap()
        .remove(0);
    assert_eq!(approved.status, DrawingStatus::Approved);
    assert_eq!(approved.approved_by, Some(director.id));
    approve_bill(&app, &amal, &drawing).await;

    let entity = entity(&app, "ZCD").await;
    let config = app
        .state()
        .provided::<wl_invoicing::InvoicingConfig>()
        .expect("the invoicing settings");
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
    assert_eq!(report.expense_total().1, 100_000);
}
