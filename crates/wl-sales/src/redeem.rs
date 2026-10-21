//! Redeeming loyalty points on a sale (wl-sales §5.9 step 3, `DiscountEngine`;
//! BL-SAL-23).
//!
//! **Asking.** [`request`] checks the sale (open, with a member attached before
//! the redemption), the programme's terms and the member's available points,
//! then sends the one-time code to the member's mobile (`Notifier::send_otp`)
//! unless the setting *confirm redemption by code* is off or the member is
//! logged in to the portal. **Confirming.** [`confirm`] checks the code
//! (`Notifier::verify_otp`); the right one holds the points and adds the
//! *Loyalty redemption* line to the sale; the third wrong one refuses the
//! redemption for that sale for good.
//!
//! **Spending.** The points are spent by the machine's `apply`: for a counter
//! sale when it completes ([`CounterSaleCompleted`]), for an order when it is
//! confirmed or, for an online order, when its payment is confirmed. Before a
//! counter sale completes or an order is submitted, [`prepare`] checks the
//! hold again; a lapsed hold or a shortfall removes the line, so the same
//! points cannot be spent twice.
//!
//! **Giving back.** A cancelled or rejected order gives back all of it
//! (`reverse_redeem`); a cancelled line and a return give back the share of
//! the sale's value; each lot slice comes back with its original expiry day,
//! or the setting's days when that day has passed. The member is told.
//!
//! Decisions:
//! - The redeemed value is VAT-inclusive: the line's price is the negative net
//!   whose gross is the points' worth (or one fil less where no net adds up
//!   to exactly that), so the VAT of the sale goes down with it (AED 8.00 off
//!   a VAT-inclusive AED 40.00 takes the VAT from 1.90 to 1.52). The
//!   treatment is `⚠ open: accountant (A-15)`; [`redemption_line`] is the one
//!   place an answer would change it.
//! - The maximum share is a share of the sale's value with VAT after the other
//!   discounts: the lines that are not redemption lines.
//! - A held counter sale keeps its hold for the setting's minutes; the hold of
//!   an order lasts until the order resolves once it is submitted
//!   ([`order_placed`]), because an online order waits for its payment.
//! - A visit reward (a per-visit programme) is not redeemed as points; the
//!   reward is a free text of the programme and needs a catalogue to price.
//! - The portal and public screens are not specified yet; the service takes
//!   `member_logged_in` for them and the back office never sets it.

use renox::chrono::{NaiveDate, TimeDelta};
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{DraftLine, VatCategory};
use wl_kernel::notify::{
    self, MessageGaveUp, NotifyChannel, NotifyRecipient, OTP_TYPE_KEY, OtpRequest, OtpSent,
    OtpVerdict,
};
use wl_kernel::sales::{
    COUNTER_SALE_SUBJECT_TYPE, CounterSaleCompleted, DiscountEngine, PointsGivenBack,
    QuoteRedemption, RedemptionQuote, ReturnResolved, SALES_ORDER_SUBJECT_TYPE,
    SalesFuture, SalesOrderCancelled, SalesOrderLineCancelled, SalesOrderRejected,
};
use wl_kernel::Subject;

use crate::config::config_of;
use crate::counter_sale::CounterSale;
use crate::loyalty_account::LoyaltyAccount;
use crate::loyalty_points::{
    CAUSE_CANCEL_LINE, CAUSE_ORDER_CANCELLED, CAUSE_ORDER_REJECTED, CAUSE_RETURN, PointsKind,
    PointsMovement,
};
use crate::loyalty_programme::{EarnKind, LoyaltyProgramme};
use crate::notices;
use crate::redemption::{
    REFUSED_CODE_NOT_SENT, REFUSED_TOO_MANY_TRIES, Redemption, RedemptionStatus, VIA_CODE,
    VIA_PORTAL,
};
use crate::redemption_machine::{APPLY, ApplyInput, CONFIRM, ConfirmInput, REFUSE, RELEASE};
use crate::sale_line::{KIND_ITEM, KIND_LOYALTY_REDEMPTION, NewLine, SaleLine, ratio_half_up, totals};
use crate::sales_order::SalesOrder;
use crate::sales_order_machine::{
    ACCEPT, CONFIRM_PAYMENT, CONFIRM_PAYMENT_FOR_ACCEPTANCE, SUBMIT,
};
use crate::sales_return::SalesReturn;
use crate::texts::english;

/// What the one-time code confirms.
pub const OTP_PURPOSE: &str = "loyalty_redemption";

/// How a redemption was confirmed when no code is needed (the setting is off).
pub const VIA_COUNTER: &str = "counter";

/// The item code of the redemption line.
pub const LINE_CODE: &str = "LOYALTY";

/// A 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// A 422 with the English text `key` and `params`.
fn refused_with(key: &str, params: &[(&str, &dyn std::fmt::Display)]) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, params))
}

/// The largest number of points the maximum share of a sale allows: `base`
/// fils with VAT × `share_bps` ÷ 10,000, in whole points of `point_value`
/// fils.
pub fn max_points(base: i64, share_bps: i64, point_value: i64) -> i64 {
    if point_value <= 0 || base <= 0 {
        return 0;
    }
    let value = i128::from(base) * i128::from(share_bps.clamp(0, 10_000)) / 10_000;
    i64::try_from(value / i128::from(point_value)).unwrap_or(0)
}

/// The price and the VAT of the redemption line for `value` fils with VAT: the
/// most negative net whose gross is not below `-value` (so the member never
/// gets more than the points are worth). This is the one place the VAT
/// treatment of a redemption is decided (`⚠ open: accountant (A-15)`).
pub fn redemption_line(value: i64) -> (i64, i64) {
    let gross = |net: i64| {
        let line = DraftLine::new("", net, VatCategory::Standard);
        line.net() + line.vat()
    };
    let mut net = -ratio_half_up(value, 100, 105);
    while gross(net) < -value {
        net += 1;
    }
    while gross(net - 1) >= -value {
        net -= 1;
    }
    (net, DraftLine::new("", net, VatCategory::Standard).vat())
}

/// A sale the points can pay part of.
struct SaleView {
    subject: Subject,
    account: Option<i64>,
    open: bool,
    entity: Option<i64>,
    is_order: bool,
}

/// The counter sale or the order named by `sale`.
async fn sale_view(db: &Db, sale: &Subject) -> Result<SaleView> {
    match sale.subject_type.as_str() {
        COUNTER_SALE_SUBJECT_TYPE => {
            let s = CounterSale::find_or_404(db, sale.subject_id).await?;
            Ok(SaleView {
                subject: s.subject(),
                account: s.loyalty_account_id,
                open: s.is_open(),
                entity: s.legal_entity_id,
                is_order: false,
            })
        }
        SALES_ORDER_SUBJECT_TYPE => {
            let o = SalesOrder::find_or_404(db, sale.subject_id).await?;
            Ok(SaleView {
                subject: o.subject(),
                account: o.loyalty_account_id,
                open: o.is_draft(),
                entity: o.legal_entity_id,
                is_order: true,
            })
        }
        _ => Err(refused("wl_sales.redeem.problem.sale_type")),
    }
}

/// Makes the stored totals of `sale` those of its lines.
async fn refresh_totals(db: &Db, sale: &Subject) -> Result {
    let lines = SaleLine::of(db, sale).await?;
    let t = totals(&lines);
    let columns: [(&str, &(dyn renox::db::ToDbValue + Sync)); 3] = [
        ("net_total", &t.net),
        ("vat_total", &t.vat),
        ("gross_total", &t.gross),
    ];
    if sale.subject_type == COUNTER_SALE_SUBJECT_TYPE {
        CounterSale::query()
            .where_(CounterSale::ID.eq(sale.subject_id))
            .update(db, &columns)
            .await?;
    } else {
        SalesOrder::query()
            .where_(SalesOrder::ID.eq(sale.subject_id))
            .update(db, &columns)
            .await?;
    }
    Ok(())
}

/// The value with VAT of the sale's lines that are not redemption lines: the
/// base of the maximum share.
fn base_of(lines: &[SaleLine]) -> i64 {
    lines
        .iter()
        .filter(|l| l.kind == KIND_ITEM)
        .map(SaleLine::gross)
        .sum()
}

/// The points `account` can spend at `at`: the smaller of its balance and the
/// points of its lots still usable on `on`, less the points other
/// redemptions hold (`ignore` is the one being checked).
pub async fn available_points(
    db: &Db,
    account: &LoyaltyAccount,
    on: NaiveDate,
    at: DateTime,
    ignore: Option<i64>,
) -> Result<i64> {
    let usable: i64 = PointsMovement::lots(db, account.id, Some(on))
        .await?
        .iter()
        .map(PointsMovement::lot_remaining)
        .sum();
    let held: i64 = Redemption::held_of_account(db, account.id)
        .await?
        .iter()
        .filter(|r| Some(r.id) != ignore && r.holds_at(at))
        .map(|r| r.points)
        .sum();
    Ok((account.points_balance.min(usable) - held).max(0))
}

/// What a redemption comes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quote {
    /// The points.
    pub points: i64,
    /// Their worth in fils.
    pub value: i64,
    /// The negative price of the line before VAT.
    pub line_net: i64,
    /// The VAT of the line (negative).
    pub line_vat: i64,
    /// The most points the maximum share allows.
    pub maximum_points: i64,
    /// The sale's value with VAT after the other discounts.
    pub base: i64,
}

/// Prices the redemption of `points` of `account` on `sale` within the
/// programme's terms.
///
/// # Errors
///
/// 422 when the points are not above zero, the account is closed or paused,
/// the programme takes no redemptions, it is a per-visit programme, the points
/// are below the minimum or above the maximum share, or the member has fewer
/// available.
pub async fn quote(
    state: &AppState,
    sale: &Subject,
    account: &LoyaltyAccount,
    programme: &LoyaltyProgramme,
    points: i64,
    ignore: Option<i64>,
) -> Result<Quote> {
    if points <= 0 {
        return Err(refused("wl_sales.redeem.problem.points"));
    }
    if !account.is_active() || account.paused {
        return Err(refused("wl_sales.loyalty.problem.no_member"));
    }
    let on = today();
    let open = programme.active && programme.closing_date.is_none_or(|last| last >= on);
    if !open {
        return Err(refused("wl_sales.redeem.problem.programme_off"));
    }
    if programme.earn_kind != EarnKind::PerAed {
        return Err(refused("wl_sales.redeem.problem.visits"));
    }
    if points < programme.min_redeem_points {
        return Err(refused_with(
            "wl_sales.redeem.problem.below_minimum",
            &[("min", &programme.min_redeem_points)],
        ));
    }
    let lines = SaleLine::of(&state.db, sale).await?;
    let base = base_of(&lines);
    let maximum = max_points(base, programme.max_share_bps, programme.point_value);
    if points > maximum {
        return Err(refused_with(
            "wl_sales.redeem.problem.above_share",
            &[("max", &maximum)],
        ));
    }
    let fresh = LoyaltyAccount::find(&state.db, account.id)
        .await?
        .unwrap_or_else(|| account.clone());
    let available = available_points(&state.db, &fresh, on, now(), ignore).await?;
    if points > available {
        return Err(refused_with(
            "wl_sales.redeem.problem.not_enough",
            &[("points", &points)],
        ));
    }
    let value = points * programme.point_value;
    let (line_net, line_vat) = redemption_line(value);
    Ok(Quote {
        points,
        value,
        line_net,
        line_vat,
        maximum_points: maximum,
        base,
    })
}

/// A redemption to ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RedeemRequest {
    /// The counter sale or sales order.
    pub sale: Subject,
    /// The points to spend.
    pub points: i64,
    /// The user who asks (the cashier).
    pub requested_by: Option<i64>,
    /// The member is logged in to the portal: no code is needed.
    pub member_logged_in: bool,
}

impl RedeemRequest {
    /// `points` on `sale`.
    pub fn new(sale: Subject, points: i64) -> Self {
        RedeemRequest {
            sale,
            points,
            requested_by: None,
            member_logged_in: false,
        }
    }

    /// Asked for by the user `user`.
    pub fn by(mut self, user: i64) -> Self {
        self.requested_by = Some(user);
        self
    }

    /// The member is logged in to the portal.
    pub fn member_logged_in(mut self) -> Self {
        self.member_logged_in = true;
        self
    }
}

/// What [`request`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Requested {
    /// The code was sent; the redemption waits for it ([`confirm`]).
    CodeSent {
        /// The redemption.
        redemption: Redemption,
        /// When the code stops being valid.
        expires_at: DateTime,
    },
    /// No code was needed: the points are held and the line is on the sale.
    Held {
        /// The redemption.
        redemption: Redemption,
    },
}

/// Refuses the pending redemption `row` with `reason`.
async fn refuse_row(state: &AppState, row: &Redemption, reason: &str) -> Result {
    let _ = row
        .transition(REFUSE)
        .by_system()
        .reason(reason.to_owned())
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    Ok(())
}

/// Releases `row` (pending or held).
async fn release_row(state: &AppState, row: &Redemption, why: &str) -> Result {
    let _ = row
        .transition(RELEASE)
        .by_system()
        .reason(why.to_owned())
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    Ok(())
}

/// Asks to redeem `request.points` on the sale: checks the terms, creates the
/// redemption and sends the code (or holds the points at once when no code is
/// needed). A pending redemption whose code expired is asked for again.
///
/// # Errors
///
/// 422 from [`quote`], when the sale is not open, no member is attached, the
/// redemption was refused for this sale after three wrong codes, points are
/// already redeemed on it, or the code could not be sent (the redemption is
/// then refused and the sale goes on without it).
pub async fn request(state: &AppState, request: RedeemRequest) -> Result<Requested> {
    let view = sale_view(&state.db, &request.sale).await?;
    if !view.open {
        return Err(refused("wl_sales.redeem.problem.sale_closed"));
    }
    let Some(account_id) = view.account else {
        return Err(refused("wl_sales.redeem.problem.no_member"));
    };
    let account = LoyaltyAccount::find_or_404(&state.db, account_id).await?;
    let programme = LoyaltyProgramme::find_or_404(&state.db, account.programme_id).await?;
    let rows = Redemption::of_sale(&state.db, &view.subject).await?;
    if rows.iter().any(|r| {
        r.status == RedemptionStatus::Refused
            && r.refusal.as_deref() == Some(REFUSED_TOO_MANY_TRIES)
    }) {
        return Err(refused("wl_sales.redeem.problem.locked"));
    }
    if rows
        .iter()
        .any(|r| matches!(r.status, RedemptionStatus::Held | RedemptionStatus::Applied))
    {
        return Err(refused("wl_sales.redeem.problem.already"));
    }
    let priced = quote(state, &view.subject, &account, &programme, request.points, None).await?;
    let pending = rows
        .into_iter()
        .rev()
        .find(|r| r.status == RedemptionStatus::Pending);
    let row = match pending {
        Some(row) => {
            Redemption::query()
                .where_(Redemption::ID.eq(row.id))
                .update(
                    &state.db,
                    &[
                        ("points", &priced.points),
                        ("value", &priced.value),
                        ("requested_by", &request.requested_by),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
            Redemption::find_or_404(&state.db, row.id).await?
        }
        None => {
            Redemption::create(
                &state.db,
                Redemption {
                    account_id: account.id,
                    sale_type: view.subject.subject_type.clone(),
                    sale_id: view.subject.subject_id,
                    points: priced.points,
                    value: priced.value,
                    requested_by: request.requested_by,
                    ..Redemption::default()
                },
            )
            .await?
        }
    };
    let by_code = config_of(state).confirm_redemption_by_code && !request.member_logged_in;
    if !by_code {
        let via = if request.member_logged_in {
            VIA_PORTAL
        } else {
            VIA_COUNTER
        };
        let held = confirm_row(state, row, via).await?;
        return Ok(Requested::Held { redemption: held });
    }
    let to = NotifyRecipient::Address {
        channel: NotifyChannel::Sms,
        address: account.mobile.clone(),
    };
    let sent = notify::notifier(state)
        .send_otp(
            state,
            OtpRequest::new(OTP_PURPOSE, view.subject.clone(), to),
        )
        .await?;
    match sent {
        OtpSent::Sent { expires_at, .. } => Ok(Requested::CodeSent {
            redemption: row,
            expires_at,
        }),
        OtpSent::NotSent { text } => {
            refuse_row(state, &row, REFUSED_CODE_NOT_SENT).await?;
            Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, text))
        }
    }
}

/// Checks `code` for the pending redemption of `sale`. The right code holds
/// the points and puts the redemption line on the sale; a wrong one counts
/// down; the third voids the code and refuses the redemption for this sale.
///
/// # Errors
///
/// 422 when no redemption waits for a code, the code is wrong or expired, the
/// terms no longer allow the points, or the redemption was refused.
pub async fn confirm(state: &AppState, sale: &Subject, code: &str) -> Result<Redemption> {
    if code.trim().is_empty() {
        return Err(refused("wl_sales.redeem.problem.no_code"));
    }
    let Some(row) = Redemption::latest_of_sale(&state.db, sale, RedemptionStatus::Pending).await?
    else {
        return Err(refused("wl_sales.redeem.problem.none_pending"));
    };
    let verdict = notify::notifier(state)
        .verify_otp(state, OTP_PURPOSE, sale, code.trim())
        .await?;
    match verdict {
        OtpVerdict::Valid => confirm_row(state, row, VIA_CODE).await,
        OtpVerdict::Wrong { tries_left } => Err(refused_with(
            "wl_sales.redeem.problem.wrong",
            &[("left", &tries_left)],
        )),
        OtpVerdict::Expired => Err(refused("wl_sales.redeem.problem.expired")),
        OtpVerdict::Locked => {
            refuse_row(state, &row, REFUSED_TOO_MANY_TRIES).await?;
            Err(refused("wl_sales.redeem.problem.locked_now"))
        }
    }
}

/// Holds the points of the pending `row` and puts the redemption line on its
/// sale, after pricing the redemption again.
async fn confirm_row(state: &AppState, row: Redemption, via: &str) -> Result<Redemption> {
    let view = sale_view(&state.db, &row.sale()).await?;
    if !view.open {
        release_row(state, &row, "sale_closed").await?;
        return Err(refused("wl_sales.redeem.problem.sale_closed"));
    }
    let account = LoyaltyAccount::find_or_404(&state.db, row.account_id).await?;
    let programme = LoyaltyProgramme::find_or_404(&state.db, account.programme_id).await?;
    let priced = quote(
        state,
        &view.subject,
        &account,
        &programme,
        row.points,
        Some(row.id),
    )
    .await?;
    let line = SaleLine::add(
        &state.db,
        &view.subject,
        NewLine::new(LINE_CODE, priced.line_net)
            .description(&english("wl_sales.redeem.line", &[]))
            .kind(KIND_LOYALTY_REDEMPTION),
    )
    .await?;
    refresh_totals(&state.db, &view.subject).await?;
    let config = config_of(state);
    let input = ConfirmInput {
        line_id: line.id,
        via: via.to_owned(),
        hold_until: Some(now() + TimeDelta::minutes(config.redemption_hold_minutes)),
        basis: priced.base,
        available: available_points(&state.db, &account, today(), now(), Some(row.id)).await?,
    };
    if let Err(error) = row
        .transition(CONFIRM)
        .by_system()
        .input(&input)
        .run(state)
        .await
    {
        SaleLine::query()
            .where_(SaleLine::ID.eq(line.id))
            .delete(&state.db)
            .await?;
        refresh_totals(&state.db, &view.subject).await?;
        return Err(error.into_error());
    }
    Redemption::find_or_404(&state.db, row.id).await
}

/// Takes the redemption line of `row` off its sale and makes the sale's
/// totals those of the lines left.
async fn drop_line(db: &Db, row: &Redemption) -> Result {
    if let Some(line) = row.line_id {
        SaleLine::query()
            .where_(SaleLine::ID.eq(line))
            .delete(db)
            .await?;
        refresh_totals(db, &row.sale()).await?;
    }
    Ok(())
}

/// Checks the redemptions of `sale` before it completes (a counter sale) or is
/// submitted (an order): a redemption still waiting for its code is released,
/// and a held one whose hold lapsed, or whose points are no longer there, is
/// released and its line removed. Returns whether a line was removed (the
/// sale's total went up).
pub async fn prepare(state: &AppState, sale: &Subject) -> Result<bool> {
    let mut dropped = false;
    for row in Redemption::of_sale(&state.db, sale).await? {
        match row.status {
            RedemptionStatus::Pending => release_row(state, &row, "abandoned").await?,
            RedemptionStatus::Held => {
                let account = LoyaltyAccount::find_or_404(&state.db, row.account_id).await?;
                let held = row.holds_at(now());
                let there = available_points(&state.db, &account, today(), now(), Some(row.id))
                    .await?
                    >= row.points;
                if held && there && account.is_active() && !account.paused {
                    continue;
                }
                release_row(state, &row, if held { "shortfall" } else { "hold_lapsed" }).await?;
                drop_line(&state.db, &row).await?;
                dropped = true;
            }
            _ => {}
        }
    }
    Ok(dropped)
}

/// An order was submitted: its held redemption stays held until the order
/// resolves (an online order waits for its payment).
pub async fn order_placed(state: &AppState, order: &Subject) -> Result {
    let none: Option<DateTime> = None;
    Redemption::query()
        .where_(Redemption::SALE_TYPE.eq(order.subject_type.clone()))
        .where_(Redemption::SALE_ID.eq(order.subject_id))
        .where_(Redemption::STATUS.eq(RedemptionStatus::Held))
        .update(&state.db, &[("hold_until", &none)])
        .await?;
    Ok(())
}

/// Spends the held redemption of `sale`, if any: the `redeem` movements are
/// written. A sale without one changes nothing.
pub async fn apply_for_sale(state: &AppState, sale: &Subject) -> Result {
    let Some(row) = Redemption::latest_of_sale(&state.db, sale, RedemptionStatus::Held).await?
    else {
        return Ok(());
    };
    let view = sale_view(&state.db, sale).await?;
    let account = LoyaltyAccount::find_or_404(&state.db, row.account_id).await?;
    let programme = LoyaltyProgramme::find_or_404(&state.db, account.programme_id).await?;
    let lines = SaleLine::of(&state.db, sale).await?;
    let input = ApplyInput {
        on: today(),
        entity: view.entity,
        point_value: programme.point_value,
        basis: base_of(&lines),
    };
    let _ = row
        .transition(APPLY)
        .by_system()
        .input(&input)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    Ok(())
}

/// Gives back points of the applied `redemption` until `target` points in all
/// are back (never more than were redeemed): one `reverse_redeem` movement per
/// lot slice, each a lot again with its original expiry day, or the setting's
/// days from today when that day has passed. Returns the points given back
/// now; the member is told with the reason of `cause`.
pub async fn give_back(
    state: &AppState,
    redemption: i64,
    target: i64,
    cause: &str,
    source: &Subject,
) -> Result<i64> {
    let Some(row) = Redemption::find(&state.db, redemption).await? else {
        return Ok(0);
    };
    if row.status != RedemptionStatus::Applied {
        return Ok(0);
    }
    let take = target.min(row.points) - row.returned_points;
    if take <= 0 {
        return Ok(0);
    }
    let account = LoyaltyAccount::find_or_404(&state.db, row.account_id).await?;
    let programme = LoyaltyProgramme::find_or_404(&state.db, account.programme_id).await?;
    let slices: Vec<PointsMovement> = PointsMovement::query()
        .where_(PointsMovement::ACCOUNT_ID.eq(account.id))
        .where_(PointsMovement::KIND.eq(PointsKind::Redeem))
        .where_(PointsMovement::SOURCE_TYPE.eq(row.sale_type.clone()))
        .where_(PointsMovement::SOURCE_ID.eq(row.sale_id))
        .order_by(PointsMovement::ID)
        .get(&state.db)
        .await?;
    let config = config_of(state);
    let on = today();
    let fallback = on + TimeDelta::days(config.give_back_days);
    let (from, to) = (row.returned_points, row.returned_points + take);
    let mut tx = state.db.begin().await?;
    let mut start = 0;
    for slice in &slices {
        let size = -slice.points;
        let end = start + size;
        let overlap = to.min(end) - from.max(start);
        if overlap > 0 {
            let expires = slice
                .lot_expires_on
                .filter(|day| *day >= on)
                .unwrap_or(fallback);
            PointsMovement {
                account_id: account.id,
                kind: PointsKind::ReverseRedeem,
                points: overlap,
                value: overlap * programme.point_value,
                legal_entity_id: slice.legal_entity_id,
                source_type: source.subject_type.clone(),
                source_id: source.subject_id,
                earning_id: slice.earning_id,
                cause: cause.to_owned(),
                lot_id: slice.lot_id,
                lot_expires_on: Some(expires),
                ..PointsMovement::default()
            }
            .insert(&mut tx)
            .await?;
        }
        start = end;
    }
    LoyaltyAccount::query()
        .where_(LoyaltyAccount::ID.eq(account.id))
        .increment(&mut tx, "points_balance", take)
        .await?;
    Redemption::query()
        .where_(Redemption::ID.eq(row.id))
        .increment(&mut tx, "returned_points", take)
        .await?;
    tx.commit().await?;

    state
        .emit(PointsGivenBack {
            account: account.account_id(),
            party: account.party(),
            points: take,
            value: take * programme.point_value,
            source: source.clone(),
        })
        .await?;
    let why = english(&format!("wl_sales.loyalty.why.{cause}"), &[]);
    notices::loyalty_given_back(state, &account, take, &why).await?;
    Ok(take)
}

/// An order was cancelled, rejected or its payment failed: a hold is let go,
/// points already spent come back in full.
async fn order_ended(state: &AppState, order: &Subject, cause: &str) -> Result {
    let Some(row) = Redemption::of_sale(&state.db, order)
        .await?
        .into_iter()
        .rev()
        .find(|r| matches!(r.status, RedemptionStatus::Held | RedemptionStatus::Applied))
    else {
        return Ok(());
    };
    match row.status {
        RedemptionStatus::Held => release_row(state, &row, cause).await,
        _ => give_back(state, row.id, row.points, cause, order)
            .await
            .map(|_| ()),
    }
}

/// A line of a confirmed order was cancelled: the share of the redemption
/// that the cancelled value makes of the order's value comes back.
async fn line_cancelled(state: &AppState, event: &SalesOrderLineCancelled) -> Result {
    let order = event.order.subject();
    let Some(row) = Redemption::latest_of_sale(&state.db, &order, RedemptionStatus::Applied)
        .await?
    else {
        return Ok(());
    };
    let lines = SaleLine::of(&state.db, &order).await?;
    let cancelled: i64 = lines
        .iter()
        .filter(|l| l.kind == KIND_ITEM)
        .map(|l| ratio_half_up(l.gross(), l.cancelled_milli, l.quantity_milli))
        .sum();
    let target = share_of(row.points, cancelled + row.returned_value, row.basis);
    give_back(state, row.id, target, CAUSE_CANCEL_LINE, &order).await?;
    Ok(())
}

/// A sales return of a sale that redeemed points: the share of the
/// redemption that the returned value makes of the sale's value comes back.
async fn return_resolved(state: &AppState, event: &ReturnResolved) -> Result {
    let Some(row) =
        Redemption::latest_of_sale(&state.db, &event.sale, RedemptionStatus::Applied).await?
    else {
        return Ok(());
    };
    let Some(r) = SalesReturn::find(&state.db, event.sales_return.0).await? else {
        return Ok(());
    };
    let source = r.subject();
    let again = PointsMovement::query()
        .where_(PointsMovement::KIND.eq(PointsKind::ReverseRedeem))
        .where_(PointsMovement::SOURCE_TYPE.eq(source.subject_type.clone()))
        .where_(PointsMovement::SOURCE_ID.eq(source.subject_id))
        .exists(&state.db)
        .await?;
    if again {
        return Ok(());
    }
    let returned = row.returned_value + event.value;
    Redemption::query()
        .where_(Redemption::ID.eq(row.id))
        .update(&state.db, &[("returned_value", &returned)])
        .await?;
    // Cancelled lines of an order count too.
    let lines = SaleLine::of(&state.db, &event.sale).await?;
    let cancelled: i64 = lines
        .iter()
        .filter(|l| l.kind == KIND_ITEM)
        .map(|l| ratio_half_up(l.gross(), l.cancelled_milli, l.quantity_milli))
        .sum();
    let target = share_of(row.points, cancelled + returned, row.basis);
    give_back(state, row.id, target, CAUSE_RETURN, &source).await?;
    Ok(())
}

/// `points × part ÷ whole`, rounded down to whole points; all of them when
/// `part` reaches `whole`.
pub fn share_of(points: i64, part: i64, whole: i64) -> i64 {
    if whole <= 0 || part <= 0 {
        return 0;
    }
    if part >= whole {
        return points;
    }
    i64::try_from(i128::from(points) * i128::from(part) / i128::from(whole)).unwrap_or(0)
}

/// Releases the holds that lapsed (the hold minutes passed on a sale nobody
/// completed) and takes their lines off the open sales. Returns how many.
pub async fn release_lapsed(state: &AppState, at: DateTime) -> Result<usize> {
    let mut released = 0;
    let held = Redemption::query()
        .where_(Redemption::STATUS.eq(RedemptionStatus::Held))
        .order_by(Redemption::ID)
        .get(&state.db)
        .await?;
    for row in held {
        if row.hold_until.is_some_and(|until| until <= at) {
            release_row(state, &row, "hold_lapsed").await?;
            drop_line(&state.db, &row).await?;
            released += 1;
        }
    }
    Ok(released)
}

/// The scheduled task and command that release lapsed holds.
pub const HOLDS_TASK: &str = "sales:redemption_holds";

/// The code could not be delivered after all: the pending redemption of its
/// sale is refused and the sale goes on without it.
async fn code_gave_up(state: &AppState, event: &MessageGaveUp) -> Result {
    if event.type_key != OTP_TYPE_KEY {
        return Ok(());
    }
    let Some(row) =
        Redemption::latest_of_sale(&state.db, &event.subject, RedemptionStatus::Pending).await?
    else {
        return Ok(());
    };
    refuse_row(state, &row, REFUSED_CODE_NOT_SENT).await
}

/// The discount engine of the module: prices the loyalty redemption line.
pub(crate) struct Engine;

impl DiscountEngine for Engine {
    fn quote_redemption<'a>(
        &'a self,
        state: &'a AppState,
        request: QuoteRedemption,
    ) -> SalesFuture<'a, RedemptionQuote> {
        Box::pin(async move {
            let account = LoyaltyAccount::find_or_404(&state.db, request.account.0).await?;
            let programme = LoyaltyProgramme::find_or_404(&state.db, account.programme_id).await?;
            let priced = quote(
                state,
                &request.sale,
                &account,
                &programme,
                request.points,
                None,
            )
            .await?;
            Ok(RedemptionQuote {
                points: priced.points,
                value: priced.value,
                line_net: priced.line_net,
                line_vat: priced.line_vat,
                maximum_points: priced.maximum_points,
            })
        })
    }
}

/// Registers the listeners, the engine and the task that releases lapsed
/// holds.
pub(crate) fn register(app: &mut Registry) {
    crate::redemption_machine::register_events(app);
    wl_kernel::sales::provide_discount_engine(app, Engine);
    app.listen(|event: CounterSaleCompleted, state: AppState| async move {
        apply_for_sale(&state, &event.sale.subject()).await
    });
    app.listen(
        |event: Transitioned<SalesOrder>, state: AppState| async move {
            let name = event.transition.as_str();
            if ![
                SUBMIT,
                ACCEPT,
                CONFIRM_PAYMENT,
                CONFIRM_PAYMENT_FOR_ACCEPTANCE,
            ]
            .contains(&name)
            {
                return Ok(());
            }
            let order = Subject::new(SALES_ORDER_SUBJECT_TYPE, event.key);
            apply_for_sale(&state, &order).await
        },
    );
    app.listen(|event: SalesOrderCancelled, state: AppState| async move {
        order_ended(&state, &event.order.subject(), CAUSE_ORDER_CANCELLED).await
    });
    app.listen(|event: SalesOrderRejected, state: AppState| async move {
        order_ended(&state, &event.order.subject(), CAUSE_ORDER_REJECTED).await
    });
    app.listen(|event: SalesOrderLineCancelled, state: AppState| async move {
        line_cancelled(&state, &event).await
    });
    app.listen(|event: ReturnResolved, state: AppState| async move {
        return_resolved(&state, &event).await
    });
    app.listen(|event: MessageGaveUp, state: AppState| async move {
        code_gave_up(&state, &event).await
    });
    app.schedule()
        .every_minute(HOLDS_TASK, |state: AppState| async move {
            release_lapsed(&state, now()).await.map(|_| ())
        });
    app.command(
        HOLDS_TASK,
        "Release the points held for sales that were not completed in time",
        |_args, state: AppState| async move {
            let released = release_lapsed(&state, now()).await?;
            println!("{released} holds released.");
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_dirhams_of_points_are_a_line_of_minus_762_and_minus_38_vat() {
        assert_eq!(redemption_line(800), (-762, -38));
        assert_eq!(redemption_line(500), (-476, -24));
    }

    #[test]
    fn a_line_never_gives_more_than_the_points_are_worth() {
        for value in [1, 5, 99, 100, 101, 800, 1_234, 99_999] {
            let (net, vat) = redemption_line(value);
            let gross = net + vat;
            assert!(gross >= -value, "{value}: {gross}");
            assert!(gross <= -value + 2, "{value}: {gross}");
        }
    }

    #[test]
    fn the_maximum_share_is_of_the_value_with_vat_in_whole_points() {
        // AED 10.00 at 50% and AED 0.01 a point: 500 points.
        assert_eq!(max_points(1_000, 5_000, 1), 500);
        assert_eq!(max_points(4_000, 10_000, 1), 4_000);
        assert_eq!(max_points(1_005, 5_000, 2), 251);
        assert_eq!(max_points(0, 5_000, 1), 0);
        assert_eq!(max_points(1_000, 5_000, 0), 0);
    }

    #[test]
    fn a_share_is_rounded_down_and_all_of_it_when_the_part_reaches_the_whole() {
        assert_eq!(share_of(500, 5_000, 10_000), 250);
        assert_eq!(share_of(500, 3_333, 10_000), 166);
        assert_eq!(share_of(500, 10_000, 10_000), 500);
        assert_eq!(share_of(500, 12_000, 10_000), 500);
        assert_eq!(share_of(500, 0, 10_000), 0);
        assert_eq!(share_of(500, 1, 0), 0);
    }
}
