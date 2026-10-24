//! The fields of a record an assistant reads with the `fetch` tool
//! (BL-AGT-07): a module registers, per subject type, the fields a user who
//! may open the record sees on its page, masked as on screen, with the free
//! text written by outsiders marked ([`RecordFields::untrusted`]).
//!
//! A type without fields still answers `fetch` from its search document
//! (title, line, status, identifiers that are not sensitive, link).
//!
//! ```ignore
//! wl_kernel::connector::fetch::register_fields(TICKET_SUBJECT_TYPE, |db, id| {
//!     Box::pin(async move {
//!         let Some(ticket) = Ticket::find(db, id).await? else { return Ok(None) };
//!         Ok(Some(RecordFields::new()
//!             .text("Priority", ticket.priority.as_str())
//!             .untrusted("Description", &ticket.description)))
//!     })
//! });
//! ```

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use renox::prelude::*;
use renox::serde_json::Value;
use serde::Serialize;

use super::untrusted::untrusted;
use crate::agent::Money;
use crate::machines::BoxFuture;

/// One field of a record as a tool returns it.
#[derive(Debug, Clone, Serialize)]
pub struct RecordField {
    /// What it is, as on the page (`Due date`).
    pub label: String,
    /// Its value: text, an amount ([`Money`]) or an untrusted-text marker.
    pub value: Value,
}

/// The fields of one record, in the order the page shows them.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RecordFields {
    fields: Vec<RecordField>,
}

impl RecordFields {
    /// No fields yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// A field written by the company (a number, a status, a date).
    pub fn text(mut self, label: &str, value: impl Into<String>) -> Self {
        let value = value.into();
        if !value.trim().is_empty() {
            self.fields.push(RecordField {
                label: label.to_owned(),
                value: Value::String(value),
            });
        }
        self
    }

    /// An amount in `currency`'s minor unit (fils for AED), returned with the
    /// currency and the number of decimals stated.
    pub fn money(mut self, label: &str, minor: i64, currency: &str) -> Self {
        let money = Money::new(minor, currency);
        self.fields.push(RecordField {
            label: label.to_owned(),
            value: renox::serde_json::to_value(money).unwrap_or(Value::Null),
        });
        self
    }

    /// Free text written by someone outside the company (a tenant's
    /// description, a portal message, an e-mail body), marked as data.
    pub fn untrusted(mut self, label: &str, value: &str) -> Self {
        if !value.trim().is_empty() {
            self.fields.push(RecordField {
                label: label.to_owned(),
                value: untrusted(value),
            });
        }
        self
    }

    /// Text that is written by outsiders when `external` is true, by the
    /// company otherwise.
    pub fn text_from(self, label: &str, value: &str, external: bool) -> Self {
        if external {
            self.untrusted(label, value)
        } else {
            self.text(label, value)
        }
    }

    /// The fields.
    pub fn fields(&self) -> &[RecordField] {
        &self.fields
    }
}

type FieldsFn =
    Arc<dyn for<'a> Fn(&'a Db, i64) -> BoxFuture<'a, Result<Option<RecordFields>>> + Send + Sync>;

static PROVIDERS: RwLock<BTreeMap<String, FieldsFn>> = RwLock::new(BTreeMap::new());

/// Registers the fields of records of `subject_type`; registering the type
/// again replaces them. The tool calls it only for a record the user may
/// open (its search document is one they may see).
pub fn register_fields<F>(subject_type: &str, fields: F)
where
    F: for<'a> Fn(&'a Db, i64) -> BoxFuture<'a, Result<Option<RecordFields>>>
        + Send
        + Sync
        + 'static,
{
    PROVIDERS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(subject_type.to_owned(), Arc::new(fields));
}

/// The registered fields of record `id` of `subject_type`; `None` when the
/// type registered none or the record is gone.
pub async fn fields_of(db: &Db, subject_type: &str, id: i64) -> Result<Option<RecordFields>> {
    let provider = PROVIDERS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(subject_type)
        .cloned();
    match provider {
        Some(provider) => provider(db, id).await,
        None => Ok(None),
    }
}
