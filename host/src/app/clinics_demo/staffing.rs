//! The staffing stories of the demo clinic, told after the appointments are
//! booked: a sick practitioner (approved at once, his note still to come)
//! whose appointments fall in conflict, a leave waiting for its decision, and
//! the nightly document run that reminds the clinic manager of the licence
//! that ends in 20 days.

use renox::prelude::*;
use wl_hr::{leave_service, sick_leave};
use wl_kernel::documents::today;

use super::super::demo_data::day;
use super::Seed;

impl Seed {
    pub(crate) async fn staffing(&mut self) -> Result {
        let bay_manager = self.user("manager.bay@palmgrove.example").id;
        let nurse = self.user("nurse@palmgrove.example").id;
        let samir = self.employee_of_login("samir@palmgrove.example");
        let grace = self.employee_of_login("nurse@palmgrove.example");
        // Dr Samir is sick today and tomorrow: the clinic manager of Business
        // Bay records it, its appointments are in conflict, and the sick note
        // is due in a few working days.
        sick_leave::record(&self.state, bay_manager, samir, today(), day(1)).await?;
        // The nurse asks for annual leave next month.
        leave_service::request(&self.state, nurse, grace, "annual", day(21), day(25)).await?;
        self.leave_stories().await?;
        // The nightly run of wl-documents: the reminder of the licence.
        wl_documents::expiry::run(&self.state, today()).await?;
        Ok(())
    }

    fn employee_of_login(&self, email: &str) -> i64 {
        self.employees
            .iter()
            .find(|(known, _)| *known == email)
            .map(|(_, employee)| employee.0)
            .unwrap_or_default()
    }

    /// Leave on the screens *My leave* and *Leave to decide*: one taken, one
    /// approved for later, one rejected with its reason, three waiting (one of
    /// them a sick leave whose note is still to come), all asked for on the
    /// employee's own account and decided by the manager of their branch.
    async fn leave_stories(&mut self) -> Result {
        let mariam = self.user("manager.jumeirah@palmgrove.example").id;
        let tariq = self.user("manager.bay@palmgrove.example").id;
        let state = self.state.clone();
        let who = |login: &'static str| (self.user(login).id, self.employee_of_login(login));
        let (sara, sara_employee) = who("reception.jumeirah@palmgrove.example");
        let (yara, yara_employee) = who("yara@palmgrove.example");
        let (khaled, khaled_employee) = who("store@palmgrove.example");
        let (noura, noura_employee) = who("reception.bay@palmgrove.example");
        // Sara took a week in the past.
        let taken =
            leave_service::request(&state, sara, sara_employee, "annual", day(-20), day(-16))
                .await?;
        leave_service::approve(&state, mariam, taken.id).await?;
        leave_service::complete_ended(&state, today()).await?;
        // Sara asks for a few days more; Dr Yara is refused; the storekeeper waits.
        leave_service::request(&state, sara, sara_employee, "annual", day(10), day(12)).await?;
        let refused =
            leave_service::request(&state, yara, yara_employee, "annual", day(30), day(31)).await?;
        leave_service::reject(
            &state,
            mariam,
            refused.id,
            "Two practitioners are away that week",
        )
        .await?;
        leave_service::request(&state, khaled, khaled_employee, "unpaid", day(14), day(14)).await?;
        // Noura's leave at Business Bay is approved for later; she is also ill
        // from tomorrow, and her sick note is still to come.
        let later =
            leave_service::request(&state, noura, noura_employee, "annual", day(40), day(44))
                .await?;
        leave_service::approve(&state, tariq, later.id).await?;
        leave_service::request(&state, noura, noura_employee, "sick", day(1), day(2)).await?;
        Ok(())
    }
}
