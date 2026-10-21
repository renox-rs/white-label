//! An invoicer for the logistics tests: the crate does not depend on
//! `wl-invoicing`, so the tests provide an `Invoicer` that records the drafts
//! it is asked for and answers their ids from 1; the rest is refused.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::invoicing::{
    InvoiceDraft, InvoiceFuture, InvoiceId, InvoiceSummary, Invoicer,
};

/// The recording `Invoicer`.
#[derive(Clone, Default)]
pub struct Invoices(Arc<Mutex<Vec<InvoiceDraft>>>);

impl Invoices {
    /// The drafts asked for, oldest first (draft `n` is invoice `n`).
    pub fn drafts(&self) -> Vec<InvoiceDraft> {
        self.0.lock().unwrap().clone()
    }
}

impl Invoicer for Invoices {
    fn draft<'a>(
        &'a self,
        _db: &'a Db,
        draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        let mut drafts = self.0.lock().unwrap();
        drafts.push(draft);
        let id = i64::try_from(drafts.len()).unwrap_or(i64::MAX);
        Box::pin(async move { Ok(Some(InvoiceId(id))) })
    }

    fn find<'a>(
        &'a self,
        _db: &'a Db,
        _id: InvoiceId,
    ) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        Box::pin(async { Ok(None) })
    }

    fn allocate<'a>(
        &'a self,
        _state: &'a AppState,
        _id: InvoiceId,
        _source: Subject,
        _amount: i64,
    ) -> InvoiceFuture<'a, ()> {
        Box::pin(async { Err(Error::NotFound) })
    }
}
