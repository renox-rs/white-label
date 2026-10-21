//! An invoicer for the logistics tests: the crate does not depend on
//! `wl-invoicing`, so the tests provide an `Invoicer` that records the drafts
//! it is asked for and answers their ids from 1, and keeps a few open
//! invoices of the clients (ids from 100) that a payment can be allocated to.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::invoicing::{
    InvoiceDraft, InvoiceFuture, InvoiceId, InvoiceStatus, InvoiceSummary, Invoicer,
};
use wl_kernel::{PartyId, Subject};

/// The first id of an open invoice added with [`Invoices::open`].
const FIRST_OPEN: i64 = 100;

#[derive(Default)]
struct Inner {
    drafts: Vec<InvoiceDraft>,
    open: Vec<InvoiceSummary>,
    allocations: Vec<(InvoiceId, Subject, i64)>,
    discarded: Vec<i64>,
}

/// The recording `Invoicer`.
#[derive(Clone, Default)]
pub struct Invoices(Arc<Mutex<Inner>>);

impl Invoices {
    /// The drafts asked for, oldest first (draft `n` is invoice `n`).
    pub fn drafts(&self) -> Vec<InvoiceDraft> {
        self.0.lock().unwrap().drafts.clone()
    }

    /// An issued invoice of `party` with `number`, a balance of `balance`
    /// fils and the due date `due`; answers its id.
    pub fn open(
        &self,
        party: PartyId,
        number: &str,
        balance: i64,
        due: (i32, u32, u32),
    ) -> InvoiceId {
        let mut inner = self.0.lock().unwrap();
        let id = InvoiceId(FIRST_OPEN + i64::try_from(inner.open.len()).unwrap());
        inner.open.push(InvoiceSummary {
            id,
            number: Some(number.to_owned()),
            status: InvoiceStatus::Issued,
            party,
            legal_entity_id: Some(1),
            currency: "AED".to_owned(),
            gross_total: balance,
            balance,
            due_date: NaiveDate::from_ymd_opt(due.0, due.1, due.2),
            disputed: false,
            chasing_paused_until: None,
        });
        id
    }

    /// Flags the invoice `id` as disputed.
    pub fn dispute(&self, id: InvoiceId) {
        self.change(id, |invoice| invoice.disputed = true);
    }

    /// Writes the invoice `id` off.
    pub fn write_off(&self, id: InvoiceId) {
        self.change(id, |invoice| invoice.status = InvoiceStatus::WrittenOff);
    }

    fn change(&self, id: InvoiceId, change: impl FnOnce(&mut InvoiceSummary)) {
        let mut inner = self.0.lock().unwrap();
        let invoice = inner
            .open
            .iter_mut()
            .find(|invoice| invoice.id == id)
            .expect("the invoice exists");
        change(invoice);
    }

    /// The invoice `id` as it is now.
    pub fn invoice(&self, id: InvoiceId) -> InvoiceSummary {
        let inner = self.0.lock().unwrap();
        inner
            .open
            .iter()
            .find(|invoice| invoice.id == id)
            .cloned()
            .expect("the invoice exists")
    }

    /// The allocations made: invoice, the payment that was allocated, amount.
    pub fn allocations(&self) -> Vec<(InvoiceId, Subject, i64)> {
        self.0.lock().unwrap().allocations.clone()
    }
}

impl Invoicer for Invoices {
    fn draft<'a>(
        &'a self,
        _db: &'a Db,
        draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        let mut inner = self.0.lock().unwrap();
        inner.drafts.push(draft);
        let id = i64::try_from(inner.drafts.len()).unwrap_or(i64::MAX);
        Box::pin(async move { Ok(Some(InvoiceId(id))) })
    }

    fn discard_draft<'a>(&'a self, _db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, bool> {
        let mut inner = self.0.lock().unwrap();
        let exists = usize::try_from(id.0).is_ok_and(|n| n >= 1 && n <= inner.drafts.len());
        let gone = exists && !inner.discarded.contains(&id.0);
        if gone {
            inner.discarded.push(id.0);
        }
        Box::pin(async move { Ok(gone) })
    }

    fn find<'a>(&'a self, _db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        let inner = self.0.lock().unwrap();
        let mut found = inner.open.iter().find(|invoice| invoice.id == id).cloned();
        // A draft the app asked for is found until it is discarded.
        if found.is_none()
            && !inner.discarded.contains(&id.0)
            && let Some(draft) = usize::try_from(id.0)
                .ok()
                .and_then(|n| n.checked_sub(1))
                .and_then(|n| inner.drafts.get(n))
        {
            found = Some(InvoiceSummary {
                id,
                number: None,
                status: InvoiceStatus::Draft,
                party: draft.party,
                legal_entity_id: None,
                currency: draft.currency.clone(),
                gross_total: 0,
                balance: 0,
                due_date: draft.due_date,
                disputed: false,
                chasing_paused_until: None,
            });
        }
        drop(inner);
        Box::pin(async move { Ok(found) })
    }

    fn open_of<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
    ) -> InvoiceFuture<'a, Vec<InvoiceSummary>> {
        let mut found: Vec<InvoiceSummary> = self
            .0
            .lock()
            .unwrap()
            .open
            .iter()
            .filter(|invoice| {
                invoice.party == party && invoice.status.is_open() && invoice.balance > 0
            })
            .cloned()
            .collect();
        found.sort_by_key(|invoice| (invoice.due_date, invoice.id.0));
        Box::pin(async move { Ok(found) })
    }

    fn allocate<'a>(
        &'a self,
        _state: &'a AppState,
        id: InvoiceId,
        source: Subject,
        amount: i64,
    ) -> InvoiceFuture<'a, ()> {
        let mut inner = self.0.lock().unwrap();
        let answer = match inner.open.iter_mut().find(|invoice| invoice.id == id) {
            Some(invoice)
                if invoice.status.is_open() && amount > 0 && amount <= invoice.balance =>
            {
                invoice.balance -= amount;
                if invoice.balance == 0 {
                    invoice.status = InvoiceStatus::Paid;
                } else {
                    invoice.status = InvoiceStatus::PartiallyPaid;
                }
                Ok(())
            }
            _ => Err(Error::NotFound),
        };
        if answer.is_ok() {
            inner.allocations.push((id, source, amount));
        }
        Box::pin(async move { answer })
    }
}
