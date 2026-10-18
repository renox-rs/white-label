//! The evidence behind an unblock (wl-parties §4 `unblock`): the basis a
//! `sanctions` block is lifted on, and the files attached to the closed
//! Block record. The files are private and live on the default storage disk
//! at `parties/<party_id>/unblock/<block_record_id>/<file name>`.

use renox::db::Executor;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// Why a `sanctions` block may be lifted. Screening does not exist yet, so
/// the module records this statement and does not check a list.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum UnblockBasis {
    /// The party is no longer listed.
    #[default]
    Delisted,
    /// The match was found to be false.
    FalseMatch,
}

/// One evidence file of a closed Block record.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_parties_unblock_evidence", index(block_record_id))]
pub struct UnblockEvidence {
    /// The primary key.
    pub id: i64,
    /// The Block record it belongs to.
    pub block_record_id: i64,
    /// Where the file is on the storage disk.
    pub storage_key: String,
    /// The file's name.
    pub file_name: String,
    /// The file's MIME type.
    pub mime_type: String,
    /// The size in bytes.
    pub size: i64,
    /// The user who attached it.
    pub uploaded_by: Option<i64>,
    /// When it was attached.
    pub uploaded_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl UnblockEvidence {
    /// The storage key of `file_name` for a Block record.
    pub fn key_for(party_id: i64, block_record_id: i64, file_name: &str) -> String {
        format!("parties/{party_id}/unblock/{block_record_id}/{file_name}")
    }

    /// A new evidence row, attached now.
    pub fn new(
        block_record_id: i64,
        storage_key: &str,
        file_name: &str,
        mime_type: &str,
        size: i64,
        uploaded_by: Option<i64>,
    ) -> UnblockEvidence {
        UnblockEvidence {
            block_record_id,
            storage_key: storage_key.to_owned(),
            file_name: file_name.to_owned(),
            mime_type: mime_type.to_owned(),
            size,
            uploaded_by,
            uploaded_at: renox::db::now(),
            ..UnblockEvidence::default()
        }
    }

    /// The files of a Block record, oldest first.
    pub async fn for_record<'c>(
        db: impl Executor<'c>,
        block_record_id: i64,
    ) -> Result<Vec<UnblockEvidence>> {
        UnblockEvidence::query()
            .where_(UnblockEvidence::BLOCK_RECORD_ID.eq(block_record_id))
            .order_by(UnblockEvidence::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_basis_is_stored_as_snake_case_text() {
        assert_eq!(UnblockBasis::Delisted.to_string(), "delisted");
        assert_eq!(UnblockBasis::FalseMatch.to_string(), "false_match");
        assert_eq!(
            "false_match".parse::<UnblockBasis>().ok(),
            Some(UnblockBasis::FalseMatch)
        );
        assert!("other".parse::<UnblockBasis>().is_err());
    }

    #[test]
    fn the_storage_key_is_private_and_per_record() {
        assert_eq!(
            UnblockEvidence::key_for(4, 9, "match.pdf"),
            "parties/4/unblock/9/match.pdf"
        );
        let row = UnblockEvidence::new(9, "k", "match.pdf", "application/pdf", 12, Some(2));
        assert_eq!(row.block_record_id, 9);
        assert_eq!(row.size, 12);
        assert_eq!(row.uploaded_by, Some(2));
    }
}
