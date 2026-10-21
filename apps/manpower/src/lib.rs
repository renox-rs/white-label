#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

mod access;
pub mod blocked;
pub mod config;
pub mod contract;
pub mod contracts;
pub mod cover;
mod db;
pub mod deployment;
pub mod deployments;
pub mod documents;
pub mod events;
pub mod hold;
pub mod notices;
pub mod pages;
pub mod permissions;
pub mod request;
pub mod requests;
pub mod site;
mod texts;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use contract::{BillingCycle, ClientContract, ContractStatus};
pub use deployment::{Deployment, DeploymentKind, DeploymentStatus};
pub use events::{
    ContractActivated, CoverRequested, DeploymentActivated, DeploymentCancelled, DeploymentEnded,
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
            .view_data::<pages::DeploymentsPage>();
        app.templates(|env| {
            for (name, source) in VIEWS {
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
        app.model::<Site>()
            .model::<ClientContract>()
            .model::<ManpowerRequest>()
            .model::<Deployment>()
            .model::<documents::DeploymentDocumentFlag>()
            .model::<ContractHold>();
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
        notices::register_types();
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
        hold::listen(app);
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
