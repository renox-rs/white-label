//! The inboxes of the demo centre: a complaint, a fee query, a classroom
//! issue and a few tasks for the front desk and the coordinator, so the
//! ticket lists are not empty on the first login. The tasks of the stories
//! above (the chargeback to collect, the tutors whose documents end, the
//! centre permit to renew) are there already.

use renox::prelude::*;
use wl_kernel::tickets::{Assignee, OpenTicket, task_creator};
use wl_kernel::{BusinessCalendar, PartyId, Subject};

use super::super::demo_data::day;
use super::people::{ELENA, HASSAN, LAYAN};
use super::{COORDINATOR, COUNSELLOR_LOGIN, MANAGER, Seed};

impl Seed {
    async fn ticket(&self, ticket: OpenTicket) -> Result {
        task_creator(&self.state)
            .open(&self.state, ticket)
            .await?
            .ok_or_else(|| renox::anyhow::anyhow!("demo seed: ticketing is off"))?;
        Ok(())
    }

    fn assignee(&self, email: &str) -> Assignee {
        Assignee::User {
            id: self.user(email).id,
        }
    }

    fn due(&self, days: i64, hour: u32) -> renox::db::DateTime {
        BusinessCalendar::default()
            .from_local(day(days).and_hms_opt(hour, 0, 0).unwrap_or_default())
    }

    pub(crate) async fn notes(&mut self) -> Result {
        let ahmed: PartyId = self.guardians[super::people::AHMED];
        let hassan = self.students[HASSAN];
        let elena = self.students[ELENA];
        let layan = self.students[LAYAN];
        self.ticket(
            OpenTicket::new(
                ahmed.subject(),
                "student_complaint",
                "Complaint: the make-up class for Mona was announced one day before",
            )
            .description("The father asked for two weeks of notice and for a call back.")
            .requester_party(ahmed)
            .assign_to(self.assignee(MANAGER))
            .due_at(self.due(2, 12)),
        )
        .await?;
        self.ticket(
            OpenTicket::new(
                hassan.subject(),
                "fee_query",
                "Why was a credit of the private tutoring package used for an absence?",
            )
            .requester_party(hassan)
            .assign_to(self.assignee(MANAGER))
            .due_at(self.due(3, 11)),
        )
        .await?;
        self.ticket(
            OpenTicket::new(
                Subject::new("edtech.batch", self.batches["ielts_run"]),
                "classroom_issue",
                "The projector in Room B flickers during the Wednesday evening class",
            )
            .assign_to(self.assignee(COORDINATOR))
            .due_at(self.due(1, 16)),
        )
        .await?;
        self.ticket(
            OpenTicket::new(
                elena.subject(),
                "task",
                "Call Elena Petrova: her discount waits for the centre manager",
            )
            .assign_to(self.assignee(COUNSELLOR_LOGIN))
            .due_at(self.due(1, 11)),
        )
        .await?;
        self.ticket(
            OpenTicket::new(
                layan.subject(),
                "task",
                "Collect the signed guardian consent for Layan Haddad before the next class",
            )
            .assign_to(self.assignee(COUNSELLOR_LOGIN))
            .due_at(self.due(-1, 15)),
        )
        .await?;
        self.ticket(
            OpenTicket::new(
                Subject::new("edtech.course", self.courses["PY-101"]),
                "task",
                "Prepare the January timetable for the IT courses",
            )
            .assign_to(self.assignee(COORDINATOR))
            .due_at(self.due(7, 14)),
        )
        .await?;
        Ok(())
    }
}
