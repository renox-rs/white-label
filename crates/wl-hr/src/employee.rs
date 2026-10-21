//! The employee (wl-hr §3 *Employee*, §4 *Employee*, issue #216): the minimal
//! record onboarding runs on: the person, the employing entity, the job role
//! or trade, the scopes they work in, the employment type, the hire date and
//! the cost rate. Payroll, visas, leave and settlements are other issues.
//!
//! The status moves only through the machine ([`crate::employee_machine`]).
//! A new record is created in `onboarding` when an onboarding template
//! applies to it, else directly `active`; an import can ask for `active`
//! outright, which starts no onboarding ([`create`]).

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::workforce::{EmployeeAssignmentChanged, EmployeeId};
use wl_kernel::{PartyId, Subject};

use crate::onboarding;
use crate::template::OnboardingTemplate;

/// Where an employee stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EmployeeStatus {
    /// Hired; visa, contract and documents in progress.
    #[default]
    Onboarding,
    /// Can work and be paid.
    Active,
    /// On approved leave.
    OnLeave,
    /// Resigned or terminated; serving notice.
    Notice,
    /// Absent without notice; MoHRE absence report filed.
    Absconded,
    /// Died in service.
    Deceased,
    /// Settled (terminal).
    Exited,
    /// Never started work (terminal).
    OnboardingCancelled,
}

impl EmployeeStatus {
    /// The states in which an employee works: `onboarding`, `active`,
    /// `on_leave` and `notice`.
    pub const WORKING: [EmployeeStatus; 4] = [
        EmployeeStatus::Onboarding,
        EmployeeStatus::Active,
        EmployeeStatus::OnLeave,
        EmployeeStatus::Notice,
    ];
}

/// How an employee is employed.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EmploymentType {
    /// Full-time.
    #[default]
    FullTime,
    /// Part-time.
    PartTime,
    /// Temporary.
    Temporary,
    /// A freelancer: no payroll.
    Freelance,
}

/// A person who works for the client company.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_hr_employees",
    state_machine,
    index(party_id),
    index(job_role, status)
)]
pub struct Employee {
    /// The primary key.
    pub id: i64,
    /// The person (a wl-parties party).
    pub party_id: i64,
    /// The account the employee signs in with, once there is one.
    pub user_id: Option<i64>,
    /// The employing legal entity.
    pub legal_entity_id: Option<i64>,
    /// The employee number.
    pub employee_number: String,
    /// The job title.
    pub job_title: String,
    /// The job role or trade key, lower case.
    pub job_role: String,
    /// The department.
    pub department: Option<String>,
    /// How the employee is employed.
    pub employment_type: EmploymentType,
    /// The hire date.
    pub hire_date: NaiveDate,
    /// The cost rate per hour, AED fils.
    pub cost_rate_per_hour: Option<i64>,
    /// Where the employee stands; changed only by the machine.
    pub status: EmployeeStatus,
    /// The last working day while the employee serves notice; cleared when
    /// the notice is withdrawn or the employee is reinstated.
    pub last_working_day: Option<NaiveDate>,
    /// How the employment ended: `onboarding_cancelled`, `absconding`, ...
    pub exit_route: Option<String>,
    /// When the portal account was closed (`cancel_onboarding`).
    pub portal_closed_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A scope (branch) an employee works in.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_hr_employee_scopes",
    unique(employee_id, scope_type, scope_id)
)]
pub struct EmployeeScope {
    /// The primary key.
    pub id: i64,
    /// The employee.
    #[model(references = "wl_hr_employees")]
    pub employee_id: i64,
    /// The scope's type.
    pub scope_type: String,
    /// The scope's id.
    pub scope_id: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl EmployeeScope {
    /// The scope as a [`Subject`], when its id is a number.
    pub fn subject(&self) -> Option<Subject> {
        self.scope_id
            .parse()
            .ok()
            .map(|id| Subject::new(&self.scope_type, id))
    }
}

impl Employee {
    /// The employee as an id.
    pub fn employee_id(&self) -> EmployeeId {
        EmployeeId(self.id)
    }

    /// The employee as a kernel subject.
    pub fn subject(&self) -> Subject {
        self.employee_id().subject()
    }

    /// The person.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// The scopes the employee works in, oldest first.
    pub async fn scopes(&self, db: &Db) -> Result<Vec<Subject>> {
        Ok(EmployeeScope::query()
            .where_(EmployeeScope::EMPLOYEE_ID.eq(self.id))
            .order_by(EmployeeScope::ID)
            .get(db)
            .await?
            .iter()
            .filter_map(EmployeeScope::subject)
            .collect())
    }
}

/// What HR enters for a new employee.
#[derive(Debug, Clone)]
pub struct NewEmployee {
    /// The person.
    pub party: PartyId,
    /// The account, if there is one.
    pub user_id: Option<i64>,
    /// The employing legal entity.
    pub legal_entity_id: Option<i64>,
    /// The employee number.
    pub employee_number: String,
    /// The job title.
    pub job_title: String,
    /// The job role or trade key.
    pub job_role: String,
    /// The employment type.
    pub employment_type: EmploymentType,
    /// The hire date.
    pub hire_date: NaiveDate,
    /// The scopes (branches) the employee works in.
    pub scopes: Vec<Subject>,
    /// The cost rate per hour, AED fils.
    pub cost_rate_per_hour: Option<i64>,
    /// The state to create the record in: `None` for `onboarding` when a
    /// template applies, else `active`; `Some(Active)` for an import, which
    /// starts no onboarding.
    pub status: Option<EmployeeStatus>,
}

impl NewEmployee {
    /// A full-time employee hired on `hire_date` in `job_role`.
    pub fn new(party: PartyId, job_role: &str, hire_date: NaiveDate) -> Self {
        NewEmployee {
            party,
            user_id: None,
            legal_entity_id: None,
            employee_number: String::new(),
            job_title: String::new(),
            job_role: job_role.to_owned(),
            employment_type: EmploymentType::FullTime,
            hire_date,
            scopes: Vec::new(),
            cost_rate_per_hour: None,
            status: None,
        }
    }

    /// Working in `scope`.
    pub fn scope(mut self, scope: Subject) -> Self {
        self.scopes.push(scope);
        self
    }

    /// With the account `user`.
    pub fn user(mut self, user: i64) -> Self {
        self.user_id = Some(user);
        self
    }

    /// Created in `status`.
    pub fn status(mut self, status: EmployeeStatus) -> Self {
        self.status = Some(status);
        self
    }

    /// With the employment type.
    pub fn employment_type(mut self, employment_type: EmploymentType) -> Self {
        self.employment_type = employment_type;
        self
    }

    /// With the job title.
    pub fn job_title(mut self, title: &str) -> Self {
        self.job_title = title.to_owned();
        self
    }
}

/// Creates an employee and, in `onboarding`, starts the onboarding from the
/// templates that apply (`HRM.RUL.16`, [`onboarding::start_on_hire`]).
///
/// The record is created in `onboarding` when `status` asks for it or, with
/// no `status`, when an active template applies to its job role (or trade)
/// and employment type; else `active`. A record created `active` (an import)
/// starts no onboarding. The job role is stored lower case.
///
/// # Errors
///
/// The database's errors; `Status(422)` for a blank job role or a state other
/// than `onboarding` and `active`.
pub async fn create(state: &AppState, input: NewEmployee) -> Result<Employee> {
    let job_role = input.job_role.trim().to_lowercase();
    if job_role.is_empty() {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            crate::texts::english("wl_hr.employees.job_role_required", &[]),
        ));
    }
    let initial = match input.status {
        Some(EmployeeStatus::Onboarding) => EmployeeStatus::Onboarding,
        Some(EmployeeStatus::Active) => EmployeeStatus::Active,
        Some(_) => {
            return Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                crate::texts::english("wl_hr.employees.bad_initial_state", &[]),
            ));
        }
        None => {
            let applies =
                !OnboardingTemplate::applicable(&state.db, &job_role, input.employment_type)
                    .await?
                    .is_empty();
            if applies {
                EmployeeStatus::Onboarding
            } else {
                EmployeeStatus::Active
            }
        }
    };
    let mut employee = Employee {
        party_id: input.party.0,
        user_id: input.user_id,
        legal_entity_id: input.legal_entity_id,
        employee_number: input.employee_number.trim().to_owned(),
        job_title: input.job_title.trim().to_owned(),
        job_role,
        employment_type: input.employment_type,
        hire_date: input.hire_date,
        cost_rate_per_hour: input.cost_rate_per_hour,
        status: initial,
        ..Employee::default()
    };
    let mut tx = state.db.begin().await?;
    employee.insert(&mut tx).await?;
    for scope in &input.scopes {
        EmployeeScope::create(
            &mut tx,
            EmployeeScope {
                employee_id: employee.id,
                scope_type: scope.subject_type.clone(),
                scope_id: scope.subject_id.to_string(),
                ..EmployeeScope::default()
            },
        )
        .await?;
    }
    tx.commit().await?;
    if initial == EmployeeStatus::Onboarding {
        onboarding::start_on_hire(state, &employee).await?;
    }
    Employee::find_or_404(&state.db, employee.id).await
}

/// Corrects the hire date of `employee`; the due dates of its open onboarding
/// steps are recomputed from the new date.
pub async fn correct_hire_date(
    state: &AppState,
    id: i64,
    hire_date: NaiveDate,
) -> Result<Employee> {
    let employee = Employee::find_or_404(&state.db, id).await?;
    if employee.hire_date == hire_date {
        return Ok(employee);
    }
    Employee::query()
        .where_(Employee::ID.eq(id))
        .update(
            &state.db,
            &[("hire_date", &hire_date), ("updated_at", &renox::db::now())],
        )
        .await?;
    onboarding::recompute_due_dates(state, id, hire_date).await?;
    Employee::find_or_404(&state.db, id).await
}

/// Changes the job role of `employee` to `job_role`; emits
/// [`EmployeeAssignmentChanged`] and re-resolves the assignees of its open
/// onboarding steps. The steps of the old role's template stay (HR waives
/// those that no longer apply and starts a manual onboarding for the new
/// role).
pub async fn change_job_role(state: &AppState, id: i64, job_role: &str) -> Result<Employee> {
    let employee = Employee::find_or_404(&state.db, id).await?;
    let job_role = job_role.trim().to_lowercase();
    if job_role.is_empty() || job_role == employee.job_role {
        return Ok(employee);
    }
    Employee::query()
        .where_(Employee::ID.eq(id))
        .update(
            &state.db,
            &[("job_role", &job_role), ("updated_at", &renox::db::now())],
        )
        .await?;
    let scope = employee.scopes(&state.db).await?.into_iter().next();
    state
        .emit(EmployeeAssignmentChanged {
            employee: employee.employee_id(),
            job_role: job_role.clone(),
            scope,
            from: Some(employee.job_role.clone()),
            to: Some(job_role),
        })
        .await?;
    Employee::find_or_404(&state.db, id).await
}

/// Adds `scope` to the scopes of the employee, or ends it (`add` false);
/// emits [`EmployeeAssignmentChanged`], which re-resolves the assignees of
/// the open onboarding steps.
pub async fn set_scope(state: &AppState, id: i64, scope: &Subject, add: bool) -> Result<()> {
    let employee = Employee::find_or_404(&state.db, id).await?;
    let present = EmployeeScope::query()
        .where_(EmployeeScope::EMPLOYEE_ID.eq(id))
        .where_(EmployeeScope::SCOPE_TYPE.eq(scope.subject_type.clone()))
        .where_(EmployeeScope::SCOPE_ID.eq(scope.subject_id.to_string()))
        .first(&state.db)
        .await?;
    match (present, add) {
        (None, true) => {
            EmployeeScope::create(
                &state.db,
                EmployeeScope {
                    employee_id: id,
                    scope_type: scope.subject_type.clone(),
                    scope_id: scope.subject_id.to_string(),
                    ..EmployeeScope::default()
                },
            )
            .await?;
        }
        (Some(mut row), false) => {
            row.delete(&state.db).await?;
        }
        _ => return Ok(()),
    }
    let (from, to) = if add {
        (None, Some(scope.to_string()))
    } else {
        (Some(scope.to_string()), None)
    };
    state
        .emit(EmployeeAssignmentChanged {
            employee: employee.employee_id(),
            job_role: employee.job_role.clone(),
            scope: Some(scope.clone()),
            from,
            to,
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_states_are_the_four_of_the_spec() {
        assert_eq!(
            EmployeeStatus::WORKING,
            [
                EmployeeStatus::Onboarding,
                EmployeeStatus::Active,
                EmployeeStatus::OnLeave,
                EmployeeStatus::Notice
            ]
        );
    }

    #[test]
    fn the_builder_sets_the_parts() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        let new = NewEmployee::new(PartyId(4), "Driver", day)
            .scope(Subject::new("property.site", 2))
            .user(9)
            .status(EmployeeStatus::Onboarding);
        assert_eq!(new.scopes, [Subject::new("property.site", 2)]);
        assert_eq!(new.user_id, Some(9));
        assert_eq!(new.status, Some(EmployeeStatus::Onboarding));
        assert_eq!(new.employment_type, EmploymentType::FullTime);
    }
}
