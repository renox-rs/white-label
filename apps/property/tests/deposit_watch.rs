//! BL-PRP-16 (#266): `PRP.RUL.11`. Fourteen days after the deposit deductions
//! of a lease are published, with no line disputed, the accountant is told the
//! settlement may proceed; a dispute left fourteen days without an answer is
//! escalated to the property manager's manager.

mod support;

use property::deposit_watch::{self, DepositWatch};
use property::notices::{DEPOSIT_DISPUTE_UNANSWERED, DEPOSIT_READY};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::deposits::{
    DeductionKind, DepositDeductionsDisputed, DepositDeductionsPublished, DepositDisputeAnswered,
    DepositOutcome, DepositSettled, DisputeAnswer,
};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::payments::DepositId;
use wl_kernel::{COMPANY_ADMIN_ROLE, Subject};

use support::{ACCOUNTANT, AGENT, Booted, MANAGER, TENANT, aed, boot, d};

struct Scene {
    t: Booted,
    lease: property::Lease,
    manager: User,
    accountant: User,
    boss: User,
    admin: User,
}

async fn scene() -> Scene {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let accountant = t.user_in("Cara", ACCOUNTANT, building.id).await;
    let boss = t.user("Bo").await;
    let admin = t.user_with("Bader", COMPANY_ADMIN_ROLE).await;
    let lease = t.active_lease(&unit, &agent, &manager).await;
    Scene {
        t,
        lease,
        manager,
        accountant,
        boss,
        admin,
    }
}

const DEPOSIT: DepositId = DepositId(5);

impl Scene {
    fn source(&self) -> Subject {
        self.lease.agreement()
    }

    /// The deductions version `version` are published on `on`.
    async fn publish(&self, version: i32, on: NaiveDate) {
        self.t
            .state()
            .emit(DepositDeductionsPublished {
                deposit: DEPOSIT,
                source: self.source(),
                party: TENANT,
                version,
                kind: DeductionKind::Deductions,
                total: aed(1_500),
                published_on: on,
                window_ends_on: Some(on + renox::chrono::Duration::days(14)),
            })
            .await
            .unwrap();
    }

    /// The tenant disputes the deductions on `on`.
    async fn dispute(&self, on: NaiveDate) {
        self.t
            .state()
            .emit(DepositDeductionsDisputed {
                deposit: DEPOSIT,
                source: self.source(),
                party: TENANT,
                version: 1,
                lines: vec![1],
                raised_on: on,
            })
            .await
            .unwrap();
    }

    /// Staff answered; `open` says whether a line is still disputed.
    async fn answer(&self, open: bool) {
        self.t
            .state()
            .emit(DepositDisputeAnswered {
                deposit: DEPOSIT,
                source: self.source(),
                party: TENANT,
                version: 1,
                answer: DisputeAnswer::Upheld,
                new_version: None,
                still_open: open,
            })
            .await
            .unwrap();
    }

    async fn run(&self, on: NaiveDate) -> deposit_watch::Report {
        deposit_watch::run(self.t.state(), on).await.unwrap()
    }

    /// The users the messages of `type_key` went to.
    fn told(&self, type_key: &str) -> Vec<i64> {
        self.t
            .notices
            .of_type(type_key)
            .iter()
            .flat_map(|r| r.recipients.iter())
            .filter_map(|r| match r {
                NotifyRecipient::User { id } => Some(*id),
                _ => None,
            })
            .collect()
    }
}

/// AC1: deductions published 2027-03-01 and no dispute: the accountant is
/// notified on 2027-03-15, not before, and once.
#[renox::test]
async fn bl_prp_16_the_accountant_is_told_when_the_window_ends_without_a_dispute() {
    let s = scene().await;
    s.publish(1, d(3, 1)).await;

    assert_eq!(s.run(d(3, 14)).await.ready, 0);
    assert!(s.t.notices.of_type(DEPOSIT_READY).is_empty());
    assert_eq!(s.run(d(3, 15)).await.ready, 1);
    assert_eq!(s.told(DEPOSIT_READY), vec![s.accountant.id]);
    let request = &s.t.notices.of_type(DEPOSIT_READY)[0];
    assert_eq!(request.subject, s.lease.subject());
    assert_eq!(request.data["version"], 1);

    // Once per published version.
    assert_eq!(s.run(d(3, 16)).await.ready, 0);
    assert_eq!(s.t.notices.of_type(DEPOSIT_READY).len(), 1);
}

/// AC3: a revision published 2027-03-10 moves the accountant's notice to
/// 2027-03-24.
#[renox::test]
async fn bl_prp_16_a_new_version_restarts_the_count() {
    let s = scene().await;
    s.publish(1, d(3, 1)).await;
    assert_eq!(s.run(d(3, 9)).await.ready, 0);
    s.publish(2, d(3, 10)).await;

    assert_eq!(s.run(d(3, 15)).await.ready, 0);
    assert_eq!(s.run(d(3, 23)).await.ready, 0);
    assert_eq!(s.run(d(3, 24)).await.ready, 1);
    let sent = s.t.notices.of_type(DEPOSIT_READY);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].data["version"], 2);
    // The watch follows the latest version.
    let watch = DepositWatch::of_deposit(s.t.db(), DEPOSIT.0).await.unwrap().unwrap();
    assert_eq!((watch.version, watch.published_on), (2, d(3, 10)));
}

/// AC2: a dispute opened 2027-03-05 and not answered: the property manager's
/// manager is notified on 2027-03-19, once; the accountant is not told while
/// the dispute is open.
#[renox::test]
async fn bl_prp_16_an_unanswered_dispute_is_escalated_to_the_managers_manager() {
    let s = scene().await;
    s.t.staff.set_managers(s.manager.id, &[s.boss.id]);
    s.publish(1, d(3, 1)).await;
    s.dispute(d(3, 5)).await;

    assert_eq!(s.run(d(3, 15)).await.ready, 0);
    assert_eq!(s.run(d(3, 18)).await.escalated, 0);
    assert!(s.t.notices.of_type(DEPOSIT_DISPUTE_UNANSWERED).is_empty());
    assert_eq!(s.run(d(3, 19)).await.escalated, 1);
    assert_eq!(s.told(DEPOSIT_DISPUTE_UNANSWERED), vec![s.boss.id]);

    assert_eq!(s.run(d(3, 20)).await.escalated, 0);
    assert_eq!(s.t.notices.of_type(DEPOSIT_DISPUTE_UNANSWERED).len(), 1);
    assert!(s.t.notices.of_type(DEPOSIT_READY).is_empty());
}

/// Without a manager on the staff profile the company admin is told.
#[renox::test]
async fn bl_prp_16_without_a_manager_the_company_admin_is_told() {
    let s = scene().await;
    s.publish(1, d(3, 1)).await;
    s.dispute(d(3, 5)).await;

    assert_eq!(s.run(d(3, 19)).await.escalated, 1);
    assert_eq!(s.told(DEPOSIT_DISPUTE_UNANSWERED), vec![s.admin.id]);
}

/// A dispute that was answered is not escalated, and the accountant is told
/// once the window has passed; a new dispute counts from its own day.
#[renox::test]
async fn bl_prp_16_an_answered_dispute_is_not_escalated_and_a_new_one_counts_again() {
    let s = scene().await;
    s.publish(1, d(3, 1)).await;
    s.dispute(d(3, 5)).await;
    s.answer(false).await;

    let report = s.run(d(3, 19)).await;
    assert_eq!((report.escalated, report.ready), (0, 1));

    // A line still open does not end the dispute.
    s.dispute(d(3, 20)).await;
    s.answer(true).await;
    assert_eq!(s.run(d(4, 3)).await.escalated, 1);
    assert_eq!(s.told(DEPOSIT_DISPUTE_UNANSWERED).len(), 1);
}

/// While a legal case of type deposit dispute is open there is no escalation.
#[renox::test]
async fn bl_prp_16_a_deposit_dispute_legal_case_stops_the_escalation() {
    let s = scene().await;
    s.publish(1, d(3, 1)).await;
    s.dispute(d(3, 5)).await;
    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/legal-cases", s.lease.id),
        &[("case_type", "deposit_dispute"), ("reference", "RDC-9"), ("ticket", "new")],
    )
    .await
    .assert_ok();

    assert_eq!(s.run(d(3, 19)).await.escalated, 0);
    assert_eq!(s.run(d(6, 1)).await.escalated, 0);
    assert!(s.t.notices.of_type(DEPOSIT_DISPUTE_UNANSWERED).is_empty());
}

/// A settled deposit is left alone, and the deposit of something that is not
/// a lease is ignored.
#[renox::test]
async fn bl_prp_16_a_settled_deposit_and_other_sources_are_ignored() {
    let s = scene().await;
    s.publish(1, d(3, 1)).await;
    s.t.state()
        .emit(DepositSettled {
            deposit: DEPOSIT,
            source: s.source(),
            party: TENANT,
            outcome: DepositOutcome::Refunded,
            deductions: aed(1_500),
            refund_amount: aed(3_500),
            new_holder: None,
        })
        .await
        .unwrap();
    assert_eq!(s.run(d(3, 15)).await.ready, 0);

    s.t.state()
        .emit(DepositDeductionsPublished {
            deposit: DepositId(9),
            source: Subject::new("property.unit", 3),
            party: TENANT,
            version: 1,
            kind: DeductionKind::Deductions,
            total: aed(100),
            published_on: d(3, 1),
            window_ends_on: None,
        })
        .await
        .unwrap();
    assert!(DepositWatch::of_deposit(s.t.db(), 9).await.unwrap().is_none());
    assert!(s.t.notices.of_type(DEPOSIT_READY).is_empty());
}
