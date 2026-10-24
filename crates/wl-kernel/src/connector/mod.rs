//! The read-only AI connector (decision 0006 points 2, 4, 6, 10,
//! `PLT.AGT.01`, BL-AGT-07): a user adds `https://<client domain>/mcp` to
//! Claude or ChatGPT and the assistant reads the company's records through
//! the same search, related records and reports as the user, nothing more.
//!
//! - [`mcp`]: the MCP endpoint, `POST /mcp`, serving every agent tool
//!   ([`crate::agent::tools`]): the read-only ones here and the actions and
//!   inbox of [`crate::agent`].
//! - [`read_tools`]: `search`, `fetch`, `related`, `list_reports`,
//!   `run_report`.
//! - [`fetch`]: the fields a module shows an assistant for its records.
//! - [`reports`]: the reports an assistant may run (the report library of
//!   wl-kernel §5.10 will fill it).
//! - [`untrusted`]: the marker on text written by outsiders.
//!
//! **What is not here yet.** Signing in with OAuth (R26, renox#82) and the
//! `renox-mcp` crate (R27, renox#83) are not released: the endpoint takes a
//! Renox API token for now ([`mcp`]). The company's policy (BL-AGT-06,
//! #546, `blocked:open-item`) is the seam of [`crate::agent::access`]:
//! off by default, so every call is refused until a deployment provides a
//! policy.

pub mod fetch;
pub mod mcp;
pub mod read_tools;
pub mod reports;
pub mod untrusted;

use renox::prelude::*;

pub use fetch::{RecordField, RecordFields, register_fields};
pub use reports::{AgentReport, ReportRows, register_report};
pub use untrusted::untrusted;

/// The connector's routes.
pub fn routes() -> Routes {
    mcp::routes()
}

/// Registers the read-only tools.
pub(crate) fn register(_app: &mut Registry) {
    read_tools::register();
}
