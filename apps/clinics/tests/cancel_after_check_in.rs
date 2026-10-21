//! BL-CLN-05 (#353): the work done is billed when a visit is cancelled after
//! the check-in. A walk-out is billed as a normal visit and the held deposit
//! settles it first, the remainder refunded; when the clinic stops the visit
//! nothing is charged unless the patient accepts the work done, and the
//! deposit is refunded in full. Through the real routes of the clinics app
//! and of wl-scheduling's cancel (`SCH.ACL.05`, `SCH.ACL.06`).

mod support;

use clinics::{ChargeLine, Claim, ClaimLine, ClaimStatus, VisitStatus};
use support::day;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};
use support::desk::{PAYER, desk};
use wl_scheduling::BookingStatus;

/// A clinic with a deposit of AED 200.00 on the consultation and a filler
/// syringe at AED 150.00.
async fn clinic() -> support::desk::Desk {
    let d = desk().await;
    d.deposit(20_000).await;
    d.t.prices.set("filler_syringe", day(-400), 15_000);
    d
}

/// Done when 1: a filler syringe issued (AED 150.00), no service delivered;
/// the patient walks out. The visit bills AED 150.00, the deposit settles it
/// and AED 50.00 is refunded.
#[renox::test]
async fn a_walk_out_bills_the_consumable_and_the_deposit_settles_it() {
    let d = clinic().await;
    let booking = d.booking(&d.consultation, 9).await;
    assert_eq!(booking.deposit_amount, 20_000);
    let visit = d.check_in(&booking).await;
    d.issue(&visit, "filler_syringe", 1).await.assert_ok();

    d.cancel(&booking, "walked_out").await.assert_status(303);

    let done = d.t.booking(booking.id).await;
    assert_eq!(done.status, BookingStatus::Cancelled);
    assert_eq!(done.cancel_initiated_by.as_deref(), Some("party"));
    assert_eq!(done.deposit_applied, 15_000);
    assert_eq!(d.t.refunds.amounts(), [5_000]);
    let visit = d.visit(visit.id).await;
    assert_eq!(visit.status, VisitStatus::Cancelled);
    assert_eq!(visit.cancel_reason.as_deref(), Some("walked_out"));
    assert_eq!((visit.patient_total, visit.payer_total), (15_000, 0));
    let invoices = d.t.books.all();
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].gross(), 15_000);
    assert!(invoices[0].issued);
    assert_eq!(
        d.t.deposits.allocated_total(),
        15_000,
        "settled by the deposit"
    );
    assert!(d.t.claims.all().is_empty());
}

/// Done when 2: the practitioner marked the consultation (AED 300.00)
/// delivered; a walk-out bills AED 300.00, the deposit AED 200.00 settles
/// part and AED 100.00 is due from the patient. Nothing is forfeited.
#[renox::test]
async fn a_delivered_service_is_billed_and_the_balance_is_due() {
    let d = clinic().await;
    d.t.prices.set("consultation", day(-300), 30_000);
    let booking = d.booking(&d.consultation, 9).await;
    let visit = d.check_in(&booking).await;
    d.deliver(&visit, "consultation", 1).await.assert_ok();

    d.cancel(&booking, "walked_out").await.assert_status(303);

    let invoice = &d.t.books.all()[0];
    assert_eq!(invoice.gross(), 30_000);
    assert_eq!(d.t.deposits.allocated_total(), 20_000);
    // What is due is read from the issued invoice itself.
    let issued = invoicer(d.t.state())
        .find(d.t.db(), InvoiceId(invoice.id))
        .await
        .unwrap()
        .expect("the invoice exists");
    assert_eq!(issued.status, InvoiceStatus::Issued);
    assert_eq!(issued.gross_total, 30_000);
    assert_eq!(issued.balance, 10_000, "AED 100.00 is due from the patient");
    assert_eq!(d.visit(visit.id).await.invoice_id, Some(invoice.id));
    assert!(d.t.refunds.asked().is_empty(), "no refund, no forfeit");
    let done = d.t.booking(booking.id).await;
    assert_eq!(done.deposit_forfeited, 0);
    assert_eq!(
        done.deposit_outcome.as_deref(),
        Some("applied_to_work_done")
    );
}

/// Done when 3: an `in_progress` visit stopped for `equipment_failure` with a
/// consumable issued and no acceptance: nothing is charged, the consumable
/// shows price 0 with the reason, AED 200.00 is refunded, and neither the
/// patient nor the payer is billed.
#[renox::test]
async fn a_clinic_stop_without_acceptance_charges_nothing() {
    let d = clinic().await;
    let booking = d.booking(&d.consultation, 9).await;
    let visit = d.check_in(&booking).await;
    d.start(&booking).await;
    d.issue(&visit, "filler_syringe", 1).await.assert_ok();

    d.cancel(&booking, "equipment_failure")
        .await
        .assert_status(303);

    let done = d.t.booking(booking.id).await;
    assert_eq!(done.cancel_initiated_by.as_deref(), Some("business"));
    assert_eq!(d.t.refunds.amounts(), [20_000]);
    let visit = d.visit(visit.id).await;
    assert_eq!(visit.status, VisitStatus::Cancelled);
    assert!(visit.clinic_stop);
    assert_eq!((visit.patient_total, visit.payer_total), (0, 0));
    let lines = ChargeLine::active_of(d.t.db(), visit.id).await.unwrap();
    assert_eq!(lines.len(), 1, "the consumable is kept for stock and cost");
    assert_eq!(lines[0].unit_price, 0);
    assert_eq!(lines[0].list_price, 15_000);
    assert_eq!(lines[0].reason.as_deref(), Some("clinic_stop"));
    assert!(d.t.books.all().is_empty());
    assert!(d.t.claims.all().is_empty());
}

/// Done when 4: with the patient's acceptance recorded, the same stop bills
/// the consumable and still refunds the deposit in full.
#[renox::test]
async fn a_clinic_stop_the_patient_accepts_is_billed() {
    let d = clinic().await;
    let booking = d.booking(&d.consultation, 9).await;
    let visit = d.check_in(&booking).await;
    d.start(&booking).await;
    d.issue(&visit, "filler_syringe", 1).await.assert_ok();
    d.post(
        &d.rita,
        &format!("/clinics/visits/{}/accept-work", visit.id),
        &[("x", "y")],
    )
    .await
    .assert_ok();
    let accepted = d.visit(visit.id).await;
    assert_eq!(accepted.work_accepted_by, Some(d.rita.id));
    assert!(accepted.work_accepted_at.is_some());

    d.cancel(&booking, "equipment_failure")
        .await
        .assert_status(303);

    assert_eq!(
        d.t.refunds.amounts(),
        [20_000],
        "the deposit is refunded in full"
    );
    let invoice = &d.t.books.all()[0];
    assert_eq!(invoice.gross(), 15_000);
    assert_eq!(
        d.t.deposits.allocated_total(),
        0,
        "the deposit is not applied"
    );
    let visit = d.visit(visit.id).await;
    assert_eq!(visit.patient_total, 15_000);
    let lines = ChargeLine::active_of(d.t.db(), visit.id).await.unwrap();
    assert_eq!(lines[0].unit_price, 15_000);
    assert_eq!(lines[0].reason, None);
}

/// Done when 5: an insured walk-out with a delivered consultation creates a
/// draft claim for the payer share; the deposit settles the patient share.
#[renox::test]
async fn an_insured_walk_out_creates_a_draft_claim_for_the_payer_share() {
    let d = clinic().await;
    d.t.prices.set("consultation", day(-300), 30_000);
    d.contract(PAYER, 2_000, 77).await;
    let policy = d.policy("Gold", PAYER).await;
    let booking = d.insured(&d.consultation, 9, &policy).await;
    let visit = d.check_in(&booking).await;
    d.deliver(&visit, "consultation", 1).await.assert_ok();

    d.cancel(&booking, "walked_out").await.assert_status(303);

    let visit = d.visit(visit.id).await;
    assert_eq!((visit.patient_total, visit.payer_total), (6_000, 24_000));
    let claims = d.t.claims.all();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].agreement, 77);
    assert_eq!(d.t.claims.claimed(), 24_000);
    assert!(visit.payer_bill_id.is_some());
    // The clinic's own claim record around the payer bill: a draft for the
    // payer share, with its line.
    let made = Claim::of_visit(d.t.db(), visit.id).await.unwrap();
    assert_eq!(made.len(), 1);
    let claim = &made[0];
    assert_eq!(claim.status, ClaimStatus::Draft);
    assert_eq!(Some(claim.payer_bill_id), visit.payer_bill_id);
    assert_eq!(claim.payer_party_id, PAYER.0);
    assert_eq!(claim.claimed, 24_000);
    assert_eq!(claim.visit_date, visit.visit_date);
    let lines = ClaimLine::of_claim(d.t.db(), claim.id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].code, "consultation");
    assert_eq!(lines[0].claimed, 24_000);
    // The deposit applies to the patient share only; the rest goes back.
    assert_eq!(d.t.books.all()[0].gross(), 6_000);
    assert_eq!(d.t.deposits.allocated_total(), 6_000);
    assert_eq!(d.t.refunds.amounts(), [14_000]);
}

/// Done when 6: an insured visit stopped for `equipment_failure` with a
/// delivered consultation and no acceptance creates no claim and no patient
/// charge; with acceptance a draft claim is created for the payer share.
#[renox::test]
async fn an_insured_clinic_stop_claims_only_after_the_patient_accepts() {
    let d = clinic().await;
    d.t.prices.set("consultation", day(-300), 30_000);
    d.contract(PAYER, 2_000, 77).await;
    let policy = d.policy("Gold", PAYER).await;

    let refused = d.insured(&d.consultation, 9, &policy).await;
    let visit = d.check_in(&refused).await;
    d.deliver(&visit, "consultation", 1).await.assert_ok();
    d.cancel(&refused, "equipment_failure")
        .await
        .assert_status(303);
    assert!(d.t.claims.all().is_empty(), "no acceptance, no claim");
    assert!(d.t.books.all().is_empty(), "and no patient charge");
    assert_eq!(d.visit(visit.id).await.payer_bill_id, None);

    let accepted = d.insured(&d.consultation, 11, &policy).await;
    let visit = d.check_in(&accepted).await;
    d.deliver(&visit, "consultation", 1).await.assert_ok();
    d.post(
        &d.rita,
        &format!("/clinics/visits/{}/accept-work", visit.id),
        &[("x", "y")],
    )
    .await
    .assert_ok();
    d.cancel(&accepted, "equipment_failure")
        .await
        .assert_status(303);
    assert_eq!(d.t.claims.all().len(), 1);
    assert_eq!(d.t.claims.claimed(), 24_000);
    assert_eq!(d.t.books.all()[0].gross(), 6_000);
    assert_eq!(d.visit(visit.id).await.payer_total, 24_000);
}

/// Done when 7: setting *none* while a consumable was issued, or a service
/// marked, is refused; until a line or *none* the booking cannot be
/// cancelled; once *none* is set, it can, and nothing is billed.
#[renox::test]
async fn none_is_refused_while_work_was_recorded() {
    let d = clinic().await;
    let booking = d.booking(&d.consultation, 9).await;
    let visit = d.check_in(&booking).await;
    let none = format!("/clinics/visits/{}/none", visit.id);

    // Nothing recorded yet: wl-scheduling will not cancel.
    let res = d.cancel(&booking, "walked_out").await;
    res.assert_status(422);
    assert!(
        res.text().contains("Record the work done first"),
        "{}",
        res.text()
    );
    assert_eq!(
        d.t.booking(booking.id).await.status,
        BookingStatus::CheckedIn
    );

    d.issue(&visit, "filler_syringe", 1).await.assert_ok();
    let res = d.post(&d.rita, &none, &[("x", "y")]).await;
    res.assert_status(422);
    assert!(res.text().contains("cannot be none"), "{}", res.text());

    // The same for a service the practitioner marked delivered.
    let other = d.booking(&d.consultation, 11).await;
    let second = d.check_in(&other).await;
    d.deliver(&second, "consultation", 1).await.assert_ok();
    d.post(
        &d.rita,
        &format!("/clinics/visits/{}/none", second.id),
        &[("x", "y")],
    )
    .await
    .assert_status(422);

    // A visit with nothing done: none, then the cancel goes through and the
    // deposit is refunded without a bill.
    let third = d.booking(&d.consultation, 13).await;
    let empty = d.check_in(&third).await;
    d.post(
        &d.rita,
        &format!("/clinics/visits/{}/none", empty.id),
        &[("x", "y")],
    )
    .await
    .assert_ok();
    assert!(d.visit(empty.id).await.work_none);
    d.cancel(&third, "walked_out").await.assert_status(303);
    assert_eq!(d.t.refunds.amounts(), [20_000]);
    assert!(d.t.books.all().is_empty());
}

/// A second cancel of the same booking is refused and the work done is not
/// billed twice.
#[renox::test]
async fn a_second_cancel_bills_nothing_more() {
    let d = clinic().await;
    let booking = d.booking(&d.consultation, 9).await;
    let visit = d.check_in(&booking).await;
    d.issue(&visit, "filler_syringe", 1).await.assert_ok();
    d.cancel(&booking, "walked_out").await.assert_status(303);

    let again = d.cancel(&booking, "walked_out").await;
    assert!(again.status.is_client_error(), "{}", again.status);
    assert_eq!(d.t.books.all().len(), 1);
    assert_eq!(d.t.refunds.amounts(), [5_000]);
}

/// A package session is consumed only when the package's service is recorded
/// as delivered: a consumable alone consumes none, a delivered service does.
#[renox::test]
async fn a_package_session_follows_the_service_delivered() {
    let d = clinic().await;
    d.t.packages.give(support::PATIENT, "consultation", 2);
    let first = d.booking(&d.consultation, 9).await;
    let visit = d.check_in(&first).await;
    d.issue(&visit, "filler_syringe", 1).await.assert_ok();
    d.cancel(&first, "walked_out").await.assert_status(303);
    assert!(
        d.t.packages.used().is_empty(),
        "a consumable uses no session"
    );

    let second = d.booking(&d.consultation, 11).await;
    let visit = d.check_in(&second).await;
    d.deliver(&visit, "consultation", 1).await.assert_ok();
    d.cancel(&second, "walked_out").await.assert_status(303);
    assert_eq!(d.t.packages.used().len(), 1);
    // The package paid the second visit: wl-scheduling applied the deposit to
    // the work, the bill had nothing to use it for and paid it back.
    assert_eq!(d.t.refunds.amounts(), [5_000, 20_000]);
    assert_eq!(d.t.books.all().len(), 1, "only the consumable was invoiced");
}

/// The visit page shows the work done and what the bill came to.
#[renox::test]
async fn the_visit_page_shows_the_work_done_and_the_bill() {
    let d = clinic().await;
    let booking = d.booking(&d.consultation, 9).await;
    let visit = d.check_in(&booking).await;
    let path = format!("/clinics/visits/{}", visit.id);
    d.get(&d.rita, &path)
        .await
        .assert_ok()
        .assert_see("Not recorded yet");
    d.issue(&visit, "filler_syringe", 1).await.assert_ok();
    d.get(&d.rita, &path)
        .await
        .assert_ok()
        .assert_see("filler_syringe")
        .assert_see("AED 150.00");
    d.cancel(&booking, "walked_out").await.assert_status(303);
    d.get(&d.rita, &path)
        .await
        .assert_ok()
        .assert_see("AED 150.00");
}

/// The decision that the clinic stopped the visit is stored with the
/// cancellation, before any bill starts: when the first attempt to bill never
/// got as far as its own first write (here the process is gone right after
/// the visit's cancel transition), billing it again from the visit page is
/// still a clinic stop that charges nothing, not a walk-out billed as a
/// normal visit.
#[renox::test]
async fn a_clinic_stop_is_still_a_clinic_stop_when_the_first_attempt_to_bill_died() {
    use clinics::visit_machine::{CANCEL, VisitInput};
    use renox::db::now;
    use renox::state_machine::StateMachine;

    let d = clinic().await;
    let booking = d.booking(&d.consultation, 9).await;
    let visit = d.check_in(&booking).await;
    d.start(&booking).await;
    d.issue(&visit, "filler_syringe", 1).await.assert_ok();

    // What the listener of `BookingCancelled` runs first; its bill never starts.
    let open = d.visit(visit.id).await;
    let _ = open
        .transition(CANCEL)
        .by_system()
        .input(&VisitInput {
            at: Some(now()),
            reason: Some("equipment_failure".to_owned()),
            clinic_stop: true,
            deposit_applied: 0,
        })
        .run(d.t.state())
        .await
        .expect("the visit is cancelled");
    let stopped = d.visit(visit.id).await;
    assert_eq!(stopped.status, VisitStatus::Cancelled);
    assert!(stopped.billed_at.is_none(), "the first attempt never billed");
    assert!(
        stopped.clinic_stop,
        "who stopped it is stored with the cancellation"
    );

    d.post(
        &d.rita,
        &format!("/clinics/visits/{}/bill", visit.id),
        &[("x", "y")],
    )
    .await
    .assert_ok();

    let billed = d.visit(visit.id).await;
    assert!(billed.billed_at.is_some());
    assert_eq!((billed.patient_total, billed.payer_total), (0, 0));
    assert!(d.t.books.all().is_empty(), "a clinic stop charges nothing");
    let lines = ChargeLine::active_of(d.t.db(), visit.id).await.unwrap();
    assert_eq!(lines[0].unit_price, 0);
    assert_eq!(lines[0].reason.as_deref(), Some("clinic_stop"));
}
