#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod adjustment;
pub mod billing;
pub mod booking;
pub mod cancel;
pub mod cash_watch;
mod claim;
pub mod cod;
mod credit;
pub mod disposal;
mod events;
pub mod hold;
pub mod loss;
mod loss_pages;
pub mod netting;
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
pub mod reversal;
mod reversal_pages;
pub mod reweigh;
pub mod scan;
mod screen_texts;
pub mod seed;
pub mod shipment;
pub mod shipment_machine;
pub mod statement;
pub mod trace;
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
    ShipmentDelivered, ShipmentLost, ShipmentStatus, TrackingEvent,
};
pub use adjustment::{AdjustmentLine, AdjustmentStatus};
pub use cash_watch::CashWatch;
pub use cod::{CodLoss, CodRule};
pub use netting::Offset;
pub use statement::Statement;
pub use reversal::{Reversal, ReversalInput, ShipmentRefunds};
pub use trace::TraceTask;
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
        pages::routes()
            .merge(remittance_pages::routes())
            .merge(cod::routes())
            .merge(reversal_pages::routes())
            .merge(loss_pages::routes())
            .merge(disposal::routes())
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<pages::IndexPage>()
            .view_data::<pages::ShowPage>()
            .view_data::<remittance_pages::IndexPage>()
            .view_data::<remittance_pages::ShowPage>()
            .view_data::<cod::IndexPage>()
            .view_data::<reversal_pages::IndexPage>();
        app.templates(|env| {
            for (name, source) in pages::VIEWS
                .iter()
                .chain(remittance_pages::VIEWS)
                .chain(cod::VIEWS)
                .chain(reversal_pages::VIEWS) {
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
            .model::<AdjustmentLine>()
            .model::<Offset>()
            .model::<Statement>()
            .model::<CodLoss>()
            .model::<Reversal>()
            .model::<TraceTask>()
            .model::<CashWatch>();
        // The shipment's rule on who absorbs a COD difference.
        wl_kernel::delivery::provide_cod_rule(app, CodRule);
        // A refund to a recipient who is no party is paid by hand.
        wl_kernel::delivery::provide_manual_refunds(app, ShipmentRefunds);
        // The remittance and its adjustment lines have machines of their own.
        renox::state_machine::register::<Remittance>();
        renox::state_machine::register::<AdjustmentLine>();
        // `LOG.RUL.06`, weekly on Monday.
        remit::register(app);
        // `LOG.RUL.12`, daily at 02:00, and the approval of `LOG.ACL.14`.
        trace::register(app);
        // `LOG.RUL.13`, every 15 minutes: a driver who does not hand over the
        // cash (replaces `FIN.RUL.22` for drivers).
        cash_watch::register(app);
        // The approval of `LOG.ACL.14` for disposal.
        disposal::register();
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
