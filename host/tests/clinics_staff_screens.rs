//! The staff screens of the Clinics client through the real host: who reaches
//! them and what the sidebar links (BL-SCH-02, BL-SCH-03).

mod support;

use host::roles::{MEMBER, RECEPTIONIST};
use support::{clinics_booted, person};

/// The receptionist books through the picker and works the waiting list, and
/// the sidebar links both; a member with no role reaches neither.
#[renox::test]
async fn the_receptionist_books_and_keeps_the_waiting_list_and_a_member_cannot() {
    let app = clinics_booted().await;
    let rita = person(&app, "Rita", RECEPTIONIST).await;
    let nobody = person(&app, "Nora", MEMBER).await;

    app.acting_as(&rita);
    app.get("/scheduling/bookings/new").await.assert_ok();
    app.get("/scheduling/waiting-list")
        .await
        .assert_ok()
        .assert_see("Waiting list");
    app.get("/dashboard")
        .await
        .assert_ok()
        .assert_see("Waiting list");

    app.acting_as(&nobody);
    app.get("/scheduling/bookings/new").await.assert_forbidden();
    app.get("/scheduling/waiting-list").await.assert_forbidden();
}
