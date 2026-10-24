#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod accident_flag;
pub mod approval;
pub mod approval_rounds;
pub mod billing;
pub mod blocked;
pub mod bookings;
pub mod claim;
pub mod claim_approval;
pub mod claim_machine;
pub mod claim_split;
pub mod claim_total_loss;
pub mod claim_void;
pub mod claim_withdrawal;
pub mod claims;
mod db;
pub mod declines;
pub mod defaults;
pub mod documents;
pub mod estimate;
pub mod export_rule;
pub mod export_sale;
pub mod export_sales;
pub mod fleet_billing;
pub mod fleet_contract;
pub mod fleet_contract_machine;
pub mod fleet_contracts;
pub mod fleet_daily;
pub mod fleet_end;
pub mod fleet_gate;
pub mod fleet_invoice;
pub mod fleet_job;
pub mod fleet_service;
pub mod fleet_services;
pub mod incident;
pub mod incidents;
pub mod insurer;
pub mod insurer_mail;
pub mod insurers;
pub mod job_card;
pub mod job_card_machine;
pub mod job_cards;
pub mod job_stops;
pub mod job_void;
pub mod labour_line;
pub mod labour_menu;
pub mod labour_operation;
pub mod late_times;
pub mod money_defaults;
pub mod notices;
pub mod pages;
pub mod part_line;
pub mod parts;
pub mod parts_delays;
pub mod parts_wait;
pub mod payer;
pub mod permissions;
pub mod reasons;
pub mod recalls;
pub mod release;
pub mod search;
pub mod seed;
pub mod settings;
pub mod stop;
pub mod stop_billing;
pub mod storage;
pub mod substitute;
pub mod supplier_claim;
pub mod supplier_claim_machine;
pub mod supplier_claims;
pub mod supplier_rules;
pub mod supplier_terms;
pub mod supply;
pub mod test_drive;
pub mod test_drives;
mod texts;
mod texts_blocked;
mod texts_claim_stops;
mod texts_claims;
mod texts_export;
mod texts_fleet;
mod texts_grids;
mod texts_incidents;
mod texts_jobs;
mod texts_notices;
mod texts_police;
mod texts_recalls;
mod texts_stops;
mod texts_storage;
mod texts_supplier;
mod texts_vehicles;
pub mod vehicle;
pub mod vehicles;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use approval::{ApprovalStatus, EstimateApproval};
pub use claim::{ClaimStatus, InsuranceClaim, RepairType};
pub use claim_approval::{ApprovalKind, ApprovalOutcome, ApprovedLine, ClaimApproval};
pub use export_sale::{ExportEvidence, ExportSale};
pub use fleet_contract::{FleetContract, FleetStatus};
pub use fleet_invoice::FleetInvoice;
pub use fleet_job::FleetJob;
pub use fleet_service::FleetService;
pub use incident::Incident;
pub use insurer::Insurer;
pub use job_card::{JobCard, JobCardStatus};
pub use job_stops::JobCardCancelled;
pub use labour_line::{LabourLine, LabourPricing, LineStatus, WorkStatus};
pub use labour_operation::LabourOperation;
pub use part_line::{PartCondition, PartLine};
pub use payer::Payer;
pub use supplier_claim::{SupplierAnswer, SupplierClaim, SupplierClaimStatus};
pub use test_drive::TestDrive;
pub use vehicle::{Emirate, Vehicle};

/// The templates of the pages, under the names the pages ask for. An app
/// replaces one by putting a file of that name in its own views.
const VIEWS: &[(&str, &str)] = &[
    (
        "auto/vehicles.html",
        include_str!("../views/auto/vehicles.html"),
    ),
    (
        "auto/vehicle.html",
        include_str!("../views/auto/vehicle.html"),
    ),
    (
        "auto/job_cards.html",
        include_str!("../views/auto/job_cards.html"),
    ),
    (
        "auto/job_card.html",
        include_str!("../views/auto/job_card.html"),
    ),
    (
        "auto/technician.html",
        include_str!("../views/auto/technician.html"),
    ),
    (
        "auto/estimate.html",
        include_str!("../views/auto/estimate.html"),
    ),
    (
        "auto/approve.html",
        include_str!("../views/auto/approve.html"),
    ),
    (
        "auto/labour_menu.html",
        include_str!("../views/auto/labour_menu.html"),
    ),
    (
        "auto/claims.html",
        include_str!("../views/auto/claims.html"),
    ),
    ("auto/claim.html", include_str!("../views/auto/claim.html")),
    (
        "auto/insurers.html",
        include_str!("../views/auto/insurers.html"),
    ),
    (
        "auto/insurer.html",
        include_str!("../views/auto/insurer.html"),
    ),
    (
        "auto/fleet_contracts.html",
        include_str!("../views/auto/fleet_contracts.html"),
    ),
    (
        "auto/fleet_contract.html",
        include_str!("../views/auto/fleet_contract.html"),
    ),
    (
        "auto/export_sales.html",
        include_str!("../views/auto/export_sales.html"),
    ),
    (
        "auto/supplier_claims.html",
        include_str!("../views/auto/supplier_claims.html"),
    ),
    (
        "auto/supplier_claim.html",
        include_str!("../views/auto/supplier_claim.html"),
    ),
    (
        "auto/recall.html",
        include_str!("../views/auto/recall.html"),
    ),
];

/// A whole number with its thousands grouped: `52,000`.
pub fn number_text(number: i64) -> String {
    let sign = if number < 0 { "-" } else { "" };
    let digits = number.unsigned_abs().to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{sign}{grouped}")
}

/// Dirhams as text from fils: `AED 1,250.00`.
pub fn money_text(fils: i64) -> String {
    let sign = if fils < 0 { "-" } else { "" };
    let magnitude = fils.unsigned_abs();
    let whole = i64::try_from(magnitude / 100).unwrap_or(i64::MAX);
    format!("{sign}AED {}.{:02}", number_text(whole), magnitude % 100)
}

/// The Auto module. Add it to the app with `.module(auto::Auto)`.
pub struct Auto;

impl Module for Auto {
    fn name(&self) -> &'static str {
        "auto"
    }

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        // The records in the global search and the Related panel (BL-AGT-13).
        search::register();
        search::listen(app);
        app.view_data::<pages::vehicles::IndexPage>()
            .view_data::<pages::vehicles::ShowPage>()
            .view_data::<pages::job_cards::IndexPage>()
            .view_data::<pages::job_cards::TechnicianPage>()
            .view_data::<pages::job_cards::ShowPage>()
            .view_data::<pages::job_cards::EstimatePage>()
            .view_data::<pages::job_cards::ApprovePage>()
            .view_data::<pages::labour::IndexPage>()
            .view_data::<pages::claims::IndexPage>()
            .view_data::<pages::claims::ShowPage>()
            .view_data::<pages::insurers::IndexPage>()
            .view_data::<pages::insurers::ShowPage>()
            .view_data::<pages::fleet::IndexPage>()
            .view_data::<pages::fleet::ShowPage>()
            .view_data::<pages::export_sales::IndexPage>()
            .view_data::<pages::supplier_claims::IndexPage>()
            .view_data::<pages::supplier_claims::ShowPage>()
            .view_data::<pages::recalls::ShowPage>();
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        app.model::<Vehicle>()
            .model::<JobCard>()
            .model::<LabourLine>()
            .model::<PartLine>()
            .model::<EstimateApproval>()
            .model::<LabourOperation>()
            .model::<Insurer>()
            .model::<InsuranceClaim>()
            .model::<ClaimApproval>()
            .model::<FleetContract>()
            .model::<FleetService>()
            .model::<FleetJob>()
            .model::<FleetInvoice>()
            .model::<ExportSale>()
            .model::<ExportEvidence>()
            .model::<SupplierClaim>()
            .model::<parts_wait::PartsWait>()
            .model::<substitute::Substitute>()
            .model::<supply::SupplyPeriod>()
            .model::<settings::Setting>()
            .model::<TestDrive>()
            .model::<Incident>();
        // The job card: the machine, and the job card that the check-in of a
        // workshop booking opens.
        job_card_machine::register();
        bookings::register(app);
        // A job card stopped after the work started: the invoice, the stock,
        // the purchaser's task and the booking follow the cancel, and the
        // booking asks the job card what was done.
        job_stops::register(app);
        // The police report guard of the accident repair (`AUT.ACL.20`).
        documents::register();
        // The liability of a vehicle incident is decided on the job card.
        incidents::register();
        // The reminders to collect a vehicle and the storage fee (`AUT.RUL.18`).
        storage::register(app);
        job_void::listen(app);
        // A job card declined: the inspection fee, the booking and the
        // message asking the customer to collect; the hourly expiry check.
        declines::register(app);
        notices::register(app);
        late_times::register();
        // The legal period to supply parts and the substitute vehicle of a
        // warranty repair (compliance-critical).
        parts_delays::register(app);
        // The insurance claim: the machine, and the listeners that settle it
        // when its invoices are paid.
        claim_machine::register();
        claims::register_listeners(app);
        // What is written to the insurer and told to the customer about a
        // claim: the survey request, the withdrawal letter, the total loss.
        insurer_mail::register();
        // The fleet contract: the machine, the daily run (ends, renewals,
        // the services that fall due, the monthly invoice) and the listener
        // that frees a service plan when its booking is cancelled.
        fleet_contract_machine::register();
        fleet_daily::register(app);
        fleet_services::register_listeners(app);
        // What follows an end or a termination (bookings, the last invoice, the
        // fee, the fleet contact told) and the gate that refuses a new booking
        // for a suspended contract.
        fleet_end::register(app);
        fleet_gate::register(app);
        // Export sales: the exit evidence is watched by `AUT.RUL.15` (the
        // accountant is told at 60 days, the sale is flagged at 90).
        export_rule::register(app);
        // What a blocked party does to a fleet contract, a job card and an
        // insurance claim (BL-AUT-17).
        blocked::register();
        // The supplier warranty claim: the machine, and the listener that
        // recovers a claim when its return to supplier is credited.
        supplier_claim_machine::register();
        supplier_claims::register_listeners(app);
        // The task when a warranty ticket closes (`AUT.RUL.07`), the email of
        // the claim to the supplier and the notice at 60 days (`AUT.RUL.08`).
        supplier_rules::register(app);
        // A recalled part traced to the vehicle owners, fleets and customers
        // who got it, and the notice they are sent (BL-AUT-16).
        recalls::register(app);
        // A bill or a document that names a vehicle or a job card shows its
        // plate or `JOB-00007`, not `auto.vehicle:7`.
        register_labels();
        // In the app group of the staff sidebar; the labels are the host's
        // `nav.job_cards`, `nav.vehicles` and `nav.labour_menu` texts.
        app.nav(
            "app",
            NavLink::route("auto.job_cards", "nav.job_cards")
                .icon("wrench")
                .group("nav.app")
                .order(80)
                .permission(permissions::JOB_CARDS_VIEW),
        );
        // The technician's own list, for the one who has no view of all.
        app.nav(
            "app",
            NavLink::route("auto.technician", "nav.my_job_cards")
                .icon("wrench")
                .group("nav.app")
                .order(81)
                .permission(permissions::JOB_CARDS_WORK),
        );
        app.nav(
            "app",
            NavLink::route("auto.vehicles", "nav.vehicles")
                .icon("truck")
                .group("nav.app")
                .order(82)
                .permission(permissions::VEHICLES_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("auto.labour_menu", "nav.labour_menu")
                .icon("clipboard-list")
                .group("nav.app")
                .order(83)
                .permission(permissions::LABOUR_MENU_MANAGE),
        );
        app.nav(
            "app",
            NavLink::route("auto.claims", "nav.insurance_claims")
                .icon("shield-check")
                .group("nav.app")
                .order(84)
                .permission(permissions::INSURANCE_MANAGE),
        );
        app.nav(
            "app",
            NavLink::route("auto.fleet_contracts", "nav.fleet_contracts")
                .icon("calendar-check")
                .group("nav.app")
                .order(86)
                .permission(permissions::FLEET_CONTRACTS_MANAGE),
        );
        app.nav(
            "app",
            NavLink::route("auto.supplier_claims", "nav.supplier_claims")
                .icon("handshake")
                .group("nav.app")
                .order(87)
                .permission(permissions::SUPPLIER_CLAIMS_MANAGE),
        );
        app.nav(
            "app",
            NavLink::route("auto.export_sales", "nav.export_sales")
                .icon("percent")
                .group("nav.app")
                .order(88)
                .permission(permissions::EXPORT_SALES_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("auto.insurers", "nav.insurers")
                .icon("building-2")
                .group("nav.app")
                .order(85)
                .permission(permissions::INSURANCE_MANAGE),
        );
    }
}

/// How the documents and bills of other modules name the records of the app
/// (wl-documents `register_subject_label`): the vehicle by its plate.
fn register_labels() {
    use wl_kernel::documents::register_subject_label;
    register_subject_label(vehicle::VEHICLE_SUBJECT_TYPE, |db, id| {
        Box::pin(async move { Ok(Vehicle::find(db, id).await?.map(|vehicle| vehicle.plate())) })
    });
    register_subject_label(job_card::JOB_CARD_SUBJECT_TYPE, |db, id| {
        Box::pin(async move { Ok(JobCard::find(db, id).await?.map(|card| card.number())) })
    });
    register_subject_label(fleet_contract::FLEET_CONTRACT_SUBJECT_TYPE, |db, id| {
        Box::pin(async move {
            Ok(FleetContract::find(db, id)
                .await?
                .map(|contract| contract.number()))
        })
    });
    register_subject_label(supplier_claim::SUPPLIER_CLAIM_SUBJECT_TYPE, |db, id| {
        Box::pin(async move {
            Ok(SupplierClaim::find(db, id)
                .await?
                .map(|claim| claim.number()))
        })
    });
    register_subject_label(claim::INSURANCE_CLAIM_SUBJECT_TYPE, |db, id| {
        Box::pin(async move {
            Ok(InsuranceClaim::find(db, id)
                .await?
                .map(|claim| claim.number()))
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_reads_as_dirhams_with_grouped_thousands() {
        assert_eq!(money_text(125_000), "AED 1,250.00");
        assert_eq!(money_text(5), "AED 0.05");
        assert_eq!(money_text(-123_456_789), "-AED 1,234,567.89");
        assert_eq!(money_text(0), "AED 0.00");
    }

    #[test]
    fn numbers_group_their_thousands() {
        assert_eq!(number_text(52_000), "52,000");
        assert_eq!(number_text(999), "999");
        assert_eq!(number_text(-1_234_567), "-1,234,567");
    }
}
