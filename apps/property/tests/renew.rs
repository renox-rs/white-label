//! BL-PRP-07 (#257): a renewal version that is activated ends the version it
//! renews as `renewed`, also from `expired`. The old version's scheduled
//! charges that the new one covers are cancelled, the invoices of a
//! holding-over period stay, and the new version does not bill them again.

mod support;

use property::lease_machine::RENEW;
use property::renew;
use property::renewal;
use property::{ChargeStatus, Lease, LeaseCharge, LeaseStatus, NewLease, NewVersion};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;

use support::{AGENT, Booted, MANAGER, TENANT, aed, boot, d};

/// A date in any year.
fn ymd(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

/// A date in 2028.
fn y28(month: u32, day: u32) -> NaiveDate {
    ymd(2028, month, day)
}

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
    /// The usual lease: 2027-01-01 to 2027-12-31, AED 10,000 a month, in its
    /// renewal window.
    async fn year(&self) -> Lease {
        let unit = self.t.unit(&self.building, "1204").await;
        let new = NewLease::new(unit.id, TENANT, d(1, 1), d(12, 31), aed(120_000), 12);
        let lease = property::leases::create_draft(self.t.state(), None, new)
            .await
            .unwrap();
        self.t.submit(&self.agent, &lease).await.assert_ok();
        self.t.activate(&self.manager, &lease).await.assert_ok();
        renewal::run(self.t.state(), d(9, 2)).await.unwrap();
        let lease = self.t.reload(&lease).await;
        assert_eq!(lease.status, LeaseStatus::RenewalDue);
        lease
    }

    /// A renewal draft of `lease` for 2028 at `rent` a year in 12 instalments.
    async fn renewal(&self, lease: &Lease, rent: i64) -> Lease {
        property::leases::create_version(
            self.t.state(),
            Some(self.agent.id),
            lease.id,
            NewVersion::renewal(y28(1, 1), y28(12, 31), rent, 0, 12),
        )
        .await
        .unwrap()
    }

    /// The charges of `lease` as stored now, by due date.
    async fn charges(&self, lease: &Lease) -> Vec<LeaseCharge> {
        let mut all = lease.charges(self.t.db()).await.unwrap();
        all.sort_by_key(|c| (c.due_date, c.id));
        all
    }
}

/// AC1: v1 `expired` with the holding-over invoices of January and February
/// 2028 issued and the charges from March scheduled; the renewal v2 starting
/// 2028-01-01 at AED 10,000 a month is activated. v1 is `renewed`, its March
/// to December charges are cancelled, the two invoices stay, v2 schedules
/// charges from March only, and the versions are linked both ways.
#[renox::test]
async fn bl_prp_07_a_renewal_from_expired_does_not_bill_a_period_twice() {
    let s = scene().await;
    let v1 = s.year().await;
    // Expired on 2028-01-01; the holding-over charges of 2028 are scheduled.
    renewal::run(s.t.state(), y28(1, 1)).await.unwrap();
    assert_eq!(s.t.status(&v1).await, LeaseStatus::Expired);
    // January and February are invoiced by the daily run.
    property::invoicing::run(s.t.state(), y28(2, 15))
        .await
        .unwrap();
    let holding: Vec<LeaseCharge> = s
        .charges(&v1)
        .await
        .into_iter()
        .filter(|c| c.holding_over)
        .collect();
    assert_eq!(holding.len(), 12);
    let invoiced: Vec<&LeaseCharge> = holding
        .iter()
        .filter(|c| c.status == ChargeStatus::Invoiced)
        .collect();
    assert_eq!(invoiced.len(), 2, "January and February 2028 are invoiced");
    let invoice_ids: Vec<i64> = invoiced.iter().filter_map(|c| c.invoice_id).collect();

    // The renewal is back-dated to 2028-01-01 and activated later.
    let v2 = s.renewal(&v1, aed(120_000)).await;
    s.t.submit(&s.agent, &v2).await.assert_ok();
    s.t.activate(&s.manager, &v2).await.assert_ok();

    let v1_now = s.t.reload(&v1).await;
    let v2_now = s.t.reload(&v2).await;
    assert_eq!(v1_now.status, LeaseStatus::Renewed);
    assert_eq!(v2_now.status, LeaseStatus::Active);
    // Linked both ways.
    assert_eq!(v1_now.next_id, Some(v2.id));
    assert_eq!(v2_now.previous_id, Some(v1.id));

    // v1: March to December 2028 are cancelled; January and February stay
    // invoiced, and so do the charges of 2027.
    let v1_charges = s.charges(&v1).await;
    for charge in v1_charges.iter().filter(|c| c.due_date >= y28(3, 1)) {
        assert_eq!(
            charge.status,
            ChargeStatus::Cancelled,
            "{}",
            charge.due_date
        );
    }
    let cancelled = v1_charges
        .iter()
        .filter(|c| c.status == ChargeStatus::Cancelled)
        .count();
    assert_eq!(cancelled, 10);
    for charge in v1_charges.iter().filter(|c| c.due_date < y28(3, 1)) {
        assert_eq!(charge.status, ChargeStatus::Invoiced, "{}", charge.due_date);
    }
    // The holding-over invoices are neither credited nor voided.
    for id in &invoice_ids {
        let invoice = s.t.invoices.get(*id).expect("the invoice stays");
        assert_ne!(invoice.summary.status, wl_kernel::InvoiceStatus::Credited);
        assert_eq!(invoice.summary.balance, invoice.summary.gross_total);
    }

    // v2: January and February are skipped, March to December are scheduled.
    let v2_charges = s.charges(&v2).await;
    assert_eq!(v2_charges.len(), 12);
    let live: Vec<&LeaseCharge> = v2_charges
        .iter()
        .filter(|c| c.status == ChargeStatus::Scheduled)
        .collect();
    assert_eq!(live.len(), 10);
    assert_eq!(live[0].due_date, y28(3, 1));
    assert_eq!(live[9].due_date, y28(12, 1));
    assert!(live.iter().all(|c| c.amount == aed(10_000)));
    for skipped in v2_charges.iter().filter(|c| c.due_date < y28(3, 1)) {
        assert_eq!(
            skipped.status,
            ChargeStatus::Cancelled,
            "{}",
            skipped.due_date
        );
    }

    // Nothing bills a period twice: the run after the activation drafts no
    // invoice for January or February again.
    let before = s.t.invoices.all().len();
    property::invoicing::run(s.t.state(), y28(3, 1))
        .await
        .unwrap();
    assert_eq!(s.t.invoices.all().len(), before + 1, "March only");
}

/// AC2: v1 `renewal_due` and v2 starting the day after v1's end: v1 is
/// `renewed` and no charge is cancelled or skipped.
#[renox::test]
async fn bl_prp_07_a_renewal_that_starts_after_the_end_cancels_nothing() {
    let s = scene().await;
    let v1 = s.year().await;
    let before = s.charges(&v1).await;
    let v2 = s.renewal(&v1, aed(120_000)).await;
    s.t.submit(&s.agent, &v2).await.assert_ok();
    s.t.activate(&s.manager, &v2).await.assert_ok();

    assert_eq!(s.t.status(&v1).await, LeaseStatus::Renewed);
    let after = s.charges(&v1).await;
    assert_eq!(after.len(), before.len());
    assert!(after.iter().all(|c| c.status == ChargeStatus::Scheduled));
    let v2_charges = s.charges(&v2).await;
    assert_eq!(v2_charges.len(), 12);
    assert!(
        v2_charges
            .iter()
            .all(|c| c.status == ChargeStatus::Scheduled)
    );
    assert!(s.t.tasks.with_key("property.relink_cheques:").is_empty());
    // No period was invoiced twice, so there is nothing to compare.
    let v2_now = s.t.reload(&v2).await;
    assert!(
        renew::differences(s.t.db(), &v2_now)
            .await
            .unwrap()
            .is_empty()
    );
}

/// AC3: the versions are linked both ways, also from `renewal_due`.
#[renox::test]
async fn bl_prp_07_the_versions_are_linked_both_ways() {
    let s = scene().await;
    let v1 = s.year().await;
    assert_eq!(v1.next_id, None);
    let v2 = s.renewal(&v1, aed(120_000)).await;
    s.t.submit(&s.agent, &v2).await.assert_ok();
    s.t.activate(&s.manager, &v2).await.assert_ok();
    let v1_now = s.t.reload(&v1).await;
    let v2_now = s.t.reload(&v2).await;
    assert_eq!(v1_now.next_id, Some(v2.id));
    assert_eq!(v2_now.previous_id, Some(v1.id));
    assert_eq!(v1_now.root_id, v2_now.root_id);
}

/// A different rent for a period already invoiced is not adjusted: the
/// property manager is shown the difference per period on the new version.
#[renox::test]
async fn bl_prp_07_a_different_rent_for_an_invoiced_period_is_shown_not_adjusted() {
    let s = scene().await;
    let v1 = s.year().await;
    renewal::run(s.t.state(), y28(1, 1)).await.unwrap();
    property::invoicing::run(s.t.state(), y28(2, 15))
        .await
        .unwrap();

    // AED 11,000 a month: the rent goes up, so the tenant accepted it.
    let v2 = s.renewal(&v1, aed(132_000)).await;
    s.t.accept(&v2);
    s.t.submit(&s.agent, &v2).await.assert_ok();
    s.t.activate(&s.manager, &v2).await.assert_ok();

    let v2_now = s.t.reload(&v2).await;
    let rows = renew::differences(s.t.db(), &v2_now).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].period, y28(1, 1));
    assert_eq!(rows[1].period, y28(2, 1));
    for row in &rows {
        assert_eq!(row.invoiced, aed(10_000));
        assert_eq!(row.renewal, aed(11_000));
        assert_eq!(row.difference(), aed(1_000));
    }
    assert!(renew::any_difference(&rows));
    // Nothing was credited or charged by the platform.
    let credited =
        s.t.invoices
            .all()
            .iter()
            .filter(|i| i.summary.status == wl_kernel::InvoiceStatus::Credited)
            .count();
    assert_eq!(credited, 0);

    s.t.app.acting_as(&s.manager);
    s.t.app
        .get(&format!("/property/leases/{}", v2.id))
        .await
        .assert_ok()
        .assert_see("Periods already invoiced before the renewal started")
        .assert_see("+AED 1,000.00");
}

/// The cheques linked to the charges that were cancelled stay `received` and
/// the accountant gets one task to relink them to the new version.
#[renox::test]
async fn bl_prp_07_cheques_of_cancelled_charges_stay_and_the_accountant_relinks_them() {
    let s = scene().await;
    let v1 = s.year().await;
    renewal::run(s.t.state(), y28(1, 1)).await.unwrap();
    let march = s
        .charges(&v1)
        .await
        .into_iter()
        .find(|c| c.holding_over && c.due_date == y28(3, 1))
        .expect("the March holding-over charge");
    let cheque = s.t.cheques.receive(
        TENANT,
        &v1.agreement(),
        "C-0301",
        y28(3, 1),
        aed(10_000),
        &[march.subject()],
    );

    let v2 = s.renewal(&v1, aed(120_000)).await;
    s.t.submit(&s.agent, &v2).await.assert_ok();
    s.t.activate(&s.manager, &v2).await.assert_ok();

    let march_now = s
        .charges(&v1)
        .await
        .into_iter()
        .find(|c| c.id == march.id)
        .unwrap();
    assert_eq!(march_now.status, ChargeStatus::Cancelled);
    assert_eq!(
        s.t.cheques.get(cheque).status,
        wl_kernel::ChequeStatus::Received,
        "the cheque is not touched"
    );
    let tasks =
        s.t.tasks
            .with_key(&renew::relink_key(&s.t.reload(&v2).await));
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].open);
    assert!(
        tasks[0].description.contains("C-0301"),
        "{}",
        tasks[0].description
    );
}

/// `renew` is the activation's: it is refused when no renewal version of the
/// lease is active, and nothing changes.
#[renox::test]
async fn bl_prp_07_renew_needs_an_active_renewal_version() {
    let s = scene().await;
    let v1 = s.year().await;
    let before = s.charges(&v1).await;
    let refused = v1.transition(RENEW).by_system().run(s.t.state()).await;
    assert!(refused.is_err());
    assert_eq!(s.t.status(&v1).await, LeaseStatus::RenewalDue);
    let after = s.charges(&v1).await;
    assert!(after.iter().zip(&before).all(|(a, b)| a.status == b.status));
}
