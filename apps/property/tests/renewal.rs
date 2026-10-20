//! BL-PRP-03 (#253): the renewal window opens 120 days before the end date
//! (`PRP.RUL.01`) and the notice-of-changed-terms deadline is guarded
//! (`PRP.RUL.09`, compliance-critical): warnings 14 and 3 days before it, the
//! current-terms-only marker the day after it, and an owner alert when the
//! run fails.

mod support;

use std::time::Duration;

use property::notices::{
    CURRENT_TERMS_ONLY, NOTICE_OF_CHANGED_TERMS, RENEWAL_DEADLINE, RENEWAL_DUE, RENEWAL_WINDOW,
};
use property::renewal::{self, RULE_DEADLINE};
use property::{Lease, LeaseStatus, NewLease, RenewalDecision, Unit};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::compliance::ComplianceAlert;
use wl_kernel::notify::{NoticeStatus, NotifyRecipient, NotifyRequest};

use support::{AGENT, Booted, MANAGER, TENANT, aed, boot, d};

/// The users a request went to.
fn users(request: &NotifyRequest) -> Vec<i64> {
    request
        .recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect()
}

/// A scene: a building with a property manager and a leasing agent.
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
    /// An `active` lease of a new unit `number` for the term `start` to
    /// `end`, with the contract's `notice` period (the 90 days when `None`).
    async fn active(
        &self,
        number: &str,
        start: NaiveDate,
        end: NaiveDate,
        notice: Option<i32>,
    ) -> Lease {
        let unit: Unit = self.t.unit(&self.building, number).await;
        let mut new = NewLease::new(unit.id, TENANT, start, end, aed(100_000), 4);
        new.contract_notice_days = notice;
        let lease = property::leases::create_draft(self.t.state(), None, new)
            .await
            .unwrap();
        self.t.submit(&self.agent, &lease).await.assert_ok();
        self.t.activate(&self.manager, &lease).await.assert_ok();
        let active = self.t.reload(&lease).await;
        assert_eq!(active.status, LeaseStatus::Active);
        active
    }

    /// The usual lease: 2027-01-01 to 2027-12-31.
    async fn year(&self) -> Lease {
        self.active("1204", d(1, 1), d(12, 31), None).await
    }

    /// Runs the daily rules for `day`.
    async fn run(&self, day: NaiveDate) -> renewal::Report {
        renewal::run(self.t.state(), day).await.unwrap()
    }

    /// Sets the renewal decision of `lease` by the page's action.
    async fn decide(&self, lease: &Lease, decision: &str) {
        self.t
            .post(
                &self.manager,
                &format!("/property/leases/{}/renewal-decision", lease.id),
                &[("decision", decision)],
            )
            .await
            .assert_ok();
    }

    /// The notice of changed terms of `lease` was delivered on `day`, as a
    /// staff proof says.
    fn proof(&self, lease: &Lease, day: NaiveDate) {
        self.t.notices.set_notice(
            &lease.subject(),
            NOTICE_OF_CHANGED_TERMS,
            NoticeStatus::ProofRecorded { delivered_on: day },
        );
    }

    /// Opens the page of `lease` as the property manager.
    async fn page(&self, lease: &Lease) -> renox::testing::TestResponse {
        self.t.app.acting_as(&self.manager);
        self.t
            .app
            .get(&format!("/property/leases/{}", lease.id))
            .await
    }
}

/// AC1: a lease ending 2027-12-31 is `renewal_due` on 2027-09-02, the manager
/// and the tenant are told, and the lease shows "notice deadline 2027-10-02".
#[renox::test]
async fn bl_prp_03_the_lease_enters_the_renewal_window_at_120_days() {
    let s = scene().await;
    let lease = s.year().await;

    let early = s.run(d(9, 1)).await;
    assert_eq!(early.flagged, 0);
    assert_eq!(s.t.status(&lease).await, LeaseStatus::Active);
    assert!(s.t.notices.of_type(RENEWAL_DUE).is_empty());

    let at_120 = s.run(d(9, 2)).await;
    assert_eq!(at_120.flagged, 1);
    let flagged = s.t.reload(&lease).await;
    assert_eq!(flagged.status, LeaseStatus::RenewalDue);
    assert_eq!(flagged.renewal_flagged_on, Some(d(9, 2)));

    let staff = s.t.notices.of_type(RENEWAL_DUE);
    assert_eq!(staff.len(), 1);
    assert_eq!(users(&staff[0]), vec![s.manager.id]);
    assert_eq!(staff[0].subject, lease.subject());
    assert_eq!(staff[0].data["deadline"], "2 Oct 2027");
    // The tenant gets a normal message, not the statutory notice.
    let tenant = s.t.notices.of_type(RENEWAL_WINDOW);
    assert_eq!(tenant.len(), 1);
    assert!(matches!(
        tenant[0].recipients.as_slice(),
        [NotifyRecipient::Party { id }] if *id == TENANT
    ));
    assert!(s.t.notices.of_type(NOTICE_OF_CHANGED_TERMS).is_empty());

    s.page(&lease).await.assert_ok().assert_see("2027-10-02");
    // A second run the same day, and the next, flag nothing again.
    assert_eq!(s.run(d(9, 2)).await.flagged, 0);
    assert_eq!(s.run(d(9, 3)).await.flagged, 0);
    assert_eq!(s.t.notices.of_type(RENEWAL_DUE).len(), 1);
}

/// AC2: with `undecided` the manager is warned on 2027-09-18 and 2027-09-29,
/// once each; the marker comes on 2027-10-03, not on the deadline day.
#[renox::test]
async fn bl_prp_03_an_undecided_lease_warns_at_14_and_3_days_then_gets_the_marker() {
    let s = scene().await;
    let lease = s.year().await;
    s.run(d(9, 2)).await;

    assert_eq!(s.run(d(9, 17)).await.warned, 0);
    let first = s.run(d(9, 18)).await;
    assert_eq!(first.warned, 1);
    // The same day again: one warning per offset.
    assert_eq!(s.run(d(9, 18)).await.warned, 0);
    assert_eq!(s.run(d(9, 28)).await.warned, 0);
    assert_eq!(s.run(d(9, 29)).await.warned, 1);
    assert_eq!(s.run(d(9, 30)).await.warned, 0);

    let warnings = s.t.notices.of_type(RENEWAL_DEADLINE);
    assert_eq!(warnings.len(), 2);
    assert_eq!(warnings[0].data["days"], 14);
    assert_eq!(warnings[1].data["days"], 3);
    assert_eq!(users(&warnings[0]), vec![s.manager.id]);
    assert_eq!(warnings[0].data["deadline"], "2 Oct 2027");

    // The deadline day itself still counts: no marker yet.
    assert_eq!(s.run(d(10, 2)).await.marked, 0);
    assert!(!s.t.reload(&lease).await.current_terms_only);
    s.page(&lease)
        .await
        .assert_ok()
        .assert_dont_see("unless the tenant signs");

    let marked = s.run(d(10, 3)).await;
    assert_eq!(marked.marked, 1);
    assert!(s.t.reload(&lease).await.current_terms_only);
    let told = s.t.notices.of_type(CURRENT_TERMS_ONLY);
    assert_eq!(told.len(), 1);
    assert_eq!(users(&told[0]), vec![s.manager.id]);
    s.page(&lease)
        .await
        .assert_ok()
        .assert_see("unless the tenant signs");

    // The marker is set once.
    assert_eq!(s.run(d(10, 4)).await.marked, 0);
    assert_eq!(s.t.notices.of_type(CURRENT_TERMS_ONLY).len(), 1);
    assert_eq!(s.t.notices.of_type(RENEWAL_DEADLINE).len(), 2);
}

/// AC3: `changed_terms` with a notice delivered on 2027-09-20 (proof recorded)
/// gets no warning and no marker, and the delivery date is kept on the lease.
#[renox::test]
async fn bl_prp_03_a_timely_notice_stops_the_warnings_and_the_marker() {
    let s = scene().await;
    let lease = s.year().await;
    s.run(d(9, 2)).await;
    s.decide(&lease, "changed_terms").await;
    s.proof(&lease, d(9, 20));

    for day in [d(9, 18), d(9, 20), d(9, 29), d(10, 2), d(10, 3), d(10, 4)] {
        let report = s.run(day).await;
        assert_eq!((report.warned, report.marked), (0, 0), "{day}");
    }
    let after = s.t.reload(&lease).await;
    assert!(!after.current_terms_only);
    assert_eq!(after.notice_of_changes_delivered_on, Some(d(9, 20)));
    assert!(s.t.notices.of_type(RENEWAL_DEADLINE).is_empty());
}

/// A notice delivered on the deadline day counts.
#[renox::test]
async fn bl_prp_03_a_notice_delivered_on_the_deadline_day_counts() {
    let s = scene().await;
    let lease = s.year().await;
    s.run(d(9, 2)).await;
    s.decide(&lease, "changed_terms").await;
    s.proof(&lease, d(10, 2));
    assert_eq!(s.run(d(10, 3)).await.marked, 0);
    assert!(!s.t.reload(&lease).await.current_terms_only);
}

/// AC4: `changed_terms` whose notice message gave up, without a proof by
/// 2027-10-02: the lease shows "current terms only" from 2027-10-03; the
/// warning at 3 days says the notice was not delivered; a proof recorded
/// later is stored and does not clear the marker.
#[renox::test]
async fn bl_prp_03_a_notice_that_gave_up_does_not_count_and_the_marker_stays() {
    let s = scene().await;
    let lease = s.year().await;
    s.run(d(9, 2)).await;
    s.decide(&lease, "changed_terms").await;
    // The property manager sends the statutory notice; the message gives up.
    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/changed-terms-notice", lease.id),
        &[],
    )
    .await
    .assert_ok();
    let sent = s.t.notices.of_type(NOTICE_OF_CHANGED_TERMS);
    assert_eq!(sent.len(), 1);
    assert!(matches!(
        sent[0].recipients.as_slice(),
        [NotifyRecipient::Party { id }] if *id == TENANT
    ));
    assert_eq!(
        s.t.reload(&lease).await.notice_of_changes_sent_on,
        Some(support::day(0))
    );

    s.run(d(9, 18)).await;
    s.run(d(9, 29)).await;
    let warnings = s.t.notices.of_type(RENEWAL_DEADLINE);
    assert_eq!(warnings.len(), 2);
    assert!(
        warnings[1].data["problem"]
            .as_str()
            .unwrap()
            .contains("notice not delivered"),
        "{}",
        warnings[1].data["problem"]
    );
    assert_eq!(s.run(d(10, 2)).await.marked, 0);
    assert_eq!(s.run(d(10, 3)).await.marked, 1);
    assert!(s.t.reload(&lease).await.current_terms_only);
    s.page(&lease)
        .await
        .assert_ok()
        .assert_see("unless the tenant signs");

    // Proof recorded later is stored; the marker stays.
    s.proof(&lease, d(10, 4));
    s.run(d(10, 6)).await;
    let after = s.t.reload(&lease).await;
    assert_eq!(after.notice_of_changes_delivered_on, Some(d(10, 4)));
    assert!(after.current_terms_only);
}

/// AC5: with `same_terms` there is no warning and no marker; neither with
/// `not_renewing`.
#[renox::test]
async fn bl_prp_03_same_terms_and_not_renewing_need_no_notice() {
    let s = scene().await;
    let lease = s.year().await;
    let other = s.active("1205", d(1, 1), d(12, 31), None).await;
    s.run(d(9, 2)).await;
    s.decide(&lease, "same_terms").await;
    s.decide(&other, "not_renewing").await;
    assert_eq!(
        s.t.reload(&lease).await.renewal_decision,
        RenewalDecision::SameTerms
    );
    for day in [d(9, 18), d(9, 29), d(10, 2), d(10, 3), d(10, 4)] {
        let report = s.run(day).await;
        assert_eq!((report.warned, report.marked), (0, 0), "{day}");
    }
    assert!(s.t.notices.of_type(RENEWAL_DEADLINE).is_empty());
    assert!(s.t.notices.of_type(CURRENT_TERMS_ONLY).is_empty());
    assert!(!s.t.reload(&lease).await.current_terms_only);
    assert!(!s.t.reload(&other).await.current_terms_only);
}

/// AC6: a contract notice period of 60 days, ending 2027-12-31: the deadline
/// is 2027-11-01; the window still opens at 120 days.
#[renox::test]
async fn bl_prp_03_a_contract_notice_of_60_days_moves_the_deadline() {
    let s = scene().await;
    let lease = s.active("1204", d(1, 1), d(12, 31), Some(60)).await;
    assert_eq!(s.run(d(9, 1)).await.flagged, 0);
    assert_eq!(s.run(d(9, 2)).await.flagged, 1);
    s.page(&lease).await.assert_ok().assert_see("2027-11-01");
    // The marker comes on 2027-11-02.
    assert_eq!(s.run(d(11, 1)).await.marked, 0);
    assert_eq!(s.run(d(11, 2)).await.marked, 1);
}

/// A contract notice longer than 90 days flags a month before its deadline:
/// 120 days of notice flags at 150 days.
#[renox::test]
async fn bl_prp_03_a_long_contract_notice_opens_the_window_earlier() {
    let s = scene().await;
    let lease = s.active("1204", d(1, 1), d(12, 31), Some(120)).await;
    assert_eq!(s.run(d(8, 2)).await.flagged, 0);
    assert_eq!(s.run(d(8, 3)).await.flagged, 1);
    assert_eq!(s.t.status(&lease).await, LeaseStatus::RenewalDue);
}

/// AC7: a 60-day lease 2027-06-01 to 2027-07-30 (contract notice 90 days,
/// deadline 2027-05-01) activated on 2027-06-01: on 2027-06-02 it is
/// `renewal_due`, shows "current terms only" and the manager is told once; no
/// 14- or 3-day warning.
#[renox::test]
async fn bl_prp_03_a_lease_flagged_after_its_deadline_gets_the_marker_at_once() {
    let s = scene().await;
    let lease = s.active("1204", d(6, 1), d(7, 30), Some(90)).await;
    assert_eq!(renewal::notice_deadline(&lease), d(5, 1));

    let report = s.run(d(6, 2)).await;
    assert_eq!((report.flagged, report.marked, report.warned), (1, 1, 0));
    let after = s.t.reload(&lease).await;
    assert_eq!(after.status, LeaseStatus::RenewalDue);
    assert!(after.current_terms_only);
    assert_eq!(s.t.notices.of_type(CURRENT_TERMS_ONLY).len(), 1);
    assert!(s.t.notices.of_type(RENEWAL_DEADLINE).is_empty());
    s.page(&lease)
        .await
        .assert_ok()
        .assert_see("unless the tenant signs");

    // Nothing more on the days after.
    for day in [d(6, 3), d(7, 1), d(7, 15)] {
        let again = s.run(day).await;
        assert_eq!((again.warned, again.marked), (0, 0), "{day}");
    }
    assert_eq!(s.t.notices.of_type(CURRENT_TERMS_ONLY).len(), 1);
}

/// A lease activated so late that only the 3-day warning is still ahead gets
/// that one and not the 14-day one.
#[renox::test]
async fn bl_prp_03_warnings_for_offsets_already_passed_are_not_sent() {
    let s = scene().await;
    // End 2027-12-31, deadline 2027-10-02; flagged on 2027-09-25.
    let lease = s.year().await;
    s.run(d(9, 25)).await;
    assert_eq!(s.t.reload(&lease).await.renewal_flagged_on, Some(d(9, 25)));
    assert_eq!(s.run(d(9, 26)).await.warned, 0);
    assert_eq!(s.run(d(9, 29)).await.warned, 1);
    let warnings = s.t.notices.of_type(RENEWAL_DEADLINE);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].data["days"], 3);
}

/// Only an `active` lease is flagged, and only a `renewal_due` one is
/// warned.
#[renox::test]
async fn bl_prp_03_other_states_are_left_alone() {
    let s = scene().await;
    let unit = s.t.unit(&s.building, "1300").await;
    let draft = NewLease::new(unit.id, TENANT, d(1, 1), d(12, 31), aed(100_000), 4);
    let draft = property::leases::create_draft(s.t.state(), None, draft)
        .await
        .unwrap();
    let report = s.run(d(10, 4)).await;
    assert_eq!((report.flagged, report.warned, report.marked), (0, 0, 0));
    assert_eq!(s.t.status(&draft).await, LeaseStatus::Draft);
}

/// AC8: `PRP.RUL.09` is compliance-critical: a run that fails five times
/// alerts the company admin; a run for a date that has one does not run
/// twice.
#[renox::test]
async fn bl_prp_03_a_failed_run_alerts_the_company_admin() {
    let s = scene().await;
    let admin = s.t.user_with("Ada", wl_kernel::COMPANY_ADMIN_ROLE).await;
    assert!(wl_kernel::compliance::is_critical(RULE_DEADLINE));
    s.year().await;
    s.run(d(9, 2)).await;
    s.t.notices.fail_on(Some(RENEWAL_DEADLINE));

    let app = &s.t.app;
    for wait in [60_u64, 300, 900, 3600] {
        app.at_travelled_time(renewal::run(s.t.state(), d(9, 18)))
            .await
            .unwrap();
        app.travel(Duration::from_secs(wait));
    }
    app.at_travelled_time(renewal::run(s.t.state(), d(9, 18)))
        .await
        .unwrap();

    let alerts = ComplianceAlert::open(s.t.db()).await.unwrap();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0].rule, RULE_DEADLINE);
    assert!(
        s.t.notices
            .all()
            .iter()
            .any(|r| users(r).contains(&admin.id)),
        "the company admin is told"
    );
}

/// The decision and the notice are the property manager's: a leasing agent is
/// refused, a lease not in force is refused, and the notice needs the
/// decision `changed_terms`.
#[renox::test]
async fn bl_prp_03_the_decision_and_the_notice_are_the_managers() {
    let s = scene().await;
    let lease = s.year().await;
    let path = format!("/property/leases/{}/renewal-decision", lease.id);
    s.t.post(&s.agent, &path, &[("decision", "same_terms")])
        .await
        .assert_forbidden();
    s.t.post(&s.manager, &path, &[("decision", "maybe")])
        .await
        .assert_status(400);

    let notice = format!("/property/leases/{}/changed-terms-notice", lease.id);
    // The decision is still undecided: no notice.
    let refused = s.t.post(&s.manager, &notice, &[]).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("changed terms"),
        "{}",
        refused.text()
    );
    s.t.post(&s.agent, &notice, &[]).await.assert_forbidden();

    s.decide(&lease, "changed_terms").await;
    s.t.post(&s.manager, &notice, &[]).await.assert_ok();
    // Once delivered, it is not sent again.
    s.proof(&lease, d(3, 1));
    let again = s.t.post(&s.manager, &notice, &[]).await;
    again.assert_status(422);
    assert!(again.text().contains("delivered"), "{}", again.text());
    assert_eq!(s.t.notices.of_type(NOTICE_OF_CHANGED_TERMS).len(), 1);

    // A draft has no renewal decision.
    let unit = s.t.unit(&s.building, "1400").await;
    let new = NewLease::new(unit.id, TENANT, d(1, 1), d(12, 31), aed(100_000), 4);
    let draft = property::leases::create_draft(s.t.state(), None, new)
        .await
        .unwrap();
    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/renewal-decision", draft.id),
        &[("decision", "same_terms")],
    )
    .await
    .assert_status(422);
}
