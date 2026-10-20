//! The back-office pages of the HR module, each behind its permission.

pub mod employees;
pub mod templates;

use renox::prelude::*;

/// The module's routes.
pub fn routes() -> Routes {
    Routes::new()
        .merge(employees::routes())
        .merge(templates::routes())
}
