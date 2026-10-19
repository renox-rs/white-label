//! Compliance-critical automation rules (wl-kernel §3 *Compliance-critical
//! flag*, §5.15, `PLT.RUL.04`).
//!
//! Some rules protect a legal duty: sanctions re-screening, WPS alerts, the
//! VAT due date, document expiry blocks, fire-safety visit checks. The
//! owning module marks them with [`CriticalRule`]; a vertical may add more.
//! A client cannot turn the flag off: it is code, and `wl:install` writes it
//! back. Only the recipients of the alert (the owner role) are editable.
//!
//! This part is the contract: the registry, the scheduled occurrences the
//! kernel watches ([`Schedule`], [`Planned`]) and the rule that an occurrence
//! is missed when its planned time is 60 minutes or more in the past with no
//! rule run ([`MISSED_AFTER_MINUTES`]).

pub mod alert;
pub mod registry;
pub mod rules;

use renox::prelude::*;

pub use alert::{AlertKind, AlertStatus, ClosedHow, ComplianceAlert};
pub use registry::{
    CriticalRule, LOOK_BACK_DAYS, MISSED_AFTER_MINUTES, Planned, PlannedFn, Schedule, all,
    is_critical, missed_before, occurrence_key, planned_daily, register, rule, schedule_subject,
};
pub use rules::{ComplianceRule, business_owner_role, install, owner_role};

/// Registers the models.
pub(crate) fn register_models(app: &mut Registry) {
    app.model::<ComplianceRule>().model::<ComplianceAlert>();
}
