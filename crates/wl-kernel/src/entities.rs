//! Legal entities (`PLT.ENT.01`, [decision 0003]): the companies of the
//! client's group that issue documents, which entity a scope belongs to, and
//! the issuer snapshot frozen on a document when it is issued.
//!
//! [decision 0003]: https://github.com/renox-rs/white-label/blob/main/docs/decisions/0003-legal-entities.md

use renox::chrono::NaiveDate;
use renox::db::Executor;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::config::{ClientConfig, LegalEntityConfig};

/// One company of the client's group.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_legal_entities", unique(code))]
pub struct LegalEntity {
    /// The primary key.
    pub id: i64,
    /// The code in document numbers, e.g. `ACME`. Never changes once a
    /// document carries it.
    pub code: String,
    /// The legal name in English.
    pub name_en: String,
    /// The legal name in Arabic.
    pub name_ar: Option<String>,
    /// The trade licence number.
    pub trade_licence_no: Option<String>,
    /// When the trade licence expires.
    pub trade_licence_expires_on: Option<NaiveDate>,
    /// The licensing authority: a mainland emirate's department or a free zone.
    pub licensing_authority: Option<String>,
    /// The emirate, e.g. `dubai`.
    pub emirate: Option<String>,
    /// The tax registration number (15 digits), when VAT registered.
    pub trn: Option<String>,
    /// Whether the entity charges VAT.
    pub vat_registered: bool,
    /// The address in English.
    pub address_en: Option<String>,
    /// The address in Arabic.
    pub address_ar: Option<String>,
    /// The footer on documents, English.
    pub footer_en: Option<String>,
    /// The footer on documents, Arabic.
    pub footer_ar: Option<String>,
    /// The entity used when a document's scope names none. Exactly one is.
    pub is_default: bool,
    /// `active` or `inactive`. An inactive entity issues nothing new.
    pub status: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Which entity a scope unit (a building, a project, a branch…) belongs to.
/// Exactly one entity per scope.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_scope_entities", unique(scope_type, scope_id))]
pub struct ScopeEntity {
    /// The primary key.
    pub id: i64,
    /// A subject type, e.g. `property.building`.
    pub scope_type: String,
    /// The scope record's id.
    pub scope_id: i64,
    /// The entity it belongs to.
    pub legal_entity_id: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The issuer's details as they were when a document was issued. Stored on
/// the document (as JSON) and never updated afterwards.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct IssuerSnapshot {
    /// The entity's id.
    pub legal_entity_id: i64,
    /// Its code.
    pub code: String,
    /// The legal name in English.
    pub name_en: String,
    /// The legal name in Arabic.
    pub name_ar: Option<String>,
    /// The TRN, when VAT registered.
    pub trn: Option<String>,
    /// The trade licence number.
    pub trade_licence_no: Option<String>,
    /// The address in English.
    pub address_en: Option<String>,
    /// The address in Arabic.
    pub address_ar: Option<String>,
    /// The footer, English.
    pub footer_en: Option<String>,
    /// The footer, Arabic.
    pub footer_ar: Option<String>,
    /// When it was taken.
    pub taken_at: DateTime,
}

/// Whether `text` is a UAE TRN: 15 digits.
pub fn is_trn(text: &str) -> bool {
    text.len() == 15 && text.bytes().all(|b| b.is_ascii_digit())
}

impl LegalEntity {
    /// The default entity.
    pub async fn default_entity<'c>(db: impl Executor<'c>) -> Result<LegalEntity> {
        LegalEntity::query()
            .where_(LegalEntity::IS_DEFAULT.eq(true))
            .first(db)
            .await?
            .ok_or_else(|| {
                renox::anyhow::anyhow!("no default legal entity: run `wl:install` after `migrate`")
                    .into()
            })
    }

    /// The entity that issues documents for a scope: the one linked to it,
    /// else the default (wl-kernel §5.6). `None` means no scope.
    pub async fn for_scope(db: &Db, scope: Option<(&str, i64)>) -> Result<LegalEntity> {
        if let Some((scope_type, scope_id)) = scope
            && let Some(link) = ScopeEntity::query()
                .where_(ScopeEntity::SCOPE_TYPE.eq(scope_type))
                .where_(ScopeEntity::SCOPE_ID.eq(scope_id))
                .first(db)
                .await?
        {
            return LegalEntity::find_or_404(db, link.legal_entity_id).await;
        }
        LegalEntity::default_entity(db).await
    }

    /// Links a scope to this entity, replacing an earlier link.
    pub async fn link_scope(
        &self,
        db: &Db,
        scope_type: &str,
        scope_id: i64,
    ) -> Result<ScopeEntity> {
        let entity_id = self.id;
        ScopeEntity::query()
            .where_(ScopeEntity::SCOPE_TYPE.eq(scope_type))
            .where_(ScopeEntity::SCOPE_ID.eq(scope_id))
            .update_or_create(
                db,
                || ScopeEntity {
                    scope_type: scope_type.to_owned(),
                    scope_id,
                    ..Default::default()
                },
                |link| link.legal_entity_id = entity_id,
            )
            .await
    }

    /// The details to freeze on a document issued now.
    pub fn snapshot(&self) -> IssuerSnapshot {
        IssuerSnapshot {
            legal_entity_id: self.id,
            code: self.code.clone(),
            name_en: self.name_en.clone(),
            name_ar: self.name_ar.clone(),
            trn: self.trn.clone().filter(|_| self.vat_registered),
            trade_licence_no: self.trade_licence_no.clone(),
            address_en: self.address_en.clone(),
            address_ar: self.address_ar.clone(),
            footer_en: self.footer_en.clone(),
            footer_ar: self.footer_ar.clone(),
            taken_at: renox::db::now(),
        }
    }

    /// Whether the entity may issue new documents.
    pub fn is_active(&self) -> bool {
        self.status == "active"
    }

    fn from_config(config: &LegalEntityConfig) -> Self {
        LegalEntity {
            code: config.code.clone(),
            name_en: config.name_en.clone(),
            name_ar: config.name_ar.clone(),
            trade_licence_no: config.trade_licence_no.clone(),
            licensing_authority: config.licensing_authority.clone(),
            emirate: config.emirate.clone(),
            trn: config.trn.clone(),
            vat_registered: config.trn.is_some(),
            address_en: config.address_en.clone(),
            address_ar: config.address_ar.clone(),
            footer_en: config.footer_en.clone(),
            footer_ar: config.footer_ar.clone(),
            is_default: config.default,
            status: "active".into(),
            ..Default::default()
        }
    }
}

/// Creates the entities `client.toml` lists that the database does not have
/// yet (by code). Entities already there are left as they are: later boots
/// never overwrite values changed in the admin (wl-kernel §5.1). Returns how
/// many were created.
pub async fn install(db: &Db, config: &ClientConfig) -> Result<usize> {
    let mut tx = db.begin().await?;
    let any_default = LegalEntity::query()
        .where_(LegalEntity::IS_DEFAULT.eq(true))
        .exists(&mut tx)
        .await?;
    let mut created = 0;
    for entity in &config.legal_entities {
        let known = LegalEntity::query()
            .where_(LegalEntity::CODE.eq(entity.code.as_str()))
            .exists(&mut tx)
            .await?;
        if known {
            continue;
        }
        let mut row = LegalEntity::from_config(entity);
        // A new entity never takes the default from one chosen in the admin.
        row.is_default &= !any_default;
        LegalEntity::create(&mut tx, row).await?;
        created += 1;
    }
    tx.commit().await?;
    Ok(created)
}
