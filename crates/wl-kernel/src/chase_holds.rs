//! Holds on chasing what a record is owed (01 Property §7.1 *Lease*, "Legal
//! case flag"; BL-PRP-13).
//!
//! While a legal case is open on a lease, the reminders on its invoices are
//! paused (as for an invoice `dispute`), and the automatic messages about
//! what is owed are not sent. The lease belongs to a vertical, the reminders
//! to `wl-invoicing` and `wl-payments`, so the vertical registers a
//! [`ChaseHolds`] provider for the type of its subjects
//! ([`register_chase_holds`], e.g. `property.lease`) and the module that
//! chases asks [`hold`] with the source of the invoice or the agreement of the
//! cheque. A source with no provider is [`ChaseHold::Free`].
//!
//! After a hold ends, chasing resumes from the next scheduled step: the steps
//! whose day was reached while it lasted are not sent ([`ChaseHold::Resumed`]
//! carries the day it ended).

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use renox::chrono::NaiveDate;
use renox::prelude::*;

use crate::subject::Subject;

/// Where chasing for a source stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChaseHold {
    /// Nothing holds it: chase as usual.
    Free,
    /// Held: send no automatic reminder.
    Held,
    /// The last hold ended on this day: chase again, but the steps whose day
    /// is on or before it are not sent.
    Resumed {
        /// The day the last hold ended (Asia/Dubai).
        on: NaiveDate,
    },
}

impl ChaseHold {
    /// Whether chasing is held.
    pub fn is_held(self) -> bool {
        self == ChaseHold::Held
    }
}

/// The future a [`ChaseHolds`] method returns.
pub type HoldFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Says whether chasing for the records of one subject type is held.
pub trait ChaseHolds: Send + Sync + 'static {
    /// The hold on chasing for `source`, a subject of the registered type.
    fn hold<'a>(&'a self, db: &'a Db, source: &'a Subject) -> HoldFuture<'a, ChaseHold>;
}

static PROVIDERS: RwLock<BTreeMap<String, Arc<dyn ChaseHolds>>> = RwLock::new(BTreeMap::new());

/// Adds `provider` for the subjects of `subject_type` (`property.lease`) to
/// the process-wide registry; registering the same type again replaces it.
pub fn register_chase_holds(subject_type: &str, provider: impl ChaseHolds) {
    PROVIDERS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(subject_type.to_owned(), Arc::new(provider));
}

/// The hold on chasing for `source`: its provider's answer, else
/// [`ChaseHold::Free`].
///
/// # Errors
///
/// The provider's errors.
pub async fn hold(db: &Db, source: &Subject) -> Result<ChaseHold> {
    let provider = PROVIDERS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&source.subject_type)
        .cloned();
    match provider {
        Some(provider) => provider.hold(db, source).await,
        None => Ok(ChaseHold::Free),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A provider that holds the even ids, resumed on a day for 3, free else.
    struct Evens;

    impl ChaseHolds for Evens {
        fn hold<'a>(&'a self, _db: &'a Db, source: &'a Subject) -> HoldFuture<'a, ChaseHold> {
            Box::pin(async move {
                Ok(match source.subject_id {
                    id if id % 2 == 0 => ChaseHold::Held,
                    3 => ChaseHold::Resumed {
                        on: NaiveDate::from_ymd_opt(2027, 3, 1).expect("a date"),
                    },
                    _ => ChaseHold::Free,
                })
            })
        }
    }

    #[renox::test]
    async fn a_registered_type_answers_and_any_other_is_free() {
        let app = renox::testing::TestApp::new(App::new()).await;
        register_chase_holds("test.chase_thing", Evens);
        let db = app.db();
        let thing = |id| Subject::new("test.chase_thing", id);
        assert_eq!(hold(db, &thing(2)).await.unwrap(), ChaseHold::Held);
        assert_eq!(
            hold(db, &thing(3)).await.unwrap(),
            ChaseHold::Resumed {
                on: NaiveDate::from_ymd_opt(2027, 3, 1).unwrap()
            }
        );
        assert_eq!(hold(db, &thing(5)).await.unwrap(), ChaseHold::Free);
        let other = Subject::new("test.other_thing", 2);
        assert_eq!(hold(db, &other).await.unwrap(), ChaseHold::Free);
    }

    #[test]
    fn only_a_held_source_is_held() {
        let day = NaiveDate::from_ymd_opt(2027, 3, 1).unwrap();
        assert!(ChaseHold::Held.is_held());
        assert!(!ChaseHold::Free.is_held());
        assert!(!ChaseHold::Resumed { on: day }.is_held());
    }
}
