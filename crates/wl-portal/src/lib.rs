#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod access;
pub mod events;
pub mod grants;
pub mod guard;
pub mod refusals;
mod texts;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use access::{NewGrant, PortalAccess};
pub use events::{
    PortalAccessActivated, PortalAccessClosed, PortalAccessReadOnly, PortalAccessSuspended,
};
pub use grants::{AccessGrant, GrantStatus};
pub use guard::{PortalAction, PortalGuard, Refused};
pub use refusals::RefusalLog;
pub use texts::neutral;

/// The module's permissions (wl-portal §6).
pub mod permissions {
    /// Send and revoke invitations.
    pub const USERS_INVITE: &str = "portal_users.invite";
    /// Suspend, reinstate, end and close grants.
    pub const USERS_MANAGE: &str = "portal_users.manage";
    /// Declared by `wl-parties` (`PTY.ACL.02`): reads the restricted block
    /// reasons. The refusal log of the portal needs it too, so that only
    /// those who may know a party is blocked see which portal actions were
    /// refused.
    pub const BLOCK_REASON_VIEW: &str = "parties.block_reason.view";
}

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
const VIEWS: &[(&str, &str)] = &[(
    "wl-portal/refusal_log.html",
    include_str!("../views/wl-portal/refusal_log.html"),
)];

/// The portal module. The host adds it with `.module(wl_portal::WlPortal)`
/// when `client.toml` enables `portal`.
pub struct WlPortal;

impl Module for WlPortal {
    fn name(&self) -> &'static str {
        "wl-portal"
    }

    fn routes(&self) -> Routes {
        refusals::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<AccessGrant>();
        app.model::<RefusalLog>();
        renox::state_machine::register::<AccessGrant>();
        app.view_data::<refusals::RefusalLogPage>();
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        // `PartyBlocked` and `PartyUnblocked` need no listener: the grants of
        // a blocked party's users stay as they are, no message is sent, and
        // the guard asks the block itself on every action (`PTY.ACL.02`).
        //
        // In the admin group of the staff sidebar; the label is the app's
        // `nav.portal_block_refusals` text.
        app.nav(
            "app",
            NavLink::route("wl_portal.block_refusals", "nav.portal_block_refusals")
                .icon("shield")
                .group("nav.admin")
                .order(46)
                .permission(permissions::BLOCK_REASON_VIEW),
        );
    }
}
