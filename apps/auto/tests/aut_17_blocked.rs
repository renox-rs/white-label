//! BL-AUT-17 (#413): react to a blocked party (08 §7.1 settings *Party blocked*,
//! wl-parties §5.6). A blocked fleet gives the owner one review task listing its
//! contracts (*suspend*, *terminate*, *continue*) and changes nothing by itself;
//! a blocked customer gives the branch manager a review of the open job cards
//! (*finish safety lines only*, *continue*), new estimate approvals are refused,
//! and the lines not yet started that are not safety work are taken off with the
//! reason *party blocked* and are not billed; a blocked insurer's claims
//! continue. The review tasks themselves are wl-parties' (the host tests run
//! them); here the agreement kinds and their sources are called as that module
//! calls them.

mod support;

use std::time::Duration;

use auto::blocked::{
    CLAIM_KIND, FLEET_KIND, JOB_CARD_KIND, LineRef, PARTY_BLOCKED, finish_safety_lines_only,
    mark_safety,
};
use auto::fleet_daily::run_daily_on;
use auto::fleet_end::NOTICE_TYPE;
use auto::fleet_invoice::{FleetInvoice, KIND_FINAL};
use auto::job_cards::{self, Delivery, NewJobCard, NewLabour, NewPart, Promise};
use auto::{
    FleetContract, FleetService, FleetStatus, JobCard, JobCardStatus, LabourLine, LineStatus,
    PartLine, WorkStatus,
};
use renox::auth::permissions::Scope;
use renox::prelude::*;
use support::fleetfx::{FLEET, active_contract, bay_and_service, book_plan, omar};
use support::{Garage, at, boot, day};
use wl_kernel::block_effects::{BlockEffect, Decision, effect_for, source_for};
use wl_kernel::documents::today;
use wl_kernel::{PartyId, Subject};
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::{Booking, BookingStatus};

/// Mona Hassan, the owner of the Hilux.
const CUSTOMER: i64 = 7;

/// The workshop of the bookings.
fn workshop() -> Subject {
    support::fleetfx::workshop()
}

/// Each agreement kind is registered with its effect and the role that decides:
/// the fleet contract and the job card are reviewed (the owner, the branch
/// manager), the insurance claim of a blocked insurer continues.
#[renox::test]
async fn aut_17_the_agreement_kinds_are_registered_with_their_block_effect() {
    let _t = boot().await;
    let global = Scope::global();
    let fleet = effect_for(FLEET_KIND, &global);
    assert_eq!(fleet.effect, BlockEffect::Review);
    assert_eq!(fleet.decider_role.as_deref(), Some("owner"));
    let card = effect_for(JOB_CARD_KIND, &global);
    assert_eq!(card.effect, BlockEffect::Review);
    assert_eq!(card.decider_role.as_deref(), Some("branch_manager"));
    let claim = effect_for(CLAIM_KIND, &global);
    assert_eq!(claim.effect, BlockEffect::Continue, "money comes in");
    assert!(source_for(FLEET_KIND).is_some());
    assert!(source_for(JOB_CARD_KIND).is_some());
    assert!(
        source_for(CLAIM_KIND).is_none(),
        "no review task for a claim: it continues"
    );
}

/// Given fleet F blocked with contract FC `active`, the owner's review lists
/// the contract and nothing changes by itself; *continue* leaves it `active`,
/// *suspend* suspends it with the neutral reason *on hold*.
#[renox::test]
async fn aut_17_a_blocked_fleet_is_reviewed_by_the_owner_and_nothing_changes_by_itself() {
    let t = boot().await;
    let (contract, _van, _plan) = active_contract(&t, 50_100).await;
    let owner = omar(&t).await;
    t.people.set_blocked(FLEET, true);
    let source = source_for(FLEET_KIND).expect("a source");

    let open = source
        .open_for(t.state(), PartyId(FLEET))
        .await
        .unwrap();
    assert_eq!(open.len(), 1, "one review item per contract");
    assert_eq!(open[0].subject, contract.subject());
    assert!(open[0].label.contains(&contract.number()), "{}", open[0].label);
    assert!(open[0].scope.is_none(), "one task for the owner");
    assert_eq!(
        FleetContract::find_or_404(t.db(), contract.id)
            .await
            .unwrap()
            .status,
        FleetStatus::Active,
        "blocking changes nothing by itself"
    );

    source
        .decide(t.state(), owner.id, &contract.subject(), Decision::Continue)
        .await
        .unwrap();
    assert_eq!(
        FleetContract::find_or_404(t.db(), contract.id)
            .await
            .unwrap()
            .status,
        FleetStatus::Active
    );

    source
        .decide(t.state(), owner.id, &contract.subject(), Decision::Pause)
        .await
        .unwrap();
    let suspended = FleetContract::find_or_404(t.db(), contract.id).await.unwrap();
    assert_eq!(suspended.status, FleetStatus::Suspended);
    assert_eq!(suspended.suspended_reason.as_deref(), Some("On hold"));
    // A suspended contract is still listed, and *suspend* again changes nothing.
    assert_eq!(
        source.open_for(t.state(), PartyId(FLEET)).await.unwrap().len(),
        1
    );
    source
        .decide(t.state(), owner.id, &contract.subject(), Decision::Pause)
        .await
        .unwrap();
}

/// *Terminate*, the owner's decision, ends the contract at once with its last
/// day of service today: the bookings after today are cancelled now, the last
/// invoice, the fee and the notice follow the first run after today.
#[renox::test]
async fn aut_17_the_owner_terminates_the_contract_of_a_blocked_fleet() {
    let t = boot().await;
    let (bay, service_type) = bay_and_service(&t).await;
    let (contract, van, plan) = active_contract(&t, 50_100).await;
    let owner = omar(&t).await;
    let sami = t.advisor().await;
    let booking = service::book(
        t.state(),
        NewBooking::new(PartyId(FLEET), service_type.id, &[bay.id], at(day(4), 10, 0))
            .for_subject(van.subject())
            .at(workshop()),
        Actor::staff(sami.id),
        renox::db::now(),
    )
    .await
    .unwrap();
    book_plan(&t, plan.id, booking.id, day(4)).await;
    t.people.set_blocked(FLEET, true);

    source_for(FLEET_KIND)
        .unwrap()
        .decide(t.state(), owner.id, &contract.subject(), Decision::End)
        .await
        .unwrap();
    let ended = FleetContract::find_or_404(t.db(), contract.id).await.unwrap();
    assert_eq!(ended.status, FleetStatus::Ended);
    assert_eq!(ended.termination_reason.as_deref(), Some("On hold"));
    assert_eq!(ended.termination_approved_by, Some(owner.id));
    assert_eq!(ended.effective_date, Some(today()));
    assert_eq!(
        Booking::find_or_404(t.db(), booking.id).await.unwrap().status,
        BookingStatus::Cancelled
    );
    assert!(
        !FleetService::find_or_404(t.db(), plan.id)
            .await
            .unwrap()
            .active
    );
    assert!(ended.wound_down_at.is_none(), "the last day is not over");
    assert!(t.books.about(&contract.subject()).is_empty());

    // The first run after the last day finishes it: last invoice, fee, notice.
    t.app.travel(Duration::from_secs(86_400));
    let run = t
        .app
        .at_travelled_time(async { run_daily_on(t.state(), today()).await })
        .await
        .unwrap();
    assert_eq!(run.wound_down, [contract.id], "{run:?}");
    let done = FleetContract::find_or_404(t.db(), contract.id).await.unwrap();
    assert!(done.wound_down_at.is_some());
    assert!(done.termination_fee_invoice_id.is_some());
    assert_eq!(t.messages.of_type(NOTICE_TYPE).len(), 1);
    let periods = FleetInvoice::of_contract(t.db(), contract.id).await.unwrap();
    assert!(periods.iter().any(|p| p.kind == KIND_FINAL));
    assert_eq!(t.terminated.all().len(), 1);
}

/// A job card with lines: brake pads (safety work, not started), paint
/// touch-up (not started), an oil change (done) and two parts, approved by the
/// customer, in progress.
struct Job {
    card: JobCard,
    brakes: LabourLine,
    paint: LabourLine,
    oil: LabourLine,
    brake_set: PartLine,
    paint_can: PartLine,
}

async fn blocked_job(t: &Garage) -> Job {
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let tareq_user = t.technician_user("Tareq").await;
    let tareq = t.technician("Tareq").await;
    let card = job_cards::check_in(
        t.state(),
        &sami,
        NewJobCard::new(vehicle.id, "Brakes and body")
            .odometer(52_100)
            .condition("Scratches")
            .acknowledged_by("Mona Hassan")
            .technician(tareq_user.id),
    )
    .await
    .unwrap();
    job_cards::start_inspection(t.state(), &tareq, card.id)
        .await
        .unwrap();
    let brakes = job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Brake pads", 1_000, 20_000),
    )
    .await
    .unwrap();
    let paint = job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Paint touch-up", 1_000, 10_000),
    )
    .await
    .unwrap();
    let oil = job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Oil change", 1_000, 8_000),
    )
    .await
    .unwrap();
    let brake_set = job_cards::add_part(
        t.state(),
        &tareq,
        card.id,
        NewPart::new("Brake pad set", 1_000, 15_000),
    )
    .await
    .unwrap();
    let paint_can = job_cards::add_part(
        t.state(),
        &tareq,
        card.id,
        NewPart::new("Touch-up paint", 1_000, 3_000),
    )
    .await
    .unwrap();
    mark_safety(t.state(), &sami, card.id, LineRef::Labour(brakes.id), true)
        .await
        .unwrap();
    mark_safety(t.state(), &sami, card.id, LineRef::Part(brake_set.id), true)
        .await
        .unwrap();
    job_cards::send_estimate(t.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();
    job_cards::approve_recorded(t.state(), &sami, card.id, "Mona Hassan", "recorded", None)
        .await
        .unwrap();
    job_cards::record_work(t.state(), &tareq, oil.id, WorkStatus::Done, None)
        .await
        .unwrap();
    Job {
        card: t.reload(card.id).await,
        brakes,
        paint,
        oil,
        brake_set,
        paint_can,
    }
}

/// Given customer C blocked with JC `in_progress`, new estimate approvals are
/// refused in words that do not say why (the customer's link says to contact the
/// workshop) and are possible again after the unblock.
#[renox::test]
async fn aut_17_new_estimate_approvals_are_refused_while_the_customer_is_blocked() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let tareq_user = t.technician_user("Tareq").await;
    let tareq = t.technician("Tareq").await;
    let card = job_cards::check_in(
        t.state(),
        &sami,
        NewJobCard::new(vehicle.id, "Brake noise")
            .odometer(52_100)
            .condition("Clean")
            .acknowledged_by("Mona Hassan")
            .technician(tareq_user.id),
    )
    .await
    .unwrap();
    job_cards::start_inspection(t.state(), &tareq, card.id)
        .await
        .unwrap();
    job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Brake pads", 1_000, 20_000),
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

    t.people.set_blocked(CUSTOMER, true);
    let desk = job_cards::approve_recorded(
        t.state(),
        &sami,
        card.id,
        "Mona Hassan",
        "recorded",
        None,
    )
    .await
    .unwrap_err();
    assert!(desk.to_string().contains("on hold"), "{desk}");
    let link = job_cards::approve_by_link(t.state(), &round.token, "Mona Hassan", None, true)
        .await
        .unwrap_err();
    assert!(link.to_string().contains("contact the workshop"), "{link}");
    assert!(!link.to_string().contains("hold"), "the customer is not told");
    assert_eq!(
        t.reload(card.id).await.status,
        JobCardStatus::AwaitingApproval
    );

    t.people.set_blocked(CUSTOMER, false);
    let approved = job_cards::approve_recorded(
        t.state(),
        &sami,
        card.id,
        "Mona Hassan",
        "recorded",
        None,
    )
    .await
    .unwrap();
    assert_eq!(approved.status, JobCardStatus::InProgress);
}

/// Given JC with "brake pads" (safety work, not started) and "paint touch-up"
/// (not started), when the branch manager chooses *finish safety lines only*,
/// "paint touch-up" is removed with the reason *party blocked* and not invoiced,
/// the lines done stay, and `finish_work` is allowed once "brake pads" is done.
#[renox::test]
async fn aut_17_finish_safety_lines_only_takes_off_the_lines_not_started() {
    let t = boot().await;
    let job = blocked_job(&t).await;
    t.people.set_blocked(CUSTOMER, true);
    let source = source_for(JOB_CARD_KIND).expect("a source");

    let open = source
        .open_for(t.state(), PartyId(CUSTOMER))
        .await
        .unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].subject, job.card.subject());
    assert!(open[0].label.contains(&job.card.number()), "{}", open[0].label);

    // *Continue* removes nothing.
    source
        .decide(t.state(), 1, &job.card.subject(), Decision::Continue)
        .await
        .unwrap();
    assert_eq!(t.labour_now(job.paint.id).await.approval, LineStatus::Approved);
    // *End* is not the review's: the advisor cancels or declines the job.
    let end = source
        .decide(t.state(), 1, &job.card.subject(), Decision::End)
        .await
        .unwrap_err();
    assert!(end.to_string().contains("not from the review"), "{end}");

    source
        .decide(t.state(), 1, &job.card.subject(), Decision::Pause)
        .await
        .unwrap();
    let paint = t.labour_now(job.paint.id).await;
    assert_eq!(paint.approval, LineStatus::Removed);
    assert_eq!(paint.removed_reason.as_deref(), Some(PARTY_BLOCKED));
    let can = t.part_now(job.paint_can.id).await;
    assert_eq!(can.approval, LineStatus::Removed, "the unstarted part goes too");
    assert_eq!(can.removed_reason.as_deref(), Some(PARTY_BLOCKED));
    // The safety lines and the work done stay.
    assert_eq!(t.labour_now(job.brakes.id).await.approval, LineStatus::Approved);
    assert_eq!(t.labour_now(job.oil.id).await.approval, LineStatus::Approved);
    assert_eq!(t.part_now(job.brake_set.id).await.approval, LineStatus::Approved);
    // Again: nothing more to take off.
    let again = finish_safety_lines_only(t.state(), 1, job.card.id).await.unwrap();
    assert_eq!((again.labour, again.parts), (0, 0));

    // The job card goes on: `finish_work` waits for the brake pads only.
    let tareq = t.technician("Tareq").await;
    let early = job_cards::finish_work(t.state(), &tareq, job.card.id)
        .await
        .unwrap_err();
    assert!(early.to_string().to_lowercase().contains("labour"), "{early}");
    job_cards::record_work(t.state(), &tareq, job.brakes.id, WorkStatus::Done, None)
        .await
        .unwrap();
    let finished = job_cards::finish_work(t.state(), &tareq, job.card.id)
        .await
        .unwrap();
    assert_eq!(finished.status, JobCardStatus::QualityCheck);

    // Delivered, the removed lines are not invoiced.
    let waleed = t.qc().await;
    let all: Vec<String> = auto::job_card::QC_KEYS
        .iter()
        .map(|k| (*k).to_owned())
        .collect();
    job_cards::save_qc(t.state(), &waleed, job.card.id, &all)
        .await
        .unwrap();
    job_cards::pass_qc(t.state(), &waleed, job.card.id)
        .await
        .unwrap();
    let sami = t.advisor().await;
    let delivered = job_cards::deliver(
        t.state(),
        &sami,
        job.card.id,
        Delivery {
            delivered_to: "Mona Hassan".to_owned(),
            ..Delivery::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(delivered.status, JobCardStatus::Delivered);
    let invoices = t.books.about(&delivered.subject());
    assert_eq!(invoices.len(), 1);
    let lines: Vec<&str> = invoices[0]
        .draft
        .lines
        .iter()
        .map(|l| l.description.as_str())
        .collect();
    assert!(lines.iter().any(|l| l.contains("Brake pads (1 h)")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("Oil change")), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("Paint")), "{lines:?}");
    assert_eq!(
        invoices[0].net(),
        20_000 + 8_000 + 15_000,
        "brake pads, the oil change and the brake pad set"
    );
}

/// Safety work is marked by the advisor on a line still in the estimate; a
/// line of another job card or a user without the right is refused; the job
/// card page shows the tag, the banner for a customer on hold and the line
/// taken off with its reason.
#[renox::test]
async fn aut_17_the_advisor_marks_safety_work_and_the_page_shows_what_was_taken_off() {
    let t = boot().await;
    let job = blocked_job(&t).await;
    let nobody = t.user("Nadia", &[auto::permissions::JOB_CARDS_VIEW]).await;
    let nadia = t.auth(&nobody).await;
    assert!(matches!(
        mark_safety(t.state(), &nadia, job.card.id, LineRef::Labour(job.paint.id), true).await,
        Err(Error::Forbidden)
    ));
    let sami = t.advisor().await;
    let wrong = mark_safety(t.state(), &sami, job.card.id + 99, LineRef::Labour(job.paint.id), true)
        .await
        .unwrap_err();
    assert!(matches!(wrong, Error::NotFound), "{wrong}");

    t.people.set_blocked(CUSTOMER, true);
    finish_safety_lines_only(t.state(), 1, job.card.id).await.unwrap();
    let removed = mark_safety(t.state(), &sami, job.card.id, LineRef::Labour(job.paint.id), true)
        .await
        .unwrap_err();
    assert!(removed.to_string().contains("still in the estimate"), "{removed}");

    let viewer = t
        .user(
            "Wasim",
            &[
                auto::permissions::JOB_CARDS_VIEW,
                auto::permissions::JOB_CARDS_ADVISE,
                auto::permissions::VEHICLES_VIEW,
            ],
        )
        .await;
    t.app.acting_as(&viewer);
    t.app
        .get(&format!("/auto/job-cards/{}", job.card.id))
        .await
        .assert_ok()
        .assert_see("Brake pads · safety work")
        .assert_see("Paint touch-up")
        .assert_see("Removed: customer on hold")
        .assert_see("This customer is on hold");
    t.app
        .htmx()
        .post(
            &format!("/auto/job-cards/{}/safety", job.card.id),
            &[("line", &format!("labour:{}", job.oil.id)), ("safety", "on")],
        )
        .await
        .assert_ok();
    assert!(t.labour_now(job.oil.id).await.safety_work);
}

/// The kind of the insurer's claims is a continue: no review task, and
/// nothing in the app asks the block of the insurer before money comes in.
#[renox::test]
async fn aut_17_the_claims_of_a_blocked_insurer_keep_their_state() {
    let t = boot().await;
    t.people.set_blocked(9, true);
    let rule = effect_for(CLAIM_KIND, &Scope::global());
    assert_eq!(rule.effect, BlockEffect::Continue);
    assert_eq!(rule.decider_role.as_deref(), Some("insurance_coordinator"));
    assert!(source_for(CLAIM_KIND).is_none());
}
