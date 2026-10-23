//! The internal job card of a vehicle incident (#402 points 2 and 4) and the
//! private repair after a rejected claim (#403 point 4) against the REAL
//! wl-documents, wl-invoicing, wl-ticketing and wl-scheduling of the host.

mod support;

use auto::claims::{self, NewClaim};
use auto::incidents::{self, NewIncident};
use auto::job_cards::{self, Delivery, NewLabour, Promise};
use auto::vehicles::{self, NewVehicle};
use auto::{ClaimStatus, Emirate, JobCard, JobCardStatus, WorkStatus};
use host::roles::{
    BRANCH_MANAGER, INSURANCE_COORDINATOR, SERVICE_ADVISOR, TECHNICIAN, WORKSHOP_MANAGER,
};
use renox::prelude::*;
use renox::upload::Upload;
use support::autofx::{World, boot, inspected_card};
use support::{company, day, tenant};
use wl_documents::{Documents, NewDocument};
use wl_kernel::Subject;

/// A police report uploaded by `by` on `subject`.
async fn upload_report(w: &World, by: &AuthUser, subject: Subject) -> wl_documents::Document {
    Documents::attach(
        w.state(),
        Some(by),
        NewDocument::new(
            subject,
            "police_report",
            Upload::new(
                "police-report.pdf",
                "application/pdf",
                b"%PDF-1.7 police report".to_vec(),
            ),
        )
        .number("DXB-PR-70"),
    )
    .await
    .unwrap()
    .expect("the report is attached")
}

/// The road accident of a test drive is decided against the workshop: the
/// internal job card is flagged as an accident, cannot be approved without the
/// accepted police report, and once repaired and delivered nothing is invoiced
/// to anyone on the real books.
#[renox::test]
async fn aut_06_real_the_internal_accident_card_needs_the_report_and_is_never_invoiced() {
    let w = boot().await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let tareq = w.staff("Tareq", TECHNICIAN).await;
    let (_, basma) = w.staff("Basma", BRANCH_MANAGER).await;
    let (_, wanda) = w.staff("Wanda", WORKSHOP_MANAGER).await;
    let owner = tenant(&w.app, "Mona Hassan").await;
    let before = wl_invoicing::Invoice::query().count(w.db()).await.unwrap();
    let card = inspected_card(&w, &sami, &tareq, owner, "12345", 0, vec![], false).await;
    let incident = incidents::report(
        w.state(),
        &tareq.1,
        card.id,
        NewIncident::new("Hit the post").road_accident(),
    )
    .await
    .unwrap();
    let decided = incidents::decide(w.state(), &basma, incident.id, "workshop", "Our fault")
        .await
        .unwrap();
    let internal = JobCard::find_or_404(w.db(), decided.internal_job_card_id.unwrap())
        .await
        .unwrap();
    assert!(internal.internal && internal.accident_damage);

    job_cards::assign_technician(w.state(), &sami, internal.id, tareq.0.id)
        .await
        .unwrap();
    job_cards::start_inspection(w.state(), &tareq.1, internal.id)
        .await
        .unwrap();
    let line = job_cards::add_labour(
        w.state(),
        &tareq.1,
        internal.id,
        NewLabour::hourly("Door repair", 1_000, 20_000),
    )
    .await
    .unwrap();
    job_cards::send_estimate(w.state(), &sami, internal.id, Promise::default())
        .await
        .unwrap();

    let refusal =
        job_cards::approve_recorded(w.state(), &sami, internal.id, "Basma", "recorded", None)
            .await
            .unwrap_err();
    assert!(
        refusal.to_string().contains("Police report required"),
        "{refusal}"
    );
    let report = upload_report(&w, &sami, internal.subject()).await;
    Documents::accept(w.state(), &wanda, &report).await.unwrap();
    let approved =
        job_cards::approve_recorded(w.state(), &sami, internal.id, "Basma", "recorded", None)
            .await
            .unwrap();
    assert_eq!(approved.status, JobCardStatus::InProgress);

    job_cards::record_work(w.state(), &tareq.1, line.id, WorkStatus::Done, None)
        .await
        .unwrap();
    job_cards::finish_work(w.state(), &tareq.1, internal.id)
        .await
        .unwrap();
    let all: Vec<String> = auto::job_card::QC_KEYS
        .iter()
        .map(|k| (*k).to_owned())
        .collect();
    job_cards::save_qc(w.state(), &wanda, internal.id, &all)
        .await
        .unwrap();
    job_cards::pass_qc(w.state(), &wanda, internal.id)
        .await
        .unwrap();
    let delivered = job_cards::deliver(
        w.state(),
        &sami,
        internal.id,
        Delivery {
            delivered_to: "Workshop yard".to_owned(),
            ..Delivery::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(delivered.status, JobCardStatus::Delivered);
    assert!(
        auto::billing::bill(w.state(), internal.id)
            .await
            .unwrap()
            .is_none()
    );
    let invoices = wl_invoicing::Invoice::query().count(w.db()).await.unwrap();
    assert_eq!(invoices, before, "no invoice was made");
    let books = wl_kernel::invoicing::invoicer(w.state());
    assert!(
        books
            .billing(w.db(), &internal.subject())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        books
            .billing(w.db(), &card.subject())
            .await
            .unwrap()
            .is_empty()
    );
}

/// JC-70 (a road accident) linked to CL-7: the report sits on the claim and is
/// accepted; the insurer rejects the claim; the customer repairs privately and
/// the job card's own `approve` passes on the report of the claim.
#[renox::test]
async fn aut_07_real_the_private_repair_after_a_rejected_claim_uses_the_report_of_the_claim() {
    let w = boot().await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let (tareq_user, tareq) = w.staff("Tareq", TECHNICIAN).await;
    let (_, wanda) = w.staff("Wanda", WORKSHOP_MANAGER).await;
    let (_, carla) = w.staff("Carla", INSURANCE_COORDINATOR).await;
    let owner = tenant(&w.app, "Mona Hassan").await;
    let vehicle = vehicles::create(
        w.state(),
        &sami,
        NewVehicle::new(Emirate::Dubai, "A", "12345", "Toyota", "Hilux", owner.0)
            .vin("JTHBK262582012345")
            .odometer(52_000),
    )
    .await
    .unwrap();
    let card = job_cards::check_in(
        w.state(),
        &sami,
        auto::job_cards::NewJobCard::new(vehicle.id, "Right side hit")
            .odometer(52_100)
            .condition("Right door dented")
            .acknowledged_by("Mona Hassan")
            .technician(tareq_user.id)
            .workshop(Subject::new("scheduling.location", 1))
            .accident("DXB-PR-70"),
    )
    .await
    .unwrap();
    job_cards::start_inspection(w.state(), &tareq, card.id)
        .await
        .unwrap();
    job_cards::add_labour(
        w.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Door panel repair", 2_000, 20_000),
    )
    .await
    .unwrap();
    job_cards::send_estimate(w.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();
    let orient = company(&w.app, "Orient Insurance").await;
    let insurer =
        auto::insurers::register(w.state(), &carla, auto::insurers::NewInsurer::new(orient.0))
            .await
            .unwrap();
    let claim = claims::register(
        w.state(),
        &carla,
        NewClaim::new(insurer.id, vehicle.id, "POL-7788", "OR-2026-118", day(-3))
            .job_card(card.id)
            .police_report("DXB-PR-70", Some(day(-2)))
            .excess(20_000),
    )
    .await
    .unwrap();
    let report = upload_report(&w, &carla, claim.subject()).await;
    Documents::accept(w.state(), &wanda, &report).await.unwrap();
    claims::submit_estimate(w.state(), &carla, claim.id)
        .await
        .unwrap();
    let rejected = claims::reject(w.state(), &carla, claim.id, "Not covered")
        .await
        .unwrap();
    assert_eq!(rejected.status, ClaimStatus::Rejected);

    let approved =
        job_cards::approve_recorded(w.state(), &sami, card.id, "Mona Hassan", "recorded", None)
            .await
            .expect("the report on the claim counts for the private repair");
    assert_eq!(approved.status, JobCardStatus::InProgress);
}
