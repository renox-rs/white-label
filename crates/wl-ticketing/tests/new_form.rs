//! The staff form that raises a ticket offers the kinds of record that can be
//! named instead of asking for the type as text.

mod support;

use wl_kernel::documents::register_subject_label;

use support::boot;

#[renox::test]
async fn the_form_offers_the_record_types_that_have_a_name() {
    register_subject_label("tickettest.thing", |_db, id| {
        Box::pin(async move { Ok(Some(format!("Thing {id}"))) })
    });
    let t = boot().await;
    let carla = t.coordinator().await;
    t.app.acting_as(&carla);
    t.app
        .get("/tickets/new")
        .await
        .assert_ok()
        .assert_see("Tickettest thing")
        .assert_see("Number of the record");
}
