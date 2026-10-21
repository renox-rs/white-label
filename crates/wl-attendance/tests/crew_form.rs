//! The crew list page without its parameters: a form to pick the work
//! location, the day and the crew, in place of an error.

mod support;

use wl_attendance::WorkLocation;

use support::boot;

#[renox::test]
async fn the_crew_page_without_parameters_is_a_form_of_the_locations_one_may_mark_at() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let nobody = t.user_with("Nadia", "reviewer").await;
    t.location_of(WorkLocation::named("Marina Tower site"))
        .await;

    t.app.acting_as(&boss);
    let page = t.app.get("/attendance/crew").await;
    page.assert_ok();
    let html = page.text();
    assert!(html.contains("Crew list"), "{html}");
    assert!(html.contains("Work location"), "the location field: {html}");
    assert!(html.contains("Marina Tower site"), "a location to pick");
    assert!(html.contains("Show the crew list"), "the button");
    assert!(html.contains("method=\"get\"") || html.contains("method=\"GET\""));

    // A location without a day is asked again, with the reason.
    let again = t.app.get("/attendance/crew?location_id=1&date=soon").await;
    again.assert_ok();
    assert!(again.text().contains("Give the day as"), "{}", again.text());
    let again = t.app.get("/attendance/crew?date=2027-07-12").await;
    again.assert_ok();
    assert!(
        again.text().contains("Choose a work location"),
        "{}",
        again.text()
    );

    // Whoever may mark nowhere is told so, and sees no location.
    t.app.acting_as(&nobody);
    let page = t.app.get("/attendance/crew").await;
    page.assert_ok();
    let html = page.text();
    assert!(html.contains("may not mark attendance"), "{html}");
    assert!(!html.contains("Marina Tower site"), "{html}");
}
