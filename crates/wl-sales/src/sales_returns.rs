//! The sales return services (wl-sales §4.5, §5.6, §6; base for
//! BL-SAL-22): request a return of a completed counter sale or of an order
//! that was handed over, receive the goods, resolve, reject or reverse it.
//! Every status change is a transition of [`crate::sales_return_machine`];
//! [`ReturnResolved`] and [`ReturnReversed`] follow the commit.

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::Subject;
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::inventory::{LOT_SUBJECT_TYPE, RECALL_SUBJECT_TYPE, ReturnedGoods, lot_trace};
use wl_kernel::sales::{
    COUNTER_SALE_SUBJECT_TYPE, ReturnResolved, ReturnReversed, SALES_ORDER_SUBJECT_TYPE,
};

use crate::config::config_of;
use crate::counter_sale::{CounterSale, CounterSaleStatus};
use crate::sale_line::{KIND_ITEM, SaleLine, ratio_half_up};
use crate::sales_order::{SalesOrder, SalesOrderStatus};
use crate::sales_return::{RETURN_SERIES, ReturnKind, ReturnStatus, SalesReturn, SalesReturnLine};
use crate::sales_return_machine::{RECEIVE, REJECT, RESOLVE, REVERSE, ReceiveInput, ResolveInput};
use crate::texts::english;

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
    /// The lot a customer presents for a sale line: (sale line, lot id).
    pub lots: Vec<(i64, i64)>,
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
            lots: Vec::new(),
        }
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

    /// The customer presents lot `lot_id` for the sale line `line`.
    pub fn lot(mut self, line: i64, lot_id: i64) -> Self {
        self.lots.push((line, lot_id));
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

/// Requests a return.
///
/// # Errors
///
/// 422 when the sale is not a counter sale or an order, was not completed or
/// handed over, a line is not an item of the sale, or a quantity is not above
/// zero or exceeds what was sold less what earlier returns take back.
pub async fn request(state: &AppState, new: NewReturn) -> Result<SalesReturn> {
    if new.lines.is_empty() {
        return Err(refused("wl_sales.returns.problem.no_lines"));
    }
    let (party, by_fulfilment) = match new.sale.subject_type.as_str() {
        COUNTER_SALE_SUBJECT_TYPE => {
            let sale = CounterSale::find_or_404(&state.db, new.sale.subject_id).await?;
            if sale.status != CounterSaleStatus::Completed {
                return Err(refused("wl_sales.returns.problem.not_completed"));
            }
            (sale.party_id, false)
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
            (Some(order.party_id), true)
        }
        _ => return Err(refused("wl_sales.returns.problem.sale_type")),
    };
    let lines = SaleLine::of(&state.db, &new.sale).await?;
    let (mut net, mut vat, mut eligible) = (0, 0, 0);
    for (n, (line_id, quantity)) in new.lines.iter().enumerate() {
        let Some(line) = lines
            .iter()
            .find(|l| l.id == *line_id && l.kind == KIND_ITEM)
        else {
            return Err(refused("wl_sales.returns.problem.unknown_line"));
        };
        let repeated = new.lines[..n].iter().any(|(other, _)| other == line_id);
        let sold = if by_fulfilment {
            line.fulfilled_milli
        } else {
            line.quantity_milli
        };
        let left = sold - returned_before(&state.db, line.id).await?;
        if *quantity <= 0 || repeated || *quantity > left {
            return Err(refused("wl_sales.returns.problem.quantity"));
        }
        net += ratio_half_up(line.net(), *quantity, line.quantity_milli);
        vat += ratio_half_up(line.vat(), *quantity, line.quantity_milli);
        eligible += line.eligible_net_of(*quantity);
    }
    if new
        .lots
        .iter()
        .any(|(line, _)| !new.lines.iter().any(|(other, _)| other == line))
    {
        return Err(refused("wl_sales.returns.problem.unknown_line"));
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
            ..SalesReturn::default()
        },
    )
    .await?;
    for (line_id, quantity) in new.lines {
        let lot_id = new
            .lots
            .iter()
            .find(|(line, _)| *line == line_id)
            .map(|(_, lot)| *lot);
        SalesReturnLine::create(
            &state.db,
            SalesReturnLine {
                return_id: created.id,
                sale_line_id: line_id,
                quantity_milli: quantity,
                lot_id,
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

/// The scheduled task that joins recalled return lines that did not join.
pub const RECALL_JOIN_TASK: &str = "sales:return-recall-join";
/// How often it runs.
pub const RECALL_JOIN_EVERY_MINUTES: u64 = 15;

/// The goods of one return line, as the stock's trace takes them.
fn goods_of(
    r: &SalesReturn,
    line: &SalesReturnLine,
    sold: &SaleLine,
    on: NaiveDate,
) -> ReturnedGoods {
    let mut goods =
        ReturnedGoods::new(&sold.item_code, line.quantity_milli, r.subject()).sold_on(on);
    if let Some(lot_id) = line.lot_id {
        goods = goods.lot(Subject::new(LOT_SUBJECT_TYPE, lot_id));
    }
    goods
}

/// The lines of `r` that are under a recall (BL-SAL-15), each with the recall
/// it is in: a lot in the scope of an open recall, or an item not tracked by
/// lot that is in an open recall by received dates and was sold inside its
/// trace window (`on` is the day of the sale). Also the number of lines.
async fn recalled_goods(
    state: &AppState,
    r: &SalesReturn,
    on: NaiveDate,
) -> Result<(Vec<(i64, Subject)>, usize)> {
    let lines = r.lines(&state.db).await?;
    let trace = lot_trace(state);
    let mut found = Vec::new();
    for line in &lines {
        let Some(sold) = SaleLine::find(&state.db, line.sale_line_id).await? else {
            continue;
        };
        let goods = goods_of(r, line, &sold, on);
        if let Some(recall) = trace.recall_of(state, &goods).await? {
            found.push((line.id, recall));
        }
    }
    Ok((found, lines.len()))
}

/// Takes the recalled lines of the received return `id` that have not joined
/// yet into quarantine and onto their recall's hold, line by line: a line that
/// joined is marked, so a retry after a failure never takes it in twice. The
/// return links to the recall once one line joined. Returns whether every
/// recalled line has joined.
///
/// # Errors
///
/// The stock's errors for the first line that could not join (the later lines
/// wait for the next call), and the database's.
pub async fn join_recalled(state: &AppState, id: i64) -> Result<bool> {
    let r = SalesReturn::find_or_404(&state.db, id).await?;
    if r.status != ReturnStatus::Received {
        return Ok(false);
    }
    let on = sale_date(state, &r).await?;
    let trace = lot_trace(state);
    let mut failure = None;
    for line in r.lines(&state.db).await? {
        let (Some(recall_id), None) = (line.recall_pending_id, line.recall_joined_at) else {
            continue;
        };
        let Some(sold) = SaleLine::find(&state.db, line.sale_line_id).await? else {
            continue;
        };
        let goods = goods_of(&r, &line, &sold, on);
        let recall = Subject::new(RECALL_SUBJECT_TYPE, recall_id);
        match trace.receive_returned(state, &recall, &goods).await {
            Ok(true) => {
                SalesReturnLine::query()
                    .where_(SalesReturnLine::ID.eq(line.id))
                    .update(&state.db, &[("recall_joined_at", &Some(now()))])
                    .await?;
            }
            Ok(false) => {
                // Not taken in: nothing to retry.
                let none: Option<i64> = None;
                SalesReturnLine::query()
                    .where_(SalesReturnLine::ID.eq(line.id))
                    .update(&state.db, &[("recall_pending_id", &none)])
                    .await?;
            }
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    let lines = r.lines(&state.db).await?;
    let joined = lines
        .iter()
        .find(|line| line.recall_joined_at.is_some())
        .and_then(|line| line.recall_pending_id);
    if let (Some(recall_id), None) = (joined, r.recall_id) {
        SalesReturn::query()
            .where_(SalesReturn::ID.eq(id))
            .update(&state.db, &[("recall_id", &Some(recall_id))])
            .await?;
    }
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(true)
}

/// The scheduled retry: every received return with a recalled line that did
/// not join yet is joined again ([`join_recalled`]). Returns the returns that
/// were done.
///
/// # Errors
///
/// The database's errors, and the first return that failed again after all
/// were tried.
pub async fn retry_recall_joins(state: &AppState) -> Result<Vec<i64>> {
    let pending = SalesReturnLine::query()
        .where_(SalesReturnLine::RECALL_PENDING_ID.is_not_null())
        .where_(SalesReturnLine::RECALL_JOINED_AT.is_null())
        .order_by(SalesReturnLine::RETURN_ID)
        .get(&state.db)
        .await?;
    let mut ids: Vec<i64> = pending.iter().map(|line| line.return_id).collect();
    ids.dedup();
    let mut done = Vec::new();
    let mut failure = None;
    for id in ids {
        match join_recalled(state, id).await {
            Ok(true) => done.push(id),
            Ok(false) => {}
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    failure.map_or(Ok(done), Err)
}

/// The goods of return `id` are back (`receive`); `approved` accepts them
/// outside the return window. Goods under an open recall are always accepted
/// (no approval, whatever the window): they go to quarantine and onto the
/// recall's stock hold through the kernel's `LotTrace`, the return links to the
/// recall, and the recall decides their outcome (BL-SAL-15). Resolving such a
/// return credits or refunds the customer but never restores stock
/// ([`SalesReturn::stock_follows_the_return`]).
///
/// The recall each recalled line is to join is written before the transition,
/// and the lines join after it ([`join_recalled`]). If taking the goods in
/// fails the return stays received, the failure is not raised to the clerk (the
/// goods are theirs to hand over either way) and the scheduled
/// [`retry_recall_joins`] joins the line later.
///
/// A return with some lines under a recall and some not needs the approval for
/// the window as before; only the recalled lines join the hold. Receiving the
/// ordinary lines into quarantine is BL-SAL-22's.
pub async fn receive(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    approved: bool,
) -> Result<SalesReturn> {
    let r = SalesReturn::find_or_404(&state.db, id).await?;
    let on = sale_date(state, &r).await?;
    let (recalled, total) = recalled_goods(state, &r, on).await?;
    let input = ReceiveInput {
        days_since_sale: (today() - on).num_days(),
        window_days: config_of(state).return_window_days,
        approved: approved || (total > 0 && recalled.len() == total),
    };
    for (line_id, recall) in &recalled {
        SalesReturnLine::query()
            .where_(SalesReturnLine::ID.eq(*line_id))
            .update(
                &state.db,
                &[("recall_pending_id", &Some(recall.subject_id))],
            )
            .await?;
    }
    let _ = r
        .transition(RECEIVE)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    if !recalled.is_empty() {
        // A failure waits for the scheduled retry; the return is received.
        let _ = join_recalled(state, id).await;
    }
    SalesReturn::find_or_404(&state.db, id).await
}

/// Resolves return `id` (`resolve`).
pub async fn resolve(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    input: ResolveInput,
) -> Result<SalesReturn> {
    let r = SalesReturn::find_or_404(&state.db, id).await?;
    let _ = r
        .transition(RESOLVE)
        .by(by)
        .input(&input)
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
    if name != RESOLVE && name != REVERSE {
        return Ok(());
    }
    let Some(r) = SalesReturn::find(&state.db, event.key).await? else {
        return Ok(());
    };
    if name == RESOLVE {
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
                credit_note: r.credit_note_id,
            })
            .await
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

/// Registers the machine and the listener.
pub(crate) fn register(app: &mut Registry) {
    renox::state_machine::register::<SalesReturn>();
    app.schedule().every_minutes(
        RECALL_JOIN_EVERY_MINUTES,
        RECALL_JOIN_TASK,
        |state: AppState| async move { retry_recall_joins(&state).await.map(|_| ()) },
    );
    app.listen(
        |event: Transitioned<SalesReturn>, state: AppState| async move {
            announce(&state, &event).await
        },
    );
}
