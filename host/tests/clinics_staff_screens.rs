//! The staff screens of the Clinics client through the real host: who reaches
//! them and what the sidebar links (BL-SCH-02, BL-SCH-03).

mod support;

use host::roles::{CLINIC_MANAGER, MEMBER, OWNER, RECEPTIONIST, STOREKEEPER};
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

/// The storekeeper reaches the stock screens and the sidebar links each of
/// them; the receptionist, who holds no stock permission, reaches none.
#[renox::test]
async fn the_storekeeper_reaches_the_stock_screens_and_the_receptionist_does_not() {
    let app = clinics_booted().await;
    let kim = person(&app, "Kim", STOREKEEPER).await;
    let rita = person(&app, "Rita", RECEPTIONIST).await;

    app.acting_as(&kim);
    for path in [
        "/stock",
        "/stock/holds",
        "/stock/transfers",
        "/purchase-orders",
        "/goods-receipts",
    ] {
        app.get(path).await.assert_ok();
    }
    app.request()
        .header("accept", "text/html")
        .get("/pending-issues")
        .await
        .assert_ok()
        .assert_see("Pending issues");
    let sidebar = app.get("/dashboard").await;
    for link in [
        "Stock",
        "Receive stock",
        "Transfers",
        "Stock holds",
        "Pending issues",
    ] {
        sidebar.assert_see(link);
    }

    app.acting_as(&rita);
    for path in [
        "/stock",
        "/stock/holds",
        "/stock/transfers",
        "/purchase-orders",
    ] {
        app.get(path).await.assert_forbidden();
    }
    app.get("/dashboard")
        .await
        .assert_ok()
        .assert_dont_see("Receive stock");
}

/// The clinic manager follows the recalls (the list and the page, which links
/// to the patients and notices) and the owner decides them; the receptionist
/// reaches neither.
#[renox::test]
async fn the_clinic_manager_follows_the_recalls_and_the_receptionist_does_not() {
    let app = clinics_booted().await;
    let mona = person(&app, "Mona", CLINIC_MANAGER).await;
    let olga = person(&app, "Olga", OWNER).await;
    let rita = person(&app, "Rita", RECEPTIONIST).await;

    for user in [&mona, &olga] {
        app.acting_as(user);
        app.get("/recalls").await.assert_ok();
        app.get("/dashboard")
            .await
            .assert_ok()
            .assert_see("Recalls");
    }
    app.acting_as(&rita);
    app.get("/recalls").await.assert_forbidden();
    app.get("/dashboard")
        .await
        .assert_ok()
        .assert_dont_see(">Recalls<");
}
