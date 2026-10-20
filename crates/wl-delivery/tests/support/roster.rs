//! A workforce for the delivery tests: the module does not depend on
//! `wl-hr`, so the tests provide a `Workforce` that maps a user to an
//! employee (id `100 + user id`) and knows who is on leave.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::workforce::{
    EmployeeId, StaffProfile, Workforce, WorkforceFuture,
};

/// The employee of a user: the tests' stand-in for wl-hr's records.
pub fn employee_of(user_id: i64) -> EmployeeId {
    EmployeeId(100 + user_id)
}

/// The users on leave and the day they are back.
#[derive(Clone, Default)]
pub struct Roster(Arc<Mutex<BTreeMap<i64, NaiveDate>>>);

impl Roster {
    /// `user_id` is on leave and back on `returns_on`.
    pub fn on_leave(&self, user_id: i64, returns_on: NaiveDate) {
        self.0.lock().unwrap().insert(user_id, returns_on);
    }
}

impl Workforce for Roster {
    fn staff<'a>(
        &'a self,
        _state: &'a AppState,
        employee: EmployeeId,
    ) -> WorkforceFuture<'a, Option<StaffProfile>> {
        let user_id = employee.0 - 100;
        let back = self.0.lock().unwrap().get(&user_id).copied();
        Box::pin(async move {
            Ok(Some(StaffProfile {
                employee,
                party: PartyId(1),
                user_id: Some(user_id),
                role: "driver".to_owned(),
                scopes: Vec::new(),
                legal_entity_id: None,
                listed: true,
                on_leave: back.is_some(),
                returns_on: back,
                managers: Vec::new(),
            }))
        })
    }

    fn employee_of_user<'a>(
        &'a self,
        _db: &'a Db,
        user_id: i64,
    ) -> WorkforceFuture<'a, Option<EmployeeId>> {
        Box::pin(async move { Ok(Some(employee_of(user_id))) })
    }
}
