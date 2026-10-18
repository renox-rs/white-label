#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod contacts;
pub mod create;
pub mod details;
pub mod directory;
mod machine;
pub mod pages;
pub mod party;
pub mod roles;
pub mod seed;
mod texts;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use contacts::{ContactPoint, ContactType};
pub use create::{NewParty, create};
pub use details::{CompanyDetails, PersonDetails};
pub use directory::Directory;
pub use party::{Channel, Language, Party};
pub use roles::PartyRole;

/// The module's permissions (wl-parties §6).
pub mod permissions {
    /// See parties, their contacts, roles and history.
    pub const VIEW: &str = "parties.view";
    /// Create parties.
    pub const CREATE: &str = "parties.create";
    /// Change a party's details.
    pub const UPDATE: &str = "parties.update";
    /// Block and unblock parties.
    pub const BLOCK: &str = "parties.block";
    /// See and use restricted block reason codes and Block records
    /// (`PTY.ACL.02`; MLRO and deputy by default).
    pub const BLOCK_REASON_VIEW: &str = "parties.block_reason.view";
    /// Export the parties list.
    pub const EXPORT: &str = "parties.export";
}

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
const VIEWS: &[(&str, &str)] = &[
    (
        "wl-parties/index.html",
        include_str!("../views/wl-parties/index.html"),
    ),
    (
        "wl-parties/new.html",
        include_str!("../views/wl-parties/new.html"),
    ),
    (
        "wl-parties/show.html",
        include_str!("../views/wl-parties/show.html"),
    ),
    (
        "wl-parties/_card.html",
        include_str!("../views/wl-parties/_card.html"),
    ),
];

/// The parties module. The host adds it with `.module(wl_parties::WlParties)`
/// when `client.toml` enables `parties`.
pub struct WlParties;

impl Module for WlParties {
    fn name(&self) -> &'static str {
        "wl-parties"
    }

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Party>()
            .model::<PersonDetails>()
            .model::<CompanyDetails>()
            .model::<PartyRole>()
            .model::<ContactPoint>()
            .view_data::<pages::IndexPage>()
            .view_data::<pages::NewPage>()
            .view_data::<pages::ShowPage>();
        // `rnx state:diagram Party` and the admin panel find the machine.
        renox::state_machine::register::<Party>();
        // Other modules read parties through the kernel (wl-parties §6).
        wl_kernel::parties::provide_directory(app, Directory);
        // `db:seed`: the demo parties (DEMO-DATA.md).
        app.seeder(|state: AppState| async move { seed::run(&state).await.map(|_| ()) });
        // In the app group of the staff sidebar; the label is the app's
        // `nav.parties` text.
        app.nav(
            "app",
            NavLink::route("wl_parties.index", "nav.parties")
                .icon("handshake")
                .group("nav.app")
                .order(30)
                .permission(permissions::VIEW),
        );
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
    }
}
