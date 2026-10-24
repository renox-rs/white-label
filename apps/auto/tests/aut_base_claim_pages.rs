//! The insurance claim pages (base for #404, #405, #406, #413): the list with
//! its tabs and search, the register form with its pickers, the claim with the
//! actions of each role by state, the insurer's answer recorded line by line,
//! and the insurers.

mod support;

use auto::claims::{self, Answer};
use auto::{ClaimStatus, InsuranceClaim, permissions};
use renox::prelude::*;
use support::claimfx::{carla, orient, registered, sent_estimate};
use support::{boot, day};

/// The claim of the Hilux, registered and with the estimate submitted.
async fn submitted(t: &support::Garage) -> (InsuranceClaim, support::claimfx::Estimate) {
    let e = sent_estimate(t).await;
    let claim = registered(t, e.card.id).await;
    let carla = carla(t).await;
    let claim = {
        claims::submit_estimate(t.state(), &carla, claim.id)
            .await
            .unwrap()
    };
    (claim, e)
}

async fn sign_in(t: &support::Garage, name: &str, grants: &[&str]) -> User {
    let user = t.user(name, grants).await;
    t.app.acting_as(&user);
    user
}

/// The list shows the claim with its insurer, plate and status in words; the
/// tabs and the search narrow it; the accountant sees it, a user with no
/// claim permission is refused.
#[renox::test]
async fn aut_base_the_claim_list_shows_filters_and_searches() {
    let t = boot().await;
    let (claim, _e) = submitted(&t).await;
    sign_in(
        &t,
        "Carla",
        &[permissions::INSURANCE_MANAGE, permissions::VEHICLES_VIEW],
    )
    .await;

    t.app
        .get("/auto/claims")
        .await
        .assert_ok()
        .assert_see(&claim.number())
        .assert_see("OR-2026-118")
        .assert_see("Orient Insurance")
        .assert_see("Dubai A 12345")
        .assert_see("Estimate submitted");
    let open = t.app.get("/auto/claims?tab=open").await;
    open.assert_ok().assert_see(&claim.number());
    let closed = t.app.get("/auto/claims?tab=closed").await;
    assert!(!closed.text().contains(&claim.number()));
    let found = t.app.get("/auto/claims?q=POL-7788").await;
    found.assert_ok().assert_see(&claim.number());
    let none = t.app.get("/auto/claims?q=nothing-here").await;
    assert!(!none.text().contains(&claim.number()));

    sign_in(&t, "Aisha", &[permissions::INVOICES_ISSUE]).await;
    t.app.get("/auto/claims").await.assert_ok();
    sign_in(&t, "Nadia", &[permissions::VEHICLES_VIEW]).await;
    t.app.get("/auto/claims").await.assert_status(403);
    t.app
        .get(&format!("/auto/claims/{}", claim.id))
        .await
        .assert_status(403);
}

/// The register form picks the insurer, the vehicle and the job card from
/// lists; a form without the policy number is refused in place.
#[renox::test]
async fn aut_base_the_register_form_picks_the_insurer_vehicle_and_job_card() {
    let t = boot().await;
    let e = sent_estimate(&t).await;
    let insurer = orient(&t).await;
    let vehicle = t.hilux().await;
    sign_in(
        &t,
        "Carla",
        &[permissions::INSURANCE_MANAGE, permissions::VEHICLES_VIEW],
    )
    .await;

    t.app
        .get("/auto/claims")
        .await
        .assert_ok()
        .assert_see("Orient Insurance");
    t.app
        .get("/auto/claim-vehicle-options?q=hilux")
        .await
        .assert_ok()
        .assert_see("Dubai A 12345");
    t.app
        .get("/auto/claim-job-options?q=JOB")
        .await
        .assert_ok()
        .assert_see(&e.card.number());

    let insurer_id = insurer.id.to_string();
    let vehicle_id = vehicle.id.to_string();
    let job_id = e.card.id.to_string();
    let accident = day(-2).to_string();
    let refused = t
        .app
        .htmx()
        .post(
            "/auto/claims",
            &[
                ("insurer", insurer_id.as_str()),
                ("vehicle", vehicle_id.as_str()),
                ("claim_no", "OR-77"),
                ("accident_date", accident.as_str()),
            ],
        )
        .await;
    refused.assert_status(422);
    assert!(refused.text().contains("policy number"));

    t.app
        .htmx()
        .post(
            "/auto/claims",
            &[
                ("insurer", insurer_id.as_str()),
                ("vehicle", vehicle_id.as_str()),
                ("job_card", job_id.as_str()),
                ("policy_no", "POL-1"),
                ("claim_no", "OR-77"),
                ("accident_date", accident.as_str()),
                ("police_report_no", "DXB-PR-1"),
                ("repair_type", "agency"),
                ("excess", "200"),
            ],
        )
        .await
        .assert_ok();
    let claim = InsuranceClaim::query()
        .first(t.db())
        .await
        .unwrap()
        .expect("the claim");
    assert_eq!(
        (claim.status, claim.excess, claim.claim_no.as_str()),
        (ClaimStatus::Registered, 20_000, "OR-77")
    );
    assert_eq!(claim.repair_type, auto::RepairType::Agency);
    assert_eq!(claim.job_card_id, Some(e.card.id));
}

/// The claim page shows the details, the job card link and the actions of the
/// coordinator by state; the insurer's answer is recorded on the page with an
/// amount per line, and the lines left blank are not approved.
#[renox::test]
async fn aut_base_the_claim_page_records_the_insurers_answer_line_by_line() {
    let t = boot().await;
    let (claim, e) = submitted(&t).await;
    sign_in(
        &t,
        "Carla",
        &[permissions::INSURANCE_MANAGE, permissions::VEHICLES_VIEW],
    )
    .await;

    let page = t.app.get(&format!("/auto/claims/{}", claim.id)).await;
    page.assert_ok()
        .assert_see(&claim.number())
        .assert_see("POL-7788")
        .assert_see("DXB-PR-5521")
        .assert_see("AED 200.00")
        .assert_see("AED 1,100.00")
        .assert_see("Waiting for the insurer")
        .assert_see("Record the approval")
        .assert_see("Record the rejection")
        .assert_see("Door panel repair")
        .assert_see(&e.card.number());
    assert!(!page.text().contains("Invoice the insurer"));

    let door = format!("line_labour_{}", e.door.id);
    let bumper = format!("line_part_{}", e.bumper.id);
    let alignment = format!("line_labour_{}", e.alignment.id);
    let refused = t
        .app
        .htmx()
        .post(
            &format!("/auto/claims/{}/approve", claim.id),
            &[
                ("reference", "LPO-1"),
                (door.as_str(), "400"),
                (bumper.as_str(), "550"),
            ],
        )
        .await;
    refused.assert_status(422);
    assert!(refused.text().contains("written approval"));

    t.app
        .htmx()
        .post(
            &format!("/auto/claims/{}/approve", claim.id),
            &[
                ("reference", "LPO-1"),
                ("document", "orient-email-1"),
                ("excess", "250"),
                (door.as_str(), "400"),
                (bumper.as_str(), "550"),
                (alignment.as_str(), ""),
            ],
        )
        .await
        .assert_ok();
    let approved = InsuranceClaim::find_or_404(t.db(), claim.id).await.unwrap();
    assert_eq!(approved.status, ClaimStatus::Approved);
    assert_eq!(
        (approved.approved_amount, approved.excess),
        (95_000, 25_000)
    );
    t.app
        .get(&format!("/auto/claims/{}", claim.id))
        .await
        .assert_ok()
        .assert_see("Approved")
        .assert_see("LPO-1")
        .assert_see("AED 950.00")
        .assert_see("Declare a total loss")
        .assert_see("Withdraw the claim");
}

/// The coordinator rejects, withdraws or voids from the page, each with its
/// reason; a reason left blank is refused in place.
#[renox::test]
async fn aut_base_the_claim_page_rejects_withdraws_and_voids() {
    let t = boot().await;
    let (claim, _e) = submitted(&t).await;
    sign_in(&t, "Carla", &[permissions::INSURANCE_MANAGE]).await;

    let blank = t
        .app
        .htmx()
        .post(
            &format!("/auto/claims/{}/reject", claim.id),
            &[("reason", " ")],
        )
        .await;
    blank.assert_status(422);
    t.app
        .htmx()
        .post(
            &format!("/auto/claims/{}/reject", claim.id),
            &[("reason", "Policy lapsed")],
        )
        .await
        .assert_ok();
    let rejected = InsuranceClaim::find_or_404(t.db(), claim.id).await.unwrap();
    assert_eq!(rejected.status, ClaimStatus::Rejected);
    t.app
        .get(&format!("/auto/claims/{}", claim.id))
        .await
        .assert_ok()
        .assert_see("Rejected")
        .assert_see("Policy lapsed");

    // Another claim registered in error is voided.
    let vehicle = t.hilux().await;
    let carla = carla(&t).await;
    let other = claims::register(
        t.state(),
        &carla,
        auto::claims::NewClaim::new(claim.insurer_id, vehicle.id, "POL-2", "OR-2", day(-3)),
    )
    .await
    .unwrap();
    t.app
        .htmx()
        .post(
            &format!("/auto/claims/{}/void", other.id),
            &[("reason", "Duplicate")],
        )
        .await
        .assert_ok();
    assert_eq!(
        InsuranceClaim::find_or_404(t.db(), other.id)
            .await
            .unwrap()
            .status,
        ClaimStatus::Voided
    );
    sign_in(&t, "Sami", &[permissions::JOB_CARDS_ADVISE]).await;
    t.app
        .htmx()
        .post(
            &format!("/auto/claims/{}/void", other.id),
            &[("reason", "x")],
        )
        .await
        .assert_status(403);
}

/// The accountant sees the invoice action once the repair is complete, and
/// only then.
#[renox::test]
async fn aut_base_the_invoice_action_is_the_accountants_and_follows_the_repair() {
    let t = boot().await;
    let (claim, e) = submitted(&t).await;
    let carla = carla(&t).await;
    claims::approve(
        t.state(),
        &carla,
        claim.id,
        Answer::new("LPO-1", "email-1")
            .labour(e.door.id, 40_000)
            .part(e.bumper.id, 55_000),
    )
    .await
    .unwrap();
    sign_in(&t, "Aisha", &[permissions::INVOICES_ISSUE]).await;
    let page = t.app.get(&format!("/auto/claims/{}", claim.id)).await;
    page.assert_ok();
    assert!(!page.text().contains("Invoice the insurer"));
    t.app
        .htmx()
        .post(&format!("/auto/claims/{}/invoice", claim.id), &[])
        .await
        .assert_status(422);
}

/// The insurers page lists the insurers and records one from the party
/// picker; the insurer's page shows its terms and the rate card in force.
#[renox::test]
async fn aut_base_the_insurer_pages_record_and_show_the_terms() {
    let t = boot().await;
    t.people.add(9, "Orient Insurance");
    t.people.add(10, "Gulf Cover");
    sign_in(&t, "Carla", &[permissions::INSURANCE_MANAGE]).await;
    t.app.get("/auto/insurers").await.assert_ok();

    let refused = t
        .app
        .htmx()
        .post(
            "/auto/insurers",
            &[("party", "P-00099"), ("email", "claims@orient.example")],
        )
        .await;
    refused.assert_status(422);
    t.app
        .htmx()
        .post(
            "/auto/insurers",
            &[
                ("party", "P-00009"),
                ("agreement", "3"),
                ("email", "claims@orient.example"),
                ("terms", "45"),
                ("labour_rate", "120"),
            ],
        )
        .await
        .assert_ok();
    let insurer = auto::Insurer::query()
        .first(t.db())
        .await
        .unwrap()
        .expect("the insurer");
    assert_eq!(
        (
            insurer.payment_terms_days,
            insurer.labour_rate,
            insurer.payer_agreement_id
        ),
        (45, Some(12_000), Some(3))
    );
    t.cards.accept(insurer.subject(), 31, 2, day(-30));
    t.app
        .get("/auto/insurers")
        .await
        .assert_ok()
        .assert_see("Orient Insurance")
        .assert_see("45 days")
        .assert_see("AED 120.00")
        .assert_see("Takes new claims");
    t.app
        .get(&format!("/auto/insurers/{}", insurer.id))
        .await
        .assert_ok()
        .assert_see("claims@orient.example")
        .assert_see("#31 v2")
        .assert_see("Stop new claims");
    t.app
        .htmx()
        .post(&format!("/auto/insurers/{}/active", insurer.id), &[])
        .await
        .assert_ok();
    assert!(
        !auto::Insurer::find_or_404(t.db(), insurer.id)
            .await
            .unwrap()
            .active
    );
    sign_in(&t, "Sami", &[permissions::JOB_CARDS_ADVISE]).await;
    t.app.get("/auto/insurers").await.assert_status(403);
}
