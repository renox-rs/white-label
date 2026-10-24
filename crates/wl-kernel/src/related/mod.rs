//! Related records of any record (wl-kernel, `PLT.REL.01`, BL-AGT-03,
//! decision 0006 "How search works" 3): the records that belong to one
//! record, across modules, grouped by relation. The *Related* panel of a
//! record page ([`web`]) and, later, the agents' `related` tool both call
//! [`related`], so they show the same groups.
//!
//! # Declaring relations
//!
//! A module declares each relation once in its `register()`, from one
//! subject type to another, with a loader that takes **many** records at
//! once and answers in one query. Never a foreign key between modules'
//! tables: a relation reads the module's own table, or rows attached by
//! `Subject` (`subject_type`, `subject_id`).
//!
//! ```ignore
//! use wl_kernel::related::Relation;
//!
//! // invoicing.invoice → its lines: rows of the lines table whose
//! // `invoice_id` is the invoice's id. `expand` shows the lines' own
//! // relations (their products) on the invoice too, at depth 2.
//! Relation::has_many("invoicing.invoice", "lines", "invoicing.invoice_line",
//!         "wl_invoicing_invoice_lines", "invoice_id")
//!     .label("Lines")
//!     .permission("invoices.view")
//!     .expand()
//!     .register();
//! // invoicing.invoice_line → its product: the column `item_id` of the
//! // line's own row.
//! Relation::belongs_to("invoicing.invoice_line", "products", "inventory.item",
//!         "wl_invoicing_invoice_lines", "item_id")
//!     .label("Products")
//!     .register();
//! // invoicing.invoice → its customer.
//! Relation::belongs_to("invoicing.invoice", "customer", "parties.party",
//!         "wl_invoicing_invoices", "party_id")
//!     .label("Customer")
//!     .register();
//! // Anything else: a loader of (from, to) pairs for many ids at once.
//! Relation::new("invoicing.invoice", "payments", "payments.payment")
//!     .label("Payments")
//!     .load(|db, ids| Box::pin(async move { allocated_payments(db, ids).await }))
//!     .register();
//! // Generic: rows attached to any record by `subject_type` / `subject_id`.
//! Relation::attached("documents", "wl_documents.document", "wl_documents_documents")
//!     .label("Documents")
//!     .permission("documents.view")
//!     .register();
//! ```
//!
//! # Who sees what
//!
//! Each target is filtered like a search result ([`Access`]): a target
//! whose type is searchable (`Searchable::of`) must have a search document
//! the user may see (the type's permission, within the user's scopes); its
//! title, line, status and void mark come from that document. A target of a
//! type that is not searchable needs the relation's `permission` and shows
//! as its entity and number ("Invoice line #4"): register the type for
//! search to give it a title. A group the user may not see at all is absent
//! (never "Payments 0"); a hidden record is neither listed nor counted. A
//! searchable target without a document (deleted) is skipped and logged
//! once.
//!
//! # Depth
//!
//! `related(.., 1)` gives the relations of the record's type and the
//! generic ones; depth 2 also follows the relations of the targets of
//! relations marked [`RelationBuilder::expand`] (invoice → lines →
//! products), all targets of a relation at once. The number of queries
//! depends on the relations, never on the number of rows: one per relation
//! used, one read of the search documents of every target.
//!
//! A relation declared from each side (A → B by one module, B → A by the
//! other) is shown from each side once: from A, its relations to B; the way
//! back to A is never followed at depth 2. Two declarations of the same name
//! and target from one type are merged.
//!
//! # Generic relations
//!
//! The kernel declares the approval requests of any record ([`APPROVALS`]);
//! the documents module declares documents with [`Relation::attached`];
//! notes join the same way once the notes module exists (wl-kernel §5.9).

pub mod web;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use renox::db::{DbValue, sql};
use renox::prelude::*;
use serde::Serialize;

use crate::machines::BoxFuture;
use crate::search::{Access, SearchDocumentRow, searchable_type};
use crate::subject::Subject;

/// Up to this many rows of a group are listed; the count is of all.
pub const ROWS_PER_GROUP: usize = 10;

/// The depth the panel shows: the record's relations, and one step further
/// along relations marked `expand`.
pub const PANEL_DEPTH: u8 = 2;

/// The type of an approval request, as the generic relation names it.
pub const APPROVAL_SUBJECT_TYPE: &str = "kernel.approval_request";

/// The name of the generic relation to approval requests.
pub const APPROVALS: &str = "approvals";

/// Any subject type: a generic relation's `from_type`.
pub const ANY_TYPE: &str = "*";

/// The future a loader returns: `(from, to)` pairs.
pub type LoadFuture<'a> = BoxFuture<'a, Result<Vec<(Subject, Subject)>>>;

type LoadFn = Arc<dyn for<'a> Fn(&'a Db, &'a str, &'a [i64]) -> LoadFuture<'a> + Send + Sync>;

/// A named link from one subject type to another, as registered.
#[derive(Clone)]
pub struct Relation {
    from_type: String,
    name: String,
    label: String,
    to_type: String,
    permission: Option<String>,
    link: Option<String>,
    expand: bool,
    load: LoadFn,
}

impl std::fmt::Debug for Relation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Relation")
            .field("from_type", &self.from_type)
            .field("name", &self.name)
            .field("label", &self.label)
            .field("to_type", &self.to_type)
            .field("permission", &self.permission)
            .field("expand", &self.expand)
            .finish()
    }
}

impl Relation {
    /// A relation `name` from `from_type` to `to_type`; give its loader with
    /// [`RelationBuilder::load`].
    pub fn new(from_type: &str, name: &str, to_type: &str) -> RelationBuilder {
        RelationBuilder {
            from_type: from_type.to_owned(),
            name: name.to_owned(),
            label: None,
            to_type: to_type.to_owned(),
            permission: None,
            link: None,
            expand: false,
            load: None,
        }
    }

    /// The rows of `table` whose column `foreign_key` holds the id of a
    /// record of `from_type` (an invoice's lines, a lease's cheques). The
    /// targets are those rows.
    pub fn has_many(
        from_type: &str,
        name: &str,
        to_type: &str,
        table: &str,
        foreign_key: &str,
    ) -> RelationBuilder {
        let statement = format!(
            "SELECT {fk} AS from_id, id AS to_id FROM {table} WHERE {fk} IN ",
            fk = identifier(foreign_key),
            table = identifier(table),
        );
        Self::by_statement(from_type, name, to_type, statement)
    }

    /// The record of `to_type` named by column `foreign_key` of the rows of
    /// `table`, the table of `from_type` (an invoice's customer, a line's
    /// product).
    pub fn belongs_to(
        from_type: &str,
        name: &str,
        to_type: &str,
        table: &str,
        foreign_key: &str,
    ) -> RelationBuilder {
        let statement = format!(
            "SELECT id AS from_id, {fk} AS to_id FROM {table} WHERE {fk} IS NOT NULL AND id IN ",
            fk = identifier(foreign_key),
            table = identifier(table),
        );
        Self::by_statement(from_type, name, to_type, statement)
    }

    /// The rows of `table` attached to any record by its `subject_type` and
    /// `subject_id` columns (documents, notes, approval requests): a generic
    /// relation every record has.
    pub fn attached(name: &str, to_type: &str, table: &str) -> RelationBuilder {
        let statement = format!(
            "SELECT subject_id AS from_id, id AS to_id FROM {} \
             WHERE subject_type = ? AND subject_id IN ",
            identifier(table)
        );
        let to = to_type.to_owned();
        let load: LoadFn = Arc::new(move |db, from_type, ids| {
            let statement = statement.clone();
            let to = to.clone();
            Box::pin(async move {
                let mut values = vec![DbValue::Text(from_type.to_owned())];
                values.extend(ids.iter().map(|id| DbValue::Integer(*id)));
                pairs(db, &statement, values, ids.len(), from_type, &to).await
            })
        });
        let mut builder = Self::new(ANY_TYPE, name, to_type);
        builder.load = Some(load);
        builder
    }

    fn by_statement(
        from_type: &str,
        name: &str,
        to_type: &str,
        statement: String,
    ) -> RelationBuilder {
        let to = to_type.to_owned();
        let load: LoadFn = Arc::new(move |db, from_type, ids| {
            let statement = statement.clone();
            let to = to.clone();
            Box::pin(async move {
                let values = ids.iter().map(|id| DbValue::Integer(*id)).collect();
                pairs(db, &statement, values, ids.len(), from_type, &to).await
            })
        });
        let mut builder = Self::new(from_type, name, to_type);
        builder.load = Some(load);
        builder
    }

    /// The type it goes from (`*` for a generic relation).
    pub fn from_type(&self) -> &str {
        &self.from_type
    }

    /// Its name, e.g. `payments`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its group's heading, e.g. `Payments`.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The type it goes to.
    pub fn to_type(&self) -> &str {
        &self.to_type
    }

    /// The permission it needs, if any (required for a target type that is
    /// not searchable).
    pub fn permission(&self) -> Option<&str> {
        self.permission.as_deref()
    }

    /// Whether the targets' own relations are shown at depth 2.
    pub fn is_expand(&self) -> bool {
        self.expand
    }

    /// The `(from, to)` pairs of the records `ids` of `from_type`, in one
    /// query.
    pub async fn load(
        &self,
        db: &Db,
        from_type: &str,
        ids: &[i64],
    ) -> Result<Vec<(Subject, Subject)>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        (self.load)(db, from_type, ids).await
    }

    /// The address of a target that is not searchable: the relation's link
    /// with `{id}` filled in, else the target's own related records page.
    fn url_of(&self, target: &Subject) -> String {
        match &self.link {
            Some(link) => link.replace("{id}", &target.subject_id.to_string()),
            None => web::url_of(target),
        }
    }
}

/// Runs `statement` (ending in `IN `) with one placeholder per id (`ids`
/// of them, the last values of `values`), reading `from_id` and `to_id` as
/// records of `from_type` and `to_type`.
async fn pairs(
    db: &Db,
    statement: &str,
    values: Vec<DbValue>,
    ids: usize,
    from_type: &str,
    to_type: &str,
) -> Result<Vec<(Subject, Subject)>> {
    let placeholders = vec!["?"; ids].join(", ");
    let rows = sql(format!("{statement}({placeholders})"))
        .bind_all(values)
        .fetch_all(db)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let from: Option<i64> = row.try_get("from_id")?;
        let to: Option<i64> = row.try_get("to_id")?;
        if let (Some(from), Some(to)) = (from, to) {
            out.push((Subject::new(from_type, from), Subject::new(to_type, to)));
        }
    }
    Ok(out)
}

/// `name` quoted as a SQL identifier.
///
/// # Panics
///
/// When it holds anything but lower-case letters, digits and `_`: table
/// and column names are constants in code, so a bad one is a programming
/// error.
fn identifier(name: &str) -> String {
    assert!(
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
        "`{name}` is not a table or column name"
    );
    format!("\"{name}\"")
}

/// A relation being declared, from [`Relation::new`] and its siblings.
#[must_use = "a relation is only shown once registered"]
pub struct RelationBuilder {
    from_type: String,
    name: String,
    label: Option<String>,
    to_type: String,
    permission: Option<String>,
    link: Option<String>,
    expand: bool,
    load: Option<LoadFn>,
}

impl RelationBuilder {
    /// The group's heading (`Payments`); the name in words by default.
    pub fn label(mut self, label: &str) -> Self {
        self.label = Some(label.to_owned());
        self
    }

    /// A permission the user needs to see this group, on top of the
    /// target's own. Required when the target type is not searchable: it is
    /// then the only check.
    pub fn permission(mut self, permission: &str) -> Self {
        self.permission = Some(permission.to_owned());
        self
    }

    /// Where a target opens when its type is not searchable, with `{id}`
    /// where its id goes (`/documents/{id}`); a searchable target opens at
    /// its type's link.
    pub fn link(mut self, link: &str) -> Self {
        self.link = Some(link.to_owned());
        self
    }

    /// Shows the targets' own relations too at depth 2 (invoice → lines →
    /// products).
    pub fn expand(mut self) -> Self {
        self.expand = true;
        self
    }

    /// The loader: the `(from, to)` pairs of many records of the from type
    /// at once, in one query.
    pub fn load(
        mut self,
        load: impl for<'a> Fn(&'a Db, &'a [i64]) -> LoadFuture<'a> + Send + Sync + 'static,
    ) -> Self {
        self.load = Some(Arc::new(move |db, _from_type, ids| load(db, ids)));
        self
    }

    /// The finished relation.
    ///
    /// # Panics
    ///
    /// Without a loader, with a type that is not `<module>.<entity>` (`*`
    /// allowed as the from type), or with a target that is not searchable
    /// and no permission: a boot error, like the other registries. (Whether
    /// the target is searchable is checked when the relation is read, as
    /// the modules register in any order.)
    pub fn build(self) -> Relation {
        let typed = |t: &str| {
            let mut parts = t.split('.');
            let ok = |p: Option<&str>| {
                p.is_some_and(|p| {
                    !p.is_empty()
                        && p.chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                })
            };
            ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
        };
        assert!(
            self.from_type == ANY_TYPE || typed(&self.from_type),
            "relation `{}` goes from `{}`, not <module>.<entity>",
            self.name,
            self.from_type
        );
        assert!(
            typed(&self.to_type),
            "relation `{}` goes to `{}`, not <module>.<entity>",
            self.name,
            self.to_type
        );
        let load = self
            .load
            .unwrap_or_else(|| panic!("relation `{}` needs a loader", self.name));
        let label = self.label.unwrap_or_else(|| {
            let words = self.name.replace('_', " ");
            let mut chars = words.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        });
        Relation {
            from_type: self.from_type,
            name: self.name,
            label,
            to_type: self.to_type,
            permission: self.permission,
            link: self.link,
            expand: self.expand,
            load,
        }
    }

    /// Adds the relation to the process-wide registry, replacing one of the
    /// same from type and name.
    pub fn register(self) {
        register(self.build());
    }
}

static REGISTRY: RwLock<Vec<Arc<Relation>>> = RwLock::new(Vec::new());

/// Adds `relation` to the registry, replacing one of the same from type and
/// name in its place (so the groups keep the order of first registration).
pub fn register(relation: Relation) {
    let mut registry = REGISTRY.write().unwrap_or_else(PoisonError::into_inner);
    match registry
        .iter_mut()
        .find(|r| r.from_type == relation.from_type && r.name == relation.name)
    {
        Some(existing) => *existing = Arc::new(relation),
        None => registry.push(Arc::new(relation)),
    }
}

/// Every registered relation, in the order registered.
pub fn registered() -> Vec<Relation> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|r| (**r).clone())
        .collect()
}

/// The relations from `subject_type`: its own, then (with `generic`) the
/// generic ones.
fn from(subject_type: &str, generic: bool) -> Vec<Arc<Relation>> {
    let registry = REGISTRY.read().unwrap_or_else(PoisonError::into_inner);
    let own = registry.iter().filter(|r| r.from_type == subject_type);
    let any = registry
        .iter()
        .filter(|r| generic && r.from_type == ANY_TYPE);
    own.chain(any).cloned().collect()
}

/// The permissions the relations ask for, for [`Access`].
pub(crate) fn permissions() -> BTreeSet<String> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter_map(|r| r.permission.clone())
        .collect()
}

/// One related record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelatedRecord {
    /// The record.
    pub subject: Subject,
    /// What it is called.
    pub title: String,
    /// The line under the title.
    pub secondary: String,
    /// Its status, if known.
    pub status: Option<String>,
    /// Whether it was voided (listed with a *Void* badge).
    pub voided: bool,
    /// The address of its page.
    pub url: String,
}

/// The records of one relation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelatedGroup {
    /// The relation's name, e.g. `payments`.
    pub name: String,
    /// The heading, e.g. `Payments`.
    pub label: String,
    /// The target type.
    pub to_type: String,
    /// How many records the user may see.
    pub count: u64,
    /// Up to [`ROWS_PER_GROUP`] of them.
    pub records: Vec<RelatedRecord>,
}

/// A relation picked for the answer, with its targets.
struct Picked {
    relation: Arc<Relation>,
    targets: Vec<Subject>,
}

/// The related records of `subject` that `access` may see, grouped by
/// relation in the order the relations were registered (generic ones after
/// the type's own, depth-2 ones last). `depth` 2 follows the relations
/// marked `expand` one step further. The caller has checked that the user
/// may open `subject` itself ([`can_open`]).
pub async fn related(
    db: &Db,
    access: &Access,
    subject: &Subject,
    depth: u8,
) -> Result<Vec<RelatedGroup>> {
    let mut picked: Vec<Picked> = Vec::new();
    for relation in from(&subject.subject_type, true) {
        if !group_visible(access, &relation) {
            continue;
        }
        let targets =
            targets_of(db, &relation, &subject.subject_type, &[subject.subject_id]).await?;
        picked.push(Picked { relation, targets });
    }
    if depth >= 2 {
        let mut deeper: Vec<Picked> = Vec::new();
        for level in picked.iter().filter(|p| p.relation.expand) {
            let ids: Vec<i64> = level.targets.iter().map(|t| t.subject_id).collect();
            for relation in from(&level.relation.to_type, false) {
                // Never back to the record, never a group shown already.
                if relation.to_type == subject.subject_type
                    || picked
                        .iter()
                        .chain(deeper.iter())
                        .any(|p| p.relation.name == relation.name)
                    || !group_visible(access, &relation)
                {
                    continue;
                }
                let targets = targets_of(db, &relation, &level.relation.to_type, &ids).await?;
                deeper.push(Picked { relation, targets });
            }
        }
        picked.extend(deeper);
    }

    // One read of the documents of every searchable target.
    let mut wanted: BTreeMap<String, BTreeSet<i64>> = BTreeMap::new();
    for target in picked.iter().flat_map(|p| p.targets.iter()) {
        if searchable_type(&target.subject_type).is_some() {
            wanted
                .entry(target.subject_type.clone())
                .or_default()
                .insert(target.subject_id);
        }
    }
    let documents = documents_of(db, &wanted).await?;

    let mut groups: Vec<RelatedGroup> = Vec::new();
    for level in picked {
        let records: Vec<RelatedRecord> = level
            .targets
            .iter()
            .filter_map(|target| visible_record(access, &level.relation, target, &documents))
            .collect();
        match groups
            .iter_mut()
            .find(|g| g.name == level.relation.name && g.to_type == level.relation.to_type)
        {
            Some(group) => {
                for record in records {
                    if !group.records.iter().any(|r| r.subject == record.subject) {
                        group.records.push(record);
                    }
                }
            }
            None => groups.push(RelatedGroup {
                name: level.relation.name.clone(),
                label: level.relation.label.clone(),
                to_type: level.relation.to_type.clone(),
                count: 0,
                records,
            }),
        }
    }
    for group in &mut groups {
        group.count = u64::try_from(group.records.len()).unwrap_or(u64::MAX);
        group.records.truncate(ROWS_PER_GROUP);
    }
    Ok(groups)
}

/// The stored documents of the `wanted` records, in one query.
async fn documents_of(
    db: &Db,
    wanted: &BTreeMap<String, BTreeSet<i64>>,
) -> Result<BTreeMap<(String, i64), SearchDocumentRow>> {
    if wanted.is_empty() {
        return Ok(BTreeMap::new());
    }
    Ok(SearchDocumentRow::query()
        .where_any(|mut any| {
            for (subject_type, ids) in wanted {
                let ids: Vec<i64> = ids.iter().copied().collect();
                any = any.where_all(|all| {
                    all.where_eq("subject_type", subject_type.as_str())
                        .where_in("subject_id", ids)
                });
            }
            any
        })
        .get(db)
        .await?
        .into_iter()
        .map(|row| ((row.subject_type.clone(), row.subject_id), row))
        .collect())
}

/// Whether the user may open `subject` (its page, its related records): a
/// searchable record when its document is one the user may see; any other
/// record when the user holds, everywhere, the permission of a relation
/// that leads to or from its type (its scope is unknown to the kernel, so a
/// scoped holder does not reach it directly: register the type for search
/// to give it a scope). The title to show it by comes with
/// it.
pub async fn can_open(db: &Db, access: &Access, subject: &Subject) -> Result<Option<String>> {
    if let Some(searchable) = searchable_type(&subject.subject_type) {
        let document = crate::search::document_of(db, subject).await?;
        return Ok(document
            .filter(|d| access.sees(searchable.permission(), &d.scope_type, &d.scope_id))
            .map(|d| d.title));
    }
    let reachable = REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter(|r| r.to_type == subject.subject_type || r.from_type == subject.subject_type)
        .filter_map(|r| r.permission.as_deref())
        .any(|permission| access.holds_everywhere(permission));
    Ok(reachable.then(|| subject.words()))
}

/// The distinct targets of `relation` from `ids` of `from_type`, in the
/// order loaded.
async fn targets_of(
    db: &Db,
    relation: &Relation,
    from_type: &str,
    ids: &[i64],
) -> Result<Vec<Subject>> {
    let mut seen = BTreeSet::new();
    Ok(relation
        .load(db, from_type, ids)
        .await?
        .into_iter()
        .map(|(_, to)| to)
        .filter(|to| seen.insert(to.clone()))
        .collect())
}

/// Whether the user may see `relation`'s group at all: the target type's
/// permission (searchable) or the relation's own (any other type), and the
/// relation's permission when it has one.
fn group_visible(access: &Access, relation: &Relation) -> bool {
    let own = relation.permission.as_deref().map(|p| access.holds(p));
    match searchable_type(&relation.to_type) {
        Some(searchable) => access.holds(searchable.permission()) && own.unwrap_or(true),
        None => own.unwrap_or(false),
    }
}

/// `target` as the user sees it, or `None` when hidden or gone.
fn visible_record(
    access: &Access,
    relation: &Relation,
    target: &Subject,
    documents: &BTreeMap<(String, i64), SearchDocumentRow>,
) -> Option<RelatedRecord> {
    let Some(searchable) = searchable_type(&target.subject_type) else {
        return Some(RelatedRecord {
            subject: target.clone(),
            title: target.words(),
            secondary: String::new(),
            status: None,
            voided: false,
            url: relation.url_of(target),
        });
    };
    let Some(document) = documents.get(&(target.subject_type.clone(), target.subject_id)) else {
        log_missing_once(relation, target);
        return None;
    };
    if !access.sees(
        searchable.permission(),
        &document.scope_type,
        &document.scope_id,
    ) {
        return None;
    }
    Some(RelatedRecord {
        subject: target.clone(),
        title: document.title.clone(),
        secondary: document.secondary.clone(),
        status: document.status.clone(),
        voided: document.voided,
        url: searchable.url_of(target.subject_id),
    })
}

/// Notes a target that no longer exists (or was never indexed), once per
/// process and target.
fn log_missing_once(relation: &Relation, target: &Subject) {
    static SEEN: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
    let key = format!("{}>{target}", relation.name);
    if SEEN
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key)
    {
        eprintln!(
            "related: `{target}` of relation `{}` has no search document; skipped",
            relation.name
        );
    }
}

/// Registers the panel's views and the kernel's own generic relation: the
/// approval requests of any record.
pub(crate) fn register(app: &mut Registry) {
    app.view_data::<web::RelatedPage>()
        .view_data::<web::RelatedPanel>();
    Relation::attached(APPROVALS, APPROVAL_SUBJECT_TYPE, "wl_approval_requests")
        .label(&crate::texts::en("wl_kernel.related.approvals"))
        .permission(crate::permissions::APPROVALS_VIEW)
        .register();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relation_needs_typed_ends_and_a_loader() {
        let relation = Relation::has_many(
            "test_rel.invoice",
            "invoice_lines",
            "test_rel.line",
            "lines",
            "invoice_id",
        )
        .build();
        assert_eq!(relation.label(), "Invoice lines");
        assert_eq!(relation.from_type(), "test_rel.invoice");
        assert!(!relation.is_expand());
        let generic = Relation::attached("notes", "test_rel.note", "notes").build();
        assert_eq!(generic.from_type(), ANY_TYPE);
        let missing = std::panic::catch_unwind(|| {
            Relation::new("test_rel.invoice", "x", "test_rel.y").build()
        });
        assert!(missing.is_err());
        let bad = std::panic::catch_unwind(|| {
            Relation::has_many("invoice", "x", "test_rel.y", "t", "c").build()
        });
        assert!(bad.is_err());
    }

    #[test]
    #[should_panic(expected = "is not a table or column name")]
    fn a_table_name_is_checked() {
        let _ = Relation::has_many("a.b", "x", "c.d", "t; drop", "c");
    }
}
