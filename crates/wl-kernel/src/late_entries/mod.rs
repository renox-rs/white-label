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
//! This part is the contract: the check itself ([`assess`]), the opt-in
//! entry a module registers ([`LateEntryDef`]), the event
//! ([`LateEntryRecorded`]) and the rule runs listed for review
//! ([`FiredRun`]).

pub mod assess;
pub mod events;
pub mod register;

pub use assess::{
    ApprovalWhy, Assessment, DEFAULT_APPROVAL_AFTER_HOURS, DEFAULT_FLAG_AFTER_MINUTES, Limits,
    Period, assess,
};
pub use events::{FiredRun, LateEntryRecorded};
pub use register::{CreateFn, LateEntryDef, NewRecord, PeriodFn, get, register, subject_types};
