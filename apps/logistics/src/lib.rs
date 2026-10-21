#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod account;
pub mod adjustment;
pub mod billing;
pub mod block;
pub mod booking;
pub mod cancel;
pub mod cash_forecast;
pub mod cash_watch;
mod claim;
pub mod cod;
mod credit;
pub mod defaults;
pub mod disposal;
pub mod dispute;
mod events;
pub mod freight;
pub mod freight_cancel;
pub mod freight_costing;
pub mod freight_flow;
pub mod freight_free_time;
pub mod freight_hold;
pub mod freight_machine;
mod freight_pages;
pub mod freight_reopen;
pub mod grounding;
pub mod hold;
pub mod loss;
mod loss_pages;
pub mod netting;
pub mod notices;
pub mod pages;
pub mod partial;
pub mod permissions;
pub mod pickup;
pub mod portal;
pub mod rates;
pub mod remit;
pub mod remittance;
pub mod remittance_machine;
mod remittance_pages;
pub mod reversal;
mod reversal_pages;
pub mod review;
pub mod reweigh;
pub mod scan;
mod screen_texts;
pub mod seed;
pub mod shipment;
pub mod shipment_machine;
pub mod sms_fallback;
pub mod statement;
mod texts;
pub mod trace;
mod voiding;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use account::{AccountChange, AccountChangeStatus};
pub use adjustment::{AdjustmentLine, AdjustmentStatus};
pub use billing::{BillingRun, Monthly};
pub use booking::{ImportReport, NewShipment, RowResult};
pub use cash_watch::CashWatch;
pub use cod::{CodLoss, CodRule};
pub use dispute::StatementDispute;
pub use freight::{FreightContainer, FreightEvent, FreightJob, FreightStatus};
pub use freight_cancel::{FreightCancellation, FreightQuotation};
pub use freight_costing::{FreightMargin, JobLine};
pub use freight_free_time::{FreeTimeLine, FreeTimeNotice};
pub use freight_hold::FreightHold;
pub use freight_machine::FreightInput;
pub use freight_reopen::FreightReopening;
pub use grounding::VehicleGrounding;
pub use netting::Offset;
pub use rates::{ClientRate, WeightBand};
pub use remittance::{
    PaymentReturn, REMITTANCE_SERIES, Remittance, RemittanceLine, RemittanceStatus,
};
pub use remittance_machine::RemittanceInput;
pub use reversal::{Reversal, ReversalInput, ShipmentRefunds};
pub use reweigh::{ClientTerms, Measurement};
pub use shipment::{
    AddressChange, Charge, PARCEL_SUBJECT_TYPE, Parcel, SHIPMENT_SERIES, SUBJECT_TYPE, Shipment,
    ShipmentDelivered, ShipmentLost, ShipmentStatus, TrackingEvent,
};
pub use shipment_machine::ShipmentInput;
pub use statement::Statement;
pub use trace::TraceTask;

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
            .merge(account::routes())
            .merge(dispute::routes())
            .merge(freight::routes())
            .merge(freight_pages::routes())
            .merge(cod::routes())
            .merge(reversal_pages::routes())
            .merge(loss_pages::routes())
            .merge(disposal::routes())
            .merge(portal::routes())
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<pages::IndexPage>()
            .view_data::<pages::ShowPage>()
            .view_data::<remittance_pages::IndexPage>()
            .view_data::<remittance_pages::ShowPage>()
            .view_data::<account::ConfirmPage>()
            .view_data::<cod::IndexPage>()
            .view_data::<freight_pages::ShowPage>()
            .view_data::<reversal_pages::IndexPage>();
        app.templates(|env| {
            for (name, source) in pages::VIEWS
                .iter()
                .chain(remittance_pages::VIEWS)
                .chain(account::VIEWS)
                .chain(cod::VIEWS)
                .chain(freight_pages::VIEWS)
                .chain(reversal_pages::VIEWS)
            {
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
            .model::<FreightJob>()
            .model::<FreightEvent>()
            .model::<FreightContainer>()
            .model::<JobLine>()
            .model::<FreightMargin>()
            .model::<FreightHold>()
            .model::<FreightReopening>()
            .model::<VehicleGrounding>()
            .model::<FreightQuotation>()
            .model::<FreeTimeLine>()
            .model::<FreeTimeNotice>()
            .model::<FreightCancellation>()
            .model::<StatementDispute>()
            .model::<PaymentReturn>()
            .model::<AccountChange>()
            .model::<Offset>()
            .model::<Statement>()
            .model::<CodLoss>()
            .model::<Reversal>()
            .model::<TraceTask>()
            .model::<CashWatch>()
            .model::<BillingRun>();
        // The shipment's rule on who absorbs a COD difference.
        wl_kernel::delivery::provide_cod_rule(app, CodRule);
        // A refund to a recipient who is no party is paid by hand.
        wl_kernel::delivery::provide_manual_refunds(app, ShipmentRefunds);
        // The freight job has a machine of its own.
        renox::state_machine::register::<FreightJob>();
        // The remittance and its adjustment lines have machines of their own.
        renox::state_machine::register::<Remittance>();
        // `pay` and `repay` are money out: a blocked client is refused and the
        // company admin can approve an exception (`PTY.ACL.03`).
        remittance_machine::subject_machine().register();
        renox::state_machine::register::<AdjustmentLine>();
        renox::state_machine::register::<AccountChange>();
        // `LOG.ACL.13`: a change not both approved and confirmed lapses.
        account::register(app);
        // `LOG.RUL.06`, weekly on Monday.
        remit::register(app);
        // The COD owed to clients is a payment in the cash-flow forecast.
        cash_forecast::register();
        // `LOG.RUL.07`, monthly on day 1: the draft invoice of each client.
        billing::register(app);
        // `LOG.RUL.09`, daily at 07:00: the ETA of a freight job.
        freight_hold::register(app);
        // `LOG.RUL.14`, daily at 06:00: the free time of a container.
        freight_free_time::register(app);
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
        // The SMS gateway going down: the dispatcher's task and the clients'
        // e-mail (BL-LOG-25).
        sms_fallback::register(app);
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
