#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod block;
pub mod block_record;
pub mod contacts;
pub mod create;
pub mod details;
pub mod directory;
pub mod evidence;
mod machine;
pub mod pages;
pub mod party;
pub mod reasons;
pub mod roles;
pub mod seed;
mod texts;
pub mod unblock;
pub mod visibility;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use block::{block, block_by_system};
pub use block_record::BlockRecord;
pub use contacts::{ContactPoint, ContactType};
pub use create::{NewParty, create};
pub use details::{CompanyDetails, PersonDetails};
pub use directory::Directory;
pub use evidence::{UnblockBasis, UnblockEvidence};
pub use party::{Channel, Language, Party};
pub use reasons::{BlockReason, BlockReasons};
pub use roles::PartyRole;
pub use unblock::{UnblockRequest, unblock};

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
    (
        "wl-parties/_block_sheet.html",
        include_str!("../views/wl-parties/_block_sheet.html"),
    ),
    (
        "wl-parties/_unblock_sheet.html",
        include_str!("../views/wl-parties/_unblock_sheet.html"),
    ),
];

/// The parties module. The host adds it with `.module(wl_parties::WlParties)`
/// when `client.toml` enables `parties`; a vertical that adds block reason
/// codes uses `WlParties::new().block_reason("tax_evasion", true)`.
#[derive(Debug, Clone, Default)]
pub struct WlParties {
    /// The block reason codes (wl-parties §7): the defaults plus the
    /// vertical's; `None` until one is added, meaning the defaults.
    reasons: Option<BlockReasons>,
}

/// The module with the default block reason codes, so the host can write
/// `.module(wl_parties::WlParties)`.
#[allow(non_upper_case_globals)]
pub const WlParties: WlParties = WlParties { reasons: None };

impl WlParties {
    /// The module with the default block reason codes.
    pub fn new() -> Self {
        WlParties::default()
    }

    /// Adds the block reason `code`, restricted or not (wl-parties §7). A
    /// restricted code is seen and used only under
    /// `parties.block_reason.view`; everyone else sees `on_hold`.
    ///
    /// # Panics
    ///
    /// When `code` is `on_hold` (always present, never restricted), is not
    /// a `snake_case` key, or would un-restrict a restricted code; see
    /// [`BlockReasons::add`].
    pub fn block_reason(mut self, code: &str, restricted: bool) -> Self {
        self.reasons
            .get_or_insert_with(BlockReasons::default)
            .add(code, restricted);
        self
    }

    /// The block reason codes this module is set up with.
    pub fn reasons(&self) -> BlockReasons {
        self.reasons.clone().unwrap_or_default()
    }
}

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
            .model::<BlockRecord>()
            .model::<UnblockEvidence>()
            .view_data::<pages::IndexPage>()
            .view_data::<pages::NewPage>()
            .view_data::<pages::ShowPage>();
        // `rnx state:diagram Party` and the admin panel find the machine.
        renox::state_machine::register::<Party>();
        // The block reason codes; `on_hold` must be there and not restricted.
        let reasons = self.reasons();
        reasons.check();
        app.provide(reasons);
        // `PartyBlocked` after a block commits.
        block::register(app);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_module_has_the_default_reasons_unless_a_vertical_adds_one() {
        assert_eq!(WlParties.reasons(), BlockReasons::default());
        assert_eq!(WlParties::new().reasons(), BlockReasons::default());
        let module = WlParties::new().block_reason("tax_evasion", true);
        let reasons = module.reasons();
        assert!(reasons.is_restricted("tax_evasion"));
        assert_eq!(reasons.all().len(), 8);
        reasons.check();
    }

    #[test]
    #[should_panic(expected = "on_hold")]
    fn a_vertical_cannot_restrict_on_hold() {
        let _ = WlParties::new().block_reason("on_hold", true);
    }
}
