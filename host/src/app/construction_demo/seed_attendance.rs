//! The sites and the attendance of the last 30 days: the site supervisor
//! marks the crew of each site at the end of every shift (through
//! `POST /attendance/mark`, one day back at most, so each day is marked on its
//! own day), HR accepts the older records. On purpose: one worker worked 14
//! hours and is flagged for overtime, three workers of an outdoor site were
//! marked across the midday break in the season, the worker whose work permit
//! expired is flagged on every record, and one worker stopped coming and has
//! an open absence case.

use construction::absence;
use renox::chrono::{Datelike, Days, NaiveDate, Weekday};
use renox::prelude::*;
use super::at;
use wl_attendance::{AttendanceRecord, AttendanceStatus, WorkLocation};

use super::people::email_of;
use super::roles::{HR, SUPERVISOR};
use super::seed_hr::{CLN, MALL, RST, STAFF, TWR};
use super::{Seed, day, text};

/// The outdoor terrace of the restaurant: the midday break applies there.
pub const TERRACE: &str = "RST-04/terrace";

/// The last day the midday break season covered before `today`: 15 June to
/// 15 September (`HRM.RUL.13`), on a working day.
pub fn season_day(today: NaiveDate) -> NaiveDate {
    let mut at = today.checked_sub_days(Days::new(1)).unwrap_or(today);
    for _ in 0..400 {
        let from = NaiveDate::from_ymd_opt(at.year(), 6, 15);
        let to = NaiveDate::from_ymd_opt(at.year(), 9, 15);
        let inside = matches!((from, to), (Some(from), Some(to)) if at >= from && at <= to);
        if inside && !matches!(at.weekday(), Weekday::Sat | Weekday::Sun) {
            return at;
        }
        at = at.checked_sub_days(Days::new(1)).unwrap_or(at);
    }
    today
}

/// Whether the sites work on `date`: Monday to Friday (Saturday is a rest day
/// of the calendar, so every hour on it would be overtime).
fn works(_code: &str, date: NaiveDate) -> bool {
    !matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
}

/// The nearest working day on or before `date`.
fn working_day_before(code: &str, date: NaiveDate) -> NaiveDate {
    let mut at = date;
    while !works(code, at) {
        at = at.checked_sub_days(Days::new(1)).unwrap_or(at);
    }
    at
}

impl Seed {
    /// The work locations of the projects in progress, in the scope of each
    /// project (the supervisor marks there). The restaurant has an outdoor
    /// terrace besides.
    pub(crate) async fn sites(&mut self) -> Result {
        for code in [TWR, MALL, CLN, RST] {
            let project = self.project(code).await?;
            let site = WorkLocation::create(
                self.db(),
                WorkLocation::named(&format!("{} site", project.code)).in_scope(&project.subject()),
            )
            .await?;
            self.sites.insert(code.to_owned(), site.id);
        }
        let project = self.project(RST).await?;
        let terrace = WorkLocation::create(
            self.db(),
            WorkLocation::named("RST-04 terrace (outdoor)")
                .in_scope(&project.subject())
                .outdoors(),
        )
        .await?;
        self.sites.insert(TERRACE.to_owned(), terrace.id);
        self.report.add("work locations", 5);
        Ok(())
    }

    /// The crew of project `code` on `date`: the workers on it and the
    /// supervisors who are there that day (a supervisor with two sites is at
    /// one of them a day, never at both).
    fn crew(code: &str, date: NaiveDate) -> Vec<&'static str> {
        STAFF
            .iter()
            .filter(|member| member.sites.contains(&code))
            .filter(|member| {
                member.sites.len() < 2
                    || member.sites[date.ordinal() as usize % member.sites.len()] == code
            })
            .map(|member| member.key)
            .collect()
    }

    /// Marks `key` present at `site` on `date` from `from` to `to`
    /// (`07:00`), as the site supervisor.
    async fn mark(
        &mut self,
        key: &str,
        site: i64,
        date: NaiveDate,
        from: &str,
        to: Option<&str>,
        note: &str,
    ) -> Result {
        let employee = self.employees[key].to_string();
        let site = site.to_string();
        let check_in = format!("{}T{from}", text(date));
        let check_out = to.map(|to| format!("{}T{to}", text(date))).unwrap_or_default();
        self.driver
            .post(
                email_of(SUPERVISOR),
                "/attendance/mark",
                &[
                    ("employee_id", employee.as_str()),
                    ("location_id", site.as_str()),
                    ("check_in", check_in.as_str()),
                    ("check_out", check_out.as_str()),
                    ("note", note),
                ],
            )
            .await?;
        self.report.add("attendance records", 1);
        Ok(())
    }

    /// Thirty days of attendance, each marked on its own day.
    pub(crate) async fn attendance(&mut self) -> Result {
        let today = crate::app::demo_data::real_today();
        let overtime_day = working_day_before(TWR, day(-12));
        let midday_day = season_day(today);
        for ago in (1..=30).rev() {
            let date = day(-ago);
            for code in [TWR, MALL, CLN, RST] {
                if !works(code, date) {
                    continue;
                }
                let site = self.sites[code];
                at_time!(self, date, 18, 0, {
                    for (index, key) in Self::crew(code, date).into_iter().enumerate() {
                        // Sher Ali stopped coming nine days ago.
                        if key == "sher" && ago < 9 {
                            continue;
                        }
                        // Everyone misses a day now and then.
                        if (index as i64 * 5 + ago * 3) % 17 == 0 {
                            continue;
                        }
                        let from = ["07:00", "07:10", "07:05", "07:05"][index % 4];
                        let to = ["16:00", "16:30", "16:15", "16:45"][(index + ago as usize) % 4];
                        if key == "karim" && date == overtime_day {
                            self.mark(
                                key,
                                site,
                                date,
                                "06:00",
                                Some("20:30"),
                                "Slab pour finished late",
                            )
                            .await?;
                        } else {
                            self.mark(key, site, date, from, Some(to), "").await?;
                        }
                    }
                    Ok::<(), renox::Error>(())
                })?;
            }
        }
        // Three workers of the outdoor terrace worked across the midday break
        // in the season: a block of 07:00 to 17:00 with no break.
        let terrace = self.sites[TERRACE];
        at_time!(self, midday_day, 18, 0, {
            for key in ["bilal", "sher", "dennis"] {
                self.mark(key, terrace, midday_day, "07:00", Some("17:00"), "")
                    .await?;
            }
            Ok::<(), renox::Error>(())
        })?;
        self.review_attendance().await?;
        // The daily rule for the workers who stopped coming.
        at!(self, today, { absence::daily(&self.state).await })?;
        let cases = absence::AbsenceCase::query().count(self.db()).await?;
        self.report.add("absence cases", cases as usize);
        Ok(())
    }

    /// HR accepts the records older than ten days that no flag holds.
    async fn review_attendance(&mut self) -> Result {
        let hr = email_of(HR);
        let records = AttendanceRecord::query()
            .where_(AttendanceRecord::STATUS.eq(AttendanceStatus::Recorded))
            .where_(AttendanceRecord::DATE.lt(day(-10)))
            .order_by(AttendanceRecord::ID)
            .get(self.db())
            .await?;
        for record in records {
            let when = super::upto(record.date.checked_add_days(Days::new(2)).unwrap_or(record.date));
            at!(self, when, {
                self.driver
                    .post(hr, &format!("/attendance/records/{}/accept", record.id), &[])
                    .await
            })?;
            self.report.add("attendance records accepted", 1);
        }
        Ok(())
    }
}
