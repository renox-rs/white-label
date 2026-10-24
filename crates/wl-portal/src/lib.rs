#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod access;
pub mod audiences;
pub mod events;
pub mod grants;
pub mod guard;
pub mod links;
pub mod refusals;
pub mod search;
pub mod sections;
pub mod settings;
mod texts;
pub mod users;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use access::{CLOSE_AFTER_REASON, NewGrant, PortalAccess};
pub use audiences::{Audience, AudienceDef, PortalRole, RoleDef};
pub use events::{
    PortalAccessActivated, PortalAccessClosed, PortalAccessReadOnly, PortalAccessSuspended,
};
pub use grants::{AccessGrant, GrantStatus};
pub use guard::{LINK_USER, PortalAction, PortalGuard, Refused};
pub use links::{SignedLinkUsed, SignedLinks};
pub use refusals::RefusalLog;
pub use sections::{
    MenuEntry, PortalMenu, PortalSection, SectionFuture, SectionRow, register_section,
};
pub use texts::{neutral, read_only_notice};

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
const VIEWS: &[(&str, &str)] = &[
    (
        "wl-portal/refusal_log.html",
        include_str!("../views/wl-portal/refusal_log.html"),
    ),
    (
        "wl-portal/section.html",
        include_str!("../views/wl-portal/section.html"),
    ),
    (
        "wl-portal/link.html",
        include_str!("../views/wl-portal/link.html"),
    ),
    (
        "wl-portal/home.html",
        include_str!("../views/wl-portal/home.html"),
    ),
    (
        "wl-portal/roles.html",
        include_str!("../views/wl-portal/roles.html"),
    ),
    (
        "wl-portal/search.html",
        include_str!("../views/wl-portal/search.html"),
    ),
];

/// The daily close-after timer (`POR.ACL.01`, BL-POR-03).
pub const CLOSE_TASK: &str = "portal:close-read-only";
/// When it runs, Asia/Dubai: just after midnight, so a grant closes on its
/// day 91 at 00:00.
pub const CLOSE_TASK_AT: &str = "00:05";

/// The portal module. The host adds it with `.module(wl_portal::WlPortal)`
/// when `client.toml` enables `portal`.
pub struct WlPortal;

impl Module for WlPortal {
    fn name(&self) -> &'static str {
        "wl-portal"
    }

    fn routes(&self) -> Routes {
        refusals::routes()
            .merge(search::routes())
            .merge(sections::routes())
            .merge(links::routes())
            .merge(settings::routes())
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<AccessGrant>();
        app.model::<RefusalLog>();
        app.model::<Audience>();
        app.model::<PortalRole>();
        renox::state_machine::register::<AccessGrant>();
        app.view_data::<refusals::RefusalLogPage>();
        app.view_data::<sections::SectionPage>();
        app.view_data::<links::LinkPage>();
        app.view_data::<sections::HomePage>();
        app.view_data::<settings::RolesPage>();
        app.view_data::<search::PortalSearchPage>();
        app.schedule()
            .daily_at(CLOSE_TASK_AT, CLOSE_TASK, |state: AppState| async move {
                PortalAccess::close_due(&state, wl_kernel::documents::today())
                    .await
                    .map(|_| ())
            })
            .timezone("Asia/Dubai");
        // Who acts for a party, for the verticals' notices (kernel `portal`).
        wl_kernel::portal::provide_portal_users(app, users::GrantUsers);
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
        // The portal roles and their permissions (BL-POR-02); the label is
        // the app's `nav.portal_roles` text.
        app.nav(
            "app",
            NavLink::route("wl_portal.roles", "nav.portal_roles")
                .icon("users")
                .group("nav.admin")
                .order(47)
                .permission(permissions::USERS_MANAGE),
        );
    }
}
