//! The access grant services (wl-portal §4, §6 `PortalAccess`, issue #44):
//! who may act for which party, and the lifecycle of a grant.
//!
//! - [`PortalAccess::grants_for`] and [`PortalAccess::active_grant`]: the
//!   grants of a portal user, and the `active` one for a party on a day.
//! - [`PortalAccess::register`]: self-registration, directly `active`.
//! - [`PortalAccess::invite`] and [`PortalAccess::accept`]: an invitation
//!   (`portal_users.invite`) and its acceptance by the invitee. The token and
//!   the mail of an invitation are another issue.
//! - [`PortalAccess::expire`], [`revoke`](PortalAccess::revoke),
//!   [`suspend`](PortalAccess::suspend), [`reinstate`](PortalAccess::reinstate),
//!   [`end`](PortalAccess::end), [`close`](PortalAccess::close).
//!
//! `PartyBlocked` changes nothing here: the grants of a blocked party's users
//! stay as they are, with no suspension and no message (`PTY.ACL.02`, no
//! tipping off).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::PartyId;
use wl_kernel::documents::today;

use crate::events::{
    PortalAccessActivated, PortalAccessClosed, PortalAccessReadOnly, PortalAccessSuspended,
};
use crate::grants::{
    ACCEPT, AcceptInput, AccessGrant, CLOSE, END, EXPIRE, GrantStatus, REINSTATE,
    REINSTATE_READ_ONLY, REVOKE, SOURCE_INVITATION, SOURCE_SELF_REGISTRATION, SUSPEND,
};
use crate::permissions;

/// What an invitation or a self-registration names.
#[derive(Debug, Clone)]
pub struct NewGrant {
    /// The person party the portal user is linked to.
    pub person: PartyId,
    /// The party the user acts for.
    pub acting_for: PartyId,
    /// The audience, e.g. `tenant`.
    pub audience: String,
    /// The portal role keys.
    pub roles: Vec<String>,
    /// The last day the grant counts, if it ends on a date.
    pub valid_until: Option<NaiveDate>,
}

impl NewGrant {
    /// A grant for `person` acting for `acting_for` in `audience` with `roles`.
    pub fn new(person: PartyId, acting_for: PartyId, audience: &str, roles: &[&str]) -> Self {
        NewGrant {
            person,
            acting_for,
            audience: audience.to_owned(),
            roles: roles.iter().map(|r| (*r).to_owned()).collect(),
            valid_until: None,
        }
    }
}

/// The portal access service.
pub struct PortalAccess;

/// `Forbidden` unless `by` holds `permission`.
fn require(by: &AuthUser, permission: &str) -> Result {
    if by.has_permission(permission) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

impl PortalAccess {
    /// The grants of the portal user `user`, oldest first.
    pub async fn grants_for(db: &Db, user: i64) -> Result<Vec<AccessGrant>> {
        AccessGrant::query()
            .where_(AccessGrant::USER_ID.eq(user))
            .order_by(AccessGrant::ID)
            .get(db)
            .await
    }

    /// The `active` grant of `user` for the party `acting_for` on `day`, if
    /// there is one.
    pub async fn active_grant(
        db: &Db,
        user: i64,
        acting_for: PartyId,
        day: NaiveDate,
    ) -> Result<Option<AccessGrant>> {
        Ok(AccessGrant::query()
            .where_(AccessGrant::USER_ID.eq(user))
            .where_(AccessGrant::PARTY_ID.eq(acting_for.0))
            .where_(AccessGrant::STATUS.eq(GrantStatus::Active))
            .order_by(AccessGrant::ID)
            .get(db)
            .await?
            .into_iter()
            .find(|grant| grant.is_active_on(day)))
    }

    /// Self-registration: a grant for `user`, directly `active`, with the
    /// audience's default role.
    pub async fn register(state: &AppState, user: i64, new: NewGrant) -> Result<AccessGrant> {
        let grant = Self::create(state, Some(user), new, SOURCE_SELF_REGISTRATION, true).await?;
        state
            .emit(PortalAccessActivated {
                grant: grant.id,
                user,
                party: grant.party(),
            })
            .await?;
        Ok(grant)
    }

    /// An invitation: a grant in `invited`, sent by `by` (holding
    /// `portal_users.invite`).
    pub async fn invite(state: &AppState, by: &AuthUser, new: NewGrant) -> Result<AccessGrant> {
        require(by, permissions::USERS_INVITE)?;
        Self::create(state, None, new, SOURCE_INVITATION, false).await
    }

    async fn create(
        state: &AppState,
        user: Option<i64>,
        new: NewGrant,
        source: &str,
        active: bool,
    ) -> Result<AccessGrant> {
        AccessGrant::create(
            &state.db,
            AccessGrant {
                user_id: user,
                person_party_id: new.person.0,
                party_id: new.acting_for.0,
                audience: new.audience,
                roles: new.roles.join(","),
                source: source.to_owned(),
                valid_from: today(),
                valid_until: new.valid_until,
                status: if active {
                    GrantStatus::Active
                } else {
                    GrantStatus::Invited
                },
                ..AccessGrant::default()
            },
        )
        .await
    }

    /// `accept`: the invitee, now the portal user `user`, takes the
    /// invitation. Emits `PortalAccessActivated`.
    pub async fn accept(state: &AppState, grant: i64, user: i64) -> Result<AccessGrant> {
        let row = AccessGrant::find_or_404(&state.db, grant).await?;
        let _outcome = row
            .transition(ACCEPT)
            .by_system()
            .input(&AcceptInput { user_id: user })
            .run(state)
            .await
            .map_err(|e| e.into_error())?;
        let done = AccessGrant::find_or_404(&state.db, grant).await?;
        state
            .emit(PortalAccessActivated {
                grant,
                user,
                party: done.party(),
            })
            .await?;
        Ok(done)
    }

    /// Runs `name` on grant `id` with an optional `reason`.
    async fn run(
        state: &AppState,
        id: i64,
        name: &str,
        reason: Option<&str>,
    ) -> Result<AccessGrant> {
        let row = AccessGrant::find_or_404(&state.db, id).await?;
        let mut run = row.transition(name).by_system();
        if let Some(reason) = reason.map(str::trim).filter(|r| !r.is_empty()) {
            run = run.reason(reason);
        }
        let _outcome = run.run(state).await.map_err(|e| e.into_error())?;
        AccessGrant::find_or_404(&state.db, id).await
    }

    /// `expire`: the invitation was not accepted in time (the system).
    pub async fn expire(state: &AppState, grant: i64) -> Result<AccessGrant> {
        Self::run(state, grant, EXPIRE, None).await
    }

    /// `revoke`: the invitation is withdrawn by `by` (`portal_users.invite`).
    pub async fn revoke(state: &AppState, by: &AuthUser, grant: i64) -> Result<AccessGrant> {
        require(by, permissions::USERS_INVITE)?;
        Self::run(state, grant, REVOKE, None).await
    }

    /// `suspend`: `by` (`portal_users.manage`) blocks the grant for `reason`.
    /// Emits `PortalAccessSuspended`.
    pub async fn suspend(
        state: &AppState,
        by: &AuthUser,
        grant: i64,
        reason: &str,
    ) -> Result<AccessGrant> {
        require(by, permissions::USERS_MANAGE)?;
        let done = Self::run(state, grant, SUSPEND, Some(reason)).await?;
        state
            .emit(PortalAccessSuspended {
                grant,
                reason: reason.trim().to_owned(),
            })
            .await?;
        Ok(done)
    }

    /// `reinstate`: `by` (`portal_users.manage`) takes a suspended grant back
    /// to the state before the suspension.
    pub async fn reinstate(state: &AppState, by: &AuthUser, grant: i64) -> Result<AccessGrant> {
        require(by, permissions::USERS_MANAGE)?;
        let row = AccessGrant::find_or_404(&state.db, grant).await?;
        let name = if row.suspended_from.as_deref() == Some("read_only") {
            REINSTATE_READ_ONLY
        } else {
            REINSTATE
        };
        Self::run(state, grant, name, None).await
    }

    /// `end`: the relationship ended. `by` is a user holding
    /// `portal_users.manage`, or `None` for the system. Emits
    /// `PortalAccessReadOnly`.
    pub async fn end(
        state: &AppState,
        by: Option<&AuthUser>,
        grant: i64,
        reason: Option<&str>,
    ) -> Result<AccessGrant> {
        if let Some(by) = by {
            require(by, permissions::USERS_MANAGE)?;
        }
        let done = Self::run(state, grant, END, reason).await?;
        state
            .emit(PortalAccessReadOnly {
                grant,
                reason: reason.map(str::to_owned),
            })
            .await?;
        Ok(done)
    }

    /// `close`: no access any more. `by` is a user holding
    /// `portal_users.manage`, or `None` for the system (the close-after
    /// period). Emits `PortalAccessClosed`.
    pub async fn close(
        state: &AppState,
        by: Option<&AuthUser>,
        grant: i64,
        reason: Option<&str>,
    ) -> Result<AccessGrant> {
        if let Some(by) = by {
            require(by, permissions::USERS_MANAGE)?;
        }
        let done = Self::run(state, grant, CLOSE, reason).await?;
        state
            .emit(PortalAccessClosed {
                grant,
                reason: reason.map(str::to_owned),
            })
            .await?;
        Ok(done)
    }
}
