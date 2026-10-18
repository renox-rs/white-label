//! The held-funds contract (wl-parties §5.4, §7): which money a module holds
//! for a party, so the freeze task of a confirmed sanctions match can be
//! pre-filled.
//!
//! A module that holds a party's money (deposits, prepaid credit, open
//! refunds, ...) registers a [`HeldFundsSource`] in its `register()`, the way
//! agreement kinds are registered in [`block_effects`](crate::block_effects):
//!
//! ```ignore
//! HeldFundsSource::new("deposits")
//!     .owner("wl-payments")
//!     .label_key("wl_payments.held_funds.deposits")
//!     .source(DepositFunds)
//!     .register();
//!
//! let funds = held_funds::held_for_all(&state, party).await?;
//! ```
//!
//! The contract:
//! - **Read only.** [`FundsSource::held_for`] lists; it never moves, freezes
//!   or releases money. Freezing is the compliance role's decision and is
//!   recorded in the freeze task.
//! - The list is a suggestion. The compliance role confirms, edits or adds
//!   lines by hand; nothing here is authoritative.
//! - Amounts are integer fils in `amount_fils`, with the ISO currency code.
//! - An `Err` from a source is returned to the caller, never swallowed: a
//!   silently missing source would hide held money.
//!
//! The registry is process-wide. Registering the same key again replaces the
//! earlier registration; no source registered means an empty list.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use renox::{AppState, Result};
use serde::{Deserialize, Serialize};

use crate::parties::PartyId;

/// One line of money a module holds for a party.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldFund {
    /// A one-line label for the task, e.g. `Deposit D-2026-0004`.
    pub label: String,
    /// The kind of money: `deposit`, `prepaid_credit`, `refund`, ...
    pub kind: String,
    /// The amount in fils (or the minor unit of `currency`).
    pub amount_fils: i64,
    /// The ISO 4217 currency code, e.g. `AED`.
    pub currency: String,
    /// The key of the [`HeldFundsSource`] that reported the line. Sources
    /// leave it empty; [`held_for_all`] fills it.
    #[serde(default)]
    pub source: String,
}

impl HeldFund {
    /// A line with no source key yet.
    pub fn new(
        label: impl Into<String>,
        kind: impl Into<String>,
        amount_fils: i64,
        currency: impl Into<String>,
    ) -> Self {
        HeldFund {
            label: label.into(),
            kind: kind.into(),
            amount_fils,
            currency: currency.into(),
            source: String::new(),
        }
    }
}

/// The future a [`FundsSource`] method returns.
pub type FundsFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// The owning module's side: it lists what it holds for a party.
pub trait FundsSource: Send + Sync + 'static {
    /// The money this module holds for `party`; empty when none.
    fn held_for<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> FundsFuture<'a, Vec<HeldFund>>;
}

#[derive(Clone)]
struct Registered {
    owner: Option<String>,
    label_key: Option<String>,
    source: Arc<dyn FundsSource>,
}

/// A registered source as the settings list shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct SourceInfo {
    /// The source's key.
    pub key: String,
    /// The owning module.
    pub owner: String,
    /// The text key of the label; `wl_kernel.held_funds.<key>` unless set.
    pub label_key: String,
}

impl fmt::Debug for SourceInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourceInfo")
            .field("key", &self.key)
            .field("owner", &self.owner)
            .field("label_key", &self.label_key)
            .finish()
    }
}

/// The default text key of a source's label.
pub fn label_key(key: &str) -> String {
    format!("wl_kernel.held_funds.{key}")
}

static BOOK: RwLock<BTreeMap<String, Registered>> = RwLock::new(BTreeMap::new());

/// Registers a held-funds source: the owning module's side. Registering the
/// same key again replaces the earlier registration.
pub struct HeldFundsSource {
    key: String,
    owner: Option<String>,
    label_key: Option<String>,
    source: Option<Arc<dyn FundsSource>>,
}

impl HeldFundsSource {
    /// A source with this key, e.g. `deposits`.
    pub fn new(key: impl Into<String>) -> Self {
        HeldFundsSource {
            key: key.into(),
            owner: None,
            label_key: None,
            source: None,
        }
    }

    /// The owning module, e.g. `wl-payments`.
    pub fn owner(mut self, owner: impl Into<String>) -> Self {
        self.owner = Some(owner.into());
        self
    }

    /// The text key of the source's label.
    pub fn label_key(mut self, key: impl Into<String>) -> Self {
        self.label_key = Some(key.into());
        self
    }

    /// Lists the held money.
    pub fn source(mut self, source: impl FundsSource) -> Self {
        self.source = Some(Arc::new(source));
        self
    }

    /// Adds the source to the process-wide registry. A registration without
    /// a [`source`](Self::source) is ignored.
    pub fn register(self) {
        let Some(source) = self.source else { return };
        let entry = Registered {
            owner: self.owner,
            label_key: self.label_key,
            source,
        };
        BOOK.write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(self.key, entry);
    }
}

fn snapshot() -> Vec<(String, Registered)> {
    BOOK.read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Every registered source, by key.
pub fn sources() -> Vec<SourceInfo> {
    snapshot()
        .into_iter()
        .map(|(key, r)| SourceInfo {
            owner: r.owner.unwrap_or_default(),
            label_key: r.label_key.unwrap_or_else(|| label_key(&key)),
            key,
        })
        .collect()
}

/// What every registered source holds for `party`, in source-key order, each
/// line carrying its source key. Empty when no source is registered. The
/// first source error is returned.
pub async fn held_for_all(state: &AppState, party: PartyId) -> Result<Vec<HeldFund>> {
    let mut out = Vec::new();
    for (key, registered) in snapshot() {
        for mut fund in registered.source.held_for(state, party).await? {
            fund.source = key.clone();
            out.push(fund);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Vec<HeldFund>);

    impl FundsSource for Fixed {
        fn held_for<'a>(
            &'a self,
            _state: &'a AppState,
            _party: PartyId,
        ) -> FundsFuture<'a, Vec<HeldFund>> {
            Box::pin(async move { Ok(self.0.clone()) })
        }
    }

    #[test]
    fn a_fund_line_has_no_source_until_collected() {
        let f = HeldFund::new("Deposit D-1", "deposit", 150_000, "AED");
        assert_eq!(f.source, "");
        assert_eq!(f.amount_fils, 150_000);
    }

    #[test]
    fn the_default_label_key_names_the_source() {
        assert_eq!(label_key("deposits"), "wl_kernel.held_funds.deposits");
    }

    #[test]
    fn registering_lists_the_source_and_replaces_the_same_key() {
        HeldFundsSource::new("kernel_test_a")
            .owner("wl-test")
            .source(Fixed(vec![]))
            .register();
        HeldFundsSource::new("kernel_test_a")
            .owner("wl-other")
            .label_key("custom.key")
            .source(Fixed(vec![]))
            .register();
        let found: Vec<_> = sources()
            .into_iter()
            .filter(|s| s.key == "kernel_test_a")
            .collect();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].owner, "wl-other");
        assert_eq!(found[0].label_key, "custom.key");
    }

    #[test]
    fn a_registration_without_a_source_is_ignored() {
        HeldFundsSource::new("kernel_test_none").register();
        assert!(sources().iter().all(|s| s.key != "kernel_test_none"));
    }

    #[test]
    fn an_unset_label_key_defaults_to_the_kernel_key() {
        HeldFundsSource::new("kernel_test_b")
            .source(Fixed(vec![HeldFund::new(
                "Credit",
                "prepaid_credit",
                5,
                "AED",
            )]))
            .register();
        let found = sources().into_iter().find(|s| s.key == "kernel_test_b");
        assert_eq!(
            found.map(|s| s.label_key).as_deref(),
            Some("wl_kernel.held_funds.kernel_test_b")
        );
    }
}
