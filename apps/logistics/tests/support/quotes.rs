//! A quotation provider for the logistics tests: the crate does not depend on
//! a quotations module (`FIN.QUO.01`), so the tests provide a `Quotations`
//! that answers the terms a test sets for a job, and none otherwise (the job
//! then has the terms entered by hand, or none).

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::quotations::{QuotationFuture, QuotationTerms, Quotations};

/// The `Quotations` of the tests.
#[derive(Clone, Default)]
pub struct Quotes(Arc<Mutex<BTreeMap<i64, QuotationTerms>>>);

impl Quotes {
    /// The accepted quotation of the freight job `job` has these terms.
    pub fn set(&self, job: i64, terms: QuotationTerms) {
        self.0.lock().unwrap().insert(job, terms);
    }
}

impl Quotations for Quotes {
    fn terms<'a>(
        &'a self,
        _db: &'a Db,
        subject: &'a Subject,
    ) -> QuotationFuture<'a, Option<QuotationTerms>> {
        let found = self.0.lock().unwrap().get(&subject.subject_id).cloned();
        Box::pin(async move { Ok(found) })
    }
}
