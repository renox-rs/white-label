//! The sales return services (wl-sales §4.5, §5.6, §6; base for
//! BL-SAL-22): request a return of a completed counter sale or of an order
//! that was handed over, receive the goods, resolve, reject or reverse it.
//! Every status change is a transition of [`crate::sales_return_machine`];
//! [`ReturnResolved`] and [`ReturnReversed`] follow the commit.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::Subject;
use wl_kernel::documents::{dubai_date, today};
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
        SalesReturnLine::create(
            &state.db,
            SalesReturnLine {
                return_id: created.id,
                sale_line_id: line_id,
                quantity_milli: quantity,
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
    app.listen(
        |event: Transitioned<SalesReturn>, state: AppState| async move {
            announce(&state, &event).await
        },
    );
}
