//! BL-EDU-18 (#453), the app's side: the 10 values of the money mechanisms the
//! app carries as tables (`edtech::money_defaults`), the refund of a package at
//! the single-session price, the tiers of the refund policy template and the
//! policies a new enrolment starts with (the catalogue defaults). What the
//! shared modules do with the other values is proved through the real host in
//! `host/tests/edtech_money.rs`.

mod support;

use edtech::Enrolment;
use edtech::money_defaults as money;
use edtech::refund::{RefundBasis, RefundInput, RefundOutcome, RefundPolicy, calculate};
use renox::prelude::*;
use support::{boot, day, next_weekday};

/// A course fee of AED 2,520 (with VAT) over 12 sessions, paid with the
/// registration fee of AED 105, an admin fee of AED 210.
fn course(sessions_held: i64, started: bool) -> RefundInput {
    RefundInput {
        basis: RefundBasis::Course,
        policy: money::refund_template(),
        price: 252_000,
        paid: 262_500,
        registration_fee: 10_500,
        admin_fee: 21_000,
        sessions_total: 12,
        sessions_held,
        started,
        pending_payment: false,
    }
}

/// Packages: a refund of 4 unused credits at a single-session price of AED 150
/// is AED 600, not counting the package discount for the credits used.
#[test]
fn edu_18_a_refund_of_4_unused_credits_at_aed_150_is_aed_600() {
    let input = RefundInput {
        basis: RefundBasis::Package {
            single_session_price: 15_000,
        },
        policy: money::refund_template(),
        price: 150_000,
        paid: 150_000,
        registration_fee: 0,
        admin_fee: 0,
        sessions_total: 10,
        sessions_held: 6,
        started: true,
        pending_payment: false,
    };
    let refund = calculate(&input);
    assert_eq!(refund.refund, 60_000, "AED 600.00");
    assert_eq!(refund.outcome, RefundOutcome::Package);
}

/// Catalogue: the refund policy template is the tiered one: before the start
/// 100% minus the registration fee; after the start the unused sessions pro
/// rata minus the admin fee; after 50% of the sessions no refund.
#[test]
fn edu_18_the_refund_template_has_the_three_tiers() {
    assert_eq!(money::refund_template().key, "standard");
    assert_eq!(money::REFUND_NO_REFUND_AFTER_PERCENT, 50);

    let before = calculate(&course(0, false));
    assert_eq!(before.outcome, RefundOutcome::BeforeStart);
    assert_eq!(
        before.refund,
        262_500 - 10_500,
        "all of it less the registration fee"
    );

    let after = calculate(&course(3, true));
    assert_eq!(after.outcome, RefundOutcome::AfterStart);
    // Used 3 of 12 sessions: AED 630; the admin and registration fees stay.
    assert_eq!(after.used_value, 63_000);
    assert_eq!(after.refund, 262_500 - 63_000 - 21_000 - 10_500);

    let at_half = calculate(&course(6, true));
    assert_eq!(
        at_half.outcome,
        RefundOutcome::AfterStart,
        "50% is still refunded"
    );
    let past = calculate(&course(7, true));
    assert_eq!(past.outcome, RefundOutcome::NoRefund);
    assert_eq!(past.refund, 0);
}

/// Catalogue: a new course enrolment starts with the tiered refund policy and
/// the absence policy (an unexcused absence consumes a credit, a notice 24
/// hours before excuses one).
#[renox::test]
async fn edu_18_a_new_enrolment_gets_the_tiered_refund_policy_and_the_absence_policy() {
    let t = boot().await;
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let batch = t.open_batch(&tutor, &room, next_weekday(day(7), 1)).await;
    let student = t.person(31, "Sara Student");

    let enrolment = t.enrol(student, &batch).await;

    let stored = Enrolment::find_or_404(t.db(), enrolment.id).await.unwrap();
    assert_eq!(stored.policy.0, RefundPolicy::default());
    assert_eq!(stored.policy.0, money::refund_template());
    assert!(stored.absence_consumes);
    assert_eq!(stored.absence_consumes, money::ABSENCE_CONSUMES_CREDIT);
    assert_eq!(stored.excuse_notice_hours, 24);
    assert_eq!(stored.excuse_notice_hours, money::EXCUSE_NOTICE_HOURS);
}

/// The values of the table the host reads: refunds retried once after 24 hours,
/// a pending payment checked after 30 minutes, memberships retried on days 1, 3
/// and 7 and suspended, a 30-day allowance shared with the freeze, cash always
/// an approval with no salary recovery, no corporate dispute in the portal, a
/// forecast alert below AED 25,000 for 13 weeks at 06:00, a budget warning at
/// 100%.
#[test]
fn edu_18_the_values_of_the_table() {
    assert_eq!(
        (money::REFUND_RETRY_HOURS, money::REFUND_MAX_RETRIES),
        (24, 1)
    );
    assert_eq!(money::PENDING_TIMEOUT_MINUTES, 30);
    assert_eq!(money::MEMBERSHIP_RETRY_DAYS, [1, 3, 7]);
    const {
        assert!(money::MEMBERSHIP_SUSPENDS);
    }
    assert_eq!(money::PAUSE_ALLOWANCE_DAYS, 30);
    assert_eq!(money::CASH_WRITE_OFF_LIMIT, 0);
    const {
        assert!(!money::SALARY_RECOVERY);
    }
    const {
        assert!(!money::PORTAL_DISPUTES);
    }
    assert_eq!(money::DISPUTE_ESCALATION_DAYS, 14);
    const {
        assert!(!money::SUSPEND_PACKAGE_WHILE_UNPAID);
    }
    assert_eq!(money::CASH_THRESHOLD, 2_500_000, "AED 25,000");
    assert_eq!(money::CASH_FORECAST_WEEKS, 13);
    assert_eq!(money::CASH_ALERT_TIME, "06:00");
    assert_eq!(money::CASH_ALERT_ROLES, ["owner", "accountant"]);
    assert_eq!(money::BUDGET_WARNING_PERCENT, 100);
    assert_eq!(money::BUDGET_CATEGORY_OWNER, "accountant");
    assert_eq!(money::BUDGET_APPROVER_ROLE, "owner");
    assert_eq!(money::DRAWINGS_APPROVER_ROLE, "owner");
    assert_eq!(money::WRITE_OFF_APPROVERS, ["centre_manager", "admin"]);
}
