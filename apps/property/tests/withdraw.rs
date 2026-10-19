//! BL-PRP-10 (#260): with the landlord side's written agreement the property
//! manager withdraws a notice (`PRP.ACL.20`): the lease goes back to the state
//! its dates give, the unit to `occupied`, the move-out inspection ticket and
//! the penalty that is not invoiced are cancelled, and `LeaseNoticeWithdrawn`
//! is emitted. The dates are relative to today, because the page runs the
//! transition for today.

mod support;

use property::charge_type::{self, EARLY_TERMINATION_FEE};
use property::notice;
use property::notices::HOLDING_OVER;
use property::renewal;
use property::withdraw::AGREEMENT_KIND;
use property::{ChargeStatus, Lease, LeaseCharge, LeaseStatus, UnitStatus};
use renox::prelude::*;
use wl_kernel::VatCategory;

use support::{AGENT, Booted, COMPANY, MANAGER, aed, boot, day};

struct Scene {
    t: Booted,
    building: property::Building,
    agent: User,
    manager: User,
}

async fn scene() -> Scene {
    let t = boot().await;
    let building = t.building().await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    Scene {
        t,
        building,
        agent,
        manager,
    }
}

impl Scene {
    /// A unit `number` with an `active` lease that started `start_in` days
    /// from today and runs for a year.
    async fn active(&self, number: &str, start_in: i64) -> (property::Unit, Lease) {
        let unit = self.t.unit(&self.building, number).await;
        let lease = self.t.pending(&self.agent, &unit, start_in).await;
        self.t.waive(&self.manager, &lease).await;
        self.t.activate(&self.manager, &lease).await.assert_ok();
        let lease = self.t.reload(&lease).await;
        assert_eq!(lease.status, LeaseStatus::Active);
        (unit, lease)
    }

    /// The tenant's notice, dated today, to leave `leaves_in` days from today.
    async fn tenant_notice(&self, lease: &Lease, leaves_in: i64, penalty: &str) {
        self.tenant_notice_dated(lease, 0, leaves_in, penalty).await;
    }

    /// The tenant's notice, dated `given_in` days from today.
    async fn tenant_notice_dated(
        &self,
        lease: &Lease,
        given_in: i64,
        leaves_in: i64,
        penalty: &str,
    ) {
        let planned = day(leaves_in).to_string();
        let given = day(given_in).to_string();
        let mut pairs = vec![
            ("date", given.as_str()),
            ("party", "tenant"),
            ("reason", "early_termination"),
            ("planned_move_out", planned.as_str()),
        ];
        if !penalty.is_empty() {
            pairs.push(("penalty", penalty));
        }
        self.t
            .post(
                &self.manager,
                &format!("/property/leases/{}/notice", lease.id),
                &pairs,
            )
            .await
            .assert_ok();
        assert_eq!(self.t.status(lease).await, LeaseStatus::NoticeGiven);
    }

    /// The landlord side's written agreement is attached to the lease.
    fn agreement(&self, lease: &Lease) {
        self.t.docs.accept(&lease.subject(), AGREEMENT_KIND);
    }

    async fn withdraw(&self, lease: &Lease) -> renox::testing::TestResponse {
        self.t
            .post(
                &self.manager,
                &format!("/property/leases/{}/withdraw-notice", lease.id),
                &[],
            )
            .await
    }

    async fn fees(&self, lease: &Lease) -> Vec<LeaseCharge> {
        lease
            .charges(self.t.db())
            .await
            .unwrap()
            .into_iter()
            .filter(|c| c.charge_type == EARLY_TERMINATION_FEE)
            .collect()
    }

    async fn configure_fee(&self) {
        charge_type::configure(
            self.t.db(),
            EARLY_TERMINATION_FEE,
            "Early termination fee",
            VatCategory::Exempt,
        )
        .await
        .unwrap();
    }
}

/// AC1: before the renewal window the lease goes back to `active`, the unit to
/// `occupied`, the inspection ticket and the uninvoiced penalty are cancelled
/// and `LeaseNoticeWithdrawn` is emitted.
#[renox::test]
async fn bl_prp_10_a_withdrawn_notice_before_the_window_goes_back_to_active() {
    let s = scene().await;
    // Ends 264 days from today: the window opens 144 days from today.
    let (unit, lease) = s.active("1204", -100).await;
    s.configure_fee().await;
    s.tenant_notice(&lease, 60, "20000").await;
    assert_eq!(s.t.unit_now(&unit).await.status, UnitStatus::NoticeGiven);
    assert_eq!(s.fees(&lease).await.len(), 1);
    let key = notice::move_out_key(&lease);
    assert!(s.t.tasks.with_key(&key)[0].open);

    s.agreement(&lease);
    s.withdraw(&lease).await.assert_ok();

    let after = s.t.reload(&lease).await;
    assert_eq!(after.status, LeaseStatus::Active);
    assert_eq!(after.notice_date, None);
    assert_eq!(after.notice_party, None);
    assert_eq!(after.planned_move_out, None);
    assert_eq!(s.t.unit_now(&unit).await.status, UnitStatus::Occupied);
    assert!(!s.t.tasks.with_key(&key)[0].open, "the inspection is cancelled");
    let fees = s.fees(&lease).await;
    assert_eq!(fees[0].status, ChargeStatus::Cancelled);
    let events = s.t.withdrawn.all();
    assert_eq!(events.len(), 1);
    assert_eq!((events[0].lease, events[0].status.as_str()), (lease.id, "active"));
}

/// AC2: inside the renewal window it becomes `renewal_due`.
#[renox::test]
async fn bl_prp_10_a_withdrawn_notice_inside_the_window_goes_back_to_renewal_due() {
    let s = scene().await;
    // Ends 64 days from today: the window opened 56 days ago.
    let (unit, lease) = s.active("1204", -300).await;
    s.tenant_notice(&lease, 30, "").await;
    s.agreement(&lease);
    s.withdraw(&lease).await.assert_ok();

    let after = s.t.reload(&lease).await;
    assert_eq!(after.status, LeaseStatus::RenewalDue);
    assert_eq!(after.renewal_flagged_on, Some(day(0)));
    assert_eq!(s.t.unit_now(&unit).await.status, UnitStatus::Occupied);
    assert_eq!(s.t.withdrawn.all()[0].status, "renewal_due");
}

/// After the end date it becomes `expired`; the extension that exists is not
/// made again.
#[renox::test]
async fn bl_prp_10_a_withdrawn_notice_after_the_end_date_goes_back_to_expired() {
    let s = scene().await;
    // Ended 36 days ago; the daily run expired it, then the landlord objected.
    let (_unit, lease) = s.active("1204", -400).await;
    renewal::run(s.t.state(), day(0)).await.unwrap();
    assert_eq!(s.t.status(&lease).await, LeaseStatus::Expired);
    let holding = s.t.reload(&lease).await.charges(s.t.db()).await.unwrap().len();
    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/notice", lease.id),
        &[
            ("date", day(0).to_string().as_str()),
            ("party", "landlord"),
            ("reason", "landlords_ground"),
        ],
    )
    .await
    .assert_ok();
    assert_eq!(s.t.status(&lease).await, LeaseStatus::NoticeGiven);

    s.agreement(&lease);
    s.withdraw(&lease).await.assert_ok();
    let after = s.t.reload(&lease).await;
    assert_eq!(after.status, LeaseStatus::Expired);
    assert_eq!(
        after.charges(s.t.db()).await.unwrap().len(),
        holding,
        "the holding-over extension is not made twice"
    );
}

/// A lease that was given notice before it expired and goes back after its end
/// date: `expire`'s effects run as if it expired that day.
#[renox::test]
async fn bl_prp_10_a_lease_that_expires_meanwhile_gets_the_holding_over_effects() {
    let s = scene().await;
    let (_unit, lease) = s.active("1204", -400).await;
    // Notice recorded while the lease is still `active` (the daily run has
    // not seen it yet).
    s.tenant_notice_dated(&lease, -30, -10, "").await;
    assert_eq!(s.t.reload(&lease).await.holding_over_until, None);
    let before = lease.charges(s.t.db()).await.unwrap().len();

    s.agreement(&lease);
    s.withdraw(&lease).await.assert_ok();
    let after = s.t.reload(&lease).await;
    assert_eq!(after.status, LeaseStatus::Expired);
    assert_eq!(after.expired_on, Some(day(0)));
    assert!(after.holding_over_until.is_some());
    let charges = after.charges(s.t.db()).await.unwrap();
    assert!(charges.len() > before);
    assert!(charges.iter().any(|c| c.holding_over));
    // The property managers are told that the lease is holding over.
    assert_eq!(s.t.notices.of_type(HOLDING_OVER).len(), 1);
}

/// AC3: without the agreement document it is refused.
#[renox::test]
async fn bl_prp_10_the_agreement_is_needed() {
    let s = scene().await;
    let (_unit, lease) = s.active("1204", -100).await;
    s.tenant_notice(&lease, 60, "").await;
    let refused = s.withdraw(&lease).await;
    refused.assert_status(422);
    assert!(refused.text().contains("written agreement"), "{}", refused.text());
    assert_eq!(s.t.status(&lease).await, LeaseStatus::NoticeGiven);
    assert!(s.t.withdrawn.all().is_empty());
}

/// AC4: with a lease for another tenant submitted on the unit it is refused;
/// a draft does not count.
#[renox::test]
async fn bl_prp_10_another_tenants_submitted_lease_blocks_it() {
    let s = scene().await;
    let (unit, lease) = s.active("1204", -100).await;
    s.tenant_notice(&lease, 60, "").await;
    s.agreement(&lease);

    // A draft for tenant U does not block.
    let other = property::leases::create_draft(
        s.t.state(),
        None,
        property::NewLease::new(unit.id, COMPANY, day(61), day(425), aed(90_000), 4),
    )
    .await
    .unwrap();
    // Submitted: `pending_signature`.
    s.t.set(&other, &[("status", &LeaseStatus::PendingSignature)]).await;
    let refused = s.withdraw(&lease).await;
    refused.assert_status(422);
    assert!(refused.text().contains("another tenant"), "{}", refused.text());
    assert_eq!(s.t.status(&lease).await, LeaseStatus::NoticeGiven);

    // Back to a draft: allowed.
    s.t.set(&other, &[("status", &LeaseStatus::Draft)]).await;
    s.withdraw(&lease).await.assert_ok();
    assert_eq!(s.t.status(&lease).await, LeaseStatus::Active);
}

/// AC5: with the move-out inspection completed it is refused.
#[renox::test]
async fn bl_prp_10_the_inspection_must_not_be_completed() {
    let s = scene().await;
    let (_unit, lease) = s.active("1204", -100).await;
    s.tenant_notice(&lease, 60, "").await;
    s.agreement(&lease);
    s.t.set(&lease, &[("inspection_done_on", &Some(day(-1)))]).await;
    let refused = s.withdraw(&lease).await;
    refused.assert_status(422);
    assert!(refused.text().contains("inspection"), "{}", refused.text());
    assert_eq!(s.t.status(&lease).await, LeaseStatus::NoticeGiven);
}

/// An invoiced penalty stays; only a credit note corrects it.
#[renox::test]
async fn bl_prp_10_an_invoiced_penalty_stays() {
    let s = scene().await;
    let (_unit, lease) = s.active("1204", -100).await;
    s.configure_fee().await;
    s.tenant_notice(&lease, 60, "20000").await;
    // The penalty is due 60 days from today; the daily run invoices it.
    property::invoicing::run(s.t.state(), day(61)).await.unwrap();
    assert_eq!(s.fees(&lease).await[0].status, ChargeStatus::Invoiced);

    s.agreement(&lease);
    s.withdraw(&lease).await.assert_ok();
    let fees = s.fees(&lease).await;
    assert_eq!(fees[0].status, ChargeStatus::Invoiced);
    assert!(fees[0].invoice_id.is_some());
}

/// Only a property manager of the building withdraws a notice, and only a
/// lease that has one.
#[renox::test]
async fn bl_prp_10_only_a_manager_withdraws_a_notice_that_exists() {
    let s = scene().await;
    let (_unit, lease) = s.active("1204", -100).await;
    // No notice yet.
    s.agreement(&lease);
    s.withdraw(&lease).await.assert_status(422);
    s.tenant_notice(&lease, 60, "").await;
    s.t.post(
        &s.agent,
        &format!("/property/leases/{}/withdraw-notice", lease.id),
        &[],
    )
    .await
    .assert_forbidden();
    assert_eq!(s.t.status(&lease).await, LeaseStatus::NoticeGiven);
}
