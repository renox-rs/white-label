//! `/menu`: every link the signed-in person may open, as a page. On a phone
//! the sidebar is gone and the tab bar holds five places; the links the
//! modules add to the sidebar (the contracts, the employees, the payroll...)
//! are reached from here.

use renox::prelude::*;

pub struct Menu;

impl Module for Menu {
    fn name(&self) -> &'static str {
        "menu"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .get("/menu", index)
            .name("menu")
            .require_auth()
            .require_verified()
    }
}

async fn index() -> Response {
    view("menu/index.html", context! {}).into_response()
}
