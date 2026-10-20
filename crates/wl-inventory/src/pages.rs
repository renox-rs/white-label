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
//! | `POST /purchase-orders/{id}/revise` | `wl_inventory.order_revise` | `purchase_orders.issue` |
//! | `GET /purchase-orders/{id}/revisions/{revision}/document` | `wl_inventory.order_document` | `purchase_orders.view` or `.issue` |
//! | `POST /landed-costs/{id}/allocate` | `wl_inventory.landed_cost_allocate` | `landed_costs.post` |
//! | `POST /landed-costs/{id}/adjustments` | `wl_inventory.landed_cost_adjustment` | `landed_costs.post` |
//! | `POST /landed-costs/{id}/adjustments/{adjustment}/post` | `wl_inventory.landed_cost_adjust` | `landed_costs.post` |
//! | `POST /goods-receipts/{id}/reverse` | `wl_inventory.receipt_reverse` | `goods_receipts.reverse` at the receiving location |
//!
//! The routes need a signed-in user and carry no permission guard of their
//! own: the guard counts global roles only, while a storekeeper may hold the
//! permission at one site store. The handlers check it in the scope of the
//! receipt and answer 403. A quantity is sent in the base unit of the item,
//! like `80` or `12.5`. The screens of the field PWA come later; the receipt
//! and its lines are recorded with [`crate::goods_receipt::create_draft`].

use std::collections::BTreeMap;

use renox::auth::permissions::users_with_permission_in;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};

use crate::goods_receipt::{DraftLine, GoodsReceipt, GoodsReceiptStatus, update_line};
use crate::landed_adjust::{AdjustOutcome, NewAdjustment};
use crate::landed_cost::{CostKind, NewCostLine};
use crate::permissions;
use crate::po_revise::{ReviseInput, ReviseOutcome};
use crate::purchase_order::PurchaseOrder;
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
        .post("/purchase-orders/{id}/revise", revise_order)
        .name("wl_inventory.order_revise")
        .get(
            "/purchase-orders/{id}/revisions/{revision}/document",
            order_document,
        )
        .name("wl_inventory.order_document")
        .post("/landed-costs/{id}/allocate", allocate_landed_cost)
        .name("wl_inventory.landed_cost_allocate")
        .post("/landed-costs/{id}/adjustments", create_adjustment)
        .name("wl_inventory.landed_cost_adjustment")
        .post(
            "/landed-costs/{id}/adjustments/{adjustment}/post",
            post_adjustment,
        )
        .name("wl_inventory.landed_cost_adjust")
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

/// `text` ("120", "1200.50", "0.5") as fils; `None` for anything else, a
/// negative number or more than two decimals.
pub fn parse_money(text: &str) -> Option<i64> {
    let text = text.trim().replace(',', "");
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (text.as_str(), ""),
    };
    if (whole.is_empty() && fraction.is_empty()) || fraction.len() > 2 {
        return None;
    }
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if !digits(whole) || !digits(fraction) {
        return None;
    }
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let fraction: i64 = format!("{fraction:0<2}").parse().ok()?;
    whole.checked_mul(100)?.checked_add(fraction)
}

/// The revision of a purchase order the form of `revise` asks for: `reason`,
/// optionally `expected_date` (like `2027-01-31`) with `requested_by` (`us`
/// or `supplier`), and `qty_<line>` and `price_<line>` for each line that
/// changes (price in the order's currency, like `1200.50`).
fn revision_of(form: &BTreeMap<String, String>) -> std::result::Result<ReviseInput, &'static str> {
    let field = |name: &str| form.get(name).map(|v| v.trim()).filter(|v| !v.is_empty());
    let mut asked = ReviseInput::new(field("reason").unwrap_or_default());
    asked.currency = field("currency").map(str::to_owned);
    if let Some(date) = field("expected_date") {
        asked.expected_date =
            Some(NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| "revise.bad_date")?);
        asked.date_requested_by = field("requested_by").map(str::to_owned);
    }
    for (name, quantity) in form {
        let Some(id) = name
            .strip_prefix("qty_")
            .and_then(|id| id.parse::<i64>().ok())
        else {
            continue;
        };
        let price = form
            .get(&format!("price_{id}"))
            .and_then(|price| parse_money(price));
        match (parse_quantity(quantity), price) {
            (Some(quantity), Some(price)) => {
                asked = asked.line(id, quantity, price);
            }
            _ => return Err("revise.bad_number"),
        }
    }
    Ok(asked)
}

/// `POST /purchase-orders/{id}/revise`: revises an issued order. A higher
/// total above a limit asks the approvers instead, and the revision in force
/// stays until they agree (`STK.ACL.03`).
async fn revise_order(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let asked = revision_of(&form).map_err(|key| refused(text(&lang, key)))?;
    match crate::po_revise::revise(&state, &user, id, &asked).await {
        Ok(ReviseOutcome::Revised(_)) => Ok((
            Toast::success(text(&lang, "revise.revised")),
            renox::HxRefresh,
        )
            .into_response()),
        Ok(ReviseOutcome::Waiting(_)) => Ok((
            Toast::success(text(&lang, "revise.waiting")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Err(refused(message)),
        Err(other) => Err(other.into_error()),
    }
}

/// `GET /purchase-orders/{id}/revisions/{revision}/document`: the PDF of a
/// revision, the one in force or an earlier one kept read-only.
async fn order_document(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, revision)): Path<(i64, i64)>,
) -> Result<Response> {
    let mut allowed = false;
    for permission in [
        permissions::PURCHASE_ORDERS_VIEW,
        permissions::PURCHASE_ORDERS_ISSUE,
    ] {
        allowed |= users_with_permission_in(
            &state.db,
            permission,
            &renox::auth::permissions::Scope::global(),
        )
        .await?
        .iter()
        .any(|holder| holder.id == user.id);
    }
    if !allowed {
        return Err(Error::Forbidden);
    }
    let order = PurchaseOrder::find_or_404(&state.db, id).await?;
    let document = if revision == order.revision {
        order.document
    } else {
        crate::po_revise::revisions_of(&state.db, id)
            .await?
            .into_iter()
            .find(|kept| kept.revision == revision)
            .and_then(|kept| kept.document)
    };
    let document = document.ok_or(Error::NotFound)?;
    Ok(([("content-type", "application/pdf")], document).into_response())
}

/// `text` ("120", "-1,200.50") as fils, negative for a refund; `None` for
/// anything else or more than two decimals.
pub fn parse_signed_money(text: &str) -> Option<i64> {
    let text = text.trim();
    match text.strip_prefix('-') {
        Some(rest) => parse_money(rest).map(|fils| -fils),
        None => parse_money(text),
    }
}

/// The form of a landed cost adjustment: `bill_reference`, `posting_date`
/// (like `2027-01-31`) and, for each cost line N from 1, `kind_N` (`freight`,
/// `duty`, `clearing` or `insurance`), `amount_N` (negative for a refund) and
/// optionally `description_N`.
fn adjustment_of(
    form: &BTreeMap<String, String>,
) -> std::result::Result<NewAdjustment, &'static str> {
    let field = |name: &str| form.get(name).map(|v| v.trim()).filter(|v| !v.is_empty());
    let posting_date =
        NaiveDate::parse_from_str(field("posting_date").unwrap_or_default(), "%Y-%m-%d")
            .map_err(|_| "landed.bad_date")?;
    let mut lines = Vec::new();
    for position in 1..=50 {
        let Some(amount) = field(&format!("amount_{position}")) else {
            continue;
        };
        let amount = parse_signed_money(amount).ok_or("landed.bad_number")?;
        let kind = match field(&format!("kind_{position}")).unwrap_or("freight") {
            "freight" => CostKind::Freight,
            "duty" => CostKind::Duty,
            "clearing" => CostKind::Clearing,
            "insurance" => CostKind::Insurance,
            _ => return Err("landed.bad_kind"),
        };
        let mut line = NewCostLine::of(kind, amount);
        line.description = field(&format!("description_{position}"))
            .unwrap_or_default()
            .to_owned();
        line.bill_reference = field("bill_reference").map(str::to_owned);
        lines.push(line);
    }
    Ok(NewAdjustment {
        bill_reference: field("bill_reference").unwrap_or_default().to_owned(),
        bill_id: None,
        posting_date,
        lines,
    })
}

/// `POST /landed-costs/{id}/allocate`: spreads the cost over the receipts;
/// `weight_<receipt line>` gives the weight or volume for those bases.
async fn allocate_landed_cost(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let mut weights = Vec::new();
    for (name, value) in &form {
        let Some(line) = name
            .strip_prefix("weight_")
            .and_then(|line| line.parse::<i64>().ok())
        else {
            continue;
        };
        let weight =
            parse_quantity(value).ok_or_else(|| refused(text(&lang, "landed.bad_number")))?;
        weights.push((line, weight));
    }
    match crate::landed_cost::allocate(&state, &user, id, &weights).await {
        Ok(_) => Ok((
            Toast::success(text(&lang, "landed.allocated")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Err(refused(message)),
        Err(other) => Err(other.into_error()),
    }
}

/// `POST /landed-costs/{id}/adjustments`: enters a draft adjustment of an
/// allocated landed cost.
async fn create_adjustment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    if !may_post_landed_costs(&state, &user).await? {
        return Err(Error::Forbidden);
    }
    let new = adjustment_of(&form).map_err(|key| refused(text(&lang, key)))?;
    crate::landed_adjust::create_adjustment(&state.db, Some(user.id), id, new).await?;
    Ok((
        Toast::success(text(&lang, "landed.drafted")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /landed-costs/{id}/adjustments/{adjustment}/post`: posts an
/// adjustment. Above the value it asks a second person to approve instead
/// (`STK.ACL.06`).
async fn post_adjustment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((id, adjustment)): Path<(i64, i64)>,
) -> Result<Response> {
    match crate::landed_adjust::adjust(&state, &user, id, adjustment).await {
        Ok(AdjustOutcome::Posted(_)) => Ok((
            Toast::success(text(&lang, "landed.adjusted")),
            renox::HxRefresh,
        )
            .into_response()),
        Ok(AdjustOutcome::Waiting(_)) => Ok((
            Toast::success(text(&lang, "landed.waiting")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Err(refused(message)),
        Err(other) => Err(other.into_error()),
    }
}

/// Whether `user` holds `landed_costs.post`.
async fn may_post_landed_costs(state: &AppState, user: &AuthUser) -> Result<bool> {
    Ok(users_with_permission_in(
        &state.db,
        permissions::LANDED_COSTS_POST,
        &renox::auth::permissions::Scope::global(),
    )
    .await?
    .iter()
    .any(|holder| holder.id == user.id))
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
    fn a_refund_is_a_negative_amount() {
        assert_eq!(parse_signed_money("-6,000"), Some(-600_000));
        assert_eq!(parse_signed_money("800"), Some(80_000));
        assert_eq!(parse_signed_money("--1"), None);
    }

    #[test]
    fn an_adjustment_form_numbers_its_lines() {
        let form: BTreeMap<String, String> = [
            ("bill_reference", "FWD-77"),
            ("posting_date", "2027-01-31"),
            ("kind_1", "freight"),
            ("amount_1", "800"),
            ("kind_2", "duty"),
            ("amount_2", "-300.50"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let new = adjustment_of(&form).unwrap();
        assert_eq!(new.bill_reference, "FWD-77");
        assert_eq!(new.lines.len(), 2);
        assert_eq!(
            (new.lines[0].kind, new.lines[0].amount),
            (CostKind::Freight, 80_000)
        );
        assert_eq!(
            (new.lines[1].kind, new.lines[1].amount),
            (CostKind::Duty, -30_050)
        );
        let mut bad = form.clone();
        bad.insert("kind_1".into(), "tea".into());
        assert!(adjustment_of(&bad).is_err());
        bad.remove("posting_date");
        assert!(adjustment_of(&bad).is_err());
    }

    #[test]
    fn a_price_is_fils() {
        assert_eq!(parse_money("120"), Some(12_000));
        assert_eq!(parse_money("1,200.5"), Some(120_050));
        assert_eq!(parse_money(" 0.05 "), Some(5));
        assert_eq!(parse_money(".5"), Some(50));
        for bad in ["", "-1", "1.234", "x", "."] {
            assert_eq!(parse_money(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_revision_form_names_lines_by_id() {
        let form: BTreeMap<String, String> = [
            ("reason", "price rise"),
            ("qty_11", "10"),
            ("price_11", "1200.00"),
            ("expected_date", "2027-01-31"),
            ("requested_by", "supplier"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let asked = revision_of(&form).unwrap();
        assert_eq!(asked.reason, "price rise");
        assert_eq!(asked.lines.len(), 1);
        assert_eq!(
            (asked.lines[0].quantity_milli, asked.lines[0].unit_price),
            (10_000, 120_000)
        );
        assert_eq!(asked.date_requested_by.as_deref(), Some("supplier"));
        let mut bad = form.clone();
        bad.insert("price_11".into(), "free".into());
        assert!(revision_of(&bad).is_err());
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
