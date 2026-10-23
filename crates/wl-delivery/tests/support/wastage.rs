//! A wastage desk that stands in for wl-inventory: it estimates what a
//! discard is worth (a figure the test sets) and records the discards it is
//! asked to record.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::delivery::{DeliveryFuture, DeliveryWastage, DiscardWastage, WastageRecorded};

/// The recorded discards and the value an estimate gives.
#[derive(Clone, Default)]
pub struct WastageDesk {
    value: Arc<Mutex<i64>>,
    recorded: Arc<Mutex<Vec<DiscardWastage>>>,
}

impl WastageDesk {
    /// The goods are worth `fils` at cost.
    pub fn worth(&self, fils: i64) {
        *self.value.lock().unwrap() = fils;
    }

    /// The discards recorded, oldest first.
    pub fn recorded(&self) -> Vec<DiscardWastage> {
        self.recorded.lock().unwrap().clone()
    }
}

impl DeliveryWastage for WastageDesk {
    fn estimate<'a>(
        &'a self,
        _state: &'a AppState,
        _request: &'a DiscardWastage,
    ) -> DeliveryFuture<'a, i64> {
        let value = *self.value.lock().unwrap();
        Box::pin(async move { Ok(value) })
    }

    fn record<'a>(
        &'a self,
        _state: &'a AppState,
        request: &'a DiscardWastage,
    ) -> DeliveryFuture<'a, Option<WastageRecorded>> {
        let value = *self.value.lock().unwrap();
        let mut recorded = self.recorded.lock().unwrap();
        recorded.push(request.clone());
        let n = i64::try_from(recorded.len()).unwrap_or(1);
        Box::pin(async move {
            Ok(Some(WastageRecorded {
                reference: Some(Subject::new("inventory.stock_movement", n)),
                value,
            }))
        })
    }
}
