//! BL-CLN-06 (#354): the practitioner decides the planned procedure is not
//! suitable. The appointment completes with the consultation in charge
//! capture, the procedure is not charged, and the deposit is applied to the
//! consultation's patient share with the rest refunded (or all of it
//! refunded when the front desk chooses). Through the real routes.

mod support;

use renox::prelude::Model;
use clinics::{AuthStatus, ChargeLine, PriorAuthorisation, VisitStatus};
use support::desk::{Desk, PAYER, desk};
use support::{PATIENT, day};
use wl_scheduling::{Booking, BookingStatus, ServiceType};

/// A clinic whose laser procedure (AED 1,200.00) takes a deposit of
/// AED 300.00 and whose consultation costs AED 250.00.
async fn clinic() -> (Desk, ServiceType) {
    let d = desk().await;
    let laser = d.service("laser", 30_000).await;
    d.t.prices.set("laser", day(-400), 120_000);
    d.t.prices.set("consultation", day(-300), 25_000);
    (d, laser)
}

fn not_suitable(visit: i64) -> String {
    format!("/clinics/visits/{visit}/not-suitable")
}

/// Done when 1 and 3: the booked laser procedure, the practitioner records
/// "not suitable" and the consultation (self-pay): the visit bills AED 250.00,
/// the deposit covers it and AED 50.00 is refunded. The appointment is
/// `completed`, not `cancelled`.
#[renox::test]
async fn the_consultation_is_billed_and_the_deposit_covers_it() {
    let (d, laser) = clinic().await;
    let booking = d.booking(&laser, 9).await;
    assert_eq!(booking.deposit_amount, 30_000);
    let visit = d.check_in(&booking).await;
    d.start(&booking).await;
    // The laser was entered before the decision.
    d.deliver(&visit, "laser", 1).await.assert_ok();

    d.post(&d.dana, &not_suitable(visit.id), &[("consultation", "consultation")])
        .await
        .assert_ok();
    let lines = ChargeLine::active_of(d.t.db(), visit.id).await.unwrap();
    assert_eq!(lines.len(), 1, "the procedure line is taken out");
    assert_eq!(lines[0].code, "consultation");
    let all = ChargeLine::of_visit(d.t.db(), visit.id).await.unwrap();
    assert_eq!(all[0].removed_reason.as_deref(), Some("not_suitable"));
    assert!(d.visit(visit.id).await.not_suitable);

    d.complete(&booking).await;

    let done = d.t.booking(booking.id).await;
    assert_eq!(done.status, BookingStatus::Completed);
    assert!(d.t.events.cancelled.all().is_empty(), "not a cancellation");
    assert_eq!(d.t.events.completed.all().len(), 1);
    let visit = d.visit(visit.id).await;
    assert_eq!(visit.status, VisitStatus::Completed);
    assert_eq!(visit.patient_total, 25_000);
    let invoices = d.t.books.all();
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].gross(), 25_000);
    assert_eq!(d.t.deposits.allocated_total(), 25_000, "the deposit covers it");
    assert_eq!(d.t.refunds.amounts(), [5_000], "and AED 50.00 goes back");
    assert_eq!(done.deposit_forfeited, 0, "never forfeited");
}

/// Done when 2: "not suitable" without a consultation service is refused,
/// and so is the planned procedure itself as the consultation; nothing
/// changed.
#[renox::test]
async fn a_consultation_service_must_be_picked() {
    let (d, laser) = clinic().await;
    let booking = d.booking(&laser, 9).await;
    let visit = d.check_in(&booking).await;
    d.deliver(&visit, "laser", 1).await.assert_ok();

    for consultation in ["", "laser"] {
        let res = d
            .post(&d.dana, &not_suitable(visit.id), &[("consultation", consultation)])
            .await;
        res.assert_status(422);
        assert!(
            res.text().contains("Pick the consultation service"),
            "{}",
            res.text()
        );
    }
    let lines = ChargeLine::active_of(d.t.db(), visit.id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].code, "laser", "the procedure line stays");
    assert!(!d.visit(visit.id).await.not_suitable);
}

/// The receptionist may refund the deposit in full instead of applying it:
/// the patient pays the consultation, AED 300.00 goes back, nothing is kept.
#[renox::test]
async fn the_desk_may_refund_the_deposit_in_full() {
    let (d, laser) = clinic().await;
    let booking = d.booking(&laser, 9).await;
    let visit = d.check_in(&booking).await;
    d.start(&booking).await;
    d.post(&d.dana, &not_suitable(visit.id), &[("consultation", "consultation")])
        .await
        .assert_ok();
    d.post(
        &d.rita,
        &format!("/clinics/visits/{}/refund-deposit", visit.id),
        &[("x", "y")],
    )
    .await
    .assert_ok();
    assert!(d.visit(visit.id).await.deposit_refund_full);

    d.complete(&booking).await;

    assert_eq!(d.t.books.all()[0].gross(), 25_000);
    assert_eq!(d.t.deposits.allocated_total(), 0);
    assert_eq!(d.t.refunds.amounts(), [30_000]);
    assert_eq!(d.t.booking(booking.id).await.deposit_forfeited, 0);
}

/// An insured visit: the consultation is covered by the payer (no copay), so
/// the deposit goes back in full and a draft claim is made for the
/// consultation; no unit of the laser authorisation is consumed, and the
/// units held for the laser are given back.
#[renox::test]
async fn no_authorisation_unit_is_consumed_for_the_procedure() {
    let (d, laser) = clinic().await;
    d.contract(PAYER, 0, 55).await;
    clinics::authorisations::require_for(d.t.state(), PAYER, "laser")
        .await
        .unwrap();
    let policy = d.policy("Gold", PAYER).await;
    let a1 = d.t.authorisation(&policy, "laser", 1, day(30)).await;
    let booking = d.insured(&laser, 9, &policy).await;
    let visit = d.check_in(&booking).await;
    d.start(&booking).await;
    assert!(
        !clinics::AuthorisationHold::of_visit(d.t.db(), visit.id)
            .await
            .unwrap()
            .is_empty(),
        "checked in, the laser holds its unit"
    );
    d.deliver(&visit, "laser", 1).await.assert_ok();
    d.post(&d.dana, &not_suitable(visit.id), &[("consultation", "consultation")])
        .await
        .assert_ok();

    d.complete(&booking).await;

    let a1 = PriorAuthorisation::find_or_404(d.t.db(), a1.id).await.unwrap();
    assert_eq!((a1.status, a1.units_consumed), (AuthStatus::Approved, 0));
    let held = clinics::AuthorisationHold::of_visit(d.t.db(), visit.id)
        .await
        .unwrap();
    assert!(held.iter().all(|h| h.released_on.is_some()), "given back");
    let visit = d.visit(visit.id).await;
    assert_eq!((visit.patient_total, visit.payer_total), (0, 25_000));
    assert_eq!(d.t.claims.claimed(), 25_000, "the claim is the consultation");
    assert!(d.t.books.all().is_empty(), "the patient owes nothing");
    assert_eq!(d.t.refunds.amounts(), [30_000], "the whole deposit goes back");
}

/// A self-pay patient with a package for the laser: no session is consumed
/// for the procedure that was not performed.
#[renox::test]
async fn no_package_session_is_consumed_for_the_procedure() {
    let (d, laser) = clinic().await;
    d.t.packages.give(PATIENT, "laser", 3);
    let booking = d.booking(&laser, 9).await;
    let visit = d.check_in(&booking).await;
    d.start(&booking).await;
    d.deliver(&visit, "laser", 1).await.assert_ok();
    d.post(&d.dana, &not_suitable(visit.id), &[("consultation", "consultation")])
        .await
        .assert_ok();
    d.complete(&booking).await;
    assert!(d.t.packages.used().is_empty(), "{:?}", d.t.packages.used());
    assert_eq!(d.t.books.all()[0].gross(), 25_000);
}

/// Only the practitioner records it; and not once the visit is over.
#[renox::test]
async fn only_the_practitioner_records_it_and_only_while_the_visit_is_open() {
    let (d, laser) = clinic().await;
    let booking: Booking = d.booking(&laser, 9).await;
    let visit = d.check_in(&booking).await;
    d.start(&booking).await;
    d.post(&d.rita, &not_suitable(visit.id), &[("consultation", "consultation")])
        .await
        .assert_forbidden();
    d.post(&d.dana, &not_suitable(visit.id), &[("consultation", "consultation")])
        .await
        .assert_ok();
    d.complete(&booking).await;
    let res = d
        .post(&d.dana, &not_suitable(visit.id), &[("consultation", "consultation")])
        .await;
    res.assert_status(422);
    assert!(res.text().contains("already billed"), "{}", res.text());
}
