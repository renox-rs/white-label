//! BL-DLV-03 (#159), `WRK.ACL.09`, `FIN.CSH.01`, `X-11`: a cash on delivery
//! collected short is accepted only with a difference reason and an
//! approver's agreement, who chooses who absorbs it; the driver cannot
//! approve their own difference, can give up waiting (a failed attempt), and
//! a request nobody answers lapses. The cash goes to the driver's custody.

mod support;

use renox::prelude::*;
use wl_delivery::cod::{self, ApprovalChoice};
use wl_delivery::{Attempt, Delivery, DeliveryLine, DeliveryStatus, WlDelivery, permissions};
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus};

use support::{World, boot_custody_with, boot_with_custody, stop_fields, user_with};

/// A delivery of one tile line (10, AED 250 to collect) that is out.
async fn cod_delivery(w: &World) -> (Delivery, DeliveryLine) {
    let delivery = w.request(&[("Tiles", 10_000, None)], 25_000).await;
    let out = w.out_for_delivery(&delivery).await;
    let line = w.lines(&out).await.remove(0);
    (out, line)
}

/// The fields of the stop with `cash` collected and the difference `reason`.
fn short_fields(line: &DeliveryLine, cash: &str, reason: &str) -> Vec<(String, String)> {
    let mut fields = stop_fields(&[(line.id, "10", "0", "", "")], cash);
    if !reason.is_empty() {
        fields.push(("difference_reason".into(), reason.into()));
    }
    fields
}

async fn pending_request(w: &World, delivery: &Delivery) -> ApprovalRequest {
    ApprovalRequest::pending_about(w.db(), &delivery.delivery_id().subject())
        .await
        .unwrap()
        .pop()
        .expect("a request waits for approval")
}

async fn request_status(w: &World, request: &ApprovalRequest) -> ApprovalStatus {
    ApprovalRequest::find(w.db(), request.id)
        .await
        .unwrap()
        .unwrap()
        .status
}

/// The driver asks for AED 200 of 250 and the request waits.
async fn asked(w: &World) -> (Delivery, ApprovalRequest) {
    let (out, line) = cod_delivery(w).await;
    w.deliver(&w.driver, &out, &short_fields(&line, "200.00", "recipient_short"))
        .await
        .assert_status(303);
    let request = pending_request(w, &out).await;
    (out, request)
}

async fn approve_as(w: &World, delivery: &Delivery, absorbed_by: &str) {
    w.post(
        &w.approver,
        &format!("/deliveries/{}/cod-difference/approve", delivery.id),
        &[("absorbed_by", absorbed_by)],
    )
    .await
    .assert_ok();
}

/// AC1: COD AED 250, tolerance AED 0, driver A enters AED 200 with the reason
/// *recipient short of cash*: an approval request is created and the delivery
/// stays `out_for_delivery`; nothing is delivered or collected yet.
#[renox::test]
async fn a_short_payment_asks_for_approval_and_the_delivery_stays_out() {
    let w = boot_with_custody().await;
    let (out, request) = asked(&w).await;

    assert_eq!(w.reload(&out).await.status, DeliveryStatus::OutForDelivery);
    assert_eq!(request.action, cod::ACTION);
    assert_eq!(request.requester_id, w.driver.id);
    assert!(w.delivered.all().is_empty());
    assert!(w.custody.collections().is_empty());
    let minutes = (request.expires_at.expect("a lapse time") - renox::db::now()).num_minutes();
    assert!((9..=10).contains(&minutes), "{minutes} minutes");
}

/// Without a reason the short payment is refused and nothing is asked.
#[renox::test]
async fn a_short_payment_without_a_reason_is_refused() {
    let w = boot_with_custody().await;
    let (out, line) = cod_delivery(&w).await;
    w.deliver(&w.driver, &out, &short_fields(&line, "200.00", ""))
        .await
        .assert_status(422)
        .assert_see("short of the AED 250.00 expected");
    assert!(
        ApprovalRequest::pending_about(w.db(), &out.delivery_id().subject())
            .await
            .unwrap()
            .is_empty()
    );
}

/// AC2: user B (with `deliveries.approve_cod_difference`) approves and picks
/// *business*: the delivery is `delivered`, the custody of A increases by
/// AED 200, and `DeliveryDelivered` carries expected 250, collected 200,
/// difference 50, the reason and absorbed by *business*.
#[renox::test]
async fn the_approver_picks_business_and_the_cash_collected_goes_to_custody() {
    let w = boot_with_custody().await;
    let (out, request) = asked(&w).await;

    approve_as(&w, &out, "business").await;

    let after = w.reload(&out).await;
    assert_eq!(after.status, DeliveryStatus::Delivered);
    assert_eq!(request_status(&w, &request).await, ApprovalStatus::Approved);
    let collections = w.custody.collections();
    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0].holder_user, w.driver.id);
    assert_eq!(collections[0].amount, 20_000);
    assert_eq!(collections[0].source, out.delivery_id().subject());
    let run = wl_delivery::Run::find_or_404(w.db(), after.run_id.unwrap())
        .await
        .unwrap();
    assert_eq!(collections[0].period.subject, Some(run.subject()));
    let events = w.delivered.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].cash_expected, 25_000);
    assert_eq!(events[0].cash_collected, 20_000);
    assert_eq!(events[0].cod_difference, 5_000);
    assert_eq!(
        events[0].cod_difference_reason.as_deref(),
        Some("recipient_short")
    );
    assert_eq!(events[0].absorbed_by.as_deref(), Some("business"));
    let proof = after.latest_proof(w.db()).await.unwrap().unwrap();
    assert_eq!(proof.approval_request_id, Some(request.id));
    let choice = ApprovalChoice::query()
        .where_(ApprovalChoice::REQUEST_ID.eq(request.id))
        .first(w.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(choice.chosen_by, Some(w.approver.id));
}

/// AC3: the same, but the approver picks *driver*: the custody of A expects
/// the full AED 250 (the missing AED 50 is the shortage at the count).
#[renox::test]
async fn when_the_driver_absorbs_the_custody_expects_the_full_amount() {
    let w = boot_with_custody().await;
    let (out, _request) = asked(&w).await;

    approve_as(&w, &out, "driver").await;

    assert_eq!(w.reload(&out).await.status, DeliveryStatus::Delivered);
    let collections = w.custody.collections();
    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0].amount, 25_000);
    let events = w.delivered.all();
    assert_eq!(events[0].cash_collected, 20_000);
    assert_eq!(events[0].cod_difference, 5_000);
    assert_eq!(events[0].absorbed_by.as_deref(), Some("driver"));
}

/// Without a choice the reason's default applies (*customer* for a recipient
/// short of cash); an absorber that is not offered is refused.
#[renox::test]
async fn without_a_choice_the_reasons_default_applies_and_unknown_absorbers_are_refused() {
    let w = boot_with_custody().await;
    let (out, _request) = asked(&w).await;

    w.post(
        &w.approver,
        &format!("/deliveries/{}/cod-difference/approve", out.id),
        &[("absorbed_by", "the_moon")],
    )
    .await
    .assert_status(422);
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::OutForDelivery);

    approve_as(&w, &out, "").await;
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::Delivered);
    assert_eq!(w.delivered.all()[0].absorbed_by.as_deref(), Some("customer"));
}

/// 03 does not offer the customer: with no default that is offered and no
/// choice the approval is refused, and picking *client* works.
#[renox::test]
async fn the_customer_is_not_offered_where_the_client_settings_leave_it_out() {
    use wl_kernel::delivery::CodAbsorber;
    let module = WlDelivery::new().cod_absorbers(&[
        CodAbsorber::Client,
        CodAbsorber::Business,
        CodAbsorber::Driver,
    ]);
    let w = boot_custody_with(module).await;
    let (out, _request) = asked(&w).await;

    w.post(
        &w.approver,
        &format!("/deliveries/{}/cod-difference/approve", out.id),
        &[("absorbed_by", "customer")],
    )
    .await
    .assert_status(422);
    w.post(
        &w.approver,
        &format!("/deliveries/{}/cod-difference/approve", out.id),
        &[("absorbed_by", "")],
    )
    .await
    .assert_status(422)
    .assert_see("Choose who absorbs");
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::OutForDelivery);

    approve_as(&w, &out, "client").await;
    assert_eq!(w.delivered.all()[0].absorbed_by.as_deref(), Some("client"));
}

/// AC4: driver A cannot approve their own request, even holding the
/// permission; a user without the permission cannot either.
#[renox::test]
async fn the_driver_cannot_approve_their_own_difference() {
    let w = boot_with_custody().await;
    let dora = user_with(
        &w,
        "Dora",
        &[
            permissions::RUNS_DRIVE,
            permissions::COMPLETE,
            permissions::APPROVE_COD_DIFFERENCE,
            permissions::VIEW,
        ],
    )
    .await;
    let delivery = w.request(&[("Tiles", 10_000, None)], 25_000).await;
    let (out, _run) = w.out_for_delivery_by(&dora, &delivery).await;
    let line = w.lines(&out).await.remove(0);
    w.deliver(&dora, &out, &short_fields(&line, "200.00", "recipient_short"))
        .await
        .assert_status(303);
    pending_request(&w, &out).await;

    let approve = format!("/deliveries/{}/cod-difference/approve", out.id);
    w.post(&dora, &approve, &[("absorbed_by", "business")])
        .await
        .assert_forbidden();
    w.post(&w.driver, &approve, &[("absorbed_by", "business")])
        .await
        .assert_forbidden();
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::OutForDelivery);
    assert!(w.delivered.all().is_empty());
}

/// AC5: with the approval pending, A chooses *no answer: fail attempt*: the
/// delivery is `attempted` with the reason *refused to pay* and the request
/// is withdrawn.
#[renox::test]
async fn giving_up_waiting_withdraws_the_request_and_fails_the_attempt() {
    let w = boot_with_custody().await;
    let (out, request) = asked(&w).await;

    w.post(
        &w.driver,
        &format!("/driver/stops/{}/no-answer", out.id),
        &[("latitude", "25.204849"), ("longitude", "55.270783")],
    )
    .await
    .assert_status(303);

    assert_eq!(w.reload(&out).await.status, DeliveryStatus::Attempted);
    assert_eq!(request_status(&w, &request).await, ApprovalStatus::Withdrawn);
    let attempts = Attempt::query()
        .where_(Attempt::DELIVERY_ID.eq(out.id))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].failure_reason.as_deref(), Some("refused_to_pay"));
    assert_eq!(w.failed.all()[0].reason, "refused_to_pay");
    assert!(w.delivered.all().is_empty());
    assert!(w.custody.collections().is_empty());
}

/// AC6: no answer within 10 minutes: the request lapses, the phone offers
/// only `fail` with the reason *refused to pay*, and a later approval is
/// refused.
#[renox::test]
async fn a_request_nobody_answers_lapses_and_a_late_approval_is_refused() {
    let w = boot_with_custody().await;
    let (out, request) = asked(&w).await;
    let past = Some(renox::db::now() - renox::chrono::Duration::minutes(1));
    ApprovalRequest::query()
        .where_(ApprovalRequest::ID.eq(request.id))
        .update(w.db(), &[("expires_at", &past)])
        .await
        .unwrap();

    // The task that runs every minute lapses it.
    assert_eq!(cod::lapse_all(w.state()).await.unwrap(), 1);
    assert_eq!(request_status(&w, &request).await, ApprovalStatus::Expired);
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::OutForDelivery);

    // The approver is too late.
    w.post(
        &w.approver,
        &format!("/deliveries/{}/cod-difference/approve", out.id),
        &[("absorbed_by", "business")],
    )
    .await
    .assert_status(422);
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::OutForDelivery);

    // The driver cannot ask again for this visit: only a failed attempt.
    let line = w.lines(&out).await.remove(0);
    w.deliver(&w.driver, &out, &short_fields(&line, "200.00", "recipient_short"))
        .await
        .assert_status(422)
        .assert_see("No approver answered in time");
    w.fail(&w.driver, &out, "refused_to_pay")
        .await
        .assert_status(303);
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::Attempted);
}

/// The request also lapses when the approver is the first to look at it
/// after the time is up: the page of the delivery lapses it, and approving is
/// refused with nothing delivered.
#[renox::test]
async fn an_approval_after_the_time_is_up_is_refused_without_the_task() {
    let w = boot_with_custody().await;
    let (out, request) = asked(&w).await;
    let past = Some(renox::db::now() - renox::chrono::Duration::minutes(1));
    ApprovalRequest::query()
        .where_(ApprovalRequest::ID.eq(request.id))
        .update(w.db(), &[("expires_at", &past)])
        .await
        .unwrap();

    w.post(
        &w.approver,
        &format!("/deliveries/{}/cod-difference/approve", out.id),
        &[("absorbed_by", "business")],
    )
    .await
    .assert_status(422);

    assert_eq!(request_status(&w, &request).await, ApprovalStatus::Expired);
    assert!(w.delivered.all().is_empty());
}

/// AC7: with the setting "always a failed attempt", entering AED 200 of 250
/// offers only `fail`: the delivery is refused and nothing is asked.
#[renox::test]
async fn with_approval_off_a_short_payment_can_only_fail() {
    let w = boot_custody_with(WlDelivery::new().cod_approval(false)).await;
    let (out, line) = cod_delivery(&w).await;

    w.deliver(&w.driver, &out, &short_fields(&line, "200.00", "recipient_short"))
        .await
        .assert_status(422)
        .assert_see("cannot be approved");
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::OutForDelivery);
    assert!(
        ApprovalRequest::pending_about(w.db(), &out.delivery_id().subject())
            .await
            .unwrap()
            .is_empty()
    );
    w.fail(&w.driver, &out, "refused_to_pay")
        .await
        .assert_status(303);
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::Attempted);
}

/// AC8: collected AED 0 is not a difference: `deliver` is refused and only
/// `fail` is left.
#[renox::test]
async fn paying_nothing_is_a_failed_attempt_not_a_difference() {
    let w = boot_with_custody().await;
    let (out, line) = cod_delivery(&w).await;

    w.deliver(&w.driver, &out, &short_fields(&line, "0.00", "recipient_short"))
        .await
        .assert_status(422)
        .assert_see("Nothing was collected");
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::OutForDelivery);
}

/// An approver's *no* leaves the delivery out; the driver collects in full or
/// fails the attempt, and cannot ask for the same difference again.
#[renox::test]
async fn a_rejected_difference_leaves_the_delivery_out() {
    let w = boot_with_custody().await;
    let (out, request) = asked(&w).await;

    let reject = format!("/deliveries/{}/cod-difference/reject", out.id);
    w.post(&w.approver, &reject, &[("comment", "")])
        .await
        .assert_status(422);
    w.post(&w.approver, &reject, &[("comment", "Ask for the rest")])
        .await
        .assert_ok();

    assert_eq!(request_status(&w, &request).await, ApprovalStatus::Rejected);
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::OutForDelivery);
    let line = w.lines(&out).await.remove(0);
    w.deliver(&w.driver, &out, &short_fields(&line, "200.00", "recipient_short"))
        .await
        .assert_status(422)
        .assert_see("rejected the difference");
    w.deliver(&w.driver, &out, &short_fields(&line, "250.00", ""))
        .await
        .assert_status(303);
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::Delivered);
}

/// Within the tolerance a difference needs a reason but no approval: the
/// delivery is delivered at once with the reason's default absorber.
#[renox::test]
async fn a_difference_within_the_tolerance_needs_no_approval() {
    let w = boot_custody_with(WlDelivery::new().cod_tolerance(500)).await;
    let (out, line) = cod_delivery(&w).await;

    w.deliver(&w.driver, &out, &short_fields(&line, "245.00", ""))
        .await
        .assert_status(422);
    w.deliver(&w.driver, &out, &short_fields(&line, "245.00", "no_change"))
        .await
        .assert_status(303);

    let after = w.reload(&out).await;
    assert_eq!(after.status, DeliveryStatus::Delivered);
    let events = w.delivered.all();
    assert_eq!(events[0].cod_difference, 500);
    assert_eq!(events[0].cod_difference_reason.as_deref(), Some("no_change"));
    // The default of *no change* is the driver, so the custody expects it all.
    assert_eq!(events[0].absorbed_by.as_deref(), Some("driver"));
    assert_eq!(w.custody.collections()[0].amount, 25_000);
}

/// `FIN.CSH.01`: a delivery with the cash collected in full adds it to the
/// driver's custody for the run, once.
#[renox::test]
async fn the_cash_collected_goes_to_the_drivers_custody_for_the_run() {
    let w = boot_with_custody().await;
    let delivery = w.delivered_cod(12_000).await;

    let collections = w.custody.collections();
    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0].holder_user, w.driver.id);
    assert_eq!(collections[0].amount, 12_000);
    assert_eq!(collections[0].source, delivery.delivery_id().subject());
    let run = wl_delivery::Run::find_or_404(w.db(), delivery.run_id.unwrap())
        .await
        .unwrap();
    assert_eq!(collections[0].period.subject, Some(run.subject()));
    assert_eq!(collections[0].period.date, run.run_date);
    assert!(collections[0].period.escalate);
}

/// The 03 settings give up the payments escalation of the driver's period.
#[renox::test]
async fn a_client_that_replaces_the_escalation_collects_without_it() {
    let w = boot_custody_with(WlDelivery::new().replace_cash_escalation(true)).await;
    w.delivered_cod(12_000).await;
    assert!(!w.custody.collections()[0].period.escalate);
}

/// A delivery without cash adds nothing to the custody.
#[renox::test]
async fn a_delivery_without_cash_collects_nothing() {
    let w = boot_with_custody().await;
    let delivery = w.simple_delivery().await;
    let out = w.out_for_delivery(&delivery).await;
    let line = w.lines(&out).await.remove(0);
    w.deliver(&w.driver, &out, &stop_fields(&[(line.id, "10", "0", "", "")], "0"))
        .await
        .assert_status(303);
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::Delivered);
    assert!(w.custody.collections().is_empty());
}

/// The screens: while the request waits, the phone offers only the wait and
/// *no answer: fail attempt* (no delivered form); once it lapsed the phone
/// says why; the approver sees the request on the delivery with both
/// buttons, and the driver does not.
#[renox::test]
async fn the_screens_follow_the_state_of_the_request() {
    let w = boot_with_custody().await;
    let (out, request) = asked(&w).await;

    w.app.acting_as(&w.driver);
    w.app
        .get(&format!("/driver/stops/{}", out.id))
        .await
        .assert_ok()
        .assert_see("Waiting for approval")
        .assert_see("No answer: fail attempt")
        .assert_dont_see("name=\"cash_collected\"");

    w.app.acting_as(&w.approver);
    w.app
        .get(&format!("/deliveries/{}", out.id))
        .await
        .assert_ok()
        .assert_see("Cash difference waiting for approval")
        .assert_see("Approve difference")
        .assert_see("Reject difference");
    w.app.acting_as(&w.manager);
    w.app
        .get(&format!("/deliveries/{}", out.id))
        .await
        .assert_ok()
        .assert_dont_see("Approve difference");

    let past = Some(renox::db::now() - renox::chrono::Duration::minutes(1));
    ApprovalRequest::query()
        .where_(ApprovalRequest::ID.eq(request.id))
        .update(w.db(), &[("expires_at", &past)])
        .await
        .unwrap();
    w.app.acting_as(&w.driver);
    w.app
        .get(&format!("/driver/stops/{}", out.id))
        .await
        .assert_ok()
        .assert_see("Only a failed visit is left")
        .assert_see("No approver answered in time");
}
