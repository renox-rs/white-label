//! UAE VAT arithmetic (wl-invoicing §10, §5.9): line amounts, half-up
//! rounding to fils, AED VAT for a foreign-currency invoice, totals per VAT
//! category ([`VatTotals`]) and the VAT share of a bad-debt write-off
//! ([`bad_debt_vat_share`]).
//!
//! Everything is integer arithmetic on fils (widened to `i128` while
//! multiplying), never `f64`. Rounding is half-up (half away from zero for a
//! negative amount) at each documented step: a line's net, a line's VAT, a
//! line's AED VAT, and each category's bad-debt share.

use serde::Serialize;
use wl_kernel::invoicing::{DEFAULT_CURRENCY, VatCategory};

/// Basis points in one: a rate of 500 bp is 5%.
const BP: i128 = 10_000;
/// Thousandths in one unit of quantity.
const MILLI: i128 = 1_000;
/// The scale of an exchange rate: 1,000,000 = 1.
pub const RATE_SCALE: i64 = 1_000_000;

/// `num ÷ den` rounded half-up (half away from zero when negative).
///
/// # Panics
///
/// When `den` is 0, or the result does not fit an `i64`.
pub fn div_round_half_up(num: i128, den: i128) -> i64 {
    assert!(den != 0, "division by zero");
    let (num, den) = if den < 0 { (-num, -den) } else { (num, den) };
    // floor((2·|num| + den) ÷ 2·den) is |num| ÷ den with `x.5` rounded up.
    let magnitude = (2 * num.abs() + den) / (2 * den);
    let rounded = if num < 0 { -magnitude } else { magnitude };
    i64::try_from(rounded).expect("the amount fits an i64")
}

/// The net of a line, in fils: quantity × unit price, rounded half-up to
/// fils, minus the discount.
pub fn line_net(quantity_milli: i64, unit_price: i64, discount: i64) -> i64 {
    div_round_half_up(i128::from(quantity_milli) * i128::from(unit_price), MILLI) - discount
}

/// The VAT of a line with net `net` fils in `category`, rounded half-up to
/// fils per line.
pub fn line_vat(net: i64, category: VatCategory) -> i64 {
    div_round_half_up(i128::from(net) * i128::from(category.rate_bp()), BP)
}

/// `amount` fils of the invoice's currency in AED fils at `rate_micro`
/// (rate to AED × [`RATE_SCALE`]), rounded half-up.
pub fn to_aed(amount: i64, rate_micro: i64) -> i64 {
    div_round_half_up(
        i128::from(amount) * i128::from(rate_micro),
        i128::from(RATE_SCALE),
    )
}

/// The rate to AED of an invoice in `currency`: 1 for AED, else the stored
/// rate (`None` while it is not set).
pub fn aed_rate(currency: &str, exchange_rate_micro: Option<i64>) -> Option<i64> {
    if currency.eq_ignore_ascii_case(DEFAULT_CURRENCY) {
        Some(RATE_SCALE)
    } else {
        exchange_rate_micro
    }
}

/// What can still be credited on an invoice line of net `line_net` after
/// `credited` fils were credited by issued credit notes (never below 0).
pub fn remaining_credit(line_net: i64, credited: i64) -> i64 {
    (line_net - credited).max(0)
}

/// The computed amounts of one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LineAmounts {
    /// The VAT treatment.
    pub category: VatCategory,
    /// The rate charged, in basis points.
    pub rate_bp: i64,
    /// Quantity × unit price − discount, in fils.
    pub net: i64,
    /// The line VAT, in fils.
    pub vat: i64,
    /// The line VAT in AED fils; `None` while the rate is unknown.
    pub vat_aed: Option<i64>,
}

impl LineAmounts {
    /// The amounts of a line, at `rate_micro` to AED when known.
    pub fn compute(
        quantity_milli: i64,
        unit_price: i64,
        discount: i64,
        category: VatCategory,
        rate_micro: Option<i64>,
    ) -> Self {
        let net = line_net(quantity_milli, unit_price, discount);
        let vat = line_vat(net, category);
        LineAmounts {
            category,
            rate_bp: category.rate_bp(),
            net,
            vat,
            vat_aed: rate_micro.map(|rate| to_aed(vat, rate)),
        }
    }

    /// The amounts of a credit line of `net` fils in `category`: the VAT is
    /// that net × the category rate, rounded half-up, so crediting a whole
    /// invoice line gives exactly the line's VAT.
    pub fn for_net(net: i64, category: VatCategory, rate_micro: Option<i64>) -> Self {
        let vat = line_vat(net, category);
        LineAmounts {
            category,
            rate_bp: category.rate_bp(),
            net,
            vat,
            vat_aed: rate_micro.map(|rate| to_aed(vat, rate)),
        }
    }

    /// Net plus VAT, in fils.
    pub fn gross(&self) -> i64 {
        self.net + self.vat
    }
}

/// The totals of one VAT category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CategoryTotal {
    /// The VAT treatment.
    pub category: VatCategory,
    /// The net of its lines, in fils.
    pub net: i64,
    /// The VAT of its lines, in fils.
    pub vat: i64,
    /// The VAT of its lines in AED fils; `None` while the rate is unknown.
    pub vat_aed: Option<i64>,
}

/// An invoice's totals, overall and per VAT category (the sums of the
/// rounded line amounts).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct VatTotals {
    /// One entry per category that has a line, in [`VatCategory::ALL`]
    /// order.
    pub categories: Vec<CategoryTotal>,
    /// The net total, in fils.
    pub net: i64,
    /// The VAT total, in fils.
    pub vat: i64,
    /// Net plus VAT, in fils.
    pub gross: i64,
    /// The VAT total in AED fils; `None` while the rate is unknown.
    pub vat_aed: Option<i64>,
}

impl VatTotals {
    /// The totals of `lines`.
    pub fn of<'a>(lines: impl IntoIterator<Item = &'a LineAmounts>) -> Self {
        let lines: Vec<&LineAmounts> = lines.into_iter().collect();
        let sum_aed = |of: &[&LineAmounts]| -> Option<i64> {
            of.iter().map(|l| l.vat_aed).sum::<Option<i64>>()
        };
        let categories = VatCategory::ALL
            .iter()
            .filter_map(|&category| {
                let of: Vec<&LineAmounts> = lines
                    .iter()
                    .copied()
                    .filter(|l| l.category == category)
                    .collect();
                (!of.is_empty()).then(|| CategoryTotal {
                    category,
                    net: of.iter().map(|l| l.net).sum(),
                    vat: of.iter().map(|l| l.vat).sum(),
                    vat_aed: sum_aed(&of),
                })
            })
            .collect();
        let net = lines.iter().map(|l| l.net).sum();
        let vat = lines.iter().map(|l| l.vat).sum();
        VatTotals {
            categories,
            net,
            vat,
            gross: net + vat,
            vat_aed: sum_aed(&lines),
        }
    }

    /// The totals of `category`, if a line has it.
    pub fn category(&self, category: VatCategory) -> Option<&CategoryTotal> {
        self.categories.iter().find(|c| c.category == category)
    }
}

/// The VAT share of a bad-debt write-off, in AED fils (§5.9 step 3): for
/// each category that carries VAT, `written_off × category AED VAT ÷ gross
/// total`, rounded half-up to fils per category, then summed. Zero-rated,
/// exempt, out-of-scope and reverse-charge lines give no relief.
///
/// `written_off` and `totals.gross` are in the invoice's currency; the
/// share is in AED. `None` while a category's AED VAT is unknown (no rate).
pub fn bad_debt_vat_share(written_off: i64, totals: &VatTotals) -> Option<i64> {
    if totals.gross == 0 {
        return Some(0);
    }
    totals
        .categories
        .iter()
        .filter(|c| c.category.carries_vat())
        .map(|c| {
            c.vat_aed.map(|vat_aed| {
                div_round_half_up(
                    i128::from(written_off) * i128::from(vat_aed),
                    i128::from(totals.gross),
                )
            })
        })
        .sum()
}

/// The VAT share of one recovery on a written-off invoice, in AED fils
/// (#66 A5): the share of everything recovered after it less the share of
/// what was recovered before, both by [`bad_debt_vat_share`]. The shares of
/// several part recoveries add up to the relief exactly. `None` while a
/// category's AED VAT is unknown.
pub fn recovery_share(recovered_before: i64, amount: i64, totals: &VatTotals) -> Option<i64> {
    let after = bad_debt_vat_share(recovered_before + amount, totals)?;
    let before = bad_debt_vat_share(recovered_before, totals)?;
    Some(after - before)
}

#[cfg(test)]
mod tests {
    use super::*;
    use VatCategory::{Exempt, ReverseCharge, Standard, ZeroRated};

    #[test]
    fn a_credit_line_has_vat_from_its_own_net() {
        let part = LineAmounts::for_net(20_000, Standard, Some(RATE_SCALE));
        assert_eq!((part.net, part.vat, part.gross()), (20_000, 1_000, 21_000));
        assert_eq!(part.vat_aed, Some(1_000));
        let zero = LineAmounts::for_net(50_000, ZeroRated, None);
        assert_eq!((zero.vat, zero.vat_aed), (0, None));
        // A whole line credited gives the line's VAT.
        let line = aed(3_000, 33_333, Standard);
        let whole = LineAmounts::for_net(line.net, Standard, Some(RATE_SCALE));
        assert_eq!(whole, line);
    }

    #[test]
    fn remaining_credit_is_the_net_not_yet_credited() {
        assert_eq!(remaining_credit(100_000, 0), 100_000);
        assert_eq!(remaining_credit(100_000, 20_000), 80_000);
        assert_eq!(remaining_credit(100_000, 100_000), 0);
        assert_eq!(remaining_credit(100_000, 120_000), 0);
    }

    fn aed(quantity_milli: i64, unit_price: i64, category: VatCategory) -> LineAmounts {
        LineAmounts::compute(
            quantity_milli,
            unit_price,
            0,
            category,
            aed_rate("AED", None),
        )
    }

    #[test]
    fn rounding_is_half_up_and_symmetric() {
        assert_eq!(div_round_half_up(5, 10), 1);
        assert_eq!(div_round_half_up(4, 10), 0);
        assert_eq!(div_round_half_up(15, 10), 2);
        assert_eq!(div_round_half_up(14, 10), 1);
        assert_eq!(div_round_half_up(-5, 10), -1);
        assert_eq!(div_round_half_up(-4, 10), 0);
        assert_eq!(div_round_half_up(5, -10), -1);
        assert_eq!(div_round_half_up(1, 3), 0);
        assert_eq!(div_round_half_up(2, 3), 1);
        assert_eq!(div_round_half_up(3, 2), 2);
        assert_eq!(div_round_half_up(0, 7), 0);
    }

    #[test]
    fn line_vat_rounds_half_up_per_line() {
        // 0.10 × 5% = 0.005 → 0.01.
        assert_eq!(line_vat(10, Standard), 1);
        // 0.09 × 5% = 0.0045 → 0.00.
        assert_eq!(line_vat(9, Standard), 0);
        assert_eq!(line_vat(100_000, Standard), 5_000);
        for category in [ZeroRated, Exempt, ReverseCharge, VatCategory::OutOfScope] {
            assert_eq!(line_vat(100_000, category), 0);
        }
    }

    #[test]
    fn a_line_net_is_quantity_times_price_minus_discount() {
        // 2.5 × 12.34 = 30.85, less 0.85.
        assert_eq!(line_net(2_500, 1_234, 85), 3_000);
        // 0.333 × 1.00 = 0.333 → 0.33.
        assert_eq!(line_net(333, 100, 0), 33);
        let line = LineAmounts::compute(2_000, 25_000, 5_000, Standard, None);
        assert_eq!((line.net, line.vat, line.gross()), (45_000, 2_250, 47_250));
        assert_eq!(line.vat_aed, None);
    }

    #[test]
    fn totals_per_category_sum_the_rounded_lines() {
        // AC2: 1,000.00 standard + 500.00 zero rated.
        let lines = [aed(1_000, 100_000, Standard), aed(1_000, 50_000, ZeroRated)];
        let totals = VatTotals::of(&lines);
        assert_eq!(
            (totals.net, totals.vat, totals.gross),
            (150_000, 5_000, 155_000)
        );
        assert_eq!(totals.vat_aed, Some(5_000));
        let standard = totals.category(Standard).unwrap();
        assert_eq!((standard.net, standard.vat), (100_000, 5_000));
        let zero = totals.category(ZeroRated).unwrap();
        assert_eq!((zero.net, zero.vat), (50_000, 0));
        assert!(totals.category(Exempt).is_none());
        let order: Vec<_> = totals.categories.iter().map(|c| c.category).collect();
        assert_eq!(order, [Standard, ZeroRated]);
        // Two 0.10 lines: 0.01 VAT each, so 0.02 — not 5% of 0.20 rounded once.
        let small = [aed(1_000, 10, Standard), aed(1_000, 10, Standard)];
        assert_eq!(VatTotals::of(&small).vat, 2);
    }

    #[test]
    fn foreign_currency_lines_carry_their_aed_vat() {
        // USD 1,000.00 standard at 3.6725: VAT USD 50.00 = AED 183.625 → 183.63.
        let rate = aed_rate("USD", Some(3_672_500));
        assert_eq!(rate, Some(3_672_500));
        let line = LineAmounts::compute(1_000, 100_000, 0, Standard, rate);
        assert_eq!((line.vat, line.vat_aed), (5_000, Some(18_363)));
        assert_eq!(aed_rate("USD", None), None);
        assert_eq!(aed_rate("aed", None), Some(RATE_SCALE));
        let unknown = LineAmounts::compute(1_000, 100_000, 0, Standard, None);
        assert_eq!(VatTotals::of(&[line, unknown]).vat_aed, None);
    }

    #[test]
    fn bad_debt_share_of_the_a14_examples() {
        let standard_only = VatTotals::of(&[aed(1_000, 100_000, Standard)]);
        assert_eq!(standard_only.gross, 105_000);
        // 1,050.00 written off → 50.00.
        assert_eq!(bad_debt_vat_share(105_000, &standard_only), Some(5_000));
        // 630.00 of 1,050.00 (420.00 paid) → 30.00.
        assert_eq!(bad_debt_vat_share(63_000, &standard_only), Some(3_000));
        // 333.33 of 1,050.00 → 15.8728 → 15.87.
        assert_eq!(bad_debt_vat_share(33_333, &standard_only), Some(1_587));
        // 775.00 of 1,550.00 (1,000 standard + 500 zero rated) → 25.00.
        let mixed = VatTotals::of(&[aed(1_000, 100_000, Standard), aed(1_000, 50_000, ZeroRated)]);
        assert_eq!(mixed.gross, 155_000);
        assert_eq!(bad_debt_vat_share(77_500, &mixed), Some(2_500));
    }

    #[test]
    fn recovery_shares_add_up_to_the_relief() {
        let totals = VatTotals::of(&[aed(1_000, 100_000, Standard)]);
        // 525.00 then 525.00 → 25.00 and 25.00.
        assert_eq!(recovery_share(0, 52_500, &totals), Some(2_500));
        assert_eq!(recovery_share(52_500, 52_500, &totals), Some(2_500));
        // 333.33 then 716.67 → 15.87 and 34.13 (50.00 in all).
        assert_eq!(recovery_share(0, 33_333, &totals), Some(1_587));
        assert_eq!(recovery_share(33_333, 71_667, &totals), Some(3_413));
        // No VAT, or an unknown rate.
        let exempt = VatTotals::of(&[aed(1_000, 100_000, Exempt)]);
        assert_eq!(recovery_share(0, 50_000, &exempt), Some(0));
        let unknown = VatTotals::of(&[LineAmounts::compute(1_000, 100_000, 0, Standard, None)]);
        assert_eq!(recovery_share(0, 50_000, &unknown), None);
    }

    #[test]
    fn lines_without_vat_give_no_relief() {
        let exempt = VatTotals::of(&[
            aed(1_000, 100_000, Exempt),
            aed(1_000, 100_000, ReverseCharge),
        ]);
        assert_eq!(bad_debt_vat_share(200_000, &exempt), Some(0));
        assert_eq!(bad_debt_vat_share(0, &VatTotals::default()), Some(0));
    }

    #[test]
    fn a_foreign_currency_share_uses_the_aed_vat() {
        // USD 1,050.00 gross, VAT USD 50.00 = AED 183.63; USD 525.00 written off
        // → 183.63 × 525 ÷ 1,050 = 91.815 → AED 91.82.
        let rate = Some(3_672_500);
        let totals = VatTotals::of(&[LineAmounts::compute(1_000, 100_000, 0, Standard, rate)]);
        assert_eq!(bad_debt_vat_share(52_500, &totals), Some(9_182));
        let unknown = VatTotals::of(&[LineAmounts::compute(1_000, 100_000, 0, Standard, None)]);
        assert_eq!(bad_debt_vat_share(52_500, &unknown), None);
    }
}
