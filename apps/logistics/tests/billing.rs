//! BL-LOG-20 (#318), 03 §4.1 step 12, §4.4 step 4, §7.2 `LOG.RUL.07`,
//! `FIN.USG.01`: on day 1 the monthly rule drafts one invoice per client with
//! the delivery lines, the re-weigh surcharges and the return charges of the
//! previous month, cumulatively and once, and leaves out the shipments that
//! are under review, lost, voided, held or disposed for a cause that is not
//! the client's. The delivery module is the real wl-delivery; the invoicer is
//! the recording stand-in of the support module.

mod support;

use logistics::notices::STAFF_NOTICE;
use logistics::permissions as perm;
use logistics::shipment::charge_kind;
use logistics::{BillingRun, Charge, Shipment, ShipmentStatus, billing, rates, reweigh};
use renox::auth::permissions;
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::approvals;
use wl_kernel::invoicing::InvoiceDraft;

use support::{OTHER_CLIENT, SOUQ, World, boot};

const INSTRUCTION: &str = "client e-mail 2026-10-27 10:15";

fn at(year: i32, month: u32, day: u32) -> DateTime {
    NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .and_hms_opt(8, 0, 0)
        .unwrap()
        .and_utc()
}

fn day(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

/// Weight bands (2 kg AED 20, 3 kg AED 24, 4 kg AED 28), the 0.5 kg tolerance
/// and the return, disposal and cancellation prices of Souq Express.
async fn card(w: &World) {
    for (up_to, amount) in [(2_000, 2_000), (3_000, 2_400), (4_000, 2_800)] {
        rates::set_band(w.db(), SOUQ, "", up_to, amount)
            .await
            .unwrap();
    }
    reweigh::set_terms(w.db(), SOUQ, 500, None).await.unwrap();
    rates::set_price(w.db(), SOUQ, charge_kind::RETURN, 1_500)
        .await
        .unwrap();
    rates::set_price(w.db(), SOUQ, charge_kind::DISPOSAL, 1_000)
        .await
        .unwrap();
    rates::set_price(w.db(), SOUQ, charge_kind::CANCELLATION, 1_000)
        .await
        .unwrap();
}

/// Writes a date column of `shipment`.
async fn set_time(w: &World, shipment: &Shipment, column: &str, when: DateTime) {
    let when = Some(when);
    Shipment::query()
        .where_(Shipment::ID.eq(shipment.id))
        .update(w.db(), &[(column, &when)])
        .await
        .unwrap();
}

/// A shipment delivered on `when` (the date is written over the real clock).
async fn delivered_on(w: &World, reference: &str, when: DateTime) -> Shipment {
    let shipment = w.delivered(SOUQ, reference, "100").await;
    set_time(w, &shipment, "delivered_at", when).await;
    w.reload(&shipment).await
}

/// A shipment booked at 2 kg, re-weighed at the hub at 4 kg (a surcharge of
/// AED 8) and delivered on `when`.
async fn reweighed_delivery(w: &World, reference: &str, when: DateTime) -> Shipment {
    let booked = w
        .book_with(&w.noor, SOUQ, reference, &[("weight_grams", "2000")])
        .await;
    let at_hub = w.at_hub(&w.pick_up(&booked).await).await;
    w.post(
        &w.hub,
        &format!("/shipments/{}/reweigh", at_hub.id),
        &[
            ("weight_kg", "4"),
            ("length_cm", ""),
            ("width_cm", ""),
            ("height_cm", ""),
            ("photo", "scale-0001.jpg"),
        ],
    )
    .await
    .assert_status(303);
    let (_out, delivery) = w.dispatch_from_hub(&w.reload(&at_hub).await).await;
    w.deliver_stop(&delivery, "0.00").await.assert_status(303);
    let after = w.reload(&booked).await;
    assert_eq!(after.status, ShipmentStatus::Delivered);
    set_time(w, &after, "delivered_at", when).await;
    w.reload(&booked).await
}

/// A picked-up shipment sent back on the client's instruction and returned
/// with its e-POD, the return dated `when`.
async fn returned_on(w: &World, reference: &str, when: DateTime) -> Shipment {
    let shipment = w.book_as(&w.noor, SOUQ, reference).await;
    let picked = w.pick_up(&shipment).await;
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/cancel-return", picked.id),
        &[("instruction", INSTRUCTION)],
    )
    .await
    .assert_status(303);
    w.post(
        &w.driver,
        &format!("/shipments/{}/returned", shipment.id),
        &[("pod_reference", "epod-0001")],
    )
    .await
    .assert_status(303);
    set_time(w, &shipment, "returned_at", when).await;
    let after = w.reload(&shipment).await;
    assert_eq!(after.status, ShipmentStatus::Returned);
    after
}

/// The delivery of `shipment` is disputed: it is `under_review`.
async fn under_review(w: &World, shipment: &Shipment) {
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/dispute", shipment.id),
        &[("reason", "The recipient denies the delivery")],
    )
    .await
    .assert_status(303);
    assert_eq!(
        w.reload(shipment).await.status,
        ShipmentStatus::UnderReview
    );
}

/// The review of the delivery of `shipment` is closed (upheld).
async fn close_review(w: &World, shipment: &Shipment) {
    let delivery = w.delivery_of(shipment).await;
    w.post(
        &w.dispatcher,
        &format!("/deliveries/{}/close-review", delivery.id),
        &[("outcome", "upheld"), ("note", "The recipient did sign")],
    )
    .await
    .assert_ok();
}

/// An operations manager who approves a disposal.
async fn manager(w: &World) -> User {
    permissions::define_role(
        w.db(),
        perm::OPERATIONS_MANAGER_ROLE,
        &[perm::SHIPMENTS_VIEW],
    )
    .await
    .unwrap();
    let user = w
        .user_with(
            "Mona",
            &[
                perm::SHIPMENTS_VIEW,
                perm::SHIPMENTS_MANAGE,
                perm::SHIPMENTS_APPROVE_DISPOSAL,
            ],
        )
        .await;
    user.assign_role(w.db(), perm::OPERATIONS_MANAGER_ROLE)
        .await
        .unwrap();
    user
}

/// A shipment held for *damaged*, then disposed of with `liability` after the
/// manager approved; the disposal is dated `when`.
async fn disposed_on(w: &World, reference: &str, liability: &str, when: DateTime) -> Shipment {
    let boss = manager(w).await;
    let booked = w.book_as(&w.noor, SOUQ, reference).await;
    let at_hub = w.at_hub(&w.pick_up(&booked).await).await;
    w.post(
        &w.hub,
        &format!("/shipments/{}/hold", at_hub.id),
        &[("reason", "damaged"), ("photos", "damage-1.jpg")],
    )
    .await
    .assert_status(303);
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/dispose", at_hub.id),
        &[
            ("reason", "Leaking, the client wants it destroyed"),
            ("photos", "leak-1.jpg"),
            ("instruction", "client-email-2026-10-20.eml"),
            ("liability", liability),
        ],
    )
    .await
    .assert_status(303);
    let request = approvals::ApprovalRequest::pending_about(w.db(), &at_hub.subject())
        .await
        .unwrap()
        .pop()
        .expect("a request waits for approval");
    approvals::approve(w.state(), request.id, &boss, "agreed")
        .await
        .unwrap();
    let after = w.reload(&booked).await;
    assert_eq!(after.status, ShipmentStatus::Disposed);
    set_time(w, &after, "disposed_at", when).await;
    w.reload(&booked).await
}

/// The drafts the module asked the invoicer for.
fn drafts(w: &World) -> Vec<InvoiceDraft> {
    w.invoices.drafts()
}

/// The descriptions of the lines of a draft.
fn descriptions(draft: &InvoiceDraft) -> Vec<String> {
    draft.lines.iter().map(|l| l.description.clone()).collect()
}

fn mentions(draft: &InvoiceDraft, number: &str) -> bool {
    descriptions(draft).iter().any(|d| d.contains(number))
}

/// AC1: client C's October has delivered shipments (3 with re-weigh
/// surcharges of AED 24 in all), 2 returned, 1 `under_review`, 1 `lost`, 1
/// `voided` and 1 disposed with liability *business*: the 1 November draft has
/// the delivery lines, the AED 24 of surcharges and the 2 return charges, and
/// none of the other 4.
#[renox::test]
async fn the_november_draft_bills_deliveries_surcharges_and_returns_and_none_of_the_rest() {
    let w = boot().await;
    card(&w).await;
    let lost = {
        let shipment = w
            .book_with(&w.dispatcher, SOUQ, "ORD-11", &[("cod", "100")])
            .await;
        let (_out, delivery) = w.out_for_delivery(&shipment).await;
        w.fail_stop(&delivery, "not_home").await.assert_status(303);
        let marked = wl_delivery::lost::run(
            w.state(),
            renox::db::now() + renox::chrono::TimeDelta::days(8),
        )
        .await
        .unwrap();
        assert_eq!(marked, 1);
        let after = w.reload(&shipment).await;
        assert_eq!(after.status, ShipmentStatus::Lost);
        after
    };
    let october = at(2026, 10, 15);
    let plain_a = delivered_on(&w, "ORD-1", october).await;
    let plain_b = delivered_on(&w, "ORD-2", october).await;
    let heavy: Vec<Shipment> = vec![
        reweighed_delivery(&w, "ORD-3", october).await,
        reweighed_delivery(&w, "ORD-4", october).await,
        reweighed_delivery(&w, "ORD-5", october).await,
    ];
    let returned_a = returned_on(&w, "ORD-6", at(2026, 10, 20)).await;
    let returned_b = returned_on(&w, "ORD-7", at(2026, 10, 21)).await;
    let reviewed = delivered_on(&w, "ORD-8", october).await;
    under_review(&w, &reviewed).await;
    let voided = w.book_as(&w.noor, SOUQ, "ORD-9").await;
    w.post(
        &w.noor,
        &format!("/shipments/{}/void", voided.id),
        &[("reason", "duplicate")],
    )
    .await
    .assert_status(303);
    let business = disposed_on(&w, "ORD-10", "business", at(2026, 10, 22)).await;
    let drafted = billing::run_monthly(w.state(), day(2026, 11, 1))
        .await
        .unwrap();
    assert_eq!(drafted.len(), 1);
    assert_eq!(drafted[0].client, SOUQ);
    assert_eq!(drafted[0].period, "2026-10");
    let all = drafts(&w);
    assert_eq!(all.len(), 1);
    let draft = &all[0];
    assert_eq!(draft.party, SOUQ);
    // 5 delivery lines, 3 surcharges of AED 8 and 2 return charges.
    assert_eq!(draft.lines.len(), 10);
    let net: i64 = draft.lines.iter().map(|l| l.net()).sum();
    assert_eq!(net, 5 * 2_000 + 3 * 800 + 2 * 1_500);
    let surcharges: i64 = draft
        .lines
        .iter()
        .filter(|l| l.description.starts_with("Re-weigh surcharge"))
        .map(|l| l.net())
        .sum();
    assert_eq!(surcharges, 2_400);
    for shipment in [&plain_a, &plain_b] {
        assert!(mentions(draft, &format!("Delivery {}", shipment.number)));
    }
    for shipment in &heavy {
        assert!(mentions(draft, &format!("Delivery {}", shipment.number)));
        assert!(mentions(
            draft,
            &format!("Re-weigh surcharge {}", shipment.number)
        ));
    }
    for shipment in [&returned_a, &returned_b] {
        assert!(mentions(draft, &format!("Return charge {}", shipment.number)));
        assert!(
            !mentions(draft, &format!("Delivery {}", shipment.number)),
            "a return before any attempt is billed the return charge only"
        );
    }
    for left_out in [&reviewed, &voided, &business, &lost] {
        assert!(
            !mentions(draft, &left_out.number),
            "{} is not billed",
            left_out.number
        );
    }
    // Billed once: the shipments are marked, the charge lines too.
    assert!(w.reload(&plain_a).await.invoiced_on.is_some());
    assert!(w.reload(&reviewed).await.invoiced_on.is_none());
    let unbilled = Charge::query()
        .where_(Charge::BILLED_ON.is_null())
        .get(w.db())
        .await
        .unwrap();
    assert!(
        unbilled.iter().all(|c| c.shipment_id != returned_a.id),
        "{unbilled:?}"
    );
}

/// AC2: the shipment under review on 1 November is left out; after its review
/// is closed on 10 November it is on the 1 December draft.
#[renox::test]
async fn a_shipment_under_review_on_day_one_is_billed_on_the_december_draft() {
    let w = boot().await;
    card(&w).await;
    let shipment = delivered_on(&w, "ORD-1", at(2026, 10, 12)).await;
    under_review(&w, &shipment).await;

    let november = billing::run_monthly(w.state(), day(2026, 11, 1))
        .await
        .unwrap();
    assert!(november.is_empty());
    assert!(drafts(&w).is_empty());

    close_review(&w, &shipment).await;
    assert_eq!(
        w.reload(&shipment).await.status,
        ShipmentStatus::Delivered
    );
    let december = billing::run_monthly(w.state(), day(2026, 12, 1))
        .await
        .unwrap();
    assert_eq!(december.len(), 1);
    assert_eq!(december[0].period, "2026-11");
    let all = drafts(&w);
    assert_eq!(all.len(), 1);
    assert!(mentions(&all[0], &format!("Delivery {}", shipment.number)));
}

/// AC3: a `cancel_return` on 29 October and the e-POD on 2 November: the
/// return charge is not on the 1 November draft and is on the 1 December
/// one.
#[renox::test]
async fn a_return_charge_is_billed_in_the_month_of_the_epod() {
    let w = boot().await;
    card(&w).await;
    let shipment = returned_on(&w, "ORD-1", at(2026, 11, 2)).await;
    let charges = shipment.charges(w.db()).await.unwrap();
    assert_eq!(charges.len(), 1);
    assert_eq!(charges[0].kind, charge_kind::RETURN);

    let november = billing::run_monthly(w.state(), day(2026, 11, 1))
        .await
        .unwrap();
    assert!(november.is_empty());
    assert!(drafts(&w).is_empty());

    let december = billing::run_monthly(w.state(), day(2026, 12, 1))
        .await
        .unwrap();
    assert_eq!(december.len(), 1);
    let all = drafts(&w);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].lines.len(), 1);
    assert_eq!(all[0].lines[0].net(), 1_500);
    assert!(mentions(&all[0], &format!("Return charge {}", shipment.number)));
}

/// A shipment returned after a failed visit is billed its delivery charge and
/// the return charge; a client with only returns still gets a draft.
#[renox::test]
async fn a_returned_shipment_that_was_attempted_is_billed_the_delivery_too() {
    let w = boot().await;
    card(&w).await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    let (_out, delivery) = w.out_for_delivery(&shipment).await;
    w.fail_stop(&delivery, "not_home").await.assert_status(303);
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/return", shipment.id),
        &[],
    )
    .await
    .assert_status(303);
    w.post(
        &w.driver,
        &format!("/shipments/{}/returned", shipment.id),
        &[("pod_reference", "epod-0001")],
    )
    .await
    .assert_status(303);
    set_time(&w, &shipment, "returned_at", at(2026, 10, 25)).await;

    billing::run_monthly(w.state(), day(2026, 11, 1))
        .await
        .unwrap();
    let all = drafts(&w);
    assert_eq!(all.len(), 1);
    let net: i64 = all[0].lines.iter().map(|l| l.net()).sum();
    assert_eq!(net, 2_000 + 1_500);
    assert!(mentions(&all[0], &format!("Delivery {}", shipment.number)));
    assert!(mentions(&all[0], &format!("Return charge {}", shipment.number)));
}

/// AC4: a disposed shipment with liability *client* is billed its delivery
/// charge and its disposal charge.
#[renox::test]
async fn a_disposal_the_client_caused_bills_the_delivery_and_the_disposal_charge() {
    let w = boot().await;
    card(&w).await;
    let shipment = disposed_on(&w, "ORD-1", "client", at(2026, 10, 22)).await;

    billing::run_monthly(w.state(), day(2026, 11, 1))
        .await
        .unwrap();
    let all = drafts(&w);
    assert_eq!(all.len(), 1);
    let net: i64 = all[0].lines.iter().map(|l| l.net()).sum();
    assert_eq!(net, 2_000 + 1_000);
    assert!(mentions(&all[0], &format!("Delivery {}", shipment.number)));
    assert!(mentions(
        &all[0],
        &format!("Disposal charge {}", shipment.number)
    ));
}

/// AC5: running the rule twice on 1 November leaves one draft for C.
#[renox::test]
async fn running_the_rule_twice_leaves_one_draft_per_client() {
    let w = boot().await;
    card(&w).await;
    delivered_on(&w, "ORD-1", at(2026, 10, 15)).await;

    let first = billing::run_monthly(w.state(), day(2026, 11, 1))
        .await
        .unwrap();
    let second = billing::run_monthly(w.state(), day(2026, 11, 1))
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    assert!(second.is_empty());
    assert_eq!(drafts(&w).len(), 1);
    let runs = BillingRun::query()
        .where_(BillingRun::CLIENT_PARTY_ID.eq(SOUQ.0))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].period, "2026-10");
    assert_eq!(runs[0].invoice_id, Some(1));
    assert_eq!(runs[0].line_count, 1);

    // The next month finds nothing new: a shipment is billed once.
    let december = billing::run_monthly(w.state(), day(2026, 12, 1))
        .await
        .unwrap();
    assert!(december.is_empty());
    assert_eq!(drafts(&w).len(), 1);
}

/// The rule is on the scheduler under its task name (monthly, day 1): the
/// scheduled task drafts for what was delivered before this month.
#[renox::test]
async fn the_rule_is_scheduled_and_the_scheduled_task_builds_the_draft() {
    let w = boot().await;
    card(&w).await;
    assert_eq!(billing::DAY, 1);
    delivered_on(
        &w,
        "ORD-1",
        renox::db::now() - renox::chrono::TimeDelta::days(40),
    )
    .await;

    w.app
        .kernel()
        .run_scheduled(billing::TASK)
        .await
        .expect("the task is registered and runs");

    assert_eq!(drafts(&w).len(), 1);
}

/// One draft per client: two clients each get theirs.
#[renox::test]
async fn each_client_gets_its_own_draft() {
    let w = boot().await;
    card(&w).await;
    delivered_on(&w, "ORD-1", at(2026, 10, 15)).await;
    let other = w.book_as(&w.farah, OTHER_CLIENT, "ORD-1").await;
    let (_out, delivery) = w.out_for_delivery(&other).await;
    w.deliver_stop(&delivery, "0.00").await.assert_status(303);
    set_time(&w, &other, "delivered_at", at(2026, 10, 16)).await;

    let drafted = billing::run_monthly(w.state(), day(2026, 11, 1))
        .await
        .unwrap();
    assert_eq!(drafted.len(), 2);
    let parties: Vec<i64> = drafts(&w).iter().map(|d| d.party.0).collect();
    assert!(parties.contains(&SOUQ.0));
    assert!(parties.contains(&OTHER_CLIENT.0));
}

/// Closes the seam of BL-LOG-04: a surcharge recorded after its shipment was
/// invoiced goes on the next draft alone, and the issued invoice is not
/// changed.
#[renox::test]
async fn a_surcharge_recorded_after_the_invoice_goes_on_the_next_draft_alone() {
    let w = boot().await;
    card(&w).await;
    let booked = w
        .book_with(&w.noor, SOUQ, "ORD-1", &[("weight_grams", "2000")])
        .await;
    let at_hub = w.at_hub(&w.pick_up(&booked).await).await;
    // The shipment is on an issued invoice already.
    billing::mark_shipment_invoiced(w.db(), at_hub.id, at(2026, 10, 1))
        .await
        .unwrap();
    w.post(
        &w.hub,
        &format!("/shipments/{}/reweigh", at_hub.id),
        &[
            ("weight_kg", "4"),
            ("length_cm", ""),
            ("width_cm", ""),
            ("height_cm", ""),
            ("photo", "scale-0001.jpg"),
        ],
    )
    .await
    .assert_status(303);
    let (_out, delivery) = w.dispatch_from_hub(&w.reload(&at_hub).await).await;
    w.deliver_stop(&delivery, "0.00").await.assert_status(303);
    set_time(&w, &booked, "delivered_at", at(2026, 10, 28)).await;

    billing::run_monthly(w.state(), day(2026, 11, 1))
        .await
        .unwrap();
    let all = drafts(&w);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].lines.len(), 1);
    assert_eq!(all[0].lines[0].net(), 800);
    assert!(all[0].lines[0].description.starts_with("Re-weigh surcharge"));
}

/// Closes the seam of BL-LOG-04: a surcharge held by an open re-weigh ticket
/// stays out until the ticket is resolved.
#[renox::test]
async fn a_surcharge_held_by_a_ticket_is_left_out_until_it_is_resolved() {
    let w = boot().await;
    card(&w).await;
    let heavy = reweighed_delivery(&w, "ORD-1", at(2026, 10, 15)).await;
    w.post(
        &w.noor,
        &format!("/shipments/{}/reweigh-dispute", heavy.id),
        &[("note", "the parcel is 2 kg")],
    )
    .await
    .assert_status(303);

    billing::run_monthly(w.state(), day(2026, 11, 1))
        .await
        .unwrap();
    let all = drafts(&w);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].lines.len(), 1);
    assert!(all[0].lines[0].description.starts_with("Delivery"));

    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/reweigh-dispute/resolve", heavy.id),
        &[("outcome", "confirm")],
    )
    .await
    .assert_status(303);
    billing::run_monthly(w.state(), day(2026, 12, 1))
        .await
        .unwrap();
    let all = drafts(&w);
    assert_eq!(all.len(), 2);
    assert_eq!(all[1].lines.len(), 1);
    assert_eq!(all[1].lines[0].net(), 800);
}

/// A shipment on hold is left out of the monthly draft with its surcharge,
/// and is billed once it is released and delivered.
#[renox::test]
async fn a_held_shipment_and_its_surcharge_are_left_out() {
    let w = boot().await;
    card(&w).await;
    let booked = w
        .book_with(&w.noor, SOUQ, "ORD-1", &[("weight_grams", "2000")])
        .await;
    let at_hub = w.at_hub(&w.pick_up(&booked).await).await;
    w.post(
        &w.hub,
        &format!("/shipments/{}/reweigh", at_hub.id),
        &[
            ("weight_kg", "4"),
            ("length_cm", ""),
            ("width_cm", ""),
            ("height_cm", ""),
            ("photo", "scale-0001.jpg"),
        ],
    )
    .await
    .assert_status(303);
    w.post(
        &w.hub,
        &format!("/shipments/{}/hold", at_hub.id),
        &[("reason", "client_instruction")],
    )
    .await
    .assert_status(303);
    assert_eq!(w.reload(&booked).await.status, ShipmentStatus::OnHold);
    assert!(Shipment::billable(w.db(), SOUQ).await.unwrap().is_empty());

    let drafted = billing::run_monthly(w.state(), day(2026, 11, 1))
        .await
        .unwrap();
    assert!(drafted.is_empty());
    assert!(drafts(&w).is_empty());
}

/// A shipment cancelled before pickup is billed its cancellation fee only
/// when the rate card has one.
#[renox::test]
async fn a_cancellation_fee_is_billed_and_a_cancelled_shipment_has_no_delivery_line() {
    let w = boot().await;
    card(&w).await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    w.post(&w.noor, &format!("/shipments/{}/cancel", shipment.id), &[])
        .await
        .assert_status(303);
    assert_eq!(
        w.reload(&shipment).await.status,
        ShipmentStatus::Cancelled
    );
    assert!(Shipment::billable(w.db(), SOUQ).await.unwrap().is_empty());

    // The fee was recorded now; the run on 1 January is after it.
    billing::run_monthly(w.state(), day(2027, 1, 1))
        .await
        .unwrap();
    let all = drafts(&w);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].lines.len(), 1);
    assert_eq!(all[0].lines[0].net(), 1_000);
    assert!(all[0].lines[0].description.starts_with("Cancellation fee"));
}

/// No return price on the rate card: the return is billed at AED 0 and the
/// accountants are told "no return price for client X".
#[renox::test]
async fn a_return_without_a_price_is_billed_at_zero_and_the_accountants_are_told() {
    let w = boot().await;
    let shipment = returned_on(&w, "ORD-1", at(2026, 10, 25)).await;

    billing::run_monthly(w.state(), day(2026, 11, 1))
        .await
        .unwrap();
    let all = drafts(&w);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].lines.len(), 1);
    assert_eq!(all[0].lines[0].net(), 0);
    assert!(mentions(&all[0], &shipment.number));
    let users = w.notices.users_of_type(STAFF_NOTICE);
    assert!(users.contains(&w.amal.id), "{users:?}");
    let drafted: Vec<_> = w
        .notices
        .of_type(STAFF_NOTICE)
        .into_iter()
        .filter(|n| n.data["title"].as_str().unwrap().contains("Monthly draft"))
        .collect();
    assert!(!drafted.is_empty());
    let body = drafted[0].data["body"].as_str().unwrap();
    assert!(body.contains("no return price for client 501"), "{body}");
}
