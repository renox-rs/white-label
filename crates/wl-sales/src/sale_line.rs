//! The lines of a sale (wl-sales §3 *Sales order*, §4.2, §4.3): one table for
//! the lines of a counter sale and of a sales order, because a return, the
//! loyalty rules and the invoice all read "the lines of this sale" the same
//! way. A line belongs to the record named by its subject.
//!
//! Money is `i64` fils before VAT; quantities are thousandths. A line's net,
//! VAT and gross are those of the kernel's [`DraftLine`], so a sale shows the
//! figures its invoice will carry.
//!
//! Decisions of this base:
//! - There is no catalogue in this crate yet, so a line carries the code and
//!   the description of the item as sold, and the flag the loyalty programme
//!   reads (`loyalty_excluded`, wl-sales §3 *Loyalty programme* "excluded
//!   items or categories").
//! - `kind` is `item` for goods and services. The loyalty redemption (BL-SAL-23)
//!   is a line of kind `loyalty_redemption` with a negative price, so the sum
//!   of the lines already excludes "the redeemed value" from what earns.

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::invoicing::{DraftLine, VatCategory};

/// The kind of an ordinary line: goods or a service.
pub const KIND_ITEM: &str = "item";

/// The kind of the line a loyalty redemption adds (BL-SAL-23): a negative
/// price that lowers the taxable amount of the sale it is on.
pub const KIND_LOYALTY_REDEMPTION: &str = "loyalty_redemption";

/// One line of a counter sale or a sales order.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_sales_sale_lines", index(subject_type, subject_id))]
pub struct SaleLine {
    /// The primary key.
    pub id: i64,
    /// The type of the record the line belongs to.
    pub subject_type: String,
    /// The id of the record the line belongs to.
    pub subject_id: i64,
    /// The position, from 1.
    pub line_no: i64,
    /// The code of the item as sold.
    pub item_code: String,
    /// The description as sold.
    pub description: String,
    /// `item` or `loyalty_redemption`.
    pub kind: String,
    /// The quantity in thousandths (1000 = 1).
    pub quantity_milli: i64,
    /// The price of one unit before VAT, in fils.
    pub unit_price: i64,
    /// The discount on the line before VAT, in fils.
    pub discount: i64,
    /// The VAT treatment.
    pub vat_category: VatCategory,
    /// Whether the loyalty programme leaves the line out.
    pub loyalty_excluded: bool,
    /// The quantity handed over (sales orders), in thousandths.
    pub fulfilled_milli: i64,
    /// The quantity cancelled (sales orders), in thousandths.
    pub cancelled_milli: i64,
    /// The quantity of the open part set aside in stock (back-orders on),
    /// in thousandths.
    pub reserved_milli: i64,
    /// The quantity of the open part that could not be reserved and waits
    /// for stock, in thousandths.
    pub backordered_milli: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A line to add to a sale.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NewLine {
    /// The code of the item.
    pub item_code: String,
    /// The description.
    pub description: String,
    /// The quantity in thousandths.
    pub quantity_milli: i64,
    /// The price of one unit before VAT, in fils.
    pub unit_price: i64,
    /// The discount before VAT, in fils.
    pub discount: i64,
    /// The VAT treatment.
    pub vat_category: VatCategory,
    /// Whether the loyalty programme leaves it out.
    pub loyalty_excluded: bool,
    /// `item` or `loyalty_redemption`.
    pub kind: String,
}

impl NewLine {
    /// One unit of `item_code` at `unit_price` fils before VAT, standard
    /// rated.
    pub fn new(item_code: &str, unit_price: i64) -> Self {
        NewLine {
            item_code: item_code.to_owned(),
            description: item_code.to_owned(),
            quantity_milli: 1000,
            unit_price,
            discount: 0,
            vat_category: VatCategory::Standard,
            loyalty_excluded: false,
            kind: KIND_ITEM.to_owned(),
        }
    }

    /// With this description.
    pub fn description(mut self, description: &str) -> Self {
        self.description = description.to_owned();
        self
    }

    /// With this quantity in thousandths.
    pub fn quantity_milli(mut self, quantity_milli: i64) -> Self {
        self.quantity_milli = quantity_milli;
        self
    }

    /// With this quantity in whole units.
    pub fn quantity(self, units: i64) -> Self {
        self.quantity_milli(units * 1000)
    }

    /// With this discount in fils before VAT.
    pub fn discount(mut self, discount: i64) -> Self {
        self.discount = discount;
        self
    }

    /// With this VAT treatment.
    pub fn vat_category(mut self, category: VatCategory) -> Self {
        self.vat_category = category;
        self
    }

    /// Left out of the loyalty programme.
    pub fn loyalty_excluded(mut self) -> Self {
        self.loyalty_excluded = true;
        self
    }

    /// Of this kind (`item` or `loyalty_redemption`).
    pub fn kind(mut self, kind: &str) -> Self {
        self.kind = kind.to_owned();
        self
    }
}

/// The money of a sale, in fils.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Totals {
    /// The sum of the lines before VAT.
    pub net: i64,
    /// The VAT.
    pub vat: i64,
    /// Net plus VAT.
    pub gross: i64,
}

/// `numerator ÷ denominator` rounded half up; zero for a zero denominator.
pub fn ratio_half_up(value: i64, numerator: i64, denominator: i64) -> i64 {
    if denominator == 0 {
        return 0;
    }
    let num = i128::from(value) * i128::from(numerator);
    let den = i128::from(denominator);
    let magnitude = (2 * num.abs() + den.abs()) / (2 * den.abs());
    let signed = if (num < 0) != (den < 0) {
        -magnitude
    } else {
        magnitude
    };
    i64::try_from(signed).unwrap_or(0)
}

impl SaleLine {
    /// The lines of the record `subject`, in order.
    pub async fn of(db: &Db, subject: &Subject) -> Result<Vec<SaleLine>> {
        SaleLine::query()
            .where_(SaleLine::SUBJECT_TYPE.eq(subject.subject_type.clone()))
            .where_(SaleLine::SUBJECT_ID.eq(subject.subject_id))
            .order_by(SaleLine::ID)
            .get(db)
            .await
    }

    /// Adds `line` at the end of the lines of `subject`.
    pub async fn add(db: &Db, subject: &Subject, line: NewLine) -> Result<SaleLine> {
        let next = SaleLine::of(db, subject).await?.len() as i64 + 1;
        SaleLine::create(
            db,
            SaleLine {
                subject_type: subject.subject_type.clone(),
                subject_id: subject.subject_id,
                line_no: next,
                item_code: line.item_code,
                description: line.description,
                kind: line.kind,
                quantity_milli: line.quantity_milli,
                unit_price: line.unit_price,
                discount: line.discount,
                vat_category: line.vat_category,
                loyalty_excluded: line.loyalty_excluded,
                ..SaleLine::default()
            },
        )
        .await
    }

    /// The line as the kernel's invoice draft line.
    pub fn draft_line(&self) -> DraftLine {
        DraftLine::new(&self.description, self.unit_price, self.vat_category)
            .quantity_milli(self.quantity_milli)
            .discount(self.discount)
    }

    /// The net of the line before VAT, in fils.
    pub fn net(&self) -> i64 {
        self.draft_line().net()
    }

    /// The VAT of the line, in fils.
    pub fn vat(&self) -> i64 {
        self.draft_line().vat()
    }

    /// The line with VAT, in fils.
    pub fn gross(&self) -> i64 {
        self.draft_line().gross()
    }

    /// What the loyalty programme counts of the line before VAT: its net, or
    /// nothing when the line is excluded. A redemption line is negative, so it
    /// takes the redeemed value off what the other lines earn.
    pub fn eligible_net(&self) -> i64 {
        if self.loyalty_excluded { 0 } else { self.net() }
    }

    /// The loyalty-eligible net of `quantity_milli` of the line (a returned or
    /// no longer fulfilled part), rounded half up.
    pub fn eligible_net_of(&self, quantity_milli: i64) -> i64 {
        ratio_half_up(self.eligible_net(), quantity_milli, self.quantity_milli)
    }

    /// The quantity still open on an order line: not handed over, not
    /// cancelled.
    pub fn open_milli(&self) -> i64 {
        self.quantity_milli - self.fulfilled_milli - self.cancelled_milli
    }
}

/// The totals of `lines`.
pub fn totals(lines: &[SaleLine]) -> Totals {
    let net: i64 = lines.iter().map(SaleLine::net).sum();
    let vat: i64 = lines.iter().map(SaleLine::vat).sum();
    Totals {
        net,
        vat,
        gross: net + vat,
    }
}

/// What the loyalty programme counts of `lines` before VAT, in fils.
pub fn eligible_net(lines: &[SaleLine]) -> i64 {
    lines.iter().map(SaleLine::eligible_net).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(price: i64, qty_milli: i64) -> SaleLine {
        SaleLine {
            quantity_milli: qty_milli,
            unit_price: price,
            ..SaleLine::default()
        }
    }

    #[test]
    fn a_line_carries_five_percent_vat_like_the_invoice_line() {
        let l = line(10_000, 1000);
        assert_eq!((l.net(), l.vat(), l.gross()), (10_000, 500, 10_500));
    }

    #[test]
    fn an_excluded_line_earns_nothing_and_a_redemption_takes_value_off() {
        let item = line(10_000, 1000);
        let excluded = SaleLine {
            loyalty_excluded: true,
            ..line(5_000, 1000)
        };
        let redeemed = line(-800, 1000);
        assert_eq!(eligible_net(&[item, excluded, redeemed]), 9_200);
    }

    #[test]
    fn the_totals_add_up_the_lines() {
        let t = totals(&[line(10_000, 1000), line(2_000, 2000)]);
        assert_eq!((t.net, t.vat, t.gross), (14_000, 700, 14_700));
    }

    #[test]
    fn a_part_of_a_line_is_rounded_half_up() {
        let l = line(10_000, 3000);
        assert_eq!(l.eligible_net_of(1000), 10_000);
        assert_eq!(ratio_half_up(100, 1, 3), 33);
        assert_eq!(ratio_half_up(100, 1, 0), 0);
    }

    #[test]
    fn the_open_quantity_is_what_is_left() {
        let l = SaleLine {
            fulfilled_milli: 1000,
            cancelled_milli: 500,
            ..line(1, 3000)
        };
        assert_eq!(l.open_milli(), 1500);
    }
}
