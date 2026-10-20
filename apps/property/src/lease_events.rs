//! What follows the commit of a lease transition (01 Property §7.1).
//!
//! - **`activate`:** `LeaseActivated` is emitted, the accountant gets one task
//!   to relink the cheques of the charges a renewal cancelled
//!   ([`crate::renew::after_renew`]), the Ejari task is opened for
//!   the property manager (`PRP.RUL.02`: the registration is chased 14 days
//!   after activation), and a renewal or amendment whose deposit differs from
//!   the held amount asks the deposit to `adjust` (system). An amendment's
//!   invoices, cheques and `LeaseAmended` follow ([`crate::amend::settle`]).
//! - **`flag_renewal`:** the property managers and the tenant are told that
//!   the lease entered its renewal window, with the notice deadline
//!   (`PRP.RUL.01`).
//! - **`expire`:** the property managers are told that the lease is holding
//!   over (`PRP.RUL.03`).
//! - **`submit`:** a renewal version closes the holding-over task of the
//!   version it renews (`PRP.RUL.14`).
//! - **`withdraw_notice*`:** the move-out inspection ticket is closed, the
//!   property managers are told when the lease started to hold over, and
//!   `LeaseNoticeWithdrawn` is emitted.
//! - **`terminate`:** the credit notes, the cheques and the deposit settlement
//!   ([`crate::termination::settle`]) and `LeaseTerminated`.
//! - **`force_terminate`:** as `terminate`, after the cheques the requester
//!   chose to keep are held as evidence and before the eviction flags are
//!   closed and `LeaseForceTerminated` is emitted ([`crate::force`]).
//! - **`record_notice`:** the move-out inspection ticket is created
//!   (`PRP.RUL.04`); the proof of delivery of a landlord's notice computes the
//!   planned move-out date.

use renox::chrono::Duration;
use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::deposits::{AdjustDeposit, deposits};
use wl_kernel::notify::StatutoryNoticeProofRecorded;
use wl_kernel::role_tasks::RoleTask;

use crate::events::LeaseActivated;
use crate::lease::{Lease, VersionKind};
use crate::lease_machine::{
    ACTIVATE, EXPIRE, FLAG_RENEWAL, FORCE_TERMINATE, FORCE_TERMINATE_APPROVED, RECORD_NOTICE,
    SUBMIT, TERMINATE, WITHDRAW_NOTICE, WITHDRAW_NOTICE_TO_EXPIRED, WITHDRAW_NOTICE_TO_RENEWAL_DUE,
};
use crate::leases::ejari_task_key;
use crate::permissions::{ACCOUNTANT_ROLE, PROPERTY_MANAGER_ROLE};
use crate::texts::english;

/// The days after activation the Ejari registration is due (`PRP.RUL.02`).
pub const EJARI_DAYS: i64 = 14;

/// Registers the listener.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Lease>, state: AppState| async move {
        match event.transition.as_str() {
            ACTIVATE => after_activate(&state, event.key, event.actor_id).await,
            FLAG_RENEWAL => after_flag_renewal(&state, event.key).await,
            EXPIRE => crate::holding::announce(&state, event.key).await,
            SUBMIT => crate::holding::renewal_submitted(&state, event.key).await,
            RECORD_NOTICE => crate::notice::after_record(&state, event.key).await,
            TERMINATE => {
                crate::termination::after_terminate(&state, event.key, event.actor_id).await
            }
            FORCE_TERMINATE | FORCE_TERMINATE_APPROVED => {
                crate::force::after_force(&state, &event).await
            }
            WITHDRAW_NOTICE | WITHDRAW_NOTICE_TO_RENEWAL_DUE | WITHDRAW_NOTICE_TO_EXPIRED => {
                let holding_starts = event
                    .meta
                    .get("holding_starts")
                    .and_then(renox::serde_json::Value::as_bool)
                    .unwrap_or(false);
                crate::withdraw::after_withdraw(&state, event.key, &event.to, holding_starts).await
            }
            _ => Ok(()),
        }
    });
    // The proof of delivery of a landlord's notice starts what depends on it.
    app.listen(
        |event: StatutoryNoticeProofRecorded, state: AppState| async move {
            crate::notice::on_proof(&state, &event).await
        },
    );
}

/// The effects of `flag_renewal` that need the app state: the notices.
async fn after_flag_renewal(state: &AppState, id: i64) -> Result {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    crate::notices::renewal_due(state, &lease, crate::renewal::notice_deadline(&lease)).await
}

/// The title of the Ejari task of `lease`, by the kind of version.
fn ejari_title(lease: &Lease) -> String {
    let key = match lease.version_kind {
        VersionKind::New => "property.tasks.ejari_new",
        VersionKind::Renewal => "property.tasks.ejari_renewal",
        VersionKind::Amendment => "property.tasks.ejari_amendment",
    };
    english(key, &[("lease", &lease.number_with_version())])
}

/// The effects of `activate` that need the app state.
async fn after_activate(state: &AppState, id: i64, by: Option<i64>) -> Result {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    state
        .emit(LeaseActivated {
            lease: lease.id,
            root: lease.root_id,
            version: lease.version,
            kind: lease.version_kind,
            unit: lease.unit_id,
            building: lease.building_id,
            tenant: lease.tenant(),
            start_date: lease.start_date,
            end_date: lease.end_date,
            annual_rent: lease.annual_rent,
        })
        .await?;
    crate::renew::after_renew(state, &lease).await?;
    let task = RoleTask::new(
        lease.subject(),
        PROPERTY_MANAGER_ROLE,
        &ejari_title(&lease),
        &ejari_task_key(&lease),
    )
    .description(&english("property.tasks.ejari_description", &[]))
    .due_at(renox::db::now() + Duration::days(EJARI_DAYS));
    crate::tasks::open(state, &lease.scope(), task).await?;
    adjust_deposit(state, &lease).await?;
    crate::amend::after_amend(state, &lease, by).await
}

/// A renewal or amendment with another deposit than the held one asks the
/// deposit to `adjust`; when it cannot (an earlier adjustment is open, the
/// new amount is 0), the accountant gets a task to do it by hand.
async fn adjust_deposit(state: &AppState, lease: &Lease) -> Result {
    if !lease.is_follow_up() {
        return Ok(());
    }
    let Some(previous) = lease.previous(&state.db).await? else {
        return Ok(());
    };
    if previous.deposit_amount == lease.deposit_amount {
        return Ok(());
    }
    let request = AdjustDeposit {
        source: lease.agreement(),
        new_amount: lease.deposit_amount,
        source_version: lease.version,
        effective_on: lease.effective_date.unwrap_or(lease.start_date),
    };
    let done = if lease.deposit_amount > 0 {
        deposits(state).adjust(state, request, None).await
    } else {
        Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            english("property.leases.deposit_zero", &[]),
        ))
    };
    if let Err(error) = done {
        eprintln!(
            "property: deposit of {} not adjusted: {error:?}",
            lease.number()
        );
        let task = RoleTask::new(
            lease.subject(),
            ACCOUNTANT_ROLE,
            &english(
                "property.tasks.deposit_adjust",
                &[
                    ("lease", &lease.number_with_version()),
                    ("amount", &crate::money_text(lease.deposit_amount)),
                ],
            ),
            &format!("property.deposit_adjust:{}", lease.id),
        );
        crate::tasks::open(state, &lease.scope(), task).await?;
    }
    Ok(())
}
