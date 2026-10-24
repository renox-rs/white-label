//! The job card pages (base for #397 .. #403): the list with its filters, the
//! check-in form, the job card with its actions, the estimate sheet, the
//! page the customer answers on without signing in, the technician's list and
//! the labour menu.

mod support;

use auto::job_cards::{self, NewLabour, NewPart, Promise};
use auto::{
    ApprovalStatus, EstimateApproval, JobCard, JobCardStatus, LabourOperation, permissions,
};
use renox::prelude::*;
use support::{Garage, boot};

/// The advisor Sami, signed in.
async fn sign_in_advisor(t: &Garage) -> User {
    let user = t.advisor_user().await;
    t.app.acting_as(&user);
    user
}

/// A job card in `inspecting` with a priced part and labour, assigned to Tareq.
async fn inspecting(t: &Garage) -> JobCard {
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::Inspecting).await;
    let tareq = t.technician("Tareq").await;
    job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Brake pad replacement", 2_000, 15_000),
    )
    .await
    .unwrap();
    job_cards::add_part(
        t.state(),
        &tareq,
        card.id,
        NewPart::new("Brake pad set", 1_000, 30_000),
    )
    .await
    .unwrap();
    card
}

/// The list shows the job card with its plate and status in words, the status
/// tabs and the search narrow it, and a user with no job card permission is
/// refused.
#[renox::test]
async fn aut_base_the_list_shows_filters_and_searches_the_job_cards() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let open = t.card_in(&vehicle, JobCardStatus::Inspecting).await;
    let closed = t.card_in(&vehicle, JobCardStatus::Declined).await;
    sign_in_advisor(&t).await;

    t.app
        .get("/auto/job-cards")
        .await
        .assert_ok()
        .assert_see(&open.number())
        .assert_see(&closed.number())
        .assert_see("Dubai A 12345")
        .assert_see("Inspecting");
    let only_open = t.app.get("/auto/job-cards?in.status=inspecting").await;
    only_open.assert_ok().assert_see(&open.number());
    assert!(!only_open.text().contains(&closed.number()));
    let only_closed = t.app.get("/auto/job-cards?in.status=declined").await;
    only_closed.assert_ok().assert_see(&closed.number());
    assert!(!only_closed.text().contains(&open.number()));
    let found = t.app.get("/auto/job-cards?search=12345").await;
    found.assert_ok().assert_see(&open.number());
    let none = t.app.get("/auto/job-cards?search=ferrari").await;
    assert!(!none.text().contains(&open.number()));

    let nobody = t.user("Nadia", &[permissions::VEHICLES_VIEW]).await;
    t.app.acting_as(&nobody);
    t.app.get("/auto/job-cards").await.assert_status(403);
}

/// The check-in form picks the vehicle from the register and opens the job
/// card with the intake; a form without the condition is refused in place.
#[renox::test]
async fn aut_base_the_check_in_form_opens_a_job_card() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    sign_in_advisor(&t).await;
    let id = vehicle.id.to_string();

    let found = t.app.get("/auto/vehicles/options?q=hilux").await;
    found.assert_ok().assert_see("Dubai A 12345");

    let refused = t
        .app
        .htmx()
        .post(
            "/auto/job-cards",
            &[
                ("vehicle", id.as_str()),
                ("complaint", "Noise"),
                ("odometer", "52,400"),
            ],
        )
        .await;
    refused.assert_status(422);
    assert!(refused.text().contains("condition"));

    t.app
        .htmx()
        .post(
            "/auto/job-cards",
            &[
                ("vehicle", id.as_str()),
                ("complaint", "Brake noise"),
                ("odometer", "52,400"),
                ("fuel", "half"),
                ("condition", "Scratch on the left door"),
                ("acknowledged_by", "Mona Hassan"),
                ("inspection_fee", "150"),
            ],
        )
        .await
        .assert_ok();
    let card = JobCard::query()
        .first(t.db())
        .await
        .unwrap()
        .expect("the job card");
    assert_eq!(
        (card.status, card.odometer_in, card.inspection_fee),
        (JobCardStatus::CheckedIn, 52_400, 15_000)
    );
    assert_eq!(card.fuel_level.as_deref(), Some("half"));
}

/// The job card page shows the intake, the estimate lines with the condition of
/// each part, the totals with VAT, and the actions by state; the estimate
/// sheet and the customer's link follow the estimate that was sent.
#[renox::test]
async fn aut_base_the_job_card_page_and_the_estimate_sheet() {
    let t = boot().await;
    let card = inspecting(&t).await;
    let sami = t.advisor().await;
    sign_in_advisor(&t).await;

    let page = t.app.get(&format!("/auto/job-cards/{}", card.id)).await;
    page.assert_ok()
        .assert_see(&card.number())
        .assert_see("Scratch on the left door")
        .assert_see("Brake pad replacement")
        .assert_see("Brake pad set")
        .assert_see("AED 300.00")
        .assert_see("AED 600.00")
        .assert_see("Send the estimate");

    job_cards::send_estimate(t.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();
    let round = job_cards::pending_round(t.db(), card.id)
        .await
        .unwrap()
        .unwrap();
    t.app.acting_as(&t.advisor_user().await);
    let page = t.app.get(&format!("/auto/job-cards/{}", card.id)).await;
    page.assert_ok()
        .assert_see("Awaiting approval")
        .assert_see(&round.path())
        .assert_see("Record the approval");
    t.app
        .get(&format!("/auto/job-cards/{}/estimate", card.id))
        .await
        .assert_ok()
        .assert_see("Estimate sheet")
        .assert_see("AED 600.00")
        .assert_see("AED 630.00")
        .assert_see(&round.path());
}

/// The customer opens the link without signing in, reads the lines, and
/// approves with the name and the consent: the job card goes on and the
/// consent is recorded.
#[renox::test]
async fn aut_base_the_customer_approves_on_the_page_without_signing_in() {
    let t = boot().await;
    let card = inspecting(&t).await;
    let sami = t.advisor().await;
    job_cards::send_estimate(t.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();
    let round = job_cards::pending_round(t.db(), card.id)
        .await
        .unwrap()
        .unwrap();
    t.app.logout();

    t.app
        .get(&round.path())
        .await
        .assert_ok()
        .assert_see("Your estimate")
        .assert_see("Brake pad set")
        .assert_see("New")
        .assert_see("AED 630.00")
        .assert_see("15 days");
    t.app.get("/auto/approve/nobody").await.assert_status(404);

    let no_consent = t
        .app
        .htmx()
        .post(&round.path(), &[("name", "Mona Hassan")])
        .await;
    no_consent.assert_status(422);
    assert!(no_consent.text().contains("Tick the box"));

    t.app
        .htmx()
        .post(
            &round.path(),
            &[
                ("name", "Mona Hassan"),
                ("signature", "Mona Hassan"),
                ("consent", "on"),
            ],
        )
        .await
        .assert_ok();
    let after = t.reload(card.id).await;
    assert_eq!(after.status, JobCardStatus::InProgress);
    let answered = EstimateApproval::find_or_404(t.db(), round.id)
        .await
        .unwrap();
    assert_eq!(answered.status, ApprovalStatus::Approved);
    assert_eq!(answered.method.as_deref(), Some("link"));
    t.app
        .get(&round.path())
        .await
        .assert_ok()
        .assert_see("Approved · Mona Hassan");
}

/// The customer declines on the page: the whole estimate ends the job card
/// when nothing was approved.
#[renox::test]
async fn aut_base_the_customer_declines_on_the_page() {
    let t = boot().await;
    let card = inspecting(&t).await;
    let sami = t.advisor().await;
    job_cards::send_estimate(t.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();
    let round = job_cards::pending_round(t.db(), card.id)
        .await
        .unwrap()
        .unwrap();
    t.app.logout();

    t.app
        .htmx()
        .post(
            &format!("{}/decline", round.path()),
            &[("name", "Mona Hassan"), ("reason", "Too dear")],
        )
        .await
        .assert_ok();
    let after = t.reload(card.id).await;
    assert_eq!(after.status, JobCardStatus::Declined);
    assert_eq!(after.closed_reason.as_deref(), Some("Too dear"));
}

/// The advisor's actions on the page: record the approval, decline, void and
/// cancel run the machine and answer in place.
#[renox::test]
async fn aut_base_the_advisor_actions_run_the_machine() {
    let t = boot().await;
    let card = inspecting(&t).await;
    let sami = t.advisor().await;
    job_cards::send_estimate(t.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();
    t.app.acting_as(&t.advisor_user().await);
    let base = format!("/auto/job-cards/{}", card.id);

    t.app
        .htmx()
        .post(
            &format!("{base}/approve"),
            &[
                ("name", "Mona Hassan"),
                ("method", "signature"),
                ("signature", "M. Hassan"),
            ],
        )
        .await
        .assert_ok();
    let approved = t.reload(card.id).await;
    assert_eq!(approved.status, JobCardStatus::InProgress);
    let round = EstimateApproval::of_job_card(t.db(), card.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(round.method.as_deref(), Some("signature"));
    assert_eq!(round.decided_by_user, Some(sami.id));

    let unsigned = t
        .app
        .htmx()
        .post(&format!("{base}/cancel"), &[("code", "customer_stopped")])
        .await;
    unsigned.assert_status(422);
    assert!(
        unsigned.text().contains("acknowledged"),
        "{}",
        unsigned.text()
    );
    t.app
        .htmx()
        .post(
            &format!("{base}/cancel"),
            &[
                ("code", "customer_stopped"),
                ("ack", "signed"),
                ("ack_name", "Mona Hassan"),
            ],
        )
        .await
        .assert_ok();
    assert_eq!(t.reload(card.id).await.status, JobCardStatus::Cancelled);
    t.app
        .get(&base)
        .await
        .assert_ok()
        .assert_see("Cancelled")
        .assert_see("The customer stopped the job");
}

/// The labour menu page lists the operations and adds one; the menu is for the
/// holder of `labour_menu.manage`.
#[renox::test]
async fn aut_base_the_labour_menu_lists_and_adds_operations() {
    let t = boot().await;
    let waleed = t.qc_user().await;
    t.app.acting_as(&waleed);

    t.app
        .htmx()
        .post(
            "/auto/labour-menu",
            &[
                ("code", "brk-pads"),
                ("name_en", "Front brake pads"),
                ("pricing", "flat_rate"),
                ("hours", "1.5"),
                ("rate", "180"),
            ],
        )
        .await
        .assert_ok();
    let bad = t
        .app
        .htmx()
        .post(
            "/auto/labour-menu",
            &[
                ("code", "BRK-PADS"),
                ("name_en", "Again"),
                ("pricing", "hourly"),
                ("rate", "150"),
            ],
        )
        .await;
    bad.assert_status(422);
    assert!(bad.text().contains("on the menu already"));

    let stored = LabourOperation::by_code(t.db(), "BRK-PADS")
        .await
        .unwrap()
        .expect("on the menu");
    assert_eq!((stored.hours_milli, stored.rate), (1_500, 18_000));
    t.app
        .get("/auto/labour-menu")
        .await
        .assert_ok()
        .assert_see("BRK-PADS")
        .assert_see("Front brake pads")
        .assert_see("AED 180.00")
        .assert_see("1.5");

    let sami = t.advisor_user().await;
    t.app.acting_as(&sami);
    t.app.get("/auto/labour-menu").await.assert_ok();
    t.app
        .htmx()
        .post(
            "/auto/labour-menu",
            &[
                ("code", "X"),
                ("name_en", "X"),
                ("rate", "1"),
                ("hours", "1"),
            ],
        )
        .await
        .assert_status(403);
}

/// The vehicle page lists the job cards of the vehicle.
#[renox::test]
async fn aut_base_the_vehicle_page_lists_its_job_cards() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::Ready).await;
    sign_in_advisor(&t).await;
    t.app
        .get(&format!("/auto/vehicles/{}", vehicle.id))
        .await
        .assert_ok()
        .assert_see(&card.number())
        .assert_see("Ready for collection");
}
