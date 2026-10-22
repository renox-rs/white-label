//! The inboxes of the demo clinic: a complaint from a low-rated visit and a
//! few tasks for the desk, the stock and the billing coordinator, so the
//! ticket lists are not empty on the first login. The messages of the stories
//! above (licence reminders, conflicts, stock shortages, the recall) are
//! already in the staff's notifications.

use renox::prelude::*;
use wl_kernel::tickets::{Assignee, OpenTicket, task_creator};
use wl_kernel::{PartyId, Subject};

use super::super::demo_data::day;
use super::Seed;
use super::people::{FATIMA, MOHAMMED};

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

    pub(crate) async fn notes(&mut self) -> Result {
        let fatima: PartyId = self.patients[FATIMA];
        let mohammed: PartyId = self.patients[MOHAMMED];
        self.ticket(
            OpenTicket::new(
                fatima.subject(),
                "general",
                "Complaint: waited 50 minutes at Business Bay (2-star feedback)",
            )
            .description(
                "The patient waited 50 minutes after her check-up and asked for a call back.",
            )
            .requester_party(fatima)
            .assign_to(self.assignee("manager.bay@palmgrove.example"))
            .due_at(wl_kernel::BusinessCalendar::default().from_local(self.local(2, 12, 0))),
        )
        .await?;
        self.ticket(
            OpenTicket::new(
                mohammed.subject(),
                "task",
                "Call Mohammed Raza about the unpaid balance of his last visit",
            )
            .assign_to(self.assignee("reception.jumeirah@palmgrove.example"))
            .due_at(wl_kernel::BusinessCalendar::default().from_local(self.local(1, 11, 0))),
        )
        .await?;
        self.ticket(
            OpenTicket::new(
                Subject::new(super::BRANCH, super::BUSINESS_BAY),
                "task",
                "Reorder anaesthetic cartridges before the recalled lot is returned",
            )
            .assign_to(self.assignee("store@palmgrove.example"))
            .due_at(wl_kernel::BusinessCalendar::default().from_local(self.local(3, 10, 0))),
        )
        .await?;
        let _ = day;
        Ok(())
    }
}
