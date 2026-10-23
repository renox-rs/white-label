//! The stand-in of wl-inventory's stock service: the store of a workshop, the
//! items it knows and their balances. It keeps what it was asked in memory so
//! a test can read it back.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::inventory::{
    ConsumeRequest, Consumed, ReturnRequest, Returned, StockChoice, StockFuture, StockService,
};

/// The balances of the items by code, and the requests made.
#[derive(Default)]
struct Inner {
    items: BTreeMap<String, (i64, i64)>,
    consumed: Vec<ConsumeRequest>,
    returned: Vec<ReturnRequest>,
    store: bool,
}

/// The stock of the workshop's store.
#[derive(Clone, Default)]
pub struct Stock(Arc<Mutex<Inner>>);

impl Stock {
    /// The workshop has a store (`inventory.location:1`); without it a part
    /// is a charge only.
    pub fn with_store(&self) {
        self.0.lock().unwrap().store = true;
    }

    /// The store holds `quantity_milli` of the item `code` (item number `id`).
    pub fn stock(&self, id: i64, code: &str, quantity_milli: i64) {
        self.0
            .lock()
            .unwrap()
            .items
            .insert(code.to_owned(), (id, quantity_milli));
    }

    /// What the store holds of `code`, in thousandths.
    pub fn balance(&self, code: &str) -> i64 {
        self.0
            .lock()
            .unwrap()
            .items
            .get(code)
            .map_or(0, |(_, quantity)| *quantity)
    }

    /// The consumptions asked for, oldest first.
    pub fn consumed(&self) -> Vec<ConsumeRequest> {
        self.0.lock().unwrap().consumed.clone()
    }

    /// The returns asked for, oldest first.
    pub fn returned(&self) -> Vec<ReturnRequest> {
        self.0.lock().unwrap().returned.clone()
    }

    fn code_of(&self, item: &Subject) -> Option<String> {
        self.0
            .lock()
            .unwrap()
            .items
            .iter()
            .find(|(_, (id, _))| *id == item.subject_id)
            .map(|(code, _)| code.clone())
    }
}

impl StockService for Stock {
    fn consume<'a>(
        &'a self,
        _state: &'a AppState,
        request: ConsumeRequest,
    ) -> StockFuture<'a, Consumed> {
        let code = self.code_of(&request.item);
        let mut inner = self.0.lock().unwrap();
        let available = code
            .as_ref()
            .and_then(|code| inner.items.get(code))
            .map_or(0, |(_, quantity)| *quantity);
        let posted = available.min(request.quantity_milli);
        if let Some(code) = &code
            && let Some(entry) = inner.items.get_mut(code)
        {
            entry.1 -= posted;
        }
        let short = request.quantity_milli - posted;
        inner.consumed.push(request);
        Box::pin(async move {
            Ok(Consumed {
                posted_milli: posted,
                short_milli: short,
                ..Consumed::default()
            })
        })
    }

    fn store_of<'a>(
        &'a self,
        _state: &'a AppState,
        _scope_kind: &'a str,
        _scope_id: i64,
    ) -> StockFuture<'a, Option<Subject>> {
        let store = self
            .0
            .lock()
            .unwrap()
            .store
            .then(|| Subject::new("inventory.location", 1));
        Box::pin(async move { Ok(store) })
    }

    fn choices<'a>(
        &'a self,
        _state: &'a AppState,
        code: &'a str,
        _location: &'a Subject,
    ) -> StockFuture<'a, Option<StockChoice>> {
        let found = self
            .0
            .lock()
            .unwrap()
            .items
            .get(code)
            .map(|(id, _)| StockChoice {
                item: Subject::new("inventory.item", *id),
                lot_tracked: false,
                lots: Vec::new(),
            });
        Box::pin(async move { Ok(found) })
    }

    fn return_to_stock<'a>(
        &'a self,
        _state: &'a AppState,
        request: ReturnRequest,
    ) -> StockFuture<'a, Returned> {
        let code = self.code_of(&request.item);
        let mut inner = self.0.lock().unwrap();
        if let Some(code) = &code
            && let Some(entry) = inner.items.get_mut(code)
        {
            entry.1 += request.quantity_milli;
        }
        let posted = request.quantity_milli;
        inner.returned.push(request);
        Box::pin(async move {
            Ok(Returned {
                posted_milli: posted,
            })
        })
    }
}
