//! The scope of a record read **inside the caller's transaction**, for the
//! search documents of records that take their scope from another record
//! (an invoice from its lease, a ticket from its unit).
//!
//! [`crate::scopes::scope_of`] reads through the pool, outside any
//! transaction: inside the transaction that indexes a record it waits for a
//! second connection, which a single SQLite connection never gives (the
//! indexing deadlocks). [`scope_in`] reads everything through `tx`:
//!
//! 1. the scope of the record's registered
//!    [`SubjectMachine`](crate::SubjectMachine) (a lease: its building);
//! 2. a resolver registered here with [`register_scope_in`], for a type
//!    without a machine scope;
//! 3. the scope of the record's own search document (a unit indexed in its
//!    building), which the record's module wrote earlier;
//!
//! and goes on to the next candidate while a record has none (global).
//!
//! ```ignore
//! let scope = wl_kernel::search::scope_in(tx, &[invoice.source(), unit]).await?;
//! SearchDocument::new(number).scope(scope)
//! ```

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use renox::auth::permissions::Scope;
use renox::db::Transaction;
use renox::prelude::*;

use super::SearchDocumentRow;
use crate::machines::{self, BoxFuture};
use crate::subject::Subject;

type ResolverFn =
    Arc<dyn for<'a> Fn(&'a mut Transaction, i64) -> BoxFuture<'a, Result<Scope>> + Send + Sync>;

static RESOLVERS: RwLock<BTreeMap<String, ResolverFn>> = RwLock::new(BTreeMap::new());

/// Registers how to find the scope of record `id` of `subject_type` in a
/// transaction, for a type whose machine gives no scope. Registering the
/// same type again replaces the resolver.
pub fn register_scope_in<F>(subject_type: &str, resolver: F)
where
    F: for<'a> Fn(&'a mut Transaction, i64) -> BoxFuture<'a, Result<Scope>> + Send + Sync + 'static,
{
    RESOLVERS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(subject_type.to_owned(), Arc::new(resolver));
}

/// The scope of `subject`, read in `tx`: its machine's, else its resolver's,
/// else its search document's; global when none gives one. A record that
/// is gone (or a source a test made up) is global.
pub async fn scope_of_in(tx: &mut Transaction, subject: &Subject) -> Result<Scope> {
    if let Some(machine) = machines::get(&subject.subject_type).filter(|m| m.has_scope()) {
        match machine.scope_in(tx, subject.subject_id).await {
            Ok(scope) if !scope.is_global() => return Ok(scope),
            Ok(_) | Err(Error::NotFound) => {}
            Err(error) => return Err(error),
        }
    }
    let resolver = RESOLVERS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&subject.subject_type)
        .cloned();
    if let Some(resolver) = resolver {
        match resolver(tx, subject.subject_id).await {
            Ok(scope) if !scope.is_global() => return Ok(scope),
            Ok(_) | Err(Error::NotFound) => {}
            Err(error) => return Err(error),
        }
    }
    let document = SearchDocumentRow::query()
        .where_eq("subject_type", subject.subject_type.as_str())
        .where_eq("subject_id", subject.subject_id)
        .first(&mut *tx)
        .await?;
    Ok(match document {
        Some(document) if !document.scope_type.is_empty() => {
            Scope::new(&document.scope_type, &document.scope_id)
        }
        _ => Scope::global(),
    })
}

/// The scope of the first of `candidates` that has one ([`scope_of_in`]),
/// read in `tx`; global when none has one.
pub async fn scope_in(tx: &mut Transaction, candidates: &[Subject]) -> Result<Scope> {
    for subject in candidates {
        let scope = scope_of_in(tx, subject).await?;
        if !scope.is_global() {
            return Ok(scope);
        }
    }
    Ok(Scope::global())
}
