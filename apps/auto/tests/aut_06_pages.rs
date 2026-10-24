//! BL-AUT-06 (#402) pages and police report pages (#403): the job card page
//! shows the test drives, the incidents and the accident damage with their
//! forms, and the forms' routes refuse in words inside the form.

mod support;

use auto::JobCardStatus;
use auto::incident::Incident;
use auto::permissions as aut;
use auto::test_drive::TestDrive;
use renox::prelude::*;
use support::boot;

/// The technician records a test drive and reports an incident through the
/// forms; the branch manager decides it through the form; the page shows each
/// step in words.
#[renox::test]
async fn aut_06_the_job_card_page_records_drives_and_decides_incidents() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    let tareq = t.technician_user("Tareq").await;
    t.app.acting_as(&tareq);
    t.app
        .htmx()
        .post(
            &format!("/auto/job-cards/{}/test-drives", card.id),
            &[("odometer", "52,100")],
        )
        .await
        .assert_status(200);
    let drives = TestDrive::of_job_card(t.db(), card.id).await.unwrap();
    assert_eq!(drives.len(), 1);
    t.app
        .get(&format!("/auto/job-cards/{}", card.id))
        .await
        .assert_ok()
        .assert_see("Test drives")
        .assert_see("Still out");
    let low = t
        .app
        .htmx()
        .post(
            &format!("/auto/test-drives/{}/finish", drives[0].id),
            &[("odometer", "52,000")],
        )
        .await;
    low.assert_status(422);
    assert!(low.text().contains("below"));
    t.app
        .htmx()
        .post(
            &format!("/auto/test-drives/{}/finish", drives[0].id),
            &[("odometer", "52,112")],
        )
        .await
        .assert_status(200);

    t.app
        .htmx()
        .post(
            &format!("/auto/job-cards/{}/incidents", card.id),
            &[
                ("description", "Scratch on the door"),
                ("road_accident", "on"),
            ],
        )
        .await
        .assert_status(200);
    let incident = Incident::of_job_card(t.db(), card.id)
        .await
        .unwrap()
        .remove(0);
    assert!(incident.road_accident);
    t.app
        .get(&format!("/auto/job-cards/{}", card.id))
        .await
        .assert_ok()
        .assert_see("Vehicle incidents")
        .assert_see("Waiting for the branch manager");

    let basma = t
        .user("Basma", &[aut::JOB_CARDS_VIEW, aut::JOB_CARDS_LIABILITY])
        .await;
    t.app.acting_as(&tareq);
    t.app
        .htmx()
        .post(
            &format!("/auto/incidents/{}/decide", incident.id),
            &[("liability", "workshop"), ("reason", "ours")],
        )
        .await
        .assert_status(403);
    t.app.acting_as(&basma);
    let blank = t
        .app
        .htmx()
        .post(
            &format!("/auto/incidents/{}/decide", incident.id),
            &[("liability", "workshop"), ("reason", " ")],
        )
        .await;
    blank.assert_status(422);
    assert!(blank.text().contains("reason"));
    t.app
        .htmx()
        .post(
            &format!("/auto/incidents/{}/decide", incident.id),
            &[("liability", "workshop"), ("reason", "ours")],
        )
        .await
        .assert_status(200);
    t.app
        .get(&format!("/auto/job-cards/{}", card.id))
        .await
        .assert_ok()
        .assert_see("The workshop");
}

/// The accident damage card of the job card page: a flagged job card shows the
/// police report status and the form to ask for the flag to be cleared; the
/// workshop manager sees the request and decides it.
#[renox::test]
async fn aut_07_the_job_card_page_shows_the_police_report_and_the_flag_request() {
    let t = boot().await;
    t.documents.police_blocks();
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::Inspecting).await;
    auto::JobCard::query()
        .where_(auto::JobCard::ID.eq(card.id))
        .update(t.db(), &[("accident_damage", &true)])
        .await
        .unwrap();
    let sami = t.advisor_user().await;
    t.app.acting_as(&sami);
    t.app
        .get(&format!("/auto/job-cards/{}", card.id))
        .await
        .assert_ok()
        .assert_see("Accident damage")
        .assert_see("Police report required");
    t.app
        .htmx()
        .post(
            &format!("/auto/job-cards/{}/accident/clear", card.id),
            &[("reason", " ")],
        )
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(
            &format!("/auto/job-cards/{}/accident/clear", card.id),
            &[("reason", "Flag set by mistake")],
        )
        .await
        .assert_status(200);
    let wanda = t
        .user("Wanda", &[aut::JOB_CARDS_VIEW, aut::JOB_CARDS_ACCIDENT])
        .await;
    t.app.acting_as(&wanda);
    t.app
        .get(&format!("/auto/job-cards/{}", card.id))
        .await
        .assert_ok()
        .assert_see("Flag set by mistake");
    t.app
        .htmx()
        .post(
            &format!("/auto/job-cards/{}/accident/decide", card.id),
            &[("decision", "approve")],
        )
        .await
        .assert_status(200);
    assert!(!t.reload(card.id).await.accident_damage);
}
