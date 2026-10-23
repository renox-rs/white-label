//! The refund an enrolment gets when the student leaves (10 `EDU.REF.01`,
//! §4.3, §7.1 *Enrolment* `withdraw`).
//!
//! [`calculate`] is a pure function of what the policy, the fees and the
//! sessions say; it reads no database and no clock, so the same facts always
//! give the same answer. The service ([`crate::enrolments::withdraw`]) gathers
//! the facts and the machine stores the answer on the enrolment; the credit
//! note, the cancelled instalments and the refund payment follow from it
//! (BL-EDU-03).
//!
//! The default policy template (10 §13, BL-EDU-03 Terms):
//!
//! - before the start: everything paid, less the registration fee;
//! - after the start: everything paid, less the sessions used (price divided
//!   by the sessions, for the sessions `held` up to the withdrawal date,
//!   attended or not), less the admin fee and the registration fee;
//! - after more than 50% of the sessions are held: no refund.
//!
//! When the sessions used and the fees come to more than what was paid, the
//! difference is the amount owed (one invoice), whatever the 50% limit says;
//! it never includes the sessions still to come. A package is valued by
//! `wl-sales` at the single-session price without the package discount; a
//! membership by `wl-invoicing` pro rata: here they are only estimated or left
//! to those modules ([`RefundOutcome::Membership`]).
//!
//! All amounts are fils with VAT. The VAT share of a refund is
//! [`vat_share`]; half-up everywhere.

use serde::{Deserialize, Serialize};

/// The VAT rate of a course fee in basis points (5%).
pub const VAT_BP: i64 = 500;

/// What the policy accepted at enrolment says (stored with the enrolment, so
/// a later change of the template does not change a student's terms).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RefundPolicy {
    /// The name of the template, e.g. `standard`.
    pub key: String,
    /// The share of the sessions, in percent, after which nothing is refunded.
    pub no_refund_after_percent: i64,
}

impl Default for RefundPolicy {
    fn default() -> Self {
        RefundPolicy {
            key: "standard".to_owned(),
            no_refund_after_percent: 50,
        }
    }
}

/// What the enrolment is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefundBasis {
    /// A seat in a batch, a course fee.
    Course,
    /// A package of sessions: unused credits at the single-session price.
    Package {
        /// The price of one session without the package discount, in fils.
        single_session_price: i64,
    },
    /// A membership: `wl-invoicing` credits the unused part itself.
    Membership,
}

/// The facts [`calculate`] needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefundInput {
    /// What the enrolment is for.
    pub basis: RefundBasis,
    /// The policy accepted at enrolment.
    pub policy: RefundPolicy,
    /// The course fee after any discount, in fils.
    pub price: i64,
    /// What was paid in all (fee, registration fee), in fils.
    pub paid: i64,
    /// The registration fee, in fils; kept as revenue.
    pub registration_fee: i64,
    /// The admin fee, in fils; kept as revenue once the course started.
    pub admin_fee: i64,
    /// The sessions (or credits) the fee covers.
    pub sessions_total: i64,
    /// The sessions `held` up to the withdrawal date (or credits used).
    pub sessions_held: i64,
    /// Whether the batch started on or before the withdrawal date.
    pub started: bool,
    /// Whether the enrolment is still `pending_payment`: nothing is paid and
    /// nothing is credited.
    pub pending_payment: bool,
}

/// How the figures came about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefundOutcome {
    /// `pending_payment`: nothing was paid, nothing is credited or billed.
    NothingPaid,
    /// Before the start: the paid amount less the registration fee.
    BeforeStart,
    /// After the start, within the limit: the paid amount less the sessions
    /// used and the fees.
    AfterStart,
    /// After the limit: no refund.
    NoRefund,
    /// A package: the unused credits at the single-session price.
    Package,
    /// A membership: settled by the subscription's own pro rata credit.
    Membership,
    /// A batch `terminate`: the unused sessions pro rata, no admin fee, no
    /// no-refund tier, the registration fee not refunded.
    Termination,
    /// A batch `cancel`: everything paid, the registration fee included.
    Cancellation,
}

/// The result of [`calculate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refund {
    /// The credit note amount, in fils (with VAT).
    pub refund: i64,
    /// The invoice for the amount owed, in fils (with VAT); 0 when none.
    pub owed: i64,
    /// The value of the sessions used, in fils.
    pub used_value: i64,
    /// The fees that stay revenue (admin fee, registration fee), in fils.
    pub kept_fees: i64,
    /// The VAT share of [`refund`](Refund::refund), in fils.
    pub vat: i64,
    /// How the figures came about.
    pub outcome: RefundOutcome,
}

impl Refund {
    fn zero(outcome: RefundOutcome) -> Self {
        Refund {
            refund: 0,
            owed: 0,
            used_value: 0,
            kept_fees: 0,
            vat: 0,
            outcome,
        }
    }
}

/// `amount * num / den`, half-up, for non-negative values.
pub fn ratio(amount: i64, num: i64, den: i64) -> i64 {
    if den <= 0 || amount <= 0 || num <= 0 {
        return 0;
    }
    let n = i128::from(amount) * i128::from(num);
    let d = i128::from(den);
    i64::try_from((2 * n + d) / (2 * d)).unwrap_or(i64::MAX)
}

/// The VAT inside a gross amount at 5%, half-up: AED 1,785.00 holds AED 85.00.
pub fn vat_share(gross: i64) -> i64 {
    ratio(gross, VAT_BP, 10_000 + VAT_BP)
}

/// The amount before VAT of a gross amount at 5%, half-up.
pub fn net_of_gross(gross: i64) -> i64 {
    gross - vat_share(gross)
}

/// The refund and the amount owed when the student leaves (`EDU.REF.01`).
pub fn calculate(input: &RefundInput) -> Refund {
    if input.pending_payment {
        return Refund::zero(RefundOutcome::NothingPaid);
    }
    let paid = input.paid.max(0);
    match input.basis {
        RefundBasis::Membership => Refund::zero(RefundOutcome::Membership),
        RefundBasis::Package {
            single_session_price,
        } => {
            // The used credits are valued at the single-session price, so the
            // package discount is not given back for them.
            let used_value = input
                .sessions_held
                .max(0)
                .saturating_mul(single_session_price);
            let refund = (paid - used_value).max(0);
            Refund {
                refund,
                owed: 0,
                used_value,
                kept_fees: 0,
                vat: vat_share(refund),
                outcome: RefundOutcome::Package,
            }
        }
        RefundBasis::Course => {
            if !input.started {
                let refund = (paid - input.registration_fee).max(0);
                let owed = (input.registration_fee - paid).max(0);
                return Refund {
                    refund,
                    owed,
                    used_value: 0,
                    kept_fees: input.registration_fee.min(paid),
                    vat: vat_share(refund),
                    outcome: RefundOutcome::BeforeStart,
                };
            }
            let used_value = ratio(input.price, input.sessions_held, input.sessions_total);
            let kept_fees = input.admin_fee + input.registration_fee;
            let charges = used_value + kept_fees;
            let past_limit = input.sessions_held.saturating_mul(100)
                > input
                    .sessions_total
                    .saturating_mul(input.policy.no_refund_after_percent);
            let owed = (charges - paid).max(0);
            let refund = if past_limit {
                0
            } else {
                (paid - charges).max(0)
            };
            Refund {
                refund,
                owed,
                used_value,
                kept_fees,
                vat: vat_share(refund),
                outcome: if past_limit {
                    RefundOutcome::NoRefund
                } else {
                    RefundOutcome::AfterStart
                },
            }
        }
    }
}

/// Nothing was paid, nothing is credited or billed (`pending_payment`).
pub fn nothing_paid() -> Refund {
    Refund::zero(RefundOutcome::NothingPaid)
}

/// The refund of a batch `terminate` (10 §7.1 *Batch* `terminate`): what was
/// paid for the course (`paid` less the registration fee, which is only
/// refunded by a `cancel`) divided by the sessions, times the sessions not
/// used, half-up, never more than was paid; no admin fee and no no-refund tier.
/// Nothing was paid, nothing is refunded. Money in fils with VAT.
pub fn for_termination(
    paid: i64,
    registration_fee: i64,
    sessions_total: i64,
    sessions_held: i64,
) -> Refund {
    let course_paid = (paid - registration_fee.min(paid)).max(0);
    let unused = (sessions_total - sessions_held).clamp(0, sessions_total.max(0));
    let refund = if sessions_total <= 0 {
        0
    } else {
        ratio(course_paid, unused, sessions_total).min(course_paid)
    };
    Refund {
        refund,
        owed: 0,
        used_value: course_paid - refund,
        kept_fees: registration_fee.min(paid),
        vat: vat_share(refund),
        outcome: RefundOutcome::Termination,
    }
}

/// The refund of a batch `cancel` (10 §7.1 *Batch* `cancel`): everything paid,
/// the registration fee included; no policy tier and no fee is deducted.
pub fn for_cancellation(paid: i64) -> Refund {
    let refund = paid.max(0);
    Refund {
        refund,
        owed: 0,
        used_value: 0,
        kept_fees: 0,
        vat: vat_share(refund),
        outcome: RefundOutcome::Cancellation,
    }
}

/// The value that moves to the new enrolment when a student changes batch
/// (`transfer`, 10 §7.1): what was paid for the course (the registration fee
/// stays where it was paid) for the sessions not yet used, half-up, never
/// more than was paid. Before the start nothing is used and all of it moves.
pub fn carried_value(
    paid: i64,
    registration_fee: i64,
    sessions_total: i64,
    sessions_held: i64,
) -> i64 {
    let course_paid = (paid - registration_fee.min(paid)).max(0);
    if sessions_total <= 0 {
        return course_paid;
    }
    let unused = (sessions_total - sessions_held).clamp(0, sessions_total);
    ratio(course_paid, unused, sessions_total).min(course_paid)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn half_up_ratio_and_the_vat_inside_a_gross_amount() {
        assert_eq!(ratio(252_000, 3, 12), 63_000);
        assert_eq!(ratio(100, 1, 3), 33);
        assert_eq!(ratio(100, 2, 3), 67);
        assert_eq!(ratio(0, 1, 3), 0);
        assert_eq!(ratio(100, 1, 0), 0);
        assert_eq!(vat_share(178_500), 8_500);
        assert_eq!(net_of_gross(252_000), 240_000);
    }

    #[test]
    fn after_the_start_the_used_sessions_and_the_admin_fee_are_kept() {
        let r = calculate(&course(252_000, 3, true));
        assert_eq!(r.refund, 178_500);
        assert_eq!(r.vat, 8_500);
        assert_eq!(r.owed, 0);
        assert_eq!(r.outcome, RefundOutcome::AfterStart);
    }

    #[test]
    fn nothing_is_paid_back_after_half_the_course() {
        let r = calculate(&course(252_000, 7, true));
        assert_eq!((r.refund, r.owed), (0, 0));
        assert_eq!(r.outcome, RefundOutcome::NoRefund);
        // Exactly half is still within the limit.
        let half = calculate(&course(252_000, 6, true));
        assert_eq!(half.outcome, RefundOutcome::AfterStart);
        assert_eq!(half.refund, 252_000 - 126_000 - 10_500);
    }

    #[test]
    fn a_student_who_used_more_than_was_paid_owes_the_difference() {
        let r = calculate(&course(126_000, 8, true));
        assert_eq!(r.refund, 0);
        assert_eq!(r.owed, 52_500);
    }

    #[test]
    fn before_the_start_only_the_registration_fee_is_kept() {
        let mut input = course(262_500, 0, false);
        input.registration_fee = 10_500;
        let r = calculate(&input);
        assert_eq!(r.refund, 252_000);
        assert_eq!(r.owed, 0);
        assert_eq!(r.outcome, RefundOutcome::BeforeStart);
    }

    #[test]
    fn a_hold_that_never_paid_credits_and_bills_nothing() {
        let mut input = course(0, 0, false);
        input.pending_payment = true;
        input.registration_fee = 10_500;
        assert_eq!(calculate(&input), Refund::zero(RefundOutcome::NothingPaid));
    }

    #[test]
    fn a_package_gives_back_the_unused_credits_at_the_single_price() {
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

    #[test]
    fn a_membership_is_left_to_the_subscription() {
        let mut input = course(10_000, 1, true);
        input.basis = RefundBasis::Membership;
        assert_eq!(calculate(&input), Refund::zero(RefundOutcome::Membership));
    }

    #[test]
    fn a_termination_refunds_the_unused_sessions_pro_rata_without_fees() {
        // AED 2,520.00 paid for 12 sessions, 5 held: 7/12 = AED 1,470.00.
        let r = for_termination(252_000, 0, 12, 5);
        assert_eq!((r.refund, r.owed), (147_000, 0));
        assert_eq!(r.outcome, RefundOutcome::Termination);
        // Past half the course there is still a refund, and no admin fee.
        assert_eq!(for_termination(252_000, 0, 12, 9).refund, 63_000);
        // The registration fee is not refunded.
        assert_eq!(for_termination(262_500, 10_500, 12, 5).refund, 147_000);
        // Never above what was paid, nothing for nothing paid.
        assert_eq!(for_termination(0, 0, 12, 5).refund, 0);
        assert_eq!(for_termination(252_000, 0, 12, 0).refund, 252_000);
        assert_eq!(for_termination(252_000, 0, 0, 0).refund, 0);
    }

    #[test]
    fn a_cancellation_refunds_everything_paid_with_its_vat() {
        let r = for_cancellation(273_000);
        assert_eq!(r.refund, 273_000);
        assert_eq!(r.vat, 13_000);
        assert_eq!(r.outcome, RefundOutcome::Cancellation);
        assert_eq!(for_cancellation(-5).refund, 0);
    }

    #[test]
    fn what_moves_with_a_transfer_is_the_paid_value_of_the_unused_sessions() {
        // Nothing used: everything paid for the course moves.
        assert_eq!(carried_value(262_500, 10_500, 12, 0), 252_000);
        // Three of twelve used: nine twelfths move.
        assert_eq!(carried_value(252_000, 0, 12, 3), 189_000);
        // All used: nothing moves; never more than was paid.
        assert_eq!(carried_value(252_000, 0, 12, 12), 0);
        assert_eq!(carried_value(252_000, 0, 12, 20), 0);
        assert_eq!(carried_value(100, 500, 12, 0), 0);
        assert_eq!(carried_value(252_000, 0, 0, 0), 252_000);
    }
}
