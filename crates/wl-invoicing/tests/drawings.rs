//! Owner drawings (wl-invoicing §3 *Owner drawings*, §5.16, `FIN.ACL.21` to
//! `FIN.ACL.23`): the acceptance checks of #90. The checks of expense claims
//! need the wl-payments claim lifecycle, which this build does not have; a
//! supplier bill carries the drawing here.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::bill_machine::{approve, void};
use wl_invoicing::bill_submit::submit;
use wl_invoicing::budget_category::{self, CategoryInput};
use wl_invoicing::budget_report::{self, ReportQuery};
use wl_invoicing::credit_notes::By;
use wl_invoicing::supplier_bill::{self, BillDraft};
use wl_invoicing::vat_summary::VatSummaryStatus;
use wl_invoicing::{
    BillLineDraft, BudgetKind, DrawingApproval, DrawingStatus, MappingKind, SupplierBill,
    SupplierBillStatus, WlInvoicing, permissions as perm,
};
use wl_kernel::approvals::{self, ApprovalPolicy, ApprovalStatus};
use wl_kernel::invoicing::{PostCost, VatCategory, post_cost};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::{LegalEntity, Subject};

use support::{BLUE_DUNES, Booted, boot, boot_with};

const SELF_APPROVED: &str = "invoicing.drawing_self_approved";
const CORRECTED: &str = "invoicing.drawing_corrected";

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2027, m, d).unwrap()
}

/// An accountant: enters, submits and approves supplier bills.
async fn accountant(t: &Booted, name: &str) -> User {
    t.helper(
        name,
        &[
            perm::VIEW,
            perm::SUPPLIER_BILLS_VIEW,
            perm::SUPPLIER_BILLS_CREATE,
            perm::SUPPLIER_BILLS_APPROVE,
            perm::BUDGETS_VIEW,
            perm::VAT_VIEW,
        ],
    )
    .await
}

/// An owner: holds `drawings.approve`.
async fn owner(t: &Booted, name: &str) -> User {
    t.helper(
        name,
        &[
            perm::SUPPLIER_BILLS_VIEW,
            perm::DRAWINGS_APPROVE,
            perm::BUDGETS_VIEW,
        ],
    )
    .await
}

/// A draft bill of Blue Dunes to `entity` with the lines
/// `(description, price in fils, cost code, VAT category)`, entered by `by`.
async fn draft_bill(
    t: &Booted,
    entity: &LegalEntity,
    number: &str,
    lines: &[(&str, i64, &str, VatCategory)],
    by: &User,
) -> SupplierBill {
    let mut draft = BillDraft::new(BLUE_DUNES, number, day(3, 15));
    draft.legal_entity_id = Some(entity.id);
    for (description, price, code, category) in lines {
        draft = draft.line(BillLineDraft::new(description, *price, *category).cost_code(code));
    }
    supplier_bill::create(t.db(), &draft, Some(by.id))
        .await
        .unwrap()
}

/// The bill as stored now.
async fn bill_now(t: &Booted, bill: &SupplierBill) -> SupplierBill {
    SupplierBill::find(t.db(), bill.id).await.unwrap().unwrap()
}

/// The approvals of the drawing on `bill`, newest first.
async fn drawings_of(t: &Booted, bill: &SupplierBill) -> Vec<DrawingApproval> {
    DrawingApproval::for_source(t.db(), &bill.subject())
        .await
        .unwrap()
}

/// A 422 with a message that holds `word`.
fn assert_refused<T: std::fmt::Debug>(result: Result<T>, word: &str) {
    match result {
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)) => {
            assert!(message.contains(word), "{message}");
        }
        other => panic!("expected a refusal with `{word}`, got {other:?}"),
    }
}

/// Whether approving the bill is refused with a message that holds `word`.
async fn approve_refused(t: &Booted, bill: &SupplierBill, by: &User, word: &str) {
    let result = approve(t.state(), By::System(Some(by.id)), bill.id, "")
        .await
        .map_err(|e| e.into_error());
    assert_refused(result, word);
    assert_eq!(bill_now(t, bill).await.status, SupplierBillStatus::Matched);
}

/// The users `recorded` notices of one type went to.
fn told(t: &Booted, kind: &str) -> Vec<Vec<i64>> {
    t.notices
        .of_type(kind)
        .iter()
        .map(|request| {
            let mut ids: Vec<i64> = request
                .recipients
                .iter()
                .filter_map(|r| match r {
                    NotifyRecipient::User { id } => Some(*id),
                    _ => None,
                })
                .collect();
            ids.sort_unstable();
            ids
        })
        .collect()
}

/// Given accountant A submits supplier bill SB-90 with a line of AED 2,000
/// coded to *Owner drawings*, the bill cannot be approved until owner B
/// (holder of `drawings.approve`) approves the drawing; A cannot approve it.
/// After the approval SB-90 is approved; no cost is posted; the line appears
/// on the owner drawings line of budget vs actual. The VAT on the drawings
/// line is shown as blocked in the VAT summary.
#[renox::test]
async fn a_bill_with_a_drawing_waits_for_the_owner() {
    let t = boot().await;
    let e1 = t.entity("PGP").await;
    let a = accountant(&t, "Amal").await;
    let b = owner(&t, "Bilal").await;
    let bill = draft_bill(
        &t,
        &e1,
        "SB-90",
        &[
            ("Office chairs", 100_000, "other", VatCategory::Standard),
            (
                "Villa decorator",
                200_000,
                "owner_drawings",
                VatCategory::Standard,
            ),
        ],
        &a,
    )
    .await;
    t.post_as(&format!("/supplier-bills/{}/submit", bill.id), &a, &[])
        .await
        .assert_redirect(&format!("/supplier-bills/{}", bill.id));
    assert_eq!(
        bill_now(&t, &bill).await.status,
        SupplierBillStatus::Matched
    );

    // The drawing: AED 2,000 plus the 5% VAT that is not recovered.
    let drawing = drawings_of(&t, &bill).await.remove(0);
    assert_eq!(drawing.status, DrawingStatus::Requested);
    assert_eq!(drawing.amount, 210_000);
    assert_eq!(drawing.requested_by, Some(a.id));
    assert!(!drawing.self_approved);
    let request_id = drawing.approval_request_id.expect("an approval request");
    let request = wl_kernel::approvals::ApprovalRequest::find(t.db(), request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.action, "invoicing.owner_drawing");
    assert_eq!(request.status, ApprovalStatus::Pending);
    assert_eq!(t.notices_of(&b, "approval-requested").await.len(), 1);

    // Until the drawing is approved the bill is not, whoever asks.
    approve_refused(&t, &bill, &a, "waits for the approval").await;
    t.post_as(
        &format!("/supplier-bills/{}/transitions/approve", bill.id),
        &a,
        &[],
    )
    .await
    .assert_status(422);
    assert!(
        approvals::approve(t.state(), request_id, &a, "")
            .await
            .is_err()
    );

    approvals::approve(t.state(), request_id, &b, "")
        .await
        .unwrap()
        .expect("the approval runs");
    let approved = drawings_of(&t, &bill).await.remove(0);
    assert_eq!(approved.status, DrawingStatus::Approved);
    assert_eq!(approved.approved_by, Some(b.id));
    assert!(!approved.self_approved);
    let done = approve(t.state(), By::System(Some(a.id)), bill.id, "")
        .await
        .unwrap();
    assert_eq!(done.status, SupplierBillStatus::Approved);

    // No cost object, and the line is on the drawings line of the report.
    let report = budget_report::build(
        t.db(),
        &t.config(),
        &ReportQuery {
            entities: vec![e1.id],
            month: day(3, 1),
            ytd: false,
            drawings_lines: true,
            uncategorised: "Uncategorised".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(report.drawings_total().1, 210_000);
    assert_eq!(report.expense_total().1, 100_000);
    assert_eq!(report.drawings_items.len(), 1);
    assert!(!report.drawings_items[0].self_approved);

    // The VAT of the drawing is blocked in the VAT summary.
    let summary = t.summary(&e1, day(3, 15), VatSummaryStatus::Open).await;
    t.app.acting_as(&a);
    t.app
        .get(&format!("/vat-summaries/{}", summary.id))
        .await
        .assert_ok()
        .assert_see("Input VAT blocked on owner drawings")
        .assert_see("AED 100.00");
}

/// A drawings line with a cost object is refused, and a `CostPoster` call for
/// a drawings cost code is refused with a clear error.
#[renox::test]
async fn a_drawing_is_never_posted_to_a_cost_object() {
    let t = boot().await;
    let e1 = t.entity("PGP").await;
    let mut draft = BillDraft::new(BLUE_DUNES, "SB-1", day(3, 1));
    draft.legal_entity_id = Some(e1.id);
    let draft = draft.line(
        BillLineDraft::new("Personal", 10_000, VatCategory::Exempt)
            .cost_code("owner_drawings")
            .cost_object(Subject::new("property.unit", 4)),
    );
    assert_refused(
        supplier_bill::create(t.db(), &draft, Some(1)).await,
        "never posted to a cost object",
    );
    // The same line coded to a business cost code is fine.
    let ok = BillDraft::new(BLUE_DUNES, "SB-2", day(3, 1)).line(
        BillLineDraft::new("Repair", 10_000, VatCategory::Exempt)
            .cost_code("other")
            .cost_object(Subject::new("property.unit", 4)),
    );
    supplier_bill::create(t.db(), &ok, Some(1)).await.unwrap();

    let request = |code: &str| {
        PostCost::new(
            Subject::new("invoicing.supplier_bill", 1),
            "invoicing.supplier_bill_line:1",
            Subject::new("property.unit", 4),
            10_000,
        )
        .cost_code(code)
    };
    assert_refused(
        post_cost(t.state(), request("owner_drawings")).await,
        "owner drawings",
    );
    post_cost(t.state(), request("repairs")).await.unwrap();
}

/// B rejects the drawing on SB-91; SB-91 cannot be approved until A recodes
/// the line to *Other expense*; the approver of SB-91 sees the rejection and
/// the recoding. A line recoded to another drawings category needs a new
/// request.
#[renox::test]
async fn a_rejected_drawing_blocks_the_bill_until_the_line_is_recoded() {
    let t = boot().await;
    let e1 = t.entity("PGP").await;
    let a = accountant(&t, "Amal").await;
    let b = owner(&t, "Bilal").await;
    let bill = draft_bill(
        &t,
        &e1,
        "SB-91",
        &[(
            "Weekend villa",
            300_000,
            "owner_drawings",
            VatCategory::Exempt,
        )],
        &a,
    )
    .await;
    t.post_as(&format!("/supplier-bills/{}/submit", bill.id), &a, &[])
        .await
        .assert_redirect(&format!("/supplier-bills/{}", bill.id));
    let first = drawings_of(&t, &bill).await.remove(0);
    approvals::reject(
        t.state(),
        first.approval_request_id.unwrap(),
        &b,
        "this is personal",
    )
    .await
    .unwrap();
    let rejected = drawings_of(&t, &bill).await.remove(0);
    assert_eq!(rejected.status, DrawingStatus::Rejected);
    assert_eq!(
        rejected.rejected_reason.as_deref(),
        Some("this is personal")
    );
    approve_refused(&t, &bill, &a, "this is personal").await;

    // A recodes the line to another drawings category: a new request is
    // needed, and made on demand.
    let partner = budget_category::save(
        t.db(),
        None,
        CategoryInput {
            code: "partner_c".into(),
            name_en: "Partner C drawings".into(),
            kind: BudgetKind::OwnerDrawings,
            owner_party_id: Some(3),
            mappings: vec![(MappingKind::CostCode, "partner_c".into())],
            active: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(partner.kind, BudgetKind::OwnerDrawings);
    let line = supplier_bill::lines_of(t.db(), bill.id)
        .await
        .unwrap()
        .remove(0);
    let line_id = line.id.to_string();
    t.post_as(
        &format!("/supplier-bills/{}/recode", bill.id),
        &a,
        &[("line_id", line_id.as_str()), ("cost_code", "partner_c")],
    )
    .await
    .assert_ok();
    approve_refused(&t, &bill, &a, "not approved").await;
    t.post_as(
        &format!("/supplier-bills/{}/drawings/request", bill.id),
        &a,
        &[],
    )
    .await
    .assert_redirect(&format!("/supplier-bills/{}", bill.id));
    let again = drawings_of(&t, &bill).await;
    assert_eq!(again.len(), 2);
    assert_eq!(again[0].status, DrawingStatus::Requested);
    // Asking again while it waits changes nothing.
    t.post_as(
        &format!("/supplier-bills/{}/drawings/request", bill.id),
        &a,
        &[],
    )
    .await
    .assert_redirect(&format!("/supplier-bills/{}", bill.id));
    assert_eq!(drawings_of(&t, &bill).await.len(), 2);
    approvals::reject(
        t.state(),
        again[0].approval_request_id.unwrap(),
        &b,
        "still personal",
    )
    .await
    .unwrap();

    // A recodes it to Other expense: the bill can be approved.
    t.post_as(
        &format!("/supplier-bills/{}/recode", bill.id),
        &a,
        &[("line_id", line_id.as_str()), ("cost_code", "other")],
    )
    .await
    .assert_ok();
    let done = approve(t.state(), By::System(Some(a.id)), bill.id, "")
        .await
        .unwrap();
    assert_eq!(done.status, SupplierBillStatus::Approved);

    // Whoever approves the bill sees the rejection and the recoding.
    t.app.acting_as(&a);
    t.app
        .get(&format!("/supplier-bills/{}", bill.id))
        .await
        .assert_ok()
        .assert_see("still personal")
        .assert_see("Recoded lines")
        .assert_see("partner_c")
        .assert_see("other");
    let log = wl_invoicing::drawings::recodings_of(t.db(), bill.id)
        .await
        .unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].from_code.as_deref(), Some("owner_drawings"));
    assert_eq!(log[1].to_code.as_deref(), Some("other"));
}

/// In entity E4 whose only holder of `drawings.approve` is owner O, O's bill
/// with a drawings line is approved without a second approver, flagged
/// *self-approved*, and the accountant is notified. With the setting off the
/// drawing waits until a second holder exists.
#[renox::test]
async fn the_sole_owner_records_their_own_drawing_and_the_accountant_is_told() {
    let t = boot().await;
    let e4 = t.new_entity("E4").await;
    let accountant_user = t.accountant("Aya").await;
    // Omar holds `drawings.approve` in E4 only, and enters the bill himself.
    let omar = t
        .scoped_user("Omar", &[perm::DRAWINGS_APPROVE], e4.id)
        .await;
    let bill = draft_bill(
        &t,
        &e4,
        "SB-E4",
        &[(
            "Family holiday",
            500_000,
            "owner_drawings",
            VatCategory::Exempt,
        )],
        &omar,
    )
    .await;
    // The requester is the only holder of the entity: recorded at once.
    let outcome = submit(t.state(), By::System(Some(omar.id)), bill.id)
        .await
        .unwrap();
    assert_eq!(outcome.status, SupplierBillStatus::Matched);
    let drawing = drawings_of(&t, &bill).await.remove(0);
    assert_eq!(drawing.status, DrawingStatus::Approved);
    assert!(drawing.self_approved);
    assert_eq!(drawing.approved_by, Some(omar.id));
    assert_eq!(drawing.approval_request_id, None);
    assert_eq!(told(&t, SELF_APPROVED), [vec![accountant_user.id]]);
    let done = approve(t.state(), By::System(Some(omar.id)), bill.id, "")
        .await
        .unwrap();
    assert_eq!(done.status, SupplierBillStatus::Approved);

    // The report flags the line for the owner.
    let report = budget_report::build(
        t.db(),
        &t.config(),
        &ReportQuery {
            entities: vec![e4.id],
            month: day(3, 1),
            ytd: false,
            drawings_lines: true,
            uncategorised: "Uncategorised".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(report.drawings_items.len(), 1);
    assert!(report.drawings_items[0].self_approved);
}

/// With the `FIN.ACL.23` setting off, a sole owner's drawing waits for a
/// second holder, who approves it.
#[renox::test]
async fn without_the_setting_the_sole_owner_waits_for_a_second_holder() {
    let t = boot_with(WlInvoicing::new().drawings_self_approve(false)).await;
    let e4 = t.new_entity("E4").await;
    let omar = t
        .scoped_user(
            "Omar",
            &[perm::DRAWINGS_APPROVE, perm::SUPPLIER_BILLS_VIEW],
            e4.id,
        )
        .await;
    let bill = draft_bill(
        &t,
        &e4,
        "SB-E4",
        &[(
            "Family holiday",
            500_000,
            "owner_drawings",
            VatCategory::Exempt,
        )],
        &omar,
    )
    .await;
    submit(t.state(), By::System(Some(omar.id)), bill.id)
        .await
        .unwrap();
    let drawing = drawings_of(&t, &bill).await.remove(0);
    assert_eq!(drawing.status, DrawingStatus::Requested);
    assert!(!drawing.self_approved);
    assert!(t.notices.of_type(SELF_APPROVED).is_empty());
    let request_id = drawing.approval_request_id.unwrap();
    let request = wl_kernel::approvals::ApprovalRequest::find(t.db(), request_id)
        .await
        .unwrap()
        .unwrap();
    let nobody = approvals::notify::step_approvers(t.db(), &request)
        .await
        .unwrap();
    assert!(nobody.is_empty());
    assert!(
        approvals::approve(t.state(), request_id, &omar, "")
            .await
            .is_err()
    );

    let partner = t.scoped_user("Pia", &[perm::DRAWINGS_APPROVE], e4.id).await;
    approvals::approve(t.state(), request_id, &partner, "")
        .await
        .unwrap()
        .expect("the approval runs");
    let approved = drawings_of(&t, &bill).await.remove(0);
    assert_eq!(approved.status, DrawingStatus::Approved);
    assert!(!approved.self_approved);
}

/// A user without `drawings.approve` sees the drawings total on the bill but
/// not the drawings line; a holder sees the line.
#[renox::test]
async fn the_drawings_line_of_a_bill_is_hidden_from_other_users() {
    let t = boot().await;
    let e1 = t.entity("PGP").await;
    let a = accountant(&t, "Amal").await;
    let b = owner(&t, "Bilal").await;
    let bill = draft_bill(
        &t,
        &e1,
        "SB-90",
        &[
            ("Office chairs", 100_000, "other", VatCategory::Exempt),
            (
                "Villa decorator",
                200_000,
                "owner_drawings",
                VatCategory::Exempt,
            ),
        ],
        &a,
    )
    .await;
    t.app.acting_as(&a);
    t.app
        .get(&format!("/supplier-bills/{}", bill.id))
        .await
        .assert_ok()
        .assert_see("Office chairs")
        .assert_see("Owner drawing (shown to holders of drawings.approve)")
        .assert_dont_see("Villa decorator")
        .assert_see("2,000.00");
    t.app.acting_as(&b);
    t.app
        .get(&format!("/supplier-bills/{}", bill.id))
        .await
        .assert_ok()
        .assert_see("Villa decorator");
    // The approver reaches the bill from the approval.
    submit(t.state(), By::System(Some(a.id)), bill.id)
        .await
        .unwrap();
    let drawing = drawings_of(&t, &bill).await.remove(0);
    t.app
        .get(&format!("/drawing-approvals/{}", drawing.id))
        .await
        .assert_redirect(&format!("/supplier-bills/{}", bill.id));
}

/// A limit raised above AED 0 skips the approval for a drawing at or below
/// it; above it the approval is needed again.
#[renox::test]
async fn a_limit_above_zero_records_a_small_drawing_at_once() {
    let t = boot().await;
    let e1 = t.entity("PGP").await;
    let a = accountant(&t, "Amal").await;
    owner(&t, "Bilal").await;
    ApprovalPolicy::query()
        .where_(ApprovalPolicy::ACTION.eq("invoicing.owner_drawing"))
        .update(t.db(), &[("threshold", &250_000_i64)])
        .await
        .unwrap();
    let small = draft_bill(
        &t,
        &e1,
        "SB-S",
        &[("Parking", 200_000, "owner_drawings", VatCategory::Exempt)],
        &a,
    )
    .await;
    submit(t.state(), By::System(Some(a.id)), small.id)
        .await
        .unwrap();
    let drawing = drawings_of(&t, &small).await.remove(0);
    assert_eq!(drawing.status, DrawingStatus::Approved);
    assert!(!drawing.self_approved);
    assert_eq!(drawing.approved_by, None);
    approve(t.state(), By::System(Some(a.id)), small.id, "")
        .await
        .unwrap();

    let large = draft_bill(
        &t,
        &e1,
        "SB-L",
        &[("Holiday", 300_000, "owner_drawings", VatCategory::Exempt)],
        &a,
    )
    .await;
    submit(t.state(), By::System(Some(a.id)), large.id)
        .await
        .unwrap();
    let drawing = drawings_of(&t, &large).await.remove(0);
    assert_eq!(drawing.status, DrawingStatus::Requested);
}

/// A drawing whose amount changes while its request waits fails at the last
/// yes: nothing is approved and it is asked for again.
#[renox::test]
async fn a_changed_amount_fails_the_waiting_request() {
    let t = boot().await;
    let e1 = t.entity("PGP").await;
    let a = accountant(&t, "Amal").await;
    let b = owner(&t, "Bilal").await;
    let bill = draft_bill(
        &t,
        &e1,
        "SB-92",
        &[
            ("Holiday", 300_000, "owner_drawings", VatCategory::Exempt),
            ("Phone", 50_000, "other", VatCategory::Exempt),
        ],
        &a,
    )
    .await;
    submit(t.state(), By::System(Some(a.id)), bill.id)
        .await
        .unwrap();
    let waiting = drawings_of(&t, &bill).await.remove(0);
    let lines = supplier_bill::lines_of(t.db(), bill.id).await.unwrap();
    // The second line becomes a drawing too: the amount is no longer 3,000.
    wl_invoicing::drawings::recode_line(
        t.state(),
        bill.id,
        lines[1].id,
        "owner_drawings",
        Some(a.id),
    )
    .await
    .unwrap();
    approvals::approve(t.state(), waiting.approval_request_id.unwrap(), &b, "")
        .await
        .unwrap();
    let failed = drawings_of(&t, &bill).await.remove(0);
    assert_eq!(failed.id, waiting.id);
    assert_eq!(failed.status, DrawingStatus::Rejected);
    assert!(
        failed
            .rejected_reason
            .unwrap_or_default()
            .contains("changed")
    );
    approve_refused(&t, &bill, &a, "Recode or remove").await;
    // Asked again for the new amount, it can be approved.
    let fresh = wl_invoicing::drawings::request_for_bill(t.state(), bill.id, a.id)
        .await
        .unwrap()
        .expect("a new approval");
    assert_eq!(fresh.amount, 350_000);
    approvals::approve(t.state(), fresh.approval_request_id.unwrap(), &b, "")
        .await
        .unwrap();
    let done = approve(t.state(), By::System(Some(a.id)), bill.id, "")
        .await
        .unwrap();
    assert_eq!(done.status, SupplierBillStatus::Approved);
}

/// A bill with an approved drawing that is voided as an error tells the
/// holders of `drawings.approve`.
#[renox::test]
async fn voiding_a_bill_with_an_approved_drawing_tells_the_holders() {
    let t = boot().await;
    let e1 = t.entity("PGP").await;
    let a = accountant(&t, "Amal").await;
    let b = owner(&t, "Bilal").await;
    let bill = draft_bill(
        &t,
        &e1,
        "SB-93",
        &[("Holiday", 300_000, "owner_drawings", VatCategory::Exempt)],
        &a,
    )
    .await;
    submit(t.state(), By::System(Some(a.id)), bill.id)
        .await
        .unwrap();
    let drawing = drawings_of(&t, &bill).await.remove(0);
    approvals::approve(t.state(), drawing.approval_request_id.unwrap(), &b, "")
        .await
        .unwrap();
    void(
        t.state(),
        By::System(Some(a.id)),
        bill.id,
        "entered in error",
    )
    .await
    .unwrap();
    assert_eq!(told(&t, CORRECTED), [vec![b.id]]);
}
