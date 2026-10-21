//! The qualification guard of wl-hr and the workforce, faked (BL-ACC-19):
//! wl-hr has no Qualification records yet (BL-HRM-01), so the tests give the
//! staff users employee records and FTA tax agent registrations here.

use std::sync::{Arc, Mutex};

use accounting::filing::{AGENCY_SUBJECT_TYPE, TAX_AGENT_KIND, TAX_AGENT_ROLE};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::qualifications::{QualificationFuture, QualificationStatus, Qualifications};
use wl_kernel::workforce::{EmployeeId, Workforce, WorkforceFuture};

/// One FTA tax agent registration of an employee.
#[derive(Debug, Clone)]
struct Registration {
    employee: EmployeeId,
    number: String,
    valid_to: NaiveDate,
    /// Whether it is linked to the firm's tax agency.
    in_agency: bool,
}

/// The staff users with an employee record, and their registrations.
#[derive(Clone, Default)]
pub struct QualificationBook {
    staff: Arc<Mutex<Vec<(i64, EmployeeId)>>>,
    registrations: Arc<Mutex<Vec<Registration>>>,
}

impl QualificationBook {
    /// The employee record of `user`, made on first use.
    fn employee(&self, user: i64) -> EmployeeId {
        let mut staff = self.staff.lock().unwrap();
        if let Some((_, employee)) = staff.iter().find(|(u, _)| *u == user) {
            return *employee;
        }
        let employee = EmployeeId(staff.len() as i64 + 1);
        staff.push((user, employee));
        employee
    }

    /// `user` is staff with an employee record and no registration.
    pub fn staff(&self, user: i64) {
        let _ = self.employee(user);
    }

    /// `user` holds the FTA tax agent registration `number`, valid to
    /// `valid_to` and linked to the firm's tax agency.
    pub fn register(&self, user: i64, number: &str, valid_to: NaiveDate) {
        self.add(user, number, valid_to, true);
    }

    /// Like [`QualificationBook::register`], but not linked to the firm's
    /// agency (another firm's).
    pub fn register_elsewhere(&self, user: i64, number: &str, valid_to: NaiveDate) {
        self.add(user, number, valid_to, false);
    }

    /// `user` has a registration, valid to the end of 2027, if they have none.
    pub fn ensure(&self, user: i64) {
        let employee = self.employee(user);
        let has = self
            .registrations
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.employee == employee);
        if !has {
            self.register(
                user,
                &format!("TA-{user:04}"),
                NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
            );
        }
    }

    fn add(&self, user: i64, number: &str, valid_to: NaiveDate, in_agency: bool) {
        let employee = self.employee(user);
        self.registrations.lock().unwrap().push(Registration {
            employee,
            number: number.to_owned(),
            valid_to,
            in_agency,
        });
    }
}

impl Workforce for QualificationBook {
    fn employee_of_user<'a>(
        &'a self,
        _db: &'a Db,
        user_id: i64,
    ) -> WorkforceFuture<'a, Option<EmployeeId>> {
        let found = self
            .staff
            .lock()
            .unwrap()
            .iter()
            .find(|(u, _)| *u == user_id)
            .map(|(_, e)| *e);
        Box::pin(async move { Ok(found) })
    }
}

impl Qualifications for QualificationBook {
    fn qualification_valid<'a>(
        &'a self,
        _state: &'a AppState,
        employee: EmployeeId,
        kind: &'a str,
        on_date: NaiveDate,
        role: Option<&'a str>,
        scope: Option<&'a Subject>,
    ) -> QualificationFuture<'a, QualificationStatus> {
        let asked_for_the_agency = kind == TAX_AGENT_KIND
            && role == Some(TAX_AGENT_ROLE)
            && scope.is_some_and(|s| s.subject_type == AGENCY_SUBJECT_TYPE);
        let found = self
            .registrations
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.employee == employee)
            .max_by_key(|r| r.valid_to)
            .cloned();
        let status = match found {
            Some(r) => QualificationStatus {
                valid: asked_for_the_agency && r.in_agency && on_date <= r.valid_to,
                number: Some(r.number),
                valid_to: Some(r.valid_to),
            },
            None => QualificationStatus::default(),
        };
        Box::pin(async move { Ok(status) })
    }
}
