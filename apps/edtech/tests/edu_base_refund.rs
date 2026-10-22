//! The refund calculation of 10 EdTech (base for #438, `EDU.REF.01`): a pure
//! function of the policy, the fees and the sessions. The figures are those of
//! the Done-when points of BL-EDU-03.

use edtech::refund::{
    Refund, RefundBasis, RefundInput, RefundOutcome, RefundPolicy, calculate, net_of_gross,
    ratio, vat_share,
};

/// A 12-session course at AED 2,520 (with VAT), admin fee AED 105.
fn course(paid: i64, held: i64, started: bool) -> RefundInput {
    RefundInput {
        basis: RefundBasis::Course,
        policy: RefundPolicy::default(),
        price: 252_000,
        paid,
        registration_fee: 0,
        admin_fee: 10_500,
        sessions_total: 12,
        sessions_held: held,
        started,
        pending_payment: false,
    }
}

/// Paid upfront, 3 sessions held, policy "after start": 2,520 - 3 x 210 - 105
/// = AED 1,785 refund, of which AED 85.00 is VAT.
#[test]
fn edu_base_refund_after_the_start_is_what_was_paid_less_used_sessions_and_admin_fee() {
    let r = calculate(&course(252_000, 3, true));
    assert_eq!(r.refund, 178_500);
    assert_eq!(r.vat, 8_500);
    assert_eq!(r.owed, 0);
    assert_eq!(r.used_value, 63_000);
    assert_eq!(r.kept_fees, 10_500);
    assert_eq!(r.outcome, RefundOutcome::AfterStart);
}

/// Two instalments of AED 1,260, the second not invoiced, 8 sessions held:
/// no refund and an invoice for 8 x 210 + 105 - 1,260 = AED 525.
#[test]
fn edu_base_refund_a_student_who_used_more_than_was_paid_owes_the_difference() {
    let r = calculate(&course(126_000, 8, true));
    assert_eq!(r.refund, 0);
    assert_eq!(r.owed, 52_500);
}

/// 7 of 12 sessions held (after 50%) and paid upfront: no refund, no invoice.
#[test]
fn edu_base_refund_nothing_after_half_the_course() {
    let r = calculate(&course(252_000, 7, true));
    assert_eq!((r.refund, r.owed), (0, 0));
    assert_eq!(r.outcome, RefundOutcome::NoRefund);
}

/// Exactly half is still within the limit.
#[test]
fn edu_base_refund_exactly_half_is_within_the_limit() {
    let r = calculate(&course(252_000, 6, true));
    assert_eq!(r.outcome, RefundOutcome::AfterStart);
    assert_eq!(r.refund, 252_000 - 126_000 - 10_500);
}

/// Before the start only the registration fee is kept.
#[test]
fn edu_base_refund_before_the_start_keeps_only_the_registration_fee() {
    let mut input = course(262_500, 0, false);
    input.registration_fee = 10_500;
    let r = calculate(&input);
    assert_eq!(r.refund, 252_000);
    assert_eq!(r.outcome, RefundOutcome::BeforeStart);
}

/// A seat that was never paid for credits and bills nothing.
#[test]
fn edu_base_refund_a_pending_payment_has_nothing_to_credit() {
    let mut input = course(0, 0, false);
    input.pending_payment = true;
    input.registration_fee = 10_500;
    let r = calculate(&input);
    assert_eq!((r.refund, r.owed), (0, 0));
    assert_eq!(r.outcome, RefundOutcome::NothingPaid);
}

/// A 10-credit package with 4 unused at AED 150 a session: AED 600.
#[test]
fn edu_base_refund_a_package_gives_back_unused_credits_at_the_single_price() {
    let input = RefundInput {
        basis: RefundBasis::Package {
            single_session_price: 15_000,
        },
        policy: RefundPolicy::default(),
        price: 150_000,
        paid: 150_000,
        registration_fee: 0,
        admin_fee: 0,
        sessions_total: 10,
        sessions_held: 6,
        started: true,
        pending_payment: false,
    };
    let r = calculate(&input);
    assert_eq!(r.refund, 60_000);
    assert_eq!(r.outcome, RefundOutcome::Package);
}

/// A membership is settled by the subscription's own credit, not repeated.
#[test]
fn edu_base_refund_a_membership_is_left_to_the_subscription() {
    let mut input = course(10_000, 1, true);
    input.basis = RefundBasis::Membership;
    let r = calculate(&input);
    assert_eq!(r.outcome, RefundOutcome::Membership);
    assert_eq!((r.refund, r.owed), (0, 0));
}

/// The same facts give the same answer, and the figures are half-up.
#[test]
fn edu_base_refund_is_pure_and_rounds_half_up() {
    let input = course(252_000, 5, true);
    let a: Refund = calculate(&input);
    let b: Refund = calculate(&input);
    assert_eq!(a, b);
    assert_eq!(ratio(100, 1, 3), 33);
    assert_eq!(ratio(100, 2, 3), 67);
    assert_eq!(vat_share(10_500), 500);
    assert_eq!(net_of_gross(10_500), 10_000);
}

/// The policy accepted at enrolment is stored with it; the default is 50%.
#[test]
fn edu_base_refund_a_stricter_policy_stops_the_refund_sooner() {
    let mut input = course(252_000, 4, true);
    assert_eq!(calculate(&input).outcome, RefundOutcome::AfterStart);
    input.policy.no_refund_after_percent = 25;
    assert_eq!(calculate(&input).outcome, RefundOutcome::NoRefund);
}
