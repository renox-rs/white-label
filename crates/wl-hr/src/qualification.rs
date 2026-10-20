//! Qualifications (wl-hr §2 `HRM.QUA.01`, §3 *Qualification*, §5.5, §6
//! *Provides* `Qualifications`, issue #191): a licence, permit or certificate
//! of an employee with the roles and scopes it covers.
//!
//! - **The record.** [`Qualification`] names the document kind that proves it,
//!   the number and issuer, the roles and scopes (branches) it covers, the
//!   validity dates and the stored document. The dates are shown to staff
//!   only: the validity is the document's.
//! - **The guard (X-13).** [`qualification_valid`] answers *yes* when the
//!   employee has a valid qualification or employee document of the kind on
//!   the date, asked of wl-documents (`document_valid`), and, for a
//!   role-bound kind, when a qualification of the kind covers the role and the
//!   scope passed ([`qualification_valid_for`]). There is no second expiry
//!   check here that could disagree with `document_valid`, and the expiry
//!   date itself is valid (wl-documents: on or after the date). Past and
//!   future dates go through to `document_valid`.
//! - **Role-bound kinds.** Spec silent on how a kind is marked role-bound, so
//!   (decision) a kind is role-bound for an employee when one of their
//!   qualifications of that kind lists roles or scopes. Called without role
//!   and scope, the guard then answers *no*. An empty list of roles (or of
//!   scopes) means any role (or any scope).
//! - **Freelancers** work the same: their permit is a document kind.
//!
//! [`Staff`](crate::workforce::Staff) aside, [`Quals`] is the app's
//! [`Qualifications`] service for other modules.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Json};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::documents::guard;
use wl_kernel::qualifications::{QualificationFuture, QualificationStatus, Qualifications};
use wl_kernel::workforce::EmployeeId;

use crate::employee::Employee;

/// A licence, permit or certificate of an employee.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_hr_qualifications", index(employee_id, kind))]
pub struct Qualification {
    /// The primary key.
    pub id: i64,
    /// The employee.
    #[model(references = "wl_hr_employees")]
    pub employee_id: i64,
    /// The document kind that proves it, e.g. `practitioner_licence`.
    pub kind: String,
    /// The type as staff read it, e.g. "Practitioner licence".
    pub label: String,
    /// The number on the licence.
    pub number: String,
    /// Who issued it.
    pub issuer: String,
    /// The roles it covers, lower case; empty for any.
    pub roles: Json<Vec<String>>,
    /// The scopes (branches) it covers; empty for any.
    pub scopes: Json<Vec<Subject>>,
    /// Valid from (display only).
    pub valid_from: Option<NaiveDate>,
    /// Valid to (display only).
    pub valid_to: Option<NaiveDate>,
    /// The stored document, when there is one.
    pub document_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Qualification {
    /// The qualification as an id.
    pub fn qualification_id(&self) -> wl_kernel::workforce::QualificationId {
        wl_kernel::workforce::QualificationId(self.id)
    }

    /// Whether the validity also depends on role and scope: it lists roles
    /// or scopes.
    pub fn is_role_bound(&self) -> bool {
        !self.roles.is_empty() || !self.scopes.is_empty()
    }

    /// Whether the qualification covers work in `role` at `scope`.
    pub fn covers(&self, role: &str, scope: &Subject) -> bool {
        let role = role.trim().to_lowercase();
        (self.roles.is_empty() || self.roles.iter().any(|r| r.trim().to_lowercase() == role))
            && (self.scopes.is_empty() || self.scopes.contains(scope))
    }

    /// The qualifications of `employee` of `kind`, oldest first.
    pub async fn of_kind(db: &Db, employee: i64, kind: &str) -> Result<Vec<Qualification>> {
        Qualification::query()
            .where_(Qualification::EMPLOYEE_ID.eq(employee))
            .where_(Qualification::KIND.eq(kind))
            .order_by(Qualification::ID)
            .get(db)
            .await
    }
}

/// What HR enters for a qualification.
#[derive(Debug, Clone)]
pub struct NewQualification {
    /// The employee.
    pub employee: EmployeeId,
    /// The document kind that proves it.
    pub kind: String,
    /// The type as staff read it.
    pub label: String,
    /// The number.
    pub number: String,
    /// Who issued it.
    pub issuer: String,
    /// The roles it covers; empty for any.
    pub roles: Vec<String>,
    /// The scopes it covers; empty for any.
    pub scopes: Vec<Subject>,
    /// Valid from.
    pub valid_from: Option<NaiveDate>,
    /// Valid to.
    pub valid_to: Option<NaiveDate>,
    /// The stored document.
    pub document: Option<i64>,
}

impl NewQualification {
    /// A qualification of `kind` for `employee`, covering any role and scope.
    pub fn new(employee: EmployeeId, kind: &str) -> Self {
        NewQualification {
            employee,
            kind: kind.trim().to_owned(),
            label: String::new(),
            number: String::new(),
            issuer: String::new(),
            roles: Vec::new(),
            scopes: Vec::new(),
            valid_from: None,
            valid_to: None,
            document: None,
        }
    }

    /// Covering `role`.
    pub fn role(mut self, role: &str) -> Self {
        self.roles.push(role.trim().to_lowercase());
        self
    }

    /// Covering `scope`.
    pub fn scope(mut self, scope: Subject) -> Self {
        self.scopes.push(scope);
        self
    }

    /// With the number.
    pub fn number(mut self, number: &str) -> Self {
        self.number = number.trim().to_owned();
        self
    }

    /// Valid until `date` (display only).
    pub fn valid_to(mut self, date: NaiveDate) -> Self {
        self.valid_to = Some(date);
        self
    }
}

/// Records a qualification.
///
/// # Errors
///
/// `NotFound` for an unknown employee; `Status(422)` for a blank kind.
pub async fn record(db: &Db, input: NewQualification) -> Result<Qualification> {
    if input.kind.trim().is_empty() {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            crate::texts::english("wl_hr.qualifications.kind_required", &[]),
        ));
    }
    Employee::find_or_404(db, input.employee.0).await?;
    Qualification::create(
        db,
        Qualification {
            employee_id: input.employee.0,
            kind: input.kind.trim().to_owned(),
            label: input.label,
            number: input.number,
            issuer: input.issuer,
            roles: Json(input.roles),
            scopes: Json(input.scopes),
            valid_from: input.valid_from,
            valid_to: input.valid_to,
            document_id: input.document,
            ..Qualification::default()
        },
    )
    .await
}

/// `qualification_valid(employee, kind, on_date)`: whether the employee has
/// a valid qualification or employee document of `kind` on `on_date`. A
/// role-bound kind answers *no* (see [`qualification_valid_for`]).
pub async fn qualification_valid(
    state: &AppState,
    employee: EmployeeId,
    kind: &str,
    on_date: NaiveDate,
) -> Result<bool> {
    answer(state, employee, kind, on_date, None).await
}

/// `qualification_valid(employee, kind, on_date, role, scope)`: also the
/// qualification's roles and scope must include `role` and `scope`, the role
/// and branch of the work being guarded.
pub async fn qualification_valid_for(
    state: &AppState,
    employee: EmployeeId,
    kind: &str,
    on_date: NaiveDate,
    role: &str,
    scope: &Subject,
) -> Result<bool> {
    answer(state, employee, kind, on_date, Some((role, scope))).await
}

async fn answer(
    state: &AppState,
    employee: EmployeeId,
    kind: &str,
    on_date: NaiveDate,
    asked: Option<(&str, &Subject)>,
) -> Result<bool> {
    let rows = Qualification::of_kind(&state.db, employee.0, kind).await?;
    if rows.iter().any(Qualification::is_role_bound) {
        let Some((role, scope)) = asked else {
            return Ok(false);
        };
        if !rows.iter().any(|q| q.covers(role, scope)) {
            return Ok(false);
        }
    }
    let validity = guard(state)
        .is_valid(&state.db, &employee.subject(), kind, on_date)
        .await?;
    Ok(validity.is_valid())
}

/// The app's [`Qualifications`] service.
#[derive(Debug, Clone, Copy, Default)]
pub struct Quals;

impl Qualifications for Quals {
    fn qualification_valid<'a>(
        &'a self,
        state: &'a AppState,
        employee: EmployeeId,
        kind: &'a str,
        on_date: NaiveDate,
        role: Option<&'a str>,
        scope: Option<&'a Subject>,
    ) -> QualificationFuture<'a, QualificationStatus> {
        Box::pin(async move {
            let asked = match (role, scope) {
                (Some(role), Some(scope)) => Some((role, scope)),
                _ => None,
            };
            let valid = answer(state, employee, kind, on_date, asked).await?;
            let rows = Qualification::of_kind(&state.db, employee.0, kind).await?;
            let found = rows.iter().max_by_key(|q| q.valid_to);
            Ok(QualificationStatus {
                valid,
                number: found.map(|q| q.number.clone()),
                valid_to: found.and_then(|q| q.valid_to),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn licence(roles: &[&str], scopes: &[Subject]) -> Qualification {
        Qualification {
            roles: Json(roles.iter().map(|r| (*r).to_owned()).collect()),
            scopes: Json(scopes.to_vec()),
            ..Qualification::default()
        }
    }

    #[test]
    fn a_licence_with_roles_or_scopes_is_role_bound() {
        let branch = Subject::new("test.branch", 1);
        assert!(!licence(&[], &[]).is_role_bound());
        assert!(licence(&["practitioner"], &[]).is_role_bound());
        assert!(licence(&[], &[branch]).is_role_bound());
    }

    #[test]
    fn it_covers_the_roles_and_scopes_it_lists() {
        let one = Subject::new("test.branch", 1);
        let two = Subject::new("test.branch", 2);
        let q = licence(&["practitioner"], std::slice::from_ref(&one));
        assert!(q.covers("practitioner", &one));
        assert!(q.covers(" Practitioner ", &one));
        assert!(!q.covers("practitioner", &two));
        assert!(!q.covers("nurse", &one));
        // Empty lists cover anything.
        assert!(licence(&[], &[]).covers("nurse", &two));
        assert!(licence(&["nurse"], &[]).covers("nurse", &two));
    }
}
