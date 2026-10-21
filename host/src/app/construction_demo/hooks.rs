//! Routes the seed uses where a transition needs the signed-in user and no
//! page of the vertical calls it yet: the project's `start` (the setup screen
//! comes with the BOQ issue). They answer only in `local` and `testing`, and
//! only to a signed-in user who may do what the transition asks (the same
//! permission, guards and history as any other run), so a server in
//! production has them as 404s.

use renox::Environment;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};

/// The module with the seed's routes.
pub struct Hooks;

#[derive(serde::Deserialize)]
struct ReasonForm {
    reason: Option<String>,
}

fn allowed(state: &AppState) -> Result {
    if matches!(state.config.env, Environment::Local | Environment::Testing) {
        Ok(())
    } else {
        Err(Error::NotFound)
    }
}

/// `POST /demo/projects/{id}/{transition}`: runs the project's transition
/// `name` (`start`, `hold`, `resume`) as the signed-in user, with the form's
/// optional `reason`.
async fn project_transition(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, name)): Path<(i64, String)>,
    Form(form): Form<ReasonForm>,
) -> Result<String> {
    allowed(&state)?;
    let project = construction::Project::find_or_404(&state.db, id).await?;
    let mut run = project.transition(&name).by(&user);
    if let Some(reason) = form.reason.as_deref().map(str::trim)
        && !reason.is_empty()
    {
        run = run.reason(reason);
    }
    // `run` commits and emits the transition's event.
    let _outcome = run.run(&state).await.map_err(TransitionError::into_error)?;
    Ok(name)
}

impl Module for Hooks {
    fn name(&self) -> &'static str {
        "construction_demo_hooks"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .post("/demo/projects/{id}/{transition}", project_transition)
            .require_auth()
    }
}
