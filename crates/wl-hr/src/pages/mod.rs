//! The back-office pages of the HR module, each behind its permission.

pub mod employees;
pub mod leave;
pub mod pay_accounts;
pub mod payroll;
pub mod payroll_list;
pub mod settlement_list;
pub mod settlements;
pub mod templates;
pub mod visas;

use renox::prelude::*;

/// The module's routes.
pub fn routes() -> Routes {
    Routes::new()
        .merge(employees::routes())
        .merge(leave::routes())
        .merge(pay_accounts::routes())
        .merge(payroll::routes())
        .merge(payroll_list::routes())
        .merge(settlement_list::routes())
        .merge(settlements::routes())
        .merge(templates::routes())
        .merge(visas::routes())
}
