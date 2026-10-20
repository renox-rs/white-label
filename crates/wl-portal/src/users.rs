//! The kernel's [`PortalUsers`] from the access grants: a user acts for a
//! party while their grant for it is `active` on the day (wl-portal §3).
//! Verticals use it to reach a client's portal users and to check who may
//! acknowledge a notice (04 `ACC.RUL.09`).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::portal::{PortalFuture, PortalUser, PortalUsers};

use crate::grants::{AccessGrant, GrantStatus};

/// The grants' [`PortalUsers`], provided by [`crate::WlPortal`].
#[derive(Debug, Clone, Copy, Default)]
pub struct GrantUsers;

/// The portal user of `grant`, when it has a user.
fn user_of(grant: &AccessGrant) -> Option<PortalUser> {
    Some(PortalUser {
        user_id: grant.user_id?,
        person: PartyId(grant.person_party_id),
        acting_for: grant.party(),
        roles: grant.role_keys(),
    })
}

impl PortalUsers for GrantUsers {
    fn users_for<'a>(
        &'a self,
        db: &'a Db,
        party: PartyId,
        on: NaiveDate,
    ) -> PortalFuture<'a, Vec<PortalUser>> {
        Box::pin(async move {
            let grants = AccessGrant::query()
                .where_(AccessGrant::PARTY_ID.eq(party.0))
                .where_(AccessGrant::STATUS.eq(GrantStatus::Active))
                .order_by(AccessGrant::ID)
                .get(db)
                .await?;
            Ok(grants
                .iter()
                .filter(|grant| grant.is_active_on(on))
                .filter_map(user_of)
                .collect())
        })
    }

    fn acting_for<'a>(
        &'a self,
        db: &'a Db,
        user: i64,
        party: PartyId,
        on: NaiveDate,
    ) -> PortalFuture<'a, Option<PortalUser>> {
        Box::pin(async move {
            let grant = crate::access::PortalAccess::active_grant(db, user, party, on).await?;
            Ok(grant.as_ref().and_then(user_of))
        })
    }
}
