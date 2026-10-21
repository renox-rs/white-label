#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod contract;
pub mod contracts;
mod db;
pub mod events;
pub mod pages;
pub mod permissions;
pub mod site;
mod texts;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use contract::{BillingCycle, ClientContract, ContractStatus};
pub use events::ContractActivated;
pub use site::Site;

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
        app.view_data::<pages::ContractsPage>();
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        // The client contracts in the app group of the sidebar; the label is
        // the app's `nav.contracts` text.
        app.nav(
            "app",
            NavLink::route("manpower.contracts", "nav.contracts")
                .icon("file-text")
                .group("nav.app")
                .order(9)
                .permission(permissions::CONTRACTS_VIEW),
        );
        app.model::<Site>().model::<ClientContract>();
        contract::register();
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
const VIEWS: &[(&str, &str)] = &[(
    "manpower/contracts.html",
    include_str!("../views/manpower/contracts.html"),
)];
