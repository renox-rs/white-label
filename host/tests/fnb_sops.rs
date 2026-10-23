//! BL-FNB-17 (#381), 06 §5, `HRM.ONB.01`, `DOC.RUL.04`, `HRM.ACL.10`: the SOP
//! library of a fresh 06 deployment through the real wl-documents and wl-hr:
//! a version of the food safety SOP published by its owner (the head chef)
//! asks the line cook for an acknowledgement due in 7 days; a newer version
//! replaces the acknowledgement of the older one and asks again.

mod support;

use host::roles::{HEAD_CHEF, LINE_COOK};
use renox::chrono::Days;
use renox::prelude::*;
use support::*;
use wl_documents::acknowledgements::run::run;
use wl_documents::{AckStatus, Document, Sop, SopAcknowledgement};
use wl_kernel::documents::today;

/// The head chef uploads a version of `sop` and accepts it; its document.
async fn publish(app: &renox::testing::TestApp, chef: &User, sop: &Sop, version: u8) -> Document {
    app.acting_as(chef);
    app.htmx()
        .post_multipart(
            &format!("/documents/sops/{}/versions", sop.id),
            &[("note", "Revised")],
            &[(
                "files",
                "food-safety.pdf",
                format!("%PDF-1.4 version {version}").as_bytes(),
            )],
        )
        .await
        .assert_ok();
    let document = Document::query()
        .order_by_desc(Document::ID)
        .first(app.db())
        .await
        .unwrap()
        .expect("the version was uploaded");
    app.htmx()
        .post(
            &format!("/documents/sop-versions/{}/accept", document.id),
            &[("review_date", "")],
        )
        .await
        .assert_ok();
    Document::find_or_404(app.db(), document.id).await.unwrap()
}

#[renox::test]
async fn a_new_version_of_the_food_safety_sop_asks_the_line_cook_again() {
    let app = fnb_booted().await;
    let chef = person(&app, "Samir", HEAD_CHEF).await;
    let cook = person(&app, "Rizwan", LINE_COOK).await;
    let party = tenant(&app, "Rizwan Ahmed").await;
    let employee = wl_hr::employees::create(
        app.state(),
        wl_hr::NewEmployee::new(party, "line_cook", today()).user(cook.id),
    )
    .await
    .unwrap();
    let sop = Sop::query()
        .where_(Sop::CODE.eq("SOP-FNB-FOOD-SAFETY"))
        .first(app.db())
        .await
        .unwrap()
        .expect("the SOP library is seeded");
    assert_eq!(sop.ack_due_days, 7);

    let first = publish(&app, &chef, &sop, 1).await;
    run(app.state(), today()).await.unwrap();
    let ack = SopAcknowledgement::query()
        .where_(SopAcknowledgement::EMPLOYEE_ID.eq(employee.id))
        .where_(SopAcknowledgement::DOCUMENT_ID.eq(first.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("the line cook is asked to acknowledge the first version");
    assert_eq!(ack.status, AckStatus::Pending);
    assert_eq!(ack.due_date, today() + Days::new(7));

    let second = publish(&app, &chef, &sop, 2).await;
    run(app.state(), today()).await.unwrap();
    let old = SopAcknowledgement::find_or_404(app.db(), ack.id).await.unwrap();
    assert_eq!(old.status, AckStatus::Replaced, "the older version is replaced");
    let again = SopAcknowledgement::query()
        .where_(SopAcknowledgement::EMPLOYEE_ID.eq(employee.id))
        .where_(SopAcknowledgement::DOCUMENT_ID.eq(second.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("the line cook is asked again for the new version");
    assert_eq!(again.status, AckStatus::Pending);

    // The cook acknowledges only what they opened.
    app.acting_as(&cook);
    let path = format!("/documents/acknowledgements/{}/acknowledge", again.id);
    app.htmx().post(&path, &[("x", "")]).await.assert_status(422);
    app.get(&format!(
        "/documents/acknowledgements/{}/files/{}",
        again.id, second.id
    ))
    .await
    .assert_ok();
    app.htmx().post(&path, &[("x", "")]).await.assert_ok();
    let done = SopAcknowledgement::find_or_404(app.db(), again.id).await.unwrap();
    assert_eq!(done.status, AckStatus::Acknowledged);
}
