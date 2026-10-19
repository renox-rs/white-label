//! The checks and the database effects of `submit` and `activate`
//! (01 Property §7.1 *Lease*, BL-PRP-05).
//!
//! A Renox guard cannot read the database and an effect runs in the
//! transition's transaction, so the checks that need rows run as the first
//! effect of the transition: a refusal rolls the run back with a 422 that
//! says why. What needs the kernel's contracts (the tenant's documents, the
//! signed contract, the deposit) is checked by the service before the run
//! ([`crate::leases`]), because a contract call needs the app state.

use renox::chrono::Datelike;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::approvals::submit::pending_for_action;
use wl_kernel::documents::today;

use crate::charge::{ChargeStatus, LeaseCharge, plan_total};
use crate::db::{refused, set_lease};
use crate::lease::{Lease, LeaseStatus, VersionKind};
use crate::lease_machine::{AMEND, AmendInput, CANCEL_ACTION, RENEW, RenewInput};
use crate::legal_case::has_open;
use crate::texts::english;
use crate::unit::{Unit, UnitStatus};
use crate::unit_machine::OCCUPY;

/// A renewal or an amendment is refused while the lease carries an open legal
/// case flag (BL-PRP-13): "legal case open". A draft stays.
async fn legal_case_check(tx: &mut Transaction, lease: &Lease) -> Result {
    if lease.is_follow_up() && has_open(&mut *tx, lease.root_id).await? {
        return Err(refused(english("property.legal.open", &[])));
    }
    Ok(())
}

/// The version `lease` replaces, as stored now.
async fn previous_in(tx: &mut Transaction, lease: &Lease) -> Result<Lease> {
    let Some(id) = lease.previous_id else {
        return Err(refused(english("property.leases.no_previous", &[])));
    };
    Lease::find(&mut *tx, id)
        .await?
        .ok_or_else(|| refused(english("property.leases.no_previous", &[])))
}

/// Whether the previous version is in a state its `renew` or `amend`
/// accepts, for the follow-up `lease`.
pub(crate) fn previous_accepts(lease: &Lease, previous: &Lease) -> bool {
    match lease.version_kind {
        VersionKind::Renewal => LeaseStatus::RENEWABLE.contains(&previous.status),
        VersionKind::Amendment => LeaseStatus::AMENDABLE.contains(&previous.status),
        VersionKind::New => true,
    }
}

/// The text that says why the previous version cannot be replaced.
fn previous_refusal(lease: &Lease, previous: &Lease) -> Error {
    let key = match lease.version_kind {
        VersionKind::Amendment => "property.leases.previous_not_amendable",
        _ => "property.leases.previous_not_renewable",
    };
    refused(english(
        key,
        &[("status", &previous.status.as_str())],
    ))
}

/// The first effect of `submit`: the unit, the previous version, the
/// amendment's date and the payment plan (§7.1 `submit`).
///
/// Not checked here: the tenant's documents (the document guard, run by
/// [`crate::leases::submit`]) and a blocked tenant (`PTY.ACL.03`, BL-PRP-15:
/// [`crate::leases::hold_refusal`], run by `submit` and again by `activate`).
/// Changed terms against the tenant (`PRP.ACL.17`) need the notifier and the
/// document guard, so [`crate::terms::refusal`] decides before the run and the
/// transition refuses with its message after these checks
/// ([`crate::lease_machine::SubmitInput`]).
pub(crate) async fn submit_checks(tx: &mut Transaction, lease: &Lease) -> Result {
    legal_case_check(&mut *tx, lease).await?;
    if lease.version_kind == VersionKind::New {
        let unit = Unit::find(&mut *tx, lease.unit_id)
            .await?
            .ok_or(Error::NotFound)?;
        // Reserved for this tenant needs reservations (`PRP.LEA.02`, Later).
        if unit.status != UnitStatus::Available {
            return Err(refused(english("property.leases.unit_not_available", &[])));
        }
    } else {
        let previous = previous_in(tx, lease).await?;
        if !previous_accepts(lease, &previous) {
            return Err(previous_refusal(lease, &previous));
        }
        if lease.version_kind == VersionKind::Amendment {
            let Some(effective) = lease.effective_date else {
                return Err(refused(english("property.leases.effective_required", &[])));
            };
            let first_of_month = today().with_day(1).unwrap_or_else(today);
            if effective < first_of_month {
                return Err(refused(english("property.leases.effective_past", &[])));
            }
            if effective < previous.start_date || effective > previous.end_date {
                return Err(refused(english("property.leases.effective_outside", &[])));
            }
        }
    }
    let charges = LeaseCharge::query()
        .where_(LeaseCharge::LEASE_ID.eq(lease.id))
        .get(&mut *tx)
        .await?;
    let planned = plan_total(&charges);
    if planned != lease.annual_rent {
        return Err(refused(english(
            "property.leases.plan_mismatch",
            &[
                ("plan", &crate::money_text(planned)),
                ("rent", &crate::money_text(lease.annual_rent)),
            ],
        )));
    }
    Ok(())
}

/// The first effect of `activate`: nothing waits for an approval, the unit is
/// free for a new lease, and the previous version of a renewal or amendment
/// still accepts `renew` or `amend` (checked again, in the transaction that
/// ends it).
pub(crate) async fn activate_checks(tx: &mut Transaction, lease: &Lease) -> Result {
    legal_case_check(&mut *tx, lease).await?;
    if pending_for_action(&mut *tx, &lease.subject(), CANCEL_ACTION)
        .await?
        .is_some()
    {
        return Err(refused(english("property.leases.cancel_pending", &[])));
    }
    if lease.version_kind == VersionKind::New {
        let unit = Unit::find(&mut *tx, lease.unit_id)
            .await?
            .ok_or(Error::NotFound)?;
        if !matches!(unit.status, UnitStatus::Available | UnitStatus::Reserved) {
            return Err(refused(english("property.leases.unit_not_available", &[])));
        }
    } else {
        let previous = previous_in(tx, lease).await?;
        if !previous_accepts(lease, &previous) {
            return Err(previous_refusal(lease, &previous));
        }
    }
    Ok(())
}

/// The effects of `activate` that touch the database, in the transaction of
/// the transition: the day, the charge schedule, and the unit (new lease:
/// `occupy`) or the previous version (renewal: `renew`; amendment: `amend`).
/// A renewal or an amendment never changes the unit's state (§7.1 *Unit*).
///
/// The effects of `renew` on the previous version's charges are in
/// [`crate::renew`] (BL-PRP-07); those of `amend` in [`crate::amend`]
/// (BL-PRP-08).
pub(crate) async fn apply_activation(tx: &mut Transaction, lease: &Lease) -> Result {
    set_lease(&mut *tx, lease.id, &[("activated_on", &Some(today()))]).await?;
    LeaseCharge::query()
        .where_(LeaseCharge::LEASE_ID.eq(lease.id))
        .where_(LeaseCharge::STATUS.eq(ChargeStatus::Planned))
        .update(
            &mut *tx,
            &[
                ("status", &ChargeStatus::Scheduled),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    match lease.version_kind {
        VersionKind::New => {
            let unit = Unit::find(&mut *tx, lease.unit_id)
                .await?
                .ok_or(Error::NotFound)?;
            let outcome = unit
                .transition(OCCUPY)
                .by_system()
                .run_in(&mut *tx)
                .await
                .map_err(TransitionError::into_error)?;
            drop(outcome);
        }
        VersionKind::Renewal | VersionKind::Amendment => {
            let previous = previous_in(tx, lease).await?;
            let name = if lease.version_kind == VersionKind::Renewal {
                RENEW
            } else {
                AMEND
            };
            let mut run = previous.transition(name).by_system();
            run = if lease.version_kind == VersionKind::Renewal {
                run.input(&RenewInput {
                    next: Some(lease.id),
                })
            } else {
                run.input(&AmendInput {
                    next: Some(lease.id),
                })
            };
            let outcome = run
                .run_in(&mut *tx)
                .await
                .map_err(TransitionError::into_error)?;
            drop(outcome);
        }
    }
    Ok(())
}
