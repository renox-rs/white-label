#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod absence;
pub mod award;
pub mod boq;
pub mod call;
pub mod cancellation;
pub mod certificate;
pub mod chase;
pub mod completion;
pub mod cutoff;
pub mod deduction;
pub mod events;
pub mod expiry;
pub mod final_account;
pub mod guarantee;
pub mod invoice;
pub mod ipa;
pub mod notice_deadline;
pub mod notices;
pub mod omission;
pub mod pages;
pub mod permissions;
pub mod project;
pub mod registration;
pub mod release;
pub mod retention;
pub mod retention_deduction;
mod screen_texts;
mod screens;
pub mod seed;
pub mod submission;
mod tasks;
pub mod termination;
mod texts;
pub mod variation;
mod variation_pages;
pub mod vat_notice;
pub mod watch;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;
use renox::state_machine::Transitioned;

pub use absence::AbsenceCase;
pub use boq::{BoqLine, IpaProgress};
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

/// The name of the daily scheduled task of the unanswered variations
/// (`CON.RUL.09`).
pub const VARIATION_CHASE_TASK: &str = "construction:variation-chase-daily";

/// The name of the daily scheduled task of the notice deadlines
/// (`CON.RUL.10`).
pub const NOTICE_DEADLINE_TASK: &str = "construction:notice-deadline-daily";

/// The name of the daily scheduled task of the absence cases (`CON.RUL.08`).
pub const ABSENCE_TASK: &str = "construction:absence-daily";

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
        app.view_data::<screens::projects::ProjectsPage>()
            .view_data::<screens::ipa::IpaPage>()
            .view_data::<screens::closing::ClosingPage>()
            .view_data::<screens::variations::VariationReportPage>();
        app.templates(|env| {
            for (name, source) in screens::VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        // The projects list in the app group of the sidebar; the label is the
        // app's `nav.projects` text.
        app.nav(
            "app",
            NavLink::route("construction.projects", "nav.projects")
                .icon("hard-hat")
                .group("nav.app")
                .order(9)
                .permission(permissions::PROJECTS_VIEW),
        );
        app.model::<Project>()
            .model::<Ipa>()
            .model::<IpaDeduction>()
            .model::<Retention>()
            .model::<RetentionNotice>()
            .model::<RetentionDeduction>()
            .model::<Guarantee>()
            .model::<GuaranteeDocument>()
            .model::<GuaranteeCall>()
            .model::<Variation>()
            .model::<BoqLine>()
            .model::<IpaProgress>()
            .model::<AbsenceCase>();
        project::register();
        variation::register();
        variation::listen(app);
        renox::state_machine::register::<Ipa>();
        renox::state_machine::register::<Retention>();
        renox::state_machine::register::<Guarantee>();
        // `CON.ACL.13`: a purchase order to a subcontractor without a valid
        // registration on a Dubai project waits for the project manager.
        registration::register();
        wl_kernel::inventory::provide_order_rules(app, registration::RegistrationRule);
        award::register_block_effect();
        // The labour rules the owner is alerted about (BL-CON-18).
        seed::register_critical_rules();
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
        // `CON.RUL.09`: the project manager is reminded to chase a variation
        // the client does not answer after 14 days, the MD told at 30.
        app.schedule()
            .daily_at(
                "02:00",
                VARIATION_CHASE_TASK,
                |state: AppState| async move { chase::daily(&state).await },
            )
            .timezone("Asia/Dubai");
        // `CON.RUL.10`: the project manager and QS are warned 7 and 2 days
        // before the contract's notice deadline of a delay event.
        app.schedule()
            .daily_at(
                "02:00",
                NOTICE_DEADLINE_TASK,
                |state: AppState| async move { notice_deadline::daily(&state).await },
            )
            .timezone("Asia/Dubai");
        // `CON.RUL.08`: a worker who stops coming gets an absence case for
        // HR after 3 working days, a reminder at 7.
        app.schedule()
            .daily_at("02:00", ABSENCE_TASK, |state: AppState| async move {
                absence::daily(&state).await
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
