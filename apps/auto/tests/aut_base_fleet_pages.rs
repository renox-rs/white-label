//! The fleet contract pages (base for #408, #409, #413): the list with its
//! tabs and search, the form that agrees a contract, the contract with its
//! vehicles, service plan, bookings, job cards and invoices and the actions of
//! each role by state.

mod support;

use auto::{FleetContract, FleetService, FleetStatus, Vehicle, permissions};
use renox::prelude::*;
use support::fleetfx::{FLEET, active_contract, bay_and_service, draft_contract, van};
use support::{boot, day};

async fn sign_in(t: &support::Garage, name: &str, grants: &[&str]) -> User {
    let user = t.user(name, grants).await;
    t.app.acting_as(&user);
    user
}

const MANAGER: [&str; 4] = [
    permissions::FLEET_CONTRACTS_MANAGE,
    permissions::VEHICLES_VIEW,
    permissions::VEHICLES_MANAGE,
    permissions::JOB_CARDS_VIEW,
];

/// The list shows the contract with its fleet, term, vehicles and status in
/// words; the tabs and the search narrow it; the advisor and the accountant
/// may look, a user with no fleet permission is refused.
#[renox::test]
async fn aut_base_the_contract_list_shows_filters_and_searches() {
    let t = boot().await;
    let (active, _van, _plan) = active_contract(&t, 59_800).await;
    let draft = draft_contract(&t, 0, 100).await;
    sign_in(&t, "Fatima", &MANAGER).await;

    t.app
        .get("/auto/fleet")
        .await
        .assert_ok()
        .assert_see(&active.number())
        .assert_see(&draft.number())
        .assert_see("Al Noor Logistics")
        .assert_see("Active")
        .assert_see("Draft");
    let in_force = t.app.get("/auto/fleet?in.status=active").await;
    in_force.assert_ok().assert_see(&active.number());
    assert!(!in_force.text().contains(&draft.number()));
    let drafts = t.app.get("/auto/fleet?in.status=draft").await;
    drafts.assert_ok().assert_see(&draft.number());
    assert!(!drafts.text().contains(&active.number()));
    t.app
        .get(&format!("/auto/fleet?search={}", draft.number()))
        .await
        .assert_ok()
        .assert_see(&draft.number());

    sign_in(&t, "Aisha", &[permissions::INVOICES_ISSUE]).await;
    t.app.get("/auto/fleet").await.assert_ok();
    sign_in(&t, "Nadia", &[permissions::VEHICLES_VIEW]).await;
    t.app.get("/auto/fleet").await.assert_status(403);
    t.app
        .get(&format!("/auto/fleet/{}", active.id))
        .await
        .assert_status(403);
}

/// The form agrees a contract with the fleet and its contact picked from the
/// parties; a form without the last day is refused in place.
#[renox::test]
async fn aut_base_the_form_agrees_a_contract() {
    let t = boot().await;
    t.people.add(FLEET, "Al Noor Logistics");
    t.people.add(21, "Hamdan Saeed");
    sign_in(&t, "Fatima", &MANAGER).await;
    let start = day(0).to_string();
    let end = day(365).to_string();

    let refused = t
        .app
        .htmx()
        .post(
            "/auto/fleet",
            &[
                ("fleet", "P-00020"),
                ("title", "Fleet 2027"),
                ("start_date", start.as_str()),
            ],
        )
        .await;
    refused.assert_status(422);
    assert!(refused.text().contains("last day of service"));

    t.app
        .htmx()
        .post(
            "/auto/fleet",
            &[
                ("fleet", "P-00020"),
                ("contact", "P-00021"),
                ("title", "Fleet 2027"),
                ("start_date", start.as_str()),
                ("end_date", end.as_str()),
                ("labour_rate", "120"),
                ("fee", "2000"),
                ("notice_days", "45"),
            ],
        )
        .await
        .assert_ok();
    let contract = FleetContract::query()
        .first(t.db())
        .await
        .unwrap()
        .expect("the contract");
    assert_eq!(
        (
            contract.fleet_party_id,
            contract.contact_party_id,
            contract.status
        ),
        (FLEET, Some(21), FleetStatus::Draft)
    );
    assert_eq!(
        (
            contract.labour_rate,
            contract.early_termination_fee,
            contract.notice_period_days
        ),
        (12_000, 200_000, 45)
    );
}

/// The contract page shows the terms, the vehicles, the plan and the actions
/// by state; the draft is built up on the page and activated; a suspended
/// contract shows its reason and the bookings made.
#[renox::test]
async fn aut_base_the_contract_page_builds_up_activates_suspends_and_resumes() {
    let t = boot().await;
    bay_and_service(&t).await;
    let draft = draft_contract(&t, 0, 300).await;
    let van = van(&t, 59_800).await;
    sign_in(&t, "Fatima", &MANAGER).await;
    let path = format!("/auto/fleet/{}", draft.id);

    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see(&draft.number())
        .assert_see("Fleet service 2026")
        .assert_see("AED 120.00")
        .assert_see("AED 2,000.00")
        .assert_see("Not attached")
        .assert_see("Activate")
        .assert_see("Put a vehicle under contract");
    t.app
        .get("/auto/fleet-vehicle-options?q=urvan")
        .await
        .assert_ok()
        .assert_see("Dubai B 555");

    let vehicle_id = van.id.to_string();
    t.app
        .htmx()
        .post(&format!("{path}/activate"), &[])
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(
            &format!("{path}/vehicles"),
            &[("vehicle", vehicle_id.as_str())],
        )
        .await
        .assert_ok();
    assert_eq!(
        Vehicle::find_or_404(t.db(), van.id)
            .await
            .unwrap()
            .fleet_contract_id,
        Some(draft.id)
    );
    t.app
        .htmx()
        .post(
            &format!("{path}/plans"),
            &[
                ("vehicle", vehicle_id.as_str()),
                ("service", "periodic_service"),
                ("interval_km", "10000"),
                ("last_date", day(-100).to_string().as_str()),
                ("last_km", "50,000"),
            ],
        )
        .await
        .assert_ok();
    let plan = FleetService::query()
        .first(t.db())
        .await
        .unwrap()
        .expect("the plan");
    assert_eq!(plan.next_due_km, Some(60_000));
    t.app
        .htmx()
        .post(&format!("{path}/document"), &[("document", "signed-2026")])
        .await
        .assert_ok();
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("every 10,000 km")
        .assert_see("60,000 km")
        .assert_see("signed-2026");
    t.app
        .htmx()
        .post(&format!("{path}/activate"), &[])
        .await
        .assert_ok();
    assert_eq!(
        FleetContract::find_or_404(t.db(), draft.id)
            .await
            .unwrap()
            .status,
        FleetStatus::Active
    );

    // Suspended with a reason: it is shown, and resuming needs the contract
    // suspended.
    let blank = t
        .app
        .htmx()
        .post(&format!("{path}/suspend"), &[("reason", " ")])
        .await;
    blank.assert_status(422);
    t.app
        .htmx()
        .post(&format!("{path}/suspend"), &[("reason", "Unpaid invoices")])
        .await
        .assert_ok();
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("Suspended")
        .assert_see("Unpaid invoices")
        .assert_see("Resume");
    t.app
        .htmx()
        .post(&format!("{path}/resume"), &[])
        .await
        .assert_ok();
    assert_eq!(
        FleetContract::find_or_404(t.db(), draft.id)
            .await
            .unwrap()
            .status,
        FleetStatus::Active
    );
    t.app
        .htmx()
        .post(
            &format!("{path}/vehicles/remove"),
            &[("vehicle", vehicle_id.as_str())],
        )
        .await
        .assert_ok();
}

/// The job cards of the contract show their PO reference and the PO is set on
/// the page; a period is invoiced from the page.
#[renox::test]
async fn aut_base_the_contract_page_sets_the_po_and_invoices_a_period() {
    let t = boot().await;
    let (contract, van, _plan) = active_contract(&t, 59_800).await;
    let sami = t.advisor().await;
    let card = auto::job_cards::check_in(
        t.state(),
        &sami,
        auto::job_cards::NewJobCard::new(van.id, "Service")
            .odometer(59_900)
            .condition("Clean"),
    )
    .await
    .unwrap();
    sign_in(&t, "Fatima", &MANAGER).await;
    let path = format!("/auto/fleet/{}", contract.id);

    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see(&card.number())
        .assert_see("Invoice a period");
    sign_in(&t, "Sami", &[permissions::JOB_CARDS_ADVISE]).await;
    t.app
        .htmx()
        .post(
            &format!("/auto/fleet/jobs/{}/po", card.id),
            &[("po", "PO-77")],
        )
        .await
        .assert_ok();
    sign_in(&t, "Fatima", &MANAGER).await;
    t.app.get(&path).await.assert_ok().assert_see("PO-77");

    let from = day(-30).to_string();
    let to = day(-1).to_string();
    t.app
        .htmx()
        .post(
            &format!("{path}/invoice"),
            &[("from", from.as_str()), ("to", to.as_str())],
        )
        .await
        .assert_ok();
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("Monthly")
        .assert_see(&format!("{} to {}", day(-30), day(-1)));
    let again = t
        .app
        .htmx()
        .post(
            &format!("{path}/invoice"),
            &[("from", from.as_str()), ("to", to.as_str())],
        )
        .await;
    again.assert_status(422);
}
