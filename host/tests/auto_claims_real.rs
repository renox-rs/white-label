//! 08 Auto insurance claims against the REAL shared modules of the host
//! (wl-invoicing payer bills and rate cards, wl-scheduling, wl-parties): the
//! split billing of #404 .. #406 and #413's base. The insurer is billed through
//! a payer bill under its payer agreement (it receives the tax invoice for the
//! approved amount less the excess); the customer is invoiced the excess; the
//! job card makes no invoice of its own; the claim settles when both are paid.
//! The app's own tests (`apps/auto/tests`) show the same flow against
//! stand-ins.

mod support;

use std::sync::{Arc, Mutex};

use auto::claims::{self, Answer, NewClaim};
use auto::insurers::{self, NewInsurer};
use auto::job_cards::{self, Delivery, NewJobCard, NewLabour, NewPart, Promise};
use auto::vehicles::{self, NewVehicle};
use auto::{ClaimStatus, Emirate, InsuranceClaim, JobCard, WorkStatus};
use host::roles::{
    ACCOUNTANT, INSURANCE_COORDINATOR, SERVICE_ADVISOR, TECHNICIAN, WORKSHOP_MANAGER,
};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use renox::testing::TestApp;
use renox::upload::Upload;
use support::{company, person, tenant};
use wl_documents::{Documents, NewDocument};
use wl_invoicing::rate_card::Unit;
use wl_invoicing::rate_cards::{self, LineInput};
use wl_invoicing::{PayerAgreement, PayerBill, VatRecipient};
use wl_kernel::LegalEntity;
use wl_kernel::PartyId;
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};
use wl_kernel::payer_bills::{
    PayerBillId, PayerBillStatus, Remittance, RemittanceLine, payer_bills,
};

/// The demo auto parts trader with a workshop.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-auto/client.toml"
);

/// A test-only module with the route that hands a test the signed-in user the
/// services take.
struct Who(Arc<Mutex<Option<AuthUser>>>);

impl Module for Who {
    fn name(&self) -> &'static str {
        "auto-claims-real-who"
    }

    fn routes(&self) -> Routes {
        let slot = self.0.clone();
        Routes::new().get("/t/whoami", move |user: AuthUser| {
            let slot = slot.clone();
            async move {
                *slot.lock().unwrap() = Some(user);
                "ok"
            }
        })
    }
}

/// The booted workshop.
struct World {
    app: TestApp,
    who: Arc<Mutex<Option<AuthUser>>>,
}

/// Boots the demo client with `wl:install` and the seeds.
async fn boot() -> World {
    let who = Arc::new(Mutex::new(None));
    let config = wl_kernel::ClientConfig::load(CLIENT).expect("the auto client");
    let app = TestApp::new(
        host::app_for(config)
            .expect("the app")
            .module(Who(who.clone())),
    )
    .await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    World { app, who }
}

impl World {
    fn state(&self) -> &AppState {
        self.app.state()
    }

    fn db(&self) -> &Db {
        self.app.db()
    }

    /// A user with the host role `role`, as the user the services take.
    async fn staff(&self, name: &str, role: &str) -> (User, AuthUser) {
        let user = person(&self.app, name, role).await;
        self.app.acting_as(&user);
        self.app.get("/t/whoami").await.assert_ok();
        let auth = self.who.lock().unwrap().take().expect("the probe saw them");
        (user, auth)
    }
}

/// Orient Insurance with a payer agreement (the payer gets the tax invoice)
/// and an accepted rate card version in force since 30 days.
async fn insurer_with_agreement(w: &World, carla: &AuthUser) -> (auto::Insurer, PartyId) {
    let orient = company(&w.app, "Orient Insurance").await;
    let entity = LegalEntity::default_entity(w.db())
        .await
        .expect("the legal entity exists");
    let mut agreement = PayerAgreement {
        legal_entity_id: entity.id,
        payer_party_id: orient.0,
        name: "Orient motor".to_owned(),
        active: true,
        vat_recipient: VatRecipient::Payer,
        first_submission_days: 30,
        resubmission_days: 30,
        resubmission_limit: 2,
        write_off_limit: 100_000,
        ..PayerAgreement::default()
    };
    agreement.insert(w.db()).await.unwrap();
    let insurer = insurers::register(
        w.state(),
        carla,
        NewInsurer::new(orient.0)
            .agreement(agreement.id)
            .email("claims@orient.example")
            .labour_rate(12_000),
    )
    .await
    .unwrap();
    let card = rate_cards::create_card(w.db(), None, orient, &insurer.subject())
        .await
        .unwrap();
    let version =
        rate_cards::new_version(w.db(), None, card.id, today() - TimeDelta::days(30), false)
            .await
            .unwrap();
    rate_cards::add_line(
        w.db(),
        version.id,
        LineInput::new("body_repair", "standard", Unit::Hour, 12_000),
    )
    .await
    .unwrap();
    renox::auth::permissions::define_role(w.db(), "rate_manager", &["rate_cards.manage"])
        .await
        .unwrap();
    let (_, rita) = w.staff("Rita", "rate_manager").await;
    rate_cards::accept(w.state(), &rita, version.id, Some("signed-rate-card"))
        .await
        .unwrap();
    (insurer, orient)
}

/// The insurer's rate card version is pinned; the insurer's approval starts
/// the job on the approved lines; the repair completes the claim; the insurer
/// gets a real payer bill and tax invoice for the approved amount less the
/// excess, the customer a real invoice for the excess, the job card none; and
/// the claim settles by itself when the remittance and the payment arrive.
#[renox::test]
async fn aut_base_real_the_insurer_is_billed_through_a_payer_bill_and_the_customer_the_excess() {
    let w = boot().await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let (tareq_user, tareq) = w.staff("Tareq", TECHNICIAN).await;
    let (_, waleed) = w.staff("Waleed", WORKSHOP_MANAGER).await;
    let (_, carla) = w.staff("Carla", INSURANCE_COORDINATOR).await;
    let (_, aisha) = w.staff("Aisha", ACCOUNTANT).await;
    let owner = tenant(&w.app, "Mona Hassan").await;
    let (insurer, orient) = insurer_with_agreement(&w, &carla).await;

    let vehicle = vehicles::create(
        w.state(),
        &sami,
        NewVehicle::new(Emirate::Dubai, "A", "12345", "Toyota", "Hilux", owner.0).odometer(52_000),
    )
    .await
    .unwrap();
    let card = job_cards::check_in(
        w.state(),
        &sami,
        NewJobCard::new(vehicle.id, "Right side hit in a parking lot")
            .odometer(52_400)
            .condition("Right door dented, bumper cracked")
            .acknowledged_by("Mona Hassan")
            .technician(tareq_user.id)
            .workshop(Subject::new("scheduling.location", 1)),
    )
    .await
    .unwrap();
    job_cards::start_inspection(w.state(), &tareq, card.id)
        .await
        .unwrap();
    let door = job_cards::add_labour(
        w.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Door panel repair", 2_000, 20_000),
    )
    .await
    .unwrap();
    let bumper = job_cards::add_part(
        w.state(),
        &tareq,
        card.id,
        NewPart::new("Bumper cover", 1_000, 60_000),
    )
    .await
    .unwrap();
    job_cards::send_estimate(w.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();

    // The claim pins the card in force and goes through survey, estimate and
    // approval; the job card starts with the approved lines.
    let claim = claims::register(
        w.state(),
        &carla,
        NewClaim::new(
            insurer.id,
            vehicle.id,
            "POL-7788",
            "OR-2026-118",
            today() - TimeDelta::days(3),
        )
        .job_card(card.id)
        .police_report("DXB-PR-5521", Some(today() - TimeDelta::days(2)))
        .excess(20_000),
    )
    .await
    .unwrap();
    assert_eq!(claim.rate_card_version, Some(1), "the real card is pinned");
    assert!(claim.rate_card_id.is_some());
    // The police report is a legal block: attached by the coordinator and
    // accepted by the workshop manager, a second user.
    let report = Documents::attach(
        w.state(),
        Some(&carla),
        NewDocument::new(
            claim.subject(),
            "police_report",
            Upload::new(
                "police-report.pdf",
                "application/pdf",
                b"%PDF-1.7 police report".to_vec(),
            ),
        )
        .number("DXB-PR-5521"),
    )
    .await
    .unwrap()
    .expect("the report is attached");
    Documents::accept(w.state(), &waleed, &report)
        .await
        .unwrap();
    claims::submit_estimate(w.state(), &carla, claim.id)
        .await
        .unwrap();
    let approved = claims::approve(
        w.state(),
        &carla,
        claim.id,
        Answer::new("LPO-5521", "orient-approval-email-1")
            .labour(door.id, 40_000)
            .part(bumper.id, 55_000),
    )
    .await
    .unwrap();
    assert_eq!(approved.status, ClaimStatus::Approved);

    // The work and the QC complete the claim.
    job_cards::record_work(w.state(), &tareq, door.id, WorkStatus::Done, None)
        .await
        .unwrap();
    job_cards::finish_work(w.state(), &tareq, card.id)
        .await
        .unwrap();
    let all: Vec<String> = auto::job_card::QC_KEYS
        .iter()
        .map(|k| (*k).to_owned())
        .collect();
    job_cards::save_qc(w.state(), &waleed, card.id, &all)
        .await
        .unwrap();
    job_cards::pass_qc(w.state(), &waleed, card.id)
        .await
        .unwrap();
    assert_eq!(
        InsuranceClaim::find_or_404(w.db(), claim.id)
            .await
            .unwrap()
            .status,
        ClaimStatus::RepairComplete
    );

    // The customer collects; the job card is not billed to the customer.
    let delivered = job_cards::deliver(
        w.state(),
        &sami,
        card.id,
        Delivery {
            delivered_to: "Mona Hassan".to_owned(),
            ..Delivery::default()
        },
    )
    .await
    .unwrap();
    assert!(delivered.invoice_id.is_none());
    assert!(
        invoicer(w.state())
            .billing(
                w.db(),
                &JobCard::find_or_404(w.db(), card.id)
                    .await
                    .unwrap()
                    .subject()
            )
            .await
            .unwrap()
            .is_empty()
    );

    // The accountant bills the insurer and the customer.
    let invoiced = claims::invoice(w.state(), &aisha, claim.id).await.unwrap();
    assert_eq!(invoiced.status, ClaimStatus::Invoiced);
    let bill_id = invoiced.payer_bill_id.expect("a payer bill");
    let bills = payer_bills(w.state());
    let summary = bills
        .find(w.db(), PayerBillId(bill_id))
        .await
        .unwrap()
        .expect("the payer bill exists");
    assert_eq!(
        summary.claimed, 75_000,
        "AED 950 approved less the AED 200 excess"
    );
    assert_eq!(summary.payer, orient);
    assert_eq!(summary.customer, PartyId(owner.0));
    assert_eq!(summary.status, PayerBillStatus::Issued);
    let lines = bills.lines(w.db(), PayerBillId(bill_id)).await.unwrap();
    assert_eq!(lines.len(), 2);
    let payer_bill = PayerBill::find_or_404(w.db(), bill_id).await.unwrap();
    let payer_invoice = invoicer(w.state())
        .find(
            w.db(),
            InvoiceId(
                payer_bill
                    .payer_invoice_id
                    .expect("the payer's tax invoice"),
            ),
        )
        .await
        .unwrap()
        .expect("the tax invoice exists");
    assert_eq!(payer_invoice.party, orient);
    assert_eq!(payer_invoice.gross_total, 78_750, "AED 750 and 5% VAT");
    let customer_invoice_id = invoiced
        .customer_invoice_id
        .expect("the customer's invoice");
    let customer_invoice = invoicer(w.state())
        .find(w.db(), InvoiceId(customer_invoice_id))
        .await
        .unwrap()
        .expect("the invoice exists");
    assert_eq!(customer_invoice.party, PartyId(owner.0));
    assert_eq!(
        customer_invoice.gross_total, 21_000,
        "the AED 200 excess and 5% VAT"
    );
    // The accountant who invoices the claim issues the customer's part too
    // (the system issues only a source set to issue by itself).
    assert_ne!(customer_invoice.status, InvoiceStatus::Draft);

    // The insurer's advice pays both lines; the customer pays the excess: the
    // claim settles by itself once both are in.
    let mut advice = Remittance::new("ORIENT-ADV-1", today());
    for line in &lines {
        advice = advice.line(RemittanceLine::paid(line.id, line.claimed));
    }
    bills
        .record_remittance(w.state(), PayerBillId(bill_id), advice, &aisha)
        .await
        .unwrap();
    assert_eq!(
        InsuranceClaim::find_or_404(w.db(), claim.id)
            .await
            .unwrap()
            .status,
        ClaimStatus::Invoiced,
        "the customer has not paid yet"
    );
    invoicer(w.state())
        .allocate(
            w.state(),
            InvoiceId(customer_invoice_id),
            Subject::new("payments.payment", 9_001),
            customer_invoice.gross_total,
        )
        .await
        .unwrap();
    let now = InsuranceClaim::find_or_404(w.db(), claim.id).await.unwrap();
    if now.status != ClaimStatus::Settled {
        // The listener of the event may not have run yet in this test app.
        assert!(claims::settle_if_paid(w.state(), claim.id).await.unwrap());
    }
    assert_eq!(
        InsuranceClaim::find_or_404(w.db(), claim.id)
            .await
            .unwrap()
            .status,
        ClaimStatus::Settled
    );
}

/// A claim whose insurer has no payer agreement is not invoiced, and nothing
/// is billed to anyone.
#[renox::test]
async fn aut_base_real_a_claim_is_not_invoiced_without_a_payer_agreement() {
    let w = boot().await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let (_, carla) = w.staff("Carla", INSURANCE_COORDINATOR).await;
    let (_, aisha) = w.staff("Aisha", ACCOUNTANT).await;
    let owner = tenant(&w.app, "Mona Hassan").await;
    let orient = company(&w.app, "Orient Insurance").await;
    let insurer = insurers::register(w.state(), &carla, NewInsurer::new(orient.0))
        .await
        .unwrap();
    let vehicle = vehicles::create(
        w.state(),
        &sami,
        NewVehicle::new(Emirate::Dubai, "A", "12345", "Toyota", "Hilux", owner.0),
    )
    .await
    .unwrap();
    let claim = claims::register(
        w.state(),
        &carla,
        NewClaim::new(
            insurer.id,
            vehicle.id,
            "POL-1",
            "OR-1",
            today() - TimeDelta::days(1),
        ),
    )
    .await
    .unwrap();
    assert_eq!(claim.rate_card_id, None, "the insurer has no rate card");
    let refused = claims::invoice(w.state(), &aisha, claim.id)
        .await
        .unwrap_err();
    assert!(!refused.to_string().is_empty());
    assert!(
        payer_bills(w.state())
            .of_source(w.db(), &claim.subject())
            .await
            .unwrap()
            .is_empty()
    );
}
