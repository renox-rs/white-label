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
use wl_kernel::invoicing::{DEFAULT_CURRENCY, InvoiceDraft, invoicer};
use wl_kernel::sales::{
    FulfilOrder, FulfilledLine, FulfilmentOutcome, OrderFulfilment, ReverseFulfilment,
    SALES_ORDER_SUBJECT_TYPE, SalesFuture, SalesOrderCancelled, SalesOrderClosed,
    SalesOrderConfirmed, SalesOrderFulfilled, SalesOrderFulfilmentReversed,
    SalesOrderLineCancelled, SalesOrderRejected,
};
use wl_kernel::{PartyId, Subject};

use crate::config::config_of;
use crate::promo_code::PromoCode;
use crate::sale_line::{KIND_ITEM, NewLine, SaleLine, totals};
use crate::sales_order::{
    FulfilmentRecord, ORDER_SERIES, PAYMENT_ONLINE, SalesOrder, SalesOrderStatus,
};
use crate::sales_order_machine::{
    ACCEPT, CANCEL, CANCEL_LINE, CANCEL_LINE_PARTLY, CLOSE, CONFIRM_PAYMENT,
    CONFIRM_PAYMENT_FOR_ACCEPTANCE, CancelLineInput, CloseInput, FULFIL, FULFIL_LAST, FulfilInput,
    PAYMENT_FAILED, PayInput, REJECT, REVERSE_FULFILMENT, REVERSE_FULFILMENT_ALL, ReverseInput,
    SUBMIT, SUBMIT_FOR_ACCEPTANCE, SUBMIT_ONLINE, SubmitInput,
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
            external_number: None,
            promo_code: None,
        }
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
    let config = config_of(state);
    let online = order.payment_mode == PAYMENT_ONLINE;
    // Paid already (the gateway answered before the order was placed): no wait.
    let prepaid = online && crate::online_payment::paid_before_submit(state, &order, t.gross).await?;
    let input = SubmitInput {
        line_count: lines.len() as i64,
        net: t.net,
        vat: t.vat,
        gross: t.gross,
        window_minutes: config.online_payment_window_minutes,
    };
    let name = if online && !prepaid {
        SUBMIT_ONLINE
    } else if config.order_acceptance_channels.contains(&order.channel) {
        SUBMIT_FOR_ACCEPTANCE
    } else {
        SUBMIT
    };
    let _ = order
        .transition(name)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
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

/// Emits the kernel events after an order transition committed.
async fn announce(state: &AppState, event: &Transitioned<SalesOrder>) -> Result {
    let name = event.transition.as_str();
    let Some(order) = SalesOrder::find(&state.db, event.key).await? else {
        return Ok(());
    };
    let (id, party) = (order.order_id(), order.party());
    match name {
        SUBMIT | ACCEPT | CONFIRM_PAYMENT => {
            state
                .emit(SalesOrderConfirmed {
                    order: id,
                    party,
                    channel: order.channel.clone(),
                })
                .await
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
        CLOSE => state.emit(SalesOrderClosed { order: id, party }).await,
        _ => Ok(()),
    }
}

/// Registers the machine, the listener and the kernel's `OrderFulfilment`.
pub(crate) fn register(app: &mut Registry) {
    renox::state_machine::register::<SalesOrder>();
    wl_kernel::sales::provide_order_fulfilment(app, Fulfilment);
    app.listen(|event: Transitioned<SalesOrder>, state: AppState| async move {
        announce(&state, &event).await
    });
    // The online payment: `PaymentReceived`, `PaymentFailed` and the window.
    crate::online_payment::register(app);
}
