//! The stand-ins of wl-inventory's recall trace (`LotTrace`) and of wl-sales'
//! customers (`SaleCustomers`): the lines a test puts in, the notice status
//! the vertical sets, and the customer of each sale.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::inventory::{LotTrace, TraceFuture, TraceMark, TracedHolder};
use wl_kernel::sales::{SaleCustomer, SaleCustomers, SalesFuture};
use wl_kernel::{PartyId, Subject};

/// The lines of the recall, oldest first.
#[derive(Clone, Default)]
pub struct Trace(Arc<Mutex<Vec<TracedHolder>>>);

impl Trace {
    /// A line for `subject` (a job card or a sale) whose party is `party`.
    pub fn line(&self, subject: &Subject, party: Option<i64>, on: NaiveDate) {
        let mut lines = self.0.lock().unwrap();
        let id = i64::try_from(lines.len()).unwrap() + 1;
        lines.push(TracedHolder {
            line: Subject::new("inventory.recall_line", id),
            subject: Some(subject.clone()),
            party: party.map(PartyId),
            lot: None,
            item: Some(Subject::new("inventory.item", 3)),
            location: None,
            quantity_milli: 1_000,
            on: Some(on),
            kind: "possible".to_owned(),
            mark: TraceMark::Possible,
            sales: 1,
            delivery_status: None,
            notice: "pending".to_owned(),
        });
    }

    /// The count of `sales` walk-in sales at `location` on `on`.
    pub fn walk_in(&self, location: &Subject, sales: i64, on: NaiveDate) {
        let mut lines = self.0.lock().unwrap();
        let id = i64::try_from(lines.len()).unwrap() + 1;
        lines.push(TracedHolder {
            line: Subject::new("inventory.recall_line", id),
            subject: None,
            party: None,
            lot: None,
            item: Some(Subject::new("inventory.item", 3)),
            location: Some(location.clone()),
            quantity_milli: sales * 1_000,
            on: Some(on),
            kind: "walk_in".to_owned(),
            mark: TraceMark::Possible,
            sales,
            delivery_status: None,
            notice: "pending".to_owned(),
        });
    }

    /// The status of each line now, by the line's id.
    pub fn notices(&self) -> BTreeMap<i64, String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|line| (line.line.subject_id, line.notice.clone()))
            .collect()
    }
}

impl LotTrace for Trace {
    fn holders<'a>(
        &'a self,
        _state: &'a AppState,
        _recall: &'a Subject,
    ) -> TraceFuture<'a, Vec<TracedHolder>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().clone()) })
    }

    fn set_notice<'a>(
        &'a self,
        _state: &'a AppState,
        line: &'a Subject,
        sent: bool,
        _by: Option<i64>,
    ) -> TraceFuture<'a, bool> {
        Box::pin(async move {
            let mut lines = self.0.lock().unwrap();
            let Some(found) = lines.iter_mut().find(|l| l.line == *line) else {
                return Ok(false);
            };
            let (from, to) = if sent {
                ("pending", "sent")
            } else {
                ("sent", "pending")
            };
            if found.notice == from {
                found.notice = to.to_owned();
                Ok(true)
            } else {
                Ok(false)
            }
        })
    }
}

/// The customer of each sale.
#[derive(Clone, Default)]
pub struct Customers(Arc<Mutex<BTreeMap<Subject, SaleCustomer>>>);

impl Customers {
    /// `customer` is behind `sale`.
    pub fn set(&self, sale: &Subject, customer: SaleCustomer) {
        self.0.lock().unwrap().insert(sale.clone(), customer);
    }
}

impl SaleCustomers for Customers {
    fn customer<'a>(
        &'a self,
        _state: &'a AppState,
        sale: &'a Subject,
    ) -> SalesFuture<'a, SaleCustomer> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(sale)
                .copied()
                .unwrap_or(SaleCustomer::Unknown))
        })
    }
}
