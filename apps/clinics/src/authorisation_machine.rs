//! The prior authorisation machine (05 §7.1 *Prior authorisation*;
//! `CLN.INS.03`; base for BL-CLN-02 and BL-CLN-03).
//!
//! | Transition | From | To | By | Guards |
//! |---|---|---|---|---|
//! | [`REQUEST`] | draft | requested | `insurance.manage` | services and units named |
//! | [`APPROVE`] | requested | approved | `insurance.manage` | approval reference and validity recorded |
//! | [`APPROVE_PARTIALLY`] | requested | partially_approved | `insurance.manage` | fewer units than asked, reference and validity recorded |
//! | [`DENY`] | requested | denied | `insurance.manage` | reason recorded |
//! | [`CONSUME`] / [`CONSUME_PARTIAL`] | approved / partially_approved | the same | system (charge capture) | units remaining, some left afterwards |
//! | [`CONSUME_LAST`] | approved, partially_approved | used | system (charge capture) | the units left are what is used |
//! | [`EXPIRE`] | approved, partially_approved | expired | system | the validity date has passed |
//! | [`CANCEL`] | draft, requested | cancelled | `insurance.manage` | — |
//!
//! Decisions:
//!
//! - The spec's `consume` ends in `approved`, `partially_approved` or `used`;
//!   a transition has one target, so it is three transitions with guards that
//!   exclude one another (as `wl-sales` does for its packages).
//! - The reasons and the payer's reference are carried by the run's input
//!   ([`AuthInput`]), as the guards and effects cannot read the services'
//!   arguments otherwise.
//! - "Supporting documents attached" (the `request` guard) needs the document
//!   upload of the authorisation, which is not built; the guard checks the
//!   services and the units.
//! - Effects write only the columns of the record: the listeners
//!   ([`crate::authorisations`]) react to the committed transition (patient
//!   informed on `deny`, the look-ahead flags of `CLN.RUL.09`, appointments
//!   unflagged on `approve`).

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use crate::authorisation::{AuthStatus, PriorAuthorisation};
use crate::permissions;
use crate::texts::english;

/// `draft → requested`: sent to the payer.
pub const REQUEST: &str = "request";
/// `requested → approved`.
pub const APPROVE: &str = "approve";
/// `requested → partially_approved`.
pub const APPROVE_PARTIALLY: &str = "approve_partially";
/// `requested → denied`.
pub const DENY: &str = "deny";
/// `approved → approved`: units consumed, some left.
pub const CONSUME: &str = "consume";
/// `partially_approved → partially_approved`: units consumed, some left.
pub const CONSUME_PARTIAL: &str = "consume_partial";
/// `approved | partially_approved → used`: the last units consumed.
pub const CONSUME_LAST: &str = "consume_last";
/// `approved | partially_approved → expired`: the validity date passed.
pub const EXPIRE: &str = "expire";
/// `draft | requested → cancelled`: no longer needed.
pub const CANCEL: &str = "cancel";

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthInput {
    /// The payer's reference (`request`, `approve`, `approve_partially`).
    #[serde(default)]
    pub reference: Option<String>,
    /// The last day it is valid (`approve`, `approve_partially`).
    #[serde(default)]
    pub valid_until: Option<NaiveDate>,
    /// The units approved (`approve_partially`) or consumed (`consume*`).
    #[serde(default)]
    pub units: Option<i64>,
    /// The reason the payer refused it (`deny`).
    #[serde(default)]
    pub reason: Option<String>,
    /// The day of the run (Asia/Dubai).
    #[serde(default)]
    pub on: Option<NaiveDate>,
    /// The user who recorded the answer.
    #[serde(default)]
    pub by: Option<i64>,
}

fn input_of(ctx: &TransitionCtx) -> AuthInput {
    ctx.input_as::<AuthInput>().unwrap_or_default()
}

/// `Ok` when `ok`, else the English text `key`.
fn check(ok: bool, key: &str) -> std::result::Result<(), String> {
    if ok { Ok(()) } else { Err(english(key, &[])) }
}

/// The text of `value`, trimmed; `None` when blank.
fn clean(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// Sets the columns of authorisation `id` other than the status.
async fn set(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    PriorAuthorisation::query()
        .where_(PriorAuthorisation::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

impl StateMachine for PriorAuthorisation {
    type State = AuthStatus;
    const COLUMN: renox::db::Col<PriorAuthorisation, AuthStatus> = PriorAuthorisation::STATUS;

    fn machine() -> Machine<Self> {
        use AuthStatus::{
            Approved, Cancelled, Denied, Draft, Expired, PartiallyApproved, Requested, Used,
        };
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Requested)
            .tone("info")
            .state(Approved)
            .tone("success")
            .state(PartiallyApproved)
            .tone("warning")
            .state(Denied)
            .tone("error")
            .terminal()
            .state(Used)
            .tone("success")
            .terminal()
            .state(Expired)
            .tone("neutral")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(request())
            .transition(approve())
            .transition(approve_partially())
            .transition(deny())
            .transition(consume(CONSUME, Approved, Approved))
            .transition(consume(
                CONSUME_PARTIAL,
                PartiallyApproved,
                PartiallyApproved,
            ))
            .transition(consume_last())
            .transition(expire())
            .transition(cancel())
    }
}

/// `request`: the services and the units are named; the payer's reference,
/// if it gave one with the request, is recorded.
fn request() -> Transition<PriorAuthorisation> {
    Transition::new(REQUEST)
        .label("Request")
        .tone("primary")
        .permission(permissions::AUTHORISATIONS_MANAGE)
        .from([AuthStatus::Draft])
        .to(AuthStatus::Requested)
        .guard("services_named", |a: &PriorAuthorisation, _ctx| {
            let ok = !a.services.0.is_empty() && a.units_requested >= 1;
            Box::pin(async move { check(ok, "clinics.authorisation.problem.services") })
        })
        .effect(|a: &PriorAuthorisation, ctx, tx| {
            let id = a.id;
            let input = input_of(ctx);
            let reference = clean(&input.reference).or_else(|| a.reference.clone());
            Box::pin(async move {
                set(
                    tx,
                    id,
                    &[("reference", &reference), ("requested_on", &input.on)],
                )
                .await
            })
        })
}

/// The reference the answer is recorded with: the one given, else the one
/// recorded at the request.
fn reference_of(a: &PriorAuthorisation, input: &AuthInput) -> Option<String> {
    clean(&input.reference).or_else(|| a.reference.clone())
}

/// `approve`: approved for the units asked for.
fn approve() -> Transition<PriorAuthorisation> {
    Transition::new(APPROVE)
        .label("Approve")
        .tone("success")
        .permission(permissions::AUTHORISATIONS_MANAGE)
        .from([AuthStatus::Requested])
        .to(AuthStatus::Approved)
        .guard("reference_and_validity", |a: &PriorAuthorisation, ctx| {
            let input = input_of(ctx);
            let ok = reference_of(a, &input).is_some()
                && input
                    .valid_until
                    .zip(input.on)
                    .is_some_and(|(until, on)| until >= on);
            Box::pin(async move { check(ok, "clinics.authorisation.problem.approval") })
        })
        .effect(|a: &PriorAuthorisation, ctx, tx| {
            let id = a.id;
            let units = a.units_requested;
            let input = input_of(ctx);
            let reference = reference_of(a, &input);
            Box::pin(async move {
                set(
                    tx,
                    id,
                    &[
                        ("units_approved", &units),
                        ("reference", &reference),
                        ("valid_until", &input.valid_until),
                        ("decided_on", &input.on),
                        ("decided_by", &input.by),
                    ],
                )
                .await
            })
        })
}

/// `approve_partially`: approved for fewer units than asked for.
fn approve_partially() -> Transition<PriorAuthorisation> {
    Transition::new(APPROVE_PARTIALLY)
        .label("Approve partially")
        .tone("warning")
        .permission(permissions::AUTHORISATIONS_MANAGE)
        .from([AuthStatus::Requested])
        .to(AuthStatus::PartiallyApproved)
        .guard("units_recorded", |a: &PriorAuthorisation, ctx| {
            let asked = a.units_requested;
            let ok = input_of(ctx)
                .units
                .is_some_and(|units| units >= 1 && units < asked);
            Box::pin(async move { check(ok, "clinics.authorisation.problem.units") })
        })
        .guard("reference_and_validity", |a: &PriorAuthorisation, ctx| {
            let input = input_of(ctx);
            let ok = reference_of(a, &input).is_some()
                && input
                    .valid_until
                    .zip(input.on)
                    .is_some_and(|(until, on)| until >= on);
            Box::pin(async move { check(ok, "clinics.authorisation.problem.approval") })
        })
        .effect(|a: &PriorAuthorisation, ctx, tx| {
            let id = a.id;
            let input = input_of(ctx);
            let reference = reference_of(a, &input);
            Box::pin(async move {
                set(
                    tx,
                    id,
                    &[
                        ("units_approved", &input.units.unwrap_or_default()),
                        ("reference", &reference),
                        ("valid_until", &input.valid_until),
                        ("decided_on", &input.on),
                        ("decided_by", &input.by),
                    ],
                )
                .await
            })
        })
}

/// `deny`: the payer refused; the reason is recorded.
fn deny() -> Transition<PriorAuthorisation> {
    Transition::new(DENY)
        .label("Deny")
        .tone("danger")
        .permission(permissions::AUTHORISATIONS_MANAGE)
        .from([AuthStatus::Requested])
        .to(AuthStatus::Denied)
        .guard("reason_recorded", |_a: &PriorAuthorisation, ctx| {
            let ok = clean(&input_of(ctx).reason).is_some();
            Box::pin(async move { check(ok, "clinics.authorisation.problem.reason") })
        })
        .effect(|a: &PriorAuthorisation, ctx, tx| {
            let id = a.id;
            let input = input_of(ctx);
            let reason = clean(&input.reason);
            Box::pin(async move {
                set(
                    tx,
                    id,
                    &[
                        ("denial_reason", &reason),
                        ("decided_on", &input.on),
                        ("decided_by", &input.by),
                    ],
                )
                .await
            })
        })
}

/// Adds the units of the run to the units consumed of authorisation `id`, in
/// one statement, so two consumptions cannot lose one another; fails when
/// they would take more than were approved.
async fn take_units(tx: &mut renox::db::Transaction, id: i64, units: i64) -> Result {
    let changed = renox::db::sql(format!(
        "UPDATE {} SET units_consumed = units_consumed + ?, updated_at = ? \
         WHERE id = ? AND units_consumed + ? <= units_approved",
        PriorAuthorisation::TABLE
    ))
    .bind(units)
    .bind(now())
    .bind(id)
    .bind(units)
    .execute(&mut *tx)
    .await?;
    if changed == 0 {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            english("clinics.authorisation.problem.units_left", &[]),
        ));
    }
    Ok(())
}

/// The guard of `consume*`: `units` are between 1 and what is left, and
/// `last` says whether they are all that is left.
fn units_guard(
    a: &PriorAuthorisation,
    ctx: &TransitionCtx,
    last: bool,
) -> std::result::Result<(), String> {
    let left = a.units_left();
    match input_of(ctx).units {
        Some(units) if units >= 1 && units <= left && (units == left) == last => Ok(()),
        Some(units) if units >= 1 && units <= left && last => {
            Err(english("clinics.authorisation.problem.not_last", &[]))
        }
        Some(units) if units >= 1 && units <= left => {
            Err(english("clinics.authorisation.problem.is_last", &[]))
        }
        _ => Err(english("clinics.authorisation.problem.units_left", &[])),
    }
}

/// `consume` and `consume_partial`: units used, some stay.
fn consume(name: &str, from: AuthStatus, to: AuthStatus) -> Transition<PriorAuthorisation> {
    Transition::new(name)
        .label("Consume")
        .tone("info")
        .system()
        .from([from])
        .to(to)
        .guard("units_remaining", |a: &PriorAuthorisation, ctx| {
            let answer = units_guard(a, ctx, false);
            Box::pin(async move { answer })
        })
        .effect(|a: &PriorAuthorisation, ctx, tx| {
            let id = a.id;
            let units = input_of(ctx).units.unwrap_or_default();
            Box::pin(async move { take_units(tx, id, units).await })
        })
}

/// `consume_last`: the units left are used; the authorisation is `used`.
fn consume_last() -> Transition<PriorAuthorisation> {
    Transition::new(CONSUME_LAST)
        .label("Consume the last units")
        .tone("info")
        .system()
        .from([AuthStatus::Approved, AuthStatus::PartiallyApproved])
        .to(AuthStatus::Used)
        .guard("units_remaining", |a: &PriorAuthorisation, ctx| {
            let answer = units_guard(a, ctx, true);
            Box::pin(async move { answer })
        })
        .effect(|a: &PriorAuthorisation, ctx, tx| {
            let id = a.id;
            let units = input_of(ctx).units.unwrap_or_default();
            Box::pin(async move { take_units(tx, id, units).await })
        })
}

/// `expire`: the validity date has passed.
fn expire() -> Transition<PriorAuthorisation> {
    Transition::new(EXPIRE)
        .label("Expire")
        .tone("neutral")
        .system()
        .from([AuthStatus::Approved, AuthStatus::PartiallyApproved])
        .to(AuthStatus::Expired)
        .guard("validity_passed", |a: &PriorAuthorisation, ctx| {
            let passed = a
                .valid_until
                .zip(input_of(ctx).on)
                .is_some_and(|(until, on)| until < on);
            Box::pin(async move { check(passed, "clinics.authorisation.problem.not_passed") })
        })
}

/// `cancel`: no longer needed.
fn cancel() -> Transition<PriorAuthorisation> {
    Transition::new(CANCEL)
        .label("Cancel")
        .tone("neutral")
        .permission(permissions::AUTHORISATIONS_MANAGE)
        .from([AuthStatus::Draft, AuthStatus::Requested])
        .to(AuthStatus::Cancelled)
}

/// Registers the machine for `rnx state:diagram`.
pub(crate) fn register() {
    renox::state_machine::register::<PriorAuthorisation>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_every_state_and_transition() {
        let machine = PriorAuthorisation::machine();
        let states: Vec<(&str, Option<&str>, bool)> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("draft", Some("neutral"), false),
                ("requested", Some("info"), false),
                ("approved", Some("success"), false),
                ("partially_approved", Some("warning"), false),
                ("denied", Some("error"), true),
                ("used", Some("success"), true),
                ("expired", Some("neutral"), true),
                ("cancelled", Some("neutral"), true),
            ]
        );
        let expected = [
            (REQUEST, vec!["draft"], "requested", false),
            (APPROVE, vec!["requested"], "approved", false),
            (
                APPROVE_PARTIALLY,
                vec!["requested"],
                "partially_approved",
                false,
            ),
            (DENY, vec!["requested"], "denied", false),
            (CONSUME, vec!["approved"], "approved", true),
            (
                CONSUME_PARTIAL,
                vec!["partially_approved"],
                "partially_approved",
                true,
            ),
            (
                CONSUME_LAST,
                vec!["approved", "partially_approved"],
                "used",
                true,
            ),
            (
                EXPIRE,
                vec!["approved", "partially_approved"],
                "expired",
                true,
            ),
            (CANCEL, vec!["draft", "requested"], "cancelled", false),
        ];
        assert_eq!(machine.transitions().len(), expected.len());
        for (name, from, to, system) in expected {
            let transition = machine.find(name).unwrap();
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.is_system(), system, "{name}");
        }
    }
}
