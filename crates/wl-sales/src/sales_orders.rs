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
use renox::state_machine::Transitioned;
use wl_kernel::PartyId;
use wl_kernel::invoicing::{DEFAULT_CURRENCY, InvoiceDraft, invoicer};
use wl_kernel::sales::{
    FulfilOrder, FulfilledLine, FulfilmentOutcome, OrderFulfilment, ReverseFulfilment,
    SalesFuture, SalesOrderCancelled, SalesOrderClosed, SalesOrderConfirmed, SalesOrderFulfilled,
    SalesOrderFulfilmentReversed, SalesOrderRejected,
};

use crate::config::config_of;
use crate::sale_line::{KIND_ITEM, NewLine, SaleLine, totals};
use crate::sales_order::{
    FulfilmentRecord, ORDER_SERIES, PAYMENT_ONLINE, SalesOrder, SalesOrderStatus,
};
use crate::sales_order_machine::{
    ACCEPT, CANCEL, CLOSE, CONFIRM_PAYMENT, CONFIRM_PAYMENT_FOR_ACCEPTANCE, CloseInput, FULFIL,
    FULFIL_LAST, FulfilInput, PAYMENT_FAILED, PayInput, REJECT, REVERSE_FULFILMENT,
    REVERSE_FULFILMENT_ALL, ReverseInput, SUBMIT, SUBMIT_FOR_ACCEPTANCE, SUBMIT_ONLINE,
    SubmitInput,
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
}

impl NewSalesOrder {
    /// An order on account for `party`, taken by phone.
    pub fn new(party: PartyId) -> Self {
        NewSalesOrder {
            party,
            channel: "phone".to_owned(),
            payment_mode: crate::sales_order::PAYMENT_ON_ACCOUNT.to_owned(),
            legal_entity: None,
        }
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
    let order = SalesOrder::find_or_404(&state.db, id).await?;
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let t = totals(&lines);
    let input = SubmitInput {
        line_count: lines.len() as i64,
        net: t.net,
        vat: t.vat,
        gross: t.gross,
    };
    let config = config_of(state);
    let name = if order.payment_mode == PAYMENT_ONLINE {
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

/// Drafts the invoice of a fulfilled order, once, with the order as its
/// source; nothing when invoicing is off.
async fn draft_invoice(state: &AppState, id: i64) -> Result {
    let order = SalesOrder::find_or_404(&state.db, id).await?;
    if order.invoice_id.is_some() {
        return Ok(());
    }
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let mut draft =
        InvoiceDraft::new(order.party(), order.subject()).currency(&order.currency);
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
}
