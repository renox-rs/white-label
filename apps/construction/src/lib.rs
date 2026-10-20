#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod award;
pub mod call;
pub mod cancellation;
pub mod certificate;
pub mod completion;
pub mod cutoff;
pub mod deduction;
pub mod events;
pub mod expiry;
pub mod final_account;
pub mod guarantee;
pub mod invoice;
pub mod ipa;
pub mod notices;
pub mod pages;
pub mod permissions;
pub mod project;
pub mod release;
pub mod retention;
pub mod retention_deduction;
pub mod submission;
mod tasks;
pub mod termination;
mod texts;
pub mod variation;
mod variation_pages;
pub mod vat_notice;

use renox::db::Migration;
use renox::prelude::*;
use renox::state_machine::Transitioned;

pub use call::GuaranteeCall;
pub use deduction::{DeductionKind, DeductionStatus, IpaDeduction};
pub use events::ProjectTerminated;
pub use guarantee::{Guarantee, GuaranteeDocument, GuaranteeKind, GuaranteeStatus};
pub use ipa::{Ipa, IpaKind, IpaStatus};
pub use project::{Project, ProjectStatus};
pub use retention::{Retention, RetentionPart, RetentionStatus};
pub use retention_deduction::RetentionDeduction;
pub use termination::{TerminateRequest, TerminationReason};
pub use variation::{Variation, VariationKind, VariationStatus};
pub use vat_notice::RetentionNotice;

/// The name of the daily scheduled task of the retention (`CON.RUL.07`).
pub const RETENTION_TASK: &str = "construction:retention-daily";

/// The name of the daily scheduled task of the guarantees (`CON.RUL.11`).
pub const GUARANTEE_TASK: &str = "construction:guarantee-daily";

/// The Construction module. Add it to the app with
/// `.module(construction::Construction)`.
pub struct Construction;

impl Module for Construction {
    fn name(&self) -> &'static str {
        "construction"
    }

    fn routes(&self) -> Routes {
        pages::routes().merge(variation_pages::routes())
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Project>()
            .model::<Ipa>()
            .model::<IpaDeduction>()
            .model::<Retention>()
            .model::<RetentionNotice>()
            .model::<RetentionDeduction>()
            .model::<Guarantee>()
            .model::<GuaranteeDocument>()
            .model::<GuaranteeCall>()
            .model::<Variation>();
        project::register();
        variation::register();
        variation::listen(app);
        renox::state_machine::register::<Ipa>();
        renox::state_machine::register::<Retention>();
        renox::state_machine::register::<Guarantee>();
        award::register_block_effect();
        deduction::register();
        retention_deduction::register();
        retention_deduction::listen(app);
        // A guarantee never expires silently, and `CON.RUL.11` asks the bank
        // for an extension 60 days before: daily at 02:00 Asia/Dubai.
        expiry::listen(app);
        // A call on a guarantee is announced at once and its payment booked.
        call::listen(app);
        app.schedule()
            .daily_at("02:00", GUARANTEE_TASK, |state: AppState| async move {
                expiry::daily(&state).await
            })
            .timezone("Asia/Dubai");
        submission::register_types();
        cutoff::schedule(app);
        termination::register_types();
        notices::register_types();
        // A retention part follows its invoice (`settle`).
        release::listen(app);
        // `CON.RUL.07` and the final retention part at the end of the
        // defects liability period, daily at 02:00 Asia/Dubai.
        app.schedule()
            .daily_at("02:00", RETENTION_TASK, |state: AppState| async move {
                release::daily(&state).await
            })
            .timezone("Asia/Dubai");
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
