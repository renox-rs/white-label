//! The scope of a subject: where the holders of a role are looked for
//! (wl-kernel §4.1, wl-notify staff tasks).
//!
//! One lookup serves approvals and the modules that need "the people
//! responsible for this record": [`scope_of`] tries, in this order,
//!
//! 1. the scope of the subject's registered [`SubjectMachine`](crate::SubjectMachine),
//! 2. a resolver registered here with [`register`] (for records without a
//!    machine, e.g. a ticket takes its subject's scope),
//! 3. the global scope.
//!
//! ```ignore
//! wl_kernel::scopes::register("ticketing.ticket", |db, id| {
//!     Box::pin(async move { ticket_scope(db, id).await })
//! });
//! ```

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use renox::auth::permissions::Scope;
use renox::prelude::*;

use crate::machines::{self, BoxFuture};
use crate::subject::Subject;

/// The role of the company administrators: whom a task goes to when nobody
/// holds the role it is meant for. The host's `roles::ADMIN` is this role.
pub const COMPANY_ADMIN_ROLE: &str = "admin";

type ResolverFn = Arc<dyn for<'a> Fn(&'a Db, i64) -> BoxFuture<'a, Result<Scope>> + Send + Sync>;

static RESOLVERS: RwLock<BTreeMap<String, ResolverFn>> = RwLock::new(BTreeMap::new());

/// Registers how to find the scope of record `id` of `subject_type`.
/// Registering the same subject type again replaces the resolver. A machine
/// with a scope for the same type wins over it.
pub fn register<F>(subject_type: &str, resolver: F)
where
    F: for<'a> Fn(&'a Db, i64) -> BoxFuture<'a, Result<Scope>> + Send + Sync + 'static,
{
    RESOLVERS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(subject_type.to_owned(), Arc::new(resolver));
}

/// The scope of `subject`: its registered machine's, else its registered
/// resolver's, else global.
pub async fn scope_of(db: &Db, subject: &Subject) -> Result<Scope> {
    if let Some(machine) = machines::get(&subject.subject_type).filter(|m| m.has_scope()) {
        let mut tx = db.begin().await?;
        let scope = machine.scope_in(&mut tx, subject.subject_id).await?;
        tx.commit().await?;
        return Ok(scope);
    }
    let resolver = RESOLVERS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&subject.subject_type)
        .cloned();
    match resolver {
        Some(resolver) => resolver(db, subject.subject_id).await,
        None => Ok(Scope::global()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::testing::TestApp;

    #[renox::test]
    async fn an_unknown_subject_is_global() {
        let app = TestApp::new(App::new()).await;
        let scope = scope_of(app.db(), &Subject::new("scopes_test.unknown", 1))
            .await
            .unwrap();
        assert_eq!(scope, Scope::global());
    }

    #[renox::test]
    async fn a_registered_resolver_gives_the_scope() {
        let app = TestApp::new(App::new()).await;
        register("scopes_test.ticket", |_db, id| {
            Box::pin(async move {
                Ok(if id == 42 {
                    Scope::of_id::<ScopeEntity>(7)
                } else {
                    Scope::global()
                })
            })
        });
        let tower = scope_of(app.db(), &Subject::new("scopes_test.ticket", 42))
            .await
            .unwrap();
        assert_eq!(tower, Scope::of_id::<ScopeEntity>(7));
        let other = scope_of(app.db(), &Subject::new("scopes_test.ticket", 1))
            .await
            .unwrap();
        assert_eq!(other, Scope::global());
    }

    #[renox::test]
    async fn registering_again_replaces_the_resolver() {
        let app = TestApp::new(App::new()).await;
        register("scopes_test.replaced", |_db, _id| {
            Box::pin(async { Ok(Scope::of_id::<ScopeEntity>(1)) })
        });
        register("scopes_test.replaced", |_db, _id| {
            Box::pin(async { Ok(Scope::of_id::<ScopeEntity>(2)) })
        });
        let scope = scope_of(app.db(), &Subject::new("scopes_test.replaced", 5))
            .await
            .unwrap();
        assert_eq!(scope, Scope::of_id::<ScopeEntity>(2));
    }

    use crate::entities::ScopeEntity;
}
