//! Late entry after an outage (wl-kernel §5.13, `PLT.ACL.10`).
//!
//! When a site loses its connection, staff keep working on paper and enter
//! the records later. A record keeps its **business time** (when the work
//! happened) and gets a **late-entry flag** with a reason when the business
//! time is more than M minutes before the entry time. Beyond N hours, or in
//! a closed period the module allows to reopen, the entry needs approval by
//! a holder of `late_entries.approve`; in a period the module forbids to
//! reopen it is refused.
//!
//! The contract: the check itself ([`assess`]), the opt-in entry a module
//! registers ([`LateEntryDef`]), the event ([`LateEntryRecorded`]) and the
//! rule runs listed for review ([`FiredRun`]). The records: the
//! [`LateEntry`] with its machine, and the limits M and N per entity
//! ([`limits_for`], [`set_limits`]).

pub mod assess;
pub mod enter;
pub mod entry;
pub mod events;
pub mod fired;
pub mod limits;
pub mod register;

use renox::prelude::*;

use crate::approvals::PolicyDefault;
use crate::machines::SubjectMachine;

/// The subject type of a late entry, which is the subject of its approval.
pub const SUBJECT_TYPE: &str = "kernel.late_entry";

/// The approval policies' action key of a late entry that needs approval.
pub const ACTION: &str = "kernel.late_entry";

pub use assess::{
    ApprovalWhy, Assessment, DEFAULT_APPROVAL_AFTER_HOURS, DEFAULT_FLAG_AFTER_MINUTES, Limits,
    Period, assess,
};
pub use enter::{Entered, EntrySpec, announce_approved, enter, preview};
pub use entry::{LateEntry, LateEntryStatus, transitions};
pub use events::{FiredRun, LateEntryRecorded};
pub use limits::{LateEntryLimits, limits_for, set_limits};
pub use register::{CreateFn, LateEntryDef, NewRecord, PeriodFn, get, register, subject_types};

/// The machine's registry entry: the waiting state `awaiting_approval`, and
/// the scope of the record as the scope where approvers are looked for.
pub fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<LateEntry>(SUBJECT_TYPE)
        .waiting(LateEntryStatus::AwaitingApproval)
        .action(transitions::REQUEST_APPROVAL, ACTION)
        .scope(|entry: &LateEntry| entry.scope())
        .build()
}

/// The default policy of a late entry that needs approval: one step, any
/// holder of `late_entries.approve` in the record's scope, SoD on.
pub fn default_policy() -> PolicyDefault {
    PolicyDefault::new(ACTION).step_permission(crate::permissions::LATE_ENTRIES_APPROVE)
}

/// Registers the models, the machine and the default policy.
pub(crate) fn register_models(app: &mut Registry) {
    app.model::<LateEntry>().model::<LateEntryLimits>();
    renox::state_machine::register::<LateEntry>();
    subject_machine().register();
    default_policy().register();
}
