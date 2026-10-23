//! The sales order services (wl-sales §4.2, §5.2, §5.3, §6
//! `OrderFulfilment`; base for BL-SAL-22): create an order, add lines,
//! submit it, confirm its online payment, fulfil and reverse the fulfilment
//! of its goods, cancel, reject, close. Every status change is a transition
//! of [`crate::sales_order_machine`]; the kernel events follow the commit, in
//! a listener on the machine's `Transitioned`.
//!
//! Decisions:
//! - `submit` picks the transition from the order: an online payment waits in
//!   `awaiting_payment`; a channel listed in
//!   [`SalesConfig::order_acceptance_channels`](crate::config::SalesConfig)
//!   (website and portal) waits in `pending_acceptance`; any other channel is
//!   `confirmed` at once.
//! - Fulfilment is idempotent per source (a dispatch note, a delivery, a
//!   collection): a source that fulfilled the order changes nothing the
//!   second time. Only `item` lines are handed over.
//! - The invoice is drafted when the order becomes `fulfilled`, with the
//!   order as its source and the order's lines (§7 *Invoice timing for
//!   orders*, on fulfilment). Invoicing at confirmation or periodically, and
//!   an invoice per hand-over, come with BL-SAL-13.
//! - `PaymentReceived` and `PaymentFailed` are not wired here
//!   (BL-SAL-02, BL-SAL-03): [`confirm_payment`] and [`payment_failed`] are
//!   the services those listeners call.

use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::approvals;
use wl_kernel::invoicing::{DEFAULT_CURRENCY, InvoiceDraft, invoicer};
use wl_kernel::sales::{
    CreditFailure, FulfilOrder, FulfilledLine, FulfilmentOutcome, OrderFulfilment,
    ReverseFulfilment, SALES_ORDER_SUBJECT_TYPE, SalesFuture, SalesOrderCancelled,
    SalesOrderClosed, SalesOrderConfirmed, SalesOrderCreditHeld, SalesOrderFulfilled,
    SalesOrderFulfilmentReversed, SalesOrderLineCancelled, SalesOrderRejected,
};
use wl_kernel::{PartyId, Subject};

use crate::config::config_of;
use crate::credit;
use crate::customer_account::CustomerAccount;
use crate::sale_line::{KIND_ITEM, NewLine, SaleLine, totals};
use crate::sales_order::{
    FulfilmentRecord, ORDER_SERIES, PAYMENT_ONLINE, SalesOrder, SalesOrderStatus,
};
use crate::sales_order_machine::{
    ACCEPT, ACCEPT_HOLD, AcceptInput, CANCEL, CANCEL_LINE, CANCEL_LINE_PARTLY, CLOSE,
    CONFIRM_PAYMENT, CONFIRM_PAYMENT_FOR_ACCEPTANCE, CancelLineInput, CloseInput, FULFIL,
    FULFIL_LAST, FulfilInput, HOLD, PAYMENT_FAILED, PayInput, REJECT, RELEASE, RELEASE_APPROVED,
    RELEASE_PARTLY, RELEASE_PARTLY_APPROVED, REVERSE_FULFILMENT, REVERSE_FULFILMENT_ALL,
    ReverseInput, SUBMIT, SUBMIT_FOR_ACCEPTANCE, SUBMIT_HOLD, SUBMIT_ONLINE, SubmitInput,
};
use crate::texts::english;

/// A 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// An order to enter.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NewSalesOrder {
    /// The customer.
    pub party: PartyId,
    /// The channel: `phone`, `website`, `portal`...
    pub channel: String,
    /// `on_account`, `online`, `cash_on_delivery`, `cash` or `card`.
    pub payment_mode: String,
    /// The legal entity whose order it is.
    pub legal_entity: Option<i64>,
    /// The customer's own PO number.
    pub customer_po: Option<String>,
    /// The call-off number against a blanket PO.
    pub call_off: Option<String>,
}

impl NewSalesOrder {
    /// An order on account for `party`, taken by phone.
    pub fn new(party: PartyId) -> Self {
        NewSalesOrder {
            party,
            channel: "phone".to_owned(),
            payment_mode: crate::sales_order::PAYMENT_ON_ACCOUNT.to_owned(),
            legal_entity: None,
            customer_po: None,
            call_off: None,
        }
    }

    /// With the customer's PO number (blank for none).
    pub fn customer_po(mut self, po: &str) -> Self {
        self.customer_po = (!po.trim().is_empty()).then(|| po.trim().to_owned());
        self
    }

    /// With the call-off number against a blanket PO (blank for none).
    pub fn call_off(mut self, call_off: &str) -> Self {
        self.call_off = (!call_off.trim().is_empty()).then(|| call_off.trim().to_owned());
        self
    }

    /// Through `channel`.
    pub fn channel(mut self, channel: &str) -> Self {
        self.channel = channel.to_owned();
        self
    }

    /// Paid with `mode`.
    pub fn payment_mode(mut self, mode: &str) -> Self {
        self.payment_mode = mode.to_owned();
        self
    }

    /// For the legal entity `id`.
    pub fn legal_entity(mut self, id: i64) -> Self {
        self.legal_entity = Some(id);
        self
    }
}

/// Enters a draft order; `by` is the user id.
pub async fn create(state: &AppState, by: Option<i64>, new: NewSalesOrder) -> Result<SalesOrder> {
    let number = ORDER_SERIES
        .next_allowing_gaps(&state.db, None, None, wl_kernel::documents::today())
        .await?;
    SalesOrder::create(
        &state.db,
        SalesOrder {
            number,
            party_id: new.party.0,
            legal_entity_id: new.legal_entity,
            customer_po: new.customer_po,
            call_off: new.call_off,
            channel: new.channel,
            payment_mode: new.payment_mode,
            currency: DEFAULT_CURRENCY.to_owned(),
            revision: 1,
            created_by: by,
            ..SalesOrder::default()
        },
    )
    .await
}

/// Adds `line` to the draft order `id` and refreshes its totals.
///
/// # Errors
///
/// 422 when the order is not a draft, the quantity is not positive, or an
/// item has a negative price.
pub async fn add_line(state: &AppState, id: i64, line: NewLine) -> Result<SalesOrder> {
    let order = SalesOrder::find_or_404(&state.db, id).await?;
    if !order.is_draft() {
        return Err(refused("wl_sales.orders.problem.not_draft"));
    }
    if line.quantity_milli <= 0 || (line.kind == KIND_ITEM && line.unit_price < 0) {
        return Err(refused("wl_sales.orders.problem.line"));
    }
    SaleLine::add(&state.db, &order.subject(), line).await?;
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let t = totals(&lines);
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(id))
        .update(
            &state.db,
            &[
                ("net_total", &t.net),
                ("vat_total", &t.vat),
                ("gross_total", &t.gross),
            ],
        )
        .await?;
    SalesOrder::find_or_404(&state.db, id).await
}

/// Submits the draft order `id` as `by`: `awaiting_payment` for an online
/// payment, `pending_acceptance` for a channel that needs acceptance, else
/// `confirmed`.
pub async fn submit(state: &AppState, by: &AuthUser, id: i64) -> Result<SalesOrder> {
    // A redemption that was not confirmed, or whose hold lapsed, goes before
    // the total is read.
    crate::redeem::prepare(state, &Subject::new(SALES_ORDER_SUBJECT_TYPE, id)).await?;
    let order = SalesOrder::find_or_404(&state.db, id).await?;
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let t = totals(&lines);
    // `SAL.RUL.05`: an order on account of a customer whose account is on
    // hold is refused and stays a draft; cash, card and online still pass.
    if order.is_on_account() && crate::account_hold::is_on_hold(&state.db, order.party_id).await? {
        return Err(refused("wl_sales.credit.problem.account_on_hold"));
    }
    let config = config_of(state);
    let credit_gate = config.credit_check
        && order.is_on_account()
        && CustomerAccount::of_party(&state.db, order.party_id)
            .await?
            .is_some();
    // The customer's PO number: refused naming the order that holds it, else
    // claimed by this submit (`Unique customer PO number per customer`).
    let po = crate::po_number::claim_for(state, &order).await?;
    let mut input = SubmitInput {
        line_count: lines.len() as i64,
        net: t.net,
        vat: t.vat,
        gross: t.gross,
        credit_gate,
        hold: None,
        po,
    };
    let mut name = if order.payment_mode == PAYMENT_ONLINE {
        SUBMIT_ONLINE
    } else if config.order_acceptance_channels.contains(&order.channel) {
        SUBMIT_FOR_ACCEPTANCE
    } else {
        SUBMIT
    };
    // An order that would be confirmed now is credit-checked first; one that
    // waits for acceptance is checked when it is accepted.
    if name == SUBMIT
        && let Some(hold) = credit::hold_for_placing(state, &order).await?
    {
        name = SUBMIT_HOLD;
        input.hold = Some(hold);
    }
    if let Err(error) = order
        .transition(name)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(|e| e.into_error())
    {
        // Another submit may have claimed the PO number a moment ago.
        return Err(crate::po_number::explain_race(state, &order, error).await);
    }
    // The points held for the order stay held until it resolves.
    crate::redeem::order_placed(state, &Subject::new(SALES_ORDER_SUBJECT_TYPE, id)).await?;
    SalesOrder::find_or_404(&state.db, id).await
}

/// Accepts the order `id` waiting in `pending_acceptance` as `by`:
/// `confirmed`, or `credit_hold` when the credit check of an order on account
/// fails (the credit check of `accept`, §4.2).
pub async fn accept(state: &AppState, by: &AuthUser, id: i64) -> Result<SalesOrder> {
    let order = SalesOrder::find_or_404(&state.db, id).await?;
    let hold = credit::hold_for_placing(state, &order).await?;
    let (name, input) = match hold {
        Some(hold) => (
            ACCEPT_HOLD,
            AcceptInput {
                checked: false,
                hold: Some(hold),
            },
        ),
        None => (
            ACCEPT,
            AcceptInput {
                checked: true,
                hold: None,
            },
        ),
    };
    let _ = order
        .transition(name)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    SalesOrder::find_or_404(&state.db, id).await
}

/// A credit releaser `by` asks to release the order `id` from its credit hold
/// (`release`), with an optional `note` for the approver. The order goes back
/// to the state it was held from once another holder of `credit.release`
/// approves; the request waits for that and the order stays on hold until
/// then (and after a rejection). Returns the order and whether it was
/// released at once, which happens only while no approval policy exists.
///
/// # Errors
///
/// 422 when the order is not on credit hold or a release is already
/// requested; 403 when `by` entered the order (`SAL.ACL.03`).
pub async fn release(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    note: &str,
) -> Result<(SalesOrder, bool)> {
    let order = SalesOrder::find_or_404(&state.db, id).await?;
    if order.status != SalesOrderStatus::CreditHold {
        return Err(refused("wl_sales.orders.problem.not_on_hold"));
    }
    if order.created_by == Some(by.id) {
        return Err(Error::Forbidden);
    }
    if !wl_kernel::ApprovalRequest::pending_about(&state.db, &order.subject())
        .await?
        .into_iter()
        .filter(|request| request.action == crate::policy::RELEASE_ACTION)
        .collect::<Vec<_>>()
        .is_empty()
    {
        return Err(refused("wl_sales.orders.problem.release_pending"));
    }
    let name = if order.state_before_hold.as_deref()
        == Some(SalesOrderStatus::PartiallyFulfilled.as_str())
    {
        RELEASE_PARTLY
    } else {
        RELEASE
    };
    let mut run = order.transition(name).by(by);
    if !note.trim().is_empty() {
        run = run.reason(note.trim().to_owned());
    }
    let outcome = run.run(state).await.map_err(|e| e.into_error())?;
    approvals::announce(state, &outcome).await?;
    let released = SalesOrder::find_or_404(&state.db, id).await?;
    let done = released.status != SalesOrderStatus::CreditHold;
    Ok((released, done))
}

/// The gateway confirmed `amount` fils for the order `id` waiting in
/// `awaiting_payment`: `confirmed`, or `pending_acceptance` on a channel
/// that needs acceptance.
pub async fn confirm_payment(state: &AppState, id: i64, amount: i64) -> Result<SalesOrder> {
    let order = SalesOrder::find_or_404(&state.db, id).await?;
    let name = if config_of(state)
        .order_acceptance_channels
        .contains(&order.channel)
    {
        CONFIRM_PAYMENT_FOR_ACCEPTANCE
    } else {
        CONFIRM_PAYMENT
    };
    let _ = order
        .transition(name)
        .by_system()
        .input(&PayInput { amount })
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    SalesOrder::find_or_404(&state.db, id).await
}

/// The online payment of the order `id` failed or timed out (`payment_failed`).
pub async fn payment_failed(state: &AppState, id: i64) -> Result<SalesOrder> {
    let order = SalesOrder::find_or_404(&state.db, id).await?;
    let _ = order
        .transition(PAYMENT_FAILED)
        .by_system()
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    SalesOrder::find_or_404(&state.db, id).await
}

/// Lines of the order that can be handed over: items only.
fn item_lines(lines: &[SaleLine]) -> Vec<&SaleLine> {
    lines.iter().filter(|l| l.kind == KIND_ITEM).collect()
}

/// The first problem with `requested` against the order's `lines`, as an
/// English text: an unknown line, a quantity that is not positive, a line
/// named twice, or more than is open (`fulfil`) or held (`reverse`).
fn problem_with(
    lines: &[SaleLine],
    requested: &[FulfilledLine],
    limit: impl Fn(&SaleLine) -> i64,
) -> Option<String> {
    if requested.is_empty() {
        return Some(english("wl_sales.orders.problem.no_quantities", &[]));
    }
    let items = item_lines(lines);
    for (n, wanted) in requested.iter().enumerate() {
        let Some(line) = items.iter().find(|l| l.id == wanted.line) else {
            return Some(english("wl_sales.orders.problem.unknown_line", &[]));
        };
        let repeated = requested[..n].iter().any(|r| r.line == wanted.line);
        if wanted.quantity_milli <= 0 || repeated || wanted.quantity_milli > limit(line) {
            return Some(english("wl_sales.orders.problem.quantity", &[]));
        }
    }
    None
}

/// Fulfils the quantities of `request` (`fulfil` or `fulfil_last`),
/// idempotent per source.
///
/// # Errors
///
/// 422 when the order is not `confirmed` or `partially_fulfilled`, or a
/// quantity is unknown, not positive, repeated or above what is open.
pub async fn fulfil(state: &AppState, request: FulfilOrder) -> Result<FulfilmentOutcome> {
    let Some(order) = SalesOrder::find(&state.db, request.order.0).await? else {
        return Ok(FulfilmentOutcome::NoOrder);
    };
    if FulfilmentRecord::of_source(&state.db, order.id, &request.source)
        .await?
        .is_some()
    {
        return Ok(FulfilmentOutcome::Already {
            order: order.order_id(),
        });
    }
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let problem = problem_with(&lines, &request.lines, SaleLine::open_milli);
    let complete = problem.is_none()
        && item_lines(&lines).iter().all(|line| {
            let taken = request
                .lines
                .iter()
                .find(|r| r.line == line.id)
                .map_or(0, |r| r.quantity_milli);
            line.open_milli() - taken == 0
        });
    let input = FulfilInput {
        lines: request.lines.clone(),
        source: request.source.clone(),
        complete,
        problem,
    };
    let name = if complete { FULFIL_LAST } else { FULFIL };
    let meta = renox::serde_json::to_value(&request).unwrap_or(Value::Null);
    let _ = order
        .transition(name)
        .by_system()
        .input(&input)
        .meta(meta)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    if complete {
        draft_invoice(state, order.id).await?;
    }
    Ok(FulfilmentOutcome::Fulfilled {
        order: order.order_id(),
        complete,
    })
}

/// Takes the quantities of an earlier hand-over off the order again
/// (`reverse_fulfilment` or `reverse_fulfilment_all`).
///
/// # Errors
///
/// 422 when the order is not `partially_fulfilled` or `fulfilled`, or the
/// source did not hand over those quantities.
pub async fn reverse_fulfilment(
    state: &AppState,
    request: ReverseFulfilment,
) -> Result<FulfilmentOutcome> {
    let Some(order) = SalesOrder::find(&state.db, request.order.0).await? else {
        return Ok(FulfilmentOutcome::NoOrder);
    };
    let Some(record) = FulfilmentRecord::of_source(&state.db, order.id, &request.source).await?
    else {
        return Err(refused("wl_sales.orders.problem.unknown_source"));
    };
    let held = record.held();
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let held_of = |line: &SaleLine| {
        held.iter()
            .find(|h| h.line == line.id)
            .map_or(0, |h| h.quantity_milli)
    };
    let problem = problem_with(&lines, &request.lines, held_of);
    let held_after: Vec<FulfilledLine> = held
        .iter()
        .map(|h| FulfilledLine {
            line: h.line,
            quantity_milli: h.quantity_milli
                - request
                    .lines
                    .iter()
                    .find(|r| r.line == h.line)
                    .map_or(0, |r| r.quantity_milli),
        })
        .filter(|h| h.quantity_milli > 0)
        .collect();
    let none_left = item_lines(&lines).iter().all(|line| {
        let taken = request
            .lines
            .iter()
            .find(|r| r.line == line.id)
            .map_or(0, |r| r.quantity_milli);
        line.fulfilled_milli - taken == 0
    });
    let input = ReverseInput {
        lines: request.lines.clone(),
        source: request.source.clone(),
        held_after: renox::serde_json::to_string(&held_after).unwrap_or_default(),
        none_left,
        problem,
    };
    let name = if none_left {
        REVERSE_FULFILMENT_ALL
    } else {
        REVERSE_FULFILMENT
    };
    let meta = renox::serde_json::to_value(&request).unwrap_or(Value::Null);
    let _ = order
        .transition(name)
        .by_system()
        .input(&input)
        .meta(meta)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    Ok(FulfilmentOutcome::Reversed {
        order: order.order_id(),
    })
}

/// Cancels `quantity_milli` of the open quantity of the item line `line` of
/// the order `id` as `by`, with `reason` (`cancel_line`).
///
/// Decision: the order's stored totals stay as they were ordered; the money
/// side of a cancelled line (refund, credit note, a smaller invoice) is
/// BL-SAL-06 and BL-SAL-09. A cancellation that would leave nothing open or
/// fulfilled is `cancel`'s, and one that would leave only fulfilled
/// quantities is `cancel_remaining`'s; both are refused here.
///
/// # Errors
///
/// 422 when the order is not `confirmed` or `partially_fulfilled`, the line
/// is not an item of the order, the quantity is not above zero or exceeds
/// what is open, the reason is blank, or nothing would stay open.
pub async fn cancel_line(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    line: i64,
    quantity_milli: i64,
    reason: &str,
) -> Result<SalesOrder> {
    let order = SalesOrder::find_or_404(&state.db, id).await?;
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let items = item_lines(&lines);
    let problem = match items.iter().find(|l| l.id == line) {
        None => Some(english("wl_sales.orders.problem.unknown_line", &[])),
        Some(found) if quantity_milli <= 0 || quantity_milli > found.open_milli() => {
            Some(english("wl_sales.orders.problem.quantity", &[]))
        }
        Some(_) => {
            let left: i64 = items.iter().map(|l| l.open_milli()).sum::<i64>() - quantity_milli;
            (left <= 0).then(|| english("wl_sales.orders.problem.cancel_line_last", &[]))
        }
    };
    let input = CancelLineInput {
        line,
        quantity_milli,
        problem,
    };
    let name = if order.status == SalesOrderStatus::PartiallyFulfilled {
        CANCEL_LINE_PARTLY
    } else {
        CANCEL_LINE
    };
    let meta = renox::serde_json::to_value(&input).unwrap_or(Value::Null);
    let _ = order
        .transition(name)
        .by(by)
        .reason(reason.trim().to_owned())
        .input(&input)
        .meta(meta)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    SalesOrder::find_or_404(&state.db, id).await
}

/// Drafts the invoice of a fulfilled order, once, with the order as its
/// source; nothing when invoicing is off.
async fn draft_invoice(state: &AppState, id: i64) -> Result {
    let order = SalesOrder::find_or_404(&state.db, id).await?;
    if order.invoice_id.is_some() {
        return Ok(());
    }
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let mut draft = InvoiceDraft::new(order.party(), order.subject()).currency(&order.currency);
    if let Some(entity) = order.legal_entity_id {
        draft = draft.legal_entity(entity);
    }
    for line in &lines {
        draft = draft.line(line.draft_line().origin(order.subject()));
    }
    if let Some(invoice) = invoicer(state).draft(&state.db, draft).await? {
        SalesOrder::query()
            .where_(SalesOrder::ID.eq(id))
            .update(&state.db, &[("invoice_id", &Some(invoice.0))])
            .await?;
    }
    Ok(())
}

/// Closes the `fulfilled` order `id` when it has an invoice (`close`).
/// Returns whether it closed.
pub async fn close(state: &AppState, id: i64) -> Result<bool> {
    let order = SalesOrder::find_or_404(&state.db, id).await?;
    let input = CloseInput {
        invoiced: order.invoice_id.is_some(),
    };
    match order
        .transition(CLOSE)
        .by_system()
        .input(&input)
        .run(state)
        .await
    {
        Ok(_) => Ok(true),
        Err(
            renox::state_machine::TransitionError::NotFrom(_)
            | renox::state_machine::TransitionError::Guard(_),
        ) => Ok(false),
        Err(error) => Err(error.into_error()),
    }
}

/// The kernel's [`OrderFulfilment`] for `wl-inventory`, `wl-delivery` and the
/// verticals.
pub(crate) struct Fulfilment;

impl OrderFulfilment for Fulfilment {
    fn fulfil<'a>(
        &'a self,
        state: &'a AppState,
        request: FulfilOrder,
    ) -> SalesFuture<'a, FulfilmentOutcome> {
        Box::pin(async move { fulfil(state, request).await })
    }

    fn reverse<'a>(
        &'a self,
        state: &'a AppState,
        request: ReverseFulfilment,
    ) -> SalesFuture<'a, FulfilmentOutcome> {
        Box::pin(async move { reverse_fulfilment(state, request).await })
    }
}

/// Emits `SalesOrderConfirmed` for `order` unless it was emitted before, and
/// remembers that it was (a second kitchen ticket or work order must never
/// follow a release at dispatch).
async fn confirmed_once(state: &AppState, order: &SalesOrder) -> Result {
    if order.confirmed_announced {
        return Ok(());
    }
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(order.id))
        .update(&state.db, &[("confirmed_announced", &true)])
        .await?;
    state
        .emit(SalesOrderConfirmed {
            order: order.order_id(),
            party: order.party(),
            channel: order.channel.clone(),
        })
        .await
}

/// Emits the kernel events after an order transition committed.
async fn announce(state: &AppState, event: &Transitioned<SalesOrder>) -> Result {
    let name = event.transition.as_str();
    let Some(order) = SalesOrder::find(&state.db, event.key).await? else {
        return Ok(());
    };
    let (id, party) = (order.order_id(), order.party());
    match name {
        SUBMIT | ACCEPT | CONFIRM_PAYMENT => confirmed_once(state, &order).await,
        RELEASE | RELEASE_PARTLY | RELEASE_APPROVED | RELEASE_PARTLY_APPROVED => {
            // The next dispatch passes while the customer owes no more than now.
            credit::remember_release(state, &order).await?;
            // Held at submit or accept, the order is confirmed for the first time.
            confirmed_once(state, &order).await
        }
        FULFIL | FULFIL_LAST => {
            let Ok(request) = renox::serde_json::from_value::<FulfilOrder>(event.meta.clone())
            else {
                return Ok(());
            };
            state
                .emit(SalesOrderFulfilled {
                    order: id,
                    party,
                    lines: request.lines,
                    source: request.source,
                    complete: order.status == SalesOrderStatus::Fulfilled,
                })
                .await
        }
        REVERSE_FULFILMENT | REVERSE_FULFILMENT_ALL => {
            let Ok(request) =
                renox::serde_json::from_value::<ReverseFulfilment>(event.meta.clone())
            else {
                return Ok(());
            };
            state
                .emit(SalesOrderFulfilmentReversed {
                    order: id,
                    party,
                    lines: request.lines,
                    source: request.source,
                    cause: request.cause,
                })
                .await
        }
        CANCEL | PAYMENT_FAILED => {
            state
                .emit(SalesOrderCancelled {
                    order: id,
                    party,
                    reason: order.reason.clone().unwrap_or_default(),
                })
                .await
        }
        REJECT => {
            state
                .emit(SalesOrderRejected {
                    order: id,
                    party,
                    reason: order.reason.clone().unwrap_or_default(),
                })
                .await
        }
        CANCEL_LINE | CANCEL_LINE_PARTLY => {
            let Ok(input) = renox::serde_json::from_value::<CancelLineInput>(event.meta.clone())
            else {
                return Ok(());
            };
            state
                .emit(SalesOrderLineCancelled {
                    order: id,
                    party,
                    line: input.line,
                    quantity_milli: input.quantity_milli,
                    reason: event.reason.clone().unwrap_or_default(),
                })
                .await
        }
        SUBMIT_HOLD | ACCEPT_HOLD | HOLD => {
            crate::notices::order_credit_held(state, &order).await?;
            state
                .emit(SalesOrderCreditHeld {
                    order: id,
                    party,
                    open_balance: order.hold_open_balance.unwrap_or_default(),
                    overdue: order.hold_overdue.unwrap_or_default(),
                    limit: order.hold_limit,
                    reason: order
                        .hold_reason
                        .as_deref()
                        .and_then(CreditFailure::parse)
                        .unwrap_or(CreditFailure::Limit),
                })
                .await
        }
        CLOSE => state.emit(SalesOrderClosed { order: id, party }).await,
        _ => Ok(()),
    }
}

/// Registers the machine, the listener and the kernel's `OrderFulfilment`.
pub(crate) fn register(app: &mut Registry) {
    renox::state_machine::register::<SalesOrder>();
    wl_kernel::sales::provide_order_fulfilment(app, Fulfilment);
    wl_kernel::sales::provide_credit_checker(app, credit::Checker);
    crate::sales_order_machine::subject_machine().register();
    crate::policy::release_policy().register();
    // The order's creator may not decide the release of its hold, besides
    // the user who asks for it (`SAL.ACL.03`).
    approvals::register_excluded_approvers(SALES_ORDER_SUBJECT_TYPE, |db, request| {
        Box::pin(async move {
            Ok(SalesOrder::find(db, request.subject_id)
                .await?
                .and_then(|order| order.created_by)
                .into_iter()
                .collect())
        })
    });
    approvals::register_subject_link(SALES_ORDER_SUBJECT_TYPE, |id| format!("/sales-orders/{id}"));
    app.listen(|event: Transitioned<SalesOrder>, state: AppState| async move {
        announce(&state, &event).await
    });
}
