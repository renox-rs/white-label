//! The Redemption state machine (wl-sales §5.9 step 3; BL-SAL-23).
//!
//! ```text
//! [*] pending --confirm--> held --apply--> applied
//!       |                    |
//!       |--refuse--> refused |--release--> released
//!       +--release---------------------->
//! ```
//!
//! Every transition is `.system()`: [`crate::redeem`] checks who may ask, the
//! one-time code and the programme's terms, and gives the machine what its
//! guards and effects cannot fetch (an effect has no `AppState`).
//!
//! - `confirm` holds the points: `points_held` of the account goes up, so the
//!   same points cannot be redeemed on another till.
//! - `apply` spends them: one `redeem` movement per lot slice, earliest expiry
//!   first, the balance and the hold go down. A shortfall fails the
//!   transition, which rolls back, so a lot is never over-consumed.
//! - `release` lets the hold go (the hold lapsed, the sale was abandoned or
//!   the order's payment failed) and writes no movement.
//! - `refuse` ends a redemption whose code was wrong three times or could not
//!   be sent.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::sales::PointsRedeemed;

use crate::loyalty_account::LoyaltyAccount;
use crate::loyalty_points::{CAUSE_REDEEM, PointsKind, PointsMovement};
use crate::redemption::{Redemption, RedemptionStatus};
use crate::texts::english;

/// `pending → held`: the member confirmed; the points are held.
pub const CONFIRM: &str = "confirm";
/// `held → applied`: the sale completed or the order was confirmed.
pub const APPLY: &str = "apply";
/// `pending | held → released`: nothing was spent.
pub const RELEASE: &str = "release";
/// `pending → refused`: three wrong codes, or the code could not be sent.
pub const REFUSE: &str = "refuse";

/// What `confirm` takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmInput {
    /// The redemption line on the sale.
    pub line_id: i64,
    /// `code` or `portal`.
    pub via: String,
    /// Held until; empty: until the sale resolves.
    pub hold_until: Option<DateTime>,
    /// The sale's value with VAT after the other discounts, in fils.
    pub basis: i64,
    /// The points the member can spend now, other holds taken off.
    pub available: i64,
}

/// What `apply` takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyInput {
    /// The day (Asia/Dubai) the points are spent: lots past it are out.
    pub on: NaiveDate,
    /// The legal entity of the sale.
    pub entity: Option<i64>,
    /// What one point is worth, in fils.
    pub point_value: i64,
    /// The sale's value with VAT after the other discounts, in fils.
    pub basis: i64,
}

/// A guard that fails with the English `key` unless `ok`.
fn check(ok: bool, key: &str) -> std::result::Result<(), String> {
    if ok { Ok(()) } else { Err(english(key, &[])) }
}

/// Spends `points` of `account` from its lots, earliest expiry first, one
/// `redeem` movement per slice, and takes them off the balance and the hold.
/// Fails (422, the transaction rolls back) when the usable lots hold fewer.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn consume_lots(
    tx: &mut renox::db::Transaction,
    account: i64,
    points: i64,
    on: NaiveDate,
    point_value: i64,
    source: &Subject,
    entity: Option<i64>,
    kind: PointsKind,
    cause: &str,
) -> Result {
    let mut left = points;
    for lot in PointsMovement::lots(&mut *tx, account, Some(on)).await? {
        if left == 0 {
            break;
        }
        let take = lot.lot_remaining().min(left);
        PointsMovement {
            account_id: account,
            kind,
            points: -take,
            value: take * point_value,
            legal_entity_id: entity,
            source_type: source.subject_type.clone(),
            source_id: source.subject_id,
            earning_id: lot.earning_id,
            cause: cause.to_owned(),
            lot_id: Some(lot.id),
            lot_expires_on: lot.lot_expires_on,
            ..PointsMovement::default()
        }
        .insert(&mut *tx)
        .await?;
        PointsMovement::query()
            .where_(PointsMovement::ID.eq(lot.id))
            .increment(&mut *tx, "consumed", take)
            .await?;
        left -= take;
    }
    if left > 0 {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            english("wl_sales.redeem.problem.short", &[]),
        ));
    }
    LoyaltyAccount::query()
        .where_(LoyaltyAccount::ID.eq(account))
        .increment(&mut *tx, "points_balance", -points)
        .await?;
    Ok(())
}

impl StateMachine for Redemption {
    type State = RedemptionStatus;
    const COLUMN: renox::db::Col<Redemption, RedemptionStatus> = Redemption::STATUS;

    fn machine() -> Machine<Self> {
        use RedemptionStatus::{Applied, Held, Pending, Refused, Released};
        Machine::new()
            .initial([Pending])
            .state(Pending)
            .tone("info")
            .state(Held)
            .tone("info")
            .state(Applied)
            .tone("success")
            .terminal()
            .state(Released)
            .tone("neutral")
            .terminal()
            .state(Refused)
            .tone("danger")
            .terminal()
            .transition(confirm())
            .transition(apply())
            .transition(release())
            .transition(refuse())
    }
}

/// `confirm`: pending → held.
fn confirm() -> Transition<Redemption> {
    Transition::new(CONFIRM)
        .label("Confirm")
        .tone("primary")
        .system()
        .from([RedemptionStatus::Pending])
        .to(RedemptionStatus::Held)
        .guard("points_available", |r: &Redemption, ctx| {
            let points = r.points;
            let available = ctx.input_as::<ConfirmInput>().map(|i| i.available);
            Box::pin(async move {
                check(
                    available.is_ok_and(|a| a >= points),
                    "wl_sales.redeem.problem.not_enough",
                )
            })
        })
        .effect(|r: &Redemption, ctx, tx| {
            let (id, account, points) = (r.id, r.account_id, r.points);
            let input = ctx.input_as::<ConfirmInput>().ok();
            Box::pin(async move {
                let Some(input) = input else {
                    return Ok(());
                };
                let via = Some(input.via.clone());
                let line = Some(input.line_id);
                Redemption::query()
                    .where_(Redemption::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("hold_until", &input.hold_until),
                            ("confirmed_via", &via),
                            ("line_id", &line),
                            ("basis", &input.basis),
                            ("updated_at", &now()),
                        ],
                    )
                    .await?;
                LoyaltyAccount::query()
                    .where_(LoyaltyAccount::ID.eq(account))
                    .increment(&mut *tx, "points_held", points)
                    .await?;
                Ok(())
            })
        })
}

/// `apply`: held → applied; the points are spent.
fn apply() -> Transition<Redemption> {
    Transition::new(APPLY)
        .label("Apply")
        .tone("success")
        .system()
        .from([RedemptionStatus::Held])
        .to(RedemptionStatus::Applied)
        .guard("has_input", |_r: &Redemption, ctx| {
            let ok = ctx.input_as::<ApplyInput>().is_ok();
            Box::pin(async move { check(ok, "wl_sales.redeem.problem.input") })
        })
        .effect(|r: &Redemption, ctx, tx| {
            let r = r.clone();
            let input = ctx.input_as::<ApplyInput>().ok();
            Box::pin(async move {
                let Some(input) = input else {
                    return Ok(());
                };
                consume_lots(
                    tx,
                    r.account_id,
                    r.points,
                    input.on,
                    input.point_value,
                    &r.sale(),
                    input.entity,
                    PointsKind::Redeem,
                    CAUSE_REDEEM,
                )
                .await?;
                LoyaltyAccount::query()
                    .where_(LoyaltyAccount::ID.eq(r.account_id))
                    .increment(&mut *tx, "points_held", -r.points)
                    .await?;
                let applied = Some(now());
                Redemption::query()
                    .where_(Redemption::ID.eq(r.id))
                    .update(
                        &mut *tx,
                        &[
                            ("applied_at", &applied),
                            ("basis", &input.basis),
                            ("updated_at", &now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `release`: pending or held → released.
fn release() -> Transition<Redemption> {
    Transition::new(RELEASE)
        .label("Release")
        .tone("warning")
        .system()
        .from([RedemptionStatus::Pending, RedemptionStatus::Held])
        .to(RedemptionStatus::Released)
        .effect(|r: &Redemption, _ctx, tx| {
            let (account, points) = (r.account_id, r.points);
            let was_held = r.status == RedemptionStatus::Held;
            Box::pin(async move {
                if was_held {
                    LoyaltyAccount::query()
                        .where_(LoyaltyAccount::ID.eq(account))
                        .increment(&mut *tx, "points_held", -points)
                        .await?;
                }
                Ok(())
            })
        })
}

/// `refuse`: pending → refused, with the reason (`too_many_tries` or
/// `code_not_sent`).
fn refuse() -> Transition<Redemption> {
    Transition::new(REFUSE)
        .label("Refuse")
        .tone("danger")
        .system()
        .from([RedemptionStatus::Pending])
        .to(RedemptionStatus::Refused)
        .effect(|r: &Redemption, ctx, tx| {
            let id = r.id;
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move {
                Redemption::query()
                    .where_(Redemption::ID.eq(id))
                    .update(&mut *tx, &[("refusal", &reason), ("updated_at", &now())])
                    .await?;
                Ok(())
            })
        })
}

/// Registers the machine and the listener that announces the points spent.
pub(crate) fn register_events(app: &mut Registry) {
    renox::state_machine::register::<Redemption>();
    app.listen(
        |event: Transitioned<Redemption>, state: AppState| async move {
            if event.transition != APPLY {
                return Ok(());
            }
            let Some(row) = Redemption::find(&state.db, event.key).await? else {
                return Ok(());
            };
            let Some(account) = LoyaltyAccount::find(&state.db, row.account_id).await? else {
                return Ok(());
            };
            state
                .emit(PointsRedeemed {
                    account: account.account_id(),
                    party: account.party(),
                    points: row.points,
                    value: row.value,
                    source: row.sale(),
                })
                .await
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_is_declared_and_system() {
        let machine = Redemption::machine();
        let table = [
            (CONFIRM, vec!["pending"], "held"),
            (APPLY, vec!["held"], "applied"),
            (RELEASE, vec!["pending", "held"], "released"),
            (REFUSE, vec!["pending"], "refused"),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to) in table {
            let t = machine.find(name).expect(name);
            assert!(t.is_system(), "{name}");
            assert_eq!(t.from_states(), from, "{name}");
            assert_eq!(t.to_state(), Some(to), "{name}");
        }
        assert_eq!(machine.initial_states(), ["pending"]);
        let states: Vec<&str> = machine.states().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(states, ["pending", "held", "applied", "released", "refused"]);
    }

    #[test]
    fn confirm_waits_for_points_that_are_available() {
        let machine = Redemption::machine();
        assert_eq!(
            machine.find(CONFIRM).unwrap().guard_names(),
            ["points_available"]
        );
    }
}
