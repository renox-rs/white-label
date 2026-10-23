//! BL-AUT-06 (#402, `AUT.ACL.22`): test drives are recorded on the job card
//! and an open one blocks `pass_qc`; damage is reported as a ticket of the
//! category `vehicle_incident`; only the branch manager (or the owner), never
//! an involved person, decides the liability; the workshop's liability opens
//! an internal job card that no customer invoice carries, flagged accident
//! damage for a road accident. The tickets are the stand-in's here and the
//! real wl-ticketing's in `host/tests/auto_incidents_real.rs`.

mod support;

use auto::incident::{Incident, NOT_WORKSHOP, WORKSHOP};
use auto::incidents::{self, NewIncident};
use auto::job_card::QC_KEYS;
use auto::job_cards::{self, NewLabour, Promise};
use auto::permissions as aut;
use auto::test_drive::TestDrive;
use auto::test_drives::{self, NewDrive};
use auto::{JobCard, JobCardStatus};
use renox::prelude::*;
use support::{Garage, at, boot, day};

fn words(error: &Error) -> String {
    error.to_string()
}

/// The branch manager Basma: decides liabilities.
async fn manager(t: &Garage) -> AuthUser {
    let user = t
        .user("Basma", &[aut::JOB_CARDS_VIEW, aut::JOB_CARDS_LIABILITY])
        .await;
    t.auth(&user).await
}

/// JC-60 in progress, assigned to Tareq.
async fn jc60(t: &Garage) -> JobCard {
    let vehicle = t.hilux().await;
    t.card_in(&vehicle, JobCardStatus::InProgress).await
}

/// Points 1 and 5: JC-60 has a test drive by the technician (out 10:00 at
/// 52,210 km, in 10:25 at 52,222 km), listed on the job card; a drive without
/// mileage in blocks `pass_qc` until its mileage is recorded, and the mileage
/// in is not below the mileage out.
#[renox::test]
async fn aut_06_a_test_drive_is_listed_and_an_open_one_blocks_the_qc() {
    let t = boot().await;
    let card = jc60(&t).await;
    let tareq = t.technician("Tareq").await;
    let drive = test_drives::record(
        t.state(),
        &tareq,
        card.id,
        NewDrive::out(52_210)
            .left_at(at(day(0), 10, 0))
            .back(at(day(0), 10, 25), 52_222),
    )
    .await
    .unwrap();
    let listed = TestDrive::of_job_card(t.db(), card.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, drive.id);
    assert_eq!(listed[0].driver_user_id, tareq.id);
    assert_eq!(listed[0].distance(), Some(12));
    assert!(!listed[0].is_open());

    // A second drive goes out and does not come back.
    let open = test_drives::record(t.state(), &tareq, card.id, NewDrive::out(52_222))
        .await
        .unwrap();
    assert!(open.is_open());
    let again = test_drives::record(t.state(), &tareq, card.id, NewDrive::out(52_222))
        .await
        .unwrap_err();
    assert!(words(&again).contains("still out"), "{again}");

    // The job card is at the QC with every check done.
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(t.db(), &[("status", &JobCardStatus::QualityCheck)])
        .await
        .unwrap();
    let qc = t.qc().await;
    let keys: Vec<String> = QC_KEYS.iter().map(|k| (*k).to_owned()).collect();
    job_cards::save_qc(t.state(), &qc, card.id, &keys)
        .await
        .unwrap();
    let refusal = job_cards::pass_qc(t.state(), &qc, card.id)
        .await
        .unwrap_err();
    assert!(words(&refusal).contains("odometer"), "{refusal}");
    assert_eq!(t.reload(card.id).await.status, JobCardStatus::QualityCheck);

    let low = test_drives::finish(t.state(), &tareq, open.id, 52_000, None)
        .await
        .unwrap_err();
    assert!(words(&low).contains("below"), "{low}");
    test_drives::finish(t.state(), &tareq, open.id, 52_230, None)
        .await
        .unwrap();
    let ready = job_cards::pass_qc(t.state(), &qc, card.id).await.unwrap();
    assert_eq!(ready.status, JobCardStatus::Ready);
}

/// Points 2, 3 and 4: the technician raises the incident ticket with the road
/// accident answer; the technician and the advisor cannot decide it; the branch
/// manager decides *workshop* and an internal job card for the vehicle is
/// opened, flagged accident damage, whose approval needs the police report.
#[renox::test]
async fn aut_06_the_branch_manager_decides_and_the_workshop_liability_opens_an_internal_job_card() {
    let t = boot().await;
    t.documents.police_blocks();
    let card = jc60(&t).await;
    let tareq = t.technician("Tareq").await;
    let sami = t.advisor().await;
    let incident = incidents::report(
        t.state(),
        &tareq,
        card.id,
        NewIncident::new("Scratch on the right door while parking").road_accident(),
    )
    .await
    .unwrap();
    assert!(incident.road_accident);
    let tasks = t.tickets.about(&card.subject());
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].ticket.category_key, "vehicle_incident");
    assert_eq!(incident.ticket_id, Some(tasks[0].id));

    for who in [&tareq, &sami] {
        let refusal = incidents::decide(t.state(), who, incident.id, WORKSHOP, "Our fault")
            .await
            .unwrap_err();
        assert!(matches!(refusal, Error::Forbidden), "{refusal}");
    }
    let basma = manager(&t).await;
    let blank = incidents::decide(t.state(), &basma, incident.id, WORKSHOP, " ")
        .await
        .unwrap_err();
    assert!(words(&blank).contains("reason"), "{blank}");

    let decided = incidents::decide(
        t.state(),
        &basma,
        incident.id,
        WORKSHOP,
        "The technician hit the post",
    )
    .await
    .unwrap();
    assert_eq!(decided.liability.as_deref(), Some(WORKSHOP));
    assert_eq!(decided.decided_by, Some(basma.id));
    let internal = t
        .reload(decided.internal_job_card_id.expect("an internal job card"))
        .await;
    assert!(internal.internal && internal.accident_damage);
    assert_eq!(internal.vehicle_id, card.vehicle_id);
    assert_eq!(internal.incident_id, Some(incident.id));
    assert_eq!(internal.status, JobCardStatus::CheckedIn);
    let resolved = t.tickets.about(&card.subject())[0].resolved.clone();
    assert!(resolved.unwrap().contains("the workshop"));
    let again = incidents::decide(t.state(), &basma, incident.id, NOT_WORKSHOP, "x")
        .await
        .unwrap_err();
    assert!(words(&again).contains("decided already"), "{again}");

    // The internal job card is a road accident repair: its approval needs the
    // police report.
    let tareq_user = t.technician_user("Tareq").await;
    job_cards::assign_technician(t.state(), &sami, internal.id, tareq_user.id)
        .await
        .unwrap();
    job_cards::start_inspection(t.state(), &tareq, internal.id)
        .await
        .unwrap();
    job_cards::add_labour(
        t.state(),
        &tareq,
        internal.id,
        NewLabour::hourly("Door respray", 2_000, 20_000),
    )
    .await
    .unwrap();
    job_cards::send_estimate(t.state(), &sami, internal.id, Promise::default())
        .await
        .unwrap();
    let refusal = job_cards::approve_recorded(
        t.state(),
        &sami,
        internal.id,
        "Workshop manager",
        "recorded",
        None,
    )
    .await
    .unwrap_err();
    assert!(
        words(&refusal).contains("Police report required"),
        "{refusal}"
    );
    t.documents.set(&internal.subject(), "police_report", None);
    let started = job_cards::approve_recorded(
        t.state(),
        &sami,
        internal.id,
        "Workshop manager",
        "recorded",
        None,
    )
    .await
    .unwrap();
    assert_eq!(started.status, JobCardStatus::InProgress);
}

/// Point 2, last part: nothing from the internal job card is invoiced.
#[renox::test]
async fn aut_06_nothing_of_the_internal_job_card_reaches_a_customer_invoice() {
    let t = boot().await;
    let card = jc60(&t).await;
    let tareq = t.technician("Tareq").await;
    let incident = incidents::report(t.state(), &tareq, card.id, NewIncident::new("Dent"))
        .await
        .unwrap();
    let basma = manager(&t).await;
    let decided = incidents::decide(t.state(), &basma, incident.id, WORKSHOP, "Our fault")
        .await
        .unwrap();
    let internal = t
        .reload(decided.internal_job_card_id.expect("internal"))
        .await;
    assert!(!internal.accident_damage, "not a road accident");
    t.approved_labour(&internal, "Dent repair", 1_000, 20_000)
        .await;
    JobCard::query()
        .where_(JobCard::ID.eq(internal.id))
        .update(
            t.db(),
            &[
                ("status", &JobCardStatus::Delivered),
                ("delivered_at", &Some(renox::db::now())),
            ],
        )
        .await
        .unwrap();
    let billed = auto::billing::bill(t.state(), internal.id).await.unwrap();
    assert!(billed.is_none());
    assert!(t.books.all().is_empty());
}

/// Point 3: B resolves with *not workshop* and a reason: no internal job card
/// is created, and the ticket carries the reason.
#[renox::test]
async fn aut_06_not_the_workshop_creates_no_internal_job_card() {
    let t = boot().await;
    let card = jc60(&t).await;
    let tareq = t.technician("Tareq").await;
    let incident = incidents::report(t.state(), &tareq, card.id, NewIncident::new("Scratch"))
        .await
        .unwrap();
    let basma = manager(&t).await;
    let decided = incidents::decide(
        t.state(),
        &basma,
        incident.id,
        NOT_WORKSHOP,
        "scratch visible on check-in photo 3",
    )
    .await
    .unwrap();
    assert_eq!(decided.liability.as_deref(), Some(NOT_WORKSHOP));
    assert_eq!(decided.internal_job_card_id, None);
    let cards = JobCard::of_vehicle(t.db(), card.vehicle_id).await.unwrap();
    assert!(cards.iter().all(|c| !c.internal));
    let resolved = t.tickets.about(&card.subject())[0]
        .resolved
        .clone()
        .unwrap();
    assert!(resolved.contains("check-in photo 3"), "{resolved}");
    assert_eq!(
        Incident::of_job_card(t.db(), card.id).await.unwrap().len(),
        1
    );
}

/// Point 6: branch manager B drove the test drive, so B cannot decide the
/// incident; the job card's advisor cannot either; branch manager C or the
/// owner can.
#[renox::test]
async fn aut_06_an_involved_person_cannot_decide_but_another_manager_or_the_owner_can() {
    let t = boot().await;
    let card = jc60(&t).await;
    let tareq = t.technician("Tareq").await;
    // B holds the advisor's and the liability permissions and drives.
    let b_user = t
        .user(
            "Bilal",
            &[
                aut::JOB_CARDS_VIEW,
                aut::JOB_CARDS_ADVISE,
                aut::JOB_CARDS_LIABILITY,
            ],
        )
        .await;
    let bilal = t.auth(&b_user).await;
    test_drives::record(
        t.state(),
        &bilal,
        card.id,
        NewDrive::out(52_100)
            .left_at(at(day(0), 9, 0))
            .back(at(day(0), 9, 20), 52_112),
    )
    .await
    .unwrap();
    let incident = incidents::report(t.state(), &tareq, card.id, NewIncident::new("Dent"))
        .await
        .unwrap();

    let refusal = incidents::decide(t.state(), &bilal, incident.id, WORKSHOP, "Our fault")
        .await
        .unwrap_err();
    assert!(words(&refusal).contains("involved"), "{refusal}");

    // The advisor of the job card, who also holds the permission, is involved.
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(t.db(), &[("advisor_user_id", &Some(bilal.id))])
        .await
        .unwrap();
    let owner_user = t
        .user("Omar", &[aut::JOB_CARDS_VIEW, aut::JOB_CARDS_LIABILITY])
        .await;
    let omar = t.auth(&owner_user).await;
    let decided = incidents::decide(t.state(), &omar, incident.id, NOT_WORKSHOP, "Customer's")
        .await
        .unwrap();
    assert_eq!(decided.decided_by, Some(omar.id));
}

/// The kernel rule keeps the ticket's own `resolve` from deciding the incident.
#[renox::test]
async fn aut_06_the_ticket_of_an_incident_is_not_resolved_from_its_own_page() {
    let _t = boot().await;
    let rule = wl_kernel::tickets::resolve_rule("vehicle_incident").expect("the rule");
    let state = _t.state();
    let verdict = rule
        .check(
            state,
            wl_kernel::tickets::TicketId(1),
            &wl_kernel::Subject::new("auto.job_card", 1),
            7,
        )
        .await
        .unwrap();
    let words = verdict.unwrap_err();
    assert!(words.contains("job card"), "{words}");
}
