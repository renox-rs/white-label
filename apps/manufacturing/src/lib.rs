#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod abort;
pub mod cost;
mod db;
pub mod events;
pub mod issue;
pub mod notices;
pub mod pages;
pub mod permissions;
pub mod pick_list;
pub mod plant;
pub mod report;
mod screens;
mod texts;
pub mod unlink;
pub mod work_order;
pub mod work_orders;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use abort::AbortLine;
pub use events::{
    WorkOrderAborted, WorkOrderCancelled, WorkOrderCompleted, WorkOrderReleased, WorkOrderUnlinked,
};
pub use issue::MaterialIssue;
pub use pick_list::PickLine;
pub use plant::{Line, Plant};
pub use report::{ProductionReport, ReportStatus};
pub use work_order::{WorkOrder, WorkOrderSource, WorkOrderStatus};

/// The name of the scheduled task that posts the movements of an abort that
/// did not post (every five minutes).
pub const ABORT_SWEEP_TASK: &str = "manufacturing:abort-sweep";

/// The Manufacturing module. Add it to the app with
/// `.module(manufacturing::Manufacturing)`.
pub struct Manufacturing;

impl Module for Manufacturing {
    fn name(&self) -> &'static str {
        "manufacturing"
    }

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<screens::work_orders_page::WorkOrdersPage>()
            .view_data::<screens::work_order_page::WorkOrderPage>();
        // An approval of a work order links to its page.
        wl_kernel::approvals::register_subject_link(work_order::SUBJECT_TYPE, |id| {
            screens::work_order_page::url(id)
        });
        app.templates(|env| {
            for (name, source) in screens::VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        app.nav(
            "app",
            NavLink::route("manufacturing.work_orders", "nav.work_orders")
                .icon("factory")
                .group("nav.app")
                .order(9)
                .permission(permissions::WORK_ORDERS_VIEW),
        );
        app.model::<Plant>()
            .model::<Line>()
            .model::<WorkOrder>()
            .model::<PickLine>()
            .model::<MaterialIssue>()
            .model::<ProductionReport>()
            .model::<AbortLine>();
        work_order::register();
        report::register(app);
        notices::register_types();
        // `MFG.ACL.12`: the posted quantities still match the request at the
        // final approval.
        wl_kernel::approvals::register_final_check(work_order::SUBJECT_TYPE, |tx, request| {
            Box::pin(abort::recheck(tx, request))
        });
        abort::listen(app);
        app.schedule()
            .every_minutes(5, ABORT_SWEEP_TASK, |state: AppState| async move {
                abort::sweep(&state).await?;
                report::sweep(&state).await.map(|_| ())
            });
    }
}
