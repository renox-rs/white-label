//! The prior authorisation lifecycle (05 §7.1; base for BL-CLN-02 and
//! BL-CLN-03): request, approval with the payer's reference and validity,
//! partial approval, denial with its reason, units consumed until the
//! authorisation is used, expiry, cancel and the payer's list of services
//! that need one — through the real routes and transitions.

mod support;

use renox::db::now;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::notify::NotifyRecipient;

use clinics::authorisations::{self, DENIED_NOTICE};
use clinics::policies::{self, NewPolicy};
use clinics::{AuthStatus, InsurancePolicy, PriorAuthService, PriorAuthorisation};
use support::{Clinic, PATIENT, at, boot, day};

const PAYER: PartyId = PartyId(900);

async fn policy(t: &Clinic) -> InsurancePolicy {
    policies::create(
        t.state(),
        None,
        NewPolicy::new(PATIENT, PAYER, "Gold", "M-1", day(-200)).valid_to(day(300)),
    )
    .await
    .expect("the policy is recorded")
}

/// The authorisation prepared through the route: 3 units of laser.
async fn prepared(t: &Clinic, policy: &InsurancePolicy) -> PriorAuthorisation {
    let bea = t.coordinator().await;
    t.app.acting_as(&bea);
    t.app
        .htmx()
        .post(
            "/clinics/authorisations",
            &[
                ("policy", policy.id.to_string().as_str()),
                ("services", "laser, laser"),
                ("units", "3"),
            ],
        )
        .await
        .assert_ok();
    PriorAuthorisation::query()
        .order_by_desc(PriorAuthorisation::ID)
        .first(t.db())
        .await
        .unwrap()
        .expect("the authorisation was prepared")
}

async fn act(
    t: &Clinic,
    id: i64,
    action: &str,
    fields: &[(&str, &str)],
) -> renox::testing::TestResponse {
    t.app
        .htmx()
        .post(&format!("/clinics/authorisations/{id}/{action}"), fields)
        .await
}

async fn list(t: &Clinic, code: &str) -> renox::testing::TestResponse {
    t.app
        .htmx()
        .post(
            "/clinics/prior-auth-services",
            &[("payer", "900"), ("code", code)],
        )
        .await
}

async fn status(t: &Clinic, id: i64) -> AuthStatus {
    PriorAuthorisation::find_or_404(t.db(), id)
        .await
        .unwrap()
        .status
}

/// A draft is requested, approved with the payer's reference and validity,
/// and consumed down to `used`; each step is a transition, a user without
/// `insurance.manage` is refused, and approval without a reference or with a
/// validity in the past is refused.
#[renox::test]
async fn an_authorisation_is_requested_approved_and_used_up() {
    let t = boot().await;
    let policy = policy(&t).await;
    let a = prepared(&t, &policy).await;
    assert_eq!(a.status, AuthStatus::Draft);
    assert_eq!(a.services.0, ["laser"]);
    assert_eq!(a.patient_party_id, PATIENT.0);
    assert_eq!(a.payer_party_id, PAYER.0);

    // The receptionist records coverage but does not manage authorisations.
    let rita = t.receptionist().await;
    t.app.acting_as(&rita);
    act(&t, a.id, "request", &[("reference", "REQ-1")])
        .await
        .assert_forbidden();
    assert_eq!(status(&t, a.id).await, AuthStatus::Draft);

    let bea = t.coordinator().await;
    t.app.acting_as(&bea);
    act(
        &t,
        a.id,
        "approve",
        &[("reference", "A-1"), ("valid_until", &day(30).to_string())],
    )
    .await
    .assert_status(422);
    act(&t, a.id, "request", &[("reference", "REQ-1")])
        .await
        .assert_ok();
    assert_eq!(status(&t, a.id).await, AuthStatus::Requested);

    // No reference given at the approval: the one of the request stands, but
    // a validity in the past is refused.
    act(
        &t,
        a.id,
        "approve",
        &[("valid_until", &day(-1).to_string())],
    )
    .await
    .assert_status(422);
    act(&t, a.id, "approve", &[("valid_until", "")])
        .await
        .assert_status(422);
    act(
        &t,
        a.id,
        "approve",
        &[("valid_until", &day(30).to_string())],
    )
    .await
    .assert_ok();
    let approved = PriorAuthorisation::find_or_404(t.db(), a.id).await.unwrap();
    assert_eq!(approved.status, AuthStatus::Approved);
    assert_eq!(approved.units_approved, 3);
    assert_eq!(approved.reference.as_deref(), Some("REQ-1"));
    assert_eq!(approved.valid_until, Some(day(30)));
    assert!(approved.is_valid_on(day(30)));
    assert!(!approved.is_valid_on(day(31)));

    // Units consumed: some left, then the rest.
    let after = authorisations::consume(t.state(), a.id, 1, now())
        .await
        .unwrap();
    assert_eq!(
        (after.status, after.units_left()),
        (AuthStatus::Approved, 2)
    );
    authorisations::consume(t.state(), a.id, 3, now())
        .await
        .expect_err("more than are left");
    let last = authorisations::consume(t.state(), a.id, 2, now())
        .await
        .unwrap();
    assert_eq!((last.status, last.units_left()), (AuthStatus::Used, 0));
    authorisations::consume(t.state(), a.id, 1, now())
        .await
        .expect_err("a used authorisation takes none");
}

/// A partial approval takes fewer units than asked for; its consumption stays
/// `partially_approved` until the last unit.
#[renox::test]
async fn a_partial_approval_keeps_its_own_state_until_it_is_used() {
    let t = boot().await;
    let policy = policy(&t).await;
    let a = prepared(&t, &policy).await;
    act(&t, a.id, "request", &[("reference", "REQ-2")])
        .await
        .assert_ok();
    let until = day(20).to_string();
    for units in ["0", "3", "4"] {
        act(
            &t,
            a.id,
            "approve-partially",
            &[("units", units), ("valid_until", &until)],
        )
        .await
        .assert_status(422);
    }
    act(
        &t,
        a.id,
        "approve-partially",
        &[("units", "2"), ("valid_until", &until)],
    )
    .await
    .assert_ok();
    let partly = PriorAuthorisation::find_or_404(t.db(), a.id).await.unwrap();
    assert_eq!(partly.status, AuthStatus::PartiallyApproved);
    assert_eq!(partly.units_approved, 2);

    let one = authorisations::consume(t.state(), a.id, 1, now())
        .await
        .unwrap();
    assert_eq!(one.status, AuthStatus::PartiallyApproved);
    let two = authorisations::consume(t.state(), a.id, 1, now())
        .await
        .unwrap();
    assert_eq!(two.status, AuthStatus::Used);
}

/// A denial needs the payer's reason; the patient is told, without the
/// reason, and offered the self-pay option; a denied authorisation covers
/// nothing.
#[renox::test]
async fn a_denial_records_its_reason_and_tells_the_patient_without_it() {
    let t = boot().await;
    let policy = policy(&t).await;
    let a = prepared(&t, &policy).await;
    act(&t, a.id, "request", &[("reference", "REQ-3")])
        .await
        .assert_ok();
    act(&t, a.id, "deny", &[("reason", "  ")])
        .await
        .assert_status(422);
    assert!(t.messages.sent().is_empty());

    act(&t, a.id, "deny", &[("reason", "Not medically necessary")])
        .await
        .assert_ok();
    let denied = PriorAuthorisation::find_or_404(t.db(), a.id).await.unwrap();
    assert_eq!(denied.status, AuthStatus::Denied);
    assert_eq!(
        denied.denial_reason.as_deref(),
        Some("Not medically necessary")
    );

    let sent = t.messages.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].type_key, DENIED_NOTICE);
    assert!(
        sent[0]
            .recipients
            .contains(&NotifyRecipient::Party { id: PATIENT })
    );
    let body = support::Messages::body(&sent[0]);
    assert!(body.contains("pay for the service yourself"), "{body}");
    assert!(!body.contains("medically"), "{body}");
    assert!(
        authorisations::covering(t.db(), PATIENT.0, PAYER.0, "laser", day(1), None)
            .await
            .unwrap()
            .is_empty()
    );
}

/// An approved authorisation expires when its last valid day has passed: not
/// on that day, from the next morning's run.
#[renox::test]
async fn an_authorisation_expires_the_day_after_its_last_valid_day() {
    let t = boot().await;
    let policy = policy(&t).await;
    let a = prepared(&t, &policy).await;
    act(&t, a.id, "request", &[("reference", "REQ-4")])
        .await
        .assert_ok();
    act(&t, a.id, "approve", &[("valid_until", &day(1).to_string())])
        .await
        .assert_ok();

    // The last valid day: still usable.
    let daily = authorisations::run_daily(t.state(), at(day(1), 2))
        .await
        .unwrap();
    assert!(daily.expired.is_empty());
    assert_eq!(status(&t, a.id).await, AuthStatus::Approved);

    // The next morning it ends; a second run changes nothing.
    let daily = authorisations::run_daily(t.state(), at(day(2), 2))
        .await
        .unwrap();
    assert_eq!(daily.expired, [a.id]);
    assert_eq!(status(&t, a.id).await, AuthStatus::Expired);
    let again = authorisations::run_daily(t.state(), at(day(2), 2))
        .await
        .unwrap();
    assert!(again.expired.is_empty());
}

/// A draft or requested authorisation can be cancelled; an approved one
/// cannot. The payer's list of services is kept through the page, once per
/// payer and service.
#[renox::test]
async fn cancel_and_the_payers_list_of_services_that_need_one() {
    let t = boot().await;
    let policy = policy(&t).await;
    let a = prepared(&t, &policy).await;
    act(&t, a.id, "cancel", &[("x", "y")]).await.assert_ok();
    assert_eq!(status(&t, a.id).await, AuthStatus::Cancelled);
    act(&t, a.id, "request", &[("reference", "R")])
        .await
        .assert_status(422);

    let b = prepared(&t, &policy).await;
    act(&t, b.id, "request", &[("reference", "R")])
        .await
        .assert_ok();
    act(&t, b.id, "approve", &[("valid_until", &day(5).to_string())])
        .await
        .assert_ok();
    act(&t, b.id, "cancel", &[("x", "y")])
        .await
        .assert_status(422);

    list(&t, "laser").await.assert_ok();
    list(&t, "laser").await.assert_ok();
    list(&t, "  ").await.assert_status(422);
    let listed = PriorAuthService::of_payer(t.db(), PAYER.0).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(
        PriorAuthService::needed(t.db(), PAYER.0, "laser")
            .await
            .unwrap()
    );
    assert!(
        !PriorAuthService::needed(t.db(), 901, "laser")
            .await
            .unwrap()
    );

    t.app
        .get("/clinics/authorisations")
        .await
        .assert_ok()
        .assert_see("PA-")
        .assert_see("laser");
    t.app
        .get(&format!("/clinics/authorisations/{}", b.id))
        .await
        .assert_ok()
        .assert_see("Approved");
    let rita = t.receptionist().await;
    t.app.acting_as(&rita);
    t.app
        .htmx()
        .post(
            "/clinics/prior-auth-services",
            &[("payer", "900"), ("code", "x")],
        )
        .await
        .assert_forbidden();
    let viewer = t.viewer().await;
    t.app.acting_as(&viewer);
    t.app
        .get("/clinics/authorisations")
        .await
        .assert_forbidden();
}
