//! The base of the 10 EdTech app (base for #436 .. #453): the module boots
//! with the real scheduling and HR, its permissions and role names are the
//! spec's, and its starter tables are what the host installs.

mod support;

use edtech::{permissions, reasons, seed};
use support::boot;

/// The app boots next to wl-scheduling and wl-hr and the pages' layout.
#[renox::test]
async fn edu_base_the_app_boots_with_scheduling_and_hr() {
    let t = boot().await;
    t.app.get("/health").await.assert_ok();
}

/// The role names the rules use are the roles of 10 §3.1.
#[test]
fn edu_base_the_roles_are_the_spec_roles() {
    assert_eq!(permissions::OWNER_ROLE, "owner");
    assert_eq!(permissions::CENTRE_MANAGER_ROLE, "centre_manager");
    assert_eq!(permissions::COUNSELLOR_ROLE, "counsellor");
    assert_eq!(permissions::COORDINATOR_ROLE, "academic_coordinator");
    assert_eq!(permissions::TUTOR_ROLE, "tutor");
    assert_eq!(permissions::ACCOUNTANT_ROLE, "accountant");
}

/// A tutor business-cancelled booking refunds in full: the reason codes 10
/// adds are the centre's side.
#[test]
fn edu_base_the_tutor_reason_codes_are_the_centres_side() {
    let codes: Vec<&str> = reasons::SCHEDULING_REASON_CODES
        .iter()
        .map(|(code, _)| *code)
        .collect();
    assert_eq!(codes, ["tutor_unavailable", "tutor_ineligible"]);
}

/// The starter service types are the trial, the placement test and the
/// one-to-one session (§7.1 *Booking*).
#[test]
fn edu_base_the_starter_service_types() {
    let codes: Vec<&str> = seed::SERVICE_TYPES.iter().map(|s| s.code).collect();
    assert_eq!(codes, ["trial_class", "placement_test", "one_to_one"]);
}
