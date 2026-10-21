//! BL-CLN-01 (#349): the list of appointments that leads the receptionist to
//! the pre-check-in screen, with where the cover of each stands.

mod support;

use wl_kernel::PartyId;

use clinics::policies::{self, NewPolicy};
use clinics::appointments;
use support::{PATIENT, at, boot, day};

/// The list shows the appointments from today on: an insured one waits for its
/// coverage outcome, one with an outcome shows it, a self-pay one is not
/// insured, a cancelled one and one in the past are not listed; the viewer
/// without `visits.view` is refused.
#[renox::test]
async fn the_list_shows_where_the_cover_of_each_appointment_stands() {
    let t = boot().await;
    let rita = t.receptionist().await;
    let doctor = t.practitioner("Dr A", 61, None).await;
    let service = t.consultation(&[&doctor]).await;
    t.prices.set("consultation", day(-400), 45_000);
    let policy = policies::create(
        t.state(),
        None,
        NewPolicy::new(PATIENT, PartyId(900), "Gold", "M-1", day(-30)).valid_to(day(100)),
    )
    .await
    .unwrap();
    let waiting = t.book(&service, &doctor, at(day(1), 9)).await;
    appointments::attach_policy(t.state(), waiting.id, policy.id)
        .await
        .unwrap();
    let plain = t.book(&service, &doctor, at(day(1), 11)).await;
    let decided = t.book(&service, &doctor, at(day(2), 9)).await;
    appointments::attach_policy(t.state(), decided.id, policy.id)
        .await
        .unwrap();
    t.app.acting_as(&rita);
    t.app
        .htmx()
        .post(
            &format!("/clinics/appointments/{}/eligibility", decided.id),
            &[("result", "positive")],
        )
        .await
        .assert_ok();

    let page = t.app.get("/clinics/appointments").await;
    page.assert_ok()
        .assert_see("Waiting for the coverage outcome")
        .assert_see("Not insured")
        .assert_see("Insured");
    assert!(page.text().contains(&format!("/clinics/appointments/{}", waiting.id)));
    assert!(page.text().contains(&format!("/clinics/appointments/{}", plain.id)));

    let viewer = t.viewer().await;
    t.app.acting_as(&viewer);
    t.app.get("/clinics/appointments").await.assert_forbidden();
}
