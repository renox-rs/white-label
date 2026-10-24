//! The registry of searchable entities, keyed by subject type like
//! [`crate::machines`] and [`crate::voids`].

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use renox::Result;
use renox::db::{Model, Transaction};

use super::document::SearchDocument;
use crate::machines::BoxFuture;

type BuildFn = Arc<
    dyn for<'a> Fn(&'a mut Transaction, i64) -> BoxFuture<'a, Result<Option<SearchDocument>>>
        + Send
        + Sync,
>;
type IdsFn = Arc<
    dyn for<'a> Fn(&'a mut Transaction, i64, u64) -> BoxFuture<'a, Result<Vec<i64>>> + Send + Sync,
>;
type DocumentFn<M> = Arc<
    dyn for<'a> Fn(&'a mut Transaction, &'a M) -> BoxFuture<'a, Result<SearchDocument>>
        + Send
        + Sync,
>;

/// A searchable entity, as registered.
#[derive(Clone)]
pub struct Searchable {
    subject_type: String,
    table: &'static str,
    permission: String,
    portal_visible: bool,
    build: BuildFn,
    ids: IdsFn,
}

impl std::fmt::Debug for Searchable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Searchable")
            .field("subject_type", &self.subject_type)
            .field("table", &self.table)
            .field("permission", &self.permission)
            .field("portal_visible", &self.portal_visible)
            .finish()
    }
}

impl Searchable {
    /// Starts the entry for model `M` under `subject_type`
    /// (`<module>.<entity>`); end with [`SearchableBuilder::register`].
    pub fn of<M>(subject_type: &str) -> SearchableBuilder<M>
    where
        M: Model<Key = i64> + Send + Sync + 'static,
    {
        SearchableBuilder {
            subject_type: subject_type.to_owned(),
            permission: String::new(),
            portal_visible: false,
            document: None,
        }
    }

    /// The subject type, e.g. `invoicing.invoice`.
    pub fn subject_type(&self) -> &str {
        &self.subject_type
    }

    /// The model's table.
    pub fn table(&self) -> &'static str {
        self.table
    }

    /// The permission a user needs to see this type.
    pub fn permission(&self) -> &str {
        &self.permission
    }

    /// Whether external users may find this type in the portal.
    pub fn is_portal_visible(&self) -> bool {
        self.portal_visible
    }

    /// The document of record `id`; `None` when the record does not exist.
    pub(super) async fn document_in(
        &self,
        tx: &mut Transaction,
        id: i64,
    ) -> Result<Option<SearchDocument>> {
        (self.build)(tx, id).await
    }

    /// Up to `limit` ids above `after`, in order.
    pub(super) async fn ids_in(
        &self,
        tx: &mut Transaction,
        after: i64,
        limit: u64,
    ) -> Result<Vec<i64>> {
        (self.ids)(tx, after, limit).await
    }
}

/// The entry being declared, from [`Searchable::of`].
#[must_use = "a searchable type is only indexed once registered"]
pub struct SearchableBuilder<M> {
    subject_type: String,
    permission: String,
    portal_visible: bool,
    document: Option<DocumentFn<M>>,
}

impl<M> SearchableBuilder<M>
where
    M: Model<Key = i64> + Send + Sync + 'static,
{
    /// The permission a user needs to see this type (required).
    pub fn permission(mut self, permission: &str) -> Self {
        self.permission = permission.to_owned();
        self
    }

    /// External users may find this type in the portal (their own records
    /// only, BL-AGT-05).
    pub fn portal_visible(mut self) -> Self {
        self.portal_visible = true;
        self
    }

    /// How a record reads as a search document, from the record alone.
    pub fn document(
        mut self,
        document: impl Fn(&M) -> SearchDocument + Send + Sync + 'static,
    ) -> Self {
        let document = Arc::new(document);
        self.document = Some(Arc::new(move |_tx, record| {
            let document = Arc::clone(&document);
            Box::pin(async move { Ok(document(record)) })
        }));
        self
    }

    /// How a record reads as a search document, when it needs other rows
    /// (the customer's name, the building): read them in `tx`.
    pub fn document_in(
        mut self,
        document: impl for<'a> Fn(&'a mut Transaction, &'a M) -> BoxFuture<'a, Result<SearchDocument>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.document = Some(Arc::new(document));
        self
    }

    /// The finished entry.
    ///
    /// # Panics
    ///
    /// Without a permission or a document, or with a subject type that is
    /// not `<module>.<entity>`: a boot error, like the other registries.
    pub fn build(self) -> Searchable {
        assert!(
            self.subject_type.contains('.') && !self.subject_type.starts_with('.'),
            "searchable type `{}` is not <module>.<entity>",
            self.subject_type
        );
        assert!(
            !self.permission.trim().is_empty(),
            "searchable type `{}` needs a permission",
            self.subject_type
        );
        let document = self
            .document
            .unwrap_or_else(|| panic!("searchable type `{}` needs a document", self.subject_type));
        let build: BuildFn = Arc::new(move |tx, id| {
            let document = Arc::clone(&document);
            Box::pin(async move {
                let record = M::unscoped().where_eq("id", id).first(&mut *tx).await?;
                match record {
                    Some(record) => document(tx, &record).await.map(Some),
                    None => Ok(None),
                }
            })
        });
        let ids: IdsFn = Arc::new(|tx, after, limit| {
            Box::pin(async move {
                let rows = M::unscoped()
                    .where_op("id", ">", after)
                    .order_by("id")
                    .limit(limit)
                    .get(&mut *tx)
                    .await?;
                Ok(rows.iter().map(Model::id).collect())
            })
        });
        Searchable {
            subject_type: self.subject_type,
            table: M::TABLE,
            permission: self.permission,
            portal_visible: self.portal_visible,
            build,
            ids,
        }
    }

    /// Adds the entry to the process-wide registry, replacing one of the
    /// same subject type.
    pub fn register(self) {
        register(self.build());
    }
}

static REGISTRY: RwLock<BTreeMap<String, Arc<Searchable>>> = RwLock::new(BTreeMap::new());

/// Adds `searchable` to the process-wide registry.
pub(super) fn register(searchable: Searchable) {
    REGISTRY
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(searchable.subject_type.clone(), Arc::new(searchable));
}

/// The entry of `subject_type`, if it is searchable.
pub(super) fn get(subject_type: &str) -> Option<Arc<Searchable>> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(subject_type)
        .cloned()
}

/// The entries whose model's table is `table` (usually one).
pub(super) fn by_table(table: &str) -> Vec<Arc<Searchable>> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .values()
        .filter(|searchable| searchable.table == table)
        .cloned()
        .collect()
}

/// The registered searchable types, sorted by subject type.
pub fn registered_types() -> Vec<Searchable> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .values()
        .map(|searchable| (**searchable).clone())
        .collect()
}
