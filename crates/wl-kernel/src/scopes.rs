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
use renox::db::Query;
use renox::prelude::*;

use crate::machines::{self, BoxFuture};
use crate::subject::Subject;

/// The role of the company administrators: whom a task goes to when nobody
/// holds the role it is meant for. The host's `roles::ADMIN` is this role.
pub const COMPANY_ADMIN_ROLE: &str = "admin";

type ResolverFn = Arc<dyn for<'a> Fn(&'a Db, i64) -> BoxFuture<'a, Result<Scope>> + Send + Sync>;

static RESOLVERS: RwLock<BTreeMap<String, ResolverFn>> = RwLock::new(BTreeMap::new());

type LabelFn =
    Arc<dyn for<'a> Fn(&'a Db, &'a str) -> BoxFuture<'a, Result<Option<String>>> + Send + Sync>;

static LABELS: RwLock<BTreeMap<String, LabelFn>> = RwLock::new(BTreeMap::new());

/// Registers how to name a scope of `kind` (the table of its model, as
/// [`Scope::kind`] gives it) for people: `Tower A` for the building with the
/// id the resolver is given. Registering the same kind again replaces the
/// resolver.
pub fn register_label<F>(kind: &str, resolver: F)
where
    F: for<'a> Fn(&'a Db, &'a str) -> BoxFuture<'a, Result<Option<String>>> + Send + Sync + 'static,
{
    LABELS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(kind.to_owned(), Arc::new(resolver));
}

/// The name of `scope` for people: its registered label, else `kind id`; the
/// global scope is "All locations".
pub async fn label_of(db: &Db, scope: &Scope) -> Result<String> {
    if scope.is_global() {
        return Ok(crate::texts::en("wl_kernel.scopes.global"));
    }
    let resolver = LABELS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(scope.kind())
        .cloned();
    if let Some(resolver) = resolver
        && let Some(label) = resolver(db, scope.id()).await?
    {
        return Ok(label);
    }
    Ok(format!("{} {}", scope.kind(), scope.id()))
}

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

type MembersFn =
    Arc<dyn for<'a> Fn(&'a Db, &'a [Scope]) -> BoxFuture<'a, Result<Vec<i64>>> + Send + Sync>;

static MEMBERS: RwLock<BTreeMap<String, (MembersFn, bool)>> = RwLock::new(BTreeMap::new());

/// Registers how to list the records of `subject_type` that lie in some
/// scopes: the ids of the units whose building is one of them, of the leases
/// of those buildings. It is the other direction of [`register`] (record to
/// scope), used to narrow a list ([`Reach::apply`]) to the records of the
/// scopes a user holds a permission in. The resolver is given scopes of every
/// kind and ignores those it does not know. Registering the same type again
/// replaces the resolver.
///
/// This is for a type that belongs to a scope itself (a unit, a lease): a
/// list whose rows name any subject ([`Reach::apply`]) takes the rows of
/// every such type. A type that is only reached through others (an invoice
/// is in a building because it is billed for it) is registered with
/// [`register_derived_members`].
pub fn register_members<F>(subject_type: &str, resolver: F)
where
    F: for<'a> Fn(&'a Db, &'a [Scope]) -> BoxFuture<'a, Result<Vec<i64>>> + Send + Sync + 'static,
{
    MEMBERS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(subject_type.to_owned(), (Arc::new(resolver), true));
}

/// Like [`register_members`], for a type whose resolver is itself built on
/// [`Reach::apply`] (the invoices of the buildings): [`members_in`] asks it,
/// but a list that names any subject ([`Reach::apply`]) does not, which is
/// what keeps the resolvers from calling one another forever.
pub fn register_derived_members<F>(subject_type: &str, resolver: F)
where
    F: for<'a> Fn(&'a Db, &'a [Scope]) -> BoxFuture<'a, Result<Vec<i64>>> + Send + Sync + 'static,
{
    MEMBERS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(subject_type.to_owned(), (Arc::new(resolver), false));
}

/// The ids of the records of `subject_type` in `scopes`; none when nobody
/// registered the type ([`register_members`]).
pub async fn members_in(db: &Db, subject_type: &str, scopes: &[Scope]) -> Result<Vec<i64>> {
    let resolver = MEMBERS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(subject_type)
        .map(|(resolver, _)| resolver.clone());
    match resolver {
        Some(resolver) if !scopes.is_empty() => {
            let mut ids = resolver(db, scopes).await?;
            ids.sort_unstable();
            ids.dedup();
            Ok(ids)
        }
        _ => Ok(Vec::new()),
    }
}

/// Where a user holds a permission: everywhere, or only in some scopes (a
/// building). A list page asks for it ([`reach`]) and shows the records of
/// that reach; 403 only when the user holds the permission nowhere. This is
/// layer 3 of docs/ACCESS-CONTROL.md for lists, and the reason the list
/// routes are behind [`anywhere`] and not behind a global
/// `require_permission` (which counts global roles only).
#[derive(Debug, Clone)]
pub struct Reach {
    all: bool,
    scopes: Vec<Scope>,
}

impl Reach {
    /// A reach of everything, for a global holder.
    pub fn everything() -> Self {
        Self {
            all: true,
            scopes: Vec::new(),
        }
    }

    /// A reach of these scopes only.
    pub fn only(scopes: Vec<Scope>) -> Self {
        Self { all: false, scopes }
    }

    /// Whether the permission is held everywhere (a global role).
    pub fn is_all(&self) -> bool {
        self.all
    }

    /// The scopes of a scoped holder (none for a global holder).
    pub fn scopes(&self) -> &[Scope] {
        &self.scopes
    }

    /// The ids of the records of `subject_type` in reach, `None` for every
    /// one.
    pub async fn ids(&self, db: &Db, subject_type: &str) -> Result<Option<Vec<i64>>> {
        if self.all {
            return Ok(None);
        }
        Ok(Some(members_in(db, subject_type, &self.scopes).await?))
    }

    /// Whether the record `subject` is in reach: its scope is one of the
    /// scopes (a record that belongs to no scope is for global holders only).
    pub async fn contains(&self, db: &Db, subject: &Subject) -> Result<bool> {
        if self.all {
            return Ok(true);
        }
        let scope = scope_of(db, subject).await?;
        Ok(self.scopes.contains(&scope))
    }

    /// Whether the record `id` of the entity called `entity` (`lease`, the
    /// last part of its subject type `property.lease`) is in reach: some
    /// registered type of that name lists it. For records that are named by
    /// their entity only (a document flag's `lease.activate`).
    pub async fn contains_entity(&self, db: &Db, entity: &str, id: i64) -> Result<bool> {
        if self.all {
            return Ok(true);
        }
        let types: Vec<String> = MEMBERS
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(name, (_, leaf))| *leaf && name.rsplit('.').next() == Some(entity))
            .map(|(name, _)| name.clone())
            .collect();
        for subject_type in types {
            if members_in(db, &subject_type, &self.scopes)
                .await?
                .contains(&id)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Keeps the rows of `query` whose subject (`type_col`, `id_col`) is in
    /// reach: for each subject type that has registered its members, rows of
    /// that type with one of the ids. A row without a subject, or of a type
    /// nobody registered, is for global holders only.
    pub async fn apply<M: Model>(
        &self,
        db: &Db,
        query: Query<M>,
        type_col: &str,
        id_col: &str,
    ) -> Result<Query<M>> {
        if self.all {
            return Ok(query);
        }
        let types: Vec<String> = MEMBERS
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(_, (_, leaf))| *leaf)
            .map(|(name, _)| name.clone())
            .collect();
        let mut groups: Vec<(String, Vec<i64>)> = Vec::new();
        for subject_type in types {
            let ids = members_in(db, &subject_type, &self.scopes).await?;
            if !ids.is_empty() {
                groups.push((subject_type, ids));
            }
        }
        if groups.is_empty() {
            return Ok(query.none());
        }
        Ok(query.where_any(|mut any| {
            for (subject_type, ids) in groups {
                any =
                    any.where_all(|all| all.where_eq(type_col, subject_type).where_in(id_col, ids));
            }
            any
        }))
    }

    /// Keeps the rows of `query` whose own `id_col` is the id of a record of
    /// `subject_type` in reach (the parties of the buildings, the invoices of
    /// the buildings).
    pub async fn apply_ids<M: Model>(
        &self,
        db: &Db,
        query: Query<M>,
        id_col: &str,
        subject_type: &str,
    ) -> Result<Query<M>> {
        match self.ids(db, subject_type).await? {
            None => Ok(query),
            Some(ids) if ids.is_empty() => Ok(query.none()),
            Some(ids) => Ok(query.where_in(id_col, ids)),
        }
    }
}

/// Where `user` holds `permission`: everywhere when a global role (or the
/// app's `gate_before`) grants it, else the scopes whose assigned roles grant
/// it today.
///
/// # Errors
///
/// `Error::Forbidden` when the user holds the permission nowhere.
pub async fn reach(db: &Db, user: &AuthUser, permission: &str) -> Result<Reach> {
    if user.allows(permission) {
        return Ok(Reach::everything());
    }
    let granting: Vec<String> = renox::auth::permissions::roles(db)
        .await?
        .into_iter()
        .filter(|(_, grants)| grants.iter().any(|g| g == permission))
        .map(|(role, _)| role)
        .collect();
    let mut scopes: Vec<Scope> = Vec::new();
    for assignment in user.assignments(db).await? {
        if assignment.is_active()
            && !assignment.scope.is_global()
            && granting.contains(&assignment.role)
            && !scopes.contains(&assignment.scope)
        {
            scopes.push(assignment.scope);
        }
    }
    if scopes.is_empty() {
        return Err(Error::Forbidden);
    }
    Ok(Reach::only(scopes))
}

/// The routes added so far need `permission` globally or in at least one
/// scope (403 otherwise; guests go to log in). The handler narrows what it
/// shows with [`reach`]. `require_permission` is for pages that are the same
/// for everyone who may open them.
pub fn anywhere(routes: renox::Routes, permission: &str) -> renox::Routes {
    use renox::axum::extract::Request;
    use renox::axum::middleware::{Next, from_fn};
    use renox::axum::response::IntoResponse;

    let permission = permission.to_owned();
    routes
        .route_layer(from_fn(
            move |user: Option<AuthUser>, req: Request, next: Next| {
                let permission = permission.clone();
                async move {
                    match user {
                        Some(user)
                            if user.allows(&permission)
                                || user.has_permission_anywhere(&permission) =>
                        {
                            next.run(req).await
                        }
                        Some(_) => Error::Forbidden.into_response(),
                        None => Redirect::to("/login").into_response(),
                    }
                }
            },
        ))
        .require_auth()
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
    async fn a_scope_is_named_by_its_label_else_by_kind_and_id() {
        let app = TestApp::new(App::new()).await;
        register_label("scopes_test_buildings", |_db, id| {
            let id = id.to_owned();
            Box::pin(async move { Ok((id == "1").then(|| "Tower A".to_owned())) })
        });
        let db = app.db();
        let named = label_of(db, &Scope::new("scopes_test_buildings", 1))
            .await
            .unwrap();
        assert_eq!(named, "Tower A");
        let unknown = label_of(db, &Scope::new("scopes_test_buildings", 2))
            .await
            .unwrap();
        assert_eq!(unknown, "scopes_test_buildings 2");
        let plain = label_of(db, &Scope::new("scopes_test_other", 5))
            .await
            .unwrap();
        assert_eq!(plain, "scopes_test_other 5");
        assert_eq!(
            label_of(db, &Scope::global()).await.unwrap(),
            "All locations"
        );
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

    #[renox::test]
    async fn members_are_listed_by_the_registered_type_and_scopes() {
        let app = TestApp::new(App::new()).await;
        register_members("scopes_test.unit", |_db, scopes| {
            let ids: Vec<i64> = scopes
                .iter()
                .filter(|s| s.kind() == "scopes_test_towers")
                .filter_map(|s| s.id().parse::<i64>().ok())
                .flat_map(|tower| [tower * 10 + 1, tower * 10 + 2])
                .collect();
            Box::pin(async move { Ok(ids) })
        });
        let towers = [
            Scope::new("scopes_test_towers", 2),
            Scope::new("elsewhere", 9),
        ];
        let db = app.db();
        assert_eq!(
            members_in(db, "scopes_test.unit", &towers).await.unwrap(),
            [21, 22]
        );
        assert!(
            members_in(db, "scopes_test.unit", &[])
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            members_in(db, "scopes_test.nobody", &towers)
                .await
                .unwrap()
                .is_empty()
        );

        let reach = Reach::only(towers.to_vec());
        assert!(!reach.is_all());
        assert_eq!(
            reach.ids(db, "scopes_test.unit").await.unwrap(),
            Some(vec![21, 22])
        );
        assert!(reach.contains_entity(db, "unit", 22).await.unwrap());
        assert!(!reach.contains_entity(db, "unit", 31).await.unwrap());
        // A global holder reaches everything and is not narrowed.
        let all = Reach::everything();
        assert_eq!(all.ids(db, "scopes_test.unit").await.unwrap(), None);
        assert!(all.contains_entity(db, "unit", 31).await.unwrap());
    }

    use crate::entities::ScopeEntity;
}
