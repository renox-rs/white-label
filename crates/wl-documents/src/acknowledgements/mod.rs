//! SOP acknowledgements (wl-documents §3 *Acknowledgement*, §4.3, §5.7,
//! `DOC.RUL.04`, `HRM.ACL.10`, issue #53).
//!
//! Every required reader acknowledges the current version of each SOP that
//! applies to them: when a version is published, when they join and when they
//! take a new role. Overdue readers are reminded, then their manager and the
//! SOP's owner role are told. The acknowledgement is the reader's own act:
//! in their session (`click-to-accept`, with IP address, device, the file's
//! checksum and language) or in person with a drawn signature and a staff
//! user as witness.
//!
//! | Part | Module |
//! |---|---|
//! | The record and its machine | [`model`], [`machine`] |
//! | Request, open, acknowledge, replace, cancel | [`service`] |
//! | The daily run: request, remind, escalate | [`run`] |
//! | The events that ask and cancel | `events` |
//! | The kernel contract wl-hr asks | [`contract`] |
//! | The staff inbox and the in-person list | [`web`] |

pub mod contract;
mod events;
pub mod machine;
pub mod model;
pub(crate) mod notices;
pub mod run;
pub mod service;
pub mod web;

pub use contract::SopDesk;
pub use machine::AckRun;
pub use model::{AckStatus, METHOD_CLICK, METHOD_DRAWN, SopAcknowledgement};
pub use service::{AckRefused, Acknowledgements, OpenedFile, OwnSession, Reconciled};

use renox::prelude::*;

/// Registers the acknowledgement model, machine, listeners, notification
/// types, the daily task and the kernel contract.
pub(crate) fn register(app: &mut Registry) {
    app.model::<SopAcknowledgement>();
    renox::state_machine::register::<SopAcknowledgement>();
    events::register(app);
    notices::register_types();
    run::schedule(app);
    wl_kernel::documents::provide_sop_acknowledgements(app, SopDesk);
}
