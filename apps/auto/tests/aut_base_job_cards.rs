//! The base of the 08 Auto job card (base for #397 .. #403, `AUT.JOB.01`,
//! `AUT.JOB.02`, `AUT.ACL.08`, `AUT.ACL.09`): a vehicle is checked in with its
//! intake, the technician inspects, the estimate goes to the customer who
//! approves it on a link, the work is done per line, parts are issued from
//! the store, the QC passes and the customer collects the vehicle against an
//! invoice. The guard slots the negative flows fill pass for now.

mod support;

use auto::approval::EstimateApproval;
use auto::job_card_machine::{
    APPROVE_SYSTEM, CANCEL, DECLINE, DECLINE_ADDED, PARTS_ARRIVED, VOID, WAIT_PARTS,
};
use auto::job_cards::{
    self, Delivery, Intake, NewJobCard, NewLabour, NewPart, Promise, visible_to,
};
use auto::labour_menu::{self, NewOperation};
use auto::{
    ApprovalStatus, JobCard, JobCardStatus, LabourPricing, LineStatus, PartCondition, WorkStatus,
    parts,
};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::{boot, day};
use wl_kernel::Subject;

/// A job card checked in at the desk by Sami for the Hilux, assigned to Tareq
/// and the workshop `scheduling.location:1`.
async fn checked_in(t: &support::Garage) -> JobCard {
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let tareq = t.technician_user("Tareq").await;
    job_cards::check_in(
        t.state(),
        &sami,
        NewJobCard::new(vehicle.id, " Brake noise when stopping ")
            .odometer(52_400)
            .fuel("half")
            .condition("Scratch on the left door; no valuables left")
            .acknowledged_by("Mona Hassan")
            .inspection_fee(15_000)
            .technician(tareq.id)
            .workshop(Subject::new("scheduling.location", 1)),
    )
    .await
    .expect("the vehicle is checked in")
}

/// A vehicle is checked in with its complaint, odometer, fuel level and the
/// condition the customer acknowledged; the odometer becomes the vehicle's.
#[renox::test]
async fn aut_base_a_vehicle_is_checked_in_with_its_intake() {
    let t = boot().await;
    let card = checked_in(&t).await;

    assert_eq!(card.status, JobCardStatus::CheckedIn);
    assert_eq!(card.number(), format!("JOB-{:05}", card.id));
    assert_eq!(card.complaint, "Brake noise when stopping");
    assert_eq!(
        (card.odometer_in, card.fuel_level.as_deref()),
        (52_400, Some("half"))
    );
    assert_eq!(
        card.intake_condition.as_deref(),
        Some("Scratch on the left door; no valuables left")
    );
    assert_eq!(card.intake_acknowledged_by.as_deref(), Some("Mona Hassan"));
    assert!(card.intake_done());
    assert_eq!(
        card.customer_party_id, 7,
        "billed to the owner of the vehicle"
    );
    assert_eq!(card.inspection_fee, 15_000);
    assert_eq!(
        card.workshop().map(|w| w.to_string()),
        Some("scheduling.location:1".to_owned())
    );
    let vehicle = auto::Vehicle::find_or_404(t.db(), card.vehicle_id)
        .await
        .unwrap();
    assert_eq!(vehicle.odometer_km, 52_400);
    let history = renox::state_machine::history(t.db(), JobCard::TABLE, card.id)
        .await
        .unwrap();
    assert_eq!(history.len(), 1, "the check-in is the first change");
}

/// The intake is required (the condition of the vehicle is documented before
/// any work), the odometer does not read below the vehicle's, the fuel level is
/// one of the list, and only an advisor checks in.
#[renox::test]
async fn aut_base_the_intake_is_required_and_checked() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;

    let no_condition = job_cards::check_in(
        t.state(),
        &sami,
        NewJobCard::new(vehicle.id, "Noise").odometer(52_100),
    )
    .await
    .unwrap_err();
    assert!(
        no_condition.to_string().contains("condition"),
        "{no_condition}"
    );

    let lower = job_cards::check_in(
        t.state(),
        &sami,
        NewJobCard::new(vehicle.id, "Noise")
            .odometer(51_000)
            .condition("Clean"),
    )
    .await
    .unwrap_err();
    assert!(lower.to_string().contains("last reading"), "{lower}");

    let fuel = job_cards::check_in(
        t.state(),
        &sami,
        NewJobCard::new(vehicle.id, "Noise")
            .odometer(52_100)
            .condition("Clean")
            .fuel("brimming"),
    )
    .await
    .unwrap_err();
    assert!(fuel.to_string().contains("fuel level"), "{fuel}");

    let blank = job_cards::check_in(
        t.state(),
        &sami,
        NewJobCard::new(vehicle.id, "  ")
            .odometer(52_100)
            .condition("Clean"),
    )
    .await
    .unwrap_err();
    assert!(blank.to_string().contains("complaint"), "{blank}");

    let tareq = t.technician("Tareq").await;
    let refused = job_cards::check_in(
        t.state(),
        &tareq,
        NewJobCard::new(vehicle.id, "Noise")
            .odometer(52_100)
            .condition("Clean"),
    )
    .await;
    assert!(matches!(refused, Err(Error::Forbidden)));
}

/// The whole job: inspection, an estimate of labour (hourly and from the menu)
/// and parts (new and used), the customer's approval on the link with the
/// consent recorded, the work done per line, a part issued from the store,
/// the QC checklist, the delivery and the invoice with each part's condition.
#[renox::test]
async fn aut_base_check_in_estimate_approve_work_deliver_and_invoice() {
    let t = boot().await;
    t.stock.with_store();
    t.stock.stock(11, "BRK-PAD", 5_000);
    let card = checked_in(&t).await;
    let sami = t.advisor().await;
    let tareq = t.technician("Tareq").await;

    // The technician inspects and records what he found.
    let card = job_cards::start_inspection(t.state(), &tareq, card.id)
        .await
        .unwrap();
    assert_eq!(card.status, JobCardStatus::Inspecting);
    let card = job_cards::record_findings(t.state(), &tareq, card.id, "Front pads worn to 2 mm")
        .await
        .unwrap();
    assert_eq!(card.findings.as_deref(), Some("Front pads worn to 2 mm"));

    // The estimate: an hourly line, a menu operation, a new and a used part.
    let hourly = job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Brake inspection and pad replacement", 2_000, 15_000),
    )
    .await
    .unwrap();
    assert_eq!(hourly.price(), 30_000);
    let menu_keeper = t.qc().await;
    labour_menu::create(
        t.state(),
        &menu_keeper,
        NewOperation::flat("oil-chg", "Oil change", 1_000, 12_000),
    )
    .await
    .unwrap();
    let oil = job_cards::add_operation(t.state(), &sami, card.id, "OIL-CHG", 0)
        .await
        .unwrap();
    assert_eq!(
        (oil.pricing, oil.price()),
        (LabourPricing::FlatRate, 12_000)
    );
    let pad = job_cards::add_part(
        t.state(),
        &tareq,
        card.id,
        NewPart::new("Brake pad set", 1_000, 30_000).item("brk-pad"),
    )
    .await
    .unwrap();
    assert_eq!(pad.item_code.as_deref(), Some("BRK-PAD"));
    let mirror = job_cards::add_part(
        t.state(),
        &sami,
        card.id,
        NewPart::new("Door mirror", 1_000, 9_000).condition(PartCondition::Used),
    )
    .await
    .unwrap();

    // The advisor sends it with the day promised and a warranty of 30 days.
    let sent = job_cards::send_estimate(
        t.state(),
        &sami,
        card.id,
        Promise {
            promised_for: Some(day(2)),
            warranty_days: Some(30),
        },
    )
    .await
    .unwrap();
    assert_eq!(sent.status, JobCardStatus::AwaitingApproval);
    assert_eq!(sent.estimate_revision, 1);
    assert!(sent.awaiting_since.is_some());
    let round = job_cards::pending_round(t.db(), card.id)
        .await
        .unwrap()
        .expect("a round waits");
    assert_eq!((round.revision, round.net_total), (1, 81_000));
    assert_eq!(
        (round.warranty_days, round.promised_for),
        (30, Some(day(2)))
    );
    assert_eq!(round.path(), format!("/auto/approve/{}", round.token));
    assert_eq!(t.labour_now(hourly.id).await.approval, LineStatus::Sent);
    assert_eq!(t.part_now(mirror.id).await.approval, LineStatus::Sent);

    // The customer approves on the link: the consent is recorded.
    let under_way = job_cards::approve_by_link(
        t.state(),
        &round.token,
        "Mona Hassan",
        Some("Mona Hassan"),
        true,
    )
    .await
    .unwrap();
    assert_eq!(under_way.status, JobCardStatus::InProgress);
    assert!(under_way.first_approved_at.is_some());
    assert!(under_way.awaiting_since.is_none());
    let answered = EstimateApproval::find_or_404(t.db(), round.id)
        .await
        .unwrap();
    assert_eq!(answered.status, ApprovalStatus::Approved);
    assert_eq!(answered.decided_by_name.as_deref(), Some("Mona Hassan"));
    assert_eq!(answered.method.as_deref(), Some("link"));
    assert_eq!(answered.signature.as_deref(), Some("Mona Hassan"));
    assert!(answered.decided_at.is_some());
    assert_eq!(t.labour_now(oil.id).await.approval, LineStatus::Approved);

    // The work is recorded per line; the pad comes from the store, the
    // mirror is a charge only.
    job_cards::record_work(t.state(), &tareq, hourly.id, WorkStatus::Done, None)
        .await
        .unwrap();
    job_cards::record_work(t.state(), &tareq, oil.id, WorkStatus::Done, None)
        .await
        .unwrap();
    let issued = parts::issue(t.state(), &tareq, pad.id, None).await.unwrap();
    assert_eq!(
        (issued.issued_milli, issued.posted_milli, issued.short_milli),
        (1_000, 1_000, 0)
    );
    assert_eq!(t.stock.balance("BRK-PAD"), 4_000);
    let asked = t.stock.consumed();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].source, card.subject());
    assert_eq!(asked[0].cost_object, Some(card.subject()));
    let charge_only = parts::issue(t.state(), &tareq, mirror.id, None)
        .await
        .unwrap();
    assert_eq!(
        (charge_only.issued_milli, charge_only.posted_milli),
        (1_000, 0)
    );
    assert_eq!(
        t.stock.consumed().len(),
        1,
        "a part without an item posts nothing"
    );
    parts::fit(t.state(), &tareq, pad.id, None).await.unwrap();
    parts::fit(t.state(), &tareq, mirror.id, None)
        .await
        .unwrap();

    // The work is finished: the QC checklist starts and must be complete.
    let checking = job_cards::finish_work(t.state(), &tareq, card.id)
        .await
        .unwrap();
    assert_eq!(checking.status, JobCardStatus::QualityCheck);
    assert_eq!(checking.qc_checks.0.len(), 5);
    let waleed = t.qc().await;
    let early = job_cards::pass_qc(t.state(), &waleed, card.id)
        .await
        .unwrap_err();
    assert!(early.to_string().contains("not complete"), "{early}");
    let all: Vec<String> = auto::job_card::QC_KEYS
        .iter()
        .map(|k| (*k).to_owned())
        .collect();
    job_cards::save_qc(t.state(), &waleed, card.id, &all)
        .await
        .unwrap();
    let ready = job_cards::pass_qc(t.state(), &waleed, card.id)
        .await
        .unwrap();
    assert_eq!(ready.status, JobCardStatus::Ready);
    assert!(ready.ready_at.is_some());
    assert_eq!(ready.qc_by, Some(waleed.id));

    // The customer collects the vehicle: the invoice and the next service.
    let delivered = job_cards::deliver(
        t.state(),
        &sami,
        card.id,
        Delivery {
            delivered_to: "Mona Hassan".to_owned(),
            odometer_out: Some(52_410),
            next_service_date: Some(day(180)),
            next_service_odometer: Some(62_400),
        },
    )
    .await
    .unwrap();
    assert_eq!(delivered.status, JobCardStatus::Delivered);
    assert_eq!(delivered.delivered_to.as_deref(), Some("Mona Hassan"));
    let invoices = t.books.about(&card.subject());
    assert_eq!(invoices.len(), 1);
    let invoice = &invoices[0];
    assert!(invoice.issued);
    assert_eq!(delivered.invoice_id, Some(invoice.id));
    assert_eq!(invoice.party().0, 7);
    assert_eq!((invoice.net(), invoice.gross()), (81_000, 85_050));
    let described: Vec<&str> = invoice
        .draft
        .lines
        .iter()
        .map(|l| l.description.as_str())
        .collect();
    assert_eq!(
        described,
        [
            "Brake inspection and pad replacement (2 h)",
            "Oil change",
            "Brake pad set (new)",
            "Door mirror (used)"
        ]
    );
    let vehicle = auto::Vehicle::find_or_404(t.db(), card.vehicle_id)
        .await
        .unwrap();
    assert_eq!(vehicle.odometer_km, 52_410);
    assert_eq!(vehicle.next_service_date, Some(day(180)));
    assert_eq!(vehicle.next_service_odometer, Some(62_400));
}

/// The customer's link: a name and the ticked consent are needed, an
/// answered link answers no more, a token nobody knows is a 404, and a decline
/// before anything was approved ends the job card.
#[renox::test]
async fn aut_base_the_customers_link_is_answered_once_and_with_consent() {
    let t = boot().await;
    let card = checked_in(&t).await;
    let sami = t.advisor().await;
    let tareq = t.technician("Tareq").await;
    job_cards::start_inspection(t.state(), &tareq, card.id)
        .await
        .unwrap();
    job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Brakes", 1_000, 20_000),
    )
    .await
    .unwrap();
    job_cards::send_estimate(t.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();
    let round = job_cards::pending_round(t.db(), card.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(round.warranty_days, 15, "the warranty is 15 days at least");

    let no_consent = job_cards::approve_by_link(t.state(), &round.token, "Mona", None, false)
        .await
        .unwrap_err();
    assert!(
        no_consent.to_string().contains("Tick the box"),
        "{no_consent}"
    );
    let no_name = job_cards::approve_by_link(t.state(), &round.token, " ", None, true)
        .await
        .unwrap_err();
    assert!(no_name.to_string().contains("name"), "{no_name}");
    let unknown = job_cards::approve_by_link(t.state(), "nobody", "Mona", None, true).await;
    assert!(matches!(unknown, Err(Error::NotFound)));

    let declined = job_cards::decline_by_link(t.state(), &round.token, "Mona", "Too dear")
        .await
        .unwrap();
    assert_eq!(declined.status, JobCardStatus::Declined);
    assert_eq!(declined.closed_reason.as_deref(), Some("Too dear"));
    let again = job_cards::approve_by_link(t.state(), &round.token, "Mona", None, true)
        .await
        .unwrap_err();
    assert!(again.to_string().contains("answered already"), "{again}");
    let round = EstimateApproval::find_or_404(t.db(), round.id)
        .await
        .unwrap();
    assert_eq!(round.status, ApprovalStatus::Declined);
}

/// Added work found during the job goes to the customer in a new round; the
/// customer declines it and the job goes on without it, and the job card is
/// not delivered while a line waits for an answer.
#[renox::test]
async fn aut_base_added_work_is_a_new_round_and_declining_it_keeps_the_job_going() {
    let t = boot().await;
    let card = checked_in(&t).await;
    let sami = t.advisor().await;
    let tareq = t.technician("Tareq").await;
    job_cards::start_inspection(t.state(), &tareq, card.id)
        .await
        .unwrap();
    job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Brakes", 1_000, 20_000),
    )
    .await
    .unwrap();
    job_cards::send_estimate(t.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();
    let first = job_cards::pending_round(t.db(), card.id)
        .await
        .unwrap()
        .unwrap();
    job_cards::approve_by_link(t.state(), &first.token, "Mona", None, true)
        .await
        .unwrap();

    // Added work: a disc is found worn.
    let disc = job_cards::add_part(
        t.state(),
        &tareq,
        card.id,
        NewPart::new("Brake disc", 2_000, 25_000),
    )
    .await
    .unwrap();
    let asked = job_cards::send_estimate(t.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();
    assert_eq!(asked.status, JobCardStatus::AwaitingApproval);
    assert_eq!(asked.estimate_revision, 2);
    let second = job_cards::pending_round(t.db(), card.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((second.revision, second.net_total), (2, 50_000));

    let kept_going = job_cards::decline_by_link(t.state(), &second.token, "Mona", "")
        .await
        .unwrap();
    assert_eq!(
        kept_going.status,
        JobCardStatus::InProgress,
        "the job goes on"
    );
    assert_eq!(t.part_now(disc.id).await.approval, LineStatus::Declined);
    assert_eq!(
        EstimateApproval::find_or_404(t.db(), second.id)
            .await
            .unwrap()
            .status,
        ApprovalStatus::Declined
    );
    assert!(kept_going.work_approved(), "the first approval stands");
}

/// An estimate goes out priced: with lines, every one with a price, a
/// warranty of at least 15 days, a day that has not passed, and after the
/// intake.
#[renox::test]
async fn aut_base_an_estimate_needs_priced_lines_and_a_lawful_warranty() {
    let t = boot().await;
    let card = checked_in(&t).await;
    let sami = t.advisor().await;
    let tareq = t.technician("Tareq").await;
    job_cards::start_inspection(t.state(), &tareq, card.id)
        .await
        .unwrap();

    let empty = job_cards::send_estimate(t.state(), &sami, card.id, Promise::default())
        .await
        .unwrap_err();
    assert!(empty.to_string().contains("price"), "{empty}");

    job_cards::add_part(
        t.state(),
        &tareq,
        card.id,
        NewPart::new("Free sample", 1_000, 0),
    )
    .await
    .unwrap();
    let unpriced = job_cards::send_estimate(t.state(), &sami, card.id, Promise::default())
        .await
        .unwrap_err();
    assert!(unpriced.to_string().contains("price"), "{unpriced}");

    job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Brakes", 1_000, 20_000),
    )
    .await
    .unwrap();
    let short = job_cards::send_estimate(
        t.state(),
        &sami,
        card.id,
        Promise {
            promised_for: None,
            warranty_days: Some(7),
        },
    )
    .await
    .unwrap_err();
    assert!(short.to_string().contains("15 days"), "{short}");
    let past = job_cards::send_estimate(
        t.state(),
        &sami,
        card.id,
        Promise {
            promised_for: Some(day(-1)),
            warranty_days: None,
        },
    )
    .await
    .unwrap_err();
    assert!(past.to_string().contains("passed"), "{past}");
    let not_advisor =
        job_cards::send_estimate(t.state(), &tareq, card.id, Promise::default()).await;
    assert!(matches!(not_advisor, Err(Error::Forbidden)));

    // A removed draft does not hold the estimate back.
    let drafts = auto::estimate::Estimate::load(t.db(), card.id)
        .await
        .unwrap();
    job_cards::remove_part(t.state(), &tareq, drafts.parts[0].id)
        .await
        .unwrap();
    job_cards::send_estimate(t.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();
}

/// A technician sees and works only the job cards assigned to them, and no
/// contact details (`AUT.ACL.08`).
#[renox::test]
async fn aut_base_a_technician_sees_only_the_job_cards_assigned_to_them() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let mine = t.card_in(&vehicle, JobCardStatus::CheckedIn).await;
    let sami = t.advisor().await;
    let other = t.technician_user("Omar").await;
    let theirs = job_cards::check_in(
        t.state(),
        &sami,
        NewJobCard::new(vehicle.id, "Oil leak")
            .odometer(52_000)
            .condition("Clean")
            .technician(other.id),
    )
    .await
    .unwrap();
    let tareq = t.technician("Tareq").await;

    let seen: Vec<i64> = visible_to(t.state(), &tareq)
        .await
        .unwrap()
        .iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(seen, [mine.id]);
    let all: Vec<i64> = visible_to(t.state(), &sami)
        .await
        .unwrap()
        .iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(all.len(), 2);

    let refused = job_cards::start_inspection(t.state(), &tareq, theirs.id).await;
    assert!(matches!(refused, Err(Error::Forbidden)));
    let refused = job_cards::add_labour(
        t.state(),
        &tareq,
        theirs.id,
        NewLabour::hourly("Seal", 1_000, 10_000),
    )
    .await;
    assert!(matches!(refused, Err(Error::Forbidden)));

    let tareq_user = t.technician_user("Tareq").await;
    t.app.acting_as(&tareq_user);
    t.app
        .get(&format!("/auto/job-cards/{}", mine.id))
        .await
        .assert_ok()
        .assert_see("Dubai A 12345")
        .assert_see("Brake noise when stopping");
    let page = t.app.get(&format!("/auto/job-cards/{}", mine.id)).await;
    assert!(
        !page.text().contains("Mona Hassan"),
        "no contact details for a technician"
    );
    t.app
        .get(&format!("/auto/job-cards/{}", theirs.id))
        .await
        .assert_status(403);
    t.app
        .get("/auto/technician")
        .await
        .assert_ok()
        .assert_see("Brake noise");
}

/// The QC: the checklist is ticked, the vehicle passes only when it is
/// complete, and a failed QC records the findings, resets the checklist and
/// sends the job back to the technician.
#[renox::test]
async fn aut_base_the_qc_checklist_must_be_complete_and_a_failed_qc_resets_it() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    t.approved_labour(&card, "Brakes", 1_000, 20_000).await;
    let tareq = t.technician("Tareq").await;
    let waleed = t.qc().await;

    let open = job_cards::finish_work(t.state(), &tareq, card.id)
        .await
        .unwrap_err();
    assert!(open.to_string().contains("not done yet"), "{open}");
    let line = auto::estimate::Estimate::load(t.db(), card.id)
        .await
        .unwrap()
        .labour[0]
        .id;
    job_cards::record_work(t.state(), &tareq, line, WorkStatus::Done, None)
        .await
        .unwrap();
    job_cards::finish_work(t.state(), &tareq, card.id)
        .await
        .unwrap();

    job_cards::save_qc(
        t.state(),
        &waleed,
        card.id,
        &["fluids".to_owned(), "lights".to_owned()],
    )
    .await
    .unwrap();
    assert!(
        job_cards::pass_qc(t.state(), &waleed, card.id)
            .await
            .is_err()
    );
    let wrong = job_cards::save_qc(t.state(), &waleed, card.id, &["polish".to_owned()]).await;
    assert!(wrong.is_err());

    let nothing = job_cards::fail_qc(t.state(), &waleed, card.id, " ").await;
    assert!(nothing.is_err(), "the findings are the reason");
    let back = job_cards::fail_qc(t.state(), &waleed, card.id, "Pedal feels soft")
        .await
        .unwrap();
    assert_eq!(back.status, JobCardStatus::InProgress);
    assert_eq!(back.qc_findings.as_deref(), Some("Pedal feels soft"));
    assert!(
        back.qc_checks.0.iter().all(|check| !check.done),
        "the checklist starts again"
    );
    let not_qc = job_cards::pass_qc(t.state(), &tareq, card.id).await;
    assert!(matches!(not_qc, Err(Error::Forbidden)));
}

/// A vehicle is not delivered while a line waits for the customer's answer
/// (`AUT.ACL.09`: all added work approved), and a job card that is not ready
/// is not delivered at all.
#[renox::test]
async fn aut_base_a_vehicle_is_not_delivered_with_added_work_unanswered() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let card = t.card_in(&vehicle, JobCardStatus::Ready).await;
    let mut checks = auto::job_card::QcCheck::standard();
    for check in &mut checks {
        check.done = true;
    }
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(t.db(), &[("qc_checks", &renox::db::Json(checks))])
        .await
        .unwrap();
    let mut waiting = auto::LabourLine {
        job_card_id: card.id,
        description: "Added: seal".to_owned(),
        hours_milli: 1_000,
        rate: 10_000,
        approval: LineStatus::Sent,
        ..auto::LabourLine::default()
    };
    waiting.save(t.db()).await.unwrap();

    let delivery = || Delivery {
        delivered_to: "Mona Hassan".to_owned(),
        ..Delivery::default()
    };
    let refused = job_cards::deliver(t.state(), &sami, card.id, delivery())
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("waiting for the customer"),
        "{refused}"
    );
    let nobody = job_cards::deliver(t.state(), &sami, card.id, Delivery::default())
        .await
        .unwrap_err();
    assert!(nobody.to_string().contains("who collects"), "{nobody}");
    assert_eq!(t.reload(card.id).await.status, JobCardStatus::Ready);
    assert!(t.books.all().is_empty());
}

/// The guard slots the negative flows fill pass for now: cancel, void, decline,
/// wait for parts and parts arrived run with their reason where one is
/// needed, and the transitions that need one refuse without.
#[renox::test]
async fn aut_base_the_transitions_the_issues_extend_are_declared_and_their_slots_pass() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;

    // void: from checked_in, with a reason.
    let duplicate = t.card_in(&vehicle, JobCardStatus::CheckedIn).await;
    assert!(
        job_cards::void(t.state(), &sami, duplicate.id, " ")
            .await
            .is_err()
    );
    let voided = job_cards::void(t.state(), &sami, duplicate.id, "duplicate of JOB-00001")
        .await
        .unwrap();
    assert_eq!(voided.status, JobCardStatus::Voided);
    assert_eq!(
        voided.closed_reason.as_deref(),
        Some("duplicate of JOB-00001")
    );
    assert!(voided.closed_at.is_some());

    // decline: from checked_in; the customer left.
    let left = t.card_in(&vehicle, JobCardStatus::CheckedIn).await;
    let declined = job_cards::decline(t.state(), &sami, left.id, "customer left")
        .await
        .unwrap();
    assert_eq!(declined.status, JobCardStatus::Declined);

    // cancel: after work started, not from checked_in.
    let running = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    let cancelled = job_cards::cancel(t.state(), &sami, running.id, "customer stopped")
        .await
        .unwrap();
    assert_eq!(cancelled.status, JobCardStatus::Cancelled);
    assert_eq!(cancelled.closed_reason.as_deref(), Some("customer stopped"));
    let fresh = t.card_in(&vehicle, JobCardStatus::CheckedIn).await;
    assert!(
        job_cards::cancel(t.state(), &sami, fresh.id, "customer stopped")
            .await
            .is_err()
    );
    let closed = job_cards::void(t.state(), &sami, cancelled.id, "again").await;
    assert!(closed.is_err(), "a closed job card takes no transition");

    // wait_parts and parts_arrived.
    let waiting = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    let tareq = t.technician("Tareq").await;
    let parked = job_cards::wait_parts(t.state(), &tareq, waiting.id)
        .await
        .unwrap();
    assert_eq!(parked.status, JobCardStatus::AwaitingParts);
    assert!(parked.parts_wait_started_at.is_some());
    let resumed = support::run_as_system(&t, &parked, PARTS_ARRIVED, Default::default())
        .await
        .unwrap();
    assert_eq!(resumed.status, JobCardStatus::InProgress);

    // The names of the transitions are the spec's.
    let machine = JobCard::machine();
    for name in [
        CANCEL,
        VOID,
        DECLINE,
        DECLINE_ADDED,
        WAIT_PARTS,
        PARTS_ARRIVED,
        APPROVE_SYSTEM,
    ] {
        assert!(machine.find(name).is_ok(), "{name}");
    }
}

/// Parts: what the store cannot give is short and stays on the line, a part
/// without a store is a charge only, and what was issued and not fitted goes
/// back to stock with a return against the job card.
#[renox::test]
async fn aut_base_parts_are_issued_from_the_store_and_what_is_not_fitted_goes_back() {
    let t = boot().await;
    t.stock.with_store();
    t.stock.stock(11, "BRK-PAD", 2_000);
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    let tareq = t.technician("Tareq").await;
    let pads = t
        .approved_part(&card, "Brake pad set", Some("BRK-PAD"), 3_000, 30_000)
        .await;

    let over = parts::issue(t.state(), &tareq, pads.id, Some(5_000))
        .await
        .unwrap_err();
    assert!(over.to_string().contains("still to issue"), "{over}");
    let issued = parts::issue(t.state(), &tareq, pads.id, None)
        .await
        .unwrap();
    assert_eq!(
        (issued.issued_milli, issued.posted_milli, issued.short_milli),
        (3_000, 2_000, 1_000)
    );
    assert_eq!(issued.store_location_id, Some(1));
    assert_eq!(t.stock.balance("BRK-PAD"), 0);
    let fitted = parts::fit(t.state(), &tareq, pads.id, Some(1_000))
        .await
        .unwrap();
    assert_eq!(fitted.unfitted_milli(), 2_000);
    let too_many = parts::fit(t.state(), &tareq, pads.id, Some(3_000)).await;
    assert!(too_many.is_err());

    let back = parts::return_unfitted(t.state(), card.id).await.unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].returned_milli, 2_000);
    assert_eq!(back[0].unfitted_milli(), 0);
    let returned = t.stock.returned();
    assert_eq!(returned.len(), 1);
    assert_eq!(returned[0].quantity_milli, 2_000);
    assert_eq!(returned[0].source, card.subject());
    assert_eq!(returned[0].cost_object, Some(card.subject()));
    assert_eq!(returned[0].reason.as_deref(), Some("not fitted"));
    assert_eq!(t.stock.balance("BRK-PAD"), 2_000);
    assert!(
        parts::return_unfitted(t.state(), card.id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// A job card that waits for parts goes on, by the system, when the last
/// approved part is issued.
#[renox::test]
async fn aut_base_the_job_goes_on_when_the_last_part_is_issued() {
    let t = boot().await;
    t.stock.with_store();
    t.stock.stock(11, "BRK-PAD", 5_000);
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    let pads = t
        .approved_part(&card, "Brake pad set", Some("BRK-PAD"), 1_000, 30_000)
        .await;
    let tareq = t.technician("Tareq").await;
    let parked = job_cards::wait_parts(t.state(), &tareq, card.id)
        .await
        .unwrap();
    assert_eq!(parked.status, JobCardStatus::AwaitingParts);

    parts::issue(t.state(), &tareq, pads.id, None)
        .await
        .unwrap();
    assert_eq!(t.reload(card.id).await.status, JobCardStatus::InProgress);
    let history = renox::state_machine::history(t.db(), JobCard::TABLE, card.id)
        .await
        .unwrap();
    let last = history.last().unwrap();
    assert_eq!(last.transition, PARTS_ARRIVED);
    assert_eq!(last.actor_id, None, "the system");
}

/// The work done is recorded per line: a part done needs its hours, a line
/// the customer has not approved cannot be worked.
#[renox::test]
async fn aut_base_the_work_is_recorded_per_line() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    let line = t.approved_labour(&card, "Service", 3_000, 15_000).await;
    let tareq = t.technician("Tareq").await;

    let no_hours =
        job_cards::record_work(t.state(), &tareq, line.id, WorkStatus::PartDone, None).await;
    assert!(no_hours.is_err());
    let part = job_cards::record_work(
        t.state(),
        &tareq,
        line.id,
        WorkStatus::PartDone,
        Some(2_000),
    )
    .await
    .unwrap();
    assert_eq!(
        (part.work, part.hours_done_milli),
        (WorkStatus::PartDone, 2_000)
    );
    assert_eq!(part.done_value(), 30_000, "2 of 3 hours at AED 150");
    let done = job_cards::record_work(t.state(), &tareq, line.id, WorkStatus::Done, None)
        .await
        .unwrap();
    assert_eq!(done.work, WorkStatus::Done);
    assert_eq!(done.hours_done_milli, 3_000);
    assert!(done.done_at.is_some());

    let draft = job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Extra", 1_000, 10_000),
    )
    .await
    .unwrap();
    let not_approved =
        job_cards::record_work(t.state(), &tareq, draft.id, WorkStatus::Done, None).await;
    assert!(not_approved.is_err());
}

/// The invoice is raised once: billing a delivered job card again answers the
/// same invoice, and a job card that was not delivered is not billed.
#[renox::test]
async fn aut_base_a_delivered_job_card_is_invoiced_once() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::Delivered).await;
    t.approved_labour(&card, "Brakes", 2_000, 15_000).await;
    let mut declined = t
        .approved_labour(&card, "Declined work", 1_000, 99_000)
        .await;
    declined.approval = LineStatus::Declined;
    declined.save(t.db()).await.unwrap();

    let first = auto::billing::bill(t.state(), card.id)
        .await
        .unwrap()
        .expect("an invoice");
    let second = auto::billing::bill(t.state(), card.id)
        .await
        .unwrap()
        .expect("the same invoice");
    assert_eq!(first, second);
    let invoices = t.books.about(&card.subject());
    assert_eq!(invoices.len(), 1);
    assert_eq!(
        invoices[0].net(),
        30_000,
        "only what the customer approved is billed"
    );
    assert_eq!(t.reload(card.id).await.invoice_id, Some(first.0));

    let open = t.card_in(&vehicle, JobCardStatus::Ready).await;
    let refused = auto::billing::bill(t.state(), open.id).await.unwrap_err();
    assert!(refused.to_string().contains("delivered"), "{refused}");
}

/// The intake of a job card that came without one is recorded by the advisor,
/// and the inspection waits for it.
#[renox::test]
async fn aut_base_the_inspection_waits_for_the_intake_and_the_technician() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let tareq_user = t.technician_user("Tareq").await;
    let tareq = t.technician("Tareq").await;
    let mut bare = JobCard {
        vehicle_id: vehicle.id,
        customer_party_id: 7,
        complaint: "Booked service".to_owned(),
        ..JobCard::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    bare.insert_in_state(&mut tx, JobCardStatus::CheckedIn, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // A user who is an advisor and works the job cards too.
    let mixed_user = t
        .user(
            "Mixed",
            &[
                auto::permissions::JOB_CARDS_VIEW,
                auto::permissions::JOB_CARDS_ADVISE,
                auto::permissions::JOB_CARDS_WORK,
            ],
        )
        .await;
    let mixed = t.auth(&mixed_user).await;
    let no_technician = job_cards::start_inspection(t.state(), &mixed, bare.id)
        .await
        .unwrap_err();
    assert!(
        no_technician.to_string().contains("technician"),
        "{no_technician}"
    );
    let not_a_technician = job_cards::assign_technician(t.state(), &sami, bare.id, sami.id).await;
    assert!(not_a_technician.is_err());
    job_cards::assign_technician(t.state(), &sami, bare.id, tareq_user.id)
        .await
        .unwrap();
    let no_intake = job_cards::start_inspection(t.state(), &tareq, bare.id)
        .await
        .unwrap_err();
    assert!(no_intake.to_string().contains("intake"), "{no_intake}");

    let recorded = job_cards::record_intake(
        t.state(),
        &sami,
        bare.id,
        Intake {
            complaint: "Service and brake noise".to_owned(),
            odometer_in: 52_050,
            fuel_level: Some("full".to_owned()),
            condition: "Dent on the rear bumper".to_owned(),
            acknowledged_by: Some("Mona Hassan".to_owned()),
            signature: None,
            accident_damage: false,
            police_report_no: None,
            inspection_fee: 0,
            storage_fee_daily: 0,
        },
    )
    .await
    .unwrap();
    assert!(recorded.intake_done());
    assert_eq!(recorded.advisor_user_id, Some(sami.id));
    let inspecting = job_cards::start_inspection(t.state(), &tareq, bare.id)
        .await
        .unwrap();
    assert_eq!(inspecting.status, JobCardStatus::Inspecting);
    let too_late = job_cards::record_intake(
        t.state(),
        &sami,
        bare.id,
        Intake {
            complaint: "x".to_owned(),
            odometer_in: 52_050,
            fuel_level: None,
            condition: "y".to_owned(),
            acknowledged_by: None,
            signature: None,
            accident_damage: false,
            police_report_no: None,
            inspection_fee: 0,
            storage_fee_daily: 0,
        },
    )
    .await;
    assert!(
        too_late.is_ok(),
        "the intake can still be corrected while inspecting"
    );
}
