//! BL-PAY-17 (#110), `FIN.ACL.19`, open item L-20: recovering a cash shortage
//! by a salary deduction is a setting, off by default and never decided in
//! code. While it is off the option is not offered and a call is refused;
//! when the client turns it on, a deduction needs the approval of someone other
//! than the requester and the holder's acknowledgement, and is then passed to
//! wl-hr (a recording stand-in here; the crate does not depend on wl-hr).

mod support;

use wl_kernel::approvals;
use wl_kernel::custody::HandoverStatus;
use wl_payments::WlPayments;
use wl_payments::cash_recovery::{ACTION, RecoveryKind, RecoveryStatus};
use wl_payments::gateway::FakeGateway;

use support::{boot, boot_with};

/// AC6: with the default setting (salary deduction off) the deduction option is
/// not offered on the handover page and an API call is refused; nothing is
/// recorded and wl-hr is not asked.
#[renox::test]
async fn with_the_default_setting_the_deduction_is_not_offered_and_is_refused() {
    let t = boot().await;
    assert!(!t.state().provided::<wl_payments::PaymentsConfig>().unwrap().cash_salary_deduction);
    let r = t.holder("Rana").await;
    let c = t.counter("Chandra").await;
    let handover = t.short_handover(&r, "2027-04-01", 230_000, "2250.00", &c).await;

    t.app.acting_as(&c);
    let page = t.app.get(&format!("/cash/{}", handover.id)).await;
    page.assert_ok();
    page.assert_see("Record a recovery");
    page.assert_dont_see("Deduct from salary");

    t.cash_post(&c, &format!("/cash/{}/deductions", handover.id), &[("amount", "50.00")])
        .await
        .assert_status(422);
    assert!(t.recoveries(&handover).await.is_empty());
    assert!(t.hr.deductions().is_empty());
    assert_eq!(t.reloaded(&handover).await.status, HandoverStatus::Short);
}

/// AC7: after the client turns salary deduction on, a deduction needs approval
/// by someone other than C and R's acknowledgement, and is passed to wl-hr; it
/// then counts as a recovery and the handover can be recovered.
#[renox::test]
async fn a_deduction_needs_an_approval_and_the_holders_acknowledgement_before_it_goes_to_hr() {
    let t = boot_with(
        WlPayments::new().cash_salary_deduction(true),
        FakeGateway::new(),
    )
    .await;
    let r = t.holder("Rana").await;
    let c = t.counter("Chandra").await;
    let n = t.cash_manager("Nadia").await;
    let handover = t.short_handover(&r, "2027-04-01", 230_000, "2250.00", &c).await;

    // The option is offered now; the holder cannot ask for it.
    t.app.acting_as(&c);
    t.app
        .get(&format!("/cash/{}", handover.id))
        .await
        .assert_see("Deduct from salary");
    t.cash_post(&r, &format!("/cash/{}/deductions", handover.id), &[("amount", "50.00")])
        .await
        .assert_forbidden();

    t.cash_post(&c, &format!("/cash/{}/deductions", handover.id), &[("amount", "50.00")])
        .await
        .assert_ok();

    let recoveries = t.recoveries(&handover).await;
    assert_eq!(recoveries.len(), 1);
    let recovery = &recoveries[0];
    assert_eq!(recovery.kind, RecoveryKind::SalaryDeduction);
    assert_eq!(recovery.status, RecoveryStatus::Requested);
    assert_eq!(recovery.amount, 5_000);
    let request = t
        .pending_request(&recovery.subject())
        .await
        .expect("an approval waits");
    assert_eq!(request.action, ACTION);
    assert_eq!(request.requester_id, c.id);
    assert_eq!(recovery.approval_request_id, Some(request.id));
    assert!(approvals::approve(t.state(), request.id, &c, "mine").await.is_err());
    assert!(approvals::approve(t.state(), request.id, &r, "mine").await.is_err());
    assert_eq!(t.reloaded(&handover).await.recovered, 0, "nothing counts yet");

    approvals::approve(t.state(), request.id, &n, "agreed").await.unwrap();

    let authorised = &t.recoveries(&handover).await[0];
    assert_eq!(authorised.status, RecoveryStatus::Authorised);
    assert_eq!(authorised.approved_by, Some(n.id));
    assert!(t.hr.deductions().is_empty(), "HR hears when the holder agrees");
    assert_eq!(t.reloaded(&handover).await.recovered, 0);

    // Only the person it is taken from acknowledges it.
    t.cash_post(&c, &format!("/cash/recoveries/{}/acknowledge", authorised.id), &[])
        .await
        .assert_status(422);
    assert!(t.hr.deductions().is_empty());
    t.cash_post(&r, &format!("/cash/recoveries/{}/acknowledge", authorised.id), &[])
        .await
        .assert_ok();

    let passed = t.hr.deductions();
    assert_eq!(passed.len(), 1);
    assert_eq!(passed[0].amount, 5_000);
    assert_eq!(passed[0].responsible_user, r.id);
    assert_eq!(passed[0].approved_by, Some(n.id));
    assert!(passed[0].acknowledged);
    assert_eq!(passed[0].handover.0, handover.id);
    let recorded = &t.recoveries(&handover).await[0];
    assert_eq!(recorded.status, RecoveryStatus::Recorded);
    assert_eq!(recorded.hr_reference.as_deref(), Some("ADJ-1"));
    assert_eq!(recorded.acknowledged_by, Some(r.id));
    let after = t.reloaded(&handover).await;
    assert_eq!(after.recovered, 5_000);

    t.cash_post(&c, &format!("/cash/{}/recover", handover.id), &[])
        .await
        .assert_ok();
    assert_eq!(t.reloaded(&handover).await.status, HandoverStatus::Recovered);
}

/// A deduction whose approval is rejected is `rejected`: nothing counts, HR is
/// not asked, and the shortage can still be recovered in cash.
#[renox::test]
async fn a_deduction_that_is_not_approved_changes_nothing() {
    let t = boot_with(
        WlPayments::new().cash_salary_deduction(true),
        FakeGateway::new(),
    )
    .await;
    let r = t.holder("Rana").await;
    let c = t.counter("Chandra").await;
    let n = t.cash_manager("Nadia").await;
    let handover = t.short_handover(&r, "2027-04-01", 230_000, "2250.00", &c).await;
    t.cash_post(&c, &format!("/cash/{}/deductions", handover.id), &[("amount", "30.00")])
        .await
        .assert_ok();
    let recovery = t.recoveries(&handover).await.remove(0);
    let request = t.pending_request(&recovery.subject()).await.unwrap();

    approvals::reject(t.state(), request.id, &n, "Not agreed with HR")
        .await
        .unwrap();

    let after = t.recoveries(&handover).await.remove(0);
    assert_eq!(after.status, RecoveryStatus::Rejected);
    assert!(after.failure.is_some());
    assert!(t.hr.deductions().is_empty());
    let handover_after = t.reloaded(&handover).await;
    assert_eq!(handover_after.recovered, 0);
    // The deduction no longer holds room back: the rest can be recovered in cash.
    t.cash_post(&c, &format!("/cash/{}/recover", handover.id), &[("amount", "50.00")])
        .await
        .assert_ok();
    assert_eq!(t.reloaded(&handover).await.status, HandoverStatus::Recovered);
}

/// A deduction larger than what is left is refused, and a deduction waiting
/// holds its room back from a second one.
#[renox::test]
async fn deductions_stay_within_what_is_left_to_recover() {
    let t = boot_with(
        WlPayments::new().cash_salary_deduction(true),
        FakeGateway::new(),
    )
    .await;
    let r = t.holder("Rana").await;
    let c = t.counter("Chandra").await;
    let handover = t.short_handover(&r, "2027-04-01", 230_000, "2250.00", &c).await;
    let path = format!("/cash/{}/deductions", handover.id);

    t.cash_post(&c, &path, &[("amount", "50.01")]).await.assert_status(422);
    t.cash_post(&c, &path, &[("amount", "30.00")]).await.assert_ok();
    t.cash_post(&c, &path, &[("amount", "30.00")]).await.assert_status(422);
    t.cash_post(&c, &path, &[("amount", "20.00")]).await.assert_ok();
    assert_eq!(t.recoveries(&handover).await.len(), 2);
}
