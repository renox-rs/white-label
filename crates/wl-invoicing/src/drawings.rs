//! Owner drawings on supplier bills (wl-invoicing §3 *Owner drawings*, §5.16,
//! `FIN.ACL.21`–`23`, §10, `⚠ open: accountant (A-16)`).
//!
//! A supplier bill line whose cost code a budget category of kind
//! `owner_drawings` collects is a **drawing** (`Mapped::is_drawings_code`).
//! The bill's drawing is the sum of those lines in AED with the VAT, which is
//! not recovered (§10 default). Drawings are never operating cost: the lines
//! are left out of expense in budget vs actual ([`crate::budget_actual`]),
//! have no cost object (a draft with one is refused, and `post_cost` refuses
//! the code) and are hidden from users without `drawings.approve`
//! (`FIN.ACL.22`).
//!
//! [`request_for_bill`], run when the bill is submitted, makes the approval of
//! the drawing ([`crate::drawings_machine`]): an approval request to the
//! holders of `drawings.approve` other than the requester; or, when the
//! requester is the entity's only holder and the setting is on
//! (`FIN.ACL.23`), the drawing is recorded at once, flagged *self-approved*,
//! and the accountant role is told; or, at or below a limit raised above
//! AED 0, recorded at once. The bill is approved only after an `approved`
//! approval for the same amount ([`ensure_bill_approved`], run by the bill's
//! `approve`). A rejection, a withdrawal or an expiry approves nothing: the
//! bill stays where it is with the reason shown, until the line is recoded
//! ([`recode_line`], which is logged and shown to whoever approves the bill)
//! or a new request is made for the same line.

use renox::auth::permissions::{Scope, users_with_permission_in};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{StateMachine, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, ApprovalPolicy};
use wl_kernel::notify::{self, NotifyRequest, fill};
use wl_kernel::{LegalEntity, Subject};

use crate::allocate::amount_text;
use crate::bill_line::BillLine;
use crate::budget_actual::Mapped;
use crate::budget_category::{refused, refused_with};
use crate::budget_notices::{DRAWING_CORRECTED, DRAWING_SELF_APPROVED, holders_of};
use crate::config_of;
use crate::drawings_machine::{
    ACTION, DrawingApproval, DrawingStatus, RECORD_BELOW_LIMIT, RECORD_SELF,
    REQUEST, RequestInput, SUBJECT_TYPE,
};
use crate::permissions;
use crate::supplier_bill::{SupplierBill, SupplierBillStatus};
use crate::texts::english;

/// The subject type of a supplier bill.
pub const BILL_TYPE: &str = "invoicing.supplier_bill";

/// A bill line recoded to another cost code.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_invoicing_bill_line_recodings", index(bill_id))]
pub struct BillRecoding {
    /// The primary key.
    pub id: i64,
    /// The bill.
    pub bill_id: i64,
    /// The line.
    pub line_id: i64,
    /// The cost code before.
    pub from_code: Option<String>,
    /// The cost code after.
    pub to_code: Option<String>,
    /// The user who recoded it.
    pub recoded_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The lines of `lines` that are drawings.
pub fn drawing_lines(mapped: &Mapped, lines: Vec<BillLine>) -> Vec<BillLine> {
    lines
        .into_iter()
        .filter(|l| mapped.is_drawings_code(l.cost_code.as_deref()))
        .collect()
}

/// The drawing of `lines` in AED fils: the net of each line with its VAT.
pub fn amount_of(lines: &[BillLine]) -> i64 {
    lines
        .iter()
        .map(|l| l.net_amount_aed.unwrap_or(0) + l.vat_amount_aed.unwrap_or(0))
        .sum()
}

/// The drawing lines of bill `bill_id`, read in `tx`.
pub async fn drawing_lines_in(
    tx: &mut renox::db::Transaction,
    bill_id: i64,
) -> Result<Vec<BillLine>> {
    let mapped = Mapped::load_in(tx).await?;
    let lines = BillLine::query()
        .where_(BillLine::BILL_ID.eq(bill_id))
        .order_by(BillLine::POSITION)
        .get(&mut *tx)
        .await?;
    Ok(drawing_lines(&mapped, lines))
}

/// The drawing lines of bill `bill_id`.
pub async fn drawing_lines_of(db: &Db, bill_id: i64) -> Result<Vec<BillLine>> {
    let mapped = Mapped::load(db).await?;
    let lines = BillLine::query()
        .where_(BillLine::BILL_ID.eq(bill_id))
        .order_by(BillLine::POSITION)
        .get(db)
        .await?;
    Ok(drawing_lines(&mapped, lines))
}

/// Why the bill cannot be approved yet, or `Ok` when it has no drawing or an
/// approved approval for the same amount. Run by `approve` in its
/// transaction.
///
/// # Errors
///
/// 422 saying the drawing needs its approval, is waiting for it, or was
/// rejected and why.
pub async fn ensure_bill_approved(tx: &mut renox::db::Transaction, bill_id: i64) -> Result<()> {
    let drawings = drawing_lines_in(tx, bill_id).await?;
    if drawings.is_empty() {
        return Ok(());
    }
    let amount = amount_of(&drawings);
    let source = Subject::new(BILL_TYPE, bill_id);
    let approvals = DrawingApproval::for_source(&mut *tx, &source).await?;
    if approvals
        .iter()
        .any(|a| a.status == DrawingStatus::Approved && a.amount == amount)
    {
        return Ok(());
    }
    let message = match approvals.first() {
        Some(a) if a.status == DrawingStatus::Requested && a.amount == amount => {
            english("wl_invoicing.drawings.problem.waiting").to_owned()
        }
        Some(a) if a.status == DrawingStatus::Rejected => fill(
            english("wl_invoicing.drawings.problem.rejected"),
            &json!({ "reason": a.rejected_reason.clone().unwrap_or_default() }),
        ),
        _ => english("wl_invoicing.drawings.problem.needed").to_owned(),
    };
    Err(refused(message))
}

/// Asks for the approval of the owner drawing on bill `bill_id`, for the user
/// `requester`. Returns `None` when the bill has no drawing, and the existing
/// approval when one that is not rejected already covers the same amount.
///
/// # Errors
///
/// 422 when the bill is not found, is already approved or voided, or has no
/// receiving entity.
pub async fn request_for_bill(
    state: &AppState,
    bill_id: i64,
    requester: i64,
) -> Result<Option<DrawingApproval>> {
    let bill = SupplierBill::find_or_404(&state.db, bill_id).await?;
    let drawings = drawing_lines_of(&state.db, bill.id).await?;
    if drawings.is_empty() {
        return Ok(None);
    }
    if !matches!(
        bill.status,
        SupplierBillStatus::Draft | SupplierBillStatus::Matched | SupplierBillStatus::Exception
    ) {
        return Err(refused(english("wl_invoicing.drawings.problem.bill_state")));
    }
    let entity = bill
        .legal_entity_id
        .ok_or_else(|| refused(english("wl_invoicing.supplier_bill.problem.entity")))?;
    let amount = amount_of(&drawings);
    let source = bill.subject();
    let earlier = DrawingApproval::for_source(&state.db, &source).await?;
    if let Some(same) = earlier.iter().find(|a| {
        a.amount == amount
            && matches!(
                a.status,
                DrawingStatus::Draft | DrawingStatus::Requested | DrawingStatus::Approved
            )
    }) {
        return Ok(Some(same.clone()));
    }
    let mut approval = DrawingApproval {
        legal_entity_id: entity,
        source_type: source.subject_type.clone(),
        source_id: source.subject_id,
        amount,
        requested_by: Some(requester),
        ..Default::default()
    };
    approval.insert(&state.db).await?;
    let config = config_of(state);
    let holders =
        users_with_permission_in(&state.db, permissions::DRAWINGS_APPROVE, &Scope::of_id::<LegalEntity>(entity))
            .await?;
    let sole = config.drawings_self_approve
        && holders.len() == 1
        && holders.first().is_some_and(|u| u.id == requester);
    let below_limit = ApprovalPolicy::for_action(&state.db, ACTION, &json!({ "amount": amount }))
        .await?
        .is_none();
    let name = if below_limit {
        RECORD_BELOW_LIMIT
    } else if sole {
        RECORD_SELF
    } else {
        REQUEST
    };
    let outcome = approval
        .transition(name)
        .by_system()
        .input(&RequestInput {
            requester_id: requester,
        })
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    approvals::announce(state, &outcome).await?;
    outcome.emit(state).await?;
    let stored = DrawingApproval::find_or_404(&state.db, approval.id).await?;
    if stored.self_approved {
        tell_self_approved(state, &bill, &stored).await?;
    }
    Ok(Some(stored))
}

/// Tells the accountant role that the sole owner recorded a drawing without a
/// second approver (`FIN.ACL.23`).
async fn tell_self_approved(
    state: &AppState,
    bill: &SupplierBill,
    approval: &DrawingApproval,
) -> Result {
    let config = config_of(state);
    let recipients = holders_of(&state.db, std::slice::from_ref(&config.drawings_notify_role)).await?;
    if recipients.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(DRAWING_SELF_APPROVED, approval.subject()).data(json!({
        "bill": bill.supplier_invoice_number,
        "amount": amount_text(&bill.currency, approval.amount),
        "link": format!("/supplier-bills/{}", bill.id),
    }));
    for id in recipients {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Registers the final re-check: a drawing whose amount changed while the
/// request waited fails the request (it is submitted again).
async fn recheck(
    tx: &mut renox::db::Transaction,
    request: &approvals::ApprovalRequest,
) -> Result<Option<String>> {
    let Some(approval) = DrawingApproval::find(&mut *tx, request.subject_id).await? else {
        return Ok(Some(english("wl_invoicing.drawings.problem.not_found").to_owned()));
    };
    let drawings = drawing_lines_in(tx, approval.source_id).await?;
    if amount_of(&drawings) == approval.amount {
        Ok(None)
    } else {
        Ok(Some(
            english("wl_invoicing.drawings.problem.amount_changed").to_owned(),
        ))
    }
}

/// Codes line `line_id` of bill `bill_id` to `cost_code` while the bill is
/// not approved, and logs it.
///
/// # Errors
///
/// 422 when the bill is approved, paid or void, the line is not the bill's,
/// the code is blank, or the new code is owner drawings on a line that has a
/// cost object.
pub async fn recode_line(
    state: &AppState,
    bill_id: i64,
    line_id: i64,
    cost_code: &str,
    by: Option<i64>,
) -> Result<BillLine> {
    let code = cost_code.trim().to_lowercase();
    if code.is_empty() {
        return Err(refused(english("wl_invoicing.drawings.problem.code_blank")));
    }
    let mapped = Mapped::load(&state.db).await?;
    let mut tx = state.db.begin().await?;
    let bill = SupplierBill::find_or_404(&mut tx, bill_id).await?;
    if !matches!(
        bill.status,
        SupplierBillStatus::Draft | SupplierBillStatus::Matched | SupplierBillStatus::Exception
    ) {
        return Err(refused(english("wl_invoicing.drawings.problem.bill_state")));
    }
    let mut line = BillLine::find(&mut tx, line_id)
        .await?
        .filter(|l| l.bill_id == bill.id)
        .ok_or(Error::NotFound)?;
    if mapped.is_drawings_code(Some(&code)) && line.cost_type.is_some() {
        return Err(refused(english(
            "wl_invoicing.drawings.problem.cost_object",
        )));
    }
    let from = line.cost_code.clone();
    line.cost_code = Some(code.clone());
    line.save(&mut tx).await?;
    BillRecoding {
        bill_id: bill.id,
        line_id: line.id,
        from_code: from,
        to_code: Some(code),
        recoded_by: by,
        ..Default::default()
    }
    .insert(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(line)
}

/// The recodings of bill `bill_id`, oldest first.
pub async fn recodings_of(db: &Db, bill_id: i64) -> Result<Vec<BillRecoding>> {
    BillRecoding::query()
        .where_(BillRecoding::BILL_ID.eq(bill_id))
        .order_by(BillRecoding::ID)
        .get(db)
        .await
}

/// The input VAT that is not recovered on owner drawings for `entity`: the
/// VAT of the drawing lines of approved, partly paid and paid supplier bills
/// dated from `from` to `to`, in AED fils (§10; shown as blocked on the VAT
/// summary).
pub async fn blocked_input_vat(
    db: &Db,
    entity: i64,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<i64> {
    let bills = SupplierBill::query()
        .where_(SupplierBill::LEGAL_ENTITY_ID.eq(entity))
        .where_(SupplierBill::STATUS.is_in([
            SupplierBillStatus::Approved,
            SupplierBillStatus::PartiallyPaid,
            SupplierBillStatus::Paid,
        ]))
        .where_(SupplierBill::BILL_DATE.gte(from))
        .where_(SupplierBill::BILL_DATE.lte(to))
        .get(db)
        .await?;
    if bills.is_empty() {
        return Ok(0);
    }
    let mapped = Mapped::load(db).await?;
    let ids: Vec<i64> = bills.iter().map(|b| b.id).collect();
    let lines = BillLine::query()
        .where_(BillLine::BILL_ID.is_in(ids))
        .get(db)
        .await?;
    Ok(drawing_lines(&mapped, lines)
        .iter()
        .map(|l| l.vat_amount_aed.unwrap_or(0))
        .sum())
}

/// Tells the holders of `drawings.approve` that a bill with an approved
/// drawing was voided (`FIN.ACL.21`: a drawing recorded in error is
/// corrected with the bill's own correction).
async fn tell_corrected(state: &AppState, bill_id: i64) -> Result {
    let Some(bill) = SupplierBill::find(&state.db, bill_id).await? else {
        return Ok(());
    };
    let approvals = DrawingApproval::for_source(&state.db, &bill.subject()).await?;
    let Some(approved) = approvals
        .iter()
        .find(|a| a.status == DrawingStatus::Approved)
    else {
        return Ok(());
    };
    let Some(entity) = bill.legal_entity_id else {
        return Ok(());
    };
    let holders = users_with_permission_in(
        &state.db,
        permissions::DRAWINGS_APPROVE,
        &Scope::of_id::<LegalEntity>(entity),
    )
    .await?;
    if holders.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(DRAWING_CORRECTED, approved.subject()).data(json!({
        "bill": bill.supplier_invoice_number,
        "amount": amount_text(&bill.currency, approved.amount),
        "reason": bill.void_reason.clone().unwrap_or_default(),
        "link": format!("/supplier-bills/{}", bill.id),
    }));
    for user in holders {
        request = request.to_user(user.id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Registers the final re-check and the notice of a corrected drawing.
pub(crate) fn register(app: &mut Registry) {
    approvals::register_final_check(SUBJECT_TYPE, |tx, request| Box::pin(recheck(tx, request)));
    app.listen(
        |event: wl_kernel::invoicing::SupplierBillVoided, state: AppState| async move {
            tell_corrected(&state, event.bill.0).await
        },
    );
}

/// The refusal for a drawing line that has a cost object, for the bill form.
pub(crate) fn cost_object_refusal(position: usize) -> Error {
    refused_with(
        "wl_invoicing.drawings.problem.line_cost_object",
        &json!({ "line": position }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget_category::{BudgetKind, MappingKind};

    fn mapped() -> Mapped {
        let mut mapped = Mapped::default();
        mapped.insert_for_test(MappingKind::CostCode, "owner_drawings", 2, BudgetKind::OwnerDrawings);
        mapped.insert_for_test(MappingKind::CostCode, "marketing", 1, BudgetKind::Expense);
        mapped
    }

    fn line(code: &str, net: i64, vat: i64) -> BillLine {
        BillLine {
            cost_code: Some(code.to_owned()),
            net_amount_aed: Some(net),
            vat_amount_aed: Some(vat),
            ..Default::default()
        }
    }

    #[test]
    fn only_the_lines_coded_to_drawings_are_drawings() {
        let lines = vec![
            line("marketing", 100_000, 5_000),
            line("owner_drawings", 200_000, 10_000),
            line("owner_drawings", 50_000, 0),
            BillLine::default(),
        ];
        let drawings = drawing_lines(&mapped(), lines);
        assert_eq!(drawings.len(), 2);
        // The VAT is not recovered: it stays in the drawing.
        assert_eq!(amount_of(&drawings), 260_000);
    }

    #[test]
    fn a_line_with_an_unknown_rate_counts_as_nothing() {
        let unpriced = BillLine {
            cost_code: Some("owner_drawings".into()),
            ..Default::default()
        };
        assert_eq!(amount_of(&[unpriced]), 0);
    }
}
