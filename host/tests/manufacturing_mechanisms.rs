//! BL-MFG-14 (#395): the mechanisms that the 07 values drive, run for real
//! in the manufacturing deployment with the shared modules (the modules
//! test them with their own defaults; here the values are the 07 ones):
//! *Credit and cheques* (an invoice 31 days overdue holds the next order, a
//! bounced cheque holds the account and its orders, a cheque is presented
//! twice at most), *Budget and drawings* (the warning at 100%, the drawing
//! without self-approval), *Supplier scorecard* (69 is reviewed, 70 is not)
//! and *Cancellation* (fee 0%, the plan the cancel routes run). The rest of
//! the issue runs elsewhere: the customer block holds the orders while the
//! work orders continue (`manufacturing_orders.rs`), the wastage of AED 2,100
//! and the return of AED 5,100 (`manufacturing_ncr.rs`), the transfer below
//! zero (`manufacturing_material_screens.rs`), the recall notice that gives
//! up (`manufacturing_recall.rs`), the button wording
//! (`manufacturing_dispute.rs`).

mod support;

use host::roles::{ACCOUNTANT, ADMIN, OWNER, SALES};
use renox::chrono::Datelike;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_inventory::SupplierScore;
use wl_inventory::supplier_review::{self, SupplierReview};
use wl_inventory::supplier_score::{Figures, ScoreBand, band_of, score_of};
use wl_invoicing::BillLineDraft;
use wl_invoicing::bill_submit;
use wl_invoicing::budget_lines::{self, LineInput, NewBudget};
use wl_invoicing::credit_notes::By;
use wl_invoicing::supplier_bill::{self, BillDraft};
use wl_invoicing::{
    BudgetCategory, CompanyBudget, DrawingApproval, DrawingStatus, StartChoice, SupplierBill,
};
use wl_kernel::PartyId;
use wl_kernel::approvals;
use wl_kernel::cheques::{ChequeStatus, RegisterCheque, cheques};
use wl_kernel::documents::today;
use wl_kernel::invoicing::VatCategory;
use wl_kernel::tickets::task_creator;
use wl_payments::Cheque;
use wl_sales::customer_account::{self, CustomerAccount, Terms};
use wl_sales::{SalesOrder, SalesOrderStatus};

use support::{
    company, count_messages, day, entity, issued_invoice, manufacturing_booted, messages_to, person,
};

/// A draft order on account of `customer` for 10 panels, entered through the
/// pages of wl-sales and submitted; the order as stored after the submit.
async fn submitted(app: &TestApp, sales: &User, customer: PartyId) -> renox::testing::TestResponse {
    app.acting_as(sales);
    app.htmx()
        .post(
            "/sales-orders",
            &[
                ("party_id", &customer.0.to_string()),
                ("channel", "phone"),
                ("payment_mode", "on_account"),
            ],
        )
        .await
        .assert_status(303);
    let order = latest_order(app).await;
    app.htmx()
        .post(
            &format!("/sales-orders/{}/lines", order.id),
            &[
                ("item_code", "PANEL"),
                ("quantity", "10"),
                ("unit_price", "10.00"),
                ("vat", "standard"),
            ],
        )
        .await
        .assert_status(303);
    app.htmx()
        .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
        .await
}

async fn latest_order(app: &TestApp) -> SalesOrder {
    SalesOrder::query()
        .order_by_desc(SalesOrder::ID)
        .first(app.db())
        .await
        .unwrap()
        .expect("an order")
}

async fn credit_for(app: &TestApp, customer: PartyId) {
    customer_account::save(app.state(), customer, Terms::default().limit(50_000_000))
        .await
        .unwrap();
}

async fn status_of(app: &TestApp, id: i64) -> SalesOrderStatus {
    SalesOrder::find_or_404(app.db(), id).await.unwrap().status
}

/// *Credit and cheques*: with grace 30 days an invoice 31 days past its due
/// date holds the customer's next order on account (`credit_hold`), while an
/// invoice 30 days past, or no invoice, lets the order through.
#[renox::test]
async fn an_invoice_31_days_overdue_holds_the_next_order_and_30_days_does_not() {
    let app = manufacturing_booted().await;
    let sales = person(&app, "Sara", SALES).await;
    let accountant = person(&app, "Alia", ADMIN).await;
    let late = company(&app, "Al Noor Trading LLC").await;
    let on_time = company(&app, "Bin Rashid Contracting LLC").await;
    for party in [late, on_time] {
        credit_for(&app, party).await;
    }
    issued_invoice(
        &app,
        &accountant,
        late,
        100_000,
        VatCategory::Standard,
        day(-31),
    )
    .await;
    issued_invoice(
        &app,
        &accountant,
        on_time,
        100_000,
        VatCategory::Standard,
        day(-30),
    )
    .await;

    submitted(&app, &sales, late).await.assert_status(303);
    assert_eq!(
        status_of(&app, latest_order(&app).await.id).await,
        SalesOrderStatus::CreditHold,
        "31 days overdue holds the order"
    );
    submitted(&app, &sales, on_time).await.assert_status(303);
    assert_ne!(
        status_of(&app, latest_order(&app).await.id).await,
        SalesOrderStatus::CreditHold,
        "30 days is within the grace"
    );
}

/// *Credit and cheques*: a bounced cheque holds the customer's account and
/// its order that is not dispatched; a new order on account is refused; the
/// cheque is presented a second time and when it bounces again the third
/// presentation is refused.
#[renox::test]
async fn a_bounced_cheque_holds_the_account_and_a_third_presentation_is_refused() {
    let app = manufacturing_booted().await;
    let sales = person(&app, "Sara", SALES).await;
    let accountant = person(&app, "Alia", ACCOUNTANT).await;
    let customer = company(&app, "Al Noor Trading LLC").await;
    credit_for(&app, customer).await;
    submitted(&app, &sales, customer).await.assert_status(303);
    let open_order = latest_order(&app).await;
    assert_eq!(
        status_of(&app, open_order.id).await,
        SalesOrderStatus::Confirmed
    );

    let epi = entity(&app, "EPI").await;
    let request = RegisterCheque::new(customer, epi.id, "ENBD", "3001", day(0), 2_000_000);
    let cheque = cheques(app.state())
        .register(app.state(), request, Some(accountant.id))
        .await
        .expect("the cheque is registered");
    let id = cheque.id.0;
    let act = |action: &'static str, form: Vec<(&'static str, String)>| {
        let (app, accountant) = (&app, &accountant);
        async move {
            app.acting_as(accountant);
            let pairs: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
            app.htmx()
                .post(&format!("/cheques/{id}/{action}"), &pairs)
                .await
        }
    };
    let bounce = || {
        act(
            "bounce",
            vec![
                ("reason", "insufficient_funds".into()),
                ("certificate", "CERT-1".into()),
                ("honoured", String::new()),
            ],
        )
    };
    let represent = || {
        act(
            "represent",
            vec![
                ("date", day(1).format("%Y-%m-%d").to_string()),
                ("funds_confirmed", "on".into()),
                ("note", String::new()),
            ],
        )
    };
    act("deposit", vec![]).await.assert_ok();
    bounce().await.assert_ok();

    let account = CustomerAccount::of_party(app.db(), customer.0)
        .await
        .unwrap()
        .expect("the account");
    assert!(account.is_on_hold(app.db()).await.unwrap(), "account held");
    assert_eq!(
        status_of(&app, open_order.id).await,
        SalesOrderStatus::CreditHold,
        "the order not dispatched is held"
    );
    let refused = submitted(&app, &sales, customer).await;
    refused.assert_status(422);
    assert!(refused.text().contains("on hold"), "{}", refused.text());
    assert_eq!(
        status_of(&app, latest_order(&app).await.id).await,
        SalesOrderStatus::Draft,
        "the new order stays a draft"
    );

    represent().await.assert_ok();
    let second = Cheque::find_or_404(app.db(), id).await.unwrap();
    assert_eq!(second.presentations, 2);
    bounce().await.assert_ok();
    represent().await.assert_status(422);
    let last = Cheque::find_or_404(app.db(), id).await.unwrap();
    assert_eq!(
        (last.status, last.presentations),
        (ChequeStatus::Bounced, 2)
    );
}

// ------------------------------------------------------ budget and drawings

const VARIANCE: &str = "invoicing.budget_variance";

/// A draft budget of EPI for this year with `amount` fils for the category
/// `code` in the current month, prepared by `by`.
async fn draft_budget(app: &TestApp, by: &User, code: &str, amount: i64) -> CompanyBudget {
    let entity = entity(app, "EPI").await;
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

/// Submits the budget as `by` and has `approver` approve it.
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
    let pending = approvals::pending_for(app.db(), &budget.subject())
        .await
        .unwrap();
    assert_eq!(pending.len(), 1, "one pending request");
    assert!(
        approvals::approve(app.state(), pending[0].id, by, "")
            .await
            .is_err(),
        "the preparer cannot approve their own budget"
    );
    approvals::approve(app.state(), pending[0].id, approver, "")
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
}

/// `FIN.RUL.24` with the 07 values: the bill that takes the month's actual of
/// *Rent and utilities* to 100% of its line (approved by the owner, never by
/// the accountant who prepared it) warns the accountants once; the owner is
/// not told.
#[renox::test]
async fn a_month_reaching_100_percent_of_its_budget_line_warns_the_accountants_once() {
    let app = manufacturing_booted().await;
    let amal = person(&app, "Amal", ACCOUNTANT).await;
    let bilal = person(&app, "Bilal", ACCOUNTANT).await;
    let owner = person(&app, "Omar", OWNER).await;
    let vendor = company(&app, "Dubai Utilities LLC").await;
    let code = wl_parties::Party::find_or_404(app.db(), vendor.0)
        .await
        .unwrap()
        .code;
    approved_budget(&app, &amal, &owner, "rent_utilities", 500_000).await;

    let first = bill(&app, &amal, &code, "UT-1", "4000", "utilities").await;
    approve_bill(&app, &bilal, &first).await;
    assert_eq!(count_messages(&app, VARIANCE).await, 0, "80% does not warn");

    let second = bill(&app, &amal, &code, "UT-2", "1000", "utilities").await;
    approve_bill(&app, &bilal, &second).await;
    assert_eq!(messages_to(&app, VARIANCE, &amal).await.len(), 1);
    assert_eq!(messages_to(&app, VARIANCE, &bilal).await.len(), 1);
    assert!(messages_to(&app, VARIANCE, &owner).await.is_empty());

    let third = bill(&app, &amal, &code, "UT-3", "300", "utilities").await;
    approve_bill(&app, &bilal, &third).await;
    assert_eq!(messages_to(&app, VARIANCE, &amal).await.len(), 1, "once");
}

/// `FIN.ACL.21` with the 07 values (`drawings_self_approve` off): a bill line
/// coded *owner drawings* is approved by the owner, never by the accountant
/// who recorded it; with a sole owner who enters it himself the drawing waits
/// for a second owner.
#[renox::test]
async fn an_owner_drawing_is_never_approved_by_the_person_who_recorded_it() {
    let app = manufacturing_booted().await;
    let amal = person(&app, "Amal", ACCOUNTANT).await;
    let omar = person(&app, "Omar", OWNER).await;
    let osman = person(&app, "Osman", OWNER).await;
    let vendor = company(&app, "Villa Decor LLC").await;
    let code = wl_parties::Party::find_or_404(app.db(), vendor.0)
        .await
        .unwrap()
        .code;

    let drawing = bill(&app, &amal, &code, "VD-1", "2000", "owner_drawings").await;
    let row = DrawingApproval::for_source(app.db(), &drawing.subject())
        .await
        .unwrap()
        .remove(0);
    assert_eq!(row.status, DrawingStatus::Requested);
    let request_id = row.approval_request_id.expect("an approval request");
    app.acting_as(&amal);
    app.htmx()
        .post(
            &format!("/supplier-bills/{}/transitions/approve", drawing.id),
            &[("reason", "")],
        )
        .await
        .assert_status(422);
    assert!(
        approvals::approve(app.state(), request_id, &amal, "")
            .await
            .is_err(),
        "the requester cannot approve their drawing"
    );
    approvals::approve(app.state(), request_id, &omar, "")
        .await
        .unwrap()
        .expect("the approval runs");

    // Omar records one himself: he is not told "approved", he waits for
    // Osman (no self-approval in the 07 deployment).
    let epi = entity(&app, "EPI").await;
    let mut draft = BillDraft::new(vendor, "VD-2", day(0));
    draft.legal_entity_id = Some(epi.id);
    draft = draft.line(
        BillLineDraft::new("Family", 50_000, VatCategory::Exempt).cost_code("owner_drawings"),
    );
    let own = supplier_bill::create(app.db(), &draft, Some(omar.id))
        .await
        .unwrap();
    bill_submit::submit(app.state(), By::System(Some(omar.id)), own.id)
        .await
        .unwrap();
    let waiting = DrawingApproval::for_source(app.db(), &own.subject())
        .await
        .unwrap()
        .remove(0);
    assert_eq!(waiting.status, DrawingStatus::Requested);
    assert!(!waiting.self_approved);
    let request = waiting.approval_request_id.expect("a request");
    assert!(
        approvals::approve(app.state(), request, &omar, "")
            .await
            .is_err()
    );
    approvals::approve(app.state(), request, &osman, "")
        .await
        .unwrap();
    let done = DrawingApproval::for_source(app.db(), &own.subject())
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        (done.status, done.approved_by),
        (DrawingStatus::Approved, Some(osman.id))
    );
}

// -------------------------------------------------------- supplier scorecard

/// `STK.RUL.09` with the 07 threshold of 70: figures that score 69 open a
/// review task for procurement, figures that score 70 do not. The scores come
/// from the module's own `score_of` with the deployment's settings.
#[renox::test]
async fn a_supplier_scoring_69_is_reviewed_and_one_scoring_70_is_not() {
    let app = manufacturing_booted().await;
    let settings = host::manufacturing_seed::inventory(wl_inventory::WlInventory::new())
        .config()
        .score
        .clone();
    assert_eq!(settings.threshold, 70);
    let figures = |on_time_bp: i64| Figures {
        receipts: settings.min_receipts,
        on_time: 1,
        on_time_bp: Some(on_time_bp),
        rejected_bp: Some(0),
        price_variance_bp: Some(0),
        ..Figures::default()
    };
    // Weights 40 / 30 / 15 / 15: a share of 22.5% on time scores 69, 25% 70.
    let (low, high) = (figures(2_250), figures(2_500));
    assert_eq!(score_of(&low, &settings), Some(69));
    assert_eq!(score_of(&high, &settings), Some(70));

    let mut made = Vec::new();
    for (supplier, found) in [(31, &low), (32, &high)] {
        let score = score_of(found, &settings);
        let mut row = SupplierScore {
            supplier_party_id: supplier,
            snapshot_date: day(0),
            window_from: day(-90),
            window_to: day(-1),
            receipts: found.receipts,
            on_time: found.on_time,
            on_time_bp: found.on_time_bp,
            rejected_bp: found.rejected_bp,
            price_variance_bp: found.price_variance_bp,
            score,
            band: band_of(score, &settings),
            computed_at: Some(renox::db::now()),
            ..SupplierScore::default()
        };
        row.save(app.db()).await.unwrap();
        made.push(row);
    }
    assert_eq!(made[0].band, ScoreBand::BelowThreshold);
    assert_eq!(made[1].band, ScoreBand::Watch);
    supplier_review::on_snapshots(app.state(), &made)
        .await
        .unwrap();

    let reviews = SupplierReview::query().get(app.db()).await.unwrap();
    assert_eq!(reviews.len(), 1, "only the 69 is reviewed");
    assert_eq!(reviews[0].supplier_party_id, 31);
    let tasks = task_creator(app.state());
    assert!(
        tasks
            .find_open(app.state(), &supplier_review::task_key(31))
            .await
            .unwrap()
            .is_some(),
        "procurement has the review task"
    );
    assert!(
        tasks
            .find_open(app.state(), &supplier_review::task_key(32))
            .await
            .unwrap()
            .is_none()
    );
}

// ------------------------------------------------------------- cancellation

/// *Cancellation outcome* with the 07 values: the three outcomes are offered
/// for a started line, and the default cancellation fee on a prepaid share of
/// AED 1,000 is nothing (`SAL.ACL.05`: 0%). The plan is the one the cancel
/// routes run, fed with the settings of the 07 sales module.
#[renox::test]
async fn cancelling_a_started_prepaid_line_in_07_charges_no_fee() {
    use wl_sales::cancel_lines::{CancelAsk, CancelKind, PlanFacts, WantedLine, plan};
    use wl_sales::sale_line::{KIND_ITEM, SaleLine};

    let sales = host::sales_for("manufacturing");
    let config = sales.config();
    assert_eq!(
        config.cancel_outcomes,
        ["finish_to_stock", "scrap", "return_to_stock"]
    );
    let facts = PlanFacts {
        started_lines: vec![1],
        offered: config.cancel_outcomes.clone(),
        fee_bps: config.cancellation_fee_bps,
        fee_amount: config.cancellation_fee_amount,
        ..PlanFacts::default()
    };
    let order = SalesOrder {
        status: SalesOrderStatus::PartiallyFulfilled,
        ..SalesOrder::default()
    };
    let lines = [
        SaleLine {
            id: 1,
            line_no: 1,
            kind: KIND_ITEM.to_owned(),
            quantity_milli: 10_000,
            unit_price: 10_000,
            vat_category: VatCategory::ZeroRated,
            ..SaleLine::default()
        },
        SaleLine {
            id: 2,
            line_no: 2,
            kind: KIND_ITEM.to_owned(),
            quantity_milli: 1_000,
            unit_price: 500,
            vat_category: VatCategory::ZeroRated,
            ..SaleLine::default()
        },
    ];
    for outcome in ["finish_to_stock", "scrap", "return_to_stock"] {
        let ask =
            CancelAsk {
                kind: CancelKind::Line,
                reason: "The customer withdrew".into(),
                lines: vec![WantedLine::new(1, 10_000).outcome(
                    wl_kernel::sales::LineOutcomeKind::parse(outcome).expect("an outcome"),
                )],
                keep_payment: false,
            };
        let found = plan(&order, &lines, &ask, &facts);
        assert_eq!(found.problem, None, "{outcome}");
        assert_eq!(found.lines[0].share, 100_000, "AED 1,000 prepaid");
        assert_eq!(found.fee_total(), 0, "{outcome}: no fee");
        assert!(found.approval, "a started line needs the manager");
    }
}

// ------------------------------------------------- credit at the dispatch

/// BL-STK-21 AC1 and BL-SAL-10 AC1 with the real pieces on both sides: the
/// order of a customer was confirmed within its limit; an invoice of AED
/// 600,000 (not overdue) takes the customer over a limit of AED 500,000; the
/// dispatch of the order is picked and `dispatch` is refused by the real
/// credit re-check of wl-sales ("credit re-check failed"), the dispatch stays
/// `picked` with its stock reserved and the order is on `credit_hold`.
#[renox::test]
async fn a_dispatch_over_the_limit_is_refused_by_the_real_credit_recheck_and_holds_the_order() {
    let app = manufacturing_booted().await;
    let sales = person(&app, "Sara", SALES).await;
    let admin = person(&app, "Alia", ADMIN).await;
    let keeper = person(&app, "Karim", host::roles::STOREKEEPER).await;
    let customer = company(&app, "Al Noor Trading LLC").await;
    credit_for(&app, customer).await;
    submitted(&app, &sales, customer).await.assert_status(303);
    let order = latest_order(&app).await;
    assert_eq!(order.status, SalesOrderStatus::Confirmed);
    issued_invoice(
        &app,
        &admin,
        customer,
        60_000_000,
        VatCategory::ZeroRated,
        day(30),
    )
    .await;

    let mut panel = wl_inventory::Item::new("PANEL", "Panel", "pcs");
    panel.save(app.db()).await.unwrap();
    let mut store = wl_inventory::Location::new("FG", "Finished goods");
    store.save(app.db()).await.unwrap();
    wl_inventory::movement::StockMovement::receive(
        panel.id,
        None,
        store.id,
        20_000,
        1_000,
        20_000,
        &wl_kernel::Subject::new("inventory.goods_receipt", 1),
        None,
    )
    .save(app.db())
    .await
    .unwrap();
    let dispatch = wl_inventory::create_dispatch(
        app.db(),
        None,
        wl_inventory::NewDispatch {
            source: order.subject(),
            recipient: Some(customer),
            recipient_name: "Al Noor".into(),
            address: "Dubai".into(),
            location_id: store.id,
            lines: vec![wl_inventory::NewDispatchLine::of(panel.id, 10_000)],
        },
    )
    .await
    .unwrap();
    let act = |action: &'static str, form: Vec<(&'static str, &'static str)>| {
        let (app, keeper, id) = (&app, &keeper, dispatch.id);
        async move {
            app.acting_as(keeper);
            app.htmx()
                .post(&format!("/dispatches/{id}/{action}"), &form)
                .await
        }
    };
    act("pick", vec![]).await.assert_ok();

    let refused = act(
        "dispatch",
        vec![("handover", "courier"), ("tracking_number", "TRK-1")],
    )
    .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("credit re-check failed"),
        "{}",
        refused.text()
    );
    let after = wl_inventory::Dispatch::find_or_404(app.db(), dispatch.id)
        .await
        .unwrap();
    assert_eq!(after.status, wl_inventory::DispatchStatus::Picked);
    assert_eq!(
        status_of(&app, order.id).await,
        SalesOrderStatus::CreditHold,
        "the real checker held the real order"
    );
}
