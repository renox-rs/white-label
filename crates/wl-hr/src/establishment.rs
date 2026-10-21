//! The MoHRE establishment ID of a legal entity (wl-hr §10 *WPS*: each entity
//! is its own establishment, `HRM.WPS.01`: the SIF file is named with the
//! ID). The kernel's legal entity does not carry it, so wl-hr keeps it
//! ([`Establishment`], one row per entity); the entity's settings page and
//! its audit come with the legal entity settings.
//!
//! A run without a legal entity belongs to the client's default entity
//! (decision: a client with one company has no other).

use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::entities::LegalEntity;

/// The establishment ID of one legal entity.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_hr_establishments", unique(legal_entity_id))]
pub struct Establishment {
    /// The primary key.
    pub id: i64,
    /// The legal entity.
    pub legal_entity_id: i64,
    /// The MoHRE establishment ID, e.g. `EST-1001`.
    pub mohre_establishment_id: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Sets the establishment ID of `legal_entity_id` to `id` (a blank `id` is
/// refused).
pub async fn set(db: &Db, legal_entity_id: i64, id: &str) -> Result<()> {
    let id = id.trim();
    if id.is_empty() {
        return Err(Error::BadRequest("mohre_establishment_id".to_owned()));
    }
    let existing = Establishment::query()
        .where_(Establishment::LEGAL_ENTITY_ID.eq(legal_entity_id))
        .first(db)
        .await?;
    match existing {
        Some(_) => {
            Establishment::query()
                .where_(Establishment::LEGAL_ENTITY_ID.eq(legal_entity_id))
                .update(
                    db,
                    &[
                        ("mohre_establishment_id", &id.to_owned()),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
        }
        None => {
            Establishment::create(
                db,
                Establishment {
                    legal_entity_id,
                    mohre_establishment_id: id.to_owned(),
                    ..Establishment::default()
                },
            )
            .await?;
        }
    }
    Ok(())
}

/// The establishment ID of the entity of a run (`legal_entity_id`, else the
/// default entity), inside `tx`; `None` when none is recorded.
pub async fn id_of_in(
    tx: &mut Transaction,
    legal_entity_id: Option<i64>,
) -> Result<Option<String>> {
    let entity = match legal_entity_id {
        Some(id) => id,
        None => LegalEntity::default_entity(&mut *tx).await?.id,
    };
    Ok(Establishment::query()
        .where_(Establishment::LEGAL_ENTITY_ID.eq(entity))
        .first(&mut *tx)
        .await?
        .map(|e| e.mohre_establishment_id))
}
