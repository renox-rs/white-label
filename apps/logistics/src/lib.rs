#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod adjustment;
pub mod billing;
pub mod booking;
pub mod cancel;
mod events;
pub mod hold;
pub mod notices;
pub mod pages;
pub mod partial;
pub mod permissions;
pub mod pickup;
pub mod rates;
pub mod remit;
pub mod remittance;
pub mod remittance_machine;
mod remittance_pages;
pub mod review;
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
    AddressChange, Charge, PARCEL_SUBJECT_TYPE, Parcel, SHIPMENT_SERIES, SUBJECT_TYPE, Shipment,
    ShipmentDelivered, ShipmentStatus, TrackingEvent,
};
pub use adjustment::{AdjustmentLine, AdjustmentStatus};
pub use remittance::{REMITTANCE_SERIES, Remittance, RemittanceLine, RemittanceStatus};
pub use remittance_machine::RemittanceInput;
pub use shipment_machine::ShipmentInput;

/// The Logistics module. Add it to the app with
/// `.module(logistics::Logistics)`.
pub struct Logistics;

impl Module for Logistics {
    fn name(&self) -> &'static str {
        "logistics"
    }

    fn routes(&self) -> Routes {
        pages::routes().merge(remittance_pages::routes())
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<pages::IndexPage>()
            .view_data::<pages::ShowPage>()
            .view_data::<remittance_pages::IndexPage>()
            .view_data::<remittance_pages::ShowPage>();
        app.templates(|env| {
            for (name, source) in pages::VIEWS.iter().chain(remittance_pages::VIEWS) {
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
            .model::<Measurement>()
            .model::<AddressChange>()
            .model::<Remittance>()
            .model::<RemittanceLine>()
            .model::<AdjustmentLine>();
        // The remittance and its adjustment lines have machines of their own.
        renox::state_machine::register::<Remittance>();
        renox::state_machine::register::<AdjustmentLine>();
        // `LOG.RUL.06`, weekly on Monday.
        remit::register(app);
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
        app.nav(
            "app",
            NavLink::route("logistics.remittances", "nav.remittances")
                .icon("banknote")
                .group("nav.app")
                .order(9)
                .permission(permissions::REMITTANCES_VIEW),
        );
    }
}
