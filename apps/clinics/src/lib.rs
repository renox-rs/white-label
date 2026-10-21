#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod appointment;
pub mod appointments;
pub mod auth_check;
pub mod authorisation;
pub mod authorisation_machine;
pub mod authorisations;
pub mod billing;
pub mod card_requests;
pub mod charge;
pub mod charges;
pub mod claim;
pub mod claim_closing;
pub mod claim_machine;
pub mod claim_money;
pub mod claim_rules;
pub mod claim_void;
pub mod claim_window;
pub mod claims;
pub mod consent;
pub mod consumable_issue;
pub mod contract;
pub mod contracts;
pub mod coverage;
mod db;
pub mod flags;
pub mod licences;
pub mod lots;
pub mod pages;
pub mod permissions;
pub mod policies;
pub mod policy;
pub mod prior_auth_rules;
pub mod reasons;
pub mod recall_notice;
pub mod recalls;
pub mod seed;
pub mod split;
mod texts;
pub mod visit;
pub mod visit_machine;
pub mod visits;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use appointment::Appointment;
pub use authorisation::{AuthStatus, AuthorisationHold, PriorAuthService, PriorAuthorisation};
pub use charge::{ChargeKind, ChargeLine};
pub use claim::{
    AnswerKind, AttemptKind, Claim, ClaimAttempt, ClaimError, ClaimLine, ClaimRemittance,
    ClaimStatus,
};
pub use consent::{ConsentLine, SelfPayConsent};
pub use consumable_issue::ConsumableIssue;
pub use contract::{PayerContract, PayerTariff};
pub use coverage::{CoverageOutcome, PendingCheckIn};
pub use flags::{AppointmentFlag, FlagCause};
pub use licences::LicenceAlert;
pub use policy::{EligibilityCheck, InsurancePolicy};
pub use recall_notice::{RecallNotice, RecallNoticeStatus};
pub use visit::{Visit, VisitStatus};

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory (`resources/views/clinics/policies.html`).
const VIEWS: &[(&str, &str)] = &[
    (
        "clinics/policies.html",
        include_str!("../views/clinics/policies.html"),
    ),
    (
        "clinics/appointments.html",
        include_str!("../views/clinics/appointments.html"),
    ),
    (
        "clinics/appointment.html",
        include_str!("../views/clinics/appointment.html"),
    ),
    (
        "clinics/visit.html",
        include_str!("../views/clinics/visit.html"),
    ),
    (
        "clinics/contracts.html",
        include_str!("../views/clinics/contracts.html"),
    ),
    (
        "clinics/authorisations.html",
        include_str!("../views/clinics/authorisations.html"),
    ),
    (
        "clinics/authorisation.html",
        include_str!("../views/clinics/authorisation.html"),
    ),
    (
        "clinics/claims.html",
        include_str!("../views/clinics/claims.html"),
    ),
    (
        "clinics/claim.html",
        include_str!("../views/clinics/claim.html"),
    ),
    (
        "clinics/recall.html",
        include_str!("../views/clinics/recall.html"),
    ),
    (
        "clinics/recall_calls.html",
        include_str!("../views/clinics/recall_calls.html"),
    ),
];

/// Dirhams as text from fils: `AED 450.00`.
pub fn money_text(fils: i64) -> String {
    let sign = if fils < 0 { "-" } else { "" };
    let magnitude = fils.unsigned_abs();
    let whole = (magnitude / 100).to_string();
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{sign}AED {grouped}.{:02}", magnitude % 100)
}

/// The Clinics module. Add it to the app with `.module(clinics::Clinics)`.
pub struct Clinics;

impl Module for Clinics {
    fn name(&self) -> &'static str {
        "clinics"
    }

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<pages::policies::IndexPage>()
            .view_data::<pages::appointments::IndexPage>()
            .view_data::<pages::appointments::ShowPage>()
            .view_data::<pages::visits::ShowPage>()
            .view_data::<pages::contracts::IndexPage>()
            .view_data::<pages::authorisations::IndexPage>()
            .view_data::<pages::authorisations::ShowPage>()
            .view_data::<pages::claims::IndexPage>()
            .view_data::<pages::claims::ShowPage>()
            .view_data::<pages::recalls::ShowPage>()
            .view_data::<pages::recalls::CallsPage>();
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        app.model::<Appointment>()
            .model::<Visit>()
            .model::<InsurancePolicy>()
            .model::<EligibilityCheck>()
            .model::<LicenceAlert>()
            .model::<PendingCheckIn>()
            .model::<SelfPayConsent>()
            .model::<PriorAuthorisation>()
            .model::<PriorAuthService>()
            .model::<AuthorisationHold>()
            .model::<AppointmentFlag>()
            .model::<ChargeLine>()
            .model::<ConsumableIssue>()
            .model::<RecallNotice>()
            .model::<PayerContract>()
            .model::<PayerTariff>()
            .model::<Claim>()
            .model::<ClaimLine>()
            .model::<ClaimAttempt>()
            .model::<ClaimRemittance>();
        visit_machine::register();
        // The claim: its machine, its subject (`clinics.claim`) with the
        // void's own approval action, and voidable (`CLN.ACL.14`).
        claim_machine::register();
        // The clinic's record follows the booking (`BookingBooked`,
        // `BookingRescheduled`, `BookingCancelled`).
        appointments::register(app);
        // The check-in opens the visit, which follows the booking.
        visits::register(app);
        // What the visit did, for wl-scheduling's cancel after the check-in
        // (`SCH.ACL.06`): answered from the charge lines (BL-CLN-05).
        wl_kernel::scheduling::provide_work_done_status(app, charges::ChargeWork);
        // `STK.LOT.01`: a visit with a lot-tracked consumable that has no lot
        // is not checked out (BL-CLN-13).
        wl_kernel::scheduling::provide_complete_guard(app, lots::LotGuard);
        // `CLN.ACL.15`: an insured visit is checked in only with its coverage
        // outcome (BL-CLN-01).
        coverage::register(app);
        // The prior authorisation, its notice to the patient and the daily
        // expiry (BL-CLN-02, BL-CLN-03).
        authorisations::register(app);
        // `CLN.RUL.07`, `CLN.RUL.09`: the appointments that need an
        // authorisation are flagged (BL-CLN-03).
        prior_auth_rules::register(app);
        // `CLN.RUL.10`: a policy that ends or a card that expired before the
        // visit is asked for again (BL-CLN-04).
        card_requests::register(app);
        // The claim follows the money its payer bill records (BL-CLN-08), and
        // its tasks: the technical rejection and `CLN.RUL.05` (BL-CLN-07,
        // BL-CLN-08).
        claim_money::register(app);
        claim_rules::register(app);
        // `CLN.ACL.16`: the write-off of a claim, and the claim following the
        // owner's approval of the payer bill's (BL-CLN-09).
        claim_closing::register(app);
        // `CLN.RUL.03`: the claims not sent before the first-submission
        // window ends are chased (BL-CLN-11).
        claim_window::register(app);
        // `CLN.RUL.01`: an expired licence or set of privileges is told to
        // the clinic manager of the branch and the owner (BL-CLN-12).
        licences::register(app);
        // A recalled lot traced to patients, the neutral notice and the call
        // task when it gives up (BL-CLN-14).
        lots::register(app);
        recalls::register(app);
        // In the app group of the staff sidebar; the label is the host's
        // `nav.insurance_policies` text.
        app.nav(
            "app",
            NavLink::route("clinics.appointments", "nav.clinic_appointments")
                .icon("clipboard-list")
                .group("nav.app")
                .order(71)
                .permission(permissions::VISITS_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("clinics.authorisations", "nav.prior_authorisations")
                .icon("file-text")
                .group("nav.app")
                .order(72)
                .permission(permissions::POLICIES_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("clinics.contracts", "nav.payer_contracts")
                .icon("receipt")
                .group("nav.app")
                .order(73)
                .permission(permissions::POLICIES_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("clinics.claims", "nav.claims")
                .icon("receipt")
                .group("nav.app")
                .order(74)
                .permission(permissions::CLAIMS_MANAGE),
        );
        app.nav(
            "app",
            NavLink::route("clinics.recall_calls", "nav.recall_calls")
                .icon("clipboard-list")
                .group("nav.app")
                .order(75)
                .permission(permissions::RECALLS_CALL),
        );
        app.nav(
            "app",
            NavLink::route("clinics.policies", "nav.insurance_policies")
                .icon("shield-check")
                .group("nav.app")
                .order(70)
                .permission(permissions::POLICIES_VIEW),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_reads_as_dirhams_with_grouped_thousands() {
        assert_eq!(money_text(45_000), "AED 450.00");
        assert_eq!(money_text(5), "AED 0.05");
        assert_eq!(money_text(-123_456_789), "-AED 1,234,567.89");
        assert_eq!(money_text(0), "AED 0.00");
    }
}
