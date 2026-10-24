//! Writing off unbilled time, or work in progress (04 §4.5, `ACC.ACL.09`;
//! BL-ACC-06).
//!
//! The unbilled time of an engagement lives in the module that records time
//! (`wl-attendance`, `HRM.TMS.01`; it exists but does not yet implement the
//! seam), reached through the kernel's
//! [`UnbilledTime`](wl_kernel::wip::UnbilledTime) seam; without a provider
//! the WIP is zero. A manager writes off up to a share of the engagement's
//! WIP ([`SHARE_BPS`], 10%); above it the write-off waits for a partner:
//! the same transition, deferred by the kernel's approval interceptor
//! ([`write_off_policy`]) and run again as [`WRITE_OFF_APPROVED`] on the
//! partner's yes. A partner's own write-off needs no approval. After the
//! write-off commits, the seam writes the time off ([`crate::events`]).
//!
//! Decision: the share is of the WIP when the write-off is asked, in basis
//! points rounded up, so 10.01% counts as above 10%; a write-off of exactly
//! 10% does not need a partner.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{
    Machine, Outcome, StateMachine, Transition, TransitionCtx, TransitionError,
};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, PolicyDefault, submit::REQUEST_KEY};
use wl_kernel::documents::today;
use wl_kernel::machines::SubjectMachine;
use wl_kernel::wip::unbilled_time;

use crate::db::refused;
use crate::engagement::Engagement;
use crate::permissions::{self, PARTNER_ROLE};
use crate::texts::english;

/// The subject type of a write-off.
pub const WIP_WRITE_OFF_SUBJECT_TYPE: &str = "accounting.wip_write_off";
/// The share of the WIP a manager writes off without a partner, in basis
/// points (`ACC.ACL.09`: 10%).
pub const SHARE_BPS: i64 = 1000;
/// The `write_off` transition's name.
pub const WRITE_OFF: &str = "write_off";
/// The write-off the kernel runs once the partner approved it.
pub const WRITE_OFF_APPROVED: &str = "write_off_approved";
/// The `discard` transition's name.
pub const DISCARD: &str = "discard";
/// The approval action of a write-off (`accounting.wip_write_off`).
pub const WRITE_OFF_ACTION: &str = "accounting.wip_write_off";

/// Where a write-off stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WriteOffStatus {
    /// Asked; waiting for a partner above the share.
    #[default]
    Proposed,
    /// The time is written off (terminal).
    WrittenOff,
    /// Dropped without writing anything off (terminal).
    Discarded,
}

/// A write-off of unbilled time of an engagement.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "accounting_wip_write_offs",
    state_machine,
    index(engagement_id)
)]
pub struct WipWriteOff {
    /// The primary key.
    pub id: i64,
    /// The engagement.
    #[model(references = "accounting_engagements")]
    pub engagement_id: i64,
    /// The amount written off, in fils.
    pub amount: i64,
    /// The engagement's unbilled time when it was asked, in fils.
    pub wip_amount: i64,
    /// Why.
    pub reason: String,
    /// Who asked.
    pub requested_by: Option<i64>,
    /// The day the time was written off.
    pub written_off_on: Option<NaiveDate>,
    /// The lifecycle state; changed only by a transition.
    pub status: WriteOffStatus,
    /// When the row was created.
    pub created_at: Option<renox::db::DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<renox::db::DateTime>,
}

impl WipWriteOff {
    /// The write-off as a kernel subject: `accounting.wip_write_off:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(WIP_WRITE_OFF_SUBJECT_TYPE, self.id)
    }

    /// The write-offs of engagement `id`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_engagement(db: &Db, id: i64) -> Result<Vec<WipWriteOff>> {
        WipWriteOff::query()
            .where_(WipWriteOff::ENGAGEMENT_ID.eq(id))
            .order_by(WipWriteOff::ID)
            .get(db)
            .await
    }
}

/// What a run carries to the approval policy: the share of the WIP, in basis
/// points (0 for a partner's own write-off, which needs no approval).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WipInput {
    /// The share of the WIP, in basis points, rounded up.
    #[serde(default)]
    pub percentage_bps: i64,
    /// The business day of the run (Asia/Dubai); today when empty.
    #[serde(default)]
    pub on: Option<NaiveDate>,
}

fn input_of(ctx: &TransitionCtx) -> WipInput {
    ctx.input_as::<WipInput>().unwrap_or_default()
}

/// The share `amount` is of `wip`, in basis points rounded up; 10,000 when
/// there is no WIP.
pub fn share_bps(amount: i64, wip: i64) -> i64 {
    if wip <= 0 {
        return 10_000;
    }
    (amount.saturating_mul(10_000) + wip - 1) / wip
}

/// The effect of a write-off: the day it was written off.
fn written_off(transition: Transition<WipWriteOff>) -> Transition<WipWriteOff> {
    transition
        .from([WriteOffStatus::Proposed])
        .to(WriteOffStatus::WrittenOff)
        .effect(|w: &WipWriteOff, ctx, tx| {
            let id = w.id;
            let on = input_of(ctx).on.unwrap_or_else(today);
            Box::pin(async move {
                WipWriteOff::query()
                    .where_(WipWriteOff::ID.eq(id))
                    .update(&mut *tx, &[("written_off_on", &Some(on))])
                    .await?;
                Ok(())
            })
        })
}

impl StateMachine for WipWriteOff {
    type State = WriteOffStatus;
    const COLUMN: renox::db::Col<WipWriteOff, WriteOffStatus> = WipWriteOff::STATUS;

    fn machine() -> Machine<Self> {
        Machine::new()
            .initial([WriteOffStatus::Proposed])
            .state(WriteOffStatus::Proposed)
            .tone("warning")
            .state(WriteOffStatus::WrittenOff)
            .tone("success")
            .terminal()
            .state(WriteOffStatus::Discarded)
            .tone("neutral")
            .terminal()
            .transition(written_off(
                Transition::new(WRITE_OFF)
                    .label("Write off")
                    .tone("danger")
                    .permission(permissions::WIP_WRITE_OFF),
            ))
            .transition(written_off(
                Transition::new(WRITE_OFF_APPROVED)
                    .label("Write off")
                    .system(),
            ))
            .transition(
                Transition::new(DISCARD)
                    .label("Discard")
                    .permission(permissions::WIP_WRITE_OFF)
                    .from([WriteOffStatus::Proposed])
                    .to(WriteOffStatus::Discarded),
            )
    }
}

/// The default approval policy of a write-off: only above [`SHARE_BPS`] of
/// the WIP, one step by a partner (the partner role, or `engagements.end`),
/// never the manager who asked.
pub fn write_off_policy() -> PolicyDefault {
    PolicyDefault::new(WRITE_OFF_ACTION)
        .percentage_above(SHARE_BPS)
        .step_or_permission([PARTNER_ROLE], permissions::ENGAGEMENTS_END)
}

/// Registers the machine and the policy with the kernel.
pub(crate) fn register() {
    SubjectMachine::of::<WipWriteOff>(WIP_WRITE_OFF_SUBJECT_TYPE)
        .action(WRITE_OFF, WRITE_OFF_ACTION)
        .register();
    renox::state_machine::register::<WipWriteOff>();
    write_off_policy().register();
    approvals::register_subject_link(WIP_WRITE_OFF_SUBJECT_TYPE, |id| {
        format!("/accounting/wip-write-offs/{id}")
    });
}

/// What a write-off came to.
#[derive(Debug, Clone)]
pub struct WrittenOff {
    /// The write-off after the commit: `written_off`, or still `proposed`
    /// while a partner decides.
    pub write_off: WipWriteOff,
    /// The approval request to a partner, when the share is above the limit
    /// and `by` is not a partner.
    pub approval_request: Option<i64>,
}

/// The unbilled time of `engagement` now, in fils.
///
/// # Errors
///
/// The time module's errors.
pub async fn unbilled(state: &AppState, engagement: &Engagement) -> Result<i64> {
    Ok(unbilled_time(state)
        .summary(state, &engagement.subject())
        .await?
        .unbilled)
}

/// `by` writes `amount` fils of the unbilled time of engagement `id` off
/// with `reason` (`ACC.ACL.09`): at once up to 10% of the WIP (or for a
/// partner), above it as an approval request to a partner. After the commit
/// the time module writes the time off ([`crate::events`]).
///
/// # Errors
///
/// `Forbidden` without `wip.write_off`; a 422 for a blank reason, an amount
/// of 0 or less, one above the WIP, or an engagement that does not accept
/// time entries.
pub async fn write_off(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    amount: i64,
    reason: &str,
) -> Result<WrittenOff> {
    if !by.has_permission(permissions::WIP_WRITE_OFF) {
        return Err(Error::Forbidden);
    }
    let engagement = Engagement::find_or_404(&state.db, id).await?;
    if !engagement.accepts_time_entries() {
        return Err(refused(english("accounting.wip.problem.engagement", &[])));
    }
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(refused(english("accounting.problems.reason_required", &[])));
    }
    let wip = unbilled(state, &engagement).await?;
    if amount <= 0 || amount > wip {
        return Err(refused(english(
            "accounting.wip.problem.amount",
            &[("wip", &crate::money_text(wip))],
        )));
    }
    let partner = by.has_permission(permissions::ENGAGEMENTS_END);
    let input = WipInput {
        percentage_bps: if partner { 0 } else { share_bps(amount, wip) },
        on: Some(today()),
    };
    let row = WipWriteOff::create(
        &state.db,
        WipWriteOff {
            engagement_id: id,
            amount,
            wip_amount: wip,
            reason: reason.to_owned(),
            requested_by: Some(by.id),
            ..WipWriteOff::default()
        },
    )
    .await?;
    let mut tx = state.db.begin().await?;
    let outcome = row
        .transition(WRITE_OFF)
        .by(by)
        .input(&input)
        .run_in(&mut tx)
        .await
        .map_err(TransitionError::into_error)?;
    tx.commit().await?;
    approvals::announce(state, &outcome).await?;
    let approval_request = match &outcome {
        Outcome::Pending(pending) => pending.data.get(REQUEST_KEY).and_then(Value::as_i64),
        Outcome::Done(_) => None,
    };
    outcome.emit(state).await?;
    Ok(WrittenOff {
        write_off: WipWriteOff::find_or_404(&state.db, row.id).await?,
        approval_request,
    })
}

/// After the write-off `id` committed (`write_off` or `write_off_approved`):
/// the time module writes the time off.
///
/// # Errors
///
/// The time module's and the database's errors.
pub async fn after(state: &AppState, id: i64) -> Result {
    let row = WipWriteOff::find_or_404(&state.db, id).await?;
    let engagement = Engagement::find_or_404(&state.db, row.engagement_id).await?;
    unbilled_time(state)
        .write_off(
            state,
            &engagement.subject(),
            row.amount,
            &row.reason,
            row.requested_by,
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_share_is_rounded_up_in_basis_points() {
        assert_eq!(share_bps(150_000, 1_000_000), 1500);
        assert_eq!(share_bps(80_000, 1_000_000), 800);
        assert_eq!(share_bps(100_000, 1_000_000), 1000);
        assert_eq!(share_bps(100_001, 1_000_000), 1001);
        assert_eq!(share_bps(5, 0), 10_000);
    }

    #[test]
    fn the_machine_has_its_states_and_transitions() {
        let machine = WipWriteOff::machine();
        let names: Vec<_> = machine
            .transitions()
            .iter()
            .map(|t| (t.name(), t.is_system()))
            .collect();
        assert_eq!(
            names,
            [
                (WRITE_OFF, false),
                (WRITE_OFF_APPROVED, true),
                (DISCARD, false)
            ]
        );
        assert_eq!(
            wl_kernel::machines::approved(WRITE_OFF),
            WRITE_OFF_APPROVED,
            "the kernel runs it on the partner's yes"
        );
    }
}
