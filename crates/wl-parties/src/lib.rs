#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod contacts;
pub mod create;
pub mod details;
pub mod directory;
mod machine;
pub mod party;
pub mod roles;
pub mod seed;
mod texts;

use renox::db::Migration;
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

/// The parties module. The host adds it with `.module(wl_parties::WlParties)`
/// when `client.toml` enables `parties`.
pub struct WlParties;

impl Module for WlParties {
    fn name(&self) -> &'static str {
        "wl-parties"
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Party>()
            .model::<PersonDetails>()
            .model::<CompanyDetails>()
            .model::<PartyRole>()
            .model::<ContactPoint>();
        // `rnx state:diagram Party` and the admin panel find the machine.
        renox::state_machine::register::<Party>();
        // Other modules read parties through the kernel (wl-parties §6).
        wl_kernel::parties::provide_directory(app, Directory);
        // `db:seed`: the demo parties (DEMO-DATA.md).
        app.seeder(|state: AppState| async move { seed::run(&state).await.map(|_| ()) });
    }
}
