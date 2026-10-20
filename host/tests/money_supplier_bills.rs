//! BL-PRP-26 (#276) items 20 and 20a with the real modules of the host: a
//! second bill for the same AMC contract, period and amount is held as a
//! possible duplicate (`PRP.ACL.22`) and approved only by another user with a
//! reason; a bill with a line dated after the contract's end is held with the
//! reason *work after contract end*.

mod support;

use host::roles::ACCOUNTANT;
use renox::prelude::*;
use renox::testing::TestApp;
use support::*;
use wl_invoicing::duplicates::exceptions_of;
use wl_invoicing::supplier_bill::{POSSIBLE_DUPLICATE, WORK_AFTER_CONTRACT_END};
use wl_invoicing::{SupplierBill, SupplierBillStatus};
use wl_kernel::PartyId;
use wl_kernel::tickets::SERVICE_CONTRACT_SUBJECT_TYPE;
use wl_ticketing::{NewContract, ServiceContract, ServiceContracts};

/// The AMC contract of a vendor, active from today for `days` days.
async fn amc_contract(app: &TestApp, vendor: PartyId, days: u64) -> ServiceContract {
    let admin = person(app, "Alia", host::roles::ADMIN).await;
    let contract = ServiceContracts::create_draft(
        app.state(),
        NewContract {
            vendor,
            title: "AC annual maintenance".into(),
            start_date: day(0),
            end_date: day(days as i64),
            renewal_notice_days: 60,
            sla_policy_id: None,
        },
    )
    .await
    .unwrap();
    ServiceContracts::add_coverage(
        app.db(),
        &contract,
        wl_kernel::Subject::new("property.building", 1),
        "general",
    )
    .await
    .unwrap();
    app.acting_as(&admin);
    app.htmx()
        .post(
            &format!("/service-contracts/{}/activate", contract.id),
            &[("x", "y")],
        )
        .await
        .assert_ok();
    ServiceContract::find_or_404(app.db(), contract.id)
        .await
        .unwrap()
}

/// The form of a bill of the vendor `code` against `contract` for the period
/// of this month, one line dated `service` costing `price` dirhams.
fn form(
    code: &str,
    number: &str,
    contract: &str,
    service: &str,
    price: &str,
) -> Vec<(String, String)> {
    let pairs = [
        ("supplier", code),
        ("supplier_invoice_number", number),
        ("bill_date", &day(0).to_string()),
        ("currency", "AED"),
        ("contract_type", SERVICE_CONTRACT_SUBJECT_TYPE),
        ("contract_id", contract),
        ("period_start", &day(-5).to_string()),
        ("period_end", &day(25).to_string()),
        ("line_1_description", "AMC visit"),
        ("line_1_quantity", "1"),
        ("line_1_unit_price", price),
        ("line_1_vat", "standard"),
        ("line_1_service_date", service),
    ];
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

/// Creates and submits the bill of `fields` as `by`; the bill after the
/// submission.
async fn submitted(app: &TestApp, by: &User, fields: &[(String, String)]) -> SupplierBill {
    let fields: Vec<(&str, &str)> = fields
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    app.acting_as(by);
    app.htmx()
        .post("/supplier-bills", &fields)
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

#[renox::test]
async fn a_second_bill_for_the_same_contract_period_and_amount_is_held_as_a_duplicate() {
    let app = booted().await;
    let amal = person(&app, "Amal", ACCOUNTANT).await;
    let bilal = person(&app, "Bilal", ACCOUNTANT).await;
    let vendor = company(&app, "CoolAir LLC").await;
    let code = wl_parties::Party::find_or_404(app.db(), vendor.0)
        .await
        .unwrap()
        .code;
    let contract = amc_contract(&app, vendor, 365).await;
    let id = contract.id.to_string();

    // The first bill is matched; the same contract, period and amount under
    // another invoice number is held.
    let first = submitted(
        &app,
        &amal,
        &form(&code, "AMC-1", &id, &day(2).to_string(), "800"),
    )
    .await;
    assert_eq!(first.status, SupplierBillStatus::Matched);
    let second = submitted(
        &app,
        &amal,
        &form(&code, "AMC-2", &id, &day(4).to_string(), "800"),
    )
    .await;
    assert_eq!(second.status, SupplierBillStatus::Exception);
    assert_eq!(second.duplicate_of, Some(first.id));
    let reasons = exceptions_of(app.db(), second.id).await.unwrap();
    assert_eq!(reasons[0].reason_code, POSSIBLE_DUPLICATE);

    // Another amount is not a duplicate.
    let third = submitted(
        &app,
        &amal,
        &form(&code, "AMC-3", &id, &day(4).to_string(), "950"),
    )
    .await;
    assert_eq!(third.status, SupplierBillStatus::Matched);

    // The creator cannot approve the held bill; another user can, with a
    // reason.
    let path = format!("/supplier-bills/{}/transitions/approve", second.id);
    app.acting_as(&amal);
    let refused = app.htmx().post(&path, &[("reason", "same visit")]).await;
    assert!(refused.status.is_client_error(), "{}", refused.status);
    app.acting_as(&bilal);
    app.htmx()
        .post(&path, &[("reason", "  ")])
        .await
        .assert_status(422);
    app.htmx()
        .post(&path, &[("reason", "two visits, both done")])
        .await
        .assert_status(303);
    let done = SupplierBill::find_or_404(app.db(), second.id)
        .await
        .unwrap();
    assert_eq!(done.status, SupplierBillStatus::Approved);
    assert_eq!(done.approved_by, Some(bilal.id));
}

#[renox::test]
async fn a_line_dated_after_the_contract_end_holds_the_bill() {
    let app = booted().await;
    let amal = person(&app, "Amal", ACCOUNTANT).await;
    let bilal = person(&app, "Bilal", ACCOUNTANT).await;
    let vendor = company(&app, "CoolAir LLC").await;
    let code = wl_parties::Party::find_or_404(app.db(), vendor.0)
        .await
        .unwrap()
        .code;
    // The contract ends in 5 days; a line is dated 10 days from now.
    let contract = amc_contract(&app, vendor, 5).await;
    let id = contract.id.to_string();
    let inside = day(3).to_string();
    let after = day(10).to_string();

    let kept = submitted(&app, &amal, &form(&code, "AMC-10", &id, &inside, "800")).await;
    assert_eq!(kept.status, SupplierBillStatus::Matched, "inside the term");
    let held = submitted(&app, &amal, &form(&code, "AMC-11", &id, &after, "650")).await;
    assert_eq!(held.status, SupplierBillStatus::Exception);
    assert!(held.after_end_hold);
    let reasons = exceptions_of(app.db(), held.id).await.unwrap();
    assert_eq!(reasons.len(), 1);
    assert_eq!(reasons[0].reason_code, WORK_AFTER_CONTRACT_END);

    // Another user approves it with a reason; its creator cannot.
    let path = format!("/supplier-bills/{}/transitions/approve", held.id);
    app.acting_as(&amal);
    let refused = app
        .htmx()
        .post(&path, &[("reason", "finish open visit")])
        .await;
    assert!(refused.status.is_client_error(), "{}", refused.status);
    app.acting_as(&bilal);
    app.htmx()
        .post(&path, &[("reason", "finish the open visit")])
        .await
        .assert_status(303);
    assert_eq!(
        SupplierBill::find_or_404(app.db(), held.id)
            .await
            .unwrap()
            .status,
        SupplierBillStatus::Approved
    );
}
