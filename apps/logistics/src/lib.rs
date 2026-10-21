#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod billing;
pub mod booking;
pub mod cancel;
mod events;
pub mod notices;
pub mod pages;
pub mod permissions;
pub mod pickup;
pub mod rates;
pub mod reweigh;
pub mod scan;
mod screen_texts;
pub mod seed;
pub mod shipment;
pub mod shipment_machine;
mod texts;
mod voiding;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use booking::{ImportReport, NewShipment, RowResult};
pub use rates::{ClientRate, WeightBand};
pub use reweigh::{ClientTerms, Measurement};
pub use shipment::{
    Charge, Parcel, SHIPMENT_SERIES, SUBJECT_TYPE, Shipment, ShipmentStatus, TrackingEvent,
};
pub use shipment_machine::ShipmentInput;

/// The Logistics module. Add it to the app with
/// `.module(logistics::Logistics)`.
pub struct Logistics;

impl Module for Logistics {
    fn name(&self) -> &'static str {
        "logistics"
    }

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<pages::IndexPage>()
            .view_data::<pages::ShowPage>();
        app.templates(|env| {
            for (name, source) in pages::VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        app.model::<Shipment>()
            .model::<Parcel>()
            .model::<TrackingEvent>()
            .model::<Charge>()
            .model::<ClientRate>()
            .model::<WeightBand>()
            .model::<ClientTerms>()
            .model::<Measurement>();
        // The shipment's machine, and its place in the kernel's void pattern.
        voiding::register();
        // The shipment follows its delivery and cancels it.
        events::register(app);
        notices::register_types();
        // In the app group of the staff sidebar; the label is the host's
        // `nav.shipments` text.
        app.nav(
            "app",
            NavLink::route("logistics.shipments", "nav.shipments")
                .icon("package")
                .group("nav.app")
                .order(8)
                .permission(permissions::SHIPMENTS_VIEW),
        );
    }
}
