#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod amend;
pub mod building;
pub mod cancellation;
pub mod chase;
pub mod charge;
pub mod charge_type;
pub mod cheques;
mod db;
pub mod documents;
pub mod events;
pub mod force;
pub mod holding;
pub mod invoicing;
pub mod lease;
mod lease_checks;
mod lease_events;
pub mod lease_machine;
pub mod legal_case;
pub mod leases;
pub mod notice;
pub mod notices;
pub mod pages;
pub mod parse;
pub mod permissions;
pub mod renew;
pub mod renewal;
mod tasks;
pub mod termination;
pub mod terms;
mod texts;
pub mod unit;
pub mod unit_machine;
pub mod withdraw;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use cancellation::CancelWait;
pub use building::{Building, Floor, NewBuilding, create_building, ensure_floor};
pub use charge::{ChargeStatus, LeaseCharge};
pub use charge_type::ChargeType;
pub use events::{
    LeaseActivated, LeaseAmended, LeaseCancelled, LeaseForceTerminated, LeaseNoticeWithdrawn,
    LeaseTerminated,
};
pub use lease::{CreditChoice, Lease, LeaseStatus, RenewalDecision, VersionKind};
pub use leases::{DraftEdit, EjariInput, NewLease, NewVersion};
pub use legal_case::{CaseOutcome, CaseStatus, CaseType, LegalCase};
pub use unit::{NewUnit, Unit, UnitStatus, UnitType, UnitUse, create_unit};

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory (`resources/views/property/index.html`).
const VIEWS: &[(&str, &str)] = &[
    (
        "property/index.html",
        include_str!("../views/property/index.html"),
    ),
    (
        "property/building.html",
        include_str!("../views/property/building.html"),
    ),
    (
        "property/leases.html",
        include_str!("../views/property/leases.html"),
    ),
    (
        "property/lease.html",
        include_str!("../views/property/lease.html"),
    ),
];

/// Dirhams as text from fils: `AED 25,000.00`.
pub fn money_text(fils: i64) -> String {
    let sign = if fils < 0 { "-" } else { "" };
    let magnitude = fils.unsigned_abs();
    let whole = (magnitude / 100).to_string();
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{sign}AED {grouped}.{:02}", magnitude % 100)
}

/// The Property module. Add it to the app with `.module(property::Property)`.
pub struct Property;

impl Module for Property {
    fn name(&self) -> &'static str {
        "property"
    }

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<pages::portfolio::IndexPage>()
            .view_data::<pages::portfolio::BuildingPage>()
            .view_data::<pages::leases::IndexPage>()
            .view_data::<pages::leases::ShowPage>();
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        app.model::<Building>()
            .model::<Floor>()
            .model::<Unit>()
            .model::<Lease>()
            .model::<LeaseCharge>()
            .model::<ChargeType>()
            .model::<LegalCase>()
            .model::<CancelWait>();
        unit_machine::register();
        lease_machine::register();
        documents::register();
        lease_events::register(app);
        legal_case::register(app);
        // A deposit that grew is raised as a charge of the lease (BL-PRP-08).
        wl_kernel::deposits::provide_deposit_top_ups(app, amend::LeaseTopUps);
        cancellation::register(app);
        invoicing::schedule(app);
        notices::register_types();
        chase::schedule(app);
        renewal::schedule(app);
        termination::schedule(app);
        // A building is the scope roles are assigned in.
        wl_kernel::scopes::register_label(Building::TABLE, |db, id| {
            let id = id.to_owned();
            Box::pin(async move {
                let Ok(id) = id.parse::<i64>() else {
                    return Ok(None);
                };
                Ok(Building::find(db, id).await?.map(|b| b.name_en))
            })
        });
        // In the app group of the staff sidebar; the labels are the app's
        // `nav.property` and `nav.leases` texts.
        app.nav(
            "app",
            NavLink::route("property.index", "nav.property")
                .icon("building-2")
                .group("nav.app")
                .order(10)
                .permission(permissions::UNITS_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("property.leases", "nav.leases")
                .icon("file-text")
                .group("nav.app")
                .order(11)
                .permission(permissions::LEASES_VIEW),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_reads_as_dirhams_with_grouped_thousands() {
        assert_eq!(money_text(2_500_000), "AED 25,000.00");
        assert_eq!(money_text(5), "AED 0.05");
        assert_eq!(money_text(-123_456_789), "-AED 1,234,567.89");
        assert_eq!(money_text(100_000_000), "AED 1,000,000.00");
        assert_eq!(money_text(0), "AED 0.00");
    }
}
