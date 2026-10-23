//! The stand-ins the workshop talks to through the kernel's contracts: the
//! party directory (customers, fleets, owners) and the invoice book. They keep
//! what they were asked in memory so a test can read it back.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::Subject;
use wl_kernel::invoicing::{
    InvoiceDraft, InvoiceFuture, InvoiceId, InvoiceStatus, InvoiceSummary, Invoicer,
};
use wl_kernel::notify::{ContactKind, PartyContactPoint, PartyContacts};
use wl_kernel::parties::{PartyDirectory, PartyFuture, PartyKind, PartyStatus, PartySummary};

/// The parties the workshop knows, with the ones blocked.
#[derive(Clone, Default)]
pub struct People {
    names: Arc<Mutex<BTreeMap<i64, String>>>,
    blocked: Arc<Mutex<Vec<i64>>>,
    no_mobile: Arc<Mutex<Vec<i64>>>,
}

impl People {
    /// Party `id` is called `name`.
    pub fn add(&self, id: i64, name: &str) {
        self.names.lock().unwrap().insert(id, name.to_owned());
    }

    /// Party `id` is blocked (`true`) or active again (`false`).
    pub fn set_blocked(&self, id: i64, blocked: bool) {
        let mut all = self.blocked.lock().unwrap();
        all.retain(|party| *party != id);
        if blocked {
            all.push(id);
        }
    }

    /// Party `id` has no mobile number (`true`) or has one (`false`).
    pub fn set_no_mobile(&self, id: i64, none: bool) {
        let mut all = self.no_mobile.lock().unwrap();
        all.retain(|party| *party != id);
        if none {
            all.push(id);
        }
    }

    fn status_of(&self, id: i64) -> PartyStatus {
        if self.blocked.lock().unwrap().contains(&id) {
            PartyStatus::Blocked
        } else {
            PartyStatus::Active
        }
    }

    fn summary(&self, id: i64) -> Option<PartySummary> {
        let status = self.status_of(id);
        self.names
            .lock()
            .unwrap()
            .get(&id)
            .map(|name| PartySummary {
                id: PartyId(id),
                code: format!("P-{id:05}"),
                kind: PartyKind::Person,
                display_name: name.clone(),
                status,
                display_block_reason: (status == PartyStatus::Blocked)
                    .then(|| "on_hold".to_owned()),
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

    fn contacts<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
    ) -> PartyFuture<'a, Option<PartyContacts>> {
        let known = self.names.lock().unwrap().contains_key(&party.0);
        let mobile = !self.no_mobile.lock().unwrap().contains(&party.0);
        let found = known.then(|| PartyContacts {
            party,
            language: "en".to_owned(),
            preferred_channel: None,
            points: if mobile {
                vec![PartyContactPoint {
                    id: party.0,
                    kind: ContactKind::Mobile,
                    address: "+971500000000".to_owned(),
                    primary: true,
                    verified: true,
                    valid: true,
                }]
            } else {
                Vec::new()
            },
        });
        Box::pin(async move { Ok(found) })
    }

    fn status<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        let known = self.names.lock().unwrap().contains_key(&party.0);
        let status = self.status_of(party.0);
        Box::pin(async move { Ok(known.then_some(status)) })
    }
}

/// One invoice the workshop drafted.
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

    /// The total before VAT, in fils.
    pub fn net(&self) -> i64 {
        self.draft.lines.iter().map(|l| l.net()).sum()
    }

    /// The party billed.
    pub fn party(&self) -> PartyId {
        self.draft.party
    }

    /// The record it bills.
    pub fn source(&self) -> &Subject {
        &self.draft.source
    }
}

/// The invoice book: the invoices drafted and what was paid on them.
#[derive(Clone, Default)]
pub struct Books {
    invoices: Arc<Mutex<Vec<Invoiced>>>,
    payments: Arc<Mutex<Vec<(i64, i64)>>>,
    failing: Arc<Mutex<bool>>,
}

impl Books {
    /// Makes every further draft fail (or work again).
    pub fn fail(&self, failing: bool) {
        *self.failing.lock().unwrap() = failing;
    }

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

    /// The invoices about `source`, oldest first.
    pub fn about(&self, source: &Subject) -> Vec<Invoiced> {
        self.all()
            .into_iter()
            .filter(|invoice| invoice.draft.source == *source)
            .collect()
    }
}

impl Invoicer for Books {
    fn draft<'a>(
        &'a self,
        _db: &'a Db,
        draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        if *self.failing.lock().unwrap() {
            return Box::pin(async {
                Err(Error::Status(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "the invoice book is down".to_owned(),
                ))
            });
        }
        let mut all = self.invoices.lock().unwrap();
        let id = 100 + i64::try_from(all.len()).unwrap();
        all.push(Invoiced {
            id,
            draft,
            issued: false,
        });
        Box::pin(async move { Ok(Some(InvoiceId(id))) })
    }

    fn discard_draft<'a>(&'a self, _db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, bool> {
        let mut all = self.invoices.lock().unwrap();
        let before = all.len();
        all.retain(|i| !(i.id == id.0 && !i.issued));
        let removed = all.len() < before;
        Box::pin(async move { Ok(removed) })
    }

    fn find<'a>(&'a self, _db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        let found = self
            .invoices
            .lock()
            .unwrap()
            .iter()
            .find(|i| i.id == id.0)
            .map(|i| {
                let balance = i.gross() - self.paid_of(i.id);
                let status = if !i.issued {
                    InvoiceStatus::Draft
                } else if balance == 0 && i.gross() > 0 {
                    InvoiceStatus::Paid
                } else if balance < i.gross() {
                    InvoiceStatus::PartiallyPaid
                } else {
                    InvoiceStatus::Issued
                };
                InvoiceSummary {
                    id,
                    number: i.issued.then(|| format!("INV-{}", i.id)),
                    status,
                    party: i.draft.party,
                    legal_entity_id: Some(1),
                    currency: i.draft.currency.clone(),
                    gross_total: i.gross(),
                    balance,
                    due_date: None,
                    disputed: false,
                    chasing_paused_until: None,
                }
            });
        Box::pin(async move { Ok(found) })
    }

    fn billing<'a>(
        &'a self,
        _db: &'a Db,
        origin: &'a Subject,
    ) -> InvoiceFuture<'a, Vec<InvoiceId>> {
        let found: Vec<InvoiceId> = self
            .invoices
            .lock()
            .unwrap()
            .iter()
            .filter(|i| {
                i.issued
                    && i.draft
                        .lines
                        .iter()
                        .any(|l| l.origin.as_ref() == Some(origin))
            })
            .map(|i| InvoiceId(i.id))
            .collect();
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
