//! Topping up or part-refunding a held deposit when its agreement changes
//! (wl-payments §3 *Deposit*, §4 `adjust`, §5.4 step 1, §6 `DepositAdjusted`,
//! issue #115).
//!
//! A lease amendment or renewal (01) can change the amount the deposit must
//! be while it is held. `adjust` (the system, from the source agreement, or a
//! user with `deposits.manage`) records an adjustment ([`DepositAdjustment`])
//! and sets the deposit's required `amount`; the deposit stays `held`:
//!
//! - **An increase** raises a top-up charge for the difference, due on the
//!   effective date ([`DepositTopUps`](wl_kernel::deposits::DepositTopUps),
//!   provided by the module that owns the charges; without one the accountant
//!   gets a task to raise it). The held amount rises when a payment is
//!   allocated to the deposit (the existing allocation, which takes up to what
//!   the deposit lacks); the adjustment is settled when the deposit holds its
//!   new amount ([`settle_top_ups_in`]).
//! - **A decrease** asks for a part refund of the difference through the
//!   Refund flow, with the deposit as its source (approval `FIN.ACL.04`, by
//!   another user). The held amount falls only when the refund is `paid`
//!   ([`refund_moved`]); it goes back up if the refund `fail`s, and falls again
//!   when the failed refund is cancelled (the amount then sits on the party's
//!   credit). A `rejected` refund leaves the held amount as it is, and the
//!   difference is paid back at settlement. Only what was paid in above the new
//!   amount is refunded: when the deposit holds no more than the new amount
//!   there is nothing to refund and the adjustment is settled at once.
//!
//! The same amount records nothing. A second adjustment is refused while one is
//! open: the top-up unpaid, or the part refund not yet paid, rejected or
//! cancelled. Settling the deposit before a top-up is paid uses the held amount
//! and the unpaid top-up charge stays on the party's balance; a part refund
//! that is not settled holds back the refund at settlement and the transfer.
//!
//! After the commit ([`after_adjust`]) the top-up charge is requested and
//! `DepositAdjusted` is emitted. The part refund's money in the credit of a
//! failed refund follows the Refund module's rule for deposit refunds
//! (`credit_balance`), unchanged here.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::deposits::{
    AdjustDeposit, CancelSettlement, DepositAdjusted, DepositFuture, Deposits, HeldDeposit,
    SettlementStarted, TopUpCharge, deposit_top_ups,
};
use wl_kernel::payments::RefundId;
use wl_kernel::Subject;

use crate::deposit::{Deposit, DepositStatus};
use crate::followup::{FollowupTask, money_text};
use crate::notices;
use crate::permissions;
use crate::refund::{Refund, request_part_for_deposit_in};
use crate::refund_fail::{FAIL, FAIL_BY_SYSTEM};
use crate::refund_machine::{PAY, PAY_BY_SYSTEM, REJECT};
use crate::refund_recover::CANCEL;

/// The `adjust` transition's name (a user).
pub const ADJUST: &str = "adjust";
/// The `adjust_by_system` transition's name (the source agreement changed).
pub const ADJUST_BY_SYSTEM: &str = "adjust_by_system";

/// What an adjustment does.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AdjustmentKind {
    /// The amount grew: a top-up charge.
    #[default]
    TopUp,
    /// The amount shrank: a part refund.
    PartRefund,
}

/// Where an adjustment stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AdjustmentStatus {
    /// The top-up is unpaid, or the part refund is not settled.
    #[default]
    Open,
    /// The top-up is paid, or the part refund is paid.
    Settled,
    /// The part refund was rejected or cancelled.
    Closed,
}

/// One change of the amount of a held deposit.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_deposit_adjustments",
    index(deposit_id, status),
    index(refund_id)
)]
pub struct DepositAdjustment {
    /// The primary key.
    pub id: i64,
    /// The deposit.
    pub deposit_id: i64,
    /// The version of the source that changed the amount.
    pub source_version: i32,
    /// The amount before, in fils.
    pub old_amount: i64,
    /// The amount after, in fils.
    pub new_amount: i64,
    /// The day the change takes effect.
    pub effective_on: NaiveDate,
    /// A top-up or a part refund.
    pub kind: AdjustmentKind,
    /// The increase, or the part refunded, in fils.
    pub difference: i64,
    /// The type of the top-up charge, once raised.
    pub charge_subject_type: Option<String>,
    /// The id of the top-up charge.
    pub charge_subject_id: Option<i64>,
    /// The part refund of a decrease.
    pub refund_id: Option<i64>,
    /// Whether the held amount is lowered by the part refund now.
    pub held_applied: bool,
    /// Where it stands.
    pub status: AdjustmentStatus,
    /// Why a part refund closed: rejected or cancelled.
    pub closed_reason: Option<String>,
    /// The user who adjusted; `None` for the system.
    pub adjusted_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl DepositAdjustment {
    /// The top-up charge, if one was raised.
    pub fn charge(&self) -> Option<Subject> {
        match (&self.charge_subject_type, self.charge_subject_id) {
            (Some(subject_type), Some(subject_id)) => Some(Subject {
                subject_type: subject_type.clone(),
                subject_id,
            }),
            _ => None,
        }
    }

    /// The adjustments of `deposit_id`, oldest first.
    pub async fn of_deposit(db: &Db, deposit_id: i64) -> Result<Vec<DepositAdjustment>> {
        DepositAdjustment::query()
            .where_(DepositAdjustment::DEPOSIT_ID.eq(deposit_id))
            .order_by(DepositAdjustment::ID)
            .get(db)
            .await
    }

    /// The latest adjustment of `deposit_id`.
    pub async fn latest(db: &Db, deposit_id: i64) -> Result<Option<DepositAdjustment>> {
        Ok(Self::of_deposit(db, deposit_id).await?.pop())
    }

    /// The adjustments of `deposit_id` that are still open.
    pub async fn open_of_deposit(db: &Db, deposit_id: i64) -> Result<Vec<DepositAdjustment>> {
        DepositAdjustment::query()
            .where_(DepositAdjustment::DEPOSIT_ID.eq(deposit_id))
            .where_(DepositAdjustment::STATUS.eq(AdjustmentStatus::Open))
            .order_by(DepositAdjustment::ID)
            .get(db)
            .await
    }

    /// Whether a part refund of `deposit_id` is not settled yet.
    pub async fn part_refund_open(db: &Db, deposit_id: i64) -> Result<bool> {
        Ok(Self::open_of_deposit(db, deposit_id)
            .await?
            .iter()
            .any(|a| a.kind == AdjustmentKind::PartRefund && a.refund_id.is_some()))
    }
}

/// What an adjustment of a deposit comes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    /// A top-up or a part refund.
    pub kind: AdjustmentKind,
    /// The increase, or the part refunded, in fils.
    pub difference: i64,
}

/// What changing `deposit`'s amount to `new_amount` comes to: an increase is a
/// top-up of the difference; a decrease refunds the difference, but only what
/// the deposit holds above the new amount.
pub fn plan_of(deposit: &Deposit, new_amount: i64) -> Plan {
    if new_amount > deposit.amount {
        Plan {
            kind: AdjustmentKind::TopUp,
            difference: new_amount - deposit.amount,
        }
    } else {
        let over = (deposit.held_amount - new_amount).max(0);
        Plan {
            kind: AdjustmentKind::PartRefund,
            difference: (deposit.amount - new_amount).min(over),
        }
    }
}

/// What a user (or the source agreement) gives to adjust a deposit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdjustRequest {
    /// The new amount the source requires, in fils; above 0.
    pub new_amount: i64,
    /// The version of the source that changed it; above 0.
    pub source_version: i32,
    /// The day the change takes effect.
    pub effective_on: NaiveDate,
}

/// What an `adjust` run carries to its guards and effect, gathered by
/// [`adjust`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct AdjustInput {
    /// The new amount, in fils.
    pub new_amount: i64,
    /// The version of the source.
    pub source_version: i32,
    /// The day it takes effect.
    pub effective_on: NaiveDate,
    /// Whether an earlier adjustment is still open.
    pub open_adjustment: bool,
    /// The user who adjusts; `None` for the system.
    pub by: Option<i64>,
}

fn adjust_input(ctx: &TransitionCtx) -> Option<AdjustInput> {
    ctx.input_as::<AdjustInput>().ok()
}

/// Why `deposit` cannot be adjusted with `input`, if it cannot.
pub(crate) fn adjust_problem(deposit: &Deposit, input: Option<&AdjustInput>) -> Option<String> {
    let Some(input) = input else {
        return Some("The adjustment was not prepared by the deposit service.".to_owned());
    };
    if input.new_amount <= 0 {
        return Some("The new amount must be above 0.".to_owned());
    }
    if input.source_version <= 0 {
        return Some("The version of the source that changed the amount is required.".to_owned());
    }
    if input.new_amount == deposit.amount {
        return Some("The amount is the same: there is nothing to adjust.".to_owned());
    }
    if input.open_adjustment {
        return Some(
            "An earlier adjustment is still open: wait until its top-up is paid or its part refund is paid, rejected or cancelled."
                .to_owned(),
        );
    }
    None
}

fn adjusting(transition: Transition<Deposit>) -> Transition<Deposit> {
    transition
        .from([DepositStatus::Held])
        .to(DepositStatus::Held)
        .guard("amount_changes_and_none_is_open", |deposit: &Deposit, ctx| {
            let problem = adjust_problem(deposit, adjust_input(ctx).as_ref());
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|deposit: &Deposit, ctx, tx| {
            let deposit = deposit.clone();
            let input = adjust_input(ctx);
            Box::pin(async move {
                let input = input.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("an adjustment without its input"))
                })?;
                let plan = plan_of(&deposit, input.new_amount);
                let mut adjustment = DepositAdjustment {
                    deposit_id: deposit.id,
                    source_version: input.source_version,
                    old_amount: deposit.amount,
                    new_amount: input.new_amount,
                    effective_on: input.effective_on,
                    kind: plan.kind,
                    difference: plan.difference,
                    adjusted_by: input.by,
                    status: AdjustmentStatus::Open,
                    ..Default::default()
                };
                if plan.kind == AdjustmentKind::PartRefund {
                    if plan.difference > 0 {
                        let reason = format!(
                            "Part refund of deposit {} after version {} of its agreement",
                            deposit.id, input.source_version
                        );
                        let refund = request_part_for_deposit_in(
                            &mut *tx,
                            &deposit,
                            plan.difference,
                            input.by,
                            &reason,
                        )
                        .await?;
                        adjustment.refund_id = Some(refund.id);
                    } else {
                        // Nothing was paid in above the new amount.
                        adjustment.status = AdjustmentStatus::Settled;
                    }
                }
                adjustment.insert(&mut *tx).await?;
                // Not the status column, so outside the machine's guard.
                Deposit::query()
                    .where_(Deposit::ID.eq(deposit.id))
                    .update(
                        &mut *tx,
                        &[
                            ("amount", &input.new_amount),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `adjust` and `adjust_by_system`, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Deposit>> {
    vec![
        adjusting(
            Transition::new(ADJUST)
                .label("Adjust")
                .tone("primary")
                .permission(permissions::DEPOSITS_MANAGE),
        ),
        adjusting(
            Transition::new(ADJUST_BY_SYSTEM)
                .label("Adjust (the source changed)")
                .system(),
        ),
    ]
}

async fn reload(state: &AppState, deposit: &Deposit) -> std::result::Result<Deposit, TransitionError> {
    Deposit::find_or_404(&state.db, deposit.id)
        .await
        .map_err(TransitionError::Failed)
}

/// Adjusts the held `deposit` to `request.new_amount`: `actor` is the user,
/// `None` is the system (the source agreement changed; `by` names the user
/// behind it, if any). Returns the adjustment, or `None` when the amount is
/// the same and nothing is recorded. The top-up charge and the event follow
/// the commit ([`after_adjust`]).
///
/// # Errors
///
/// `Forbidden` for a user without `deposits.manage`; `NotFrom` unless
/// `held`; `Guard` (422) for a new amount or version not above 0, or while an
/// earlier adjustment is open.
pub async fn adjust(
    state: &AppState,
    actor: Option<&AuthUser>,
    by: Option<i64>,
    deposit: &Deposit,
    request: &AdjustRequest,
) -> std::result::Result<Option<DepositAdjustment>, TransitionError> {
    if let Some(user) = actor
        && !user.has_permission(permissions::DEPOSITS_MANAGE)
    {
        return Err(TransitionError::Forbidden);
    }
    let deposit = reload(state, deposit).await?;
    if deposit.status != DepositStatus::Held {
        return Err(TransitionError::NotFrom(deposit.status.as_str().to_owned()));
    }
    if request.new_amount == deposit.amount {
        return Ok(None);
    }
    let input = AdjustInput {
        new_amount: request.new_amount,
        source_version: request.source_version,
        effective_on: request.effective_on,
        open_adjustment: !DepositAdjustment::open_of_deposit(&state.db, deposit.id)
            .await?
            .is_empty(),
        by: actor.map(|user| user.id).or(by),
    };
    let run = match actor {
        Some(user) => deposit.transition(ADJUST).by(user),
        None => deposit.transition(ADJUST_BY_SYSTEM).by_system(),
    };
    let _outcome = run.input(&input).run(state).await?;
    Ok(DepositAdjustment::latest(&state.db, deposit.id).await?)
}

/// The `DepositAdjusted` event for `adjustment` of `deposit`.
fn event_of(deposit: &Deposit, adjustment: &DepositAdjustment) -> DepositAdjusted {
    DepositAdjusted {
        deposit: deposit.deposit_id(),
        source: deposit.source(),
        party: deposit.party(),
        source_version: adjustment.source_version,
        old_amount: adjustment.old_amount,
        new_amount: adjustment.new_amount,
        effective_on: adjustment.effective_on,
        top_up_charge: adjustment.charge(),
        part_refund: adjustment.refund_id.map(RefundId),
    }
}

/// After a committed `adjust` of `deposit_id`: the top-up charge of an increase
/// is requested (the accountant gets a task when nothing can raise it), then
/// `DepositAdjusted` is emitted.
///
/// # Errors
///
/// The database's errors, or a notice that failed.
pub(crate) async fn after_adjust(state: &AppState, deposit_id: i64) -> Result<()> {
    let deposit = Deposit::find_or_404(&state.db, deposit_id).await?;
    let Some(mut adjustment) = DepositAdjustment::latest(&state.db, deposit_id).await? else {
        return Ok(());
    };
    if adjustment.kind == AdjustmentKind::TopUp {
        let charge = TopUpCharge {
            deposit: deposit.deposit_id(),
            party: deposit.party(),
            legal_entity_id: deposit.legal_entity_id,
            source: deposit.source(),
            source_version: adjustment.source_version,
            amount: adjustment.difference,
            currency: deposit.currency.clone(),
            due: adjustment.effective_on,
        };
        match deposit_top_ups(state).request(state, charge).await? {
            Some(made) => {
                adjustment.charge_subject_type = Some(made.subject_type);
                adjustment.charge_subject_id = Some(made.subject_id);
                adjustment.save(&state.db).await?;
            }
            None => {
                // Nothing can schedule the charge: the accountant raises it.
                let task = FollowupTask::raise_top_up(
                    &deposit,
                    adjustment.difference,
                    adjustment.source_version,
                    adjustment.effective_on,
                );
                if let Some(task) = FollowupTask::create_once(&state.db, task).await? {
                    notices::deposit_task(state, &deposit, &task).await?;
                }
            }
        }
    }
    state.emit(event_of(&deposit, &adjustment)).await
}

/// In `tx`, after money was allocated to the deposit `deposit_id`: the open
/// top-ups are settled once it holds its (new) amount.
///
/// # Errors
///
/// The database's errors.
pub(crate) async fn settle_top_ups_in(
    tx: &mut renox::db::Transaction,
    deposit_id: i64,
    held: i64,
    amount: i64,
) -> Result<()> {
    if held < amount {
        return Ok(());
    }
    DepositAdjustment::query()
        .where_(DepositAdjustment::DEPOSIT_ID.eq(deposit_id))
        .where_(DepositAdjustment::KIND.eq(AdjustmentKind::TopUp))
        .where_(DepositAdjustment::STATUS.eq(AdjustmentStatus::Open))
        .update(
            &mut *tx,
            &[
                ("status", &AdjustmentStatus::Settled.as_str().to_owned()),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// Sets the held amount of `deposit_id` to `held` (never below 0).
async fn set_held(state: &AppState, deposit_id: i64, held: i64) -> Result<()> {
    // Not the status column, so outside the machine's guard.
    Deposit::query()
        .where_(Deposit::ID.eq(deposit_id))
        .update(
            &state.db,
            &[
                ("held_amount", &held.max(0)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// After the refund transition `name` of `refund_id`: if it is the part refund
/// of an adjustment, the held amount follows it: lowered when it is `paid`,
/// raised again when it `fail`s, lowered again when the failed refund is
/// cancelled; a rejected refund leaves it. Any other refund is left alone.
///
/// # Errors
///
/// The database's errors.
pub(crate) async fn refund_moved(state: &AppState, refund_id: i64, name: &str) -> Result<()> {
    let Some(refund) = Refund::find(&state.db, refund_id).await? else {
        return Ok(());
    };
    if refund.source_kind != "deposit" {
        return Ok(());
    }
    let Some(mut adjustment) = DepositAdjustment::query()
        .where_(DepositAdjustment::REFUND_ID.eq(refund.id))
        .first(&state.db)
        .await?
    else {
        return Ok(());
    };
    let deposit = Deposit::find_or_404(&state.db, adjustment.deposit_id).await?;
    match name {
        PAY | PAY_BY_SYSTEM if !adjustment.held_applied => {
            set_held(state, deposit.id, deposit.held_amount - refund.amount).await?;
            adjustment.held_applied = true;
            adjustment.status = AdjustmentStatus::Settled;
        }
        FAIL | FAIL_BY_SYSTEM if adjustment.held_applied => {
            set_held(state, deposit.id, deposit.held_amount + refund.amount).await?;
            adjustment.held_applied = false;
            adjustment.status = AdjustmentStatus::Open;
        }
        REJECT => {
            adjustment.status = AdjustmentStatus::Closed;
            adjustment.closed_reason = Some("rejected".to_owned());
        }
        CANCEL => {
            if !adjustment.held_applied {
                set_held(state, deposit.id, deposit.held_amount - refund.amount).await?;
                adjustment.held_applied = true;
            }
            adjustment.status = AdjustmentStatus::Closed;
            adjustment.closed_reason = Some("cancelled".to_owned());
        }
        _ => return Ok(()),
    }
    adjustment.save(&state.db).await?;
    Ok(())
}

/// The listener on the refunds: a part refund moves the held amount.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Refund>, state: AppState| async move {
        refund_moved(&state, event.key, event.transition.as_str()).await
    });
}

/// The module's [`Deposits`] for the verticals.
#[derive(Debug, Clone, Copy, Default)]
pub struct DepositBook;

impl Deposits for DepositBook {
    fn held_for<'a>(
        &'a self,
        db: &'a Db,
        source: &'a Subject,
    ) -> DepositFuture<'a, Option<HeldDeposit>> {
        Box::pin(async move {
            let found = Deposit::query()
                .where_(Deposit::SOURCE_TYPE.eq(source.subject_type.as_str()))
                .where_(Deposit::SOURCE_ID.eq(source.subject_id))
                .where_(Deposit::STATUS.eq(DepositStatus::Held))
                .order_by_desc(Deposit::ID)
                .first(db)
                .await?;
            Ok(found.map(|deposit| HeldDeposit {
                deposit: deposit.deposit_id(),
                party: deposit.party(),
                required: deposit.amount,
                held: deposit.held_amount,
                currency: deposit.currency.clone(),
            }))
        })
    }

    fn settle_cancelled<'a>(
        &'a self,
        state: &'a AppState,
        request: CancelSettlement,
        by: Option<i64>,
    ) -> DepositFuture<'a, Option<SettlementStarted>> {
        Box::pin(async move { crate::deposit_cancel::settle(state, request, by).await })
    }

    fn start_settlement<'a>(
        &'a self,
        state: &'a AppState,
        source: &'a Subject,
    ) -> DepositFuture<'a, Option<SettlementStarted>> {
        Box::pin(async move { crate::deposit_cancel::begin(state, source).await })
    }

    fn adjust<'a>(
        &'a self,
        state: &'a AppState,
        request: AdjustDeposit,
        by: Option<i64>,
    ) -> DepositFuture<'a, Option<DepositAdjusted>> {
        Box::pin(async move {
            let deposit = Deposit::query()
                .where_(Deposit::SOURCE_TYPE.eq(request.source.subject_type.as_str()))
                .where_(Deposit::SOURCE_ID.eq(request.source.subject_id))
                .where_(Deposit::STATUS.eq(DepositStatus::Held))
                .order_by_desc(Deposit::ID)
                .first(&state.db)
                .await?
                .ok_or_else(|| {
                    Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        format!("{} has no held deposit.", request.source),
                    )
                })?;
            let adjust_request = AdjustRequest {
                new_amount: request.new_amount,
                source_version: request.source_version,
                effective_on: request.effective_on,
            };
            let adjustment = adjust(state, None, by, &deposit, &adjust_request)
                .await
                .map_err(TransitionError::into_error)?;
            let deposit = Deposit::find_or_404(&state.db, deposit.id).await?;
            Ok(adjustment.map(|a| event_of(&deposit, &a)))
        })
    }
}

/// The text of how an adjustment reads in a list: `AED 5,000.00 to AED
/// 6,000.00`.
pub fn change_text(currency: &str, adjustment: &DepositAdjustment) -> String {
    format!(
        "{} → {}",
        money_text(currency, adjustment.old_amount),
        money_text(currency, adjustment.new_amount)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(amount: i64, held: i64) -> Deposit {
        Deposit {
            id: 3,
            amount,
            held_amount: held,
            status: DepositStatus::Held,
            ..Default::default()
        }
    }

    fn input(new_amount: i64) -> AdjustInput {
        AdjustInput {
            new_amount,
            source_version: 2,
            effective_on: NaiveDate::from_ymd_opt(2027, 5, 1).unwrap(),
            open_adjustment: false,
            by: None,
        }
    }

    #[test]
    fn adjust_keeps_the_deposit_held() {
        let machine = Deposit::machine();
        for (name, system) in [(ADJUST, false), (ADJUST_BY_SYSTEM, true)] {
            let found = machine.find(name).expect("transition");
            assert_eq!(found.is_system(), system, "{name}");
            assert!(found.check_from(DepositStatus::Held).is_ok(), "{name}");
            for other in [
                DepositStatus::SettlementPending,
                DepositStatus::Refunded,
                DepositStatus::Forfeited,
                DepositStatus::Transferred,
            ] {
                assert!(found.check_from(other).is_err(), "{name} {other:?}");
            }
            assert_eq!(found.to_state(), Some("held"), "{name}");
        }
    }

    #[test]
    fn an_increase_is_a_top_up_of_the_difference() {
        let plan = plan_of(&held(500_000, 500_000), 600_000);
        assert_eq!(plan.kind, AdjustmentKind::TopUp);
        assert_eq!(plan.difference, 100_000);
    }

    #[test]
    fn a_decrease_refunds_the_difference_paid_in_above_the_new_amount() {
        let plan = plan_of(&held(600_000, 600_000), 550_000);
        assert_eq!(plan.kind, AdjustmentKind::PartRefund);
        assert_eq!(plan.difference, 50_000);
        // Only 560,000 was paid in of the 600,000: 10,000 is above 550,000.
        let partial = plan_of(&held(600_000, 560_000), 550_000);
        assert_eq!(partial.difference, 10_000);
        // Nothing above the new amount: nothing to refund.
        let little = plan_of(&held(600_000, 300_000), 550_000);
        assert_eq!(little.kind, AdjustmentKind::PartRefund);
        assert_eq!(little.difference, 0);
    }

    #[test]
    fn an_adjustment_needs_a_new_amount_a_version_and_nothing_open() {
        let deposit = held(500_000, 500_000);
        assert_eq!(adjust_problem(&deposit, Some(&input(600_000))), None);
        assert!(adjust_problem(&deposit, None).is_some());
        assert!(adjust_problem(&deposit, Some(&input(0))).is_some());
        let no_version = AdjustInput {
            source_version: 0,
            ..input(600_000)
        };
        assert!(adjust_problem(&deposit, Some(&no_version)).unwrap().contains("version"));
        assert!(adjust_problem(&deposit, Some(&input(500_000))).unwrap().contains("same"));
        let open = AdjustInput {
            open_adjustment: true,
            ..input(600_000)
        };
        assert!(adjust_problem(&deposit, Some(&open)).unwrap().contains("still open"));
    }

    #[renox::test]
    async fn the_guard_reads_the_run_input() {
        let machine = Deposit::machine();
        let found = machine.find(ADJUST).expect("transition");
        let ctx = |input: &AdjustInput| {
            TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
        };
        let deposit = held(500_000, 500_000);
        assert!(found.check_guards(&deposit, &ctx(&input(600_000))).await.is_ok());
        assert!(matches!(
            found.check_guards(&deposit, &ctx(&input(500_000))).await,
            Err(TransitionError::Guard(_))
        ));
        assert!(matches!(
            found.check_guards(&deposit, &TransitionCtx::new(None)).await,
            Err(TransitionError::Guard(_))
        ));
    }

    #[test]
    fn an_adjustment_reads_as_a_change_of_amount() {
        let adjustment = DepositAdjustment {
            old_amount: 500_000,
            new_amount: 600_000,
            ..Default::default()
        };
        assert_eq!(change_text("AED", &adjustment), "AED 5,000.00 → AED 6,000.00");
        assert_eq!(AdjustmentStatus::default(), AdjustmentStatus::Open);
        assert_eq!(AdjustmentKind::PartRefund.as_str(), "part_refund");
    }
}
