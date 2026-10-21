//! Earning and taking back loyalty points (wl-sales §5.9 steps 2 and 4,
//! `SAL.RUL.07`, `SAL.RUL.09`; BL-SAL-22).
//!
//! **Earning.** A qualifying sale earns once, for a member who was attached
//! before it completed ([`crate::loyalty_attach`]): a completed counter sale
//! (`CounterSaleCompleted`), a sales order reaching `fulfilled`
//! (`SalesOrderFulfilled` with `complete`), or a paid invoice that was not
//! made from an order or a counter sale (`InvoicePaid`). The points are
//! computed on the eligible amount: what the member paid or owes before VAT,
//! without excluded items, the redeemed value and the insurer's share, rounded
//! down; a per-visit programme counts one visit and makes the reward
//! available when the visits reach the rate.
//!
//! **Taking back.** A sales return, a credit note, a payment reversal (or
//! charge-back), a reopened or written-off invoice and a reversed fulfilment
//! reverse the earned points in proportion, rounded up and never more than
//! was earned; the balance may go below zero. A reversal happens once per
//! cause; a money cause (payment, write-off, fulfilment) is earned again, with
//! a new lot, when the sale is paid or fulfilled again.
//!
//! Decisions:
//! - The proportion of a payment reversal, a reopening or a write-off is the
//!   amount ÷ what the member paid or owes with VAT (`basis_gross`); of a
//!   return or a fulfilment reversal, the returned eligible value ÷ the
//!   eligible value; of a credit note, its net ÷ the eligible value.
//! - A paid invoice that is not an order's or a counter sale's earns for the
//!   party it bills (an open account of that party); the eligible amount is
//!   its net total less the net claimed from the insurer on the payer bills
//!   of the same source (`FIN.TPP.01`). Package credits as a means of payment
//!   (also `FIN.TPP.01`) do not exist yet and earn as cash.
//! - An account over the daily limit still earns; it is listed for review.
//! - A closed account earns nothing and a return of a sale earned before the
//!   closing reverses nothing; a paused one (party blocked, BL-SAL-26) earns
//!   nothing, and nothing is earned later for sales made while paused.

use renox::chrono::{Months, NaiveDate};
use renox::db::Transaction;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::invoicing::{
    CreditNoteIssued, INVOICE_SUBJECT_TYPE, InvoicePaid, InvoiceReopened, InvoiceWrittenOff,
    ReopenReason, invoicer,
};
use wl_kernel::payer_bills::payer_bills;
use wl_kernel::payments::{PaymentReceived, PaymentReversed};
use wl_kernel::sales::{
    COUNTER_SALE_SUBJECT_TYPE, CounterSaleCompleted, PointsEarned, PointsReversed, ReturnResolved,
    SALES_ORDER_SUBJECT_TYPE, SalesOrderFulfilled, SalesOrderFulfilmentReversed,
};

use crate::config::config_of;
use crate::counter_sale::CounterSale;
use crate::loyalty_account::LoyaltyAccount;
use crate::loyalty_points::{
    CAUSE_CREDIT_NOTE, CAUSE_EARN, CAUSE_FULFILMENT_REVERSED, CAUSE_PAYMENT_REVERSED, CAUSE_REEARN,
    CAUSE_RETURN, CAUSE_WRITTEN_OFF, LoyaltyEarning, LoyaltyReview, PointsKind, PointsMovement,
};
use crate::loyalty_programme::{EarnKind, LoyaltyProgramme};
use crate::notices;
use crate::sale_line::{SaleLine, ratio_half_up};
use crate::sales_order::SalesOrder;
use crate::sales_return::SalesReturn;
use crate::texts::english;

/// What a sale offers the programme.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SaleFacts {
    /// The sale: a counter sale, a sales order, an invoice.
    pub source: Subject,
    /// The loyalty account attached to it.
    pub account: i64,
    /// What counts before VAT, in fils.
    pub eligible_net: i64,
    /// What the member paid or owes with VAT, in fils.
    pub basis_gross: i64,
    /// The legal entity of the sale.
    pub entity: Option<i64>,
    /// The day (Asia/Dubai) the sale completed.
    pub on: NaiveDate,
}

impl SaleFacts {
    /// A sale of `source` for the account `account`.
    pub fn new(
        source: Subject,
        account: i64,
        eligible_net: i64,
        basis_gross: i64,
        on: NaiveDate,
    ) -> Self {
        SaleFacts {
            source,
            account,
            eligible_net,
            basis_gross,
            entity: None,
            on,
        }
    }

    /// Of the legal entity `id`.
    pub fn entity(mut self, id: Option<i64>) -> Self {
        self.entity = id;
        self
    }
}

/// What [`earn`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Earned {
    /// Points (or a visit) were earned; `reward` when the visit made one
    /// available.
    Earned {
        /// The points, or 1 for a visit.
        points: i64,
        /// Whether a reward became available.
        reward: bool,
    },
    /// The sale earned before; nothing changed.
    Already,
    /// The sale does not earn, and why.
    Skipped(&'static str),
}

/// What [`reverse_earn`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reversed {
    /// This many points (or visits) were taken back.
    Reversed(i64),
    /// This cause reversed before; nothing changed.
    Already,
    /// There was nothing to take back.
    Nothing,
}

/// The expiry of a lot earned on `on`: `months` later.
pub(crate) fn expiry(on: NaiveDate, months: i64) -> NaiveDate {
    on.checked_add_months(Months::new(u32::try_from(months).unwrap_or(12)))
        .unwrap_or(on)
}

/// What the member paid or owes with VAT for the part of the sale that
/// earns: `gross × eligible ÷ net`, or `gross` when the sale has no net.
pub fn basis_of(gross: i64, eligible_net: i64, net: i64) -> i64 {
    if net > 0 && eligible_net >= 0 {
        ratio_half_up(gross, eligible_net.min(net), net)
    } else {
        gross
    }
}

/// `value ÷ divisor` rounded up, for positive numbers.
fn ceil_ratio(value: i64, numerator: i64, denominator: i64) -> i64 {
    if denominator <= 0 || numerator <= 0 || value <= 0 {
        return 0;
    }
    let num = i128::from(value) * i128::from(numerator);
    let den = i128::from(denominator);
    i64::try_from((num + den - 1) / den).unwrap_or(0)
}

/// The sale an invoice bills: the counter sale or order it was made from, else
/// the invoice itself.
fn sale_of_invoice(source: &Subject, invoice: i64) -> Subject {
    match source.subject_type.as_str() {
        COUNTER_SALE_SUBJECT_TYPE | SALES_ORDER_SUBJECT_TYPE => source.clone(),
        _ => Subject::new(INVOICE_SUBJECT_TYPE, invoice),
    }
}

/// What a credit note on `invoice` belongs to: the counter sale or order
/// whose invoice it is, else the invoice.
async fn sale_of_invoice_id(db: &Db, invoice: i64) -> Result<Subject> {
    if let Some(sale) = CounterSale::of_invoice(db, invoice).await? {
        return Ok(sale.subject());
    }
    if let Some(order) = SalesOrder::of_invoice(db, invoice).await? {
        return Ok(order.subject());
    }
    Ok(Subject::new(INVOICE_SUBJECT_TYPE, invoice))
}

/// The movement of one change, ready to insert.
struct NewMovement<'a> {
    account: &'a LoyaltyAccount,
    kind: PointsKind,
    points: i64,
    value: i64,
    entity: Option<i64>,
    source: &'a Subject,
    earning: i64,
    cause: &'static str,
}

impl NewMovement<'_> {
    fn into_row(self) -> PointsMovement {
        PointsMovement {
            account_id: self.account.id,
            kind: self.kind,
            points: self.points,
            value: self.value,
            legal_entity_id: self.entity,
            source_type: self.source.subject_type.clone(),
            source_id: self.source.subject_id,
            earning_id: Some(self.earning),
            cause: self.cause.to_owned(),
            ..PointsMovement::default()
        }
    }
}

/// What [`credit`] made.
struct Credited {
    /// The earn movement: the lot.
    movement: i64,
    /// Whether the visits reached the reward.
    reward: Option<NaiveDate>,
}

/// Adds `points` (or one visit) to the account inside `tx`: the earn lot,
/// the balance and, for visits, the counter and the reward.
#[allow(clippy::too_many_arguments)]
async fn credit(
    tx: &mut Transaction,
    programme: &LoyaltyProgramme,
    account: &LoyaltyAccount,
    points: i64,
    source: &Subject,
    earning: i64,
    cause: &'static str,
    on: NaiveDate,
    entity: Option<i64>,
) -> Result<Credited> {
    let per_aed = programme.earn_kind == EarnKind::PerAed;
    let mut lot = NewMovement {
        account,
        kind: PointsKind::Earn,
        points,
        value: if per_aed {
            points * programme.point_value
        } else {
            0
        },
        entity,
        source,
        earning,
        cause,
    }
    .into_row();
    lot.lot_expires_on = Some(expiry(on, programme.validity_months));
    lot.insert(&mut *tx).await?;
    let mut reward = None;
    if per_aed {
        LoyaltyAccount::query()
            .where_(LoyaltyAccount::ID.eq(account.id))
            .increment(&mut *tx, "points_balance", points)
            .await?;
    } else {
        LoyaltyAccount::query()
            .where_(LoyaltyAccount::ID.eq(account.id))
            .increment(&mut *tx, "visits", points)
            .await?;
        let fresh = LoyaltyAccount::find(&mut *tx, account.id).await?;
        if let Some(fresh) = fresh
            && fresh.visits >= programme.earn_rate
        {
            let until = expiry(on, programme.validity_months);
            let rest: i64 = fresh.visits - programme.earn_rate;
            LoyaltyAccount::query()
                .where_(LoyaltyAccount::ID.eq(account.id))
                .update(
                    &mut *tx,
                    &[("visits", &rest), ("reward_available_until", &Some(until))],
                )
                .await?;
            reward = Some(until);
        }
    }
    Ok(Credited {
        movement: lot.id,
        reward,
    })
}

/// Records that the account earned on more sales today than the limit.
async fn flag_for_review(tx: &mut Transaction, account: i64, on: NaiveDate, limit: i64) -> Result {
    let today = LoyaltyEarning::query()
        .where_(LoyaltyEarning::ACCOUNT_ID.eq(account))
        .where_(LoyaltyEarning::EARNED_ON.eq(on))
        .count(&mut *tx)
        .await?;
    let count = i64::try_from(today).unwrap_or(i64::MAX);
    if count <= limit {
        return Ok(());
    }
    let known = LoyaltyReview::query()
        .where_(LoyaltyReview::ACCOUNT_ID.eq(account))
        .where_(LoyaltyReview::ON_DATE.eq(on))
        .exists(&mut *tx)
        .await?;
    if known {
        LoyaltyReview::query()
            .where_(LoyaltyReview::ACCOUNT_ID.eq(account))
            .where_(LoyaltyReview::ON_DATE.eq(on))
            .update(&mut *tx, &[("sales_count", &count)])
            .await?;
    } else {
        LoyaltyReview {
            account_id: account,
            on_date: on,
            sales_count: count,
            ..LoyaltyReview::default()
        }
        .insert(&mut *tx)
        .await?;
    }
    Ok(())
}

/// Earns for the sale (`SAL.RUL.07`): once per source, for an open account
/// that is not paused, when the programme earns on the day and the eligible
/// amount reaches the minimum.
pub async fn earn(state: &AppState, facts: SaleFacts) -> Result<Earned> {
    let Some(account) = LoyaltyAccount::find(&state.db, facts.account).await? else {
        return Ok(Earned::Skipped("no_account"));
    };
    if !account.earns() {
        return Ok(Earned::Skipped("account"));
    }
    let Some(programme) = LoyaltyProgramme::find(&state.db, account.programme_id).await? else {
        return Ok(Earned::Skipped("no_programme"));
    };
    // A change of the terms applies to the sales from its effective date.
    let programme = programme.terms_on(&state.db, facts.on).await?;
    if !programme.earns_on(facts.on) {
        return Ok(Earned::Skipped("programme"));
    }
    if LoyaltyEarning::of_source(&state.db, &facts.source)
        .await?
        .is_some()
    {
        return Ok(Earned::Already);
    }
    if facts.eligible_net <= 0 || facts.eligible_net < programme.minimum_amount {
        return Ok(Earned::Skipped("minimum"));
    }
    let points = programme.points_for(facts.eligible_net);
    if points < 1 {
        return Ok(Earned::Skipped("no_points"));
    }
    let limit = config_of(state).loyalty_daily_sales_limit;
    let mut tx = state.db.begin().await?;
    let mut earning = LoyaltyEarning {
        account_id: account.id,
        source_type: facts.source.subject_type.clone(),
        source_id: facts.source.subject_id,
        eligible_net: facts.eligible_net,
        basis_gross: facts.basis_gross,
        points,
        earn_kind: programme.earn_kind,
        earned_on: facts.on,
        ..LoyaltyEarning::default()
    };
    earning.insert(&mut tx).await?;
    let credited = credit(
        &mut tx,
        &programme,
        &account,
        points,
        &facts.source,
        earning.id,
        CAUSE_EARN,
        facts.on,
        facts.entity,
    )
    .await?;
    LoyaltyEarning::query()
        .where_(LoyaltyEarning::ID.eq(earning.id))
        .update(
            &mut tx,
            &[
                ("movement_id", &Some(credited.movement)),
                ("reward_made", &credited.reward.is_some()),
            ],
        )
        .await?;
    flag_for_review(&mut tx, account.id, facts.on, limit).await?;
    tx.commit().await?;

    announce_earned(state, &programme, &account, points, &facts.source).await?;
    if let Some(until) = credited.reward {
        notices::loyalty_reward(state, &account, &programme.visit_reward, until).await?;
    }
    Ok(Earned::Earned {
        points,
        reward: credited.reward.is_some(),
    })
}

/// Emits [`PointsEarned`].
async fn announce_earned(
    state: &AppState,
    programme: &LoyaltyProgramme,
    account: &LoyaltyAccount,
    points: i64,
    source: &Subject,
) -> Result {
    let value = if programme.earn_kind == EarnKind::PerAed {
        points * programme.point_value
    } else {
        0
    };
    state
        .emit(PointsEarned {
            account: account.account_id(),
            party: account.party(),
            points,
            value,
            source: source.clone(),
        })
        .await
}

/// Takes back `numerator ÷ denominator` of what the earning earned
/// (`reverse_earn`), rounded up and never more than is left, once per cause
/// and source. The member is told.
///
/// A per-visit programme takes the visit off; a reward that visit made and
/// that is still unused is withdrawn, and the visits are one short of the
/// reward again.
pub async fn reverse_earn(
    state: &AppState,
    earning: &LoyaltyEarning,
    numerator: i64,
    denominator: i64,
    cause: &'static str,
    cause_source: &Subject,
) -> Result<Reversed> {
    let Some(account) = LoyaltyAccount::find(&state.db, earning.account_id).await? else {
        return Ok(Reversed::Nothing);
    };
    if !account.is_active() {
        return Ok(Reversed::Nothing);
    }
    let again = PointsMovement::query()
        .where_(PointsMovement::ACCOUNT_ID.eq(account.id))
        .where_(PointsMovement::KIND.eq(PointsKind::ReverseEarn))
        .where_(PointsMovement::SOURCE_TYPE.eq(cause_source.subject_type.clone()))
        .where_(PointsMovement::SOURCE_ID.eq(cause_source.subject_id))
        .where_(PointsMovement::CAUSE.eq(cause))
        .exists(&state.db)
        .await?;
    if again {
        return Ok(Reversed::Already);
    }
    // The earning as it is now: an earlier reversal changed what is left.
    let Some(earning) = LoyaltyEarning::find(&state.db, earning.id).await? else {
        return Ok(Reversed::Nothing);
    };
    let Some(programme) = LoyaltyProgramme::find(&state.db, account.programme_id).await? else {
        return Ok(Reversed::Nothing);
    };
    let share = numerator.min(denominator);
    let points = ceil_ratio(earning.points, share, denominator).min(earning.outstanding());
    if points < 1 {
        return Ok(Reversed::Nothing);
    }
    let per_aed = earning.earn_kind == EarnKind::PerAed;
    let mut tx = state.db.begin().await?;
    let mut row = NewMovement {
        account: &account,
        kind: PointsKind::ReverseEarn,
        points: -points,
        value: if per_aed {
            points * programme.point_value
        } else {
            0
        },
        entity: None,
        source: cause_source,
        earning: earning.id,
        cause,
    }
    .into_row();
    row.lot_id = earning.movement_id;
    row.insert(&mut tx).await?;
    if let Some(lot) = earning.movement_id {
        PointsMovement::query()
            .where_(PointsMovement::ID.eq(lot))
            .increment(&mut tx, "reversed", points)
            .await?;
    }
    LoyaltyEarning::query()
        .where_(LoyaltyEarning::ID.eq(earning.id))
        .increment(&mut tx, "reversed_points", points)
        .await?;
    if per_aed {
        LoyaltyAccount::query()
            .where_(LoyaltyAccount::ID.eq(account.id))
            .increment(&mut tx, "points_balance", -points)
            .await?;
    } else if let Some(fresh) = LoyaltyAccount::find(&mut tx, account.id).await? {
        if earning.reward_made && fresh.reward_available_until.is_some() {
            // The reward this sale made is withdrawn: the visits are one
            // short of it again, plus those counted since.
            let visits: i64 = fresh.visits + programme.earn_rate - 1;
            let none: Option<NaiveDate> = None;
            LoyaltyAccount::query()
                .where_(LoyaltyAccount::ID.eq(account.id))
                .update(
                    &mut tx,
                    &[("visits", &visits), ("reward_available_until", &none)],
                )
                .await?;
        } else {
            let visits: i64 = (fresh.visits - 1).max(0);
            LoyaltyAccount::query()
                .where_(LoyaltyAccount::ID.eq(account.id))
                .update(&mut tx, &[("visits", &visits)])
                .await?;
        }
    }
    tx.commit().await?;

    state
        .emit(PointsReversed {
            account: account.account_id(),
            party: account.party(),
            points,
            value: row.value,
            source: cause_source.clone(),
        })
        .await?;
    let why = english(&format!("wl_sales.loyalty.why.{cause}"), &[]);
    notices::loyalty_reversed(state, &account, points, &why).await?;
    Ok(Reversed::Reversed(points))
}

/// Earns again what a money cause took back, when the sale is paid or
/// fulfilled again (`SAL.RUL.09`): a new lot for the points of the `causes`
/// that were not earned again yet. Returns the points (or visits) earned.
pub async fn earn_again(
    state: &AppState,
    earning: &LoyaltyEarning,
    on: NaiveDate,
    causes: &[&str],
) -> Result<i64> {
    let Some(account) = LoyaltyAccount::find(&state.db, earning.account_id).await? else {
        return Ok(0);
    };
    if !account.earns() {
        return Ok(0);
    }
    let Some(programme) = LoyaltyProgramme::find(&state.db, account.programme_id).await? else {
        return Ok(0);
    };
    if !programme.earns_on(on) {
        return Ok(0);
    }
    let rows: Vec<PointsMovement> = PointsMovement::query()
        .where_(PointsMovement::EARNING_ID.eq(earning.id))
        .where_(PointsMovement::KIND.eq(PointsKind::ReverseEarn))
        .where_(PointsMovement::REEARNED.eq(false))
        .order_by(PointsMovement::ID)
        .get(&state.db)
        .await?
        .into_iter()
        .filter(|m| causes.contains(&m.cause.as_str()))
        .collect();
    let total: i64 = rows.iter().map(|m| -m.points).sum();
    if total < 1 {
        return Ok(0);
    }
    let source = earning.source();
    let mut tx = state.db.begin().await?;
    let credited = credit(
        &mut tx,
        &programme,
        &account,
        total,
        &source,
        earning.id,
        CAUSE_REEARN,
        on,
        None,
    )
    .await?;
    for row in &rows {
        PointsMovement::query()
            .where_(PointsMovement::ID.eq(row.id))
            .update(&mut tx, &[("reearned", &true)])
            .await?;
    }
    LoyaltyEarning::query()
        .where_(LoyaltyEarning::ID.eq(earning.id))
        .increment(&mut tx, "reversed_points", -total)
        .await?;
    LoyaltyEarning::query()
        .where_(LoyaltyEarning::ID.eq(earning.id))
        .update(&mut tx, &[("movement_id", &Some(credited.movement))])
        .await?;
    tx.commit().await?;

    announce_earned(state, &programme, &account, total, &source).await?;
    if let Some(until) = credited.reward {
        notices::loyalty_reward(state, &account, &programme.visit_reward, until).await?;
    }
    Ok(total)
}

/// The causes a payment, a charge-back won, or a recovered invoice earns
/// again.
const PAID_AGAIN: &[&str] = &[CAUSE_PAYMENT_REVERSED, CAUSE_WRITTEN_OFF];

/// The causes a new hand-over earns again.
const FULFILLED_AGAIN: &[&str] = &[CAUSE_FULFILMENT_REVERSED];

/// The programme of the account with the terms that apply to a sale of `on`.
async fn programme_of(
    db: &Db,
    account: &LoyaltyAccount,
    on: NaiveDate,
) -> Result<Option<LoyaltyProgramme>> {
    match LoyaltyProgramme::find(db, account.programme_id).await? {
        Some(programme) => Ok(Some(programme.terms_on(db, on).await?)),
        None => Ok(None),
    }
}

/// What the lines of a sale count for `programme`, before VAT.
fn eligible_of(lines: &[SaleLine], programme: &LoyaltyProgramme) -> i64 {
    lines
        .iter()
        .filter(|l| !programme.excludes(&l.item_code))
        .map(SaleLine::eligible_net)
        .sum()
}

/// `SAL.RUL.07` for a completed counter sale.
async fn on_counter_sale(state: &AppState, event: &CounterSaleCompleted) -> Result {
    let Some(sale) = CounterSale::find(&state.db, event.sale.0).await? else {
        return Ok(());
    };
    let Some(account_id) = sale.loyalty_account_id else {
        return Ok(());
    };
    let Some(account) = LoyaltyAccount::find(&state.db, account_id).await? else {
        return Ok(());
    };
    let on = sale.completed_at.map_or_else(
        wl_kernel::documents::today,
        wl_kernel::documents::dubai_date,
    );
    let Some(programme) = programme_of(&state.db, &account, on).await? else {
        return Ok(());
    };
    let lines = SaleLine::of(&state.db, &sale.subject()).await?;
    let eligible = eligible_of(&lines, &programme);
    let facts = SaleFacts::new(
        sale.subject(),
        account.id,
        eligible,
        basis_of(sale.gross_total, eligible, sale.net_total),
        on,
    )
    .entity(sale.legal_entity_id);
    earn(state, facts).await?;
    Ok(())
}

/// `SAL.RUL.07` for an order reaching `fulfilled`, and the earn again of a
/// fulfilment that came back.
async fn on_order_fulfilled(state: &AppState, event: &SalesOrderFulfilled) -> Result {
    if !event.complete {
        return Ok(());
    }
    let Some(order) = SalesOrder::find(&state.db, event.order.0).await? else {
        return Ok(());
    };
    let on = wl_kernel::documents::today();
    if let Some(earning) = LoyaltyEarning::of_source(&state.db, &order.subject()).await? {
        earn_again(state, &earning, on, FULFILLED_AGAIN).await?;
        return Ok(());
    }
    let Some(account_id) = order.loyalty_account_id else {
        return Ok(());
    };
    let Some(account) = LoyaltyAccount::find(&state.db, account_id).await? else {
        return Ok(());
    };
    let Some(programme) = programme_of(&state.db, &account, on).await? else {
        return Ok(());
    };
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let eligible = eligible_of(&lines, &programme);
    let facts = SaleFacts::new(
        order.subject(),
        account.id,
        eligible,
        basis_of(order.gross_total, eligible, order.net_total),
        on,
    )
    .entity(order.legal_entity_id);
    earn(state, facts).await?;
    Ok(())
}

/// `SAL.RUL.07` for a paid invoice that is not an order's or a counter
/// sale's, and the earn again of a sale whose invoice is paid again.
async fn on_invoice_paid(state: &AppState, event: &InvoicePaid) -> Result {
    let sale = sale_of_invoice(&event.source, event.invoice.0);
    let on = wl_kernel::documents::today();
    if let Some(earning) = LoyaltyEarning::of_source(&state.db, &sale).await? {
        earn_again(state, &earning, on, PAID_AGAIN).await?;
        return Ok(());
    }
    if sale.subject_type != INVOICE_SUBJECT_TYPE {
        // An order or a counter sale earns by itself, not by its invoice.
        return Ok(());
    }
    let Some(account) = LoyaltyAccount::of_party(&state.db, event.party).await? else {
        return Ok(());
    };
    let books = invoicer(state);
    let (Some(net), Some(summary)) = (
        books.net_total(&state.db, event.invoice).await?,
        books.find(&state.db, event.invoice).await?,
    ) else {
        return Ok(());
    };
    let claimed: i64 = payer_bills(state)
        .of_source(&state.db, &event.source)
        .await?
        .iter()
        .map(|bill| bill.claimed)
        .sum();
    let eligible = (net - claimed).max(0);
    let facts = SaleFacts::new(
        sale,
        account.id,
        eligible,
        basis_of(summary.gross_total, eligible, net),
        on,
    )
    .entity(summary.legal_entity_id);
    earn(state, facts).await?;
    Ok(())
}

/// A credit note takes back points in proportion (`SAL.RUL.07`), unless a
/// sales return resolved with it already did.
async fn on_credit_note(state: &AppState, event: &CreditNoteIssued) -> Result {
    let sale = sale_of_invoice_id(&state.db, event.invoice.0).await?;
    let Some(earning) = LoyaltyEarning::of_source(&state.db, &sale).await? else {
        return Ok(());
    };
    if SalesReturn::of_credit_note(&state.db, event.credit_note.0)
        .await?
        .is_some()
    {
        return Ok(());
    }
    reverse_earn(
        state,
        &earning,
        event.net_amount,
        earning.eligible_net,
        CAUSE_CREDIT_NOTE,
        &event.credit_note.subject(),
    )
    .await?;
    Ok(())
}

/// A sales return takes back points in proportion to the returned eligible
/// value (`SAL.RUL.07`), unless its credit note already did.
async fn on_return_resolved(state: &AppState, event: &ReturnResolved) -> Result {
    let Some(r) = SalesReturn::find(&state.db, event.sales_return.0).await? else {
        return Ok(());
    };
    let Some(earning) = LoyaltyEarning::of_source(&state.db, &r.sale()).await? else {
        return Ok(());
    };
    if let Some(credit_note) = r.credit_note_id {
        let done = PointsMovement::query()
            .where_(PointsMovement::KIND.eq(PointsKind::ReverseEarn))
            .where_(PointsMovement::EARNING_ID.eq(earning.id))
            .where_(PointsMovement::CAUSE.eq(CAUSE_CREDIT_NOTE))
            .where_(PointsMovement::SOURCE_ID.eq(credit_note))
            .exists(&state.db)
            .await?;
        if done {
            return Ok(());
        }
    }
    reverse_earn(
        state,
        &earning,
        r.eligible_net,
        earning.eligible_net,
        CAUSE_RETURN,
        &r.subject(),
    )
    .await?;
    Ok(())
}

/// `SAL.RUL.09` for an invoice reopened by a reversed payment.
async fn on_invoice_reopened(state: &AppState, event: &InvoiceReopened) -> Result {
    if event.reason != ReopenReason::PaymentReversed {
        return Ok(());
    }
    let sale = sale_of_invoice(&event.source, event.invoice.0);
    let Some(earning) = LoyaltyEarning::of_source(&state.db, &sale).await? else {
        return Ok(());
    };
    reverse_earn(
        state,
        &earning,
        event.amount,
        earning.basis_gross,
        CAUSE_PAYMENT_REVERSED,
        &event.money_source,
    )
    .await?;
    Ok(())
}

/// `SAL.RUL.09` for an invoice written off.
async fn on_invoice_written_off(state: &AppState, event: &InvoiceWrittenOff) -> Result {
    let sale = sale_of_invoice(&event.source, event.invoice.0);
    let Some(earning) = LoyaltyEarning::of_source(&state.db, &sale).await? else {
        return Ok(());
    };
    reverse_earn(
        state,
        &earning,
        event.amount,
        earning.basis_gross,
        CAUSE_WRITTEN_OFF,
        &event.invoice.subject(),
    )
    .await?;
    Ok(())
}

/// `SAL.RUL.09` for a reversed or charged-back payment of a counter sale.
async fn on_payment_reversed(state: &AppState, event: &PaymentReversed) -> Result {
    let Some(sale) = CounterSale::of_payment(&state.db, event.payment.0).await? else {
        return Ok(());
    };
    let Some(earning) = LoyaltyEarning::of_source(&state.db, &sale.subject()).await? else {
        return Ok(());
    };
    let paid: i64 = sale
        .payments(&state.db)
        .await?
        .iter()
        .filter(|p| p.payment_id == Some(event.payment.0))
        .map(|p| p.amount)
        .sum();
    reverse_earn(
        state,
        &earning,
        paid,
        earning.basis_gross,
        CAUSE_PAYMENT_REVERSED,
        &event.payment.subject(),
    )
    .await?;
    Ok(())
}

/// A payment received again (a charge-back won) earns again what its
/// reversal took back.
async fn on_payment_received(state: &AppState, event: &PaymentReceived) -> Result {
    let Some(sale) = CounterSale::of_payment(&state.db, event.payment.0).await? else {
        return Ok(());
    };
    let Some(earning) = LoyaltyEarning::of_source(&state.db, &sale.subject()).await? else {
        return Ok(());
    };
    earn_again(state, &earning, wl_kernel::documents::today(), PAID_AGAIN).await?;
    Ok(())
}

/// `SAL.RUL.09` for a fulfilment that was reversed: the points in proportion
/// to the eligible value of the reversed quantities.
async fn on_fulfilment_reversed(state: &AppState, event: &SalesOrderFulfilmentReversed) -> Result {
    let Some(order) = SalesOrder::find(&state.db, event.order.0).await? else {
        return Ok(());
    };
    let Some(earning) = LoyaltyEarning::of_source(&state.db, &order.subject()).await? else {
        return Ok(());
    };
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let reversed: i64 = event
        .lines
        .iter()
        .filter_map(|taken| {
            lines
                .iter()
                .find(|l| l.id == taken.line)
                .map(|l| l.eligible_net_of(taken.quantity_milli))
        })
        .sum();
    reverse_earn(
        state,
        &earning,
        reversed,
        earning.eligible_net,
        CAUSE_FULFILMENT_REVERSED,
        &event.source,
    )
    .await?;
    Ok(())
}

/// Registers the listeners of `SAL.RUL.07` and `SAL.RUL.09`.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: CounterSaleCompleted, state: AppState| async move {
        on_counter_sale(&state, &event).await
    });
    app.listen(|event: SalesOrderFulfilled, state: AppState| async move {
        on_order_fulfilled(&state, &event).await
    });
    app.listen(|event: InvoicePaid, state: AppState| async move {
        on_invoice_paid(&state, &event).await
    });
    app.listen(|event: CreditNoteIssued, state: AppState| async move {
        on_credit_note(&state, &event).await
    });
    app.listen(|event: ReturnResolved, state: AppState| async move {
        on_return_resolved(&state, &event).await
    });
    app.listen(|event: InvoiceReopened, state: AppState| async move {
        on_invoice_reopened(&state, &event).await
    });
    app.listen(|event: InvoiceWrittenOff, state: AppState| async move {
        on_invoice_written_off(&state, &event).await
    });
    app.listen(|event: PaymentReversed, state: AppState| async move {
        on_payment_reversed(&state, &event).await
    });
    app.listen(|event: PaymentReceived, state: AppState| async move {
        on_payment_received(&state, &event).await
    });
    app.listen(
        |event: SalesOrderFulfilmentReversed, state: AppState| async move {
            on_fulfilment_reversed(&state, &event).await
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reversal_is_rounded_up_in_proportion() {
        // Half of AED 100 eligible reverses 50 of 100 points.
        assert_eq!(ceil_ratio(100, 5_000, 10_000), 50);
        // A third of 100 points rounds up to 34.
        assert_eq!(ceil_ratio(100, 1, 3), 34);
        assert_eq!(ceil_ratio(100, 0, 3), 0);
        assert_eq!(ceil_ratio(100, 3, 0), 0);
    }

    #[test]
    fn the_basis_follows_the_eligible_share_of_the_net() {
        // AED 1,000 net, AED 1,050 gross, AED 200 counts: AED 210 is paid.
        assert_eq!(basis_of(105_000, 20_000, 100_000), 21_000);
        assert_eq!(basis_of(10_500, 10_000, 10_000), 10_500);
        assert_eq!(basis_of(500, 0, 0), 500);
    }

    #[test]
    fn a_lot_expires_the_validity_later() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 15).unwrap();
        assert_eq!(
            expiry(day, 12),
            NaiveDate::from_ymd_opt(2027, 11, 15).unwrap()
        );
    }

    #[test]
    fn an_invoice_of_an_order_belongs_to_the_order() {
        let order = Subject::new("sales.sales_order", 4);
        assert_eq!(sale_of_invoice(&order, 9), order);
        let claim = Subject::new("clinics.visit", 4);
        assert_eq!(
            sale_of_invoice(&claim, 9),
            Subject::new("invoicing.invoice", 9)
        );
    }
}
