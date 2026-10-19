//! What follows the commit of a lease transition (01 Property §7.1).
//!
//! - **`activate`:** `LeaseActivated` is emitted, the Ejari task is opened for
//!   the property manager (`PRP.RUL.02`: the registration is chased 14 days
//!   after activation), and a renewal or amendment whose deposit differs from
//!   the held amount asks the deposit to `adjust` (system; an amendment's
//!   effects are tested with BL-PRP-08).

use renox::chrono::Duration;
use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::deposits::{AdjustDeposit, deposits};
use wl_kernel::role_tasks::RoleTask;

use crate::events::LeaseActivated;
use crate::lease::{Lease, VersionKind};
use crate::lease_machine::ACTIVATE;
use crate::leases::ejari_task_key;
use crate::permissions::{ACCOUNTANT_ROLE, PROPERTY_MANAGER_ROLE};
use crate::texts::english;

/// The days after activation the Ejari registration is due (`PRP.RUL.02`).
pub const EJARI_DAYS: i64 = 14;

/// Registers the listener.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Lease>, state: AppState| async move {
        match event.transition.as_str() {
            ACTIVATE => after_activate(&state, event.key).await,
            _ => Ok(()),
        }
    });
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
async fn after_activate(state: &AppState, id: i64) -> Result {
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
    let task = RoleTask::new(
        lease.subject(),
        PROPERTY_MANAGER_ROLE,
        &ejari_title(&lease),
        &ejari_task_key(&lease),
    )
    .description(&english("property.tasks.ejari_description", &[]))
    .due_at(renox::db::now() + Duration::days(EJARI_DAYS));
    crate::tasks::open(state, &lease.scope(), task).await?;
    adjust_deposit(state, &lease).await
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
        eprintln!("property: deposit of {} not adjusted: {error:?}", lease.number());
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
