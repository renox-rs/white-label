//! The required readers of an SOP: [`Sops::set_readers`], [`Sops::readers`]
//! and the lookup [`Sops::required_for`].
//!
//! Readers are stored and published as definitions; resolving them to
//! employees belongs to the acknowledgements (#53).

use std::collections::BTreeSet;

use renox::auth::permissions::Scope;
use renox::prelude::*;
use wl_kernel::documents::{SopId, SopReaderRef, SopReadersChanged};

use super::model::{Sop, SopReader, SopStatus};
use super::service::{Sops, refused, require_manage};
use crate::texts;

impl From<&SopReader> for SopReaderRef {
    fn from(row: &SopReader) -> Self {
        SopReaderRef {
            all_staff: row.all_staff,
            role: row.role_key.clone(),
            scope_type: row.scope_type.clone(),
            scope_id: row.scope_id.clone(),
        }
    }
}

/// A reader definition as stored: trimmed, an all-staff reader without role
/// or scope, a scope given as both type and id or not at all.
/// Returns the text key of the problem otherwise.
fn normalize(reader: &SopReaderRef) -> std::result::Result<SopReaderRef, &'static str> {
    if reader.all_staff {
        return Ok(SopReaderRef {
            all_staff: true,
            role: String::new(),
            scope_type: String::new(),
            scope_id: String::new(),
        });
    }
    let role = reader.role.trim().to_owned();
    let scope_type = reader.scope_type.trim().to_owned();
    let scope_id = reader.scope_id.trim().to_owned();
    if role.is_empty() {
        return Err("wl_documents.sops.refused.blank_reader_role");
    }
    if scope_type.is_empty() != scope_id.is_empty() {
        return Err("wl_documents.sops.refused.bad_reader_scope");
    }
    Ok(SopReaderRef {
        all_staff: false,
        role,
        scope_type,
        scope_id,
    })
}

/// Whether `reader` applies to `role` working in `scope`: an all-staff
/// reader matches every role; a role reader the same role, in every scope
/// (no scope given) or in exactly `scope`.
pub fn matches(reader: &SopReaderRef, role: &str, scope: &Scope) -> bool {
    if reader.all_staff {
        return true;
    }
    if reader.role != role {
        return false;
    }
    reader.scope_type.is_empty()
        || (reader.scope_type == scope.kind() && reader.scope_id == scope.id())
}

impl Sops {
    /// The required readers of `sop`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn readers(db: &Db, sop: i64) -> Result<Vec<SopReaderRef>> {
        Ok(reader_rows(db, sop)
            .await?
            .iter()
            .map(SopReaderRef::from)
            .collect())
    }

    /// Replaces the required readers of `sop` with `readers` (duplicates
    /// collapse). For an `active` SOP it emits `SopReadersChanged` with the
    /// readers added and removed (a changed scope counts as both), and
    /// nothing when nothing changed; a `draft` SOP emits nothing.
    ///
    /// # Errors
    ///
    /// `Forbidden` without `sops.manage`; `NotFound` for an unknown SOP; 422
    /// for a `withdrawn` SOP, a reader without a role, or a scope with only
    /// a type or an id; the database's errors.
    pub async fn set_readers(
        state: &AppState,
        by: &AuthUser,
        sop: i64,
        readers: &[SopReaderRef],
    ) -> Result<Vec<SopReaderRef>> {
        require_manage(by)?;
        let lang = state.current_lang();
        let row = Sop::find_or_404(&state.db, sop).await?;
        let status = Sops::status(&state.db, &row).await?;
        if status == SopStatus::Withdrawn {
            return Err(refused(texts::t_with(
                &lang,
                "wl_documents.sops.refused.withdrawn",
                &[("code", &row.code)],
            )));
        }
        let mut wanted: BTreeSet<SopReaderRef> = BTreeSet::new();
        for reader in readers {
            wanted.insert(normalize(reader).map_err(|key| refused(texts::t(&lang, key)))?);
        }

        let mut tx = state.db.begin().await?;
        let existing = reader_rows(&mut tx, row.id).await?;
        let current: BTreeSet<SopReaderRef> = existing.iter().map(SopReaderRef::from).collect();
        let removed: Vec<SopReaderRef> = current.difference(&wanted).cloned().collect();
        let added: Vec<SopReaderRef> = wanted.difference(&current).cloned().collect();
        for mut gone in existing
            .into_iter()
            .filter(|r| !wanted.contains(&SopReaderRef::from(r)))
        {
            gone.delete(&mut tx).await?;
        }
        for reader in &added {
            SopReader::create(
                &mut tx,
                SopReader {
                    sop_id: row.id,
                    all_staff: reader.all_staff,
                    role_key: reader.role.clone(),
                    scope_type: reader.scope_type.clone(),
                    scope_id: reader.scope_id.clone(),
                    ..Default::default()
                },
            )
            .await?;
        }
        tx.commit().await?;

        if status == SopStatus::Active && (!added.is_empty() || !removed.is_empty()) {
            state
                .emit(SopReadersChanged {
                    sop: SopId(row.id),
                    added,
                    removed,
                })
                .await?;
        }
        Self::readers(&state.db, row.id).await
    }

    /// The `active` SOPs that `role` working in `scope` must read: those
    /// with a reader for that role in that scope (or in every scope) and
    /// those with an all-staff reader. A pure lookup; it does not resolve
    /// employees.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn required_for(db: &Db, role: &str, scope: &Scope) -> Result<Vec<Sop>> {
        let mut found = Vec::new();
        for sop in Sops::list(db, Some(SopStatus::Active)).await? {
            let readers = Self::readers(db, sop.id).await?;
            if readers.iter().any(|r| matches(r, role, scope)) {
                found.push(sop);
            }
        }
        Ok(found)
    }
}

/// The reader rows of `sop`, oldest first.
async fn reader_rows<'c, E: renox::db::Executor<'c>>(db: E, sop: i64) -> Result<Vec<SopReader>> {
    SopReader::query()
        .where_(SopReader::SOP_ID.eq(sop))
        .order_by(SopReader::ID)
        .get(db)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(role: &str, scope: Option<(&str, &str)>) -> SopReaderRef {
        SopReaderRef {
            all_staff: false,
            role: role.into(),
            scope_type: scope.map_or("", |s| s.0).into(),
            scope_id: scope.map_or("", |s| s.1).into(),
        }
    }

    #[test]
    fn an_all_staff_reader_drops_role_and_scope() {
        let all = SopReaderRef {
            all_staff: true,
            ..role("x", Some(("property.site", "1")))
        };
        let n = normalize(&all).unwrap();
        assert!(n.all_staff && n.role.is_empty() && n.scope_type.is_empty());
    }

    #[test]
    fn a_role_reader_needs_a_role_and_a_whole_scope() {
        assert!(normalize(&role(" ", None)).is_err());
        assert!(normalize(&role("a", Some(("property.site", "")))).is_err());
        assert!(normalize(&role("a", Some(("", "1")))).is_err());
        assert_eq!(
            normalize(&role(" worker ", None)).unwrap(),
            role("worker", None)
        );
    }

    #[test]
    fn a_reader_matches_its_role_in_its_scope() {
        let site1 = Scope::new("property.site", 1);
        let site2 = Scope::new("property.site", 2);
        let everywhere = role("worker", None);
        assert!(matches(&everywhere, "worker", &site1));
        assert!(matches(&everywhere, "worker", &Scope::global()));
        assert!(!matches(&everywhere, "driver", &site1));
        let scoped = role("worker", Some(("property.site", "1")));
        assert!(matches(&scoped, "worker", &site1));
        assert!(!matches(&scoped, "worker", &site2));
        assert!(!matches(&scoped, "worker", &Scope::global()));
        let all = SopReaderRef {
            all_staff: true,
            ..role("", None)
        };
        assert!(matches(&all, "anyone", &site2));
    }
}
