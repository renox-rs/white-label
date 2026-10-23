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
//! - `PaymentReceived` and `PaymentFailed` are wired in
//!   [`crate::online_payment`] (BL-SAL-02 to BL-SAL-04): [`confirm_payment`]
//!   and [`payment_failed`] are the services its listeners call.
//! - An order from a channel with an external order number is entered with
//!   [`import`], which refuses a number the channel used before and names the
//!   existing order (BL-SAL-01); [`create`] is the same call that turns the
//!   refusal into a 409.

use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::approvals;
use wl_kernel::invoicing::{DEFAULT_CURRENCY, InvoiceDraft, invoicer};
use wl_kernel::sales::{
    AmendOrder, CancelByVertical, CancelLine, CancelOrder, ChangeOutcome, ChannelRefund,
    CreditFailure, FulfilOrder, FulfilledLine, FulfilmentOutcome, NoGoodsRefund, OrderFulfilment,
    OrderLineView, OrderView, ReverseFulfilment, SALES_ORDER_SUBJECT_TYPE, SalesFuture,
    SalesOrderCancelled, SalesOrderClosed, SalesOrderConfirmed, SalesOrderCreditHeld,
    SalesOrderDesk, SalesOrderFulfilled, SalesOrderFulfilmentReversed, SalesOrderId,
    SalesOrderPlaced, SalesOrderRejected,
};
use wl_kernel::{PartyId, Subject};

use crate::config::config_of;
use crate::credit;
use crate::customer_account::CustomerAccount;
use crate::promo_code::PromoCode;
use crate::sale_line::{KIND_ITEM, NewLine, SaleLine, totals};
use crate::sales_order::{
    FulfilmentRecord, ORDER_SERIES, PAYMENT_ONLINE, SalesOrder, SalesOrderStatus,
};
use crate::sales_order_machine::{
    ACCEPT, ACCEPT_HOLD, AcceptInput, CLOSE, CONFIRM_PAYMENT, CONFIRM_PAYMENT_FOR_ACCEPTANCE,
    CloseInput, FULFIL, FULFIL_LAST, FulfilInput, HOLD, PAYMENT_FAILED, PayInput, REJECT, RELEASE,
    RELEASE_APPROVED, RELEASE_PARTLY, RELEASE_PARTLY_APPROVED, RELEASE_PENDING,
    RELEASE_PENDING_APPROVED, REVERSE_FULFILMENT, REVERSE_FULFILMENT_ALL, REVERSE_FULFILMENT_HELD,
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
    /// The order number in the channel's own system (BL-SAL-01).
    pub external_number: Option<String>,
    /// The promo code the order uses; a use is taken when it is submitted
    /// (BL-SAL-02) and given back if its online payment fails (BL-SAL-03).
    pub promo_code: Option<String>,
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
            external_number: None,
            promo_code: None,
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

    /// With the number the channel gave the order (an aggregator's).
    pub fn external_number(mut self, number: &str) -> Self {
        self.external_number = Some(number.to_owned());
        self
    }

    /// Using the promo code `code`.
    pub fn promo_code(mut self, code: &str) -> Self {
        self.promo_code = Some(code.to_owned());
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

/// What entering an order from a channel gave.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)] // boxing `SalesOrder` would change the public API used by callers and tests
pub enum Imported {
    /// The draft order that was made.
    Created(SalesOrder),
    /// The channel used that external number before: no order was made, and
    /// this is the existing one (its id and number), so the integration can
    /// link to it.
    Duplicate {
        /// The existing order's id.
        id: i64,
        /// The existing order's number, e.g. `SO-2026-0101`.
        number: String,
    },
}

/// An external order number compared as the channel's unique key: trimmed and
/// lower case. `None` for a blank number, which is not checked.
pub fn external_key(number: &str) -> Option<String> {
    let key = number.trim().to_lowercase();
    (!key.is_empty()).then_some(key)
}

/// The promo code `code` as a row to hold a use of, refused (422) when it
/// does not exist or ended.
async fn promo_of(state: &AppState, code: &str) -> Result<PromoCode> {
    let found = PromoCode::query()
        .where_(PromoCode::CODE.eq(code.trim().to_uppercase()))
        .first(&state.db)
        .await?;
    let Some(promo) = found else {
        return Err(refused("wl_sales.orders.problem.promo_unknown"));
    };
    if promo
        .valid_to
        .is_some_and(|last| last < wl_kernel::documents::today())
    {
        return Err(refused("wl_sales.orders.problem.promo_ended"));
    }
    // Its uses are counted at `submit`, not when the order is entered.
    Ok(promo)
}

/// Enters a draft order from a channel (BL-SAL-01); `by` is the user id.
///
/// An external order number (compared trimmed and case-insensitive) that the
/// channel used before, also on an order since cancelled or rejected, makes no
/// order: the result names the existing one. Two imports at the same moment
/// cannot both pass, because the unique key on (channel, normalised number)
/// refuses the second insert. A blank number, and the setting *unique external
/// order number* off, skip the check.
///
/// # Errors
///
/// 422 for an unknown or ended promo code.
pub async fn import(state: &AppState, by: Option<i64>, new: NewSalesOrder) -> Result<Imported> {
    let external = new
        .external_number
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned);
    let key = external
        .as_deref()
        .and_then(external_key)
        .filter(|_| config_of(state).unique_external_order_number);
    let duplicate = |found: SalesOrder| Imported::Duplicate {
        id: found.id,
        number: found.number,
    };
    if let Some(key) = &key
        && let Some(found) = SalesOrder::of_external(&state.db, &new.channel, key).await?
    {
        return Ok(duplicate(found));
    }
    let promo = match new.promo_code.as_deref().map(str::trim) {
        Some(code) if !code.is_empty() => Some(promo_of(state, code).await?),
        _ => None,
    };
    let number = ORDER_SERIES
        .next_allowing_gaps(&state.db, None, None, wl_kernel::documents::today())
        .await?;
    let created = SalesOrder::create(
        &state.db,
        SalesOrder {
            number,
            party_id: new.party.0,
            legal_entity_id: new.legal_entity,
            customer_po: new.customer_po,
            call_off: new.call_off,
            channel: new.channel.clone(),
            payment_mode: new.payment_mode,
            currency: DEFAULT_CURRENCY.to_owned(),
            revision: 1,
            created_by: by,
            external_number: external,
            external_key: key.clone(),
            promo_code_id: promo.map(|p| p.id),
            ..SalesOrder::default()
        },
    )
    .await;
    match created {
        Ok(order) => Ok(Imported::Created(order)),
        Err(error) if error.is_unique_violation() => {
            // The other of two imports at the same moment won the key.
            if let Some(key) = &key
                && let Some(found) = SalesOrder::of_external(&state.db, &new.channel, key).await?
            {
                return Ok(duplicate(found));
            }
            Err(error)
        }
        Err(error) => Err(error),
    }
}

/// Enters a draft order; `by` is the user id.
///
/// # Errors
///
/// 409 naming the existing order when the channel used the external number
/// before ([`import`] returns that case as a value); 422 for a bad promo code.
pub async fn create(state: &AppState, by: Option<i64>, new: NewSalesOrder) -> Result<SalesOrder> {
    match import(state, by, new).await? {
        Imported::Created(order) => Ok(order),
        Imported::Duplicate { id, number } => Err(Error::Status(
            StatusCode::CONFLICT,
            english(
                "wl_sales.orders.problem.duplicate_external",
                &[("number", &number), ("id", &id)],
            ),
        )),
    }
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
/// payment not yet confirmed, `pending_acceptance` for a channel that needs
/// acceptance, else `confirmed`. A promo code of the order has a use taken.
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
    let online = order.payment_mode == PAYMENT_ONLINE;
    // Paid already (the gateway answered before the order was placed): no wait.
    let prepaid =
        online && crate::online_payment::paid_before_submit(state, &order, t.gross).await?;
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
        window_minutes: config.online_payment_window_minutes,
    };
    let mut name = if online && !prepaid {
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
    if name == SUBMIT_ONLINE {
        // The customer pays through the link until the window ends.
        crate::online_payment::open_link(state, id).await?;
    } else if prepaid {
        crate::online_payment::mark_prepaid(state, id).await?;
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
    let held = order.held_from.as_deref();
    let name = if held == Some(SalesOrderStatus::PartiallyFulfilled.as_str()) {
        RELEASE_PARTLY
    } else if held == Some(SalesOrderStatus::PendingAcceptance.as_str()) {
        RELEASE_PENDING
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
    // A source that still holds quantities fulfilled the order already; one
    // whose quantities were all reversed (a delivery reopened) may fulfil
    // again.
    if FulfilmentRecord::of_source(&state.db, order.id, &request.source)
        .await?
        .is_some_and(|record| !record.held().is_empty())
    {
        return Ok(FulfilmentOutcome::Already {
            order: order.order_id(),
        });
    }
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let problem = problem_with(&lines, &request.lines, SaleLine::fulfillable_milli);
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
    if config_of(state).invoice_on_delivery {
        crate::delivery_invoice::sync(state, order.id).await?;
    } else if complete {
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
    let name = if order.status == SalesOrderStatus::CreditHold {
        REVERSE_FULFILMENT_HELD
    } else if none_left {
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
    // A draft invoice follows what stays delivered; an issued one is left.
    crate::delivery_invoice::sync(state, order.id).await?;
    Ok(FulfilmentOutcome::Reversed {
        order: order.order_id(),
    })
}

/// Drafts the invoice of a fulfilled order, once, with the order as its
/// source; nothing when invoicing is off.
pub(crate) async fn draft_invoice(state: &AppState, id: i64) -> Result {
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
        // What was cancelled is not billed (BL-SAL-08, BL-SAL-09).
        if let Some(billed) = line.billable_draft_line() {
            draft = draft.line(billed.origin(order.subject()));
        }
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

/// The kernel's [`SalesOrderDesk`] for the verticals (06): read an order with
/// its lines, accept or reject it, cancel it as the system.
pub(crate) struct Desk;

/// The order `id` as a vertical reads it.
async fn view_of(state: &AppState, id: i64) -> Result<Option<OrderView>> {
    let Some(order) = SalesOrder::find(&state.db, id).await? else {
        return Ok(None);
    };
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    Ok(Some(OrderView {
        id: order.order_id(),
        number: order.number.clone(),
        party: order.party(),
        channel: order.channel.clone(),
        external_number: order.external_number.clone(),
        payment_mode: order.payment_mode.clone(),
        prepaid: crate::settle::is_prepaid(&order),
        currency: order.currency.clone(),
        gross_total: order.gross_total,
        legal_entity: order.legal_entity_id,
        status: order.status.as_str().to_owned(),
        revision: order.revision,
        refunded_amount: order.refunded_amount,
        aggregator: config_of(state).is_aggregator_channel(&order.channel),
        lines: lines
            .iter()
            .map(|line| OrderLineView {
                id: line.id,
                line_no: line.line_no,
                item_code: line.item_code.clone(),
                description: line.description.clone(),
                is_item: line.kind == KIND_ITEM,
                quantity_milli: line.quantity_milli,
                fulfilled_milli: line.fulfilled_milli,
                cancelled_milli: line.cancelled_milli,
                unit_price: line.unit_price,
                gross: line.gross(),
            })
            .collect(),
    }))
}

impl SalesOrderDesk for Desk {
    fn find<'a>(
        &'a self,
        state: &'a AppState,
        order: SalesOrderId,
    ) -> SalesFuture<'a, Option<OrderView>> {
        Box::pin(async move { view_of(state, order.0).await })
    }

    fn accept<'a>(
        &'a self,
        state: &'a AppState,
        user: &'a AuthUser,
        order: SalesOrderId,
    ) -> SalesFuture<'a, ()> {
        Box::pin(async move {
            let row = SalesOrder::find_or_404(&state.db, order.0).await?;
            let _ = row
                .transition(ACCEPT)
                .by(user)
                .run(state)
                .await
                .map_err(|e| e.into_error())?;
            Ok(())
        })
    }

    fn reject<'a>(
        &'a self,
        state: &'a AppState,
        user: &'a AuthUser,
        order: SalesOrderId,
        reason: &'a str,
    ) -> SalesFuture<'a, ()> {
        Box::pin(async move {
            let row = SalesOrder::find_or_404(&state.db, order.0).await?;
            let _ = row
                .transition(REJECT)
                .by(user)
                .reason(reason.to_owned())
                .run(state)
                .await
                .map_err(|e| e.into_error())?;
            Ok(())
        })
    }

    fn cancel<'a>(&'a self, state: &'a AppState, request: CancelByVertical) -> SalesFuture<'a, ()> {
        Box::pin(async move {
            let choices = request
                .outcomes
                .iter()
                .map(|(line, kind)| {
                    // No fee: the vertical decides the money.
                    crate::cancel_lines::WantedLine::new(*line, 0)
                        .outcome(*kind)
                        .fee(0)
                })
                .collect();
            crate::cancel_lines::cancel_as_system(
                state,
                request.order.0,
                &request.reason,
                choices,
                request.keep_payment,
                request.by,
            )
            .await
            .map(|_| ())
        })
    }

    fn amend<'a>(
        &'a self,
        state: &'a AppState,
        user: &'a AuthUser,
        request: AmendOrder,
    ) -> SalesFuture<'a, ChangeOutcome> {
        Box::pin(async move {
            let order = SalesOrder::find_or_404(&state.db, request.order.0).await?;
            let lines = SaleLine::of(&state.db, &order.subject()).await?;
            let mut input =
                crate::amend::AmendInput::new(&request.reason).seen_at(request.seen_revision);
            for change in &request.changes {
                // The discount of a line stays as it is.
                let discount = lines
                    .iter()
                    .find(|line| line.id == change.line)
                    .map_or(0, |line| line.discount);
                input = input.line(
                    change.line,
                    change.quantity_milli,
                    change.unit_price,
                    discount,
                );
            }
            for added in &request.added {
                input = input.add(&added.item_code, added.quantity_milli, added.unit_price);
                if let Some(line) = input.added.last_mut() {
                    line.description = added.description.clone();
                    line.vat_like = added.vat_like;
                }
            }
            match crate::amend::amend(state, user, order.id, input).await? {
                crate::amend::AmendOutcome::Amended(_) => Ok(ChangeOutcome::Done),
                crate::amend::AmendOutcome::Waiting(waiting) => Ok(ChangeOutcome::Waiting {
                    request: waiting.id,
                }),
            }
        })
    }

    fn cancel_line<'a>(
        &'a self,
        state: &'a AppState,
        user: &'a AuthUser,
        request: CancelLine,
    ) -> SalesFuture<'a, ChangeOutcome> {
        Box::pin(async move {
            let order = SalesOrder::find_or_404(&state.db, request.order.0).await?;
            let lines = SaleLine::of(&state.db, &order.subject()).await?;
            let open = lines
                .iter()
                .find(|line| line.id == request.line)
                .map_or(0, SaleLine::open_milli);
            let quantity = if request.quantity_milli > 0 {
                request.quantity_milli
            } else {
                open
            };
            // No fee: the vertical decides the money.
            let mut choice = crate::cancel_lines::WantedLine::default().fee(0);
            choice.outcome = request.outcome;
            let outcome = crate::cancel_lines::cancel_line(
                state,
                user,
                order.id,
                request.line,
                quantity,
                &request.reason,
                Some(choice),
            )
            .await?;
            Ok(change_of(outcome))
        })
    }

    fn cancel_order<'a>(
        &'a self,
        state: &'a AppState,
        user: &'a AuthUser,
        request: CancelOrder,
    ) -> SalesFuture<'a, ChangeOutcome> {
        Box::pin(async move {
            let choices = request
                .outcomes
                .iter()
                .map(|(line, kind)| {
                    crate::cancel_lines::WantedLine::new(*line, 0)
                        .outcome(*kind)
                        .fee(0)
                })
                .collect();
            let outcome = crate::cancel_lines::cancel_order(
                state,
                user,
                request.order.0,
                &request.reason,
                choices,
            )
            .await?;
            Ok(change_of(outcome))
        })
    }

    fn refund_no_goods<'a>(
        &'a self,
        state: &'a AppState,
        request: NoGoodsRefund,
    ) -> SalesFuture<'a, i64> {
        Box::pin(async move {
            let sale = Subject::new(SALES_ORDER_SUBJECT_TYPE, request.order.0);
            let mut new = crate::sales_returns::NewReturn::no_goods(sale)
                .reason(&request.reason)
                .complaint(&request.category, &request.party);
            for (line, fils) in &request.lines {
                new = new.refund_line(*line, *fils);
            }
            let done =
                crate::sales_returns::refund_by_vertical(state, new, request.credit_only).await?;
            Ok(done.id)
        })
    }

    fn find_external<'a>(
        &'a self,
        state: &'a AppState,
        channel: &'a str,
        number: &'a str,
    ) -> SalesFuture<'a, Option<OrderView>> {
        Box::pin(async move {
            let Some(key) = external_key(number) else {
                return Ok(None);
            };
            match SalesOrder::of_external(&state.db, channel, &key).await? {
                Some(order) => view_of(state, order.id).await,
                None => Ok(None),
            }
        })
    }

    fn refund_by_channel<'a>(
        &'a self,
        state: &'a AppState,
        request: ChannelRefund,
    ) -> SalesFuture<'a, i64> {
        Box::pin(async move {
            let sale = Subject::new(SALES_ORDER_SUBJECT_TYPE, request.order.0);
            let mut new = crate::sales_returns::NewReturn::no_goods(sale)
                .reason(&request.reason)
                .complaint(&request.category, &request.party);
            for (line, fils) in &request.lines {
                new = new.refund_line(*line, *fils);
            }
            let done = crate::sales_returns::refund_by_channel(state, new).await?;
            Ok(done.id)
        })
    }

    fn reverse_by_channel<'a>(
        &'a self,
        state: &'a AppState,
        sales_return: i64,
        reason: &'a str,
    ) -> SalesFuture<'a, ()> {
        Box::pin(async move {
            crate::sales_returns::reverse_by_channel(state, sales_return, reason).await?;
            Ok(())
        })
    }
}

/// What a cancellation of the kernel's desk did, as the vertical reads it.
fn change_of(outcome: crate::cancel_lines::CancelOutcome) -> ChangeOutcome {
    match outcome {
        crate::cancel_lines::CancelOutcome::Done(_) => ChangeOutcome::Done,
        crate::cancel_lines::CancelOutcome::Waiting(waiting) => ChangeOutcome::Waiting {
            request: waiting.id,
        },
    }
}

/// Emits the kernel events after an order transition committed.
async fn announce(state: &AppState, event: &Transitioned<SalesOrder>) -> Result {
    let name = event.transition.as_str();
    let Some(order) = SalesOrder::find(&state.db, event.key).await? else {
        return Ok(());
    };
    let (id, party) = (order.order_id(), order.party());
    match name {
        SUBMIT_FOR_ACCEPTANCE | CONFIRM_PAYMENT_FOR_ACCEPTANCE => {
            state
                .emit(SalesOrderPlaced {
                    order: id,
                    party,
                    channel: order.channel.clone(),
                })
                .await
        }
        SUBMIT | ACCEPT | CONFIRM_PAYMENT => confirmed_once(state, &order).await,
        RELEASE | RELEASE_PARTLY | RELEASE_APPROVED | RELEASE_PARTLY_APPROVED => {
            // The next dispatch passes while the customer owes no more than now.
            credit::remember_release(state, &order).await?;
            // Held at submit or accept, the order is confirmed for the first time.
            confirmed_once(state, &order).await
        }
        RELEASE_PENDING | RELEASE_PENDING_APPROVED => {
            // Back to waiting for acceptance: nothing is confirmed yet.
            credit::remember_release(state, &order).await
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
        REVERSE_FULFILMENT | REVERSE_FULFILMENT_ALL | REVERSE_FULFILMENT_HELD => {
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
        PAYMENT_FAILED => {
            state
                .emit(SalesOrderCancelled {
                    order: id,
                    party,
                    reason: order.reason.clone().unwrap_or_default(),
                    outcomes: Vec::new(),
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
        // An amendment or a cancelled line (BL-SAL-05, BL-SAL-08, BL-SAL-09):
        // the events and the money, from the change the transition wrote.
        other if crate::order_changes::is_change(other) => {
            crate::order_changes::announce_pending(state, order.id).await
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
    wl_kernel::sales::provide_credit_exposure(app, credit::Exposures);
    wl_kernel::sales::provide_order_desk(app, Desk);
    crate::policy::release_policy().register();
    // The order's creator may not decide the release of its hold, besides
    // the user who asks for it (`SAL.ACL.03`). The machine's entry in the
    // kernel registry and the subject link are registered with the changes
    // of a placed order (`order_changes::register`).
    approvals::register_excluded_approvers(SALES_ORDER_SUBJECT_TYPE, |db, request| {
        let creator = request.action == crate::policy::RELEASE_ACTION;
        let order_id = request.subject_id;
        Box::pin(async move {
            if !creator {
                return Ok(Vec::new());
            }
            Ok(SalesOrder::find(db, order_id)
                .await?
                .and_then(|order| order.created_by)
                .into_iter()
                .collect())
        })
    });
    app.listen(|event: Transitioned<SalesOrder>, state: AppState| async move {
        announce(&state, &event).await
    });
    // The online payment: `PaymentReceived`, `PaymentFailed` and the window.
    crate::online_payment::register(app);
    // The changes of a placed order: the approval wiring and the money jobs.
    crate::order_changes::register(app);
    crate::settle::register(app);
}
