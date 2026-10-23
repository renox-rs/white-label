//! The batches of the demo centre. The academic coordinator plans each one
//! at the batches page and opens it; the sessions come from the weekly
//! pattern (`edtech::plan`). A batch of the past is planned a few weeks before
//! its first session, on the seed's clock of that day. What happens after
//! (students enrol, sessions are held, the batch is cancelled or terminated)
//! is told in [`super::enrolments`] and [`super::stories`].

use renox::prelude::*;

use super::super::demo_data::{at, day};
use super::{COORDINATOR, HIND, LAYLA, OMAR, RAVI, SARA, Seed};

/// A batch to plan.
pub(crate) struct BatchPlan {
    /// The key the later steps use.
    pub(crate) key: &'static str,
    pub(crate) course: &'static str,
    pub(crate) capacity: i64,
    /// First and last day, from today.
    pub(crate) span: (i64, i64),
    pub(crate) weekdays: &'static str,
    pub(crate) time: &'static str,
    pub(crate) minutes: i64,
    pub(crate) tutor: &'static str,
    pub(crate) room: &'static str,
    /// The day it is planned, from today.
    pub(crate) planned: i64,
    /// Whether the coordinator opens it.
    pub(crate) open: bool,
}

const fn plan(
    key: &'static str,
    course: &'static str,
    capacity: i64,
    span: (i64, i64),
    weekdays: &'static str,
    time: &'static str,
    minutes: i64,
    tutor: &'static str,
    room: &'static str,
    planned: i64,
    open: bool,
) -> BatchPlan {
    BatchPlan {
        key,
        course,
        capacity,
        span,
        weekdays,
        time,
        minutes,
        tutor,
        room,
        planned,
        open,
    }
}

/// The batches in the order they are made.
pub(crate) const BATCHES: &[BatchPlan] = &[
    // Completed.
    plan("eng_done", "ENG-A2", 10, (-48, -27), "2,4", "10:00", 90, SARA, "Room A", -60, true),
    plan("xl_done", "XL-BIZ", 8, (-30, -16), "3", "14:00", 120, OMAR, "Computer lab", -40, true),
    // Terminated (one after three sessions, one after two) and cancelled.
    plan("code_term", "CODE-KIDS", 8, (-25, 25), "5", "16:00", 60, SARA, "Kids studio", -35, true),
    plan("xl_term", "XL-BIZ", 8, (-20, 10), "1", "09:00", 120, OMAR, "Computer lab", -30, true),
    plan("fr_cancel", "FR-A1", 8, (5, 35), "2,4", "15:00", 90, HIND, "Room A", -30, true),
    plan("eng_cancel", "ENG-A2", 10, (20, 40), "2,4", "10:00", 90, SARA, "Room A", -10, false),
    // Running.
    plan("ielts_run", "IELTS-B2", 12, (-14, 21), "1,3", "18:00", 90, SARA, "Room B", -30, true),
    plan("py_run", "PY-101", 10, (-10, 25), "2,4", "19:00", 90, OMAR, "Computer lab", -25, true),
    plan("arb_run", "ARB-KIDS", 8, (-21, 28), "6", "10:00", 60, LAYLA, "Kids studio", -35, true),
    plan("gmat_run", "GMAT-P", 8, (-14, 12), "2,4", "17:00", 120, RAVI, "Seminar room", -28, true),
    plan("fr_run", "FR-A1", 8, (-20, 15), "1,3", "12:00", 90, HIND, "Room A", -30, true),
    // Open: one is full, with a waiting list.
    plan("fr_open", "FR-A1", 3, (12, 40), "1,3", "16:00", 90, HIND, "Room A", -45, true),
    plan("code_open", "CODE-KIDS", 8, (10, 40), "5", "17:30", 60, SARA, "Kids studio", -40, true),
    plan("ielts_open", "IELTS-B2", 12, (20, 44), "6", "10:00", 120, SARA, "Room B", -10, true),
    // Planned: the permit of the centre and of the tutor end before them.
    plan("gmat_plan", "GMAT-P", 8, (30, 60), "2,4", "17:00", 120, RAVI, "Seminar room", -1, false),
    plan("xl_plan", "XL-BIZ", 8, (50, 64), "3", "14:00", 120, OMAR, "Computer lab", -1, false),
];

impl Seed {
    /// The tutor's resource and employee behind a login (or `layla`).
    pub(crate) fn tutor_ids(&self, email: &str) -> (i64, i64) {
        let resource = self
            .tutors
            .iter()
            .find(|(known, _)| *known == email)
            .map(|(_, id)| *id)
            .unwrap_or_default();
        (resource, self.employee_of(email))
    }

    /// The batches, planned (and opened) in the order of [`BATCHES`].
    pub(crate) async fn courses_and_batches(&mut self) -> Result {
        for plan in BATCHES {
            self.plan_batch(plan).await?;
        }
        Ok(())
    }

    /// Plans one batch; the cancelled and terminated ones are made when
    /// their story is told, the others here, except `fr_open`, `code_open` and
    /// `ielts_open`, which wait for the batches whose sessions they would
    /// meet (the terminated ones) to be over.
    pub(crate) async fn plan_batch(&mut self, plan: &BatchPlan) -> Result {
        let course = self.courses[plan.course].to_string();
        let (resource, employee) = self.tutor_ids(plan.tutor);
        let resource = resource.to_string();
        let employee = employee.to_string();
        let room = self.rooms[plan.room];
        let seats = self.room_seats(plan.room);
        let room_text = room.to_string();
        let seats_text = seats.to_string();
        let capacity = plan.capacity.to_string();
        let start = day(plan.span.0).to_string();
        let end = day(plan.span.1).to_string();
        let minutes = plan.minutes.to_string();
        let form = [
            ("course", course.as_str()),
            ("capacity", capacity.as_str()),
            ("start_date", start.as_str()),
            ("end_date", end.as_str()),
            ("weekdays", plan.weekdays),
            ("start_time", plan.time),
            ("duration", minutes.as_str()),
            ("tutor", resource.as_str()),
            ("employee", employee.as_str()),
            ("room", room_text.as_str()),
            ("room_limit", seats_text.as_str()),
        ];
        let reply = at!(self, day(plan.planned), {
            self.driver.post(COORDINATOR, "/edtech/batches", &form).await
        })?;
        let id = reply
            .redirected_id()
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: batch {} has no page", plan.key))?;
        self.batches.insert(plan.key, id);
        if plan.open {
            self.open_batch(plan.key, plan.planned + 1).await?;
        }
        Ok(())
    }

    fn room_seats(&self, room: &str) -> i64 {
        match room {
            "Room A" => 14,
            "Room B" => 12,
            "Computer lab" => 16,
            "Kids studio" => 10,
            _ => 20,
        }
    }

    /// The coordinator opens a planned batch on `on` (days from today).
    pub(crate) async fn open_batch(&self, key: &str, on: i64) -> Result {
        let id = self.batches[key];
        at!(self, day(on), {
            self.driver
                .post(COORDINATOR, &format!("/edtech/batches/{id}/open"), &[])
                .await
        })?;
        Ok(())
    }

    /// The sessions of a batch, in date order.
    pub(crate) async fn sessions_of(&self, key: &str) -> Result<Vec<edtech::Session>> {
        let mut rows = edtech::Session::of_batch(self.db(), self.batches[key]).await?;
        rows.sort_by_key(|s| (s.session_date, s.id));
        Ok(rows)
    }
}
