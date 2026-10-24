//! The fakes the credit control tests need: a ledger that answers as the
//! invoicer (open invoices) and as the cheque book (uncleared cheques), the
//! notifier that records, and a recorder of the order events. The credit rules
//! under test are the real `wl-sales` ones.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::chrono::TimeDelta;
use renox::prelude::*;
use wl_kernel::cheques::{
    ChequeFuture, ChequeId, ChequeSchedule, ChequeSummary, Cheques, RegisterCheque, provide_cheques,
};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{
    InvoiceDraft, InvoiceFuture, InvoiceId, InvoiceStatus, InvoiceSummary, Invoicer,
    provide_invoicer,
};
use wl_kernel::notify::provide_notifier;
use wl_kernel::sales::{SalesOrderConfirmed, SalesOrderCreditHeld};
use wl_kernel::{PartyId, Subject};

use super::shop::Mail;

/// What the books and the cheque book hold, entered by hand.
#[derive(Clone, Default)]
pub struct Ledger {
    invoices: Arc<Mutex<Vec<InvoiceSummary>>>,
    cheques: Arc<Mutex<Vec<(i64, i64)>>>,
}

impl Ledger {
    /// An open invoice of `party` with `balance` fils, due `due_in` days from
    /// today (negative: that many days overdue).
    pub fn invoice(&self, party: i64, balance: i64, due_in: i64) {
        let mut all = self.invoices.lock().unwrap();
        let id = all.len() as i64 + 1;
        all.push(InvoiceSummary {
            id: InvoiceId(id),
            number: Some(format!("INV-{id}")),
            status: if due_in < 0 {
                InvoiceStatus::Overdue
            } else {
                InvoiceStatus::Issued
            },
            party: PartyId(party),
            legal_entity_id: Some(1),
            currency: "AED".to_owned(),
            gross_total: balance,
            balance,
            due_date: Some(today() + TimeDelta::days(due_in)),
            disputed: false,
            chasing_paused_until: None,
        });
    }

    /// An uncleared post-dated cheque of `party` for `amount` fils.
    pub fn cheque(&self, party: i64, amount: i64) {
        self.cheques.lock().unwrap().push((party, amount));
    }

    /// Forgets every invoice and cheque.
    pub fn clear(&self) {
        self.invoices.lock().unwrap().clear();
        self.cheques.lock().unwrap().clear();
    }
}

impl Invoicer for Ledger {
    fn draft<'a>(
        &'a self,
        _db: &'a Db,
        _draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        Box::pin(async { Ok(None) })
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
        Box::pin(async { Ok(()) })
    }

    fn open_of<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
    ) -> InvoiceFuture<'a, Vec<InvoiceSummary>> {
        Box::pin(async move {
            Ok(self
                .invoices
                .lock()
                .unwrap()
                .iter()
                .filter(|invoice| invoice.party == party)
                .cloned()
                .collect())
        })
    }
}

impl Cheques for Ledger {
    fn register<'a>(
        &'a self,
        _state: &'a AppState,
        _cheque: RegisterCheque,
        _by: Option<i64>,
    ) -> ChequeFuture<'a, ChequeSummary> {
        Box::pin(async { Err(Error::NotFound) })
    }

    fn register_schedule<'a>(
        &'a self,
        _state: &'a AppState,
        _schedule: ChequeSchedule,
        _by: Option<i64>,
    ) -> ChequeFuture<'a, Vec<ChequeSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn find<'a>(&'a self, _db: &'a Db, _id: ChequeId) -> ChequeFuture<'a, Option<ChequeSummary>> {
        Box::pin(async { Ok(None) })
    }

    fn of_agreement<'a>(
        &'a self,
        _db: &'a Db,
        _agreement: &'a Subject,
    ) -> ChequeFuture<'a, Vec<ChequeSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn open_bounced_amount<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _currency: &'a str,
    ) -> ChequeFuture<'a, i64> {
        Box::pin(async { Ok(0) })
    }

    fn uncleared_amount<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
        _currency: &'a str,
    ) -> ChequeFuture<'a, i64> {
        Box::pin(async move {
            Ok(self
                .cheques
                .lock()
                .unwrap()
                .iter()
                .filter(|(p, _)| *p == party.0)
                .map(|(_, amount)| amount)
                .sum())
        })
    }
}

/// The ledger, the mail and the order events a test saw.
#[derive(Clone, Default)]
pub struct Credit {
    /// The invoices and cheques.
    pub ledger: Ledger,
    /// The notifications sent.
    pub mail: Mail,
    /// `SalesOrderCreditHeld`.
    pub held: Arc<Mutex<Vec<SalesOrderCreditHeld>>>,
    /// `SalesOrderConfirmed`.
    pub confirmed: Arc<Mutex<Vec<SalesOrderConfirmed>>>,
}

/// Records the events of type `E` into `log`.
fn record<E: Event + Clone + Send + Sync + 'static>(app: &mut Registry, log: &Arc<Mutex<Vec<E>>>) {
    let log = log.clone();
    app.listen(move |event: E, _state: AppState| {
        let log = log.clone();
        async move {
            log.lock().unwrap().push(event);
            Ok(())
        }
    });
}

impl Module for Credit {
    fn name(&self) -> &'static str {
        "credit_fakes"
    }

    fn register(&self, app: &mut Registry) {
        provide_invoicer(app, self.ledger.clone());
        provide_cheques(app, self.ledger.clone());
        provide_notifier(app, self.mail.clone());
        record(app, &self.held);
        record(app, &self.confirmed);
    }
}
