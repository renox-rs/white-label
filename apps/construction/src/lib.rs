#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod cancellation;
pub mod events;
pub mod final_account;
pub mod guarantee;
pub mod ipa;
pub mod pages;
pub mod permissions;
pub mod project;
pub mod retention;
mod tasks;
pub mod termination;
mod texts;

use renox::db::Migration;
use renox::prelude::*;
use renox::state_machine::Transitioned;

pub use events::ProjectTerminated;
pub use guarantee::{Guarantee, GuaranteeKind, GuaranteeStatus};
pub use ipa::{Ipa, IpaKind, IpaStatus};
pub use project::{Project, ProjectStatus};
pub use retention::{Retention, RetentionPart, RetentionStatus};
pub use termination::{TerminateRequest, TerminationReason};

/// The Construction module. Add it to the app with
/// `.module(construction::Construction)`.
pub struct Construction;

impl Module for Construction {
    fn name(&self) -> &'static str {
        "construction"
    }

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Project>()
            .model::<Ipa>()
            .model::<Retention>()
            .model::<Guarantee>();
        project::register();
        renox::state_machine::register::<Ipa>();
        renox::state_machine::register::<Retention>();
        renox::state_machine::register::<Guarantee>();
        termination::register_types();
        // The attendance module refuses a day after the last working day of a
        // terminated project (`CON.ACL.15`).
        wl_kernel::attendance::provide_context_closure(app, termination::ProjectClosure);
        // A project is the scope roles are assigned in.
        wl_kernel::scopes::register_label(project::SUBJECT_TYPE, |db, id| {
            let id = id.to_owned();
            Box::pin(async move {
                let Ok(id) = id.parse::<i64>() else {
                    return Ok(None);
                };
                Ok(Project::find(db, id).await?.map(|project| project.name))
            })
        });
        // What follows the commit of a termination, direct or approved.
        app.listen(|event: Transitioned<Project>, state: AppState| async move {
            let terminated = [project::TERMINATE, project::TERMINATE_APPROVED]
                .contains(&event.transition.as_str());
            if terminated && event.to == ProjectStatus::Terminated.as_str() {
                let _done = termination::after_termination(&state, event.key).await?;
            }
            Ok(())
        });
    }
}
