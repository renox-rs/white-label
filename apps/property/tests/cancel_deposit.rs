//! BL-PRP-01 (#251) with the real wl-payments module for the deposit and the
//! refund (no deposit book or recorder fake): the settlement of the deposit of
//! a cancelled lease is published to the tenant, the 14-day dispute window
//! passes, and the rest of the deposit is requested back.

mod support;

use std::time::Duration;

use property::{Lease, LeaseStatus};
use renox::chrono::Duration as Days;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::approvals::{ApprovalRequest, approve};
use wl_kernel::deposits::{DeductionKind, deposit_deductions};
use wl_kernel::documents::today;
use wl_kernel::payments::RefundSource;
use wl_payments::deposit::DepositStatus;
use wl_payments::refund::RefundStatus;
use wl_payments::{Deposit, Refund};

use support::{ACCOUNTANT, AGENT, Booted, MANAGER, Real, TENANT, aed, boot_with};

/// A lease waiting for signature whose deposit of AED 5,000 was received and
/// is held by wl-payments.
struct Scene {
    t: Booted,
    agent: User,
    manager: User,
    accountant: User,
    lease: Lease,
}

async fn scene() -> Scene {
    let t = boot_with(Real::PAYMENTS).await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let accountant = t.user_with("Aisha", ACCOUNTANT).await;
    let lease = t.pending(&agent, &unit, 0).await;

    t.hold_real_deposit(&lease, aed(5_000)).await;
    Scene {
        t,
        agent,
        manager,
        accountant,
        lease,
    }
}

impl Scene {
    /// The agent asks to cancel; the manager approves.
    async fn cancel(&self, extra: &[(&str, &str)]) {
        let mut form = vec![("reason", "prospect backed out")];
        form.extend_from_slice(extra);
        self.t
            .post(
                &self.agent,
                &format!("/property/leases/{}/cancel", self.lease.id),
                &form,
            )
            .await
            .assert_ok();
        let request = ApprovalRequest::pending_about(self.t.db(), &self.lease.subject())
            .await
            .unwrap()
            .remove(0);
        approve(self.t.state(), request.id, &self.manager, "ok")
            .await
            .unwrap()
            .expect("the final approval");
    }

    async fn deposit(&self) -> Deposit {
        Deposit::query()
            .where_(Deposit::SOURCE_TYPE.eq(self.lease.agreement().subject_type.as_str()))
            .where_(Deposit::SOURCE_ID.eq(self.lease.agreement().subject_id))
            .first(self.t.db())
            .await
            .unwrap()
            .expect("the deposit")
    }

    /// The accountant refunds the deposit by transfer on `days` from today:
    /// whether the route answered 200.
    async fn refund_on(&self, days: i64) -> bool {
        // Outside `at_travelled_time` the clock is the real one: the request
        // runs in the travelled one.
        let target = (today() + Days::days(days)).and_hms_opt(8, 0, 0).unwrap();
        self.t.app.travel_back();
        let seconds = (target.and_utc() - renox::db::now()).num_seconds();
        assert!(seconds > 0);
        self.t.app.travel(Duration::from_secs(seconds as u64));
        let deposit = self.deposit().await;
        let response = self
            .t
            .app
            .at_travelled_time(self.t.post(
                &self.accountant,
                &format!("/deposits/{}/refund", deposit.id),
                &[("method", "transfer")],
            ))
            .await;
        response.status.is_success() || response.status.is_redirection()
    }

    /// The refunds the deposit has, with their source.
    async fn refunds(&self) -> Vec<Refund> {
        Refund::query()
            .where_(Refund::PARTY_ID.eq(TENANT.0))
            .order_by(Refund::ID)
            .get(self.t.db())
            .await
            .unwrap()
    }
}

/// AC4: at the manager's approval, without a forfeit, the settlement of the
/// deposit is published to the tenant (no deduction), a refund waits for the
/// 14-day window, and after it AED 5,000 is requested back.
#[renox::test]
async fn bl_prp_01_the_settlement_is_published_and_the_deposit_is_refunded_after_the_window() {
    let s = scene().await;
    s.cancel(&[]).await;
    assert_eq!(s.t.status(&s.lease).await, LeaseStatus::Cancelled);

    let held = s.deposit().await;
    assert_eq!(held.status, DepositStatus::SettlementPending);
    // Published to the tenant: one version, no deduction, a window of 14 days.
    let shown = deposit_deductions(s.t.state())
        .published_for(s.t.state(), TENANT)
        .await
        .unwrap();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].source, s.lease.agreement());
    assert_eq!(shown[0].version, 1);
    assert_eq!(shown[0].kind, DeductionKind::Deductions);
    assert!(shown[0].lines.is_empty());
    assert_eq!(shown[0].published_on, today());
    assert_eq!(shown[0].window_ends_on, Some(today() + Days::days(14)));

    // No refund while the window is open, however late in it.
    assert!(!s.refund_on(1).await);
    assert!(!s.refund_on(14).await);
    assert!(s.refunds().await.is_empty());
    assert_eq!(s.deposit().await.status, DepositStatus::SettlementPending);

    // The window has passed with no dispute: AED 5,000 is requested back.
    assert!(s.refund_on(15).await, "the window has passed");
    let refunded = s.deposit().await;
    assert_eq!(refunded.status, DepositStatus::Refunded);
    let refunds = s.refunds().await;
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].amount, aed(5_000));
    assert_eq!(refunds[0].status, RefundStatus::Requested);
    assert_eq!(
        refunds[0].source_kind,
        RefundSource::Deposit(refunded.deposit_id()).kind()
    );
    assert_eq!(
        Subject::new(&refunds[0].source_type, refunds[0].source_id),
        refunded.subject()
    );
}

/// AC5: a forfeit of AED 2,000 with the clause "Reservation form §4" is
/// published as a deduction that names its clause, and after the window AED
/// 3,000 is requested back.
#[renox::test]
async fn bl_prp_01_a_forfeit_is_published_with_its_clause_and_the_rest_is_refunded() {
    let s = scene().await;
    s.cancel(&[("forfeit", "2,000.00"), ("clause", "Reservation form §4")])
        .await;

    let shown = deposit_deductions(s.t.state())
        .published_for(s.t.state(), TENANT)
        .await
        .unwrap();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].lines.len(), 1);
    assert_eq!(shown[0].lines[0].amount, aed(2_000));
    assert_eq!(shown[0].lines[0].evidence, "Reservation form §4");
    assert!(
        shown[0].lines[0]
            .description
            .contains("Reservation form §4")
    );
    assert_eq!(shown[0].window_ends_on, Some(today() + Days::days(14)));

    assert!(!s.refund_on(14).await);
    assert!(s.refund_on(15).await, "the window has passed");
    let refunds = s.refunds().await;
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].amount, aed(3_000));
    assert_eq!(refunds[0].status, RefundStatus::Requested);
}
