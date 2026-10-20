//! BL-PAY-15 (#108): the party may dispute the published deposit deductions
//! within the dispute window (a setting, off by default, on for 01 Property).
//! While a dispute is open, `refund` and `forfeit` wait; staff answer with
//! evidence and either revise the deductions (a new published version, the
//! window starts again) or uphold them (the flag clears, the window does not
//! restart). `refund` and `forfeit` run only after the window has passed.
//!
//! The dates are travelled to; the dispute comes through the kernel's
//! `DepositDeductions` contract, which is what the portal calls.

mod support;

use wl_kernel::deposits::{DeductionKind, DepositOutcome};
use wl_payments::deposit::DepositStatus;
use wl_payments::deposit_deductions::{DepositDeduction, DepositDispute, DisputeStatus};
use wl_payments::gateway::FakeGateway;
use wl_payments::{Deposit, WlPayments};

use support::{BLUE_DUNES, Booted, QUIET_SANDS, boot, boot_with};

/// A client whose vertical allows the dispute: 14 days, 7 to answer.
async fn boot_disputes() -> Booted {
    boot_with(
        WlPayments::new().deductions_dispute(14, 7),
        FakeGateway::new(),
    )
    .await
}

/// A deposit of AED 8,000.00 being settled, with the deductions AED 2,500.00
/// (repainting) and AED 300.00 (cleaning) published on `date` (travelled to).
async fn published_on(t: &Booted, a: &renox::auth::User, date: &str) -> Deposit {
    t.travel_to(date, 8, 0);
    let deposit = t.settling_deposit(a, BLUE_DUNES, 800_000).await;
    t.publish_deductions(
        a,
        &deposit,
        "Repainting | 2500.00 | photos-1\nCleaning | 300.00 | photos-2",
        "",
    )
    .await
    .assert_ok();
    deposit
}

/// The id of the line "Repainting" of the latest published version.
async fn repainting(t: &Booted) -> i64 {
    let shown = t.published_for(BLUE_DUNES).await;
    shown[0]
        .lines
        .iter()
        .find(|line| line.description == "Repainting")
        .map(|line| line.id)
        .expect("a repainting line")
}

/// AC1 and AC2: deductions published 2030-03-01; the tenant disputes the
/// repainting line on 03-05: the flag is set and `refund` is refused. Staff
/// revise repainting to AED 800.00 on 03-10: version 2 is published on 03-10,
/// the window ends 03-24, `refund` is refused on 03-24 and allowed on 03-25.
#[renox::test]
async fn a_dispute_blocks_the_refund_and_a_revision_restarts_the_window() {
    let t = boot_disputes().await;
    let a = t.user_a().await;
    let deposit = published_on(&t, &a, "2030-03-01").await;

    let v1 = DepositDeduction::latest(t.db(), deposit.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(v1.version, 1);
    assert_eq!(v1.total, 280_000);
    assert_eq!(v1.published_on.to_string(), "2030-03-01");
    assert_eq!(
        v1.window_ends_on.map(|d| d.to_string()).as_deref(),
        Some("2030-03-15")
    );
    let shown = t.published_for(BLUE_DUNES).await;
    assert!(shown[0].can_dispute);

    t.travel_to("2030-03-05", 8, 0);
    let line = repainting(&t).await;
    t.dispute_lines(BLUE_DUNES, &deposit, 1, vec![line])
        .await
        .expect("the dispute is taken");
    assert_eq!(
        DepositDispute::open_of_deposit(t.db(), deposit.id)
            .await
            .unwrap()
            .len(),
        1
    );
    t.deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await
        .assert_status(422);
    assert_eq!(
        t.deposit(deposit.id).await.status,
        DepositStatus::SettlementPending
    );
    // The portal shows the line as disputed.
    assert_eq!(
        t.published_for(BLUE_DUNES).await[0].disputed_lines,
        vec![line]
    );

    // Staff revise repainting to AED 800.00 on 03-10.
    t.travel_to("2030-03-10", 8, 0);
    let changes = format!("{line} | 800.00");
    t.answer_dispute(&a, &deposit, &changes).await.assert_ok();

    let v2 = DepositDeduction::latest(t.db(), deposit.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(v2.version, 2);
    assert_eq!(v2.published_on.to_string(), "2030-03-10");
    assert_eq!(
        v2.window_ends_on.map(|d| d.to_string()).as_deref(),
        Some("2030-03-24")
    );
    assert_eq!(v2.total, 110_000);
    // The old version is kept, unchanged.
    let versions = DepositDeduction::of_deposit(t.db(), deposit.id)
        .await
        .unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].total, 280_000);
    let disputes = DepositDispute::of_deposit(t.db(), deposit.id)
        .await
        .unwrap();
    assert_eq!(disputes[0].status, DisputeStatus::Revised);
    assert_eq!(disputes[0].revised_in, Some(v2.id));
    assert_eq!(disputes[0].evidence.as_deref(), Some("inspection-2027-02"));
    assert!(
        DepositDispute::open_of_deposit(t.db(), deposit.id)
            .await
            .unwrap()
            .is_empty()
    );

    t.travel_to("2030-03-24", 8, 0);
    t.deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await
        .assert_status(422);
    t.travel_to("2030-03-25", 8, 0);
    t.deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await
        .assert_ok();
    assert_eq!(t.deposit(deposit.id).await.status, DepositStatus::Refunded);
    // After it the portal shows no dispute button.
    assert!(t.published_for(BLUE_DUNES).await.is_empty());
    let settled = t.deposit_settled.all();
    assert_eq!(settled.len(), 1);
    assert_eq!(settled[0].outcome, DepositOutcome::Refunded);
    assert_eq!(settled[0].deductions, 110_000);
}

/// The refund pays the balance after the deductions the party was shown: AED
/// 8,000.00 held and AED 2,800.00 deducted is a refund of AED 5,200.00. Deductions
/// that take the whole deposit leave nothing to refund.
#[renox::test]
async fn the_refund_is_the_deposit_less_the_published_deductions() {
    let t = boot_disputes().await;
    let a = t.user_a().await;
    let deposit = published_on(&t, &a, "2030-03-01").await;
    t.travel_to("2030-03-16", 8, 0);
    t.deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await
        .assert_ok();
    let refund = t
        .deposit(deposit.id)
        .await
        .linked_refund(t.db())
        .await
        .unwrap()
        .expect("the refund");
    assert_eq!(refund.amount, 520_000);

    let whole = t.settling_deposit(&a, QUIET_SANDS, 100_000).await;
    t.publish_deductions(&a, &whole, "Repainting | 1000.00 | photos-1", "")
        .await
        .assert_ok();
    t.travel_to("2030-03-31", 8, 0);
    t.deposit_action(&a, &whole, "refund", &[("method", "transfer")])
        .await
        .assert_status(422);
}

/// AC3 and AC6: staff uphold instead (on 03-08, inside the window): the flag
/// clears, the window still ends 03-15 and `refund` is allowed from 03-16.
#[renox::test]
async fn upholding_clears_the_flag_without_restarting_the_window() {
    let t = boot_disputes().await;
    let a = t.user_a().await;
    let deposit = published_on(&t, &a, "2030-03-01").await;
    t.travel_to("2030-03-05", 8, 0);
    let line = repainting(&t).await;
    t.dispute_lines(BLUE_DUNES, &deposit, 1, vec![line])
        .await
        .unwrap();

    t.travel_to("2030-03-08", 8, 0);
    t.answer_dispute(&a, &deposit, "").await.assert_ok();

    let disputes = DepositDispute::of_deposit(t.db(), deposit.id)
        .await
        .unwrap();
    assert_eq!(disputes[0].status, DisputeStatus::Upheld);
    assert_eq!(
        disputes[0].answered_on.map(|d| d.to_string()).as_deref(),
        Some("2030-03-08")
    );
    // No new version, the same window.
    let versions = DepositDeduction::of_deposit(t.db(), deposit.id)
        .await
        .unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(
        versions[0].window_ends_on.map(|d| d.to_string()).as_deref(),
        Some("2030-03-15")
    );

    // Still inside the window: refused.
    t.travel_to("2030-03-15", 8, 0);
    t.deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await
        .assert_status(422);
    t.travel_to("2030-03-16", 8, 0);
    t.deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await
        .assert_ok();
}

/// AC5: with the setting on and no line disputed, `refund` is refused on
/// 03-05 (inside the window) and allowed on 03-16; after it the portal shows no
/// dispute button.
#[renox::test]
async fn the_refund_waits_for_the_whole_window_even_without_a_dispute() {
    let t = boot_disputes().await;
    let a = t.user_a().await;
    let deposit = published_on(&t, &a, "2030-03-01").await;

    t.travel_to("2030-03-05", 8, 0);
    t.deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await
        .assert_status(422);
    t.travel_to("2030-03-16", 8, 0);
    t.deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await
        .assert_ok();
    assert!(t.published_for(BLUE_DUNES).await.is_empty());
}

/// With the setting on, the deductions must be published before the refund.
#[renox::test]
async fn the_deductions_must_be_published_before_the_refund() {
    let t = boot_disputes().await;
    let a = t.user_a().await;
    let deposit = t.settling_deposit(&a, BLUE_DUNES, 800_000).await;
    t.deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await
        .assert_status(422);
    // A version with no deduction is published as such, and the window runs.
    t.publish_deductions(&a, &deposit, "", "").await.assert_ok();
    let v1 = DepositDeduction::latest(t.db(), deposit.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(v1.total, 0);
    assert!(v1.window_ends_on.is_some());
}

/// AC4: with the setting off (other verticals) there is no dispute button, a
/// dispute is refused and `refund` needs no window.
#[renox::test]
async fn with_the_setting_off_there_is_no_dispute_and_no_window() {
    let t = boot().await;
    let a = t.user_a().await;
    let deposit = t.settling_deposit(&a, BLUE_DUNES, 800_000).await;
    t.publish_deductions(&a, &deposit, "Repainting | 2500.00 | photos-1", "")
        .await
        .assert_ok();

    let shown = t.published_for(BLUE_DUNES).await;
    assert_eq!(shown.len(), 1);
    assert!(!shown[0].can_dispute);
    assert_eq!(shown[0].window_ends_on, None);
    let line = shown[0].lines[0].id;
    assert!(
        t.dispute_lines(BLUE_DUNES, &deposit, 1, vec![line])
            .await
            .is_err()
    );
    // Refunded the same day it was published.
    t.deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await
        .assert_ok();
}

/// AC7: with the setting on, a forfeiture of the whole deposit published
/// 03-01 is refused (`forfeit`) on 03-10 and allowed on 03-16 when no dispute
/// is open; a forfeit not published to the party is refused.
#[renox::test]
async fn a_forfeit_waits_for_its_publication_and_the_window() {
    let t = boot_disputes().await;
    let a = t.user_a().await;
    let b = t.user_b().await;

    // Not published: refused.
    t.travel_to("2030-03-01", 8, 0);
    let unpublished = t.settling_deposit(&a, BLUE_DUNES, 800_000).await;
    t.forfeit_deposit(&a, &unpublished, "Abandoned the unit", Some(b.id))
        .await
        .assert_status(422);
    // Published as deductions only: still not a forfeiture.
    t.publish_deductions(&a, &unpublished, "Repainting | 2500.00 | photos-1", "")
        .await
        .assert_ok();
    t.travel_to("2030-03-16", 8, 0);
    t.forfeit_deposit(&a, &unpublished, "Abandoned the unit", Some(b.id))
        .await
        .assert_status(422);

    // Published as a forfeiture of the whole deposit on 03-01.
    t.travel_to("2030-03-01", 8, 0);
    let deposit = t.settling_deposit(&a, QUIET_SANDS, 800_000).await;
    t.publish_deductions(&a, &deposit, "", "Abandoned the unit")
        .await
        .assert_ok();
    let v1 = DepositDeduction::latest(t.db(), deposit.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(v1.kind, DeductionKind::Forfeiture);
    assert_eq!(v1.total, 800_000);
    let shown = t.published_for(QUIET_SANDS).await;
    assert_eq!(shown[0].kind, DeductionKind::Forfeiture);
    assert_eq!(shown[0].reason.as_deref(), Some("Abandoned the unit"));

    t.travel_to("2030-03-10", 8, 0);
    t.forfeit_deposit(&a, &deposit, "Abandoned the unit", Some(b.id))
        .await
        .assert_status(422);
    assert_eq!(
        t.deposit(deposit.id).await.status,
        DepositStatus::SettlementPending
    );

    t.travel_to("2030-03-16", 8, 0);
    // A forfeiture needs a reason and another user's approval.
    t.forfeit_deposit(&a, &deposit, "", Some(b.id))
        .await
        .assert_status(422);
    t.forfeit_deposit(&a, &deposit, "Abandoned the unit", None)
        .await
        .assert_status(422);
    t.forfeit_deposit(&a, &deposit, "Abandoned the unit", Some(a.id))
        .await
        .assert_status(422);
    t.forfeit_deposit(&a, &deposit, "Abandoned the unit", Some(b.id))
        .await
        .assert_ok();

    let done = t.deposit(deposit.id).await;
    assert_eq!(done.status, DepositStatus::Forfeited);
    assert_eq!(done.forfeit_reason.as_deref(), Some("Abandoned the unit"));
    assert_eq!(done.forfeit_approved_by, Some(b.id));
    let settled = t.deposit_settled.all();
    assert_eq!(settled.len(), 1);
    assert_eq!(settled[0].outcome, DepositOutcome::Forfeited);
    assert_eq!(settled[0].deductions, 800_000);
    assert_eq!(settled[0].refund_amount, 0);
}

/// A forfeit with an open dispute waits, whatever the window.
#[renox::test]
async fn a_forfeit_with_a_dispute_open_waits() {
    let t = boot_disputes().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    t.travel_to("2030-03-01", 8, 0);
    let deposit = t.settling_deposit(&a, BLUE_DUNES, 800_000).await;
    t.publish_deductions(&a, &deposit, "", "Abandoned the unit")
        .await
        .assert_ok();
    let line = t.published_for(BLUE_DUNES).await[0].lines[0].id;
    t.travel_to("2030-03-02", 8, 0);
    t.dispute_lines(BLUE_DUNES, &deposit, 1, vec![line])
        .await
        .unwrap();

    t.travel_to("2030-03-20", 8, 0);
    t.forfeit_deposit(&a, &deposit, "Abandoned the unit", Some(b.id))
        .await
        .assert_status(422);
    t.answer_dispute(&a, &deposit, "").await.assert_ok();
    t.forfeit_deposit(&a, &deposit, "Abandoned the unit", Some(b.id))
        .await
        .assert_ok();
}

/// Edge: a dispute after the window is refused; so are one of an older
/// version, one with no line or no comment, one by another party and one of a
/// line that is not on the version; a line is disputed once.
#[renox::test]
async fn a_dispute_must_be_in_time_for_the_latest_version() {
    let t = boot_disputes().await;
    let a = t.user_a().await;
    let deposit = published_on(&t, &a, "2030-03-01").await;
    let line = repainting(&t).await;

    t.travel_to("2030-03-15", 8, 0);
    // Wrong party, wrong version, no line, a line of nowhere.
    assert!(
        t.dispute_lines(QUIET_SANDS, &deposit, 1, vec![line])
            .await
            .is_err()
    );
    assert!(
        t.dispute_lines(BLUE_DUNES, &deposit, 2, vec![line])
            .await
            .is_err()
    );
    assert!(
        t.dispute_lines(BLUE_DUNES, &deposit, 1, vec![])
            .await
            .is_err()
    );
    assert!(
        t.dispute_lines(BLUE_DUNES, &deposit, 1, vec![9_999])
            .await
            .is_err()
    );
    // The last day is still in time; once only.
    t.dispute_lines(BLUE_DUNES, &deposit, 1, vec![line])
        .await
        .unwrap();
    assert!(
        t.dispute_lines(BLUE_DUNES, &deposit, 1, vec![line])
            .await
            .is_err()
    );

    // After the window: refused in the portal.
    let late = published_on(&t, &a, "2030-04-01").await;
    let late_line = t.published_for(BLUE_DUNES).await[0].lines[0].id;
    t.travel_to("2030-04-16", 8, 0);
    assert!(
        !t.published_for(BLUE_DUNES)
            .await
            .iter()
            .any(|p| p.can_dispute)
    );
    assert!(
        t.dispute_lines(BLUE_DUNES, &late, 1, vec![late_line])
            .await
            .is_err()
    );
}

/// Edge: the answer needs a response and evidence; with no open dispute there
/// is nothing to answer; publishing again while a dispute is open is refused;
/// a revision of a line that is not on the version is refused.
#[renox::test]
async fn an_answer_needs_a_response_evidence_and_an_open_dispute() {
    let t = boot_disputes().await;
    let a = t.user_a().await;
    let deposit = published_on(&t, &a, "2030-03-01").await;

    // Nothing to answer yet.
    t.answer_dispute(&a, &deposit, "").await.assert_status(422);

    t.travel_to("2030-03-02", 8, 0);
    let line = repainting(&t).await;
    t.dispute_lines(BLUE_DUNES, &deposit, 1, vec![line])
        .await
        .unwrap();
    t.deposit_page(
        &a,
        &deposit,
        "answer",
        &[("response", ""), ("evidence", "x")],
    )
    .await
    .assert_status(422);
    t.deposit_page(
        &a,
        &deposit,
        "answer",
        &[("response", "No."), ("evidence", " ")],
    )
    .await
    .assert_status(422);
    t.answer_dispute(&a, &deposit, "9999 | 10")
        .await
        .assert_status(422);
    t.publish_deductions(&a, &deposit, "Painting | 1.00 | x", "")
        .await
        .assert_status(422);
    assert_eq!(
        DepositDispute::open_of_deposit(t.db(), deposit.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// Edge: a legal case on the source agreement keeps the flag until a staff
/// member records the decision.
#[renox::test]
async fn a_legal_case_keeps_the_flag_until_its_decision_is_recorded() {
    let t = boot_disputes().await;
    let a = t.user_a().await;
    let deposit = published_on(&t, &a, "2030-03-01").await;
    t.deposit_page(&a, &deposit, "legal_case/open", &[])
        .await
        .assert_ok();
    assert!(t.deposit(deposit.id).await.dispute_legal_case);

    t.travel_to("2030-03-20", 8, 0);
    t.deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await
        .assert_status(422);
    t.deposit_page(&a, &deposit, "legal_case/close", &[])
        .await
        .assert_ok();
    t.deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await
        .assert_ok();
}

/// A deposit that is only held cannot have deductions published, and the
/// permissions are checked.
#[renox::test]
async fn deductions_are_published_for_a_settling_deposit_by_deposit_managers() {
    let t = boot_disputes().await;
    let a = t.user_a().await;
    let viewer = t
        .user("Vera", &[wl_payments::permissions::DEPOSITS_VIEW])
        .await;
    let payment = t.receive(BLUE_DUNES, 800_000).await;
    let held = t
        .hold_deposit(&a, BLUE_DUNES, 800_000, Some(&payment))
        .await;
    t.publish_deductions(&a, &held, "Repainting | 100 | x", "")
        .await
        .assert_status(422);

    let settling = t.settling_deposit(&a, QUIET_SANDS, 800_000).await;
    t.publish_deductions(&viewer, &settling, "Repainting | 100 | x", "")
        .await
        .assert_forbidden();
    t.forfeit_deposit(&viewer, &settling, "x", Some(a.id))
        .await
        .assert_forbidden();
    // The party sees the other party's deposit nowhere.
    assert!(t.published_for(BLUE_DUNES).await.is_empty());
}
