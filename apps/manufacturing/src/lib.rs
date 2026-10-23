#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod abort;
pub mod conformity;
pub mod cost;
mod db;
pub mod disputes;
pub mod events;
pub mod exports;
pub mod incidents;
pub mod issue;
pub mod late_report;
pub mod material_hold;
pub mod ncr;
pub mod ncrs;
pub mod notices;
pub mod order_changes;
pub mod outcome_proposals;
pub mod pages;
pub mod permissions;
pub mod pick_list;
pub mod plant;
pub mod po_upload;
pub mod portal;
pub mod progress;
pub mod recalls;
pub mod report;
pub mod responsible;
pub mod returns;
mod screens;
mod texts;
pub mod unlink;
pub mod work_order;
pub mod work_orders;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use abort::AbortLine;
pub use conformity::ItemFlag;
pub use events::{
    NcrCancelled, NcrClosed, NcrDispositioned, NcrRaised, NcrReopened, WorkOrderAborted,
    WorkOrderCancelled, WorkOrderCompleted, WorkOrderReleased, WorkOrderUnlinked,
};
pub use issue::MaterialIssue;
pub use ncr::{Cause, Disposition, Ncr, NcrDecision, NcrStatus, Origin};
pub use pick_list::PickLine;
pub use plant::{Line, Plant};
pub use recalls::{ProductionParties, RepickFlag};
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
            .view_data::<screens::work_order_page::WorkOrderPage>()
            .view_data::<screens::ncrs_page::NcrsPage>()
            .view_data::<screens::ncr_page::NcrPage>()
            .view_data::<screens::recall_page::RecallsPage>()
            .view_data::<screens::recall_page::RecallPage>()
            .view_data::<screens::conformity_page::ConformityPage>()
            .view_data::<screens::exports_page::ExportsPage>()
            .view_data::<screens::returns_page::ReturnsPage>()
            .view_data::<screens::inspections_page::InspectionsPage>()
            .view_data::<po_upload::OrdersPage>()
            .view_data::<portal::InvoicesPage>();
        wl_kernel::approvals::register_subject_link(ncr::SUBJECT_TYPE, |id| {
            screens::ncr_page::url(id)
        });
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
        app.nav(
            "app",
            NavLink::route("manufacturing.ncrs", "nav.ncrs")
                .icon("clipboard-check")
                .group("nav.app")
                .order(10)
                .permission(permissions::NCRS_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("manufacturing.recalls", "nav.recalls")
                .icon("triangle-alert")
                .group("nav.app")
                .order(11)
                .permission(permissions::RECALLS_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("manufacturing.conformity", "nav.conformity")
                .icon("shield-check")
                .group("nav.app")
                .order(12)
                .permission(permissions::CONFORMITY_MANAGE),
        );
        app.nav(
            "app",
            NavLink::route("manufacturing.exports", "nav.exports")
                .icon("ship")
                .group("nav.app")
                .order(13)
                .permission(permissions::EXPORTS_MANAGE),
        );
        app.nav(
            "app",
            NavLink::route("manufacturing.returns", "nav.returned_goods")
                .icon("undo-2")
                .group("nav.app")
                .order(14)
                .permission(permissions::RETURNED_GOODS_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("manufacturing.inspections", "nav.inspections")
                .icon("clipboard-check")
                .group("nav.app")
                .order(15)
                .permission(permissions::INSPECTIONS_VIEW),
        );
        // `MFG.RUL.08`: the evidence of the exports, every day at 08:00.
        app.schedule()
            .daily_at(
                exports::RUN_AT,
                exports::TASK,
                |state: AppState| async move {
                    exports::run_daily(&state, wl_kernel::documents::today())
                        .await
                        .map(|_| ())
                },
            )
            .timezone("Asia/Dubai");
        app.model::<Plant>()
            .model::<Line>()
            .model::<WorkOrder>()
            .model::<PickLine>()
            .model::<MaterialIssue>()
            .model::<ProductionReport>()
            .model::<AbortLine>()
            .model::<Ncr>()
            .model::<NcrDecision>()
            .model::<disputes::NcrLink>()
            .model::<RepickFlag>()
            .model::<ItemFlag>()
            .model::<po_upload::PoUpload>()
            .model::<exports::ExportWatch>()
            .model::<exports::ContractNote>()
            .model::<order_changes::Adjustment>()
            .model::<late_report::LateReport>()
            .model::<returns::ReturnInspection>()
            .model::<returns::ReturnDecision>()
            .model::<returns::ReturnMove>();
        work_order::register();
        ncr::register();
        report::register(app);
        // `PLT.ACL.10`: a report typed in from the paper job cards.
        late_report::register(app);
        notices::register_types();
        // 07 §7.1 *Party blocked*: the work orders of a blocked customer
        // continue; the planner may `unlink` them to stock.
        wl_kernel::block_effects::AgreementKind::new("work_order")
            .owner("manufacturing")
            .effect(wl_kernel::block_effects::BlockEffect::Continue)
            .decider(permissions::PLANNER_ROLE)
            .register();
        // `MFG.ACL.12`: the posted quantities still match the request at the
        // final approval.
        wl_kernel::approvals::register_final_check(work_order::SUBJECT_TYPE, |tx, request| {
            Box::pin(abort::recheck(tx, request))
        });
        // `MFG.ACL.03`: the inspector who raised the NCR does not approve it.
        wl_kernel::approvals::register_final_check(ncr::SUBJECT_TYPE, |tx, request| {
            Box::pin(ncrs::recheck(tx, request))
        });
        abort::listen(app);
        // `MFG.RUL.09`: the work orders follow their sales order.
        order_changes::listen(app);
        // `MFG.RUL.10`: returned and refused goods are inspected and sales
        // decides to deliver again or cancel.
        returns::listen(app);
        // `SAL.ACL.05`: which lines of an order have started.
        wl_kernel::sales::provide_fulfilment_progress(app, progress::ProductionProgress);
        // 07 §7.1: the cancel sheets preselect scrap for a customer-specific
        // item and finish to stock for any other.
        wl_kernel::sales::provide_outcome_proposals(app, outcome_proposals::ItemOutcomeProposals);
        material_hold::listen(app);
        ncrs::listen(app);
        // The supplier scorecard counts the NCRs caused by the supplier.
        wl_kernel::inventory::provide_supplier_incidents(app, incidents::NcrIncidents);
        disputes::listen(app);
        // `TracePartyResolver` for 07: a dispatch and a work order are their
        // customer; a dispatch that has not left is flagged for a re-pick.
        wl_kernel::inventory::provide_trace_party_resolver(app, recalls::ProductionParties);
        recalls::listen(app);
        // `MFG.ACL.11`: the item flags and the certificates of the regulated
        // items, asked at `pick` and `dispatch`, at order `submit` and by the
        // nightly look-ahead.
        conformity::register(app);
        // A lot with an NCR open on it is not released (§7.1 *NCR*).
        wl_kernel::lot_disposal::provide_lot_release_guard(app, ncrs::NcrReleaseGuard);
        app.schedule()
            .every_minutes(5, ABORT_SWEEP_TASK, |state: AppState| async move {
                abort::sweep(&state).await?;
                ncrs::sweep(&state).await?;
                report::sweep(&state).await.map(|_| ())
            });
    }
}
