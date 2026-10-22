//! BL-MPW-06 (#420) and BL-MPW-11 (#425), `MPW.RUL.05`, `MPW.ACL.16`,
//! `FIN.USG.01` and `FIN.RAT.01`, the invoicing half: the kernel's
//! `UsageRecorder` turns the approved hours of a timesheet into usage records
//! priced by the rate card version of each work date, bills a month rate by
//! the days not suspended, bills a day rate by attendance, records a timesheet
//! once, and leaves what is invoiced as it is.

mod support;

use renox::prelude::*;
use wl_invoicing::rate_card::{RateCard, Unit};
use wl_invoicing::rate_cards::LineInput;
use wl_invoicing::usage::{self, UsageRecord};
use wl_kernel::PartyId;
use wl_kernel::timesheets::HoursLine;
use wl_kernel::usage::{DaySpan, TimesheetBilling, usage_recorder};

use support::rates::{
    accept_signed, account_manager, card, contract, d, guard, on, sheet, version,
};
use support::{BLUE_DUNES, Booted, boot};

/// `minutes` of `category` on `day`.
fn hours(day: &str, category: &str, minutes: i64) -> HoursLine {
    HoursLine {
        date: d(day),
        category: category.into(),
        minutes,
    }
}

/// Timesheet `id` for November to 2 December of a guard on the day shift who
/// ran from 1 November, with `lines` and no suspension.
fn billing(id: i64, from: &str, to: &str, lines: Vec<HoursLine>) -> TimesheetBilling {
    TimesheetBilling {
        timesheet: sheet(id),
        contract: contract(),
        role: "guard".into(),
        shift_type: "day".into(),
        period_from: d(from),
        period_to: d(to),
        deployed: DaySpan::new(d("2026-11-01"), None),
        suspended: Vec::new(),
        hours: lines,
    }
}

/// The records of timesheet `id`, oldest first.
async fn records(t: &Booted, id: i64) -> Vec<UsageRecord> {
    UsageRecord::query()
        .where_(UsageRecord::SOURCE_ID.eq(id))
        .order_by(UsageRecord::ID)
        .get(t.db())
        .await
        .unwrap()
}

/// A card with an accepted version per `(effective from, line)`.
async fn priced(t: &Booted, versions: &[(&str, LineInput)]) -> RateCard {
    let staff = account_manager(t, "Maya").await;
    let card = card(t).await;
    for (from, line) in versions {
        let v = version(t, &card, from, line.clone()).await;
        accept_signed(t, &v, &staff).await;
    }
    card
}

/// AC (#420): the hours of 30 November and of 1 December are priced by the
/// version of their own work date (AED 25, then AED 27 with overtime 1.25), a
/// day without a version is not billed and is reported, and the draft invoice
/// bills the lines of the timesheet.
#[renox::test]
async fn approved_hours_are_priced_by_the_version_of_the_work_date_and_billed_as_a_draft() {
    let t = boot().await;
    on(&t, "2026-11-15", async {
        priced(
            &t,
            &[
                ("2026-11-01", guard(25)),
                ("2026-12-01", guard(27).overtime(125)),
            ],
        )
        .await;
    })
    .await;
    on(&t, "2026-12-04", async {
        let recorder = usage_recorder(t.state());
        let mut work = billing(
            7,
            "2026-10-30",
            "2026-12-02",
            vec![
                hours("2026-10-30", "normal", 480),
                hours("2026-11-30", "normal", 480),
                hours("2026-12-01", "normal", 480),
                hours("2026-12-01", "overtime", 120),
            ],
        );
        work.deployed = DaySpan::new(d("2026-10-30"), None);
        let done = recorder.record_timesheet(t.state(), &work).await.unwrap();
        assert_eq!(done.records, 3);
        assert_eq!(done.unpriced, [d("2026-10-30")], "before the first version");

        let nets: Vec<_> = records(&t, 7)
            .await
            .iter()
            .map(|r| (r.rate_date, r.draft_line().net()))
            .collect();
        assert_eq!(
            nets,
            [
                (Some(d("2026-11-30")), 20_000),
                (Some(d("2026-12-01")), 21_600),
                (Some(d("2026-12-01")), 6_750),
            ]
        );

        let invoices = recorder.bill(t.state(), BLUE_DUNES).await.unwrap();
        assert_eq!(invoices.len(), 1);
        assert_eq!(invoices[0].lines, 3);
        assert_eq!(invoices[0].timesheets, [sheet(7)]);
        assert_eq!(
            t.sheets.billed(),
            [(sheet(7), invoices[0].invoice.to_string())],
            "billing told the timesheet it was used"
        );
        assert!(
            usage::unbilled(t.db(), BLUE_DUNES)
                .await
                .unwrap()
                .is_empty()
        );
    })
    .await;
}

/// AC2 (#425): a monthly rate of AED 3,000 for a worker suspended from 21
/// November bills 3,000 x 20/30 = AED 2,000 for November; a month before the
/// deployment started or after it ended is not billed.
#[renox::test]
async fn a_month_rate_is_reduced_by_the_suspended_days() {
    let t = boot().await;
    on(&t, "2026-12-04", async {
        priced(
            &t,
            &[(
                "2026-11-01",
                LineInput::new("guard", "day", Unit::Month, 300_000),
            )],
        )
        .await;
        let mut work = billing(8, "2026-11-01", "2026-11-30", Vec::new());
        work.suspended = vec![DaySpan::new(d("2026-11-21"), None)];
        let done = usage_recorder(t.state())
            .record_timesheet(t.state(), &work)
            .await
            .unwrap();
        assert_eq!(done.records, 1);
        let stored = records(&t, 8).await;
        assert_eq!(stored[0].draft_line().net(), 200_000);
        assert!(
            stored[0].description.contains("20/30"),
            "{}",
            stored[0].description
        );
    })
    .await;
}

/// AC2 (#425), a worker who resumed: suspended 11-10 to 11-19 (two spans of
/// five days) and a deployment that started on 5 November bill
/// 3,000 x (30 - 4 - 10) / 30 = AED 1,600.
#[renox::test]
async fn a_month_rate_leaves_out_the_days_before_the_start_and_every_suspension() {
    let t = boot().await;
    on(&t, "2026-12-04", async {
        priced(
            &t,
            &[(
                "2026-11-01",
                LineInput::new("guard", "day", Unit::Month, 300_000),
            )],
        )
        .await;
        let mut work = billing(9, "2026-11-01", "2026-11-30", Vec::new());
        work.deployed = DaySpan::new(d("2026-11-05"), None);
        work.suspended = vec![
            DaySpan::new(d("2026-11-10"), Some(d("2026-11-14"))),
            DaySpan::new(d("2026-11-15"), Some(d("2026-11-19"))),
        ];
        usage_recorder(t.state())
            .record_timesheet(t.state(), &work)
            .await
            .unwrap();
        let stored = records(&t, 9).await;
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].draft_line().net(), 160_000);
    })
    .await;
}

/// AC2 (#425), the day rate: AED 120 a day bills the days with attendance up to
/// 20 November only; the days from 21 November bill nothing, also with hours.
#[renox::test]
async fn a_day_rate_bills_the_days_with_attendance_and_none_of_the_suspended_ones() {
    let t = boot().await;
    on(&t, "2026-12-04", async {
        priced(
            &t,
            &[(
                "2026-11-01",
                LineInput::new("guard", "day", Unit::Day, 12_000),
            )],
        )
        .await;
        let mut work = billing(10, "2026-11-01", "2026-11-30", Vec::new());
        work.hours = vec![
            hours("2026-11-18", "normal", 480),
            hours("2026-11-19", "normal", 600),
            hours("2026-11-20", "normal", 480),
            hours("2026-11-22", "normal", 480),
        ];
        work.suspended = vec![DaySpan::new(d("2026-11-21"), None)];
        usage_recorder(t.state())
            .record_timesheet(t.state(), &work)
            .await
            .unwrap();
        let stored = records(&t, 10).await;
        let nets: Vec<_> = stored
            .iter()
            .map(|r| (r.rate_date, r.draft_line().net()))
            .collect();
        assert_eq!(
            nets,
            [
                (Some(d("2026-11-18")), 12_000),
                (Some(d("2026-11-19")), 12_000),
                (Some(d("2026-11-20")), 12_000),
            ]
        );
    })
    .await;
}

/// Retries: recording the same timesheet again replaces its unbilled records
/// (the same total, no double billing); once billed it is left alone, and a
/// second billing run makes no second invoice.
#[renox::test]
async fn recording_twice_bills_once_and_an_invoiced_timesheet_is_left_alone() {
    let t = boot().await;
    on(&t, "2026-12-04", async {
        priced(&t, &[("2026-11-01", guard(25))]).await;
        let recorder = usage_recorder(t.state());
        let work = billing(
            11,
            "2026-11-02",
            "2026-11-03",
            vec![
                hours("2026-11-02", "normal", 480),
                hours("2026-11-03", "normal", 480),
            ],
        );
        recorder.record_timesheet(t.state(), &work).await.unwrap();
        let again = recorder.record_timesheet(t.state(), &work).await.unwrap();
        assert_eq!((again.records, again.invoiced), (2, false));
        assert_eq!(records(&t, 11).await.len(), 2);

        let first = recorder.bill(t.state(), BLUE_DUNES).await.unwrap();
        assert_eq!(first.len(), 1);
        let after = recorder.record_timesheet(t.state(), &work).await.unwrap();
        assert!(after.invoiced);
        assert_eq!(records(&t, 11).await.len(), 2, "nothing recorded again");
        assert!(
            recorder
                .bill(t.state(), BLUE_DUNES)
                .await
                .unwrap()
                .is_empty(),
            "nothing left to bill"
        );
        assert_eq!(t.sheets.billed().len(), 1);
    })
    .await;
}

/// A party other than the client's, and a contract without a card, bill
/// nothing: the hours come back as unpriced.
#[renox::test]
async fn a_contract_without_a_rate_card_bills_nothing_and_reports_the_days() {
    let t = boot().await;
    on(&t, "2026-12-04", async {
        let work = billing(
            12,
            "2026-11-02",
            "2026-11-03",
            vec![hours("2026-11-02", "normal", 480)],
        );
        let done = usage_recorder(t.state())
            .record_timesheet(t.state(), &work)
            .await
            .unwrap();
        assert_eq!((done.records, done.unpriced), (0, vec![d("2026-11-02")]));
        let none = usage_recorder(t.state())
            .bill(t.state(), PartyId(999))
            .await
            .unwrap();
        assert!(none.is_empty());
    })
    .await;
}
