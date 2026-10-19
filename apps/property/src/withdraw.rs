//! Withdrawing a notice (`withdraw_notice`, `PRP.ACL.20`, 01 Property §3.3,
//! §4.3 row "The tenant withdraws the notice", §7.1 *Lease* and *Unit*;
//! BL-PRP-10).
//!
//! A tenant who gave notice may change their mind, and a landlord may take a
//! notice back. With the landlord side's written agreement attached to the
//! lease ([`AGREEMENT_KIND`]) the property manager runs `withdraw_notice`
//! ([`withdraw`]): the lease goes back to the state its dates give
//! ([`target_for`]), the unit to `occupied` (`cancel_notice`), the move-out
//! inspection ticket is closed and a penalty that is not invoiced yet is
//! cancelled. An invoiced penalty stays; only a credit note corrects it, by
//! hand. `LeaseNoticeWithdrawn` follows the commit.
//!
//! It is refused without the agreement, after the move-out inspection was
//! completed, and once a lease for another tenant on the unit was submitted
//! (`pending_signature` or later; a draft does not count).
//!
//! The lease goes back to:
//!
//! - `active` before the renewal window (`PRP.RUL.01`),
//! - `renewal_due` inside the window up to the end date,
//! - `expired` after the end date. When no holding-over extension exists yet,
//!   the effects of `expire` run as if the lease expired that day (the charge
//!   schedule is extended for the statutory period, and the daily rule opens
//!   the holding-over task); the property managers are told after the commit.
//!
//! The notice is cleared from the lease; the history of the machine keeps the
//! transition with its reason. The three targets are three transitions
//! (`withdraw_notice`, `withdraw_notice_to_renewal_due`,
//! `withdraw_notice_to_expired`), as the machine of the spec splits a
//! transition by target.

use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{StateMachine, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::{guard, today};
use wl_kernel::tickets::task_creator;

use crate::building::refused;
use crate::charge::{ChargeStatus, LeaseCharge};
use crate::charge_type::EARLY_TERMINATION_FEE;
use crate::events::LeaseNoticeWithdrawn;
use crate::holding::{announce, expire_effects};
use crate::lease::{Lease, LeaseStatus};
use crate::lease_machine::{
    WITHDRAW_NOTICE, WITHDRAW_NOTICE_TO_EXPIRED, WITHDRAW_NOTICE_TO_RENEWAL_DUE,
};
use crate::notice::move_out_key;
use crate::permissions;
use crate::renewal::window_opens_on;
use crate::texts::english;
use crate::unit::Unit;
use crate::unit_machine::CANCEL_NOTICE;

/// The document kind of the landlord side's written agreement to withdraw a
/// notice (the owner's or the property manager's signed agreement).
pub const AGREEMENT_KIND: &str = "notice_withdrawal_agreement";

/// What a `withdraw_notice` run carries: the day it is run for. A run without
/// it counts today.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WithdrawInput {
    /// The day (Asia/Dubai).
    #[serde(default)]
    pub day: Option<NaiveDate>,
}

/// The day a `withdraw_notice` run is for.
pub fn withdraw_day(ctx: &TransitionCtx) -> NaiveDate {
    ctx.input_as::<WithdrawInput>()
        .ok()
        .and_then(|input| input.day)
        .unwrap_or_else(today)
}

/// The state the dates of `lease` give on `day`: `active` before the renewal
/// window, `renewal_due` inside it up to the end date, `expired` after the end
/// date.
pub fn target_for(lease: &Lease, day: NaiveDate) -> LeaseStatus {
    if day > lease.end_date {
        LeaseStatus::Expired
    } else if day >= window_opens_on(lease) {
        LeaseStatus::RenewalDue
    } else {
        LeaseStatus::Active
    }
}

/// The name of the transition that goes back to `target`.
pub fn transition_for(target: LeaseStatus) -> &'static str {
    match target {
        LeaseStatus::RenewalDue => WITHDRAW_NOTICE_TO_RENEWAL_DUE,
        LeaseStatus::Expired => WITHDRAW_NOTICE_TO_EXPIRED,
        _ => WITHDRAW_NOTICE,
    }
}

/// Why `lease` cannot go back to `target` on `day`: the dates give another
/// state, or the move-out inspection was completed. `None` when it can.
pub fn problem(lease: &Lease, target: LeaseStatus, day: NaiveDate) -> Option<String> {
    if lease.inspection_done_on.is_some() {
        return Some(english("property.withdraw.inspection_done", &[]));
    }
    let given = target_for(lease, day);
    if given != target {
        return Some(english(
            "property.withdraw.wrong_target",
            &[("state", &given.as_str())],
        ));
    }
    None
}

/// The effect of `withdraw_notice` inside its transaction: the other tenant's
/// lease is checked again, the notice is cleared, the unit goes back to
/// `occupied`, the penalty that is not invoiced is cancelled and, for a lease
/// that expired meanwhile with no extension, `expire`'s effects run.
pub(crate) async fn withdraw_effects(
    tx: &mut Transaction,
    lease: &Lease,
    target: LeaseStatus,
    day: NaiveDate,
) -> Result {
    // PRP.ACL.20: a lease for another tenant on the unit was submitted.
    let blocked = Lease::query()
        .where_(Lease::UNIT_ID.eq(lease.unit_id))
        .where_(Lease::TENANT_PARTY_ID.ne(lease.tenant_party_id))
        .where_(Lease::STATUS.is_in([
            LeaseStatus::PendingSignature,
            LeaseStatus::Active,
            LeaseStatus::RenewalDue,
            LeaseStatus::Expired,
            LeaseStatus::NoticeGiven,
        ]))
        .exists(&mut *tx)
        .await?;
    if blocked {
        return Err(refused(english("property.withdraw.other_tenant", &[])));
    }

    crate::db::set_lease(
        &mut *tx,
        lease.id,
        &[
            ("notice_date", &None::<NaiveDate>),
            ("notice_party", &None::<String>),
            ("notice_reason", &None::<String>),
            ("notice_ground", &None::<String>),
            ("notice_sent_on", &None::<NaiveDate>),
            ("notice_delivered_on", &None::<NaiveDate>),
            ("planned_move_out", &None::<NaiveDate>),
            ("inspection_done_on", &None::<NaiveDate>),
            ("keys_returned_on", &None::<NaiveDate>),
            ("inspection_defects", &false),
        ],
    )
    .await?;
    if target == LeaseStatus::RenewalDue && lease.renewal_flagged_on.is_none() {
        crate::db::set_lease(&mut *tx, lease.id, &[("renewal_flagged_on", &Some(day))]).await?;
    }

    let unit = Unit::find(&mut *tx, lease.unit_id)
        .await?
        .ok_or(Error::NotFound)?;
    let outcome = unit
        .transition(CANCEL_NOTICE)
        .by_system()
        .run_in(&mut *tx)
        .await
        .map_err(TransitionError::into_error)?;
    drop(outcome);

    // The penalty that is not invoiced is cancelled; an invoiced one stays.
    LeaseCharge::query()
        .where_(LeaseCharge::LEASE_ID.eq(lease.id))
        .where_(LeaseCharge::CHARGE_TYPE.eq(EARLY_TERMINATION_FEE))
        .where_(LeaseCharge::STATUS.is_in([ChargeStatus::Planned, ChargeStatus::Scheduled]))
        .update(
            &mut *tx,
            &[
                ("status", &ChargeStatus::Cancelled),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;

    if target == LeaseStatus::Expired && lease.holding_over_until.is_none() {
        expire_effects(&mut *tx, lease, day).await?;
    }
    Ok(())
}

/// Whether the landlord side's written agreement is attached to `lease`.
///
/// # Errors
///
/// The document guard's errors.
pub async fn agreement_attached(state: &AppState, lease: &Lease, day: NaiveDate) -> Result<bool> {
    Ok(guard(state)
        .is_valid(&state.db, &lease.subject(), AGREEMENT_KIND, day)
        .await?
        .is_valid())
}

/// The lease `id`, or a 404.
async fn load(state: &AppState, id: i64) -> std::result::Result<Lease, TransitionError> {
    Lease::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// `user` withdraws the notice on lease `id` (`PRP.ACL.20`), for today.
///
/// # Errors
///
/// `Forbidden` without `leases.manage` in the lease's building, `NotFrom`
/// unless `notice_given`, `Guard` (422) without the written agreement, after
/// the move-out inspection was completed, or once a lease for another tenant
/// on the unit was submitted.
pub async fn withdraw(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Lease, TransitionError> {
    let lease = load(state, id).await?;
    if !user.has_permission_in(permissions::LEASES_MANAGE, &lease.scope()) {
        return Err(TransitionError::Forbidden);
    }
    if lease.status != LeaseStatus::NoticeGiven {
        return Err(TransitionError::NotFrom(lease.status.as_str().to_owned()));
    }
    let day = today();
    let target = target_for(&lease, day);
    if let Some(problem) = problem(&lease, target, day) {
        return Err(TransitionError::Guard(problem));
    }
    if !agreement_attached(state, &lease, day).await? {
        return Err(TransitionError::Guard(english(
            "property.withdraw.agreement_missing",
            &[],
        )));
    }
    let reason = english("property.withdraw.reason", &[]);
    let holding_starts = target == LeaseStatus::Expired && lease.holding_over_until.is_none();
    let _outcome = lease
        .transition(transition_for(target))
        .by(user)
        .reason(reason)
        .input(&WithdrawInput { day: Some(day) })
        .meta(json!({ "holding_starts": holding_starts }))
        .run(state)
        .await?;
    load(state, id).await
}

/// After the commit of `withdraw_notice*` on lease `id`: the move-out
/// inspection ticket is closed, the property managers are told when the lease
/// started to hold over, and `LeaseNoticeWithdrawn` is emitted.
///
/// # Errors
///
/// The database's, ticketing's and notifier's errors.
pub(crate) async fn after_withdraw(
    state: &AppState,
    id: i64,
    to: &str,
    holding_starts: bool,
) -> Result {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    let creator = task_creator(state);
    if let Some(ticket) = creator.find_open(state, &move_out_key(&lease)).await? {
        let note = english("property.withdraw.ticket_note", &[]);
        creator.complete(state, ticket.id, &note).await?;
    }
    if holding_starts {
        announce(state, id).await?;
    }
    state
        .emit(LeaseNoticeWithdrawn {
            lease: lease.id,
            root: lease.root_id,
            tenant: lease.tenant(),
            status: to.to_owned(),
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn lease() -> Lease {
        Lease {
            start_date: day(2027, 1, 1),
            end_date: day(2027, 12, 31),
            ..Default::default()
        }
    }

    #[test]
    fn the_dates_give_the_state_the_lease_goes_back_to() {
        let lease = lease();
        assert_eq!(target_for(&lease, day(2027, 6, 1)), LeaseStatus::Active);
        assert_eq!(target_for(&lease, day(2027, 9, 1)), LeaseStatus::Active);
        assert_eq!(target_for(&lease, day(2027, 9, 2)), LeaseStatus::RenewalDue);
        assert_eq!(target_for(&lease, day(2027, 10, 1)), LeaseStatus::RenewalDue);
        assert_eq!(target_for(&lease, day(2027, 12, 31)), LeaseStatus::RenewalDue);
        assert_eq!(target_for(&lease, day(2028, 1, 1)), LeaseStatus::Expired);
    }

    #[test]
    fn a_longer_contract_period_widens_the_window() {
        let lease = Lease {
            contract_notice_days: Some(120),
            ..lease()
        };
        // max(120, 120 + 30) = 150 days before 2027-12-31 is 2027-08-03.
        assert_eq!(target_for(&lease, day(2027, 8, 3)), LeaseStatus::RenewalDue);
        assert_eq!(target_for(&lease, day(2027, 8, 2)), LeaseStatus::Active);
    }

    #[test]
    fn each_target_has_its_transition() {
        assert_eq!(transition_for(LeaseStatus::Active), WITHDRAW_NOTICE);
        assert_eq!(
            transition_for(LeaseStatus::RenewalDue),
            WITHDRAW_NOTICE_TO_RENEWAL_DUE
        );
        assert_eq!(
            transition_for(LeaseStatus::Expired),
            WITHDRAW_NOTICE_TO_EXPIRED
        );
    }

    #[test]
    fn the_wrong_target_or_a_completed_inspection_is_a_problem() {
        let mut lease = lease();
        let on = day(2027, 6, 1);
        assert_eq!(problem(&lease, LeaseStatus::Active, on), None);
        assert!(problem(&lease, LeaseStatus::RenewalDue, on).is_some());
        lease.inspection_done_on = Some(day(2027, 5, 31));
        assert!(
            problem(&lease, LeaseStatus::Active, on)
                .unwrap()
                .contains("inspection")
        );
    }
}
