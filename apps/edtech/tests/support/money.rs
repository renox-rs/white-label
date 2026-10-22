//! The stand-ins the enrolments talk to through the kernel's contracts: the
//! party directory (students, guardians, payers), the invoice book, the
//! subscriptions and the prepaid packages. They keep what they were asked in
//! memory so a test can read it back.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::invoicing::{
    InvoiceDraft, InvoiceFuture, InvoiceId, InvoiceStatus, InvoiceSummary, Invoicer,
};
use wl_kernel::parties::{
    PartyDirectory, PartyFuture, PartyKind, PartyStatus, PartySummary,
};
use wl_kernel::sales::{
    PackageConsumption, PackageId, PackageLedger, PackageSales, SalesFuture, SellPackage,
};
use wl_kernel::subscriptions::{
    CancelSubscription, StartSubscription, SubscriptionFuture, SubscriptionId,
    SubscriptionSummary, Subscriptions,
};
use wl_kernel::{PartyId, Subject};

/// The parties the centre knows, with their guardians.
#[derive(Clone, Default)]
pub struct People {
    names: Arc<Mutex<BTreeMap<i64, String>>>,
    guardians: Arc<Mutex<BTreeMap<i64, Vec<PartyId>>>>,
}

impl People {
    /// Party `id` is called `name`.
    pub fn add(&self, id: i64, name: &str) {
        self.names.lock().unwrap().insert(id, name.to_owned());
    }

    /// `guardian` is a guardian of `child`.
    pub fn guardian(&self, child: i64, guardian: i64) {
        self.guardians
            .lock()
            .unwrap()
            .entry(child)
            .or_default()
            .push(PartyId(guardian));
    }

    fn summary(&self, id: i64) -> Option<PartySummary> {
        self.names.lock().unwrap().get(&id).map(|name| PartySummary {
            id: PartyId(id),
            code: format!("P-{id:05}"),
            kind: PartyKind::Person,
            display_name: name.clone(),
            status: PartyStatus::Active,
            display_block_reason: None,
        })
    }
}

impl PartyDirectory for People {
    fn find<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        let found = self.summary(party.0);
        Box::pin(async move { Ok(found) })
    }

    fn find_by_code<'a>(
        &'a self,
        _db: &'a Db,
        code: &'a str,
    ) -> PartyFuture<'a, Option<PartySummary>> {
        let id = code
            .strip_prefix("P-")
            .and_then(|digits| digits.parse::<i64>().ok());
        let found = id.and_then(|id| self.summary(id));
        Box::pin(async move { Ok(found) })
    }

    fn search<'a>(
        &'a self,
        _db: &'a Db,
        text: &'a str,
        limit: u64,
    ) -> PartyFuture<'a, Vec<PartySummary>> {
        let needle = text.to_lowercase();
        let ids: Vec<i64> = self.names.lock().unwrap().keys().copied().collect();
        let found: Vec<PartySummary> = ids
            .into_iter()
            .filter_map(|id| self.summary(id))
            .filter(|p| {
                p.display_name.to_lowercase().contains(&needle)
                    || p.code.to_lowercase().contains(&needle)
            })
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
            .collect();
        Box::pin(async move { Ok(found) })
    }

    fn display_name<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
        _lang: &'a str,
    ) -> PartyFuture<'a, Option<String>> {
        let name = self.names.lock().unwrap().get(&party.0).cloned();
        Box::pin(async move { Ok(name) })
    }

    fn status<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        let known = self.names.lock().unwrap().contains_key(&party.0);
        Box::pin(async move { Ok(known.then_some(PartyStatus::Active)) })
    }

    fn guardians<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Vec<PartyId>> {
        let found = self
            .guardians
            .lock()
            .unwrap()
            .get(&party.0)
            .cloned()
            .unwrap_or_default();
        Box::pin(async move { Ok(found) })
    }
}

/// One invoice the centre drafted.
#[derive(Debug, Clone)]
pub struct Invoiced {
    /// The id it answered with.
    pub id: i64,
    /// What it was asked for.
    pub draft: InvoiceDraft,
    /// Whether the system issued it.
    pub issued: bool,
}

impl Invoiced {
    /// The total with VAT, in fils.
    pub fn gross(&self) -> i64 {
        self.draft.lines.iter().map(|l| l.gross()).sum()
    }
}

/// The invoice book: the invoices drafted and what was paid on them.
#[derive(Clone, Default)]
pub struct Books {
    invoices: Arc<Mutex<Vec<Invoiced>>>,
    payments: Arc<Mutex<Vec<(i64, i64)>>>,
}

impl Books {
    /// `amount` fils were paid on invoice `id`.
    pub fn pay(&self, id: i64, amount: i64) {
        self.payments.lock().unwrap().push((id, amount));
    }

    /// What invoice `id` was paid so far, in fils.
    pub fn paid_of(&self, id: i64) -> i64 {
        self.payments
            .lock()
            .unwrap()
            .iter()
            .filter(|(invoice, _)| *invoice == id)
            .map(|(_, amount)| amount)
            .sum()
    }

    /// Every invoice, oldest first.
    pub fn all(&self) -> Vec<Invoiced> {
        self.invoices.lock().unwrap().clone()
    }
}

impl Invoicer for Books {
    fn draft<'a>(
        &'a self,
        _db: &'a Db,
        draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        let mut all = self.invoices.lock().unwrap();
        let id = 100 + i64::try_from(all.len()).unwrap();
        all.push(Invoiced {
            id,
            draft,
            issued: false,
        });
        Box::pin(async move { Ok(Some(InvoiceId(id))) })
    }

    fn find<'a>(&'a self, _db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        let found = self
            .invoices
            .lock()
            .unwrap()
            .iter()
            .find(|i| i.id == id.0)
            .map(|i| InvoiceSummary {
                id,
                number: i.issued.then(|| format!("INV-{}", i.id)),
                status: if i.issued {
                    InvoiceStatus::Issued
                } else {
                    InvoiceStatus::Draft
                },
                party: i.draft.party,
                legal_entity_id: None,
                currency: i.draft.currency.clone(),
                gross_total: i.gross(),
                balance: i.gross() - self.paid_of(i.id),
                due_date: None,
                disputed: false,
                chasing_paused_until: None,
            });
        Box::pin(async move { Ok(found) })
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

    fn issue<'a>(
        &'a self,
        _state: &'a AppState,
        id: InvoiceId,
        _by: Option<&'a AuthUser>,
    ) -> InvoiceFuture<'a, ()> {
        for invoice in self.invoices.lock().unwrap().iter_mut() {
            if invoice.id == id.0 {
                invoice.issued = true;
            }
        }
        Box::pin(async { Ok(()) })
    }
}

/// The subscriptions started and cancelled.
#[derive(Clone, Default)]
pub struct Subs {
    started: Arc<Mutex<Vec<StartSubscription>>>,
    cancelled: Arc<Mutex<Vec<(Subject, NaiveDate)>>>,
}

impl Subs {
    /// The subscriptions started, oldest first.
    pub fn started(&self) -> Vec<StartSubscription> {
        self.started.lock().unwrap().clone()
    }

    /// The agreements whose subscriptions were cancelled, with the day.
    pub fn cancelled(&self) -> Vec<(Subject, NaiveDate)> {
        self.cancelled.lock().unwrap().clone()
    }
}

impl Subscriptions for Subs {
    fn start<'a>(
        &'a self,
        _state: &'a AppState,
        start: StartSubscription,
    ) -> SubscriptionFuture<'a, Option<SubscriptionId>> {
        let mut all = self.started.lock().unwrap();
        all.push(start);
        let id = i64::try_from(all.len()).unwrap();
        Box::pin(async move { Ok(Some(SubscriptionId(id))) })
    }

    fn cancel<'a>(
        &'a self,
        _state: &'a AppState,
        agreement: &'a Subject,
        cancel: CancelSubscription,
    ) -> SubscriptionFuture<'a, Vec<SubscriptionSummary>> {
        self.cancelled
            .lock()
            .unwrap()
            .push((agreement.clone(), cancel.on));
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The prepaid packages: sold, with credits a test gives.
#[derive(Clone, Default)]
pub struct Packages {
    sold: Arc<Mutex<Vec<SellPackage>>>,
    credits: Arc<Mutex<Vec<(i64, String, i64)>>>,
}

impl Packages {
    /// The packages sold, oldest first.
    pub fn sold(&self) -> Vec<SellPackage> {
        self.sold.lock().unwrap().clone()
    }

    /// `party` holds `credits` of `item` now.
    pub fn give(&self, party: PartyId, item: &str, credits: i64) {
        self.credits
            .lock()
            .unwrap()
            .push((party.0, item.to_owned(), credits));
    }
}

impl PackageSales for Packages {
    fn sell<'a>(
        &'a self,
        _state: &'a AppState,
        sale: SellPackage,
    ) -> SalesFuture<'a, Option<PackageId>> {
        let mut all = self.sold.lock().unwrap();
        all.push(sale);
        let id = i64::try_from(all.len()).unwrap();
        Box::pin(async move { Ok(Some(PackageId(id))) })
    }
}

impl PackageLedger for Packages {
    fn consume<'a>(
        &'a self,
        _state: &'a AppState,
        _request: wl_kernel::sales::ConsumePackage,
    ) -> SalesFuture<'a, PackageConsumption> {
        Box::pin(async { Ok(PackageConsumption::NoCredit) })
    }

    fn balance<'a>(&'a self, _db: &'a Db, party: PartyId, item: &'a str) -> SalesFuture<'a, i64> {
        let left = self
            .credits
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(holder, code, _)| *holder == party.0 && code == item)
            .map_or(0, |(_, _, credits)| *credits);
        Box::pin(async move { Ok(left) })
    }
}
