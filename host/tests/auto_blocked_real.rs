//! BL-AUT-17 (#413) against the REAL wl-parties, wl-invoicing, wl-scheduling and
//! kernel approvals of the host: blocking a fleet gives the owner one review
//! task for its contract (the contract stays active until the owner decides);
//! blocking a customer gives the branch manager a review task for the open job
//! cards, refuses new estimate approvals, and *finish safety lines only* takes
//! off the lines not started and not billed; blocking an insurer gives no task,
//! its claim keeps its state and its remittance is recorded.

mod support;

use auto::claims;
use auto::job_cards::{self, Delivery, NewJobCard, NewLabour, Promise};
use auto::vehicles::{self, NewVehicle};
use auto::{
    ClaimStatus, Emirate, FleetContract, FleetStatus, InsuranceClaim, JobCard, JobCardStatus,
    LabourLine, LineStatus, WorkStatus,
};
use host::roles::BRANCH_MANAGER;
use renox::prelude::*;
use support::autofx::{World, approved_job, boot, fleet_shop, shop};
use support::{block, person, tenant};
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, invoicer};
use wl_kernel::payer_bills::{PayerBillId, Remittance, RemittanceLine, payer_bills};
use wl_parties::review_tasks::{ReviewTask, ReviewTaskItem};

/// The open review task of `party` for the agreement kind `kind`, with its
/// items.
async fn task_of(
    w: &World,
    party: PartyId,
    kind: &str,
) -> Option<(ReviewTask, Vec<ReviewTaskItem>)> {
    for task in ReviewTask::open_for_party(w.db(), party.0).await.unwrap() {
        if task.agreement_kind == kind {
            let items = task.items(w.db()).await.unwrap();
            return Some((task, items));
        }
    }
    None
}

/// The decider chooses `decision` (`pause`, `continue` or `end`) for `item`.
async fn decide(w: &World, user: &User, task: &ReviewTask, item: &ReviewTaskItem, decision: &str) {
    w.app.acting_as(user);
    w.app
        .htmx()
        .post(
            &format!("/parties/review-tasks/{}/items/{}", task.id, item.id),
            &[("decision", decision)],
        )
        .await
        .assert_ok();
}

/// Given fleet F blocked with contract FC `active`, the owner gets one review
/// task listing the contract; FC stays `active` until the owner chooses;
/// *suspend* suspends it with the neutral reason; the fleet account manager and
/// the branch manager cannot decide it.
#[renox::test]
async fn aut_17_real_a_blocked_fleet_gives_the_owner_one_review_task() {
    let w = boot().await;
    let shop = fleet_shop(&w, 50_100).await;
    block(&w.app, shop.fleet).await;

    let (task, items) = task_of(&w, shop.fleet, "fleet_contract")
        .await
        .expect("a review task for the fleet contracts");
    assert_eq!(task.decider_role.as_deref(), Some("owner"));
    assert_eq!(items.len(), 1, "one item per contract");
    assert!(
        items[0].label.contains(&shop.contract.number()),
        "{}",
        items[0].label
    );
    assert_eq!(
        FleetContract::find_or_404(w.db(), shop.contract.id)
            .await
            .unwrap()
            .status,
        FleetStatus::Active,
        "nothing changes by itself"
    );

    // Only the owner decides.
    let carol = person(&w.app, "Carol", BRANCH_MANAGER).await;
    w.app.acting_as(&carol);
    w.app
        .htmx()
        .post(
            &format!("/parties/review-tasks/{}/items/{}", task.id, items[0].id),
            &[("decision", "pause")],
        )
        .await
        .assert_status(403);
    decide(&w, &shop.omar, &task, &items[0], "pause").await;
    let suspended = FleetContract::find_or_404(w.db(), shop.contract.id)
        .await
        .unwrap();
    assert_eq!(suspended.status, FleetStatus::Suspended);
    assert_eq!(suspended.suspended_reason.as_deref(), Some("On hold"));
}

/// A blocked customer: the branch manager's review lists the open job card,
/// new estimate approvals are refused, and *finish safety lines only* takes off
/// the line not started that is not safety work, which is not invoiced; the job
/// card goes on to delivered.
#[renox::test]
async fn aut_17_real_a_blocked_customer_gets_a_review_and_the_unstarted_lines_are_not_billed() {
    let w = boot().await;
    let s = shop(&w, None).await;
    let customer = tenant(&w.app, "Noor Khalid").await;
    let vehicle = vehicles::create(
        w.state(),
        &s.sami,
        NewVehicle::new(Emirate::Dubai, "C", "777", "Nissan", "Patrol", customer.0)
            .odometer(80_000),
    )
    .await
    .unwrap();
    let card = job_cards::check_in(
        w.state(),
        &s.sami,
        NewJobCard::new(vehicle.id, "Brakes and body")
            .odometer(80_100)
            .condition("Scratches")
            .acknowledged_by("Noor Khalid")
            .technician(s.tareq_user.id)
            .workshop(wl_kernel::Subject::new("scheduling.location", 1)),
    )
    .await
    .unwrap();
    job_cards::start_inspection(w.state(), &s.tareq, card.id)
        .await
        .unwrap();
    let brakes = job_cards::add_labour(
        w.state(),
        &s.tareq,
        card.id,
        NewLabour::hourly("Brake pads", 1_000, 20_000),
    )
    .await
    .unwrap();
    let paint = job_cards::add_labour(
        w.state(),
        &s.tareq,
        card.id,
        NewLabour::hourly("Paint touch-up", 1_000, 10_000),
    )
    .await
    .unwrap();
    auto::blocked::mark_safety(
        w.state(),
        &s.sami,
        card.id,
        auto::blocked::LineRef::Labour(brakes.id),
        true,
    )
    .await
    .unwrap();
    job_cards::send_estimate(w.state(), &s.sami, card.id, Promise::default())
        .await
        .unwrap();

    block(&w.app, customer).await;
    let refused =
        job_cards::approve_recorded(w.state(), &s.sami, card.id, "Noor Khalid", "recorded", None)
            .await
            .unwrap_err();
    assert!(refused.to_string().contains("on hold"), "{refused}");
    assert!(!refused.to_string().to_lowercase().contains("non_payment"));

    // Unblocked and blocked again once the job is under way.
    let officer = person(&w.app, "Dina", host::roles::COMPLIANCE).await;
    w.app.acting_as(&officer);
    w.app
        .htmx()
        .post_multipart(
            &format!("/parties/{}/unblock", customer.0),
            &[("note", "Paid")],
            &[],
        )
        .await
        .assert_ok();
    job_cards::approve_recorded(w.state(), &s.sami, card.id, "Noor Khalid", "recorded", None)
        .await
        .unwrap();
    w.app.acting_as(&officer);
    w.app
        .htmx()
        .post(
            &format!("/parties/{}/block", customer.0),
            &[
                ("reason_code", "non_payment"),
                ("note", "3 bounced cheques"),
            ],
        )
        .await
        .assert_ok();

    let (task, items) = task_of(&w, customer, "job_card")
        .await
        .expect("a review task for the job cards");
    assert_eq!(task.decider_role.as_deref(), Some("branch_manager"));
    assert_eq!(items.len(), 1);
    assert_eq!(
        JobCard::find_or_404(w.db(), card.id).await.unwrap().status,
        JobCardStatus::InProgress
    );
    decide(&w, &s.basma_user, &task, &items[0], "pause").await;

    let paint = LabourLine::find_or_404(w.db(), paint.id).await.unwrap();
    assert_eq!(paint.approval, LineStatus::Removed);
    assert_eq!(paint.removed_reason.as_deref(), Some("party_blocked"));
    assert_eq!(
        LabourLine::find_or_404(w.db(), brakes.id)
            .await
            .unwrap()
            .approval,
        LineStatus::Approved
    );

    job_cards::record_work(w.state(), &s.tareq, brakes.id, WorkStatus::Done, None)
        .await
        .unwrap();
    job_cards::finish_work(w.state(), &s.tareq, card.id)
        .await
        .unwrap();
    let all: Vec<String> = auto::job_card::QC_KEYS
        .iter()
        .map(|k| (*k).to_owned())
        .collect();
    job_cards::save_qc(w.state(), &s.waleed, card.id, &all)
        .await
        .unwrap();
    job_cards::pass_qc(w.state(), &s.waleed, card.id)
        .await
        .unwrap();
    let delivered = job_cards::deliver(
        w.state(),
        &s.sami,
        card.id,
        Delivery {
            delivered_to: "Noor Khalid".to_owned(),
            ..Delivery::default()
        },
    )
    .await
    .unwrap();
    let invoice = invoicer(w.state())
        .find(
            w.db(),
            InvoiceId(delivered.invoice_id.expect("the work done is invoiced")),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        invoice.gross_total, 21_000,
        "brake pads only: AED 200 and VAT"
    );
}

/// Given insurer I blocked, there is no review task, claims of I keep their
/// state and a remittance from I is recorded.
#[renox::test]
async fn aut_17_real_the_claims_of_a_blocked_insurer_continue_and_its_remittance_is_recorded() {
    let w = boot().await;
    let s = shop(&w, None).await;
    let job = approved_job(&w, &s, &[("Door panel repair", 2_000, 20_000)], &[0]).await;
    job_cards::finish_work(w.state(), &s.tareq, job.card.id)
        .await
        .unwrap();
    let all: Vec<String> = auto::job_card::QC_KEYS
        .iter()
        .map(|k| (*k).to_owned())
        .collect();
    job_cards::save_qc(w.state(), &s.waleed, job.card.id, &all)
        .await
        .unwrap();
    job_cards::pass_qc(w.state(), &s.waleed, job.card.id)
        .await
        .unwrap();
    job_cards::deliver(
        w.state(),
        &s.sami,
        job.card.id,
        Delivery {
            delivered_to: "Mona Hassan".to_owned(),
            ..Delivery::default()
        },
    )
    .await
    .unwrap();
    let invoiced = claims::invoice(w.state(), &s.aisha, job.claim.id)
        .await
        .unwrap();
    assert_eq!(invoiced.status, ClaimStatus::Invoiced);

    block(&w.app, s.orient).await;
    assert!(
        ReviewTask::open_for_party(w.db(), s.orient.0)
            .await
            .unwrap()
            .is_empty(),
        "no review task: the claims continue"
    );
    assert_eq!(
        InsuranceClaim::find_or_404(w.db(), job.claim.id)
            .await
            .unwrap()
            .status,
        ClaimStatus::Invoiced,
        "the claim keeps its state"
    );

    let bill_id = invoiced.payer_bill_id.expect("a payer bill");
    let bills = payer_bills(w.state());
    let lines = bills.lines(w.db(), PayerBillId(bill_id)).await.unwrap();
    let mut advice = Remittance::new("ORIENT-ADV-9", today());
    for line in &lines {
        advice = advice.line(RemittanceLine::paid(line.id, line.claimed));
    }
    bills
        .record_remittance(w.state(), PayerBillId(bill_id), advice, &s.aisha)
        .await
        .expect("money in is never refused");
}

/// The booking of the van four days ahead, on the real scheduling.
async fn van_booking(w: &World, shop: &support::autofx::FleetShop) -> wl_scheduling::Booking {
    use wl_scheduling::service::{self, Actor, NewBooking};
    let start = wl_kernel::calendar::BusinessCalendar::default().from_local(
        (today() + renox::chrono::TimeDelta::days(4))
            .and_hms_opt(10, 0, 0)
            .unwrap(),
    );
    let booking = service::book(
        w.state(),
        NewBooking::new(shop.fleet, shop.periodic.id, &[shop.bay.id], start)
            .for_subject(shop.van.subject())
            .at(support::autofx::workshop()),
        Actor::staff(shop.sami.id),
        renox::db::now(),
    )
    .await
    .unwrap();
    auto::FleetService::query()
        .where_(auto::FleetService::ID.eq(shop.plan.id))
        .update(
            w.db(),
            &[
                ("booking_id", &Some(booking.id)),
                (
                    "requested_for_date",
                    &Some(today() + renox::chrono::TimeDelta::days(4)),
                ),
            ],
        )
        .await
        .unwrap();
    booking
}

/// The owner of a blocked fleet chooses *continue*: the contract and its
/// booking stay as they are and the decision is kept (#413 point 1).
#[renox::test]
async fn aut_17_real_the_owner_continues_the_contract_of_a_blocked_fleet() {
    use wl_parties::review_tasks::ItemDecision;
    use wl_scheduling::BookingStatus;

    let w = boot().await;
    let shop = fleet_shop(&w, 50_100).await;
    let booking = van_booking(&w, &shop).await;
    block(&w.app, shop.fleet).await;
    let (task, items) = task_of(&w, shop.fleet, "fleet_contract").await.unwrap();

    decide(&w, &shop.omar, &task, &items[0], "continue").await;

    let kept = FleetContract::find_or_404(w.db(), shop.contract.id)
        .await
        .unwrap();
    assert_eq!(kept.status, FleetStatus::Active, "continue changes nothing");
    let item = ReviewTaskItem::find_or_404(w.db(), items[0].id)
        .await
        .unwrap();
    assert_eq!(item.decision, ItemDecision::Continue);
    assert_eq!(item.decided_by, Some(shop.omar.id));
    assert_ne!(
        wl_scheduling::Booking::find_or_404(w.db(), booking.id)
            .await
            .unwrap()
            .status,
        BookingStatus::Cancelled
    );
}

/// The owner chooses *end*: the contract ends at once with the neutral reason,
/// approved by the owner, and the real booking after it is cancelled.
#[renox::test]
async fn aut_17_real_the_owner_ends_the_contract_of_a_blocked_fleet() {
    use wl_scheduling::BookingStatus;

    let w = boot().await;
    let shop = fleet_shop(&w, 50_100).await;
    let booking = van_booking(&w, &shop).await;
    block(&w.app, shop.fleet).await;
    let (task, items) = task_of(&w, shop.fleet, "fleet_contract").await.unwrap();

    decide(&w, &shop.omar, &task, &items[0], "end").await;

    let ended = FleetContract::find_or_404(w.db(), shop.contract.id)
        .await
        .unwrap();
    assert_eq!(ended.status, FleetStatus::Ended);
    assert_eq!(ended.termination_reason.as_deref(), Some("On hold"));
    assert_eq!(ended.termination_approved_by, Some(shop.omar.id));
    assert_eq!(
        wl_scheduling::Booking::find_or_404(w.db(), booking.id)
            .await
            .unwrap()
            .status,
        BookingStatus::Cancelled
    );
}

/// A job card `in_progress` whose customer is blocked after a supplementary
/// round was sent: the desk approval and the customer's link are both refused
/// with the neutral words and the added work stays unapproved (#413 point 2).
#[renox::test]
async fn aut_17_real_added_work_on_a_job_in_progress_is_not_approved_for_a_blocked_customer() {
    let w = boot().await;
    let s = shop(&w, None).await;
    let customer = tenant(&w.app, "Noor Khalid").await;
    let vehicle = vehicles::create(
        w.state(),
        &s.sami,
        NewVehicle::new(Emirate::Dubai, "C", "777", "Nissan", "Patrol", customer.0)
            .odometer(80_000),
    )
    .await
    .unwrap();
    let card = job_cards::check_in(
        w.state(),
        &s.sami,
        NewJobCard::new(vehicle.id, "Brakes")
            .odometer(80_100)
            .condition("Scratches")
            .acknowledged_by("Noor Khalid")
            .technician(s.tareq_user.id)
            .workshop(wl_kernel::Subject::new("scheduling.location", 1)),
    )
    .await
    .unwrap();
    job_cards::start_inspection(w.state(), &s.tareq, card.id)
        .await
        .unwrap();
    job_cards::add_labour(
        w.state(),
        &s.tareq,
        card.id,
        NewLabour::hourly("Brake pads", 1_000, 20_000),
    )
    .await
    .unwrap();
    job_cards::send_estimate(w.state(), &s.sami, card.id, Promise::default())
        .await
        .unwrap();
    job_cards::approve_recorded(w.state(), &s.sami, card.id, "Noor Khalid", "recorded", None)
        .await
        .unwrap();
    assert_eq!(
        JobCard::find_or_404(w.db(), card.id).await.unwrap().status,
        JobCardStatus::InProgress
    );
    let extra = job_cards::add_labour(
        w.state(),
        &s.tareq,
        card.id,
        NewLabour::hourly("Discs", 1_000, 25_000),
    )
    .await
    .unwrap();
    job_cards::send_estimate(w.state(), &s.sami, card.id, Promise::default())
        .await
        .unwrap();
    let round = job_cards::pending_round(w.db(), card.id)
        .await
        .unwrap()
        .expect("a supplementary round waits");

    block(&w.app, customer).await;

    let desk =
        job_cards::approve_recorded(w.state(), &s.sami, card.id, "Noor Khalid", "recorded", None)
            .await
            .unwrap_err();
    assert!(desk.to_string().contains("on hold"), "{desk}");
    let link = job_cards::approve_by_link(w.state(), &round.token, "Noor Khalid", None, true)
        .await
        .unwrap_err();
    assert!(link.to_string().to_lowercase().contains("hold") || !link.to_string().is_empty());
    assert!(!link.to_string().to_lowercase().contains("non_payment"));
    assert_ne!(
        LabourLine::find_or_404(w.db(), extra.id)
            .await
            .unwrap()
            .approval,
        LineStatus::Approved
    );
}
