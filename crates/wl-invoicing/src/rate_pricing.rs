//! Pricing hours by rate card version (wl-invoicing §5.2 steps 1 and 2,
//! `FIN.USG.01`; 09 Manpower `MPW.ACL.16`).
//!
//! **Which version.** A work date is priced by the `accepted` version of the
//! card with the latest effective date on or before it
//! ([`version_in_force`]); the previous version applies until the day before.
//! A version that is pending, waiting for approval or rejected prices nothing,
//! so a work date before the first accepted version is unpriced.
//!
//! **Hours.** [`price`] takes the line of the role and shift in that version:
//! base rate × quantity × the multiplier of the category (`overtime` the
//! overtime one, `public_holiday` the holiday one), with the line's minimum
//! hours per shift applied to every category but overtime (a 3-hour shift
//! with a 4-hour minimum bills 4 hours). The multipliers and the minimum are
//! those of the version in force on the work date, which for a shift that
//! crosses midnight is the date the shift starts (the caller passes it).
//! Each priced line is rounded once, half up to the fil, after all the
//! multiplying. [`record_hours`] stores it as a usage record for the billing
//! run and keeps what it was priced from ([`UsagePricing`]).
//!
//! Decision: when the multiplied rate is a whole number of fils the record is
//! hours at that unit price; when it is not (a rate of AED 27.50 × 1.15), it
//! is one line of the single rounded amount, so the rounding happens once.
//!
//! **Months.** [`price_month`] splits a month rate by the versions in force
//! over the calendar days of the month: month rate × days of the version in
//! the month ÷ calendar days of the month, one line per version, each
//! rounded once. Month lines are priced when the month is billed
//! ([`record_month`]) and are not repriced afterwards.
//!
//! **Repricing.** [`reprice`] runs when a version becomes `accepted`
//! (a past-dated one only after the approval): the hours worked from its
//! effective date that are not yet invoiced and whose timesheet is not
//! `locked` get the price of the version now in force on their work date.
//! Invoiced hours and locked timesheets keep the price they had.

use renox::chrono::{Datelike, Days, NaiveDate};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::invoicing::VatCategory;
use wl_kernel::timesheets::timesheets;

use crate::budget_category::refused;
use crate::rate_card::{RateCard, RateCardLine, RateCardVersion, Unit, VersionStatus};
use crate::texts::english;
use crate::usage::{self, UsageDraft, UsageRecord};

/// The hours category with the overtime multiplier.
pub const OVERTIME: &str = "overtime";
/// The hours category with the public holiday multiplier.
pub const PUBLIC_HOLIDAY: &str = "public_holiday";

/// What a usage record was priced from.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_usage_pricings",
    unique(usage_record_id),
    index(rate_card_id, work_date)
)]
pub struct UsagePricing {
    /// The primary key.
    pub id: i64,
    /// The usage record.
    pub usage_record_id: i64,
    /// The rate card.
    pub rate_card_id: i64,
    /// The version that priced it.
    pub version_id: i64,
    /// The date of the work.
    pub work_date: NaiveDate,
    /// The role.
    pub role: String,
    /// The shift type.
    pub shift_type: String,
    /// The hours category.
    pub category: String,
    /// The quantity worked, in thousandths of the line's unit, before the
    /// minimum.
    pub worked_milli: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Hours (or days) to price.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Work {
    /// The contract whose card prices it, `manpower.client_contract:4`.
    pub source: Subject,
    /// The role.
    pub role: String,
    /// The shift type.
    pub shift_type: String,
    /// The date of the work; for a shift that crosses midnight the date the
    /// shift starts.
    pub work_date: NaiveDate,
    /// The hours category: `normal`, [`OVERTIME`], [`PUBLIC_HOLIDAY`]...
    pub category: String,
    /// The quantity worked in thousandths of the line's unit.
    pub worked_milli: i64,
}

impl Work {
    /// `worked_milli` thousandths of an hour of `category` by `role` on
    /// `shift_type` on `work_date`, priced by the card of `source`.
    pub fn new(
        source: Subject,
        role: &str,
        shift_type: &str,
        work_date: NaiveDate,
        category: &str,
        worked_milli: i64,
    ) -> Self {
        Work {
            source,
            role: role.trim().to_lowercase(),
            shift_type: shift_type.trim().to_lowercase(),
            work_date,
            category: category.to_owned(),
            worked_milli,
        }
    }
}

/// The price of some [`Work`].
#[derive(Debug, Clone)]
pub struct Priced {
    /// The version that priced it.
    pub version: RateCardVersion,
    /// The line it used.
    pub line: RateCardLine,
    /// The quantity billed after the minimum, in thousandths.
    pub billed_milli: i64,
    /// The amount in fils, rounded once, without VAT.
    pub amount: i64,
}

/// `numerator ÷ denominator` rounded half up; both positive or the numerator
/// zero.
fn half_up(numerator: i128, denominator: i128) -> i64 {
    i64::try_from((numerator * 2 + denominator) / (denominator * 2)).unwrap_or(i64::MAX)
}

/// The amount of `billed_milli` thousandths at `rate` fils with the
/// multiplier `hundredths`, rounded once half up.
pub fn amount(rate: i64, hundredths: i64, billed_milli: i64) -> i64 {
    half_up(
        i128::from(rate) * i128::from(hundredths) * i128::from(billed_milli),
        100_000,
    )
}

/// The accepted version of card `card_id` in force on `on`: the latest
/// effective on or before that day.
///
/// # Errors
///
/// The database's errors.
pub async fn version_in_force(
    db: impl renox::db::Executor<'_>,
    card_id: i64,
    on: NaiveDate,
) -> Result<Option<RateCardVersion>> {
    RateCardVersion::query()
        .where_(RateCardVersion::RATE_CARD_ID.eq(card_id))
        .where_(RateCardVersion::STATUS.eq(VersionStatus::Accepted))
        .where_(RateCardVersion::EFFECTIVE_FROM.lte(on))
        .order_by_desc(RateCardVersion::EFFECTIVE_FROM)
        .first(db)
        .await
}

/// The line of `role` on `shift_type` in `version`.
async fn line_of(
    db: &Db,
    version: &RateCardVersion,
    role: &str,
    shift_type: &str,
) -> Result<Option<RateCardLine>> {
    Ok(version
        .lines(db)
        .await?
        .into_iter()
        .find(|line| line.role == role && line.shift_type == shift_type))
}

/// Prices `work` by the version in force on its date. `None` when the source
/// has no card, no accepted version is in force, the version has no hourly
/// or daily line for the role and shift, or the quantity is not positive.
///
/// # Errors
///
/// The database's errors.
pub async fn price(db: &Db, work: &Work) -> Result<Option<Priced>> {
    if work.worked_milli <= 0 {
        return Ok(None);
    }
    let Some(card) = RateCard::of_source(db, &work.source).await? else {
        return Ok(None);
    };
    price_with_card(db, &card, work).await
}

async fn price_with_card(db: &Db, card: &RateCard, work: &Work) -> Result<Option<Priced>> {
    let Some(version) = version_in_force(db, card.id, work.work_date).await? else {
        return Ok(None);
    };
    let Some(line) = line_of(db, &version, &work.role, &work.shift_type).await? else {
        return Ok(None);
    };
    if line.unit == Unit::Month {
        return Ok(None);
    }
    let hundredths = hundredths_of(&line, &work.category);
    let billed_milli = if work.category == OVERTIME {
        work.worked_milli
    } else {
        work.worked_milli.max(line.min_hours_milli)
    };
    let total = amount(line.rate, hundredths, billed_milli);
    Ok(Some(Priced {
        version,
        line,
        billed_milli,
        amount: total,
    }))
}

/// The multiplier in hundredths that `category` has on `line`.
fn hundredths_of(line: &RateCardLine, category: &str) -> i64 {
    match category {
        OVERTIME => line.ot_multiplier,
        PUBLIC_HOLIDAY => line.holiday_multiplier,
        _ => 100,
    }
}

/// `1234` as `1.234`.
fn milli_text(milli: i64) -> String {
    format!("{}.{:03}", milli / 1000, milli % 1000)
}

/// `2700` fils as `27.00`.
fn money_text(fils: i64) -> String {
    format!("{}.{:02}", fils / 100, fils % 100)
}

/// The text of the invoice line of `priced`.
fn describe(work: &Work, priced: &Priced) -> String {
    let unit = match priced.line.unit {
        Unit::Day => "days",
        _ => "h",
    };
    let hundredths = hundredths_of(&priced.line, &work.category);
    format!(
        "{} {}, {} {} {unit} at AED {} x {}.{:02}, {} (version {})",
        work.role,
        work.shift_type,
        work.category,
        milli_text(priced.billed_milli),
        money_text(priced.line.rate),
        hundredths / 100,
        hundredths % 100,
        work.work_date,
        priced.version.version,
    )
}

/// The quantity and unit price of a usage record for `priced`: hours at the
/// multiplied unit price when that is whole fils, else one line of the
/// rounded amount.
fn record_amounts(priced: &Priced, work: &Work) -> (i64, i64) {
    let hundredths = hundredths_of(&priced.line, &work.category);
    let scaled = priced.line.rate * hundredths;
    if scaled % 100 == 0 {
        (priced.billed_milli, scaled / 100)
    } else {
        (1000, priced.amount)
    }
}

/// Prices `work` and stores it as an unbilled usage record for the party of
/// the card, from the timesheet `timesheet`. `None` when nothing prices it
/// (see [`price`]): a version still pending prices nothing.
///
/// # Errors
///
/// The database's errors.
pub async fn record_hours(
    db: &Db,
    timesheet: &Subject,
    work: &Work,
    legal_entity_id: Option<i64>,
) -> Result<Option<UsageRecord>> {
    let Some(card) = RateCard::of_source(db, &work.source).await? else {
        return Ok(None);
    };
    let Some(priced) = price_with_card(db, &card, work).await? else {
        return Ok(None);
    };
    let (quantity, unit_price) = record_amounts(&priced, work);
    let mut draft = UsageDraft::hours(
        card.party(),
        timesheet.clone(),
        &work.category,
        quantity,
        unit_price,
    )
    .description(&describe(work, &priced))
    .rate_date(work.work_date);
    draft.currency = card.currency.clone();
    draft.vat_category = VatCategory::Standard;
    if let Some(entity) = legal_entity_id {
        draft = draft.legal_entity(entity);
    }
    let record = usage::record(db, draft).await?;
    UsagePricing::create(
        db,
        UsagePricing {
            usage_record_id: record.id,
            rate_card_id: card.id,
            version_id: priced.version.id,
            work_date: work.work_date,
            role: work.role.clone(),
            shift_type: work.shift_type.clone(),
            category: work.category.clone(),
            worked_milli: work.worked_milli,
            ..UsagePricing::default()
        },
    )
    .await?;
    Ok(Some(record))
}

/// Reprices the hours of the card of `version` worked from its effective
/// date: those not yet invoiced and not in a `locked` timesheet get the price
/// of the version now in force on their work date. Answers how many records
/// changed. A version that is not `accepted` changes nothing.
///
/// # Errors
///
/// The database's errors.
pub async fn reprice(state: &AppState, version: &RateCardVersion) -> Result<usize> {
    if version.status != VersionStatus::Accepted {
        return Ok(0);
    }
    let card = RateCard::find_or_404(&state.db, version.rate_card_id).await?;
    let pricings = UsagePricing::query()
        .where_(UsagePricing::RATE_CARD_ID.eq(card.id))
        .where_(UsagePricing::WORK_DATE.gte(version.effective_from))
        .order_by(UsagePricing::ID)
        .get(&state.db)
        .await?;
    let sheets = timesheets(state);
    let mut changed = 0;
    for mut pricing in pricings {
        let mut record = UsageRecord::find_or_404(&state.db, pricing.usage_record_id).await?;
        if record.invoice_id.is_some() || sheets.is_locked(state, &record.source()).await? {
            continue;
        }
        let work = Work::new(
            card.source(),
            &pricing.role,
            &pricing.shift_type,
            pricing.work_date,
            &pricing.category,
            pricing.worked_milli,
        );
        let Some(priced) = price_with_card(&state.db, &card, &work).await? else {
            continue;
        };
        if priced.version.id == pricing.version_id {
            continue;
        }
        let (quantity, unit_price) = record_amounts(&priced, &work);
        record.quantity_milli = quantity;
        record.unit_price = unit_price;
        record.description = describe(&work, &priced);
        record
            .save_only(&state.db, &["quantity_milli", "unit_price", "description"])
            .await?;
        pricing.version_id = priced.version.id;
        pricing.save_only(&state.db, &["version_id"]).await?;
        changed += 1;
    }
    Ok(changed)
}

/// One line of a month rate: the days one version prices in the month.
#[derive(Debug, Clone)]
pub struct MonthLine {
    /// The version.
    pub version: RateCardVersion,
    /// The first day of the version in the month.
    pub from: NaiveDate,
    /// The last day of the version in the month.
    pub to: NaiveDate,
    /// The days of the version in the month.
    pub days: i64,
    /// The calendar days of the month.
    pub month_days: i64,
    /// The month rate of that version, in fils.
    pub rate: i64,
    /// `rate × days ÷ month_days`, rounded once half up, in fils.
    pub amount: i64,
}

/// The calendar days of `year`-`month`.
fn days_in_month(year: i32, month: u32) -> Option<(NaiveDate, i64)> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)?
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)?
    };
    Some((first, (next - first).num_days()))
}

/// A month rate to price.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonthWork {
    /// The contract whose card prices it.
    pub source: Subject,
    /// The role.
    pub role: String,
    /// The shift type.
    pub shift_type: String,
    /// The year.
    pub year: i32,
    /// The month, 1 to 12.
    pub month: u32,
}

/// Prices the month rate of `work`, one line per version in force over the
/// month's days; days no accepted version prices are left out.
///
/// # Errors
///
/// 422 for a version whose line for the role and shift is not a month rate.
pub async fn price_month(db: &Db, work: &MonthWork) -> Result<Vec<MonthLine>> {
    let (source, month) = (&work.source, work.month);
    let Some((first, month_days)) = days_in_month(work.year, month) else {
        return Err(refused(english("wl_invoicing.rate_card.problem.month")));
    };
    let Some(card) = RateCard::of_source(db, source).await? else {
        return Ok(Vec::new());
    };
    let role = work.role.trim().to_lowercase();
    let shift_type = work.shift_type.trim().to_lowercase();
    let mut lines: Vec<MonthLine> = Vec::new();
    for offset in 0..month_days {
        let day = first
            .checked_add_days(Days::new(u64::try_from(offset).unwrap_or_default()))
            .unwrap_or(first);
        let Some(version) = version_in_force(db, card.id, day).await? else {
            continue;
        };
        if let Some(last) = lines.last_mut()
            && last.version.id == version.id
            && last.to.succ_opt() == Some(day)
        {
            last.to = day;
            last.days += 1;
            continue;
        }
        let Some(line) = line_of(db, &version, &role, &shift_type).await? else {
            continue;
        };
        if line.unit != Unit::Month {
            return Err(refused(english("wl_invoicing.rate_card.problem.not_month")));
        }
        lines.push(MonthLine {
            version,
            from: day,
            to: day,
            days: 1,
            month_days,
            rate: line.rate,
            amount: 0,
        });
    }
    for line in &mut lines {
        line.amount = half_up(
            i128::from(line.rate) * i128::from(line.days),
            i128::from(line.month_days),
        );
    }
    debug_assert!(lines.iter().all(|l| l.from.month() == month));
    Ok(lines)
}

/// Prices the month and stores one unbilled usage record per version line,
/// from `record_source` (the contract's month, a timesheet...).
///
/// # Errors
///
/// As [`price_month`].
pub async fn record_month(
    db: &Db,
    record_source: &Subject,
    work: &MonthWork,
    legal_entity_id: Option<i64>,
) -> Result<Vec<UsageRecord>> {
    let lines = price_month(db, work).await?;
    let Some(card) = RateCard::of_source(db, &work.source).await? else {
        return Ok(Vec::new());
    };
    let mut records = Vec::new();
    for line in lines {
        if line.amount <= 0 {
            continue;
        }
        let description = format!(
            "{} {}, AED {} x {}/{} days, {} to {} (version {})",
            work.role.trim().to_lowercase(),
            work.shift_type.trim().to_lowercase(),
            money_text(line.rate),
            line.days,
            line.month_days,
            line.from,
            line.to,
            line.version.version,
        );
        let mut draft = UsageDraft::hours(
            card.party(),
            record_source.clone(),
            "month",
            1000,
            line.amount,
        )
        .description(&description)
        .rate_date(line.from);
        draft.currency = card.currency.clone();
        if let Some(entity) = legal_entity_id {
            draft = draft.legal_entity(entity);
        }
        records.push(usage::record(db, draft).await?);
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_amount_is_rounded_once_half_up() {
        // 2 h of overtime at AED 27.00 x 1.25.
        assert_eq!(amount(2700, 125, 2000), 6750);
        // 4 h at AED 27.00.
        assert_eq!(amount(2700, 100, 4000), 10800);
        // AED 27.50 x 1.15 x 1 h is 31.625: 31.63.
        assert_eq!(amount(2750, 115, 1000), 3163);
        assert_eq!(amount(2750, 115, 0), 0);
    }

    #[test]
    fn a_month_rate_is_split_by_calendar_days() {
        assert_eq!(half_up(310_000 * 10, 31), 100_000);
        assert_eq!(half_up(341_000 * 21, 31), 231_000);
    }

    #[test]
    fn the_days_of_a_month_are_counted() {
        assert_eq!(days_in_month(2026, 12).unwrap().1, 31);
        assert_eq!(days_in_month(2028, 2).unwrap().1, 29);
        assert!(days_in_month(2026, 13).is_none());
    }

    #[test]
    fn a_whole_fil_unit_price_stays_hours_and_a_fraction_becomes_one_line() {
        let line = |rate| RateCardLine {
            rate,
            ot_multiplier: 115,
            ..RateCardLine::default()
        };
        let work = Work::new(
            Subject::new("manpower.client_contract", 1),
            "guard",
            "day",
            NaiveDate::from_ymd_opt(2026, 12, 2).unwrap(),
            OVERTIME,
            1000,
        );
        let priced = |rate| Priced {
            version: RateCardVersion::default(),
            line: line(rate),
            billed_milli: 1000,
            amount: amount(rate, 115, 1000),
        };
        assert_eq!(record_amounts(&priced(2000), &work), (1000, 2300));
        assert_eq!(record_amounts(&priced(2750), &work), (1000, 3163));
    }
}
