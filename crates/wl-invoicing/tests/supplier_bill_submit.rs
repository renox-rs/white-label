//! The Supplier bill machine and the duplicate hold (wl-invoicing §4, §7,
//! `FIN.ACL.03`, #74 part 2). A test cannot build an `AuthUser`, so the
//! services run `By::System(Some(user id))`; the permission checks are the
//! machine's and are covered with the host's users in part 5.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_invoicing::bill_machine::{approve, void};
use wl_invoicing::bill_submit::submit;
use wl_invoicing::credit_notes::By;
use wl_invoicing::duplicates::{BillException, exceptions_of};
use wl_invoicing::supplier_bill::{self, BillDraft, POSSIBLE_DUPLICATE};
use wl_invoicing::{BillLineDraft, DuplicateKey, SupplierBill, SupplierBillStatus, WlInvoicing};
use wl_kernel::invoicing::VatCategory;
use wl_kernel::{PartyId, Subject};

use support::{BLUE_DUNES, Booted, QUIET_SANDS, boot, boot_with};

const A: i64 = 7;
const B: i64 = 8;

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
}

fn line() -> BillLineDraft {
    BillLineDraft::new("Pump repair", 100_000, VatCategory::Standard)
}

async fn draft_for(t: &Booted, supplier: PartyId, number: &str) -> SupplierBill {
    let draft = BillDraft::new(supplier, number, day(5)).line(line());
    supplier_bill::create(t.db(), &draft, Some(A))
        .await
        .unwrap()
}

async fn now(t: &Booted, bill: &SupplierBill) -> SupplierBill {
    SupplierBill::find_or_404(t.db(), bill.id).await.unwrap()
}

/// Submits `bill` as user A and approves it as user B.
async fn approved(t: &Booted, number: &str) -> SupplierBill {
    let bill = draft_for(t, BLUE_DUNES, number).await;
    submit(t.state(), By::System(Some(A)), bill.id)
        .await
        .unwrap();
    approve(t.state(), By::System(Some(B)), bill.id, "")
        .await
        .unwrap()
}

fn is_guard(result: std::result::Result<SupplierBill, TransitionError>) -> bool {
    matches!(
        result.map_err(TransitionError::into_error),
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    )
}

#[renox::test]
async fn a_clean_bill_goes_to_matched() {
    let t = boot().await;
    let bill = draft_for(&t, BLUE_DUNES, "INV-1").await;
    let done = submit(t.state(), By::System(Some(A)), bill.id)
        .await
        .unwrap();
    assert_eq!(done.status, SupplierBillStatus::Matched);
    assert_eq!(done.duplicate_of, None);
    assert!(exceptions_of(t.db(), bill.id).await.unwrap().is_empty());
    let history = renox::state_machine::history(t.db(), SupplierBill::TABLE, bill.id)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].transition, "submit_system");
    // Only a draft is submitted.
    assert!(matches!(
        submit(t.state(), By::System(Some(A)), bill.id).await,
        Err(TransitionError::NotFrom { .. })
    ));
}

#[renox::test]
async fn the_same_number_holds_the_bill_and_only_another_user_approves_it() {
    let t = boot().await;
    let first = approved(&t, "INV-77").await;
    assert_eq!(first.status, SupplierBillStatus::Approved);

    let second = draft_for(&t, BLUE_DUNES, "inv-77 ").await;
    let held = submit(t.state(), By::System(Some(A)), second.id)
        .await
        .unwrap();
    assert_eq!(held.status, SupplierBillStatus::Exception);
    assert_eq!(held.duplicate_of, Some(first.id));
    let rows = exceptions_of(t.db(), second.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].reason_code, POSSIBLE_DUPLICATE);
    assert_eq!(rows[0].related_bill_id, Some(first.id));

    // The creator cannot approve, with or without a reason.
    assert!(is_guard(
        approve(t.state(), By::System(Some(A)), second.id, "same delivery").await
    ));
    // Another user needs a reason.
    assert!(is_guard(
        approve(t.state(), By::System(Some(B)), second.id, "  ").await
    ));
    assert_eq!(now(&t, &second).await.status, SupplierBillStatus::Exception);

    let done = approve(
        t.state(),
        By::System(Some(B)),
        second.id,
        "two deliveries, same number",
    )
    .await
    .unwrap();
    assert_eq!(done.status, SupplierBillStatus::Approved);
    assert_eq!(done.approved_by, Some(B));
    assert_eq!(
        done.approval_reason.as_deref(),
        Some("two deliveries, same number")
    );
    let history = renox::state_machine::history(t.db(), SupplierBill::TABLE, second.id)
        .await
        .unwrap();
    assert_eq!(
        history.last().unwrap().reason.as_deref(),
        Some("two deliveries, same number")
    );
}

#[renox::test]
async fn a_void_bill_and_another_supplier_do_not_hold() {
    let t = boot().await;
    // Void leaves draft, matched and exception, so the first bill is matched.
    let first = draft_for(&t, BLUE_DUNES, "INV-77").await;
    submit(t.state(), By::System(Some(A)), first.id)
        .await
        .unwrap();
    // Another supplier's bill with the same number is matched.
    let other = draft_for(&t, QUIET_SANDS, "INV-77").await;
    let done = submit(t.state(), By::System(Some(A)), other.id)
        .await
        .unwrap();
    assert_eq!(done.status, SupplierBillStatus::Matched);
    // A draft is not compared.
    let draft = draft_for(&t, BLUE_DUNES, "INV-77").await;
    let drafts = draft_for(&t, BLUE_DUNES, "INV-77").await;
    // Once the first bill is void the same number is matched.
    assert!(is_guard(
        void(t.state(), By::System(Some(A)), first.id, "").await
    ));
    void(t.state(), By::System(Some(A)), first.id, "entered twice")
        .await
        .unwrap();
    let done = submit(t.state(), By::System(Some(A)), draft.id)
        .await
        .unwrap();
    assert_eq!(done.status, SupplierBillStatus::Matched);
    // The next one now matches the second, which is not void.
    let held = submit(t.state(), By::System(Some(A)), drafts.id)
        .await
        .unwrap();
    assert_eq!(held.status, SupplierBillStatus::Exception);
    assert_eq!(held.duplicate_of, Some(draft.id));
}

#[renox::test]
async fn a_held_bill_is_voided_with_a_reason_and_kept() {
    let t = boot().await;
    approved(&t, "INV-77").await;
    let second = draft_for(&t, BLUE_DUNES, "INV-77").await;
    submit(t.state(), By::System(Some(A)), second.id)
        .await
        .unwrap();
    assert!(is_guard(
        void(t.state(), By::System(Some(A)), second.id, " ").await
    ));
    let done = void(t.state(), By::System(Some(A)), second.id, "duplicate")
        .await
        .unwrap();
    assert_eq!(done.status, SupplierBillStatus::Void);
    assert_eq!(done.void_reason.as_deref(), Some("duplicate"));
    assert_eq!(
        supplier_bill::lines_of(t.db(), second.id)
            .await
            .unwrap()
            .len(),
        1
    );
    // A void bill is terminal.
    assert!(matches!(
        approve(t.state(), By::System(Some(B)), second.id, "ok").await,
        Err(TransitionError::NotFrom { .. })
    ));
    // A draft can be voided too.
    let draft = draft_for(&t, BLUE_DUNES, "INV-9").await;
    let done = void(t.state(), By::System(Some(A)), draft.id, "wrong supplier")
        .await
        .unwrap();
    assert_eq!(done.status, SupplierBillStatus::Void);
}

#[renox::test]
async fn every_exception_row_is_shown_and_one_approval_covers_them() {
    let t = boot().await;
    approved(&t, "INV-77").await;
    let second = draft_for(&t, BLUE_DUNES, "INV-77").await;
    submit(t.state(), By::System(Some(A)), second.id)
        .await
        .unwrap();
    // A second reason, written as a match would.
    BillException {
        bill_id: second.id,
        reason_code: "other_reason".to_owned(),
        related_bill_id: None,
        ..Default::default()
    }
    .insert(t.db())
    .await
    .unwrap();
    let rows = exceptions_of(t.db(), second.id).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert!(is_guard(
        approve(t.state(), By::System(Some(A)), second.id, "fine").await
    ));
    let done = approve(t.state(), By::System(Some(B)), second.id, "checked")
        .await
        .unwrap();
    assert_eq!(done.status, SupplierBillStatus::Approved);
}

#[renox::test]
async fn with_the_check_off_nothing_is_held() {
    let t = boot_with(WlInvoicing::new().duplicate_check(false)).await;
    approved(&t, "INV-77").await;
    let second = draft_for(&t, BLUE_DUNES, "inv-77 ").await;
    let done = submit(t.state(), By::System(Some(A)), second.id)
        .await
        .unwrap();
    assert_eq!(done.status, SupplierBillStatus::Matched);
}

#[renox::test]
async fn the_contract_period_amount_key_holds_only_when_enabled() {
    let contract = Subject::new("ticketing.contract", 3);
    let period = Some((day(1), day(31)));
    let with_contract = |number: &str, supplier: PartyId| {
        BillDraft::new(supplier, number, day(5))
            .contract(contract.clone(), period)
            .line(line().service_date(day(10)))
    };
    for (keys, expected) in [
        (
            vec![DuplicateKey::SupplierInvoiceNumber],
            SupplierBillStatus::Matched,
        ),
        (
            vec![
                DuplicateKey::SupplierInvoiceNumber,
                DuplicateKey::ContractPeriodAmount,
            ],
            SupplierBillStatus::Exception,
        ),
    ] {
        let t = boot_with(WlInvoicing::new().duplicate_keys(&keys)).await;
        let first = supplier_bill::create(t.db(), &with_contract("C-1", BLUE_DUNES), Some(A))
            .await
            .unwrap();
        submit(t.state(), By::System(Some(A)), first.id)
            .await
            .unwrap();
        let second = supplier_bill::create(t.db(), &with_contract("C-2", BLUE_DUNES), Some(A))
            .await
            .unwrap();
        let done = submit(t.state(), By::System(Some(A)), second.id)
            .await
            .unwrap();
        assert_eq!(done.status, expected);
        if expected == SupplierBillStatus::Exception {
            assert_eq!(done.duplicate_of, Some(first.id));
        }
        // The same contract, period and amount from another supplier.
        let third = supplier_bill::create(t.db(), &with_contract("C-3", QUIET_SANDS), Some(A))
            .await
            .unwrap();
        let done = submit(t.state(), By::System(Some(A)), third.id)
            .await
            .unwrap();
        assert_eq!(done.status, SupplierBillStatus::Matched);
    }
}

#[renox::test]
async fn two_bills_submitted_together_end_with_one_held() {
    let t = boot().await;
    let one = draft_for(&t, BLUE_DUNES, "INV-80").await;
    let two = draft_for(&t, BLUE_DUNES, "INV-80").await;
    let (a, b) = renox::tokio::join!(
        submit(t.state(), By::System(Some(A)), one.id),
        submit(t.state(), By::System(Some(A)), two.id)
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    let mut statuses = [a.status, b.status];
    statuses.sort_by_key(|s| s.as_str());
    assert_eq!(
        statuses,
        [SupplierBillStatus::Exception, SupplierBillStatus::Matched]
    );
    let (held, kept) = if a.status == SupplierBillStatus::Exception {
        (a, b)
    } else {
        (b, a)
    };
    assert_eq!(held.duplicate_of, Some(kept.id));
}

#[renox::test]
async fn a_bill_that_cannot_be_submitted_is_refused() {
    let t = boot().await;
    let empty = supplier_bill::create(
        t.db(),
        &BillDraft::new(BLUE_DUNES, "INV-1", day(5)),
        Some(A),
    )
    .await
    .unwrap();
    assert!(is_guard(
        submit(t.state(), By::System(Some(A)), empty.id).await
    ));
    let stranger = draft_for(&t, PartyId(99), "INV-2").await;
    assert!(is_guard(
        submit(t.state(), By::System(Some(A)), stranger.id).await
    ));
    let usd = supplier_bill::create(
        t.db(),
        &BillDraft::new(BLUE_DUNES, "INV-3", day(5))
            .currency("USD", None)
            .line(line()),
        Some(A),
    )
    .await
    .unwrap();
    assert!(is_guard(
        submit(t.state(), By::System(Some(A)), usd.id).await
    ));
    let contract = supplier_bill::create(
        t.db(),
        &BillDraft::new(BLUE_DUNES, "INV-4", day(5))
            .contract(Subject::new("ticketing.contract", 1), None)
            .line(line()),
        Some(A),
    )
    .await
    .unwrap();
    assert!(is_guard(
        submit(t.state(), By::System(Some(A)), contract.id).await
    ));
    for bill in [&empty, &stranger, &usd, &contract] {
        assert_eq!(now(&t, bill).await.status, SupplierBillStatus::Draft);
    }
}

#[renox::test]
async fn the_status_changes_only_through_a_transition() {
    let t = boot().await;
    // Inserted in another state than draft: refused.
    let mut bill = SupplierBill {
        party_id: 1,
        number_key: "x".into(),
        supplier_invoice_number: "X".into(),
        bill_date: day(5),
        currency: "AED".into(),
        status: SupplierBillStatus::Approved,
        ..Default::default()
    };
    assert!(bill.insert(t.db()).await.is_err());
    // A changed status saved outside a transition: refused.
    let mut draft = draft_for(&t, BLUE_DUNES, "INV-1").await;
    draft.status = SupplierBillStatus::Approved;
    assert!(draft.save(t.db()).await.is_err());
    assert_eq!(now(&t, &draft).await.status, SupplierBillStatus::Draft);
}
