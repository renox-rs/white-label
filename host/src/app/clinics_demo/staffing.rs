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
}
