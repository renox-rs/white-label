//! Who acts for a party in the portal (wl-portal §6 *Access grant*): the
//! contract a vertical uses to find the client users of a party (to send
//! them a notice) and to check that a signed-in user acts for a party (to
//! let them acknowledge it), without depending on the `wl-portal` crate.
//!
//! `wl-portal` provides it from its access grants: a user acts for a party
//! while their grant for it is `active` on the day. Without the `portal`
//! module, [`portal_users`] gives [`NoPortalUsers`]: nobody acts for any
//! party.
//!
//! ```ignore
//! use wl_kernel::portal::portal_users;
//!
//! // 04 `ACC.RUL.09`: the client approvers of the client company.
//! let approvers: Vec<_> = portal_users(&state)
//!     .users_for(&state.db, client, today)
//!     .await?
//!     .into_iter()
//!     .filter(|u| u.has_any_role(&["owner", "approver"]))
//!     .collect();
//! // The acknowledge button: may this user act for the client?
//! let acting = portal_users(&state).acting_for(&state.db, user.id, client, today).await?;
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::parties::PartyId;

/// A portal user acting for a party.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalUser {
    /// The user's id.
    pub user_id: i64,
    /// The person (a party) the user is.
    pub person: PartyId,
    /// The party the user acts for.
    pub acting_for: PartyId,
    /// The role keys of the grant (`owner`, `approver`, `finance`…).
    pub roles: Vec<String>,
}

impl PortalUser {
    /// Whether the grant has one of `roles`.
    pub fn has_any_role(&self, roles: &[&str]) -> bool {
        self.roles.iter().any(|r| roles.contains(&r.as_str()))
    }
}

/// The future a [`PortalUsers`] method returns.
pub type PortalFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Who acts for a party in the portal. `wl-portal` provides it; without
/// that module, [`portal_users`] gives [`NoPortalUsers`].
pub trait PortalUsers: Send + Sync + 'static {
    /// The users acting for `party` on `on` (an `active` grant with a user,
    /// valid that day), oldest grant first; the default has none.
    fn users_for<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _on: NaiveDate,
    ) -> PortalFuture<'a, Vec<PortalUser>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// The user `user` acting for `party` on `on`, when they hold an
    /// `active` grant for it that day; the default knows none.
    fn acting_for<'a>(
        &'a self,
        _db: &'a Db,
        _user: i64,
        _party: PartyId,
        _on: NaiveDate,
    ) -> PortalFuture<'a, Option<PortalUser>> {
        Box::pin(async { Ok(None) })
    }

    /// Every party `user` acts for on `on` (their `active` grants valid that
    /// day), oldest grant first; the default has none. A vertical lists a
    /// client user's own companies with it (09 `MPW.ACL.01`).
    fn parties_of<'a>(
        &'a self,
        _db: &'a Db,
        _user: i64,
        _on: NaiveDate,
    ) -> PortalFuture<'a, Vec<PortalUser>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// The one text a portal user is shown for an action refused while the
    /// party is blocked: the same whatever the reason (`PTY.ACL.02`). `wl-portal`
    /// answers with its setting; the default is the English text.
    fn neutral_text(&self, _lang: &Lang) -> String {
        "This request cannot be completed online. Please contact us.".to_owned()
    }

    /// Takes away every portal grant for `party` that still gives access
    /// (an invitation not accepted is expired, an `active`, `read_only` or
    /// `suspended` grant is closed), with `reason`; answers how many grants
    /// were ended. Grants for other parties are not touched, so a user who
    /// acts for several companies keeps those. The default has none (04
    /// `decline`: "client users disabled").
    fn close_access<'a>(
        &'a self,
        _state: &'a AppState,
        _party: PartyId,
        _reason: &'a str,
    ) -> PortalFuture<'a, usize> {
        Box::pin(async { Ok(0) })
    }
}

/// The portal users when the `portal` module is off: nobody acts for any
/// party.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPortalUsers;

impl PortalUsers for NoPortalUsers {}

/// Makes `users` the app's [`PortalUsers`]; `wl-portal` calls it in
/// `register`.
pub fn provide_portal_users(app: &mut Registry, users: impl PortalUsers) {
    let users: Arc<dyn PortalUsers> = Arc::new(users);
    app.provide(users);
}

/// The app's [`PortalUsers`], or [`NoPortalUsers`] when none is provided.
pub fn portal_users(state: &AppState) -> Arc<dyn PortalUsers> {
    match state.provided::<Arc<dyn PortalUsers>>() {
        Some(users) => Arc::clone(&users),
        None => Arc::new(NoPortalUsers),
    }
}

/// Whether `user` is a person of the portal and nothing else: they hold no role
/// but `member` and the roles of a client's own users (`client_*`, e.g.
/// `client_approver`). The sidebar uses it to leave out the links that are
/// shown to every signed-in person but belong to staff (the review tasks, the
/// broadcasts).
pub fn is_portal_only(user: &AuthUser) -> bool {
    user.role_names()
        .iter()
        .all(|role| role == "member" || role.starts_with("client_"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_has_a_role_of_its_grant() {
        let user = PortalUser {
            user_id: 3,
            person: PartyId(4),
            acting_for: PartyId(5),
            roles: vec!["owner".into(), "finance".into()],
        };
        assert!(user.has_any_role(&["approver", "owner"]));
        assert!(!user.has_any_role(&["approver"]));
    }
}
