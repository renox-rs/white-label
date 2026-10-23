//! The stand-in of wl-inventory's Return to supplier drafts: it keeps the
//! returns it was asked for and answers `inventory.supplier_return:N`.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::inventory::{
    DraftReturn, SUPPLIER_RETURN_SUBJECT_TYPE, StockFuture, SupplierReturnDrafts,
};

/// The returns drafted.
#[derive(Clone, Default)]
pub struct Returns(Arc<Mutex<Vec<DraftReturn>>>);

impl Returns {
    /// Every return drafted, oldest first.
    pub fn all(&self) -> Vec<DraftReturn> {
        self.0.lock().unwrap().clone()
    }
}

impl SupplierReturnDrafts for Returns {
    fn draft_credit<'a>(
        &'a self,
        _state: &'a AppState,
        request: DraftReturn,
    ) -> StockFuture<'a, Option<Subject>> {
        let mut all = self.0.lock().unwrap();
        all.push(request);
        let id = 900 + i64::try_from(all.len()).unwrap();
        Box::pin(async move { Ok(Some(Subject::new(SUPPLIER_RETURN_SUBJECT_TYPE, id))) })
    }
}
