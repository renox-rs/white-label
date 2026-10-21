//! The charge capture base (`CLN.ENC.01`, `CLN.ENC.02`, `CLN.INS.01`; base for
//! #353 and #354) and what it closes of earlier items: the patient share
//! recalculated from the contract of a re-split policy (#349), a service not
//! on the self-pay consent refused until an updated consent is signed (#349),
//! and the units of a prior authorisation consumed when the service is
//! charged (#350, #351). Through the real routes of the clinics app and of
//! wl-scheduling, with the fakes of the money side.

mod support;

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_scheduling::ServiceType;

use clinics::billing::{self, Basis};
use clinics::contracts;
use clinics::flags::{AppointmentFlag, FlagCause};
use clinics::{
    AuthStatus, ChargeLine, CoverageOutcome, PriorAuthorisation, SelfPayConsent, appointments,
};
use support::desk::{Desk, PAYER, desk};
use support::{PATIENT, at, day};

const SILVER_PAYER: PartyId = PartyId(901);

/// #349 Done when 2: P1 (Gold, payer 900) is valid but the eligibility is
/// negative; the receptionist re-splits to P2 (Silver, another payer). The
/// bill divides the charge by the contract of P2: 20% copay, not P1's 50%.
#[renox::test]
async fn a_re_split_visit_is_divided_by_the_contract_of_the_second_policy() {
    let d = desk().await;
    d.contract(PAYER, 5_000, 11).await;
    d.contract(SILVER_PAYER, 2_000, 22).await;
    let p1 = d.policy("Gold", PAYER).await;
    let p2 = d.policy("Silver", SILVER_PAYER).await;
    let booking = d.booking(&d.consultation, 9).await;
    appointments::attach_policy(d.t.state(), booking.id, p1.id)
        .await
        .unwrap();
    let appointment = format!("/clinics/appointments/{}", booking.id);
    d.post(
        &d.rita,
        &format!("{appointment}/eligibility"),
        &[("result", "negative")],
    )
    .await
    .assert_ok();
    d.post(
        &d.rita,
        &format!("{appointment}/eligibility"),
        &[("result", "positive"), ("policy", p2.id.to_string().as_str())],
    )
    .await
    .assert_ok();
    d.post(
        &d.rita,
        &format!("{appointment}/outcome"),
        &[
            ("outcome", "re_split"),
            ("policy", p2.id.to_string().as_str()),
        ],
    )
    .await
    .assert_ok();
    let visit = d.check_in(&booking).await;
    assert_eq!(visit.coverage_outcome, Some(CoverageOutcome::ReSplit));
    assert_eq!(visit.coverage_policy_id, Some(p2.id));
    d.start(&booking).await;
    d.deliver(&visit, "consultation", 1).await.assert_ok();

    // The page shows the split before the bill: 20% of AED 450.00.
    d.get(&d.dana, &format!("/clinics/visits/{}", visit.id))
        .await
        .assert_ok()
        .assert_see("AED 90.00")
        .assert_see("AED 360.00");

    d.complete(&booking).await;
    let visit = d.visit(visit.id).await;
    assert!(visit.billed_at.is_some());
    assert_eq!((visit.patient_total, visit.payer_total), (9_000, 36_000));
    let invoices = d.t.books.all();
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].gross(), 9_000);
    assert!(invoices[0].issued, "the system issues the patient's share");
    assert_eq!(invoices[0].draft.party, PATIENT);
    let claims = d.t.claims.all();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].agreement, 22, "the agreement of P2's contract");
    assert_eq!(d.t.claims.claimed(), 36_000);
    assert!(claims[0].void_before_first_submission);
    // The deposit of AED 100.00 settles the patient's AED 90.00 first; the
    // remainder is asked back, never forfeited.
    assert_eq!(visit.deposit_applied, 9_000);
    assert_eq!(d.t.deposits.allocated_total(), 9_000);
    assert_eq!(d.t.refunds.amounts(), [1_000]);

    // Billing again changes nothing.
    billing::bill(d.t.state(), visit.id, Basis::Completed)
        .await
        .unwrap();
    assert_eq!(d.t.books.all().len(), 1);
    assert_eq!(d.t.claims.all().len(), 1);
}

/// The tariff of the contract prices an insured service, the deductible is
/// the patient's first, and an excluded service is the patient's in full.
#[renox::test]
async fn the_tariff_the_deductible_and_an_exclusion_shape_the_split() {
    let d = desk().await;
    let contract = d.contract(PAYER, 0, 33).await;
    contracts::set_tariff(d.t.state(), contract.id, "consultation", Some(30_000), true)
        .await
        .unwrap();
    contracts::set_tariff(d.t.state(), contract.id, "x_ray", None, false)
        .await
        .unwrap();
    d.t.prices.set("x_ray", day(-400), 12_000);
    // A deductible of AED 50.00 a visit.
    clinics::PayerContract::query()
        .where_(clinics::PayerContract::ID.eq(contract.id))
        .update(d.t.db(), &[("deductible", &5_000_i64)])
        .await
        .unwrap();
    let policy = d.policy("Gold", PAYER).await;
    let booking = d.insured(&d.consultation, 9, &policy).await;
    let visit = d.check_in(&booking).await;
    d.start(&booking).await;
    d.deliver(&visit, "consultation", 1).await.assert_ok();
    d.deliver(&visit, "x_ray", 1).await.assert_ok();
    d.complete(&booking).await;

    let visit = d.visit(visit.id).await;
    // Consultation: the payer's AED 300.00 less the AED 50.00 deductible;
    // x-ray: excluded, AED 120.00 of the patient's.
    assert_eq!((visit.patient_total, visit.payer_total), (17_000, 25_000));
    let lines = ChargeLine::active_of(d.t.db(), visit.id).await.unwrap();
    assert_eq!(lines[0].unit_price, 30_000);
    assert_eq!((lines[0].patient_share, lines[0].payer_share), (5_000, 25_000));
    assert_eq!((lines[1].patient_share, lines[1].payer_share), (12_000, 0));
    assert_eq!(d.t.books.all()[0].gross(), 17_000);
    // The deposit of AED 100.00 is all applied: the patient owes AED 70.00.
    assert_eq!(d.t.deposits.allocated_total(), 10_000);
    assert!(d.t.refunds.asked().is_empty());
}

/// An insured visit without a payer contract cannot be charged.
#[renox::test]
async fn an_insured_visit_without_a_contract_refuses_the_charge() {
    let d = desk().await;
    let policy = d.policy("Gold", PAYER).await;
    let booking = d.insured(&d.consultation, 9, &policy).await;
    let visit = d.check_in(&booking).await;
    let res = d.deliver(&visit, "consultation", 1).await;
    res.assert_status(422);
    assert!(res.text().contains("No payer contract"), "{}", res.text());
}

/// #349 Done when 8: on a self-pay visit a service not on the consent is
/// refused by charge capture until the patient signs an updated consent; the
/// first consent is kept, superseded; the bill uses the price list.
#[renox::test]
async fn a_service_not_on_the_self_pay_consent_waits_for_an_updated_one() {
    let d = desk().await;
    d.t.prices.set("x_ray", day(-400), 12_000);
    let policy = d.policy("Gold", PAYER).await;
    let booking = d.booking(&d.consultation, 9).await;
    appointments::attach_policy(d.t.state(), booking.id, policy.id)
        .await
        .unwrap();
    let appointment = format!("/clinics/appointments/{}", booking.id);
    d.post(
        &d.rita,
        &format!("{appointment}/eligibility"),
        &[("result", "negative")],
    )
    .await
    .assert_ok();
    d.post(
        &d.rita,
        &format!("{appointment}/consent"),
        &[("signer_name", "Patient One"), ("services", "")],
    )
    .await
    .assert_ok();
    let visit = d.check_in(&booking).await;
    assert_eq!(visit.coverage_outcome, Some(CoverageOutcome::SelfPay));
    d.start(&booking).await;
    let first = visit.consent_id.expect("the consent moved to the visit");

    // The consultation is on the consent; the x-ray is not.
    d.deliver(&visit, "consultation", 1).await.assert_ok();
    let res = d.deliver(&visit, "x_ray", 1).await;
    res.assert_status(422);
    assert!(
        res.text().contains("not on the self-pay consent"),
        "{}",
        res.text()
    );
    assert_eq!(
        ChargeLine::active_of(d.t.db(), visit.id).await.unwrap().len(),
        1
    );

    // The patient signs again with the x-ray added.
    d.post(
        &d.rita,
        &format!("/clinics/visits/{}/consent", visit.id),
        &[("signer_name", "Patient One"), ("services", "x_ray")],
    )
    .await
    .assert_ok();
    let signed = d.visit(visit.id).await;
    let second = signed.consent_id.unwrap();
    assert_ne!(first, second);
    let consents = SelfPayConsent::of_booking(d.t.db(), booking.id)
        .await
        .unwrap();
    assert!(consents.iter().find(|c| c.id == first).unwrap().is_superseded());
    assert!(!consents.iter().find(|c| c.id == second).unwrap().is_superseded());
    assert_eq!(consents.last().unwrap().codes(), ["consultation", "x_ray"]);
    d.deliver(&visit, "x_ray", 1).await.assert_ok();

    // The bill uses the prices the lines were recorded with, from the price
    // list on the visit day, not the estimate of the consent.
    d.complete(&booking).await;
    let visit = d.visit(visit.id).await;
    assert_eq!((visit.patient_total, visit.payer_total), (57_000, 0));
    assert_eq!(d.t.books.all()[0].gross(), 57_000);
    assert!(d.t.claims.all().is_empty(), "self-pay: no claim");
}

/// A service nobody priced is refused with the name of the service, and a
/// user who may not capture charges is forbidden.
#[renox::test]
async fn an_unpriced_service_and_a_missing_permission_are_refused() {
    let d = desk().await;
    let booking = d.booking(&d.consultation, 9).await;
    let visit = d.check_in(&booking).await;
    let res = d.deliver(&visit, "mystery", 1).await;
    res.assert_status(422);
    assert!(res.text().contains("mystery"), "{}", res.text());
    // The receptionist holds charges.desk, not charges.capture: a service is
    // the practitioner's.
    d.post(
        &d.rita,
        &format!("/clinics/visits/{}/charges", visit.id),
        &[("kind", "service"), ("code", "consultation"), ("units", "1")],
    )
    .await
    .assert_forbidden();
    let viewer = d.t.viewer().await;
    d.post(
        &viewer,
        &format!("/clinics/visits/{}/charges", visit.id),
        &[("kind", "consumable"), ("code", "consultation")],
    )
    .await
    .assert_forbidden();
    // Fewer than one unit.
    d.deliver(&visit, "consultation", 0).await.assert_status(422);
}

/// A self-pay service is paid by the patient's package when it has credit;
/// the session is consumed because the service was recorded as delivered.
#[renox::test]
async fn a_package_pays_a_delivered_self_pay_service() {
    let d = desk().await;
    d.t.packages.give(PATIENT, "consultation", 3);
    let booking = d.booking(&d.consultation, 9).await;
    let visit = d.check_in(&booking).await;
    d.start(&booking).await;
    // Nothing delivered yet: nothing consumed.
    assert!(d.t.packages.used().is_empty());
    d.deliver(&visit, "consultation", 1).await.assert_ok();
    d.complete(&booking).await;

    let used = d.t.packages.used();
    assert_eq!(used.len(), 1);
    assert_eq!((used[0].0.as_str(), used[0].1), ("consultation", 1));
    let line = ChargeLine::active_of(d.t.db(), visit.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(line.package_id, Some(9));
    assert_eq!(line.reason.as_deref(), Some("package"));
    assert!(d.t.books.all().is_empty(), "the package paid: no invoice");
    // Nothing to pay: the deposit is paid back in full.
    assert_eq!(d.t.refunds.amounts(), [10_000]);
}

/// The service that needs a prior authorisation.
async fn procedure(d: &Desk) -> ServiceType {
    let procedure = d.service("procedure", 0).await;
    d.t.prices.set("procedure", day(-400), 120_000);
    clinics::authorisations::require_for(d.t.state(), PAYER, "procedure")
        .await
        .unwrap();
    procedure
}

/// #350, #351 Done when 3: charging the last authorised unit consumes it
/// (`used`), flags the later appointment that relied on it (`CLN.RUL.09`)
/// and the payer bill carries the authorisation's reference.
#[renox::test]
async fn charging_a_service_consumes_the_authorisation() {
    let d = desk().await;
    d.contract(PAYER, 1_000, 5).await;
    let procedure = procedure(&d).await;
    let policy = d.policy("Gold", PAYER).await;
    let a1 = d.t.authorisation(&policy, "procedure", 1, day(30)).await;
    // A later appointment relies on the same unit.
    let later = d.t.book(&procedure, &d.doctor, at(day(5), 9)).await;
    appointments::attach_policy(d.t.state(), later.id, policy.id)
        .await
        .unwrap();
    assert!(
        AppointmentFlag::open_of(d.t.db(), later.id)
            .await
            .unwrap()
            .is_empty()
    );

    let booking = d.insured(&procedure, 9, &policy).await;
    let visit = d.check_in(&booking).await;
    d.start(&booking).await;
    // Checked in, nothing consumed.
    let held = PriorAuthorisation::find_or_404(d.t.db(), a1.id).await.unwrap();
    assert_eq!(held.units_consumed, 0);
    d.deliver(&visit, "procedure", 1).await.assert_ok();
    assert_eq!(
        PriorAuthorisation::find_or_404(d.t.db(), a1.id)
            .await
            .unwrap()
            .units_consumed,
        0,
        "recording a line consumes nothing; the bill does"
    );
    d.complete(&booking).await;

    let a1 = PriorAuthorisation::find_or_404(d.t.db(), a1.id).await.unwrap();
    assert_eq!((a1.status, a1.units_consumed), (AuthStatus::Used, 1));
    let visit = d.visit(visit.id).await;
    assert_eq!((visit.patient_total, visit.payer_total), (12_000, 108_000));
    assert!(visit.authorisations_consumed_at.is_some());
    let flags = AppointmentFlag::open_of(d.t.db(), later.id).await.unwrap();
    assert!(
        flags.iter().any(|f| f.cause == FlagCause::NeedsAuthorisation),
        "the later appointment is flagged: {flags:?}"
    );
    assert_eq!(d.t.claims.all()[0].payer_reference.as_deref(), Some("A-1"));
}

/// #350: units above the units left go self-pay and need the consent; the bill
/// divides the covered units by the contract and charges the rest in full.
#[renox::test]
async fn units_above_the_authorisation_go_self_pay_with_the_consent() {
    let d = desk().await;
    d.contract(PAYER, 1_000, 5).await;
    let procedure = procedure(&d).await;
    let policy = d.policy("Gold", PAYER).await;
    let a1 = d.t.authorisation(&policy, "procedure", 2, day(30)).await;
    let booking = d.insured(&procedure, 9, &policy).await;
    d.post(
        &d.rita,
        &format!("/clinics/appointments/{}/services", booking.id),
        &[("services", "procedure:3")],
    )
    .await
    .assert_ok();
    d.post(
        &d.rita,
        &format!("/clinics/appointments/{}/authorisation-consent", booking.id),
        &[("signer_name", "Patient One")],
    )
    .await
    .assert_ok();
    let visit = d.check_in(&booking).await;
    d.start(&booking).await;

    // Four units would leave two self-pay: the consent covers one.
    let res = d.deliver(&visit, "procedure", 4).await;
    res.assert_status(422);
    assert!(
        res.text().contains("not on the self-pay consent"),
        "{}",
        res.text()
    );
    d.deliver(&visit, "procedure", 3).await.assert_ok();
    d.complete(&booking).await;

    // Two units to the payer (copay 10%), one in full.
    let visit = d.visit(visit.id).await;
    assert_eq!(visit.payer_total, 216_000);
    assert_eq!(visit.patient_total, 24_000 + 120_000);
    let line = ChargeLine::active_of(d.t.db(), visit.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!((line.authorised_units, line.self_pay_units), (2, 1));
    let a1 = PriorAuthorisation::find_or_404(d.t.db(), a1.id).await.unwrap();
    assert_eq!((a1.status, a1.units_consumed), (AuthStatus::Used, 2));
}
