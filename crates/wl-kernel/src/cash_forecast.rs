//! The cash-flow forecast extension point (wl-invoicing §8, `FIN.CFL.01`):
//! how a module or a vertical adds expected receipts and payments to the
//! forecast that `wl-invoicing` computes, e.g. 01 owner payouts, 02 retention
//! releases, 03 COD remittances to clients, 06 aggregator payouts, 09 visa
//! and insurance costs, and the payroll runs and final settlements of
//! `wl-hr`.
//!
//! A source is registered under a name with [`register_cash_forecast_source`]
//! and is asked for the lines of one legal entity between two dates. A line
//! names the documents it stands for: when `wl-invoicing` already counts one
//! of them, its own line wins and the source line is dropped, so nothing
//! counts twice. Money is `i64` fils of AED.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::parties::PartyId;
use crate::payments::Direction;
use crate::subject::Subject;

/// One expected receipt or payment a source adds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForecastLine {
    /// What it is, shown in the grid, e.g. `Owner payout, Tower A`.
    pub label: String,
    /// The category, e.g. `owner_payout`, `retention`, `payroll`.
    pub category: String,
    /// The party it is paid to or by, if any.
    pub party: Option<PartyId>,
    /// The day it is expected; one before the first day of the forecast
    /// counts in week 1.
    pub date: NaiveDate,
    /// The amount, in AED fils; above 0.
    pub amount: i64,
    /// Money in ([`Direction::In`]) or out.
    pub direction: Direction,
    /// The documents it stands for; the forecast drops the line when it
    /// already counts one of them.
    pub documents: Vec<Subject>,
    /// Whether the line is a total the viewer may open only as a total
    /// (payroll, final settlements, owner drawings): its party and documents
    /// are not shown.
    pub total_only: bool,
}

impl ForecastLine {
    /// A line of `amount` AED fils expected on `date`.
    pub fn new(label: &str, category: &str, date: NaiveDate, amount: i64, direction: Direction) -> Self {
        ForecastLine {
            label: label.to_owned(),
            category: category.to_owned(),
            party: None,
            date,
            amount,
            direction,
            documents: Vec::new(),
            total_only: false,
        }
    }

    /// Names the party.
    pub fn party(mut self, party: PartyId) -> Self {
        self.party = Some(party);
        self
    }

    /// Names a document it stands for.
    pub fn document(mut self, document: Subject) -> Self {
        self.documents.push(document);
        self
    }

    /// Shows it as a total only.
    pub fn total_only(mut self) -> Self {
        self.total_only = true;
        self
    }
}

/// An entity's projected closing balance falls below its threshold in a week
/// of the horizon (`FIN.RUL.23`). Emitted by the daily check once per entity:
/// again only when the first week below moves earlier, or after a run with no
/// week below. Money is AED fils.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CashForecastBelowThreshold {
    /// The legal entity.
    pub legal_entity_id: i64,
    /// The first week below the threshold, from 1.
    pub week: u32,
    /// The first day of that week.
    pub week_start: NaiveDate,
    /// The projected closing balance of that week.
    pub projected_balance: i64,
    /// The entity's threshold.
    pub threshold: i64,
}

impl Event for CashForecastBelowThreshold {}

/// The future a [`CashForecastSource`] method returns.
pub type ForecastFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Adds expected receipts and payments to the cash-flow forecast.
pub trait CashForecastSource: Send + Sync + 'static {
    /// The lines of legal entity `legal_entity_id` expected from `from` to
    /// `to` (both included), plus the ones overdue before `from`. Read only.
    fn lines<'a>(
        &'a self,
        state: &'a AppState,
        legal_entity_id: i64,
        from: NaiveDate,
        to: NaiveDate,
    ) -> ForecastFuture<'a, Vec<ForecastLine>>;
}

static SOURCES: RwLock<BTreeMap<String, Arc<dyn CashForecastSource>>> =
    RwLock::new(BTreeMap::new());

/// Adds `source` under `name` to the process-wide registry; registering the
/// same name again replaces it.
pub fn register_cash_forecast_source(name: &str, source: impl CashForecastSource) {
    SOURCES
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(name.to_owned(), Arc::new(source));
}

/// The registered sources with their names, sorted by name.
pub fn cash_forecast_sources() -> Vec<(String, Arc<dyn CashForecastSource>)> {
    SOURCES
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|(name, source)| (name.clone(), Arc::clone(source)))
        .collect()
}

/// Removes the source `name`, for tests.
pub fn remove_cash_forecast_source(name: &str) {
    SOURCES
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(name);
}
