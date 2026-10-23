#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod approval;
pub mod approval_rounds;
pub mod billing;
pub mod bookings;
pub mod claim;
pub mod claim_approval;
pub mod claim_machine;
pub mod claim_split;
pub mod claims;
mod db;
pub mod estimate;
pub mod insurer;
pub mod insurers;
pub mod job_card;
pub mod job_card_machine;
pub mod job_cards;
pub mod labour_line;
pub mod labour_menu;
pub mod labour_operation;
pub mod pages;
pub mod part_line;
pub mod parts;
pub mod permissions;
pub mod reasons;
pub mod seed;
mod texts;
mod texts_claims;
mod texts_jobs;
mod texts_vehicles;
pub mod vehicle;
pub mod vehicles;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use approval::{ApprovalStatus, EstimateApproval};
pub use claim::{ClaimStatus, InsuranceClaim, RepairType};
pub use claim_approval::{ApprovalKind, ApprovalOutcome, ApprovedLine, ClaimApproval};
pub use insurer::Insurer;
pub use job_card::{JobCard, JobCardStatus};
pub use labour_line::{LabourLine, LabourPricing, LineStatus, WorkStatus};
pub use labour_operation::LabourOperation;
pub use part_line::{PartCondition, PartLine};
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
            .view_data::<pages::insurers::ShowPage>();
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
            .model::<ClaimApproval>();
        // The job card: the machine, and the job card that the check-in of a
        // workshop booking opens.
        job_card_machine::register();
        bookings::register(app);
        // The insurance claim: the machine, and the listeners that settle it
        // when its invoices are paid.
        claim_machine::register();
        claims::register_listeners(app);
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
