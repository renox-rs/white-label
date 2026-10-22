//! The bench (09 Manpower §4.1 row *The contract expires, or ends early*,
//! §7.1 *Client contract* `end`, *Deployment* `end`; BL-MPW-06): the employees
//! who are `active` in HR but have no deployment on site.
//!
//! A worker is *on the bench* when HR says the employee is `active` (not in
//! `notice`, not gone; the kernel's `Workforce::is_active`) and no deployment
//! of the employee is `active` or `suspended`. A planned deployment does not
//! take the worker off the bench: the worker is not on site yet.

use renox::prelude::*;
use wl_kernel::workforce::{EmployeeId, workforce};

use crate::deployment::{Deployment, DeploymentStatus};

/// Whether `employee` is on the bench now.
///
/// # Errors
///
/// The database's and the workforce contract's errors.
pub async fn is_on_bench(state: &AppState, employee: EmployeeId) -> Result<bool> {
    if !workforce(state).is_active(state, employee).await? {
        return Ok(false);
    }
    let on_site = Deployment::query()
        .where_(Deployment::EMPLOYEE_ID.eq(employee.0))
        .where_(Deployment::STATUS.is_in([DeploymentStatus::Active, DeploymentStatus::Suspended]))
        .count(&state.db)
        .await?;
    Ok(on_site == 0)
}

/// The employees of `candidates` who are on the bench now, each once, in order.
///
/// # Errors
///
/// As [`is_on_bench`].
pub async fn of(state: &AppState, candidates: &[EmployeeId]) -> Result<Vec<EmployeeId>> {
    let mut bench = Vec::new();
    for employee in candidates {
        if !bench.contains(employee) && is_on_bench(state, *employee).await? {
            bench.push(*employee);
        }
    }
    Ok(bench)
}
