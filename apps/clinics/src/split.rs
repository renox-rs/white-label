//! The charge split (05 `CLN.ENC.02`): how the covered part of a charge is
//! divided between the patient and the payer by the payer contract's terms.
//!
//! Decision (the terms are the contract's, `CLN.INS.01`, kept minimal):
//!
//! - The **deductible** is a fixed amount per visit. It is taken from the
//!   covered amounts in the order of the lines, the patient bearing it.
//! - The **copay** is a share of what is left after the deductible, in basis
//!   points (2000 = 20%), rounded half up to fils; the patient bears it and
//!   the payer the rest.
//! - A service the contract **excludes**, and the units no prior
//!   authorisation covers (they go self-pay with consent), are the patient's
//!   in full: they never reach the deductible or the copay.
//!
//! Every amount is fils; the patient's and the payer's parts of a line add
//! up to the line.

/// The contract's terms the split reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Terms {
    /// The patient's share of what the deductible leaves, in basis points.
    pub copay_bp: i64,
    /// The deductible of the visit, in fils.
    pub deductible: i64,
}

/// One line to divide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Part {
    /// The amount the payer's cover applies to, in fils.
    pub covered: i64,
    /// The amount the patient pays in full (self-pay units), in fils.
    pub self_pay: i64,
    /// The contract excludes the service: `covered` is the patient's too.
    pub excluded: bool,
}

/// Who pays what of one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Share {
    /// The patient's part, in fils.
    pub patient: i64,
    /// The payer's part, in fils.
    pub payer: i64,
}

impl Share {
    /// The whole line, in fils.
    pub fn total(&self) -> i64 {
        self.patient + self.payer
    }
}

/// `value` × `bp` ÷ 10,000, rounded half up.
fn basis_points(value: i64, bp: i64) -> i64 {
    let product = i128::from(value) * i128::from(bp);
    i64::try_from((2 * product + 10_000) / 20_000).unwrap_or(i64::MAX)
}

/// Divides `parts` by `terms`, in order.
pub fn split(parts: &[Part], terms: Terms) -> Vec<Share> {
    let copay_bp = terms.copay_bp.clamp(0, 10_000);
    let mut deductible = terms.deductible.max(0);
    parts
        .iter()
        .map(|part| {
            let mut share = Share {
                patient: part.self_pay,
                payer: 0,
            };
            if part.excluded {
                share.patient += part.covered;
                return share;
            }
            let taken = deductible.min(part.covered);
            deductible -= taken;
            let rest = part.covered - taken;
            let copay = basis_points(rest, copay_bp);
            share.patient += taken + copay;
            share.payer = rest - copay;
            share
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(covered: i64) -> Part {
        Part {
            covered,
            ..Part::default()
        }
    }

    #[test]
    fn a_copay_is_a_share_of_the_covered_amount() {
        let shares = split(
            &[part(30_000)],
            Terms {
                copay_bp: 2_000,
                deductible: 0,
            },
        );
        assert_eq!(
            shares,
            [Share {
                patient: 6_000,
                payer: 24_000
            }]
        );
    }

    #[test]
    fn the_deductible_is_taken_first_and_across_lines() {
        let shares = split(
            &[part(10_000), part(30_000)],
            Terms {
                copay_bp: 1_000,
                deductible: 15_000,
            },
        );
        // The first line is all deductible; the second loses AED 50 to it,
        // then 10% of the AED 250 left.
        assert_eq!(
            shares,
            [
                Share {
                    patient: 10_000,
                    payer: 0
                },
                Share {
                    patient: 5_000 + 2_500,
                    payer: 22_500
                }
            ]
        );
    }

    #[test]
    fn an_excluded_service_and_self_pay_units_are_the_patients_in_full() {
        let shares = split(
            &[
                Part {
                    covered: 8_000,
                    excluded: true,
                    ..Part::default()
                },
                Part {
                    covered: 10_000,
                    self_pay: 40_000,
                    excluded: false,
                },
            ],
            Terms {
                copay_bp: 0,
                deductible: 0,
            },
        );
        assert_eq!(
            shares,
            [
                Share {
                    patient: 8_000,
                    payer: 0
                },
                Share {
                    patient: 40_000,
                    payer: 10_000
                }
            ]
        );
    }

    #[test]
    fn the_parts_add_up_and_a_copay_rounds_half_up() {
        let shares = split(
            &[part(1_001)],
            Terms {
                copay_bp: 5_000,
                deductible: 0,
            },
        );
        assert_eq!(shares[0].patient, 501);
        assert_eq!(shares[0].total(), 1_001);
        // A copay outside the range is held to it.
        let all = split(
            &[part(100)],
            Terms {
                copay_bp: 99_999,
                deductible: -5,
            },
        );
        assert_eq!(all[0].patient, 100);
        assert_eq!(all[0].payer, 0);
    }
}
