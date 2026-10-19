//! The registry of entities that opt in to the late-entry check (wl-kernel
//! §5.13), keyed by subject type like [`crate::voids`].
//!
//! A module declares its entity once in its `register()`:
//!
//! ```ignore
//! LateEntryDef::of("bookings.check_in", |tx, new| {
//!     Box::pin(async move { insert_check_in(tx, new).await })
//! })
//! .business_time("checked_in_at")
//! .parent("booking")
//! .period(|tx, occurred_at| Box::pin(async move { timesheet_period(tx, occurred_at).await }))
//! .register();
//! ```
//!
//! The kernel decides, from the business time and the entry time, whether
//! the entry is flagged, needs approval or is refused ([`super::enter`]);
//! the module's `create` handler writes the record, at once or once the
//! approval is given, so a rejected late entry creates nothing.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};

use renox::db::{DateTime, Transaction};
use renox::serde_json::Value;
use renox::{Error, Result};

use super::assess::Period;
use crate::machines::BoxFuture;
use crate::subject::Subject;

/// What the module's `create` handler is told when it writes the record.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct NewRecord {
    /// The record being changed, when the business time of an existing
    /// record is edited; `None` for a new record.
    pub existing: Option<i64>,
    /// What the module passed in [`super::EntrySpec::payload`] (the form).
    pub payload: Value,
    /// The business time: when the work really happened. Reports, deadlines
    /// and SLA calculations use this, never the entry time.
    pub occurred_at: DateTime,
    /// The entry time: when it was typed in, for the audit.
    pub entered_at: DateTime,
    /// Whether the record carries the late-entry flag.
    pub late: bool,
    /// Why it is late.
    pub reason: Option<String>,
    /// The outage reference, if any.
    pub outage_reference: Option<String>,
    /// The approval request, when the entry needed approval.
    pub approval_request_id: Option<i64>,
    /// The late entry (`wl_late_entries`), when the entry is late.
    pub late_entry_id: Option<i64>,
    /// The user who entered it.
    pub user_id: i64,
    /// The parent subject the module named, if any.
    pub parent: Option<Subject>,
}

/// The module's handler that writes (or, for an edit, updates) the record in
/// the kernel's transaction and returns its id.
pub type CreateFn = Arc<
    dyn for<'a> Fn(&'a mut Transaction, &'a NewRecord) -> BoxFuture<'a, Result<i64>> + Send + Sync,
>;

/// The module's handler that says what period a business time falls in.
pub type PeriodFn = Arc<
    dyn for<'a> Fn(&'a mut Transaction, DateTime) -> BoxFuture<'a, Result<Period>> + Send + Sync,
>;

/// An entity that opts in to the late-entry check. Built with
/// [`LateEntryDef::of`].
#[derive(Clone)]
#[must_use = "an entity is only checked once registered"]
pub struct LateEntryDef {
    subject_type: String,
    business_time: Option<String>,
    parent: Option<String>,
    create: CreateFn,
    period: Option<PeriodFn>,
}

impl fmt::Debug for LateEntryDef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LateEntryDef")
            .field("subject_type", &self.subject_type)
            .field("business_time", &self.business_time)
            .field("parent", &self.parent)
            .field("period", &self.period.is_some())
            .finish()
    }
}

impl LateEntryDef {
    /// Starts the entry of `subject_type` (`<module>.<entity>`) with the
    /// handler that writes its records.
    ///
    /// # Panics
    ///
    /// When `subject_type` is not `<module>.<entity>`.
    pub fn of<F>(subject_type: &str, create: F) -> Self
    where
        F: for<'a> Fn(&'a mut Transaction, &'a NewRecord) -> BoxFuture<'a, Result<i64>>
            + Send
            + Sync
            + 'static,
    {
        // Checks the type's form; panics like `Subject::new` on a bad one.
        let _ = Subject::new(subject_type, 0);
        LateEntryDef {
            subject_type: subject_type.to_owned(),
            business_time: None,
            parent: None,
            create: Arc::new(create),
            period: None,
        }
    }

    /// The name of the business-time field (documentation and the entry
    /// page; the module passes the value in [`super::EntrySpec`]).
    pub fn business_time(mut self, field: &str) -> Self {
        self.business_time = Some(field.to_owned());
        self
    }

    /// The name of the parent-subject field: the `Subject` of the record the
    /// entry belongs to (`booking` on a check-in, `order` on a POD). The
    /// rule runs on that record are listed for review too.
    pub fn parent(mut self, field: &str) -> Self {
        self.parent = Some(field.to_owned());
        self
    }

    /// How the module tells the period of a business time (a locked
    /// timesheet, a locked VAT period, a closed payroll). Without it every
    /// business time is in an open period.
    pub fn period<F>(mut self, period: F) -> Self
    where
        F: for<'a> Fn(&'a mut Transaction, DateTime) -> BoxFuture<'a, Result<Period>>
            + Send
            + Sync
            + 'static,
    {
        self.period = Some(Arc::new(period));
        self
    }

    /// The subject type, e.g. `bookings.check_in`.
    pub fn subject_type(&self) -> &str {
        &self.subject_type
    }

    /// The name of the business-time field, if the module gave one.
    pub fn business_time_field(&self) -> Option<&str> {
        self.business_time.as_deref()
    }

    /// The name of the parent-subject field, if the module gave one.
    pub fn parent_field(&self) -> Option<&str> {
        self.parent.as_deref()
    }

    /// The period of `occurred_at`; open when the module gave no handler.
    pub async fn period_of(&self, tx: &mut Transaction, occurred_at: DateTime) -> Result<Period> {
        match &self.period {
            Some(period) => period(tx, occurred_at).await,
            None => Ok(Period::Open),
        }
    }

    /// Writes the record through the module's handler.
    pub async fn create_in(&self, tx: &mut Transaction, new: &NewRecord) -> Result<i64> {
        (self.create)(tx, new).await
    }

    /// Adds this entry to the process-wide registry, replacing one of the
    /// same subject type.
    pub fn register(self) {
        register(self);
    }
}

static REGISTRY: RwLock<BTreeMap<String, Arc<LateEntryDef>>> = RwLock::new(BTreeMap::new());

/// Adds `def` to the process-wide registry; registering the same subject
/// type again replaces the entry.
pub fn register(def: LateEntryDef) {
    REGISTRY
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(def.subject_type.clone(), Arc::new(def));
}

/// The entry of `subject_type`, if registered.
pub fn get(subject_type: &str) -> Option<Arc<LateEntryDef>> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(subject_type)
        .cloned()
}

/// The registered subject types, sorted.
pub fn subject_types() -> Vec<String> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .keys()
        .cloned()
        .collect()
}

/// The entry of `subject_type`, or a 400 saying it does not opt in.
pub fn require(subject_type: &str) -> Result<Arc<LateEntryDef>> {
    get(subject_type).ok_or_else(|| {
        Error::BadRequest(crate::texts::en("wl_kernel.late_entries.not_opted_in"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(subject_type: &str) -> LateEntryDef {
        LateEntryDef::of(subject_type, |_tx, _new| Box::pin(async { Ok(1) }))
    }

    #[test]
    fn the_registry_finds_an_entry_by_subject_type_and_replaces_it() {
        def("tests.late_a").register();
        let entry = get("tests.late_a").expect("registered");
        assert_eq!(entry.subject_type(), "tests.late_a");
        assert_eq!(entry.business_time_field(), None);
        assert_eq!(entry.parent_field(), None);

        def("tests.late_a")
            .business_time("checked_in_at")
            .parent("booking")
            .register();
        let entry = get("tests.late_a").expect("registered");
        assert_eq!(entry.business_time_field(), Some("checked_in_at"));
        assert_eq!(entry.parent_field(), Some("booking"));
        assert!(subject_types().contains(&"tests.late_a".to_owned()));
        assert!(get("tests.late_unknown").is_none());
        assert!(require("tests.late_unknown").is_err());
    }

    #[test]
    #[should_panic(expected = "not a subject type")]
    fn a_bad_subject_type_panics() {
        let _ = def("late");
    }
}
