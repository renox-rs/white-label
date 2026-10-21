//! The back-office pages of the HR module, each behind its permission.

pub mod employees;
pub mod pay_accounts;
pub mod payroll;
pub mod settlements;
pub mod templates;
pub mod visas;

use renox::prelude::*;

/// The module's routes.
pub fn routes() -> Routes {
    Routes::new()
        .merge(employees::routes())
        .merge(pay_accounts::routes())
        .merge(payroll::routes())
        .merge(settlements::routes())
        .merge(templates::routes())
        .merge(visas::routes())
}
