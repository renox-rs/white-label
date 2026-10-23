//! Holding the sessions of the past. The tutor marks the attendance of a
//! session on the page of the session on the evening it was taught (the
//! seed's clock is set to the end of the session); an absence excused by a
//! notice is recorded 30 hours before the start by the front desk; a student
//! with a package is added to the roster of a session by the coordinator.

use renox::chrono::{Duration, NaiveDate};
use renox::prelude::*;
use wl_kernel::BusinessCalendar;

use super::batches::BATCHES;
use super::super::demo_data::day;
use super::{COORDINATOR, COUNSELLOR_LOGIN, Seed};

/// The days from today (negative: before) of `date`.
pub(crate) fn offset(date: NaiveDate) -> i64 {
    (date - day(0)).num_days()
}

/// What happened to a student in a session: the key of the enrolment and one
/// of `absent`, `late`, `excused` (a notice 30 hours before), `add` (a package
/// student joins the roster) and `add_absent`.
pub(crate) type Mark<'a> = (&'a str, &'a str);

impl Seed {
    fn local_of(&self, at: renox::db::DateTime) -> renox::chrono::NaiveDateTime {
        BusinessCalendar::default().local(at).naive_local()
    }

    /// Holds session number `index` of batch `key`, with the exceptions in
    /// `marks` (everybody else is present).
    pub(crate) async fn hold(&self, key: &str, index: usize, marks: &[Mark<'_>]) -> Result {
        let session = self.sessions_of(key).await?[index].clone();
        let id = session.id;
        let tutor = BATCHES
            .iter()
            .find(|plan| plan.key == key)
            .map_or(COORDINATOR, |plan| plan.tutor);
        let starts = self.local_of(session.starts_at);
        let mut form: Vec<(String, String)> = Vec::new();
        for (enrolment, mark) in marks {
            let eid = self.enrolment(enrolment);
            match *mark {
                "add" | "add_absent" => {
                    let when = starts - Duration::hours(30);
                    self.clock
                        .at_time(when, async {
                            self.driver
                                .post(
                                    COORDINATOR,
                                    &format!("/edtech/sessions/{id}/students"),
                                    &[("enrolment", &eid.to_string())],
                                )
                                .await
                        })
                        .await?;
                    if *mark == "add_absent" {
                        form.push((format!("mark_{eid}"), "absent".to_owned()));
                    }
                }
                "excused" => {
                    let when = starts - Duration::hours(30);
                    self.clock
                        .at_time(when, async {
                            self.driver
                                .post(
                                    COUNSELLOR_LOGIN,
                                    &format!("/edtech/sessions/{id}/notice"),
                                    &[("enrolment", &eid.to_string())],
                                )
                                .await
                        })
                        .await?;
                }
                other => form.push((format!("mark_{eid}"), other.to_owned())),
            }
        }
        let pairs: Vec<(&str, &str)> = form.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let when = self.local_of(session.ends_at) + Duration::minutes(10);
        self.clock
            .at_time(when, async {
                self.driver
                    .post(tutor, &format!("/edtech/sessions/{id}/attendance"), &pairs)
                    .await
            })
            .await?;
        Ok(())
    }

    /// Holds the sessions still scheduled that are dated up to `limit` days
    /// from today (and before today). `marks` are `(session number from 0,
    /// enrolment key, mark)`.
    pub(crate) async fn hold_until(
        &self,
        key: &str,
        limit: i64,
        marks: &[(usize, &str, &str)],
    ) -> Result {
        let limit = limit.min(-1);
        for (index, session) in self.sessions_of(key).await?.iter().enumerate() {
            if session.status != edtech::SessionStatus::Scheduled
                || offset(session.session_date) > limit
            {
                continue;
            }
            let here: Vec<Mark<'_>> = marks
                .iter()
                .filter(|(at, _, _)| *at == index)
                .map(|(_, enrolment, mark)| (*enrolment, *mark))
                .collect();
            self.hold(key, index, &here).await?;
        }
        Ok(())
    }
}
