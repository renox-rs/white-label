//! The actions of the field PWA and the back office on goods receipts
//! (wl-inventory §4 *Goods receipt*, §5.1 step 4, §5.6):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /goods-receipts/{id}/lines/{line}` | `wl_inventory.receipt_line` | `goods_receipts.post` at the receiving location |
//! | `POST /goods-receipts/{id}/post` | `wl_inventory.receipt_post` | `goods_receipts.post` at the receiving location |
//! | `POST /supplier-returns/{id}/dispatch` | `wl_inventory.return_dispatch` | `supplier_returns.manage` |
//! | `POST /supplier-returns/{id}/credit` | `wl_inventory.return_credit` | `supplier_returns.manage` |
//! | `POST /supplier-returns/{id}/cancel` | `wl_inventory.return_cancel` | `supplier_returns.manage` |
//! | `POST /goods-receipts/{id}/reverse` | `wl_inventory.receipt_reverse` | `goods_receipts.reverse` at the receiving location |
//!
//! The routes need a signed-in user and carry no permission guard of their
//! own: the guard counts global roles only, while a storekeeper may hold the
//! permission at one site store. The handlers check it in the scope of the
//! receipt and answer 403. A quantity is sent in the base unit of the item,
//! like `80` or `12.5`. The screens of the field PWA come later; the receipt
//! and its lines are recorded with [`crate::goods_receipt::create_draft`].

use renox::auth::permissions::users_with_permission_in;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};

use crate::goods_receipt::{DraftLine, GoodsReceipt, GoodsReceiptStatus, update_line};
use crate::permissions;
use crate::receipt_post::{PostOutcome, post};
use crate::return_credit::{CreditBasis, CreditOutcome, KeptGoods};
use crate::return_dispatch::{CollectionProof, DispatchOutcome};
use crate::supplier_return::{RejectedHandling, WantedOutcome};
use crate::texts;

/// The routes of the module.
pub fn routes() -> Routes {
    Routes::new()
        .post("/goods-receipts/{id}/lines/{line}", save_line)
        .name("wl_inventory.receipt_line")
        .post("/goods-receipts/{id}/post", post_receipt)
        .name("wl_inventory.receipt_post")
        .post("/goods-receipts/{id}/reverse", reverse_receipt)
        .name("wl_inventory.receipt_reverse")
        .post("/supplier-returns/{id}/dispatch", dispatch_return)
        .name("wl_inventory.return_dispatch")
        .post("/supplier-returns/{id}/credit", credit_return)
        .name("wl_inventory.return_credit")
        .post("/supplier-returns/{id}/cancel", cancel_return)
        .name("wl_inventory.return_cancel")
        .require_auth()
}

/// The package text `key`.
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_inventory.{key}"))
}

/// `text` ("80", "12.5", "0.125") as thousandths of the base unit; `None` for
/// anything else, a negative number or more than three decimals.
pub fn parse_quantity(text: &str) -> Option<i64> {
    let text = text.trim().replace(',', "");
    if text.is_empty() {
        return None;
    }
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (text.as_str(), ""),
    };
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if !digits(whole) || !digits(fraction) || fraction.len() > 3 {
        return None;
    }
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let fraction: i64 = format!("{fraction:0<3}").parse().ok()?;
    whole.checked_mul(1_000)?.checked_add(fraction)
}

/// Whether the user may post at the receipt's location.
async fn may_post(state: &AppState, user: &AuthUser, receipt: &GoodsReceipt) -> Result<bool> {
    Ok(users_with_permission_in(
        &state.db,
        permissions::GOODS_RECEIPTS_POST,
        &receipt.scope(),
    )
    .await?
    .iter()
    .any(|holder| holder.id == user.id))
}

/// What the storekeeper enters for one line.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct LineForm {
    /// What arrived.
    #[validate(max = 32)]
    pub received: Option<String>,
    /// What is accepted.
    #[validate(max = 32)]
    pub accepted: Option<String>,
    /// What is rejected.
    #[validate(max = 32)]
    pub rejected: Option<String>,
    /// Why the rejected quantity was refused.
    #[validate(max = 64)]
    pub reason: Option<String>,
    /// `refused_at_door` or `kept_for_collection`.
    #[validate(max = 32)]
    pub handling: Option<String>,
    /// `replace` (the default) or `credit`.
    #[validate(max = 16)]
    pub outcome: Option<String>,
    /// The lot number.
    #[validate(max = 64)]
    pub lot: Option<String>,
    /// The expiry date, like `2027-01-31`.
    #[validate(max = 16)]
    pub expiry: Option<String>,
    /// A reading or the reference of a photo.
    #[validate(max = 500)]
    pub evidence: Option<String>,
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The quantity in `field`, 0 when empty.
fn quantity(field: &Option<String>, name: &str) -> Result<i64> {
    match field.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(0),
        Some(value) => {
            parse_quantity(value).ok_or_else(|| refused(format!("{name}: not a quantity")))
        }
    }
}

impl LineForm {
    /// The line the form enters.
    fn draft(&self) -> Result<DraftLine> {
        let received = quantity(&self.received, "received")?;
        let accepted = quantity(&self.accepted, "accepted")?;
        let rejected = quantity(&self.rejected, "rejected")?;
        let handling = match self
            .handling
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            None => None,
            Some(value) => Some(
                RejectedHandling::ALL
                    .iter()
                    .copied()
                    .find(|h| h.as_str() == value)
                    .ok_or_else(|| refused("handling: refused_at_door or kept_for_collection"))?,
            ),
        };
        let wanted = match self
            .outcome
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            None => WantedOutcome::Replace,
            Some(value) => WantedOutcome::ALL
                .iter()
                .copied()
                .find(|o| o.as_str() == value)
                .ok_or_else(|| refused("outcome: replace or credit"))?,
        };
        let expiry = match self
            .expiry
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            None => None,
            Some(value) => Some(
                NaiveDate::parse_from_str(value, "%Y-%m-%d")
                    .map_err(|_| refused("expiry: a date like 2027-01-31"))?,
            ),
        };
        Ok(DraftLine {
            po_line_id: None,
            return_line_id: None,
            received_qty_milli: received,
            accepted_qty_milli: accepted,
            rejected_qty_milli: rejected,
            rejection_reason: self.reason.clone(),
            rejected_handling: handling,
            wanted_outcome: wanted,
            lot_number: self.lot.clone(),
            expiry_date: expiry,
            evidence: self.evidence.clone(),
        })
    }
}

/// `POST /goods-receipts/{id}/lines/{line}`: the storekeeper enters the
/// received, accepted and rejected quantity of a line of a draft receipt,
/// with the reason and what became of the rejected goods.
async fn save_line(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((id, line)): Path<(i64, i64)>,
    Valid(form): Valid<LineForm>,
) -> Result<Response> {
    let receipt = GoodsReceipt::find_or_404(&state.db, id).await?;
    if !may_post(&state, &user, &receipt).await? {
        return Err(Error::Forbidden);
    }
    let mut draft = form.draft()?;
    // The order line stays the line's own.
    let stored = crate::goods_receipt::GoodsReceiptLine::find_or_404(&state.db, line).await?;
    draft.po_line_id = stored.po_line_id;
    draft.return_line_id = stored.return_line_id;
    update_line(&state.db, receipt.id, line, &draft).await?;
    Ok((
        Toast::success(text(&lang, "receipt.line_saved")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /goods-receipts/{id}/post`: posts the receipt. Above the tolerance
/// it asks for the approval of a holder of `purchase_orders.issue` instead
/// (`STK.ACL.04`).
async fn post_receipt(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let receipt = GoodsReceipt::find_or_404(&state.db, id).await?;
    if !may_post(&state, &user, &receipt).await? {
        return Err(Error::Forbidden);
    }
    if receipt.status != GoodsReceiptStatus::Draft {
        return Err(refused(text(&lang, "receipt.not_draft")));
    }
    match post(&state, &user, id).await {
        Ok(PostOutcome::Posted(_)) => Ok((
            Toast::success(text(&lang, "receipt.posted")),
            renox::HxRefresh,
        )
            .into_response()),
        Ok(PostOutcome::Waiting(_)) => Ok((
            Toast::success(text(&lang, "receipt.waiting")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Err(refused(message)),
        Err(other) => Err(other.into_error()),
    }
}

/// `POST /goods-receipts/{id}/reverse`: reverses a posted receipt. Refused
/// with the line and the missing quantity, or with the return that has left.
async fn reverse_receipt(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let receipt = GoodsReceipt::find_or_404(&state.db, id).await?;
    let holders = users_with_permission_in(
        &state.db,
        permissions::GOODS_RECEIPTS_REVERSE,
        &receipt.scope(),
    )
    .await?;
    if !holders.iter().any(|holder| holder.id == user.id) {
        return Err(Error::Forbidden);
    }
    match crate::receipt_reverse::reverse(&state, &user, id).await {
        Ok(_) => Ok((
            Toast::success(text(&lang, "receipt.reversed")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Err(refused(message)),
        Err(other) => Err(other.into_error()),
    }
}

/// What procurement enters to dispatch a return.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct DispatchForm {
    /// `driver_signature`, `courier` or `goods_receipt`.
    #[validate(max = 32)]
    pub proof_kind: Option<String>,
    /// The reference of the signature, the courier's number or the receipt.
    #[validate(max = 200)]
    pub proof_ref: Option<String>,
}

/// `POST /supplier-returns/{id}/dispatch`: dispatches a draft return with the
/// collection proof. Above the limit it asks a second person to approve
/// instead (`STK.ACL.06`).
async fn dispatch_return(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<DispatchForm>,
) -> Result<Response> {
    let holders = users_with_permission_in(
        &state.db,
        permissions::SUPPLIER_RETURNS_MANAGE,
        &renox::auth::permissions::Scope::global(),
    )
    .await?;
    if !holders.iter().any(|holder| holder.id == user.id) {
        return Err(Error::Forbidden);
    }
    let proof = CollectionProof::parse(
        form.proof_kind.as_deref().unwrap_or_default(),
        form.proof_ref.as_deref().unwrap_or_default(),
    )
    .ok_or_else(|| refused(text(&lang, "return.bad_proof")))?;
    match crate::return_dispatch::dispatch(&state, &user, id, &proof).await {
        Ok(DispatchOutcome::Dispatched(_)) => Ok((
            Toast::success(text(&lang, "return.dispatched")),
            renox::HxRefresh,
        )
            .into_response()),
        Ok(DispatchOutcome::Waiting(_)) => Ok((
            Toast::success(text(&lang, "return.waiting")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Err(refused(message)),
        Err(other) => Err(other.into_error()),
    }
}

/// Whether `user` holds `supplier_returns.manage`.
async fn may_manage_returns(state: &AppState, user: &AuthUser) -> Result<bool> {
    Ok(users_with_permission_in(
        &state.db,
        permissions::SUPPLIER_RETURNS_MANAGE,
        &renox::auth::permissions::Scope::global(),
    )
    .await?
    .iter()
    .any(|holder| holder.id == user.id))
}

/// What procurement enters to credit a dispatched return by hand.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct CreditForm {
    /// `refund_received` or `supplier_refused`.
    #[validate(max = 32)]
    pub basis: Option<String>,
    /// Why.
    #[validate(max = 500)]
    pub reason: Option<String>,
}

/// `POST /supplier-returns/{id}/credit`: settles a dispatched return by hand.
/// A refused credit above the limit asks a second person to approve instead
/// (`STK.ACL.02`).
async fn credit_return(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<CreditForm>,
) -> Result<Response> {
    if !may_manage_returns(&state, &user).await? {
        return Err(Error::Forbidden);
    }
    let basis = CreditBasis::parse(form.basis.as_deref().unwrap_or_default())
        .ok_or_else(|| refused(text(&lang, "return.bad_basis")))?;
    let reason = form.reason.as_deref().unwrap_or_default();
    match crate::return_credit::credit(&state, &user, id, basis, reason).await {
        Ok(CreditOutcome::Credited(_)) => Ok((
            Toast::success(text(&lang, "return.credited")),
            renox::HxRefresh,
        )
            .into_response()),
        Ok(CreditOutcome::Waiting(_)) => Ok((
            Toast::success(text(&lang, "return.credit_waiting")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Err(refused(message)),
        Err(other) => Err(other.into_error()),
    }
}

/// What procurement enters to cancel a draft return.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct CancelForm {
    /// Why it will not be sent.
    #[validate(max = 500)]
    pub reason: Option<String>,
    /// `receive_again` or `discard`, for goods rejected at receipt.
    #[validate(max = 32)]
    pub kept: Option<String>,
}

/// `POST /supplier-returns/{id}/cancel`: cancels a draft return with a
/// reason; the goods stay.
async fn cancel_return(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<CancelForm>,
) -> Result<Response> {
    if !may_manage_returns(&state, &user).await? {
        return Err(Error::Forbidden);
    }
    let kept = match form
        .kept
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        None => None,
        Some(value) => Some(
            KeptGoods::parse(value).ok_or_else(|| refused(text(&lang, "return.bad_kept_goods")))?,
        ),
    };
    match crate::return_credit::cancel(
        &state,
        &user,
        id,
        form.reason.as_deref().unwrap_or_default(),
        kept,
    )
    .await
    {
        Ok(_) => Ok((
            Toast::success(text(&lang, "return.cancelled")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Err(refused(message)),
        Err(TransitionError::ReasonRequired) => Err(refused(text(&lang, "return.reason_needed"))),
        Err(other) => Err(other.into_error()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quantity_is_thousandths_of_the_base_unit() {
        assert_eq!(parse_quantity("80"), Some(80_000));
        assert_eq!(parse_quantity("12.5"), Some(12_500));
        assert_eq!(parse_quantity(" 0.125 "), Some(125));
        assert_eq!(parse_quantity("1,000"), Some(1_000_000));
        assert_eq!(parse_quantity(".5"), Some(500));
    }

    #[test]
    fn anything_else_is_not_a_quantity() {
        for bad in ["", " ", "-5", "1.2345", "abc", "1.2.3", "."] {
            assert_eq!(parse_quantity(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_line_form_becomes_a_draft_line() {
        let form = LineForm {
            received: Some("100".into()),
            accepted: Some("80".into()),
            rejected: Some("20".into()),
            reason: Some("temperature".into()),
            handling: Some("refused_at_door".into()),
            evidence: Some("9 C".into()),
            ..LineForm::default()
        };
        let draft = form.draft().unwrap();
        assert_eq!(
            (
                draft.received_qty_milli,
                draft.accepted_qty_milli,
                draft.rejected_qty_milli
            ),
            (100_000, 80_000, 20_000)
        );
        assert_eq!(
            draft.rejected_handling,
            Some(RejectedHandling::RefusedAtDoor)
        );
        assert_eq!(draft.wanted_outcome, WantedOutcome::Replace);
        let bad = LineForm {
            handling: Some("thrown_away".into()),
            ..LineForm::default()
        };
        assert!(bad.draft().is_err());
        let bad_date = LineForm {
            expiry: Some("soon".into()),
            ..LineForm::default()
        };
        assert!(bad_date.draft().is_err());
    }
}
