//! The qualification guard (wl-hr §2 `HRM.QUA.01`, *Standard guard (X-13)*):
//! "may employee E do kind K, in role R at scope S, on date D?". Other
//! modules ask it without depending on `wl-hr`: 04 asks it before a return is
//! filed (`ACC.ACL.15`: the filer's FTA tax agent registration must be valid
//! on the filing date and linked to the firm's tax agency).
//!
//! Decision: the Qualification records and their answer (BL-HRM-01) are not
//! built in `wl-hr` yet, so this is the seam it will provide
//! ([`provide_qualifications`]). Until a module provides it,
//! [`qualifications`] gives [`NoQualifications`], whose answer is always *no*:
//! a guard that cannot find the qualification does not let the work through.
//!
//! ```ignore
//! use wl_kernel::qualifications::qualifications;
//!
//! let answer = qualifications(&state)
//!     .qualification_valid(&state.db, employee, "fta_tax_agent", on, Some("tax_agent"), Some(&agency))
//!     .await?;
//! if !answer.valid { /* refuse: the registration is missing or expired */ }
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;
use crate::workforce::EmployeeId;

/// What the guard knows about an employee's qualification of a kind.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct QualificationStatus {
    /// Whether the qualification is valid on the date asked, for the role and
    /// scope asked.
    pub valid: bool,
    /// The number of the qualification found (a registration number), if
    /// any, valid or not.
    pub number: Option<String>,
    /// The day the qualification found is valid to, if any.
    pub valid_to: Option<NaiveDate>,
}

/// The future a [`Qualifications`] method returns.
pub type QualificationFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// The standard qualification guard. `wl-hr` provides it; without it,
/// [`qualifications`] gives [`NoQualifications`].
pub trait Qualifications: Send + Sync + 'static {
    /// `qualification_valid(employee, kind, on_date[, role, scope])`: whether
    /// `employee` has a valid qualification of `kind` on `on_date` and, for a
    /// role-bound kind, one whose roles and scope include `role` and `scope`.
    /// The default answers *no* (nothing found).
    fn qualification_valid<'a>(
        &'a self,
        _db: &'a Db,
        _employee: EmployeeId,
        _kind: &'a str,
        _on_date: NaiveDate,
        _role: Option<&'a str>,
        _scope: Option<&'a Subject>,
    ) -> QualificationFuture<'a, QualificationStatus> {
        Box::pin(async { Ok(QualificationStatus::default()) })
    }
}

/// The guard when no module provides qualifications: every answer is *no*.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoQualifications;

impl Qualifications for NoQualifications {}

/// Makes `qualifications` the app's [`Qualifications`]; `wl-hr` calls it in
/// `register`.
pub fn provide_qualifications(app: &mut Registry, qualifications: impl Qualifications) {
    let qualifications: Arc<dyn Qualifications> = Arc::new(qualifications);
    app.provide(qualifications);
}

/// The app's [`Qualifications`], or [`NoQualifications`] when none is
/// provided.
pub fn qualifications(state: &AppState) -> Arc<dyn Qualifications> {
    match state.provided::<Arc<dyn Qualifications>>() {
        Some(found) => Arc::clone(&found),
        None => Arc::new(NoQualifications),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_valid_by_default() {
        let status = QualificationStatus::default();
        assert!(!status.valid);
        assert_eq!(status.number, None);
        assert_eq!(status.valid_to, None);
    }
}
