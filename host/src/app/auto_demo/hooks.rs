//! The one route the seed uses to be the demo users: the services of the
//! app take the signed-in user (`&AuthUser`), which only a request can build.
//! The seed posts to it with a user's token and a key of its own and takes the
//! user back with [`take`]. It answers only in `local` and `testing`, so a
//! server in production has it as a 404.

use std::collections::BTreeMap;
use std::sync::Mutex;

use renox::Environment;
use renox::prelude::*;

static SEEN: Mutex<BTreeMap<String, AuthUser>> = Mutex::new(BTreeMap::new());

#[derive(serde::Deserialize)]
struct Key {
    key: String,
}

async fn whoami(
    State(state): State<AppState>,
    user: AuthUser,
    Form(form): Form<Key>,
) -> Result<&'static str> {
    if !matches!(state.config.env, Environment::Local | Environment::Testing) {
        return Err(Error::NotFound);
    }
    SEEN.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(form.key, user);
    Ok("ok")
}

/// The route group of the module.
pub(crate) fn routes() -> Routes {
    Routes::new()
        .post("/demo/auto/whoami", whoami)
        .require_auth()
}

/// The user the request with this key was made by.
pub(crate) fn take(key: &str) -> Option<AuthUser> {
    SEEN.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(key)
}
