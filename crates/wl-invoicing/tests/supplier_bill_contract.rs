//! BL-INV-24 (#87): a vendor contract bill is matched only for work done up
//! to the contract's end date (or termination date); a line after it holds
//! the bill in `exception` with reason *work after contract end*, which only
//! a user other than the creator approves, with a reason (`FIN.ACL.03`).
//! A test cannot build an `AuthUser`, so the services run
//! `By::System(Some(user id))`.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_invoicing::bill_machine::approve;
use wl_invoicing::bill_submit::submit;
use wl_invoicing::credit_notes::By;
use wl_invoicing::duplicates::exceptions_of;
use wl_invoicing::supplier_bill::{self, BillDraft, POSSIBLE_DUPLICATE, WORK_AFTER_CONTRACT_END};
use wl_invoicing::{BillLineDraft, SupplierBill, SupplierBillStatus};
use wl_kernel::Subject;
use wl_kernel::invoicing::VatCategory;
use wl_kernel::tickets::SERVICE_CONTRACT_SUBJECT_TYPE;

use support::{BLUE_DUNES, Booted, boot};

const A: i64 = 7;
const B: i64 = 8;
/// "AMC-2026-0012", SC-12 of the issue.
const SC_12: i64 = 12;

fn date(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, month, day).unwrap()
}

fn is_guard(result: std::result::Result<SupplierBill, TransitionError>) -> bool {
    matches!(
        result.map_err(TransitionError::into_error),
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    )
}

/// A bill of Blue Dunes against SC-12 with a line per `(date, price)`.
async fn bill(t: &Booted, number: &str, lines: &[(Option<NaiveDate>, i64)]) -> SupplierBill {
    let mut draft = BillDraft::new(BLUE_DUNES, number, date(10, 6))
        .contract(Subject::new(SERVICE_CONTRACT_SUBJECT_TYPE, SC_12), None);
    for (day, price) in lines {
        let mut line = BillLineDraft::new("AMC visit", *price, VatCategory::Standard);
        line.service_date = *day;
        draft = draft.line(line);
    }
    supplier_bill::create(t.db(), &draft, Some(A))
        .await
        .unwrap()
}

async fn submitted(t: &Booted, bill: &SupplierBill) -> SupplierBill {
    submit(t.state(), By::System(Some(A)), bill.id)
        .await
        .unwrap()
}

/// AC1: SC-12 ends 2026-09-30; lines of 2026-09-28 (AED 800) and 2026-10-05
/// (AED 600): `exception`, *work after contract end*, naming the 10-05 line.
#[renox::test]
async fn inv24_a_line_after_the_contract_end_holds_the_bill() {
    let t = boot().await;
    t.contracts.ends_on(SC_12, date(9, 30));
    let draft = bill(
        &t,
        "C-1",
        &[(Some(date(9, 28)), 80_000), (Some(date(10, 5)), 60_000)],
    )
    .await;
    let held = submitted(&t, &draft).await;
    assert_eq!(held.status, SupplierBillStatus::Exception);
    assert!(held.after_end_hold);
    assert_eq!(held.duplicate_of, None);
    let rows = exceptions_of(t.db(), held.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].reason_code, WORK_AFTER_CONTRACT_END);
    let detail = rows[0].detail.clone().unwrap();
    assert!(detail.contains("line 2"), "{detail}");
    assert!(detail.contains("2026-10-05"), "{detail}");
    assert!(!detail.contains("2026-09-28"), "{detail}");
}

/// AC2: the same bill with only the 2026-09-28 line is `matched`.
#[renox::test]
async fn inv24_work_up_to_the_end_date_is_matched() {
    let t = boot().await;
    t.contracts.ends_on(SC_12, date(9, 30));
    let draft = bill(&t, "C-2", &[(Some(date(9, 28)), 80_000)]).await;
    let done = submitted(&t, &draft).await;
    assert_eq!(done.status, SupplierBillStatus::Matched);
    assert!(!done.after_end_hold);
    assert!(exceptions_of(t.db(), done.id).await.unwrap().is_empty());
    // The end date itself is covered.
    let last = bill(&t, "C-2b", &[(Some(date(9, 30)), 80_000)]).await;
    assert_eq!(
        submitted(&t, &last).await.status,
        SupplierBillStatus::Matched
    );
}

/// AC3: a contract terminated on 2026-09-15 covers no work of 2026-09-20.
#[renox::test]
async fn inv24_a_terminated_contract_covers_work_up_to_its_termination_date() {
    let t = boot().await;
    t.contracts.ends_on(SC_12, date(9, 15));
    let draft = bill(&t, "C-3", &[(Some(date(9, 20)), 80_000)]).await;
    let done = submitted(&t, &draft).await;
    assert_eq!(done.status, SupplierBillStatus::Exception);
    let on_time = bill(&t, "C-3b", &[(Some(date(9, 15)), 80_000)]).await;
    assert_eq!(
        submitted(&t, &on_time).await.status,
        SupplierBillStatus::Matched
    );
}

/// AC4: accountant B approves the exception with a reason; creator A cannot.
#[renox::test]
async fn inv24_only_another_user_approves_the_exception_with_a_reason() {
    let t = boot().await;
    t.contracts.ends_on(SC_12, date(9, 30));
    let draft = bill(&t, "C-4", &[(Some(date(10, 5)), 60_000)]).await;
    let held = submitted(&t, &draft).await;
    assert_eq!(held.status, SupplierBillStatus::Exception);
    assert!(is_guard(
        approve(t.state(), By::System(Some(A)), held.id, "finish open visit").await
    ));
    assert!(is_guard(
        approve(t.state(), By::System(Some(B)), held.id, "  ").await
    ));
    let done = approve(t.state(), By::System(Some(B)), held.id, "finish open visit")
        .await
        .unwrap();
    assert_eq!(done.status, SupplierBillStatus::Approved);
    assert_eq!(done.approved_by, Some(B));
    assert_eq!(done.approval_reason.as_deref(), Some("finish open visit"));
}

/// AC5: a contract bill line without a service date is refused at `submit`.
#[renox::test]
async fn inv24_a_line_without_a_service_date_is_refused() {
    let t = boot().await;
    t.contracts.ends_on(SC_12, date(9, 30));
    let draft = bill(&t, "C-5", &[(Some(date(9, 28)), 80_000), (None, 60_000)]).await;
    assert!(is_guard(
        submit(t.state(), By::System(Some(A)), draft.id).await
    ));
    let still = SupplierBill::find_or_404(t.db(), draft.id).await.unwrap();
    assert_eq!(still.status, SupplierBillStatus::Draft);
}

/// A duplicate that is also after the end lists both reasons; one approval
/// covers both. A contract the coverage does not know holds nothing.
#[renox::test]
async fn inv24_both_reasons_are_listed_and_an_unknown_contract_holds_nothing() {
    let t = boot().await;
    t.contracts.ends_on(SC_12, date(9, 30));
    let first = bill(&t, "C-6", &[(Some(date(9, 28)), 80_000)]).await;
    submitted(&t, &first).await;
    let second = bill(&t, "c-6 ", &[(Some(date(10, 5)), 80_000)]).await;
    let held = submitted(&t, &second).await;
    assert_eq!(held.status, SupplierBillStatus::Exception);
    assert_eq!(held.duplicate_of, Some(first.id));
    let codes: Vec<String> = exceptions_of(t.db(), held.id)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.reason_code)
        .collect();
    assert_eq!(codes, [POSSIBLE_DUPLICATE, WORK_AFTER_CONTRACT_END]);
    let done = approve(t.state(), By::System(Some(B)), held.id, "agreed")
        .await
        .unwrap();
    assert_eq!(done.status, SupplierBillStatus::Approved);

    // Contract 99 is not known to ticketing: no line is late.
    let mut other = BillDraft::new(BLUE_DUNES, "C-7", date(10, 6))
        .contract(Subject::new(SERVICE_CONTRACT_SUBJECT_TYPE, 99), None);
    let mut line = BillLineDraft::new("AMC visit", 80_000, VatCategory::Standard);
    line.service_date = Some(date(12, 1));
    other = other.line(line);
    let unknown = supplier_bill::create(t.db(), &other, Some(A))
        .await
        .unwrap();
    assert_eq!(
        submitted(&t, &unknown).await.status,
        SupplierBillStatus::Matched
    );
}

/// A period line counts by its last day: a line for 2026-09-25 to 2026-10-05
/// on a contract that ends on 2026-09-30 is after the end (the exception names
/// the last day); one for 2026-09-20 to 2026-09-30 is matched; a period over a
/// single day without an end date is as before.
#[renox::test]
async fn inv24_a_period_line_counts_by_its_last_day() {
    let t = boot().await;
    t.contracts.ends_on(SC_12, date(9, 30));
    let period = |from: NaiveDate, to: NaiveDate| {
        let mut line = BillLineDraft::new("AMC, September", 80_000, VatCategory::Standard);
        line.service_date = Some(from);
        line.service_end_date = Some(to);
        line
    };
    let mut over = BillDraft::new(BLUE_DUNES, "C-5", date(10, 6))
        .contract(Subject::new(SERVICE_CONTRACT_SUBJECT_TYPE, SC_12), None);
    over = over.line(period(date(9, 25), date(10, 5)));
    let over = supplier_bill::create(t.db(), &over, Some(A)).await.unwrap();
    let held = submitted(&t, &over).await;
    assert_eq!(held.status, SupplierBillStatus::Exception);
    assert!(held.after_end_hold);
    let rows = exceptions_of(t.db(), held.id).await.unwrap();
    assert_eq!(rows[0].reason_code, WORK_AFTER_CONTRACT_END);
    assert!(
        rows[0].detail.clone().unwrap().contains("2026-10-05"),
        "{:?}",
        rows[0].detail
    );

    let mut inside = BillDraft::new(BLUE_DUNES, "C-6", date(10, 6))
        .contract(Subject::new(SERVICE_CONTRACT_SUBJECT_TYPE, SC_12), None);
    inside = inside.line(period(date(9, 20), date(9, 30)));
    let inside = supplier_bill::create(t.db(), &inside, Some(A))
        .await
        .unwrap();
    let done = submitted(&t, &inside).await;
    assert_eq!(done.status, SupplierBillStatus::Matched);
    let stored = supplier_bill::lines_of(t.db(), done.id).await.unwrap();
    assert_eq!(stored[0].service_end_date, Some(date(9, 30)));
}
