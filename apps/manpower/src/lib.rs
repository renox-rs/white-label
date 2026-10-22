#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod absence;
mod access;
pub mod approvers;
mod attendance;
pub mod bench;
pub mod billing;
pub mod blocked;
pub mod config;
pub mod contract;
pub mod contracts;
pub mod cover;
pub mod cover_notice;
mod db;
pub mod deployment;
pub mod deployments;
pub mod documents;
pub mod ending;
pub mod ends;
pub mod events;
pub mod exits;
pub mod expiry;
pub mod guard;
pub mod hold;
pub mod notices;
pub mod overlap;
pub mod pages;
pub mod permissions;
pub mod portal;
pub mod release;
pub mod renewal;
pub mod request;
pub mod requests;
pub mod resumption;
mod screens;
pub mod seed;
pub mod site;
pub mod suspension;
pub mod termination;
mod texts;
pub mod visa_steps;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use contract::{BillingCycle, ClientContract, ContractStatus};
pub use deployment::{Deployment, DeploymentKind, DeploymentStatus};
pub use events::{
    ContractActivated, ContractEnded, ContractRenewed, CoverRequested, DeploymentActivated,
    DeploymentCancelled, DeploymentEnded, DeploymentResumed, DeploymentSuspended,
};
pub use hold::{ContractHold, HoldReason};
pub use request::{ManpowerRequest, RequestKind, RequestStatus};
pub use site::Site;

/// The name of the daily scheduled task that ends the deployments whose end
/// date has passed.
pub const END_DATE_TASK: &str = "manpower:deployment-end-daily";

/// The name of the daily scheduled task of `MPW.RUL.10`: contracts with an
/// invoice overdue more than 30 days go on credit hold.
pub const CREDIT_HOLD_TASK: &str = "manpower:credit-hold-daily";

/// The name of the daily scheduled task that ends the client contracts whose
/// end date has passed (`end`, BL-MPW-06).
pub const CONTRACT_END_TASK: &str = "manpower:contract-end-daily";

/// The name of the daily scheduled task of `MPW.RUL.05`: the approved hours
/// of the clients whose billing cycle ended become draft invoices.
pub const BILLING_TASK: &str = billing::TASK;

/// The name of the daily scheduled task of `MPW.RUL.11`: the account manager is
/// told 60 and 30 days before a client contract's end date.
pub const CONTRACT_EXPIRY_TASK: &str = "manpower:contract-expiry-daily";

/// The name of the daily scheduled task of `MPW.RUL.09`: the absence cases.
pub const ABSENCE_TASK: &str = "manpower:absence-daily";

/// The name of the daily scheduled task of `MPW.RUL.14`: a visa case step that
/// passed its target.
pub const VISA_STEP_TASK: &str = visa_steps::TASK;

/// The Manpower module. Add it to the app with
/// `.module(manpower::Manpower)`.
pub struct Manpower;

impl Module for Manpower {
    fn name(&self) -> &'static str {
        "manpower"
    }

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<pages::ContractsPage>()
            .view_data::<pages::RequestsPage>()
            .view_data::<pages::DeploymentsPage>()
            .view_data::<screens::contract_page::ContractPage>()
            .view_data::<screens::request_page::RequestPage>()
            .view_data::<screens::deployment_page::DeploymentPage>()
            .view_data::<screens::absence_page::CasesPage>()
            .view_data::<screens::absence_page::CasePage>()
            .view_data::<screens::portal_page::RequestsPage>()
            .view_data::<screens::portal_page::DeploymentsPage>()
            .view_data::<screens::portal_page::TimesheetsPage>();
        // An approval of a contract or a request links to its page.
        wl_kernel::approvals::register_subject_link(contract::SUBJECT_TYPE, |id| {
            screens::contract_page::url(id)
        });
        wl_kernel::approvals::register_subject_link(request::SUBJECT_TYPE, |id| {
            screens::request_page::url(id)
        });
        wl_kernel::approvals::register_subject_link(deployment::SUBJECT_TYPE, |id| {
            screens::deployment_page::url(id)
        });
        app.templates(|env| {
            for (name, source) in VIEWS.iter().chain(screens::VIEWS) {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        // The contracts, requests and deployments in the app group of the
        // sidebar; the labels are the app's `nav.*` texts.
        app.nav(
            "app",
            NavLink::route("manpower.contracts", "nav.contracts")
                .icon("file-text")
                .group("nav.app")
                .order(9)
                .permission(permissions::CONTRACTS_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("manpower.requests", "nav.requests")
                .icon("users")
                .group("nav.app")
                .order(10)
                .permission(permissions::REQUESTS_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("manpower.deployments", "nav.deployments")
                .icon("map-pin")
                .group("nav.app")
                .order(11)
                .permission(permissions::DEPLOYMENTS_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("manpower.absence", "nav.absence")
                .icon("user-x")
                .group("nav.app")
                .order(12)
                .permission(permissions::ABSENCE_MANAGE),
        );
        // The client's own users: their company's requests, workers on site
        // and timesheets (BL-MPW-28). Only the people of the portal see them;
        // the pages check the grant for the company.
        for (route, label, icon, order) in [
            (
                "manpower.portal_requests",
                "nav.portal_requests",
                "users",
                13,
            ),
            (
                "manpower.portal_deployments",
                "nav.portal_deployments",
                "map-pin",
                14,
            ),
            (
                "manpower.portal_timesheets",
                "nav.portal_timesheets",
                "clock",
                15,
            ),
        ] {
            app.nav(
                "app",
                NavLink::route(route, label)
                    .icon(icon)
                    .group("nav.app")
                    .order(order)
                    .when(wl_kernel::portal::is_portal_only),
            );
        }
        app.model::<Site>()
            .model::<ClientContract>()
            .model::<ManpowerRequest>()
            .model::<Deployment>()
            .model::<documents::DeploymentDocumentFlag>()
            .model::<suspension::DeploymentSuspension>()
            .model::<suspension::SuspensionSpan>()
            .model::<ContractHold>()
            .model::<expiry::ContractEndNotice>()
            .model::<absence::AbsenceCase>()
            .model::<visa_steps::VisaStepNotice>()
            .model::<approvers::ContractTerms>();
        contract::register();
        request::register();
        // `MPW.RUL.06`: an approved leave opens the cover requests the
        // contract requires.
        cover::listen(app);
        deployment::register();
        // BL-MPW-09: what a deployment depends on (`BlockSubjects`), what an
        // expiring document stops (`FutureWork`) and the flags the nightly
        // look-ahead leaves.
        documents::register();
        // BL-MPW-15: a blocked client's deployments continue (the decider is the
        // MD); its task is opened on `PartyBlocked`.
        blocked::register();
        blocked::listen(app);
        documents::listen(app);
        // `MPW.RUL.12`: a document or licence a deployment needs expires.
        suspension::register();
        // Attendance at a site is refused while the deployment is suspended.
        wl_kernel::attendance::provide_context_closure(app, attendance::SuspendedDeployments);
        suspension::listen(app);
        // `MPW.RUL.12`: the exits of the worker end or cancel the deployments.
        exits::listen(app);
        resumption::listen(app);
        notices::register_types();
        // BL-MPW-20: the rules whose failed run alerts the owner (the MD).
        seed::register_critical_rules();
        // BL-MPW-19: who approves a deployment's timesheets, who is told when
        // the client does not, and after how many days silence is approval.
        wl_kernel::timesheets::provide_approver_resolver(app, approvers::Approvers);
        // Deployments whose end date has passed end by themselves: daily at
        // 02:00 Asia/Dubai.
        app.schedule()
            .daily_at("02:00", END_DATE_TASK, |state: AppState| async move {
                deployments::daily(&state).await
            })
            .timezone("Asia/Dubai");
        // `MPW.RUL.10`: the credit hold, daily at 02:00 and on a payment or a
        // bounced cheque of the client.
        app.schedule()
            .daily_at("02:00", CREDIT_HOLD_TASK, |state: AppState| async move {
                hold::daily(&state).await
            })
            .timezone("Asia/Dubai");
        // The contracts past their end date `end`, daily at 00:05.
        app.schedule()
            .daily_at("00:05", CONTRACT_END_TASK, |state: AppState| async move {
                ending::daily(&state).await
            })
            .timezone("Asia/Dubai");
        // `MPW.RUL.05`: approved hours become usage when a timesheet is
        // approved, and draft invoices at the end of the billing cycle, daily
        // at 03:00 (after the contracts that ended have ended, 00:05).
        billing::listen(app);
        app.schedule()
            .daily_at("03:00", BILLING_TASK, |state: AppState| async move {
                billing::daily(&state).await
            })
            .timezone("Asia/Dubai");
        // `MPW.RUL.11`: the warnings before a contract ends, daily at 02:00.
        app.schedule()
            .daily_at(
                "02:00",
                CONTRACT_EXPIRY_TASK,
                |state: AppState| async move { expiry::daily(&state).await },
            )
            .timezone("Asia/Dubai");
        // `MPW.RUL.09`: a worker who has not turned up gets an absence case,
        // daily at 02:00 Asia/Dubai.
        absence::register();
        absence::listen(app);
        app.schedule()
            .daily_at("02:00", ABSENCE_TASK, |state: AppState| async move {
                absence::daily(&state).await
            })
            .timezone("Asia/Dubai");
        // `MPW.RUL.13`: cover not found in time, within a quarter of an hour
        // of the hour.
        app.schedule()
            .every_minutes(15, cover_notice::TASK, |state: AppState| async move {
                cover_notice::run(&state).await
            });
        // `MPW.RUL.14`: a visa case step that passed its target, daily at 02:00.
        app.schedule()
            .daily_at("02:00", visa_steps::TASK, |state: AppState| async move {
                visa_steps::run(&state).await
            })
            .timezone("Asia/Dubai");
        hold::listen(app);
        release::listen(app);
        termination::listen(app);
        // A site is a scope roles are assigned in, shown by its name.
        wl_kernel::scopes::register_label(site::SUBJECT_TYPE, |db, id| {
            let id = id.to_owned();
            Box::pin(async move {
                let Ok(id) = id.parse::<i64>() else {
                    return Ok(None);
                };
                Ok(Site::find(db, id).await?.map(|site| site.name))
            })
        });
    }
}

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
const VIEWS: &[(&str, &str)] = &[
    (
        "manpower/contracts.html",
        include_str!("../views/manpower/contracts.html"),
    ),
    (
        "manpower/requests.html",
        include_str!("../views/manpower/requests.html"),
    ),
    (
        "manpower/deployments.html",
        include_str!("../views/manpower/deployments.html"),
    ),
];
