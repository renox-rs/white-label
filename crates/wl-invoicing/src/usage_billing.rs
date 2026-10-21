//! The usage recorder of the kernel on this module (wl-invoicing §5.2 steps 1
//! to 3, §6 `UsageRecorder`, `FIN.USG.01`; 09 Manpower `MPW.RUL.05`).
//!
//! A vertical describes an approved timesheet as a
//! [`TimesheetBilling`](wl_kernel::usage::TimesheetBilling) and
//! [`record_timesheet`] prices it by the rate card version in force on each
//! work date ([`crate::rate_pricing`], `MPW.ACL.16`) and stores unbilled
//! usage records; [`bill_party`] is the usage billing run of a party.
//!
//! Decisions:
//!
//! - **Per day, by the unit of the line in force that day.** An hourly line
//!   bills each day's hours per category (a record per day and category, so
//!   the minimum hours per shift and the multipliers of that day's version
//!   apply). A daily line bills a day with hours once: `public_holiday` when
//!   the day has holiday hours, else `normal`; overtime on a day rate is not
//!   billed extra. A monthly line bills the days the deployment ran in the
//!   period (see [`TimesheetBilling::deployed`](wl_kernel::usage::TimesheetBilling)),
//!   not those suspended, as month rate × those days ÷ calendar days of the
//!   month, per version; a timesheet over two months gives a line per month.
//!   `night` and `rest_day` hours are billed at the base rate (the rate card
//!   has multipliers for overtime and public holidays only).
//! - **Suspended days bill nothing**, whatever the unit, also when hours were
//!   recorded for them.
//! - **Once.** The records of a timesheet are those whose source it is and
//!   that are no adjustment. Recording again deletes the ones not invoiced
//!   and records anew, so a retry or a re-approval after a reopen bills
//!   nothing twice; once one is invoiced the timesheet is left alone (a later
//!   correction is an adjustment timesheet, [`crate::usage_adjustment`]).
//! - **Unpriced days** (no card, no accepted version in force that day, no
//!   line for the role and shift) are returned, not billed.
//! - **Billing locks.** [`bill_party`] tells the timesheets the invoices bill
//!   that billing used them (`record_billing_use`), so they are `locked` and
//!   never repriced.
//! - One draft invoice per issuing entity and currency, as [`usage::bill`]
//!   does; the default entity bills every record.

use std::collections::{BTreeMap, BTreeSet};

use renox::chrono::{Datelike, Days, NaiveDate};
use renox::prelude::*;
use wl_kernel::attendance::LabourFuture;
use wl_kernel::timesheets::{TIMESHEET_SUBJECT_TYPE, timesheets};
use wl_kernel::usage::{BilledInvoice, TimesheetBilling, UsageRecorded, UsageRecorder};
use wl_kernel::{PartyId, Subject};

use crate::rate_card::{RateCard, Unit};
use crate::rate_pricing::{
    MonthWork, PUBLIC_HOLIDAY, UsagePricing, Work, record_hours, record_month_on, unit_on,
};
use crate::usage::{self, UsageRecord};

/// The category a day rate bills a day under.
const NORMAL: &str = "normal";

/// Thousandths of an hour in `minutes`, rounded half up.
fn milli_of(minutes: i64) -> i64 {
    (minutes * 1000 + 30) / 60
}

/// The records `timesheet` made by its own hours (no adjustment).
async fn own_records(db: &Db, timesheet: &Subject) -> Result<Vec<UsageRecord>> {
    UsageRecord::query()
        .where_(UsageRecord::SOURCE_TYPE.eq(timesheet.subject_type.clone()))
        .where_(UsageRecord::SOURCE_ID.eq(timesheet.subject_id))
        .where_(UsageRecord::ADJUSTMENT_ID.is_null())
        .order_by(UsageRecord::ID)
        .get(db)
        .await
}

/// Deletes unbilled `records` with what they were priced from.
async fn discard(db: &Db, records: Vec<UsageRecord>) -> Result {
    for mut record in records {
        UsagePricing::query()
            .where_(UsagePricing::USAGE_RECORD_ID.eq(record.id))
            .delete(db)
            .await?;
        record.delete(db).await?;
    }
    Ok(())
}

/// The days from `from` to `to`, oldest first.
fn days(from: NaiveDate, to: NaiveDate) -> Vec<NaiveDate> {
    let mut all = Vec::new();
    let mut day = from;
    while day <= to {
        all.push(day);
        match day.checked_add_days(Days::new(1)) {
            Some(next) => day = next,
            None => break,
        }
    }
    all
}

/// Prices and records the hours of `billing` (see the module docs).
async fn record_all(
    state: &AppState,
    card: &RateCard,
    billing: &TimesheetBilling,
    made: &mut Vec<UsageRecord>,
) -> Result<Vec<NaiveDate>> {
    let db = &state.db;
    let mut by_day: BTreeMap<NaiveDate, BTreeMap<&str, i64>> = BTreeMap::new();
    for line in &billing.hours {
        if line.minutes > 0 && !billing.is_suspended(line.date) {
            *by_day
                .entry(line.date)
                .or_default()
                .entry(line.category.as_str())
                .or_default() += line.minutes;
        }
    }
    let mut unpriced = Vec::new();
    let mut month_days: BTreeMap<(i32, u32), BTreeSet<NaiveDate>> = BTreeMap::new();
    for day in days(billing.period_from, billing.period_to) {
        if billing.is_suspended(day) {
            continue;
        }
        let worked = by_day.get(&day);
        let unit = unit_on(db, card, &billing.role, &billing.shift_type, day).await?;
        match unit {
            Some(Unit::Month) => {
                if billing.deployed.contains(day) {
                    month_days
                        .entry((day.year(), day.month()))
                        .or_default()
                        .insert(day);
                }
            }
            Some(Unit::Day) => {
                let Some(worked) = worked else { continue };
                let category = if worked.contains_key(PUBLIC_HOLIDAY) {
                    PUBLIC_HOLIDAY
                } else {
                    NORMAL
                };
                let work = work_of(billing, day, category, 1000);
                match record_hours(db, &billing.timesheet, &work, None).await? {
                    Some(record) => made.push(record),
                    None => unpriced.push(day),
                }
            }
            Some(Unit::Hour) => {
                let Some(worked) = worked else { continue };
                for (category, minutes) in worked {
                    let work = work_of(billing, day, category, milli_of(*minutes));
                    match record_hours(db, &billing.timesheet, &work, None).await? {
                        Some(record) => made.push(record),
                        None => {
                            if !unpriced.contains(&day) {
                                unpriced.push(day);
                            }
                        }
                    }
                }
            }
            None => {
                if worked.is_some() {
                    unpriced.push(day);
                }
            }
        }
    }
    for ((year, month), only) in month_days {
        let work = MonthWork {
            source: billing.contract.clone(),
            role: billing.role.clone(),
            shift_type: billing.shift_type.clone(),
            year,
            month,
        };
        made.extend(record_month_on(db, &billing.timesheet, &work, Some(&only), None).await?);
    }
    Ok(unpriced)
}

fn work_of(billing: &TimesheetBilling, day: NaiveDate, category: &str, milli: i64) -> Work {
    Work::new(
        billing.contract.clone(),
        &billing.role,
        &billing.shift_type,
        day,
        category,
        milli,
    )
}

/// Records the approved hours of `billing` as unbilled usage (see the module
/// docs).
///
/// # Errors
///
/// 422 for a month rate on a line that is not one; the database's errors.
pub async fn record_timesheet(
    state: &AppState,
    billing: &TimesheetBilling,
) -> Result<UsageRecorded> {
    let db = &state.db;
    let existing = own_records(db, &billing.timesheet).await?;
    if existing.iter().any(|record| record.invoice_id.is_some()) {
        return Ok(UsageRecorded {
            invoiced: true,
            ..UsageRecorded::default()
        });
    }
    discard(db, existing).await?;
    let Some(card) = RateCard::of_source(db, &billing.contract).await? else {
        let mut unpriced: Vec<NaiveDate> = billing
            .hours
            .iter()
            .filter(|line| line.minutes > 0 && !billing.is_suspended(line.date))
            .map(|line| line.date)
            .collect();
        unpriced.dedup();
        return Ok(UsageRecorded {
            unpriced,
            ..UsageRecorded::default()
        });
    };
    let mut made = Vec::new();
    match record_all(state, &card, billing, &mut made).await {
        Ok(unpriced) => Ok(UsageRecorded {
            records: made.len(),
            unpriced,
            invoiced: false,
        }),
        Err(error) => {
            // Nothing half recorded is left to be billed.
            discard(db, made).await?;
            Err(error)
        }
    }
}

/// The usage billing run of `party`: its unbilled records become draft
/// invoices ([`usage::bill`]) and the timesheets they bill are told that
/// billing used them.
///
/// # Errors
///
/// The database's errors; 422 for a timesheet that is no longer approved.
pub async fn bill_party(state: &AppState, party: PartyId) -> Result<Vec<BilledInvoice>> {
    let invoices = usage::bill(&state.db, party).await?;
    let sheets = timesheets(state);
    let mut billed = Vec::new();
    for invoice in invoices {
        let records = usage::of_invoice(&state.db, invoice.id).await?;
        let mut sources: Vec<Subject> = Vec::new();
        for record in &records {
            let source = record.source();
            if source.subject_type == TIMESHEET_SUBJECT_TYPE && !sources.contains(&source) {
                sources.push(source);
            }
        }
        let reference = invoice.subject().to_string();
        sheets
            .record_billing_use(state, &sources, &reference)
            .await?;
        billed.push(BilledInvoice {
            invoice: invoice.subject(),
            party,
            lines: records.len(),
            timesheets: sources,
        });
    }
    Ok(billed)
}

/// What `register` provides.
pub(crate) struct Recorder;

impl UsageRecorder for Recorder {
    fn record_timesheet<'a>(
        &'a self,
        state: &'a AppState,
        billing: &'a TimesheetBilling,
    ) -> LabourFuture<'a, UsageRecorded> {
        Box::pin(record_timesheet(state, billing))
    }

    fn bill<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> LabourFuture<'a, Vec<BilledInvoice>> {
        Box::pin(bill_party(state, party))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minutes_become_thousandths_of_an_hour() {
        assert_eq!(milli_of(480), 8_000);
        assert_eq!(milli_of(90), 1_500);
        assert_eq!(milli_of(1), 17);
    }

    #[test]
    fn days_run_from_the_first_to_the_last() {
        let from = NaiveDate::from_ymd_opt(2026, 11, 29).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 12, 2).unwrap();
        assert_eq!(days(from, to).len(), 4);
        assert!(days(to, from).is_empty());
    }
}
