#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

mod texts;

use renox::prelude::*;

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
}
