//! The counter sale services (wl-sales §4.3, §5.4; base for BL-SAL-22):
//! open a cart, add lines, complete it with the tenders, and announce
//! [`CounterSaleCompleted`] after the commit.
//!
//! Decisions:
//! - The tender payments are recorded by the till in `wl-payments`; the sale
//!   keeps the payment id of each tender (see [`crate::counter_sale`]).
//! - The invoice is drafted for a sale that names a customer, with the sale
//!   as its source and every line's origin; issuing it (the simplified tax
//!   invoice) is `wl-invoicing`'s, by its auto-issue setting for the source
//!   `sales.counter_sale`. A walk-in sale has no customer to bill, so it has
//!   no invoice.

use renox::prelude::*;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::invoicing::{DEFAULT_CURRENCY, InvoiceDraft, invoicer};
use wl_kernel::Subject;
use wl_kernel::sales::{COUNTER_SALE_SUBJECT_TYPE, CounterSaleCompleted};
use wl_kernel::PartyId;

use crate::config::config_of;
use crate::counter_sale::{CounterSale, RECEIPT_SERIES};
use crate::counter_sale_machine::{COMPLETE, CompleteInput, Tender};
use crate::sale_line::{NewLine, SaleLine, totals};
use crate::texts::english;

/// A 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// A cart to open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct NewCounterSale {
    /// The customer, when the sale names one.
    pub party: Option<PartyId>,
    /// The till session it is rung up on.
    pub till_session: Option<i64>,
    /// The legal entity whose sale it is.
    pub legal_entity: Option<i64>,
}

impl NewCounterSale {
    /// A walk-in cart.
    pub fn new() -> Self {
        Self::default()
    }

    /// For the customer `party`.
    pub fn party(mut self, party: PartyId) -> Self {
        self.party = Some(party);
        self
    }

    /// On the till session `id`.
    pub fn till_session(mut self, id: i64) -> Self {
        self.till_session = Some(id);
        self
    }

    /// For the legal entity `id`.
    pub fn legal_entity(mut self, id: i64) -> Self {
        self.legal_entity = Some(id);
        self
    }
}

/// Opens a cart for the cashier `cashier` (a user id).
pub async fn open(state: &AppState, cashier: i64, new: NewCounterSale) -> Result<CounterSale> {
    let number = RECEIPT_SERIES
        .next_allowing_gaps(&state.db, None, None, wl_kernel::documents::today())
        .await?;
    CounterSale::create(
        &state.db,
        CounterSale {
            number,
            legal_entity_id: new.legal_entity,
            party_id: new.party.map(|p| p.0),
            till_session_id: new.till_session,
            cashier_user_id: cashier,
            currency: DEFAULT_CURRENCY.to_owned(),
            ..CounterSale::default()
        },
    )
    .await
}

/// Makes the stored totals of the open sale `sale` those of its lines.
pub(crate) async fn refresh_totals(state: &AppState, sale: &CounterSale) -> Result<CounterSale> {
    let lines = SaleLine::of(&state.db, &sale.subject()).await?;
    let t = totals(&lines);
    CounterSale::query()
        .where_(CounterSale::ID.eq(sale.id))
        .update(
            &state.db,
            &[
                ("net_total", &t.net),
                ("vat_total", &t.vat),
                ("gross_total", &t.gross),
            ],
        )
        .await?;
    CounterSale::find_or_404(&state.db, sale.id).await
}

/// Adds `line` to the open sale `id`.
///
/// # Errors
///
/// 422 when the sale is completed, the quantity is not positive, or an item
/// has a negative price.
pub async fn add_line(state: &AppState, id: i64, line: NewLine) -> Result<CounterSale> {
    let sale = CounterSale::find_or_404(&state.db, id).await?;
    if !sale.is_open() {
        return Err(refused("wl_sales.counter.problem.not_open"));
    }
    if line.quantity_milli <= 0 || (line.kind == crate::sale_line::KIND_ITEM && line.unit_price < 0)
    {
        return Err(refused("wl_sales.counter.problem.line"));
    }
    SaleLine::add(&state.db, &sale.subject(), line).await?;
    refresh_totals(state, &sale).await
}

/// Completes the sale `id` as `by` with `tenders` (`complete`), then drafts
/// its invoice when it names a customer.
///
/// # Errors
///
/// 422 from the guards: no lines, tenders that do not add up to the total,
/// an unknown method, an account tender where it is not allowed.
pub async fn complete(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    tenders: Vec<Tender>,
) -> Result<CounterSale> {
    // The hold on the member's points is checked again; a lapsed hold or a
    // shortfall takes the redemption line off before the total is read.
    crate::redeem::prepare(state, &Subject::new(COUNTER_SALE_SUBJECT_TYPE, id)).await?;
    let sale = CounterSale::find_or_404(&state.db, id).await?;
    let lines = SaleLine::of(&state.db, &sale.subject()).await?;
    let t = totals(&lines);
    let input = CompleteInput {
        tenders,
        line_count: lines.len() as i64,
        net: t.net,
        vat: t.vat,
        gross: t.gross,
        allow_account: config_of(state).counter_sales_on_account,
    };
    let _ = sale
        .transition(COMPLETE)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    let done = CounterSale::find_or_404(&state.db, id).await?;
    draft_invoice(state, &done, &lines).await?;
    CounterSale::find_or_404(&state.db, id).await
}

/// Drafts the invoice of a completed sale that names a customer and stores
/// its id on the sale; nothing when invoicing is off.
async fn draft_invoice(state: &AppState, sale: &CounterSale, lines: &[SaleLine]) -> Result {
    let Some(party) = sale.party() else {
        return Ok(());
    };
    let mut draft = InvoiceDraft::new(party, sale.subject()).currency(&sale.currency);
    if let Some(entity) = sale.legal_entity_id {
        draft = draft.legal_entity(entity);
    }
    for line in lines {
        draft = draft.line(line.draft_line().origin(sale.subject()));
    }
    if let Some(invoice) = invoicer(state).draft(&state.db, draft).await? {
        CounterSale::query()
            .where_(CounterSale::ID.eq(sale.id))
            .update(&state.db, &[("invoice_id", &Some(invoice.0))])
            .await?;
    }
    Ok(())
}

/// Emits [`CounterSaleCompleted`] after the commit of `complete`.
async fn announce(state: &AppState, event: &Transitioned<CounterSale>) -> Result {
    if event.transition != COMPLETE {
        return Ok(());
    }
    let Some(sale) = CounterSale::find(&state.db, event.key).await? else {
        return Ok(());
    };
    let mut by_method: Vec<(String, i64)> = Vec::new();
    for payment in sale.payments(&state.db).await? {
        match by_method.iter_mut().find(|(m, _)| *m == payment.method) {
            Some((_, total)) => *total += payment.amount,
            None => by_method.push((payment.method.clone(), payment.amount)),
        }
    }
    state
        .emit(CounterSaleCompleted {
            sale: sale.sale_id(),
            party: sale.party(),
            till_session: sale.till_session_id,
            gross_total: sale.gross_total,
            by_method,
        })
        .await
}

/// Registers the machine and the listener.
pub(crate) fn register(app: &mut Registry) {
    renox::state_machine::register::<CounterSale>();
    app.listen(|event: Transitioned<CounterSale>, state: AppState| async move {
        announce(&state, &event).await
    });
}
