//! The sales return services (wl-sales §4.5, §5.6, §6; base for
//! BL-SAL-22): request a return of a completed counter sale or of an order
//! that was handed over, receive the goods, resolve, reject or reverse it.
//! Every status change is a transition of [`crate::sales_return_machine`];
//! [`ReturnResolved`] and [`ReturnReversed`] follow the commit.
//!
//! **Refund after completion with nothing returned (BL-SAL-14,
//! `SAL.ACL.06`).** A *no goods* return is raised from a completed order (or
//! a complaint ticket) with a complaint category, a responsible party and a
//! refund per line ([`NewReturn::refund_line`]); [`refund_after_completion`]
//! raises it and resolves it with a refund at once. Above
//! [`SalesConfig::refund_approval_limit`] the resolution waits for a manager
//! who is not the recorder (the return stays `requested`); the kernel runs
//! `resolve_approved` at the last yes. The refund is requested of wl-payments
//! once the resolution committed, and the order's amount refunded rises. The
//! cap is a value per sale line (what the line is worth less what earlier
//! returns took) and a value per order (what it was worth).
//!
//! **Refunded by the channel.** An aggregator refunded its customer itself:
//! [`refund_by_channel`] (06's payout statement `match`) raises and resolves
//! the return as the system, with no approval, refund or credit note;
//! [`reverse_by_channel`] (`void`) takes it back. The resolution is refused on
//! an order of a channel that is not an aggregator's.

use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest, Require, Required};
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::sales::{
    COUNTER_SALE_SUBJECT_TYPE, ReturnResolved, ReturnReversed, SALES_ORDER_SUBJECT_TYPE,
    SALES_RETURN_SUBJECT_TYPE,
};
use wl_kernel::tickets::{TaskComment, TicketId, task_creator};

use crate::config::config_of;
use crate::counter_sale::{CounterSale, CounterSaleStatus};
use crate::permissions;
use crate::sale_line::{KIND_ITEM, SaleLine, ratio_half_up};
use crate::sales_order::{SalesOrder, SalesOrderStatus};
use crate::sales_return::{
    RETURN_SERIES, ReturnKind, ReturnResolution, ReturnStatus, SalesReturn, SalesReturnLine,
};
use crate::sales_return_machine::{
    RECEIVE, REJECT, RESOLVE, RESOLVE_APPROVED, RESOLVE_SYSTEM, REVERSE, REVERSE_SYSTEM,
    ReceiveInput, ResolveInput,
};
use crate::texts::english;

/// The approval policies' action key of a refund or credit after completion
/// with nothing returned above the limit (`SAL.ACL.06`).
pub const ACTION_REFUND: &str = "sales.return_refund";

/// A 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// A return to request.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NewReturn {
    /// The sale returned: a counter sale or a sales order.
    pub sale: Subject,
    /// Goods or no goods.
    pub kind: ReturnKind,
    /// Why the customer returns.
    pub reason: String,
    /// The sale lines and the quantities (thousandths) taken back.
    pub lines: Vec<(i64, i64)>,
    /// The user who requests it.
    pub requested_by: Option<i64>,
    /// Refunds by value: the sale line and the amount with VAT in fils (a
    /// *no goods* return).
    pub amounts: Vec<(i64, i64)>,
    /// The complaint category (a *no goods* return).
    pub complaint_category: Option<String>,
    /// The responsible party (a *no goods* return).
    pub responsible_party: Option<String>,
    /// The complaint ticket it is raised from.
    pub ticket: Option<i64>,
}

impl NewReturn {
    /// A return of goods from `sale`, with no line yet.
    pub fn new(sale: Subject) -> Self {
        NewReturn {
            sale,
            kind: ReturnKind::Goods,
            reason: String::new(),
            lines: Vec::new(),
            requested_by: None,
            amounts: Vec::new(),
            complaint_category: None,
            responsible_party: None,
            ticket: None,
        }
    }

    /// A *no goods* return of `sale`: nothing comes back, money does.
    pub fn no_goods(sale: Subject) -> Self {
        NewReturn::new(sale).kind(ReturnKind::NoGoods)
    }

    /// Refunding `fils` (with VAT) of the sale line `line`.
    pub fn refund_line(mut self, line: i64, fils: i64) -> Self {
        self.amounts.push((line, fils));
        self
    }

    /// With this complaint category and responsible party (stored names).
    pub fn complaint(mut self, category: &str, party: &str) -> Self {
        self.complaint_category = Some(category.trim().to_lowercase());
        self.responsible_party = Some(party.trim().to_lowercase());
        self
    }

    /// Raised from the complaint ticket `ticket`.
    pub fn ticket(mut self, ticket: i64) -> Self {
        self.ticket = Some(ticket);
        self
    }

    /// With this kind.
    pub fn kind(mut self, kind: ReturnKind) -> Self {
        self.kind = kind;
        self
    }

    /// With this reason.
    pub fn reason(mut self, reason: &str) -> Self {
        self.reason = reason.to_owned();
        self
    }

    /// Taking back `quantity_milli` of the sale line `line`.
    pub fn line(mut self, line: i64, quantity_milli: i64) -> Self {
        self.lines.push((line, quantity_milli));
        self
    }

    /// Requested by the user `user`.
    pub fn requested_by(mut self, user: i64) -> Self {
        self.requested_by = Some(user);
        self
    }
}

/// The quantity of the sale line `line` that earlier returns (not rejected)
/// already take back.
async fn returned_before(db: &Db, line: i64) -> Result<i64> {
    let rows = SalesReturnLine::query()
        .where_(SalesReturnLine::SALE_LINE_ID.eq(line))
        .get(db)
        .await?;
    let mut total = 0;
    for row in rows {
        if let Some(earlier) = SalesReturn::find(db, row.return_id).await?
            && earlier.status != ReturnStatus::Rejected
        {
            total += row.quantity_milli;
        }
    }
    Ok(total)
}

/// The value of a return line of the sale line `line`, in fils with VAT: the
/// stored one, else the quantity's share of the line.
fn value_of(row: &SalesReturnLine, line: &SaleLine) -> i64 {
    if row.refund_value > 0 {
        row.refund_value
    } else {
        ratio_half_up(line.gross(), row.quantity_milli, line.quantity_milli)
    }
}

/// Whether a return in `status` still takes its value from the sale line.
fn counts(status: ReturnStatus) -> bool {
    !matches!(status, ReturnStatus::Rejected | ReturnStatus::Reversed)
}

/// What earlier returns of the sale line `line` (not rejected or reversed)
/// already take back, in fils with VAT.
async fn earlier_value(db: &Db, line: &SaleLine) -> Result<i64> {
    let rows = SalesReturnLine::query()
        .where_(SalesReturnLine::SALE_LINE_ID.eq(line.id))
        .get(db)
        .await?;
    let mut total = 0;
    for row in rows {
        if let Some(earlier) = SalesReturn::find(db, row.return_id).await?
            && counts(earlier.status)
        {
            total += value_of(&row, line);
        }
    }
    Ok(total)
}

/// [`earlier_value`] read in `tx`, leaving out the return `except`.
async fn earlier_value_in(tx: &mut Transaction, line: &SaleLine, except: i64) -> Result<i64> {
    let rows = SalesReturnLine::query()
        .where_(SalesReturnLine::SALE_LINE_ID.eq(line.id))
        .get(&mut *tx)
        .await?;
    let mut total = 0;
    for row in rows {
        if row.return_id == except {
            continue;
        }
        if let Some(earlier) = SalesReturn::find(&mut *tx, row.return_id).await?
            && counts(earlier.status)
        {
            total += value_of(&row, line);
        }
    }
    Ok(total)
}

/// What the sale line `line` is worth for a *no goods* refund, in fils with
/// VAT: the part handed over on an order, the whole line on a counter sale.
fn worth_of(line: &SaleLine, by_fulfilment: bool) -> i64 {
    if by_fulfilment {
        ratio_half_up(line.gross(), line.fulfilled_milli, line.quantity_milli)
    } else {
        line.gross()
    }
}

/// Whether `value` is one of the stored names `allowed`.
fn listed(allowed: &[String], value: &Option<String>) -> bool {
    value
        .as_deref()
        .is_some_and(|v| allowed.iter().any(|a| a == v.trim()))
}

/// Requests a return. A *no goods* return takes the lines as quantities or
/// as refunds by value ([`NewReturn::refund_line`]); its value may not exceed
/// what each line, and the order, are worth less what earlier returns took.
///
/// # Errors
///
/// 422 when the sale is not a counter sale or an order, was not completed or
/// handed over, a line is not an item of the sale, a quantity is not above
/// zero or exceeds what was sold less what earlier returns take back, a
/// *no goods* refund exceeds the line or the order, a goods return names
/// amounts, or the complaint category or responsible party is not one of the
/// settings'.
pub async fn request(state: &AppState, new: NewReturn) -> Result<SalesReturn> {
    if new.lines.is_empty() && new.amounts.is_empty() {
        return Err(refused("wl_sales.returns.problem.no_lines"));
    }
    if new.kind == ReturnKind::Goods && !new.amounts.is_empty() {
        return Err(refused("wl_sales.returns.problem.amounts_for_goods"));
    }
    let config = config_of(state);
    if new.complaint_category.is_some()
        && !listed(&config.complaint_categories, &new.complaint_category)
    {
        return Err(refused("wl_sales.returns.problem.category"));
    }
    if new.responsible_party.is_some()
        && !listed(&config.responsible_parties, &new.responsible_party)
    {
        return Err(refused("wl_sales.returns.problem.party"));
    }
    let (party, by_fulfilment, order) = match new.sale.subject_type.as_str() {
        COUNTER_SALE_SUBJECT_TYPE => {
            let sale = CounterSale::find_or_404(&state.db, new.sale.subject_id).await?;
            if sale.status != CounterSaleStatus::Completed {
                return Err(refused("wl_sales.returns.problem.not_completed"));
            }
            (sale.party_id, false, None)
        }
        SALES_ORDER_SUBJECT_TYPE => {
            let order = SalesOrder::find_or_404(&state.db, new.sale.subject_id).await?;
            if !matches!(
                order.status,
                SalesOrderStatus::PartiallyFulfilled
                    | SalesOrderStatus::Fulfilled
                    | SalesOrderStatus::Closed
            ) {
                return Err(refused("wl_sales.returns.problem.not_handed_over"));
            }
            (Some(order.party_id), true, Some(order))
        }
        _ => return Err(refused("wl_sales.returns.problem.sale_type")),
    };
    let lines = SaleLine::of(&state.db, &new.sale).await?;
    let no_goods = new.kind == ReturnKind::NoGoods;
    let (mut net, mut vat, mut eligible) = (0, 0, 0);
    // The rows to store: sale line, quantity, value with VAT.
    let mut rows: Vec<(i64, i64, i64)> = Vec::new();
    for (n, (line_id, quantity)) in new.lines.iter().enumerate() {
        let Some(line) = lines
            .iter()
            .find(|l| l.id == *line_id && l.kind == KIND_ITEM)
        else {
            return Err(refused("wl_sales.returns.problem.unknown_line"));
        };
        let repeated = new.lines[..n].iter().any(|(other, _)| other == line_id)
            || new.amounts.iter().any(|(other, _)| other == line_id);
        let value_net = ratio_half_up(line.net(), *quantity, line.quantity_milli);
        let value_vat = ratio_half_up(line.vat(), *quantity, line.quantity_milli);
        let over = if no_goods {
            // A value cap per line: what is left of the line.
            value_net + value_vat
                > worth_of(line, by_fulfilment) - earlier_value(&state.db, line).await?
        } else {
            let sold = if by_fulfilment {
                line.fulfilled_milli
            } else {
                line.quantity_milli
            };
            *quantity > sold - returned_before(&state.db, line.id).await?
        };
        if *quantity <= 0 || repeated || over {
            return Err(refused(if no_goods {
                "wl_sales.returns.problem.cap_line"
            } else {
                "wl_sales.returns.problem.quantity"
            }));
        }
        net += value_net;
        vat += value_vat;
        eligible += line.eligible_net_of(*quantity);
        rows.push((*line_id, *quantity, value_net + value_vat));
    }
    for (n, (line_id, amount)) in new.amounts.iter().enumerate() {
        let Some(line) = lines
            .iter()
            .find(|l| l.id == *line_id && l.kind == KIND_ITEM)
        else {
            return Err(refused("wl_sales.returns.problem.unknown_line"));
        };
        let repeated = new.amounts[..n].iter().any(|(other, _)| other == line_id);
        if *amount <= 0 || repeated {
            return Err(refused("wl_sales.returns.problem.amount"));
        }
        if *amount > worth_of(line, by_fulfilment) - earlier_value(&state.db, line).await? {
            return Err(refused("wl_sales.returns.problem.cap_line"));
        }
        let value_net = ratio_half_up(line.net(), *amount, line.gross().max(1));
        net += value_net;
        vat += amount - value_net;
        eligible += ratio_half_up(line.eligible_net(), *amount, line.gross().max(1));
        rows.push((*line_id, 0, *amount));
    }
    if no_goods
        && let Some(order) = &order
        && order.refunded_amount + net + vat > order.gross_total
    {
        return Err(refused("wl_sales.returns.problem.cap_order"));
    }
    let number = RETURN_SERIES
        .next_allowing_gaps(&state.db, None, None, today())
        .await?;
    let created = SalesReturn::create(
        &state.db,
        SalesReturn {
            number,
            sale_type: new.sale.subject_type.clone(),
            sale_id: new.sale.subject_id,
            party_id: party,
            kind: new.kind,
            reason: new.reason,
            net_value: net,
            vat_value: vat,
            gross_value: net + vat,
            eligible_net: eligible,
            requested_by: new.requested_by,
            complaint_category: new.complaint_category,
            responsible_party: new.responsible_party,
            ticket_id: new.ticket,
            ..SalesReturn::default()
        },
    )
    .await?;
    for (line_id, quantity, value) in rows {
        SalesReturnLine::create(
            &state.db,
            SalesReturnLine {
                return_id: created.id,
                sale_line_id: line_id,
                quantity_milli: quantity,
                refund_value: value,
                ..SalesReturnLine::default()
            },
        )
        .await?;
    }
    Ok(created)
}

/// The day (Asia/Dubai) the returned sale was completed or handed over.
async fn sale_date(state: &AppState, r: &SalesReturn) -> Result<NaiveDate> {
    let at = match r.sale_type.as_str() {
        COUNTER_SALE_SUBJECT_TYPE => CounterSale::find(&state.db, r.sale_id)
            .await?
            .and_then(|s| s.completed_at.or(s.created_at)),
        SALES_ORDER_SUBJECT_TYPE => SalesOrder::find(&state.db, r.sale_id)
            .await?
            .and_then(|o| o.fulfilled_at.or(o.confirmed_at).or(o.created_at)),
        _ => None,
    };
    Ok(at.map_or_else(today, dubai_date))
}

/// The goods of return `id` are back (`receive`); `approved` accepts them
/// outside the return window.
pub async fn receive(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    approved: bool,
) -> Result<SalesReturn> {
    let r = SalesReturn::find_or_404(&state.db, id).await?;
    let on = sale_date(state, &r).await?;
    let input = ReceiveInput {
        days_since_sale: (today() - on).num_days(),
        window_days: config_of(state).return_window_days,
        approved,
    };
    let _ = r
        .transition(RECEIVE)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    SalesReturn::find_or_404(&state.db, id).await
}

/// What a resolution did.
#[derive(Debug, Clone)]
pub enum ResolveOutcome {
    /// The return is resolved.
    Done(Box<SalesReturn>),
    /// A manager who is not the recorder must approve first; the return stays
    /// as it is.
    Waiting(Box<ApprovalRequest>),
}

/// The settings and the sale facts a resolution of `r` is checked against:
/// fills `by`, `not_aggregator`, `needs_approval` and `problem` of `input`.
async fn checked(
    state: &AppState,
    r: &SalesReturn,
    mut input: ResolveInput,
    by: Option<i64>,
) -> Result<ResolveInput> {
    let config = config_of(state);
    input.by = by;
    input.not_aggregator = if r.sale_type == SALES_ORDER_SUBJECT_TYPE {
        match SalesOrder::find(&state.db, r.sale_id).await? {
            Some(order) => !config.is_aggregator_channel(&order.channel),
            None => true,
        }
    } else {
        true
    };
    if r.kind == ReturnKind::NoGoods {
        if input.complaint_category.is_some()
            && !listed(&config.complaint_categories, &input.complaint_category)
        {
            input.problem = Some(english("wl_sales.returns.problem.category", &[]));
        } else if input.responsible_party.is_some()
            && !listed(&config.responsible_parties, &input.responsible_party)
        {
            input.problem = Some(english("wl_sales.returns.problem.party", &[]));
        }
    }
    // Money leaves the business: above the limit a manager must say yes
    // (`SAL.ACL.06`). A channel refund moves no money, a replacement none.
    input.needs_approval = r.kind == ReturnKind::NoGoods
        && matches!(
            input.resolution,
            Some(ReturnResolution::Refund | ReturnResolution::CreditNote)
        )
        && r.gross_value > config.refund_approval_limit;
    Ok(input)
}

/// Resolves return `id` (`resolve`). A refund or credit of a *no goods* return
/// above the limit of the settings does not resolve it: it asks for the
/// approval of a manager (never the recorder, `SAL.ACL.06`) and the return
/// stays `requested`.
///
/// # Errors
///
/// 403 without `returns.handle`; 422 when a guard of `resolve` refuses (no
/// resolution, no inspection or complaint, a channel refund on a goods return
/// or on an order of a channel that is not an aggregator's, a category or
/// party outside the settings), or a request for the return already waits.
pub async fn resolve(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    input: ResolveInput,
) -> Result<ResolveOutcome> {
    if !by.has_permission(permissions::RETURNS_HANDLE) {
        return Err(Error::Forbidden);
    }
    let r = SalesReturn::find_or_404(&state.db, id).await?;
    let input = checked(state, &r, input, Some(by.id)).await?;
    if input.needs_approval {
        return ask_approval(state, by, &r, &input)
            .await
            .map(ResolveOutcome::Waiting);
    }
    let _ = r
        .transition(RESOLVE)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    Ok(ResolveOutcome::Done(Box::new(
        SalesReturn::find_or_404(&state.db, id).await?,
    )))
}

/// Asks for the approval of the resolution `input` of `r`. The requester of
/// the approval is the user who recorded the return, so the recorder cannot
/// approve it.
async fn ask_approval(
    state: &AppState,
    by: &AuthUser,
    r: &SalesReturn,
    input: &ResolveInput,
) -> Result<Box<ApprovalRequest>> {
    let filled = |text: &Option<String>| text.as_deref().is_some_and(|t| !t.trim().is_empty());
    if !matches!(r.status, ReturnStatus::Requested | ReturnStatus::Received) {
        return Err(refused("wl_sales.returns.problem.state"));
    }
    if r.kind == ReturnKind::NoGoods
        && !(filled(&input.complaint_category) && filled(&input.responsible_party))
    {
        return Err(refused("wl_sales.returns.problem.complaint"));
    }
    if let Some(problem) = input.problem.clone() {
        return Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, problem));
    }
    let waiting = ApprovalRequest::pending_about(&state.db, &r.subject()).await?;
    if waiting
        .iter()
        .any(|request| request.action == ACTION_REFUND)
    {
        return Err(refused("wl_sales.returns.problem.pending"));
    }
    let payload = renox::serde_json::to_value(input)
        .map_err(|e| Error::Internal(renox::anyhow::Error::new(e)))?;
    let mut tx = state.db.begin().await?;
    let require = Require::new(
        ACTION_REFUND,
        r.subject(),
        RESOLVE,
        r.requested_by.unwrap_or(by.id),
        r.status.as_str(),
    )
    .payload(payload);
    match approvals::require_in(&mut tx, &require).await? {
        Required::Pending(request) => {
            tx.commit().await?;
            approvals::announce_request(state, &request).await?;
            Ok(request)
        }
        Required::Allowed => {
            tx.rollback().await?;
            Err(refused("wl_sales.returns.problem.no_policy"))
        }
    }
}

/// What raising a refund after completion did.
#[derive(Debug, Clone)]
pub enum RefundOutcome {
    /// The return is resolved: the refund is requested of wl-payments.
    Resolved(Box<SalesReturn>),
    /// A manager must approve before it resolves (`SAL.ACL.06`).
    Waiting(Box<SalesReturn>, Box<ApprovalRequest>),
    /// The user may request but not handle returns: a handler resolves it.
    Requested(Box<SalesReturn>),
}

/// Raises a *no goods* return from a completed order (`new`: the lines or
/// refunds by value, the complaint category and the responsible party) and
/// resolves it with a refund (`SAL.ACL.06`): at once when it is worth no more
/// than the limit of the settings, else after a manager's approval.
///
/// # Errors
///
/// 403 without `returns.request`; 422 for everything [`request`] and
/// [`resolve`] refuse, and when the category or the responsible party is
/// missing.
pub async fn refund_after_completion(
    state: &AppState,
    by: &AuthUser,
    new: NewReturn,
) -> Result<RefundOutcome> {
    if !by.has_permission(permissions::RETURNS_REQUEST) {
        return Err(Error::Forbidden);
    }
    let filled = |text: &Option<String>| text.as_deref().is_some_and(|t| !t.trim().is_empty());
    if !(filled(&new.complaint_category) && filled(&new.responsible_party)) {
        return Err(refused("wl_sales.returns.problem.complaint"));
    }
    let new = NewReturn {
        kind: ReturnKind::NoGoods,
        requested_by: Some(by.id),
        ..new
    };
    let created = request(state, new).await?;
    if !by.has_permission(permissions::RETURNS_HANDLE) {
        return Ok(RefundOutcome::Requested(Box::new(created)));
    }
    let input = ResolveInput {
        resolution: Some(ReturnResolution::Refund),
        complaint_category: created.complaint_category.clone(),
        responsible_party: created.responsible_party.clone(),
        ..ResolveInput::default()
    };
    match resolve(state, by, created.id, input).await? {
        ResolveOutcome::Done(done) => Ok(RefundOutcome::Resolved(done)),
        ResolveOutcome::Waiting(request) => Ok(RefundOutcome::Waiting(
            Box::new(SalesReturn::find_or_404(&state.db, created.id).await?),
            request,
        )),
    }
}

/// Records a refund the aggregator made to its customer itself (06: payout
/// statement `match`): raises the *no goods* return `new` and resolves it as
/// the system, *refunded by the channel*: no approval, refund or credit note;
/// the order's amount refunded rises and the cap still applies.
///
/// # Errors
///
/// 422 for everything [`request`] refuses, when the complaint category or the
/// responsible party is missing, and when the order is not of an aggregator
/// channel.
pub async fn refund_by_channel(state: &AppState, new: NewReturn) -> Result<SalesReturn> {
    let new = NewReturn {
        kind: ReturnKind::NoGoods,
        ..new
    };
    let created = request(state, new).await?;
    resolve_by_channel(state, created.id).await
}

/// Raises the *no goods* return `new` of a handed-over order and resolves it
/// as the system for the vertical (06: a delivery reversed after review),
/// with no approval: by a refund, or by credit note only when `credit_only`
/// (cash on delivery nobody paid: no refund payment is asked and no document
/// is named). The order's amount refunded rises, and the cap still applies.
///
/// # Errors
///
/// 422 for everything [`request`] refuses and when the complaint category or
/// the responsible party is missing.
pub async fn refund_by_vertical(
    state: &AppState,
    new: NewReturn,
    credit_only: bool,
) -> Result<SalesReturn> {
    let new = NewReturn {
        kind: ReturnKind::NoGoods,
        ..new
    };
    let created = request(state, new).await?;
    let resolution = if credit_only {
        ReturnResolution::CreditNote
    } else {
        ReturnResolution::Refund
    };
    let input = checked(
        state,
        &created,
        ResolveInput {
            resolution: Some(resolution),
            complaint_category: created.complaint_category.clone(),
            responsible_party: created.responsible_party.clone(),
            by_vertical: true,
            ..ResolveInput::default()
        },
        None,
    )
    .await?;
    let _ = created
        .transition(RESOLVE_SYSTEM)
        .by_system()
        .input(&input)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    SalesReturn::find_or_404(&state.db, created.id).await
}

/// Resolves the *no goods* return `id` as the system, *refunded by the
/// channel*, with the category and party it was raised with.
///
/// # Errors
///
/// 422 when the guards refuse it (no category or party, a goods return, an
/// order of a channel that is not an aggregator's).
pub async fn resolve_by_channel(state: &AppState, id: i64) -> Result<SalesReturn> {
    let r = SalesReturn::find_or_404(&state.db, id).await?;
    let input = checked(
        state,
        &r,
        ResolveInput {
            resolution: Some(ReturnResolution::RefundedByChannel),
            complaint_category: r.complaint_category.clone(),
            responsible_party: r.responsible_party.clone(),
            ..ResolveInput::default()
        },
        None,
    )
    .await?;
    let _ = r
        .transition(RESOLVE_SYSTEM)
        .by_system()
        .input(&input)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    SalesReturn::find_or_404(&state.db, id).await
}

/// Takes the channel refund `id` back as the system (06: payout statement
/// `void`, or a dispute the aggregator won): the order's amount refunded goes
/// down by the return's value; no money and no document moves.
///
/// # Errors
///
/// 422 when the reason is blank, the return is not resolved, or was not
/// resolved *refunded by the channel*.
pub async fn reverse_by_channel(state: &AppState, id: i64, reason: &str) -> Result<SalesReturn> {
    let r = SalesReturn::find_or_404(&state.db, id).await?;
    let _ = r
        .transition(REVERSE_SYSTEM)
        .by_system()
        .reason(reason.trim().to_owned())
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    SalesReturn::find_or_404(&state.db, id).await
}

/// Rejects return `id` with a reason (`reject`).
pub async fn reject(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<SalesReturn> {
    let r = SalesReturn::find_or_404(&state.db, id).await?;
    let _ = r
        .transition(REJECT)
        .by(by)
        .reason(reason.trim().to_owned())
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    SalesReturn::find_or_404(&state.db, id).await
}

/// Reverses the *refunded by the channel* return `id` (`reverse`).
pub async fn reverse(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
) -> Result<SalesReturn> {
    let r = SalesReturn::find_or_404(&state.db, id).await?;
    let _ = r
        .transition(REVERSE)
        .by(by)
        .reason(reason.trim().to_owned())
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    SalesReturn::find_or_404(&state.db, id).await
}

/// Emits the kernel events after a return transition committed.
async fn announce(state: &AppState, event: &Transitioned<SalesReturn>) -> Result {
    let name = event.transition.as_str();
    let resolved = [RESOLVE, RESOLVE_APPROVED, RESOLVE_SYSTEM].contains(&name);
    let reversed = [REVERSE, REVERSE_SYSTEM].contains(&name);
    if !resolved && !reversed {
        return Ok(());
    }
    let Some(r) = SalesReturn::find(&state.db, event.key).await? else {
        return Ok(());
    };
    if resolved {
        state
            .emit(ReturnResolved {
                sales_return: r.return_id(),
                kind: r.kind.as_str().to_owned(),
                sale: r.sale(),
                resolution: r
                    .resolution
                    .map(|x| x.as_str().to_owned())
                    .unwrap_or_default(),
                value: r.gross_value,
                net_value: r.net_value,
                credit_note: r.credit_note_id,
                complaint_category: r.complaint_category.clone(),
                responsible_party: r.responsible_party.clone(),
            })
            .await?;
        pay_out(state, &r).await
    } else {
        state
            .emit(ReturnReversed {
                sales_return: r.return_id(),
                sale: r.sale(),
                value: r.gross_value,
                reason: r.closing_reason.clone().unwrap_or_default(),
            })
            .await
    }
}

/// What follows the resolution of a *no goods* return once it committed: the
/// refund asked of wl-payments (a failure leaves a failed row on the order
/// for a person) and the comment on the complaint ticket it came from.
async fn pay_out(state: &AppState, r: &SalesReturn) -> Result {
    if r.kind != ReturnKind::NoGoods {
        return Ok(());
    }
    if r.resolution == Some(ReturnResolution::Refund)
        && r.sale_type == SALES_ORDER_SUBJECT_TYPE
        && let Some(order) = SalesOrder::find(&state.db, r.sale_id).await?
    {
        crate::settle::refund(
            state,
            &order,
            None,
            r.gross_value,
            english("wl_sales.settle.reason_return", &[]),
            r.handled_by,
        )
        .await?;
    }
    if let Some(ticket) = r.ticket_id {
        let value = format!("AED {}", crate::notices::aed(r.gross_value));
        let resolution = r
            .resolution
            .map(|x| x.as_str().to_owned())
            .unwrap_or_default();
        let note = english(
            "wl_sales.returns.ticket_comment",
            &[
                ("number", &r.number),
                ("resolution", &resolution),
                ("value", &value),
            ],
        );
        task_creator(state)
            .comment(state, TicketId(ticket), TaskComment::internal(&note))
            .await?;
    }
    Ok(())
}

/// The final check of a request to resolve a return: it must still be open,
/// and what it refunds must still fit the lines and the order (a return made
/// meanwhile may have taken the room). A message ends the request.
async fn recheck(tx: &mut Transaction, request: &ApprovalRequest) -> Result<Option<String>> {
    let Some(r) = SalesReturn::find(&mut *tx, request.subject_id).await? else {
        return Ok(Some(english("wl_sales.returns.problem.state", &[])));
    };
    if !matches!(r.status, ReturnStatus::Requested | ReturnStatus::Received) {
        return Ok(Some(english("wl_sales.returns.problem.state", &[])));
    }
    if r.kind != ReturnKind::NoGoods {
        return Ok(None);
    }
    let by_fulfilment = r.sale_type == SALES_ORDER_SUBJECT_TYPE;
    let rows = SalesReturnLine::query()
        .where_(SalesReturnLine::RETURN_ID.eq(r.id))
        .get(&mut *tx)
        .await?;
    for row in rows {
        let Some(line) = SaleLine::query()
            .where_(SaleLine::ID.eq(row.sale_line_id))
            .first(&mut *tx)
            .await?
        else {
            continue;
        };
        let others = earlier_value_in(tx, &line, r.id).await?;
        if others + value_of(&row, &line) > worth_of(&line, by_fulfilment) {
            return Ok(Some(english("wl_sales.returns.problem.cap_line", &[])));
        }
    }
    if by_fulfilment
        && let Some(order) = SalesOrder::find(&mut *tx, r.sale_id).await?
        && order.refunded_amount + r.gross_value > order.gross_total
    {
        return Ok(Some(english("wl_sales.returns.problem.cap_order", &[])));
    }
    Ok(None)
}

/// Registers the machine, the listener, the approval wiring of a refund after
/// completion (the kernel runs `resolve_approved` at the last yes) and the
/// link from "Needs your action" to the return.
pub(crate) fn register(app: &mut Registry) {
    renox::state_machine::register::<SalesReturn>();
    app.listen(
        |event: Transitioned<SalesReturn>, state: AppState| async move {
            announce(&state, &event).await
        },
    );
    wl_kernel::SubjectMachine::of::<SalesReturn>(SALES_RETURN_SUBJECT_TYPE)
        .scope(|_r: &SalesReturn| renox::auth::permissions::Scope::global())
        .register();
    crate::policy::refund_policy().register();
    approvals::register_final_check(SALES_RETURN_SUBJECT_TYPE, |tx, request| {
        Box::pin(recheck(tx, request))
    });
    approvals::register_subject_link(SALES_RETURN_SUBJECT_TYPE, |id| {
        format!("/sales-returns/{id}")
    });
}
