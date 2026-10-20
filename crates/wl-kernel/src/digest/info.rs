//! What the digest needs to know about a record and a rule: its number, and
//! whether it is confidential (wl-kernel §5.13 step 6; "content follows field
//! visibility: a late entry on a confidential record shows only its type and
//! number"), and the title of a rule to show instead of its id.
//!
//! A module registers both once, in `register()`:
//!
//! ```ignore
//! digest::register_info("bookings.check_in", |tx, id| {
//!     Box::pin(async move { check_in_info(tx, id).await })
//! });
//! digest::register_rule_title("BKG.RUL.02", "no-show rule");
//! ```
//!
//! A record with no registered info is shown by its id and as not
//! confidential.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use renox::Result;
use renox::db::Transaction;

use crate::machines::BoxFuture;
use crate::subject::Subject;

/// What the digest shows of one record.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecordInfo {
    /// The record's number, e.g. `RCP-ACME-2026-0042`.
    pub number: Option<String>,
    /// Whether the record is confidential: the digest then shows only its
    /// type and number.
    pub confidential: bool,
}

impl RecordInfo {
    /// A record with this number, not confidential.
    pub fn numbered(number: &str) -> Self {
        RecordInfo {
            number: Some(number.to_owned()),
            confidential: false,
        }
    }

    /// The record is confidential.
    pub fn confidential(mut self) -> Self {
        self.confidential = true;
        self
    }
}

/// The module's handler that tells the number and visibility of record
/// `id`.
pub type InfoFn = Arc<
    dyn for<'a> Fn(&'a mut Transaction, i64) -> BoxFuture<'a, Result<RecordInfo>> + Send + Sync,
>;

static INFOS: RwLock<BTreeMap<String, InfoFn>> = RwLock::new(BTreeMap::new());
static TITLES: RwLock<BTreeMap<String, String>> = RwLock::new(BTreeMap::new());

/// Registers how the digest reads records of `subject_type`; registering the
/// same type again replaces the handler.
pub fn register_info<F>(subject_type: &str, info: F)
where
    F: for<'a> Fn(&'a mut Transaction, i64) -> BoxFuture<'a, Result<RecordInfo>>
        + Send
        + Sync
        + 'static,
{
    INFOS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(subject_type.to_owned(), Arc::new(info));
}

/// The number and visibility of `subject`; a record whose type registered
/// nothing has no number and is not confidential.
pub async fn info_of(tx: &mut Transaction, subject: &Subject) -> Result<RecordInfo> {
    let handler = INFOS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&subject.subject_type)
        .cloned();
    match handler {
        Some(info) => info(tx, subject.subject_id).await,
        None => Ok(RecordInfo::default()),
    }
}

/// Registers the title the digest shows for `rule` (`no-show rule`).
pub fn register_rule_title(rule: &str, title: &str) {
    TITLES
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(rule.to_owned(), title.to_owned());
}

/// The title of `rule`, else its id.
pub fn rule_title(rule: &str) -> String {
    TITLES
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(rule)
        .cloned()
        .unwrap_or_else(|| rule.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rule_without_a_title_shows_its_id() {
        register_rule_title("TST.RUL.77", "no-show rule");
        assert_eq!(rule_title("TST.RUL.77"), "no-show rule");
        assert_eq!(rule_title("TST.RUL.78"), "TST.RUL.78");
    }

    #[test]
    fn info_is_built_with_a_number_and_may_be_confidential() {
        let plain = RecordInfo::numbered("RCP-1");
        assert_eq!(plain.number.as_deref(), Some("RCP-1"));
        assert!(!plain.confidential);
        assert!(plain.confidential().confidential);
        assert_eq!(RecordInfo::default().number, None);
    }
}
