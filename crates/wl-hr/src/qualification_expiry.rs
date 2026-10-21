//! `HRM.RUL.10` (wl-hr §5.5, §6 *Automations*, issue #191): a qualification
//! or employee document of a blocking kind reaches its expiry date.
//!
//! wl-documents emits `DocumentExpired` on the first nightly run after the
//! expiry date (00:00 Asia/Dubai the day after: the document is valid *on*
//! its expiry date). That event is the one source of truth: the guard
//! [`qualification_valid`](crate::qualification::qualification_valid) asks
//! `document_valid`, so it answers *no* from that day on, and this module only
//! reacts:
//!
//! - [`on_expired`] emits [`QualificationExpired`] with the open and planned
//!   work the document event carries (wl-hr collects none itself) and tells
//!   the employee's manager (every holder of the manager role in the
//!   employee's scopes) and HR; with no manager only HR is told. It runs once
//!   per qualification and expiry date: [`QualificationExpiry`] remembers it.
//!   An employee document of the kind with no qualification record gives one
//!   event with no qualification.
//! - [`on_expiring`] emits [`QualificationExpiring`] with the `affected` list
//!   of `DocumentExpiring` unchanged.
//!
//! A renewal that is accepted makes the guard answer *yes* again; no event
//! other than `DocumentAccepted` announces it (decision: spec silent).

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::documents::{DocumentExpired, DocumentExpiring};
use wl_kernel::workforce::{
    AffectedWork, EMPLOYEE_SUBJECT_TYPE, QualificationExpired, QualificationExpiring,
};

use crate::assignees;
use crate::employee::Employee;
use crate::notices;
use crate::qualification::Qualification;
use crate::template::{OWNER_HR, OWNER_MANAGER};

/// A `QualificationExpired` that was emitted: one row per qualification and
/// expiry date, so a second `DocumentExpired` for it emits nothing.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_hr_qualification_expiries",
    unique(employee_id, kind, qualification_id, expires_on)
)]
pub struct QualificationExpiry {
    /// The primary key.
    pub id: i64,
    /// The employee.
    #[model(references = "wl_hr_employees")]
    pub employee_id: i64,
    /// The document kind.
    pub kind: String,
    /// The qualification record; 0 for none.
    pub qualification_id: i64,
    /// The expiry date.
    pub expires_on: NaiveDate,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The employee a document event is about, when its subject is one.
async fn employee_of(state: &AppState, subject: &Subject) -> Result<Option<Employee>> {
    if subject.subject_type != EMPLOYEE_SUBJECT_TYPE {
        return Ok(None);
    }
    Employee::find(&state.db, subject.subject_id).await
}

/// The scopes of `rows`, without repeats.
fn scopes_of(rows: &[Qualification]) -> Vec<Subject> {
    let mut scopes: Vec<Subject> = Vec::new();
    for scope in rows.iter().flat_map(|q| q.scopes.iter()) {
        if !scopes.contains(scope) {
            scopes.push(scope.clone());
        }
    }
    scopes
}

/// The manager and HR of `employee`: the managers of the employee's scopes
/// and the HR holders, once each.
pub async fn recipients(state: &AppState, employee: &Employee) -> Result<Vec<i64>> {
    let scopes = employee.scopes(&state.db).await?;
    let mut users = assignees::resolve(state, employee, &scopes, OWNER_MANAGER).await?;
    users.extend(assignees::resolve(state, employee, &scopes, OWNER_HR).await?);
    users.sort_unstable();
    users.dedup();
    Ok(users)
}

/// `DocumentExpired`: emits `QualificationExpired` once per qualification and
/// expiry date, and tells the manager and HR.
pub(crate) async fn on_expired(state: &AppState, event: &DocumentExpired) -> Result {
    let Some(employee) = employee_of(state, &event.subject).await? else {
        return Ok(());
    };
    let affected = AffectedWork::blocked_in(&event.blocked);
    let rows = Qualification::of_kind(&state.db, employee.id, &event.kind).await?;
    let targets: Vec<Option<Qualification>> = if rows.is_empty() {
        vec![None]
    } else {
        rows.into_iter().map(Some).collect()
    };
    for qualification in targets {
        let qualification_id = qualification.as_ref().map_or(0, |q| q.id);
        let done = QualificationExpiry::query()
            .where_(QualificationExpiry::EMPLOYEE_ID.eq(employee.id))
            .where_(QualificationExpiry::KIND.eq(event.kind.as_str()))
            .where_(QualificationExpiry::QUALIFICATION_ID.eq(qualification_id))
            .where_(QualificationExpiry::EXPIRES_ON.eq(event.expires_on))
            .first(&state.db)
            .await?;
        if done.is_some() {
            continue;
        }
        QualificationExpiry::create(
            &state.db,
            QualificationExpiry {
                employee_id: employee.id,
                kind: event.kind.clone(),
                qualification_id,
                expires_on: event.expires_on,
                ..QualificationExpiry::default()
            },
        )
        .await?;
        let (scope, roles) = match &qualification {
            Some(q) => (q.scopes.0.clone(), q.roles.0.clone()),
            None => (Vec::new(), Vec::new()),
        };
        state
            .emit(QualificationExpired {
                employee: employee.employee_id(),
                qualification: qualification.as_ref().map(Qualification::qualification_id),
                kind: event.kind.clone(),
                scope,
                roles,
                expires_on: event.expires_on,
                affected: affected.clone(),
            })
            .await?;
        let users = recipients(state, &employee).await?;
        notices::qualification_expired(
            state,
            &employee,
            &event.kind,
            event.expires_on,
            &affected,
            &users,
        )
        .await?;
    }
    Ok(())
}

/// `DocumentExpiring`: emits `QualificationExpiring` with the `affected` list
/// of the document event unchanged.
pub(crate) async fn on_expiring(state: &AppState, event: &DocumentExpiring) -> Result {
    let Some(employee) = employee_of(state, &event.subject).await? else {
        return Ok(());
    };
    let rows = Qualification::of_kind(&state.db, employee.id, &event.kind).await?;
    state
        .emit(QualificationExpiring {
            employee: employee.employee_id(),
            qualification: rows.first().map(Qualification::qualification_id),
            kind: event.kind.clone(),
            scope: scopes_of(&rows),
            expires_on: event.expires_on,
            days_left: event.days_left,
            affected: AffectedWork::planned_in(&event.affected),
        })
        .await
}
