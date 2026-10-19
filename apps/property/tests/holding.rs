//! BL-PRP-06 (#256): a lease that passes its end date without a renewal or a
//! notice becomes `expired` (`PRP.RUL.03`); it is not an end: billing
//! continues for the statutory renewal period, again for each period the
//! tenant stays, and the property manager gets a task (`PRP.RUL.14`).

mod support;

use property::holding;
use property::lease_machine::{EXPIRE, ExpireInput};
use property::notices::HOLDING_OVER;
use property::renewal;
use property::{ChargeStatus, Lease, LeaseCharge, LeaseStatus, NewLease, NewVersion};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::tickets::Assignee;

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
    /// An `active` lease of a new unit `number` from `start` to `end`, AED
    /// 120,000 a year in 12 monthly instalments of AED 10,000.
    async fn active(&self, number: &str, start: NaiveDate, end: NaiveDate) -> Lease {
        let unit = self.t.unit(&self.building, number).await;
        let new = NewLease::new(unit.id, TENANT, start, end, aed(120_000), 12);
        let lease = property::leases::create_draft(self.t.state(), None, new)
            .await
            .unwrap();
        self.t.submit(&self.agent, &lease).await.assert_ok();
        self.t.activate(&self.manager, &lease).await.assert_ok();
        self.t.reload(&lease).await
    }

    /// The usual lease: 2027-01-01 to 2027-12-31, in the renewal window.
    async fn year(&self) -> Lease {
        let lease = self.active("1204", d(1, 1), d(12, 31)).await;
        renewal::run(self.t.state(), d(9, 2)).await.unwrap();
        assert_eq!(self.t.status(&lease).await, LeaseStatus::RenewalDue);
        lease
    }

    async fn run(&self, day: NaiveDate) -> renewal::Report {
        renewal::run(self.t.state(), day).await.unwrap()
    }

    /// The charges the extension created, in order.
    async fn holding_charges(&self, lease: &Lease) -> Vec<LeaseCharge> {
        lease
            .charges(self.t.db())
            .await
            .unwrap()
            .into_iter()
            .filter(|c| c.holding_over)
            .collect()
    }

    /// A renewal draft of `lease` for 2028 on the same terms.
    async fn renewal_draft(&self, lease: &Lease) -> Lease {
        property::leases::create_version(
            self.t.state(),
            Some(self.agent.id),
            lease.id,
            NewVersion::renewal(y28(1, 1), y28(12, 31), aed(120_000), 0, 12),
        )
        .await
        .unwrap()
    }
}

/// AC1: a `renewal_due` lease 2027-01-01 to 2027-12-31 at AED 10,000 a month
/// and no notice is `expired` on 2028-01-01; 12 monthly charges of AED 10,000
/// exist from 2028-01-01 to 2028-12-01; the property manager is told. The
/// charges that existed are untouched and the end date is not changed.
#[renox::test]
async fn bl_prp_06_the_lease_expires_the_day_after_its_end_date_and_billing_goes_on() {
    let s = scene().await;
    let lease = s.year().await;
    let before = lease.charges(s.t.db()).await.unwrap();
    assert_eq!(before.len(), 12);

    // The end date itself is not yet over.
    assert_eq!(s.run(d(12, 31)).await.expired, 0);
    assert_eq!(s.t.status(&lease).await, LeaseStatus::RenewalDue);

    let report = s.run(y28(1, 1)).await;
    assert_eq!(report.expired, 1);
    let expired = s.t.reload(&lease).await;
    assert_eq!(expired.status, LeaseStatus::Expired);
    assert_eq!(expired.end_date, d(12, 31), "the end date is not changed");
    assert_eq!(expired.expired_on, Some(y28(1, 1)));
    assert_eq!(expired.holding_over_until, Some(y28(12, 31)));

    let extra = s.holding_charges(&lease).await;
    assert_eq!(extra.len(), 12);
    assert_eq!(extra[0].due_date, y28(1, 1));
    assert_eq!(extra[11].due_date, y28(12, 1));
    assert!(extra.iter().all(|c| c.amount == aed(10_000)));
    assert!(extra.iter().all(|c| c.status == ChargeStatus::Scheduled));
    assert!(extra.iter().all(|c| c.charge_type == "rent"));
    assert!(extra[0].description.contains("holding over"), "{}", extra[0].description);
    // Nothing that existed was stopped or repriced.
    let after = lease.charges(s.t.db()).await.unwrap();
    assert_eq!(after.len(), 24);
    for old in &before {
        let now = after.iter().find(|c| c.id == old.id).unwrap();
        assert_eq!((now.status, now.amount, now.due_date), (old.status, old.amount, old.due_date));
    }
    // No cheque is linked to the new charges.
    assert!(s.t.cheques.all().is_empty());

    // The property manager is told.
    let told = s.t.notices.of_type(HOLDING_OVER);
    assert_eq!(told.len(), 1);
    assert!(matches!(
        told[0].recipients.as_slice(),
        [NotifyRecipient::User { id }] if *id == s.manager.id
    ));
    assert_eq!(told[0].data["until"], "31 Dec 2028");

    // The page shows it; the lease stays usable.
    s.t.app.acting_as(&s.manager);
    s.t.app
        .get(&format!("/property/leases/{}", lease.id))
        .await
        .assert_ok()
        .assert_see("2028-12-31")
        .assert_see("Holding over");
}

/// The extension is created once: a second run on the same day, and the days
/// after, add nothing.
#[renox::test]
async fn bl_prp_06_a_rerun_does_not_extend_twice() {
    let s = scene().await;
    let lease = s.year().await;
    assert_eq!(s.run(y28(1, 1)).await.expired, 1);
    for day in [y28(1, 1), y28(1, 2), y28(6, 1), y28(12, 31)] {
        let again = s.run(day).await;
        assert_eq!((again.expired, again.extended), (0, 0), "{day}");
    }
    assert_eq!(s.holding_charges(&lease).await.len(), 12);
    assert_eq!(s.t.notices.of_type(HOLDING_OVER).len(), 1);
}

/// The extension charges are invoiced on their due dates like any other.
#[renox::test]
async fn bl_prp_06_the_extended_charges_are_invoiced_on_their_due_dates() {
    let s = scene().await;
    let lease = s.year().await;
    s.run(y28(1, 1)).await;
    let extra = s.holding_charges(&lease).await;
    property::invoicing::run(s.t.state(), y28(1, 1)).await.unwrap();
    let origin = extra[0].subject().to_string();
    let invoice = s
        .t
        .invoices
        .all()
        .into_iter()
        .find(|i| {
            i.draft.lines.iter().any(|l| {
                l.origin.as_ref().map(ToString::to_string).as_deref() == Some(origin.as_str())
            })
        })
        .expect("an invoice for the first holding-over charge");
    assert_eq!(invoice.draft.due_date, Some(y28(1, 1)));
    assert_eq!(invoice.draft.lines[0].unit_price, aed(10_000));
    // The one for February waits for its date.
    let later = extra[1].subject().to_string();
    assert!(!s.t.invoices.all().iter().any(|i| i.draft.lines.iter().any(|l| {
        l.origin.as_ref().map(ToString::to_string).as_deref() == Some(later.as_str())
    })));
}

/// `expire` needs the end date to have passed.
#[renox::test]
async fn bl_prp_06_expire_is_refused_before_the_end_date_has_passed() {
    let s = scene().await;
    let lease = s.year().await;
    let refused = lease
        .transition(EXPIRE)
        .by_system()
        .input(&ExpireInput {
            day: Some(d(12, 31)),
        })
        .run(s.t.state())
        .await;
    assert!(matches!(refused, Err(TransitionError::Guard(_))));
    assert_eq!(s.t.status(&lease).await, LeaseStatus::RenewalDue);
    assert!(s.holding_charges(&lease).await.is_empty());
    // Only a `renewal_due` lease expires; the user cannot run it.
    let active = s.active("1300", d(1, 1), d(12, 31)).await;
    let not_from = active
        .transition(EXPIRE)
        .by_system()
        .input(&ExpireInput {
            day: Some(y28(1, 1)),
        })
        .run(s.t.state())
        .await;
    assert!(matches!(not_from, Err(TransitionError::NotFrom(_))));
}

/// AC2: on 2028-01-08, with no renewal version and no notice, the property
/// manager has the task; not before, and not twice.
#[renox::test]
async fn bl_prp_06_the_manager_gets_the_holding_over_task_seven_days_later() {
    let s = scene().await;
    let lease = s.year().await;
    s.run(y28(1, 1)).await;
    assert_eq!(s.run(y28(1, 7)).await.tasked, 0);
    assert!(s.t.tasks.with_key("property.holding:").is_empty());

    let report = s.run(y28(1, 8)).await;
    assert_eq!(report.tasked, 1);
    let tasks = s.t.tasks.with_key("property.holding:");
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].open);
    assert_eq!(tasks[0].subject, lease.subject());
    assert_eq!(tasks[0].assignee, Some(Assignee::user(s.manager.id)));
    assert_eq!(tasks[0].key.as_deref(), Some(holding::task_key(&lease, y28(1, 1)).as_str()));
    assert!(tasks[0].title.contains(&lease.number()), "{}", tasks[0].title);

    assert_eq!(s.run(y28(1, 9)).await.tasked, 0);
    assert_eq!(s.t.tasks.with_key("property.holding:").len(), 1);
}

/// AC3: with a renewal draft created on 2028-01-05, no task is created on
/// 2028-01-08.
#[renox::test]
async fn bl_prp_06_a_renewal_draft_stops_a_new_task() {
    let s = scene().await;
    let lease = s.year().await;
    s.run(y28(1, 1)).await;
    s.renewal_draft(&lease).await;
    assert_eq!(s.run(y28(1, 8)).await.tasked, 0);
    assert!(s.t.tasks.with_key("property.holding:").is_empty());
}

/// AC4: with the task open from 2028-01-08, a renewal draft created on
/// 2028-01-10 leaves it open; submitting that version on 2028-01-12 closes it.
#[renox::test]
async fn bl_prp_06_only_a_submitted_renewal_closes_the_open_task() {
    let s = scene().await;
    let lease = s.year().await;
    s.run(y28(1, 1)).await;
    s.run(y28(1, 8)).await;
    assert_eq!(s.t.tasks.open().len(), 1);

    let draft = s.renewal_draft(&lease).await;
    s.run(y28(1, 10)).await;
    assert_eq!(s.t.tasks.open().len(), 1, "a draft leaves the task open");
    assert_eq!(s.run(y28(1, 11)).await.tasked, 0);

    s.t.submit(&s.agent, &draft).await.assert_ok();
    assert_eq!(s.t.status(&draft).await, LeaseStatus::PendingSignature);
    let tasks = s.t.tasks.with_key("property.holding:");
    assert_eq!(tasks.len(), 1);
    assert!(!tasks[0].open, "the submission closes it");
    assert!(tasks[0].note.contains("submitted"), "{}", tasks[0].note);
    // A task that was closed is not opened again for the same period.
    assert_eq!(s.run(y28(1, 12)).await.tasked, 0);
    assert_eq!(s.t.tasks.all().len(), 1);
}

/// The daily run closes the task of a lease whose renewal version waits for
/// signature, even when the listener did not see the submission.
#[renox::test]
async fn bl_prp_06_the_daily_run_closes_the_task_of_a_submitted_renewal() {
    let s = scene().await;
    let lease = s.year().await;
    s.run(y28(1, 1)).await;
    s.run(y28(1, 8)).await;
    let draft = s.renewal_draft(&lease).await;
    // The status is moved as a fixture would, without the listener.
    s.t.set(&draft, &[("status", &LeaseStatus::PendingSignature)]).await;
    s.run(y28(1, 9)).await;
    assert!(s.t.tasks.open().is_empty());
}

/// AC5: still `expired` on 2029-01-01: 12 more charges are created and a new
/// task appears; the extension is made once per period.
#[renox::test]
async fn bl_prp_06_a_tenant_who_stays_gets_the_next_period_and_a_new_task() {
    let s = scene().await;
    let lease = s.year().await;
    s.run(y28(1, 1)).await;
    s.run(y28(1, 8)).await;
    assert_eq!(s.run(y28(12, 31)).await.extended, 0);

    let report = s.run(ymd(2029, 1, 1)).await;
    assert_eq!(report.extended, 1);
    assert_eq!(report.tasked, 1);
    let extra = s.holding_charges(&lease).await;
    assert_eq!(extra.len(), 24);
    assert_eq!(extra[12].due_date, ymd(2029, 1, 1));
    assert_eq!(extra[23].due_date, ymd(2029, 12, 1));
    assert_eq!(
        s.t.reload(&lease).await.holding_over_until,
        Some(ymd(2029, 12, 31))
    );
    let tasks = s.t.tasks.with_key("property.holding:");
    assert_eq!(tasks.len(), 2);
    assert_eq!(
        tasks[1].key.as_deref(),
        Some(holding::task_key(&lease, ymd(2029, 1, 1)).as_str())
    );
    assert!(tasks[1].open);

    // Once per period.
    let again = s.run(ymd(2029, 1, 2)).await;
    assert_eq!((again.extended, again.tasked), (0, 0));
    assert_eq!(s.holding_charges(&lease).await.len(), 24);
}

/// A run missed for years catches up period by period.
#[renox::test]
async fn bl_prp_06_a_long_gap_is_caught_up_one_period_at_a_time() {
    let s = scene().await;
    let lease = s.year().await;
    s.run(y28(1, 1)).await;
    let report = s.run(ymd(2030, 1, 1)).await;
    assert_eq!(report.extended, 2);
    assert_eq!(s.holding_charges(&lease).await.len(), 36);
    assert_eq!(
        s.t.reload(&lease).await.holding_over_until,
        Some(ymd(2030, 12, 31))
    );
}

/// AC6: a 2-year lease ending 2027-12-31 gets a 1-year extension.
#[renox::test]
async fn bl_prp_06_a_two_year_lease_gets_a_one_year_extension() {
    let s = scene().await;
    let lease = s.active("1204", ymd(2026, 1, 1), d(12, 31)).await;
    // Flagged and expired by the same run, after the end date.
    let report = s.run(y28(1, 1)).await;
    assert_eq!((report.flagged, report.expired), (1, 1));
    let expired = s.t.reload(&lease).await;
    assert_eq!(expired.holding_over_until, Some(y28(12, 31)));
    let extra = s.holding_charges(&lease).await;
    assert_eq!(extra.len(), 12);
    assert_eq!(extra[0].due_date, y28(1, 1));
    assert_eq!(extra[11].due_date, y28(12, 1));
}

/// AC6: a 6-month lease gets 6 months, and 6 more for the next period.
#[renox::test]
async fn bl_prp_06_a_six_month_lease_gets_six_months() {
    let s = scene().await;
    let lease = s.active("1204", d(1, 1), d(6, 30)).await;
    // The plan of a 6-month lease is its six months of rent.
    LeaseCharge::query()
        .where_(LeaseCharge::LEASE_ID.eq(lease.id))
        .where_(LeaseCharge::DUE_DATE.gt(d(6, 30)))
        .update(
            s.t.db(),
            &[("status", &ChargeStatus::Cancelled)],
        )
        .await
        .unwrap();
    let report = s.run(d(7, 1)).await;
    assert_eq!((report.flagged, report.expired), (1, 1));
    assert_eq!(
        s.t.reload(&lease).await.holding_over_until,
        Some(d(12, 31))
    );
    let extra = s.holding_charges(&lease).await;
    assert_eq!(extra.len(), 6);
    assert_eq!(extra[0].due_date, d(7, 1));
    assert_eq!(extra[5].due_date, d(12, 1));

    s.run(y28(1, 1)).await;
    assert_eq!(
        s.t.reload(&lease).await.holding_over_until,
        Some(y28(6, 30))
    );
    let all = s.holding_charges(&lease).await;
    assert_eq!(all.len(), 12);
    assert_eq!(all[6].due_date, y28(1, 1));
    assert_eq!(all[11].due_date, y28(6, 1));
}

/// AC7: a lease with a notice recorded does not expire; neither does one that
/// is still `active` before its window.
#[renox::test]
async fn bl_prp_06_a_lease_with_a_notice_does_not_expire() {
    let s = scene().await;
    let lease = s.year().await;
    // The notice (BL-PRP-09) is a fixture here.
    s.t.set(&lease, &[("status", &LeaseStatus::NoticeGiven)]).await;
    let report = s.run(y28(1, 1)).await;
    assert_eq!((report.expired, report.extended, report.tasked), (0, 0, 0));
    assert_eq!(s.t.status(&lease).await, LeaseStatus::NoticeGiven);
    assert!(s.holding_charges(&lease).await.is_empty());
    assert!(s.t.notices.of_type(HOLDING_OVER).is_empty());
}

/// AC8: from `expired` the lease offers `record_notice`, `renew`, `amend` and
/// `force_terminate`; `expired` is not terminal.
#[renox::test]
async fn bl_prp_06_an_expired_lease_is_not_an_end() {
    let machine = Lease::machine();
    for name in ["record_notice", "renew", "amend", "force_terminate"] {
        let transition = machine.find(name).expect(name);
        assert!(
            transition.from_states().iter().any(|s| s == "expired"),
            "{name} leaves expired"
        );
    }
    let state = machine
        .states()
        .iter()
        .find(|s| s.name == "expired")
        .expect("the state");
    assert!(!state.terminal);
    // `expire` itself leaves only `renewal_due`.
    assert_eq!(machine.find("expire").unwrap().from_states(), ["renewal_due"]);
}

/// An expired lease is still a lease in force: the Ejari can be recorded and
/// the page keeps its actions.
#[renox::test]
async fn bl_prp_06_an_expired_lease_is_not_read_only() {
    let s = scene().await;
    let lease = s.year().await;
    s.run(y28(1, 1)).await;
    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/ejari", lease.id),
        &[("number", "EJ-1"), ("registered_on", "2028-01-02")],
    )
    .await
    .assert_ok();
    assert_eq!(
        s.t.reload(&lease).await.ejari_number.as_deref(),
        Some("EJ-1")
    );
    // The renewal can be drafted from `expired`.
    let draft = s.renewal_draft(&lease).await;
    assert_eq!(draft.previous_id, Some(lease.id));
}
