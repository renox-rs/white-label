//! Document kinds and their `blocks` (wl-documents §3 *Document kind*,
//! §5.6, §7).
//!
//! Modules and verticals declare their kinds in code as [`KindSeed`]s with
//! [`register_kind`] (a process-wide registry by key, like the notification
//! types). The seeder and `documents:install` store them with [`install`]:
//! a kind the database lacks is created with its entries; a kind already
//! there keeps the client's changes, except that a **legal block** is put
//! back on every run (legal block, override `never`, compliance-critical,
//! review on, and its seeded entries locked with their seeded mode and
//! condition).
//!
//! A client changes a kind's entries with [`add_block`], [`remove_block`]
//! and [`set_block_mode`]. Every entry of a legal-block kind is locked: it
//! cannot be removed or switched to another mode. An entry can only be
//! added for a transition whose module registered a `BlockSubjects`
//! provider (`wl_kernel::documents::register_block_subjects`).
//! [`blocks_for`] lists the entries that guard a transition.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};

use renox::db::Json;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::{self, BlockCondition, BlockEntry, BlockMode, OverrideMode};

use crate::texts;

/// The file types a kind allows unless it narrows them (extensions).
pub const DEFAULT_FILE_TYPES: &[&str] = &["pdf", "jpg", "jpeg", "png", "heic", "docx", "xlsx"];

/// The largest file a kind allows unless it narrows it: 20 MB.
pub const DEFAULT_MAX_SIZE_BYTES: i64 = 20 * 1024 * 1024;

/// Who may see documents of a kind (§3; `PLT.ACL.06`).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Sensitivity {
    /// Anyone with `documents.view`.
    #[default]
    Normal,
    /// Identity documents, under PDPL.
    Personal,
    /// Health records (clinics).
    Health,
    /// Bank statements, salary.
    Financial,
}

/// A document kind as stored for this client.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_documents_kinds", unique(key))]
pub struct DocumentKind {
    /// The primary key.
    pub id: i64,
    /// The key, e.g. `residence_visa`.
    pub key: String,
    /// The English name, e.g. "Residence visa".
    pub name_en: String,
    /// The Arabic name.
    pub name_ar: String,
    /// The subject types it attaches to, e.g. `hr.employee`.
    pub subject_types: Json<Vec<String>>,
    /// Whether a document of the kind has an expiry date.
    pub has_expiry: bool,
    /// Whether a document of the kind has a number (e.g. passport no.).
    pub has_number: bool,
    /// The days before expiry to remind, e.g. `[60, 30, 7]`.
    pub reminder_offsets: Json<Vec<i64>>,
    /// Who may see its documents.
    pub sensitivity: Sensitivity,
    /// The retention rule, e.g. `employment_end+5y`; `None` keeps them.
    pub retention_rule: Option<String>,
    /// The allowed file extensions, lower case.
    pub allowed_file_types: Json<Vec<String>>,
    /// The largest allowed file, in bytes.
    pub max_size_bytes: i64,
    /// Whether a portal user may upload one.
    pub portal_upload: bool,
    /// Whether a document waits for review before it counts (always on
    /// for kinds with `blocks`, `DOC.ACL.03`).
    pub needs_review: bool,
    /// Whether a refused transition may be overridden.
    pub override_mode: OverrideMode,
    /// A kind the law requires for the act: locked by the platform.
    pub legal_block: bool,
    /// How many days ahead the nightly run looks for planned work.
    pub look_ahead_days: i64,
    /// Whether the owner is also told on expiry.
    pub compliance_critical: bool,
    /// The role whose holders renew it, e.g. `pro`.
    pub responsible_role: Option<String>,
    /// The roles told on expiry as well as the responsible role, e.g. `hr`.
    pub also_notify: Json<Vec<String>>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl DocumentKind {
    /// The stored kind `key`, if any.
    pub async fn by_key(db: &Db, key: &str) -> Result<Option<DocumentKind>> {
        DocumentKind::query()
            .where_(DocumentKind::KEY.eq(key))
            .first(db)
            .await
    }

    /// Every kind, by key.
    pub async fn ordered(db: &Db) -> Result<Vec<DocumentKind>> {
        DocumentKind::query()
            .order_by(DocumentKind::KEY)
            .get(db)
            .await
    }

    /// The kind's entries, by guarded transition.
    pub async fn blocks(&self, db: &Db) -> Result<Vec<KindBlock>> {
        KindBlock::query()
            .where_(KindBlock::KIND_ID.eq(self.id))
            .order_by(KindBlock::GUARDED)
            .get(db)
            .await
    }

    /// Whether the kind is blocking: it has at least one `blocks` entry, in
    /// either mode (`DOC.ACL.03`). A document of a blocking kind needs
    /// review and cannot be accepted by its own uploader.
    pub async fn is_blocking(&self, db: &Db) -> Result<bool> {
        Ok(KindBlock::query()
            .where_(KindBlock::KIND_ID.eq(self.id))
            .first(db)
            .await?
            .is_some())
    }

    /// The name in `locale`: Arabic for `ar` when there is one, else English.
    pub fn name(&self, locale: &str) -> &str {
        if locale == "ar" && !self.name_ar.is_empty() {
            &self.name_ar
        } else {
            &self.name_en
        }
    }

    /// Whether a document of the kind may be attached to `subject_type`.
    pub fn allows_subject_type(&self, subject_type: &str) -> bool {
        self.subject_types.0.iter().any(|t| t == subject_type)
    }

    /// Whether a file with extension `extension` is allowed (any case).
    pub fn allows_file_type(&self, extension: &str) -> bool {
        let extension = extension.trim_start_matches('.').to_ascii_lowercase();
        self.allowed_file_types.0.contains(&extension)
    }

    /// The row [`install`] creates for `seed`.
    pub fn from_seed(seed: &KindSeed) -> DocumentKind {
        let mut kind = DocumentKind {
            key: seed.key.clone(),
            name_en: seed.name_en.clone(),
            name_ar: seed.name_ar.clone(),
            subject_types: Json(seed.subject_types.clone()),
            has_expiry: seed.has_expiry,
            has_number: seed.has_number,
            reminder_offsets: Json(seed.reminder_offsets.clone()),
            sensitivity: seed.sensitivity,
            retention_rule: seed.retention_rule.clone(),
            allowed_file_types: Json(seed.allowed_file_types.clone()),
            max_size_bytes: seed.max_size_bytes,
            portal_upload: seed.portal_upload,
            needs_review: seed.needs_review || !seed.blocks.is_empty(),
            override_mode: seed.override_mode,
            legal_block: false,
            look_ahead_days: seed.look_ahead_days,
            compliance_critical: seed.compliance_critical,
            responsible_role: seed.responsible_role.clone(),
            also_notify: Json(seed.also_notify.clone()),
            ..DocumentKind::default()
        };
        if seed.legal_block {
            kind.lock_as_legal_block();
        }
        kind
    }

    /// Puts back what a legal block implies: override `never`,
    /// compliance-critical, review on. Returns whether anything changed.
    fn lock_as_legal_block(&mut self) -> bool {
        let changed = !self.legal_block
            || self.override_mode != OverrideMode::Never
            || !self.compliance_critical
            || !self.needs_review;
        self.legal_block = true;
        self.override_mode = OverrideMode::Never;
        self.compliance_critical = true;
        self.needs_review = true;
        changed
    }
}

/// One entry of a kind's `blocks`.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_documents_kind_blocks",
    unique(kind_id, guarded),
    index(guarded)
)]
pub struct KindBlock {
    /// The primary key.
    pub id: i64,
    /// The kind.
    #[model(references = "wl_documents_kinds")]
    pub kind_id: i64,
    /// The guarded transition, `<entity>.<transition>`.
    pub guarded: String,
    /// Block or flag.
    pub mode: BlockMode,
    /// The field of the guarded record the condition reads; `None` applies
    /// to every record.
    pub condition_field: Option<String>,
    /// The values that make the entry apply.
    pub condition_values: Json<Vec<String>>,
    /// A legal-block entry: it cannot be removed or changed.
    pub locked: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl KindBlock {
    /// The condition on the guarded record, if any.
    pub fn condition(&self) -> Option<BlockCondition> {
        self.condition_field.as_ref().map(|field| BlockCondition {
            field: field.clone(),
            values: self.condition_values.0.clone(),
        })
    }

    /// Stores `condition` in the row.
    fn set_condition(&mut self, condition: Option<&BlockCondition>) {
        self.condition_field = condition.map(|c| c.field.clone());
        self.condition_values = Json(condition.map(|c| c.values.clone()).unwrap_or_default());
    }

    /// The entry in the kernel's shape, with `kind`'s key and override mode.
    pub fn to_entry(&self, kind: &DocumentKind) -> BlockEntry {
        BlockEntry {
            kind: kind.key.clone(),
            guarded: self.guarded.clone(),
            mode: self.mode,
            override_mode: kind.override_mode,
            condition: self.condition(),
            locked: self.locked,
        }
    }
}

/// One seeded entry of a [`KindSeed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockSeed {
    /// The guarded transition, `<entity>.<transition>`.
    pub guarded: String,
    /// Block or flag.
    pub mode: BlockMode,
    /// The condition on the guarded record; `None` for every record.
    pub condition: Option<BlockCondition>,
}

/// A document kind as a module or vertical declares it in code; see
/// [`register_kind`] and [`install`].
///
/// ```
/// use wl_documents::kinds::KindSeed;
/// use wl_kernel::documents::BlockMode;
///
/// let visa = KindSeed::new("residence_visa", "Residence visa")
///     .subject_type("hr.employee")
///     .expires()
///     .reminders(&[60, 30, 7])
///     .responsible_role("pro")
///     .legal_block()
///     .blocks("deployment.activate", BlockMode::Block)
///     .blocks("attendance_record.accept", BlockMode::Flag);
/// assert_eq!(visa.blocks.len(), 2);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindSeed {
    /// The key, `snake_case`.
    pub key: String,
    /// The English name.
    pub name_en: String,
    /// The Arabic name.
    pub name_ar: String,
    /// The subject types it attaches to.
    pub subject_types: Vec<String>,
    /// Whether it has an expiry date.
    pub has_expiry: bool,
    /// Whether it has a number.
    pub has_number: bool,
    /// The days before expiry to remind.
    pub reminder_offsets: Vec<i64>,
    /// Who may see its documents.
    pub sensitivity: Sensitivity,
    /// The retention rule.
    pub retention_rule: Option<String>,
    /// The allowed file extensions ([`DEFAULT_FILE_TYPES`] unless narrowed).
    pub allowed_file_types: Vec<String>,
    /// The largest file, in bytes ([`DEFAULT_MAX_SIZE_BYTES`] unless narrowed).
    pub max_size_bytes: i64,
    /// Whether a portal user may upload one.
    pub portal_upload: bool,
    /// Whether it waits for review (forced on when it has blocks).
    pub needs_review: bool,
    /// The override mode (forced to `never` for a legal block).
    pub override_mode: OverrideMode,
    /// Whether it is a legal block.
    pub legal_block: bool,
    /// The look-ahead days.
    pub look_ahead_days: i64,
    /// Whether the owner is also told on expiry (forced on for a legal block).
    pub compliance_critical: bool,
    /// The role whose holders renew it.
    pub responsible_role: Option<String>,
    /// The roles told on expiry as well as the responsible role.
    pub also_notify: Vec<String>,
    /// The seeded `blocks`.
    pub blocks: Vec<BlockSeed>,
}

impl KindSeed {
    /// A kind `key` named `name_en`, with review on, override `with_approval`, the
    /// default file types and size, and nothing else.
    pub fn new(key: &str, name_en: &str) -> Self {
        KindSeed {
            key: key.to_owned(),
            name_en: name_en.to_owned(),
            name_ar: String::new(),
            subject_types: Vec::new(),
            has_expiry: false,
            has_number: false,
            reminder_offsets: Vec::new(),
            sensitivity: Sensitivity::Normal,
            retention_rule: None,
            allowed_file_types: DEFAULT_FILE_TYPES.iter().map(|t| (*t).to_owned()).collect(),
            max_size_bytes: DEFAULT_MAX_SIZE_BYTES,
            portal_upload: false,
            needs_review: true,
            override_mode: OverrideMode::WithApproval,
            legal_block: false,
            look_ahead_days: 0,
            compliance_critical: false,
            responsible_role: None,
            also_notify: Vec::new(),
            blocks: Vec::new(),
        }
    }

    /// The Arabic name.
    pub fn name_ar(mut self, name: &str) -> Self {
        self.name_ar = name.to_owned();
        self
    }

    /// Adds a subject type it attaches to, e.g. `hr.employee`.
    pub fn subject_type(mut self, subject_type: &str) -> Self {
        self.subject_types.push(subject_type.to_owned());
        self
    }

    /// Its documents have an expiry date.
    pub fn expires(mut self) -> Self {
        self.has_expiry = true;
        self
    }

    /// Its documents have a number.
    pub fn numbered(mut self) -> Self {
        self.has_number = true;
        self
    }

    /// The days before expiry to remind.
    pub fn reminders(mut self, days: &[i64]) -> Self {
        self.reminder_offsets = days.to_vec();
        self
    }

    /// Who may see its documents.
    pub fn sensitivity(mut self, sensitivity: Sensitivity) -> Self {
        self.sensitivity = sensitivity;
        self
    }

    /// The retention rule.
    pub fn retention(mut self, rule: &str) -> Self {
        self.retention_rule = Some(rule.to_owned());
        self
    }

    /// Narrows the allowed file extensions.
    pub fn file_types(mut self, extensions: &[&str]) -> Self {
        self.allowed_file_types = extensions
            .iter()
            .map(|t| t.trim_start_matches('.').to_ascii_lowercase())
            .collect();
        self
    }

    /// Narrows the largest file, in bytes.
    pub fn max_size(mut self, bytes: i64) -> Self {
        self.max_size_bytes = bytes;
        self
    }

    /// A portal user may upload one.
    pub fn portal_upload(mut self) -> Self {
        self.portal_upload = true;
        self
    }

    /// Accepted without review (ignored when it has blocks).
    pub fn without_review(mut self) -> Self {
        self.needs_review = false;
        self
    }

    /// The override mode (default `with_approval`; ignored for a legal
    /// block, which is always `never`).
    pub fn override_mode(mut self, mode: OverrideMode) -> Self {
        self.override_mode = mode;
        self
    }

    /// A legal block: override `never`, compliance-critical, every entry
    /// locked.
    pub fn legal_block(mut self) -> Self {
        self.legal_block = true;
        self.override_mode = OverrideMode::Never;
        self.compliance_critical = true;
        self
    }

    /// The look-ahead days.
    pub fn look_ahead(mut self, days: i64) -> Self {
        self.look_ahead_days = days;
        self
    }

    /// The owner is also told on expiry.
    pub fn compliance_critical(mut self) -> Self {
        self.compliance_critical = true;
        self
    }

    /// The role whose holders renew it, e.g. `pro`.
    pub fn responsible_role(mut self, role: &str) -> Self {
        self.responsible_role = Some(role.to_owned());
        self
    }

    /// The roles told on expiry as well as the responsible role, e.g. `hr`
    /// for a work permit whose responsible role is `pro`.
    pub fn also_notify(mut self, roles: &[&str]) -> Self {
        self.also_notify = roles.iter().map(|r| (*r).to_owned()).collect();
        self
    }

    /// Guards `guarded` (`<entity>.<transition>`) for every record.
    pub fn blocks(mut self, guarded: &str, mode: BlockMode) -> Self {
        self.blocks.push(BlockSeed {
            guarded: guarded.to_owned(),
            mode,
            condition: None,
        });
        self
    }

    /// Guards `guarded` for the records that match `condition`.
    pub fn blocks_when(
        mut self,
        guarded: &str,
        mode: BlockMode,
        condition: BlockCondition,
    ) -> Self {
        self.blocks.push(BlockSeed {
            guarded: guarded.to_owned(),
            mode,
            condition: Some(condition),
        });
        self
    }
}

static SEEDS: RwLock<BTreeMap<String, Arc<KindSeed>>> = RwLock::new(BTreeMap::new());

/// Adds `seed` to the process-wide registry of document kinds; modules and
/// verticals call it in `register`. Registering the same key again replaces
/// it.
pub fn register_kind(seed: KindSeed) {
    SEEDS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(seed.key.clone(), Arc::new(seed));
}

/// The registered kinds, by key.
pub fn kind_seeds() -> Vec<Arc<KindSeed>> {
    SEEDS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .values()
        .cloned()
        .collect()
}

/// Stores the registered kinds ([`install_seeds`] with [`kind_seeds`]).
pub async fn install(db: &Db) -> Result<usize> {
    let seeds = kind_seeds();
    install_seeds(db, seeds.iter().map(Arc::as_ref)).await
}

/// Stores `seeds`, idempotently. A kind the database lacks is created with
/// its entries (locked for a legal block). A kind already there keeps the
/// client's changes and entries, but a legal block is locked again: legal
/// block, override `never`, compliance-critical and review on, and each of
/// its seeded entries is put back with its seeded mode and condition and
/// locked. Returns how many kinds were created.
pub async fn install_seeds<'s>(
    db: &Db,
    seeds: impl IntoIterator<Item = &'s KindSeed>,
) -> Result<usize> {
    let mut tx = db.begin().await?;
    let mut created = 0;
    for seed in seeds {
        let found = DocumentKind::query()
            .where_(DocumentKind::KEY.eq(seed.key.as_str()))
            .first(&mut tx)
            .await?;
        let kind = match found {
            None => {
                let kind = DocumentKind::create(&mut tx, DocumentKind::from_seed(seed)).await?;
                for block in &seed.blocks {
                    let mut row = KindBlock {
                        kind_id: kind.id,
                        guarded: block.guarded.clone(),
                        mode: block.mode,
                        locked: kind.legal_block,
                        ..KindBlock::default()
                    };
                    row.set_condition(block.condition.as_ref());
                    KindBlock::create(&mut tx, row).await?;
                }
                created += 1;
                continue;
            }
            Some(kind) => kind,
        };
        if !seed.legal_block {
            continue;
        }
        let mut kind = kind;
        if kind.lock_as_legal_block() {
            kind.save(&mut tx).await?;
        }
        for block in &seed.blocks {
            let found = KindBlock::query()
                .where_(KindBlock::KIND_ID.eq(kind.id))
                .where_(KindBlock::GUARDED.eq(block.guarded.as_str()))
                .first(&mut tx)
                .await?;
            match found {
                None => {
                    let mut row = KindBlock {
                        kind_id: kind.id,
                        guarded: block.guarded.clone(),
                        mode: block.mode,
                        locked: true,
                        ..KindBlock::default()
                    };
                    row.set_condition(block.condition.as_ref());
                    KindBlock::create(&mut tx, row).await?;
                }
                Some(mut row) => {
                    let condition = row.condition();
                    if row.mode != block.mode || !row.locked || condition != block.condition {
                        row.mode = block.mode;
                        row.locked = true;
                        row.set_condition(block.condition.as_ref());
                        row.save(&mut tx).await?;
                    }
                }
            }
        }
    }
    tx.commit().await?;
    Ok(created)
}

/// The entries that guard `guarded` (`<entity>.<transition>`), by kind key,
/// with each kind's override mode (`Documents::blocks_for`, §6).
pub async fn blocks_for(db: &Db, guarded: &str) -> Result<Vec<BlockEntry>> {
    let rows = KindBlock::query()
        .where_(KindBlock::GUARDED.eq(guarded))
        .get(db)
        .await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let kinds = DocumentKind::find_many(db, rows.iter().map(|r| r.kind_id)).await?;
    let kinds: BTreeMap<i64, DocumentKind> = kinds.into_iter().map(|k| (k.id, k)).collect();
    let mut entries: Vec<BlockEntry> = rows
        .iter()
        .filter_map(|row| kinds.get(&row.kind_id).map(|kind| row.to_entry(kind)))
        .collect();
    entries.sort_by(|a, b| a.kind.cmp(&b.kind));
    Ok(entries)
}

/// Why a change to a kind's `blocks` was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefusalReason {
    /// The entry belongs to a legal block: it cannot be removed or changed.
    Locked,
    /// No module registered a `BlockSubjects` provider for the transition.
    NoProvider,
    /// The kind already guards the transition.
    AlreadyGuarded,
    /// The text is not `<entity>.<transition>`.
    InvalidTransition,
    /// The kind is a legal block: its override mode is locked to `never`.
    LegalBlock,
}

impl RefusalReason {
    /// The text key of the message.
    pub fn text_key(self) -> &'static str {
        match self {
            RefusalReason::Locked => "wl_documents.kinds.locked",
            RefusalReason::NoProvider => "wl_documents.kinds.no_provider",
            RefusalReason::AlreadyGuarded => "wl_documents.kinds.already_guarded",
            RefusalReason::InvalidTransition => "wl_documents.kinds.invalid_transition",
            RefusalReason::LegalBlock => "wl_documents.kinds.override_locked",
        }
    }
}

/// A change to a kind's `blocks` that was refused; the entries are left as
/// they were.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockChangeRefused {
    /// Why.
    pub reason: RefusalReason,
    /// The kind's key.
    pub kind: String,
    /// The kind's English name.
    pub kind_name: String,
    /// The guarded transition.
    pub guarded: String,
}

impl BlockChangeRefused {
    fn new(reason: RefusalReason, kind: &DocumentKind, guarded: &str) -> Self {
        BlockChangeRefused {
            reason,
            kind: kind.key.clone(),
            kind_name: kind.name_en.clone(),
            guarded: guarded.to_owned(),
        }
    }

    fn params(&self) -> [(&str, &dyn fmt::Display); 2] {
        [("kind", &self.kind_name), ("guarded", &self.guarded)]
    }

    /// The message in `lang` (the app's translation, else English).
    pub fn message(&self, lang: &Lang) -> String {
        texts::t_with(lang, self.reason.text_key(), &self.params())
    }

    /// The refusal as a 422 with the English message.
    pub fn into_error(self) -> Error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, self.to_string())
    }
}

impl fmt::Display for BlockChangeRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&texts::english(self.reason.text_key(), &self.params()))
    }
}

impl std::error::Error for BlockChangeRefused {}

/// The result of a change to a kind's `blocks`: the outer `Err` is a
/// failure (the kind or entry does not exist, the database), the inner one
/// a refusal.
pub type BlockChange<T> = Result<std::result::Result<T, BlockChangeRefused>>;

/// Whether `guarded` reads `<entity>.<transition>`, both `snake_case`.
pub fn is_guarded_transition(guarded: &str) -> bool {
    let part = |p: &str| {
        p.starts_with(|c: char| c.is_ascii_lowercase())
            && p.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };
    matches!(guarded.split_once('.'), Some((entity, transition)) if part(entity) && part(transition))
}

async fn kind_or_404(db: &Db, key: &str) -> Result<DocumentKind> {
    DocumentKind::by_key(db, key).await?.ok_or(Error::NotFound)
}

async fn entry_or_404(db: &Db, kind: &DocumentKind, guarded: &str) -> Result<KindBlock> {
    KindBlock::query()
        .where_(KindBlock::KIND_ID.eq(kind.id))
        .where_(KindBlock::GUARDED.eq(guarded))
        .first(db)
        .await?
        .ok_or(Error::NotFound)
}

/// Adds an entry to kind `kind`'s `blocks`: `guarded` in `mode`, for the
/// records that match `condition` (every record without one). Refused when
/// `guarded` is not `<entity>.<transition>`, has no `BlockSubjects`
/// provider, or is already guarded by the kind. An entry of a legal-block
/// kind is locked. Turns review on for the kind (`DOC.ACL.03`).
pub async fn add_block(
    db: &Db,
    kind: &str,
    guarded: &str,
    mode: BlockMode,
    condition: Option<BlockCondition>,
) -> BlockChange<KindBlock> {
    let mut kind = kind_or_404(db, kind).await?;
    let guarded = guarded.trim();
    let refuse = |reason| Ok(Err(BlockChangeRefused::new(reason, &kind, guarded)));
    if !is_guarded_transition(guarded) {
        return refuse(RefusalReason::InvalidTransition);
    }
    if documents::block_subjects(guarded).is_none() {
        return refuse(RefusalReason::NoProvider);
    }
    let exists = KindBlock::query()
        .where_(KindBlock::KIND_ID.eq(kind.id))
        .where_(KindBlock::GUARDED.eq(guarded))
        .exists(db)
        .await?;
    if exists {
        return refuse(RefusalReason::AlreadyGuarded);
    }
    let mut tx = db.begin().await?;
    let mut row = KindBlock {
        kind_id: kind.id,
        guarded: guarded.to_owned(),
        mode,
        locked: kind.legal_block,
        ..KindBlock::default()
    };
    row.set_condition(condition.as_ref());
    let row = KindBlock::create(&mut tx, row).await?;
    if !kind.needs_review {
        kind.needs_review = true;
        kind.save(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(Ok(row))
}

/// Removes the entry `guarded` from kind `kind`'s `blocks`; refused for a
/// locked entry.
pub async fn remove_block(db: &Db, kind: &str, guarded: &str) -> BlockChange<()> {
    let kind = kind_or_404(db, kind).await?;
    let mut row = entry_or_404(db, &kind, guarded).await?;
    if row.locked {
        return Ok(Err(BlockChangeRefused::new(
            RefusalReason::Locked,
            &kind,
            guarded,
        )));
    }
    row.delete(db).await?;
    Ok(Ok(()))
}

/// Switches the entry `guarded` of kind `kind` to `mode`; refused for a
/// locked entry unless it already has that mode.
pub async fn set_block_mode(
    db: &Db,
    kind: &str,
    guarded: &str,
    mode: BlockMode,
) -> BlockChange<KindBlock> {
    let kind = kind_or_404(db, kind).await?;
    let mut row = entry_or_404(db, &kind, guarded).await?;
    if row.mode == mode {
        return Ok(Ok(row));
    }
    if row.locked {
        return Ok(Err(BlockChangeRefused::new(
            RefusalReason::Locked,
            &kind,
            guarded,
        )));
    }
    row.mode = mode;
    row.save(db).await?;
    Ok(Ok(row))
}

/// Sets the override mode of kind `kind` to `mode`. A legal-block kind is
/// locked to `never`: any other mode is refused with
/// [`RefusalReason::LegalBlock`] and the kind is left as it was.
pub async fn set_override_mode(
    db: &Db,
    kind: &str,
    mode: OverrideMode,
) -> BlockChange<DocumentKind> {
    let mut kind = kind_or_404(db, kind).await?;
    if kind.legal_block && mode != OverrideMode::Never {
        return Ok(Err(BlockChangeRefused::new(
            RefusalReason::LegalBlock,
            &kind,
            "",
        )));
    }
    if kind.override_mode != mode {
        kind.override_mode = mode;
        kind.save(db).await?;
    }
    Ok(Ok(kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work_permit() -> KindSeed {
        KindSeed::new("work_permit", "Work permit")
            .subject_type("hr.employee")
            .expires()
            .responsible_role("pro")
            .also_notify(&["hr"])
            .legal_block()
            .blocks("deployment.activate", BlockMode::Block)
            .blocks("attendance_record.accept", BlockMode::Flag)
    }

    #[test]
    fn a_legal_block_seed_is_locked_never_and_critical() {
        let kind =
            DocumentKind::from_seed(&work_permit().override_mode(OverrideMode::WithApproval));
        assert!(kind.legal_block);
        assert_eq!(kind.override_mode, OverrideMode::Never);
        assert!(kind.compliance_critical);
        assert!(kind.needs_review);
        assert!(kind.allows_subject_type("hr.employee"));
        assert!(!kind.allows_subject_type("fleet.vehicle"));
    }

    #[test]
    fn a_seed_allows_an_override_with_approval_unless_it_is_a_legal_block() {
        let plain = DocumentKind::from_seed(&KindSeed::new("x", "X"));
        assert_eq!(plain.override_mode, OverrideMode::WithApproval);
        let never =
            DocumentKind::from_seed(&KindSeed::new("x", "X").override_mode(OverrideMode::Never));
        assert_eq!(never.override_mode, OverrideMode::Never);
        let legal = DocumentKind::from_seed(&KindSeed::new("x", "X").legal_block());
        assert_eq!(legal.override_mode, OverrideMode::Never);
    }

    #[test]
    fn the_legal_block_refusal_names_the_kind() {
        let kind = DocumentKind::from_seed(&work_permit());
        let refused = BlockChangeRefused::new(RefusalReason::LegalBlock, &kind, "");
        assert_eq!(
            refused.to_string(),
            "Work permit is a legal block: its override mode is locked to never"
        );
    }

    #[test]
    fn a_seed_carries_the_roles_to_notify_as_well() {
        let kind = DocumentKind::from_seed(&work_permit());
        assert_eq!(kind.also_notify.0, ["hr"]);
        assert!(
            DocumentKind::from_seed(&KindSeed::new("x", "X"))
                .also_notify
                .0
                .is_empty()
        );
    }

    #[test]
    fn a_kind_with_blocks_always_needs_review() {
        let seed = KindSeed::new("insurance", "Vehicle insurance")
            .without_review()
            .blocks("run.dispatch", BlockMode::Block);
        assert!(DocumentKind::from_seed(&seed).needs_review);
        let plain = KindSeed::new("photo", "Photo").without_review();
        assert!(!DocumentKind::from_seed(&plain).needs_review);
    }

    #[test]
    fn file_types_default_and_narrow() {
        let kind = DocumentKind::from_seed(&KindSeed::new("any", "Any"));
        assert!(kind.allows_file_type("PDF"));
        assert!(kind.allows_file_type(".heic"));
        assert!(!kind.allows_file_type("exe"));
        assert_eq!(kind.max_size_bytes, DEFAULT_MAX_SIZE_BYTES);
        let narrow = DocumentKind::from_seed(&KindSeed::new("scan", "Scan").file_types(&[".PDF"]));
        assert!(narrow.allows_file_type("pdf"));
        assert!(!narrow.allows_file_type("png"));
    }

    #[test]
    fn an_entry_keeps_its_condition_and_the_kind_override_mode() {
        let kind = DocumentKind {
            key: "civil_defence_approval".into(),
            override_mode: OverrideMode::Never,
            ..DocumentKind::default()
        };
        let mut row = KindBlock {
            guarded: "ticket.resolve".into(),
            locked: true,
            ..KindBlock::default()
        };
        let fire = BlockCondition::new("category", ["fire_alarm", "sprinkler"]);
        row.set_condition(Some(&fire));
        let entry = row.to_entry(&kind);
        assert_eq!(entry.kind, "civil_defence_approval");
        assert_eq!(entry.condition, Some(fire));
        assert!(entry.locked);
        row.set_condition(None);
        assert_eq!(row.condition(), None);
        assert!(row.condition_values.0.is_empty());
    }

    #[test]
    fn guarded_transitions_are_entity_dot_transition() {
        assert!(is_guarded_transition("deployment.activate"));
        assert!(is_guarded_transition("attendance_record.accept"));
        assert!(!is_guarded_transition("deployment"));
        assert!(!is_guarded_transition("Deployment.activate"));
        assert!(!is_guarded_transition("deployment."));
        assert!(!is_guarded_transition("a.b.c"));
    }

    #[test]
    fn a_refusal_names_the_kind_and_the_entry() {
        let kind = DocumentKind::from_seed(&work_permit());
        let refused = BlockChangeRefused::new(RefusalReason::Locked, &kind, "deployment.activate");
        assert_eq!(
            refused.to_string(),
            "Work permit is a legal block: deployment.activate cannot be removed or changed"
        );
        match refused.into_error() {
            Error::Status(code, _) => assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY),
            other => panic!("unexpected error {other:?}"),
        }
    }

    struct Nobody;

    impl documents::BlockSubjects for Nobody {
        fn subjects<'a>(
            &'a self,
            _state: &'a AppState,
            _entry: &'a BlockEntry,
            _record_id: i64,
        ) -> documents::DocumentFuture<'a, Vec<wl_kernel::Subject>> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    #[renox::test]
    async fn legal_blocks_stay_locked_and_client_entries_are_kept() {
        use renox::testing::TestApp;

        let app = TestApp::new(App::new().module(crate::WlDocuments)).await;
        let db = app.db();
        assert_eq!(install_seeds(db, [&work_permit()]).await.unwrap(), 1);

        // A locked entry can be neither removed nor switched to flag.
        let refused = remove_block(db, "work_permit", "deployment.activate")
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(refused.reason, RefusalReason::Locked);
        let refused = set_block_mode(db, "work_permit", "deployment.activate", BlockMode::Flag)
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(refused.reason, RefusalReason::Locked);
        let entries = blocks_for(db, "deployment.activate").await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].mode, BlockMode::Block);
        assert!(entries[0].locked);

        // An entry needs a provider; with one it is added.
        let refused = add_block(db, "work_permit", "kinds_unit.go", BlockMode::Block, None)
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(refused.reason, RefusalReason::NoProvider);
        documents::register_block_subjects("kinds_unit.go", Nobody);
        add_block(db, "work_permit", "kinds_unit.go", BlockMode::Block, None)
            .await
            .unwrap()
            .unwrap();
        let refused = add_block(db, "work_permit", "kinds_unit.go", BlockMode::Flag, None)
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(refused.reason, RefusalReason::AlreadyGuarded);

        // A client loosens the kind by hand; the seed locks it again and
        // keeps the entry the client added.
        let mut kind = DocumentKind::by_key(db, "work_permit")
            .await
            .unwrap()
            .unwrap();
        kind.override_mode = OverrideMode::WithApproval;
        kind.legal_block = false;
        kind.save(db).await.unwrap();
        KindBlock::query()
            .where_(KindBlock::GUARDED.eq("deployment.activate"))
            .update(db, &[("mode", &"flag"), ("locked", &false)])
            .await
            .unwrap();
        assert_eq!(install_seeds(db, [&work_permit()]).await.unwrap(), 0);
        let kind = DocumentKind::by_key(db, "work_permit")
            .await
            .unwrap()
            .unwrap();
        assert!(kind.legal_block);
        assert_eq!(kind.override_mode, OverrideMode::Never);
        let blocks = kind.blocks(db).await.unwrap();
        let guarded: Vec<_> = blocks.iter().map(|b| b.guarded.as_str()).collect();
        assert_eq!(
            guarded,
            [
                "attendance_record.accept",
                "deployment.activate",
                "kinds_unit.go"
            ]
        );
        let activate = &blocks[1];
        assert_eq!(activate.mode, BlockMode::Block);
        assert!(activate.locked);
    }

    #[renox::test]
    async fn the_override_mode_changes_except_for_a_legal_block() {
        use renox::testing::TestApp;

        let app = TestApp::new(App::new().module(crate::WlDocuments)).await;
        let db = app.db();
        let health = KindSeed::new("kinds_unit_health", "Health card");
        let legal = KindSeed::new("kinds_unit_legal", "SIRA licence").legal_block();
        install_seeds(db, [&health, &legal]).await.unwrap();

        let kind = set_override_mode(db, "kinds_unit_health", OverrideMode::Never)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(kind.override_mode, OverrideMode::Never);
        let kind = DocumentKind::by_key(db, "kinds_unit_health")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(kind.override_mode, OverrideMode::Never);
        set_override_mode(db, "kinds_unit_health", OverrideMode::WithApproval)
            .await
            .unwrap()
            .unwrap();

        let refused = set_override_mode(db, "kinds_unit_legal", OverrideMode::WithApproval)
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(refused.reason, RefusalReason::LegalBlock);
        assert_eq!(
            refused.to_string(),
            "SIRA licence is a legal block: its override mode is locked to never"
        );
        // Setting never on a legal block is no change.
        set_override_mode(db, "kinds_unit_legal", OverrideMode::Never)
            .await
            .unwrap()
            .unwrap();
        install_seeds(db, [&health, &legal]).await.unwrap();
        let kind = DocumentKind::by_key(db, "kinds_unit_legal")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(kind.override_mode, OverrideMode::Never);
        assert!(matches!(
            set_override_mode(db, "kinds_unit_missing", OverrideMode::Never).await,
            Err(Error::NotFound)
        ));
    }

    #[test]
    fn registering_a_kind_again_replaces_it() {
        register_kind(KindSeed::new("kinds_unit_test", "First"));
        register_kind(KindSeed::new("kinds_unit_test", "Second"));
        let found: Vec<_> = kind_seeds()
            .into_iter()
            .filter(|s| s.key == "kinds_unit_test")
            .collect();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name_en, "Second");
    }
}
