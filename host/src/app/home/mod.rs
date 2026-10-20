//! `/`: the app has no public front page of its own. Signed-in people go to
//! the dashboard, everyone else to the sign-in page (users are added by an
//! admin; there is no sign-up).

use renox::prelude::*;

pub struct Home;

impl Module for Home {
    fn name(&self) -> &'static str {
        "home"
    }

    fn routes(&self) -> Routes {
        Routes::new().get("/", index).name("home")
    }
}

async fn index(user: Option<AuthUser>) -> Redirect {
    Redirect::to(if user.is_some() {
        "/dashboard"
    } else {
        "/login"
    })
}
