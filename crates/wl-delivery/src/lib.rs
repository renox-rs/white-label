#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod abort;
pub mod attempted;
pub mod close;
pub mod cod;
pub mod day_list;
pub mod delivery;
mod delivery_machine;
pub mod doc_guard;
pub mod hand_over;
pub mod lost;
pub mod notices;
pub mod pages;
pub mod plan;
pub mod proof;
pub mod reasons;
pub mod request;
pub mod reverse;
pub mod review;
pub mod run;
mod run_machine;
pub mod seed;
pub mod settings;
mod texts;
pub mod trace;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;
use wl_kernel::PodMethod;

pub use delivery::{
    Attempt, AttemptOutcome, Delivery, DeliveryLine, DeliveryStatus, Direction, Proof, Trace,
};
pub use plan::NewRun;
pub use reasons::{CodReason, FailureReason, RefusalReason};
pub use request::Deliveries;
pub use run::{Run, RunStatus, VehicleType};
pub use settings::{DeliverySettings, PerishableAction, PerishablePolicy};

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
const VIEWS: &[(&str, &str)] = &[
    (
        "wl-delivery/index.html",
        include_str!("../views/wl-delivery/index.html"),
    ),
    (
        "wl-delivery/show.html",
        include_str!("../views/wl-delivery/show.html"),
    ),
    (
        "wl-delivery/runs.html",
        include_str!("../views/wl-delivery/runs.html"),
    ),
    (
        "wl-delivery/run.html",
        include_str!("../views/wl-delivery/run.html"),
    ),
    (
        "wl-delivery/driver.html",
        include_str!("../views/wl-delivery/driver.html"),
    ),
    (
        "wl-delivery/stop.html",
        include_str!("../views/wl-delivery/stop.html"),
    ),
];

/// The module's permissions (wl-delivery §6).
pub mod permissions {
    /// See deliveries.
    pub const VIEW: &str = "deliveries.view";
    /// Put deliveries on runs and take them off (`assign`, `unassign`).
    pub const DISPATCH: &str = "deliveries.dispatch";
    /// Record the outcome of a stop (`deliver`, `fail`); the run's driver.
    pub const COMPLETE: &str = "deliveries.complete";
    /// Cancel a delivery, reattempt, return or discard it, open a review.
    pub const MANAGE: &str = "deliveries.manage";
    /// See runs.
    pub const RUNS_VIEW: &str = "runs.view";
    /// Plan, dispatch, hand over and abort runs.
    pub const RUNS_DISPATCH: &str = "runs.dispatch";
    /// Drive a run: the driver's day list and stops.
    pub const RUNS_DRIVE: &str = "runs.drive";
    /// See a delivery's proof.
    pub const POD_VIEW: &str = "pod.view";
    /// Request a reversal or a manual `mark_lost`.
    pub const REVERSE: &str = "deliveries.reverse";
    /// Approve a cash on delivery difference (`WRK.ACL.09`).
    pub const APPROVE_COD_DIFFERENCE: &str = "deliveries.approve_cod_difference";
}

/// The delivery module. The host adds it with
/// `.module(wl_delivery::WlDelivery::new())` when `client.toml` enables
/// `delivery`.
#[derive(Default)]
#[non_exhaustive]
pub struct WlDelivery {
    settings: DeliverySettings,
}

impl WlDelivery {
    /// The module with the §7 defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// The settings as built, for a page that shows what is in force.
    pub fn settings(&self) -> &DeliverySettings {
        &self.settings
    }

    /// Turns partial acceptance on or off for deliveries requested by
    /// `source_type` (§7: on; 03 single parcels: off). It is copied onto a
    /// delivery when it is created.
    pub fn partial_acceptance(mut self, source_type: &str, on: bool) -> Self {
        self.settings.set_partial_acceptance(source_type, on);
        self
    }

    /// Sets the proof required for deliveries requested by `source_type`:
    /// the `methods`, and whether any one is enough (`any`) or each is
    /// needed (§7: photo and signature, each).
    pub fn required_proof(mut self, source_type: &str, methods: &[PodMethod], any: bool) -> Self {
        self.settings.set_required_proof(source_type, methods, any);
        self
    }

    /// Sets the cash difference accepted without a reason, in fils (§7:
    /// 0).
    pub fn cod_tolerance(mut self, fils: i64) -> Self {
        self.settings.set_cod_tolerance(fils);
        self
    }

    /// Allows (`true`, §7) or forbids a short payment above the tolerance
    /// with an approver's agreement (`WRK.ACL.09`); forbidden, the driver can
    /// only fail the attempt.
    pub fn cod_approval(mut self, allowed: bool) -> Self {
        self.settings.set_cod_approval(allowed);
        self
    }

    /// Sets the minutes an approver has to answer a cash difference before
    /// the request lapses (§7: 10).
    pub fn cod_approval_minutes(mut self, minutes: i64) -> Self {
        self.settings.set_cod_approval_minutes(minutes);
        self
    }

    /// Sets who may absorb a cash difference (§7: all four; 03 leaves out the
    /// customer).
    pub fn cod_absorbers(mut self, absorbers: &[wl_kernel::delivery::CodAbsorber]) -> Self {
        self.settings.set_cod_absorbers(absorbers);
        self
    }

    /// Makes the driver's cash period leave the escalation of `wl-payments`
    /// (`FIN.RUL.22`) to a vertical rule (03: `LOG.RUL.13`).
    pub fn replace_cash_escalation(mut self, replaced: bool) -> Self {
        self.settings.set_replace_cash_escalation(replaced);
        self
    }

    /// Makes the receiving driver count the cash at a run hand-over (§7: on);
    /// off, the amount recorded in the giving driver's custody moves.
    pub fn hand_over_counts_cash(mut self, on: bool) -> Self {
        self.settings.set_hand_over_counts_cash(on);
        self
    }

    /// Adds abort reasons to breakdown, accident, driver unwell, weather and
    /// other (03: `driver_unreachable`, used by `LOG.RUL.13`).
    pub fn abort_reasons(mut self, extra: &[&str]) -> Self {
        self.settings.set_extra_abort_reasons(extra);
        self
    }

    /// Names the role that gets the dispatcher's tasks, such as the one for
    /// an item reported missing at run close (default `dispatcher`); the
    /// company admin gets it when nobody holds the role.
    pub fn dispatcher_role(mut self, role: &str) -> Self {
        self.settings.set_dispatcher_role(role);
        self
    }

    /// Sets what follows a failed visit and a run `abort` for perishable
    /// deliveries of `source_type` (`WRK.RUL.16`, §7). The module default is
    /// *discard* for both; 06 sets *return* after a failed visit and
    /// *discard* when the run was aborted.
    pub fn perishable_policy(
        mut self,
        source_type: &str,
        on_failed_visit: settings::PerishableAction,
        on_abort: settings::PerishableAction,
    ) -> Self {
        self.settings
            .set_perishable_policy(source_type, on_failed_visit, on_abort);
        self
    }

    /// Sets the wastage value, in fils, above which a discard needs a second
    /// person's approval (`STK.ACL.02`, §7: AED 1,000; 06: AED 200).
    pub fn discard_approval_limit(mut self, fils: i64) -> Self {
        self.settings.set_discard_approval_fils(fils);
        self
    }

    /// Sets the delivery promise of `source_type` in minutes: the perishable
    /// hand-over window (BL-DLV-10) for a request that gives none.
    pub fn promise_minutes(mut self, source_type: &str, minutes: i64) -> Self {
        self.settings.set_promise_minutes(source_type, minutes);
        self
    }

    /// Turns `WRK.RUL.08` on (default off, see
    /// [`DeliverySettings::set_attempt_rule`]).
    pub fn attempt_rule(mut self, on: bool) -> Self {
        self.settings.set_attempt_rule(on);
        self
    }

    /// Sets the days a delivery may go untraced before `WRK.RUL.17` marks
    /// it lost (§7: 7).
    pub fn untraced_days(mut self, days: i64) -> Self {
        self.settings.set_untraced_days(days);
        self
    }
}

impl Module for WlDelivery {
    fn name(&self) -> &'static str {
        "wl-delivery"
    }

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<pages::IndexPage>()
            .view_data::<pages::ShowPage>()
            .view_data::<pages::RunsPage>()
            .view_data::<pages::RunPage>()
            .view_data::<pages::DriverPage>()
            .view_data::<pages::StopPage>();
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        // In the app group of the staff sidebar; the labels are the app's
        // `nav.deliveries`, `nav.runs` and `nav.driver` texts.
        app.nav(
            "app",
            NavLink::route("wl_delivery.index", "nav.deliveries")
                .icon("package")
                .group("nav.app")
                .order(60)
                .permission(permissions::VIEW),
        );
        app.nav(
            "app",
            NavLink::route("wl_delivery.runs", "nav.runs")
                .icon("truck")
                .group("nav.app")
                .order(61)
                .permission(permissions::RUNS_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("wl_delivery.driver", "nav.driver")
                .icon("map-pin")
                .group("nav.app")
                .order(62)
                .permission(permissions::RUNS_DRIVE),
        );
        app.model::<Run>()
            .model::<Delivery>()
            .model::<DeliveryLine>()
            .model::<Attempt>()
            .model::<Proof>()
            .model::<Trace>()
            .model::<close::Receipt>()
            .model::<hand_over::HandOver>()
            .model::<hand_over::HandOverStop>()
            .model::<FailureReason>()
            .model::<RefusalReason>()
            .model::<CodReason>()
            .model::<cod::ApprovalChoice>();
        // `rnx state:diagram Delivery` and the admin panel find the machines.
        renox::state_machine::register::<Delivery>();
        renox::state_machine::register::<Run>();
        // Sources ask for deliveries through the kernel's requester.
        wl_kernel::delivery::provide_requester(app, Deliveries);
        // The settings the lifecycles read.
        app.provide(self.settings.clone());
        // `delivery.assign` and `run.dispatch` are guardable transitions of
        // the document kinds' `blocks` (#157).
        doc_guard::register();
        // `mark_lost`'s approval (`WRK.ACL.10`), the claim ticket and
        // `DeliveryLost` after a loss, and the rule `WRK.RUL.17` at 02:00.
        lost::register(app);
        // `deliver_short`'s approval (`WRK.ACL.09`), the lapse of a request
        // nobody answered, and the finish of a difference the kernel approved.
        cod::register(app);
        // `reverse_delivery`'s approval (`WRK.ACL.10`) and what follows a
        // reversal: the cash, the ticket and `DeliveryReversed`.
        reverse::register(app);
        proof::register(app);
        // `reattempt`, `return` and `discard` after a failed attempt, the
        // approval of a wastage above the limit (`STK.ACL.02`) and what
        // follows a return or a discard.
        attempted::register(app, self.settings.discard_approval_fils());
        // The notices of an aborted run.
        notices::register_types();
        // A run closes when the driver's cash handover of it is counted.
        close::register(app);
        // The seeded defaults keep the client's changes.
        app.command(
            "delivery:install",
            "Store the default failure and refusal reasons, where the database lacks them",
            |_args, state: AppState| async move {
                let installed = seed::install(&state.db).await?;
                println!("{} delivery defaults created.", installed.total());
                Ok(())
            },
        );
        app.seeder(|state: AppState| async move { seed::install(&state.db).await.map(|_| ()) });
    }
}
