//! BL-PRP-09 (#259): a notice to end the lease is recorded from `active`,
//! `renewal_due` and `expired` with its date, party and reason. A landlord's
//! notice is a statutory notice that shows "notice not delivered" until its
//! proof is recorded; a tenant's early termination adds the penalty of the
//! contract as a charge that follows the planned move-out date while it is
//! not invoiced.

mod support;

use property::charge_type::{self, EARLY_TERMINATION_FEE};
use property::notice::{self, MOVE_OUT_CATEGORY};
use property::notices::LANDLORD_NOTICE;
use property::renewal;
use property::{ChargeStatus, Lease, LeaseCharge, LeaseStatus, NewLease, UnitStatus};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::VatCategory;
use wl_kernel::notify::{NoticeStatus, NotifyRecipient};

use support::{AGENT, Booted, MANAGER, TENANT, aed, boot, d};

/// A date in 2028.
fn y28(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2028, month, day).unwrap()
}

struct Scene {
    t: Booted,
    unit: property::Unit,
    agent: User,
    manager: User,
}

async fn scene() -> Scene {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    Scene {
        t,
        unit,
        agent,
        manager,
    }
}

impl Scene {
    /// An `active` lease 2027-01-01 to 2027-12-31, AED 120,000 a year in 12
    /// monthly instalments.
    async fn active(&self) -> Lease {
        let new = NewLease::new(self.unit.id, TENANT, d(1, 1), d(12, 31), aed(120_000), 12);
        let lease = property::leases::create_draft(self.t.state(), None, new)
            .await
            .unwrap();
        self.t.submit(&self.agent, &lease).await.assert_ok();
        self.t.activate(&self.manager, &lease).await.assert_ok();
        let lease = self.t.reload(&lease).await;
        assert_eq!(lease.status, LeaseStatus::Active);
        lease
    }

    /// `expired` on 2028-01-01, holding over.
    async fn expired(&self) -> Lease {
        let lease = self.active().await;
        renewal::run(self.t.state(), d(9, 2)).await.unwrap();
        renewal::run(self.t.state(), y28(1, 1)).await.unwrap();
        let lease = self.t.reload(&lease).await;
        assert_eq!(lease.status, LeaseStatus::Expired);
        lease
    }

    /// The early termination fee is in the catalogue, VAT exempt.
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

    /// `POST` of a notice as `user`.
    async fn notice(
        &self,
        user: &User,
        lease: &Lease,
        pairs: &[(&str, &str)],
    ) -> renox::testing::TestResponse {
        self.t
            .post(
                user,
                &format!("/property/leases/{}/notice", lease.id),
                pairs,
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
}

/// AC1: an `expired` lease, a landlord's notice (reason the landlord's
/// ground, dated 2028-02-01): the lease and the unit are `notice_given` and a
/// move-out inspection ticket is created. The notice is sent to the tenant.
#[renox::test]
async fn bl_prp_09_a_landlord_notice_from_expired_ends_the_holding_over() {
    let s = scene().await;
    let lease = s.expired().await;
    s.notice(
        &s.manager,
        &lease,
        &[
            ("date", "2028-02-01"),
            ("party", "landlord"),
            ("reason", "landlords_ground"),
            ("ground", "The owner needs the unit for his family"),
        ],
    )
    .await
    .assert_ok();

    let after = s.t.reload(&lease).await;
    assert_eq!(after.status, LeaseStatus::NoticeGiven);
    assert_eq!(after.notice_date, Some(y28(2, 1)));
    assert_eq!(after.notice_party.as_deref(), Some("landlord"));
    assert_eq!(after.notice_reason.as_deref(), Some("landlords_ground"));
    assert_eq!(
        after.notice_ground.as_deref(),
        Some("The owner needs the unit for his family")
    );
    assert!(after.notice_sent_on.is_some());
    assert_eq!(s.t.unit_now(&s.unit).await.status, UnitStatus::NoticeGiven);

    let tickets = s.t.tasks.with_key(&notice::move_out_key(&after));
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].category, MOVE_OUT_CATEGORY);
    assert!(tickets[0].open);

    // The tenant got the statutory notice.
    let sent = s.t.notices.of_type(LANDLORD_NOTICE);
    assert_eq!(sent.len(), 1);
    assert!(matches!(
        sent[0].recipients.as_slice(),
        [NotifyRecipient::Party { id }] if *id == TENANT
    ));
}

/// AC2: an `active` lease, a tenant's notice with the reason early
/// termination, the planned move-out date 2027-08-31 and a penalty of AED
/// 20,000 adds one early termination fee of AED 20,000 due 2027-08-31;
/// editing the planned date to 2027-09-15 before it is invoiced moves the
/// due date; once invoiced it stays.
#[renox::test]
async fn bl_prp_09_the_penalty_is_a_charge_that_follows_the_planned_date_until_invoiced() {
    let s = scene().await;
    let lease = s.active().await;
    s.configure_fee().await;
    s.notice(
        &s.manager,
        &lease,
        &[
            ("date", "2027-06-01"),
            ("party", "tenant"),
            ("reason", "early_termination"),
            ("planned_move_out", "2027-08-31"),
            ("penalty", "20000"),
        ],
    )
    .await
    .assert_ok();

    let after = s.t.reload(&lease).await;
    assert_eq!(after.status, LeaseStatus::NoticeGiven);
    assert_eq!(after.planned_move_out, Some(d(8, 31)));
    assert_eq!(s.t.unit_now(&s.unit).await.status, UnitStatus::NoticeGiven);
    let fees = s.fees(&after).await;
    assert_eq!(fees.len(), 1);
    assert_eq!(fees[0].amount, aed(20_000));
    assert_eq!(fees[0].due_date, d(8, 31));
    assert_eq!(fees[0].status, ChargeStatus::Scheduled);
    assert_eq!(fees[0].vat_category, VatCategory::Exempt);
    // The ticket is due on the planned date.
    let tickets = s.t.tasks.with_key(&notice::move_out_key(&after));
    assert_eq!(tickets.len(), 1);
    assert!(
        tickets[0].description.contains("2027-08-31"),
        "{}",
        tickets[0].description
    );

    // Edited before it is invoiced: the charge moves with the date.
    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/planned-move-out", lease.id),
        &[("planned_move_out", "2027-09-15")],
    )
    .await
    .assert_ok();
    assert_eq!(s.t.reload(&lease).await.planned_move_out, Some(d(9, 15)));
    assert_eq!(s.fees(&after).await[0].due_date, d(9, 15));

    // Once invoiced it stays.
    property::invoicing::run(s.t.state(), d(9, 15))
        .await
        .unwrap();
    assert_eq!(s.fees(&after).await[0].status, ChargeStatus::Invoiced);
    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/planned-move-out", lease.id),
        &[("planned_move_out", "2027-09-30")],
    )
    .await
    .assert_ok();
    assert_eq!(s.t.reload(&lease).await.planned_move_out, Some(d(9, 30)));
    assert_eq!(s.fees(&after).await[0].due_date, d(9, 15));
}

/// A penalty is refused until the client configured the early termination fee:
/// nothing is recorded.
#[renox::test]
async fn bl_prp_09_a_penalty_needs_the_charge_type_configured_first() {
    let s = scene().await;
    let lease = s.active().await;
    let refused = s
        .notice(
            &s.manager,
            &lease,
            &[
                ("date", "2027-06-01"),
                ("party", "tenant"),
                ("reason", "early_termination"),
                ("planned_move_out", "2027-08-31"),
                ("penalty", "20000"),
            ],
        )
        .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("Configure the charge type first"),
        "{}",
        refused.text()
    );
    assert_eq!(s.t.status(&lease).await, LeaseStatus::Active);
    assert_eq!(s.t.unit_now(&s.unit).await.status, UnitStatus::Occupied);
    assert!(s.fees(&lease).await.is_empty());
    assert!(s.t.tasks.with_key("property.move_out:").is_empty());
}

/// A penalty belongs to a tenant's notice for early termination.
#[renox::test]
async fn bl_prp_09_a_penalty_on_another_reason_is_refused() {
    let s = scene().await;
    let lease = s.active().await;
    s.configure_fee().await;
    let refused = s
        .notice(
            &s.manager,
            &lease,
            &[
                ("date", "2027-06-01"),
                ("party", "tenant"),
                ("reason", "end_of_term"),
                ("planned_move_out", "2027-12-31"),
                ("penalty", "20000"),
            ],
        )
        .await;
    refused.assert_status(422);
    assert_eq!(s.t.status(&lease).await, LeaseStatus::Active);
    assert!(s.fees(&lease).await.is_empty());
}

/// AC3: a tenant's notice without a planned move-out date is refused.
#[renox::test]
async fn bl_prp_09_a_tenant_notice_needs_the_planned_move_out_date() {
    let s = scene().await;
    let lease = s.active().await;
    let refused = s
        .notice(
            &s.manager,
            &lease,
            &[
                ("date", "2027-06-01"),
                ("party", "tenant"),
                ("reason", "end_of_term"),
            ],
        )
        .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("planned move-out"),
        "{}",
        refused.text()
    );
    assert_eq!(s.t.status(&lease).await, LeaseStatus::Active);
    assert_eq!(s.t.unit_now(&s.unit).await.status, UnitStatus::Occupied);
}

/// AC4: a landlord's notice shows "notice not delivered" until its proof is
/// recorded; the planned move-out date is computed from the proof date only.
#[renox::test]
async fn bl_prp_09_a_landlord_notice_shows_not_delivered_until_the_proof() {
    let s = scene().await;
    let lease = s.active().await;
    s.notice(
        &s.manager,
        &lease,
        &[
            ("date", "2027-02-01"),
            ("party", "landlord"),
            ("reason", "non_payment"),
        ],
    )
    .await
    .assert_ok();

    // The message gave up: no receipt, no proof.
    s.t.app.acting_as(&s.manager);
    s.t.app
        .get(&format!("/property/leases/{}", lease.id))
        .await
        .assert_ok()
        .assert_see("Notice not delivered");
    let notice_given = s.t.reload(&lease).await;
    assert_eq!(notice_given.planned_move_out, None);
    assert_eq!(
        notice::delivered_on(s.t.state(), &notice_given)
            .await
            .unwrap(),
        None
    );

    // Staff record the proof: delivered on 2027-02-10.
    s.t.notices.set_notice(
        &lease.subject(),
        LANDLORD_NOTICE,
        NoticeStatus::ProofRecorded {
            delivered_on: d(2, 10),
        },
    );
    s.t.app
        .get(&format!("/property/leases/{}", lease.id))
        .await
        .assert_ok()
        .assert_see("Delivered on 2027-02-10")
        .assert_dont_see("Notice not delivered");
    let proved = s.t.reload(&lease).await;
    assert_eq!(proved.notice_delivered_on, Some(d(2, 10)));
    // 90 days of notice from the proof, not from the notice date.
    assert_eq!(proved.planned_move_out, Some(d(5, 11)));
}

/// AC5: a notice without a party or without a reason is refused, by the form
/// and by the machine itself.
#[renox::test]
async fn bl_prp_09_a_notice_without_party_or_reason_is_refused() {
    let s = scene().await;
    let lease = s.active().await;
    s.notice(
        &s.manager,
        &lease,
        &[("date", "2027-06-01"), ("reason", "end_of_term")],
    )
    .await
    .assert_status(422);
    s.notice(
        &s.manager,
        &lease,
        &[("date", "2027-06-01"), ("party", "tenant")],
    )
    .await
    .assert_status(422);
    assert_eq!(s.t.status(&lease).await, LeaseStatus::Active);

    // The machine does not take a notice without its details either.
    let machine = Lease::machine();
    let transition = machine.find("record_notice").unwrap();
    let ctx = renox::state_machine::TransitionCtx::new(None);
    let refused = transition.check_guards(&lease, &ctx).await;
    assert!(matches!(refused, Err(TransitionError::Guard(_))));
    assert_eq!(s.t.status(&lease).await, LeaseStatus::Active);
}

/// A notice can be recorded from `renewal_due` too, and only a property
/// manager of the building records it.
#[renox::test]
async fn bl_prp_09_a_notice_is_recorded_from_renewal_due_by_a_manager() {
    let s = scene().await;
    let lease = s.active().await;
    renewal::run(s.t.state(), d(9, 2)).await.unwrap();
    assert_eq!(s.t.status(&lease).await, LeaseStatus::RenewalDue);
    let pairs = [
        ("date", "2027-09-10"),
        ("party", "tenant"),
        ("reason", "end_of_term"),
        ("planned_move_out", "2027-12-31"),
    ];
    // The leasing agent does not hold `leases.manage`.
    s.notice(&s.agent, &lease, &pairs).await.assert_forbidden();
    assert_eq!(s.t.status(&lease).await, LeaseStatus::RenewalDue);

    s.notice(&s.manager, &lease, &pairs).await.assert_ok();
    assert_eq!(s.t.status(&lease).await, LeaseStatus::NoticeGiven);
    // A second notice on a lease that has one is refused.
    s.notice(&s.manager, &lease, &pairs)
        .await
        .assert_status(422);
}
