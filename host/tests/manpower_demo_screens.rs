//! The screen map of the demo agency (`host::manpower_demo::screen_map`, the
//! map `manpower:demo` prints): every screen built in BL-MPW-22 to BL-MPW-32
//! opens for the role that owns it, says words and not keys, and holds on a
//! phone (every table in its own wrapper).

mod manpower_screens_support;
mod support;

use manpower_screens_support::*;

/// Every screen of the map answers 200 for its owner, shows visible words
/// only, and keeps its tables in their wrappers (390 px).
#[renox::test]
async fn every_screen_of_the_map_opens_for_its_owner_in_words() {
    let app = demo().await;
    let screens = host::manpower_demo::screen_map(app.state()).await.unwrap();
    assert!(screens.len() >= 40, "{} screens", screens.len());

    // The groups the issues BL-MPW-22 to BL-MPW-32 built are all there.
    for group in [
        "Start",
        "Contracts",
        "Rate cards",
        "Requests",
        "Deployments",
        "Absence",
        "Timesheets",
        "Payroll",
        "Workers",
        "Client portal (Gulf Tower approver)",
    ] {
        assert!(
            screens.iter().any(|screen| screen.group == group),
            "no screen in `{group}`"
        );
    }
    // A record page of each kind has its real id.
    for prefix in [
        "/manpower/contracts/",
        "/rate-cards/",
        "/rate-card-versions/",
        "/manpower/requests/",
        "/manpower/deployments/",
        "/manpower/absence/",
        "/timesheets/",
        "/hr/payroll/",
        "/hr/settlements/",
        "/hr/pay-accounts/",
        "/hr/employees/",
    ] {
        assert!(
            screens.iter().any(|screen| screen.url.starts_with(prefix)),
            "no screen at {prefix}<id>"
        );
    }

    for screen in &screens {
        let response = browse(&app, screen.login, &screen.url).await;
        assert_eq!(
            response.status.as_u16(),
            200,
            "{} as {} ({})",
            screen.url,
            screen.login,
            screen.what
        );
        let html = response.text();
        no_raw_words(&html);
        fits_a_phone(&html);
        assert!(!screen.what.is_empty());
    }
}
