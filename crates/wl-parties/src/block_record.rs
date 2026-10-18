//! The Block record (wl-parties §3): the trace of one `block`, with the real
//! reason. One is kept per `block`, and one per confirmed screening match on
//! a party that is already `blocked` or `archived` (added without a
//! transition).
//!
//! The real reason code lives only here, never in the party's
//! `hold_reason`, an event or a history row. A restricted record is readable
//! only under `parties.block_reason.view` (`PTY.ACL.02`); see
//! [`crate::visibility`].

use renox::db::Executor;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::evidence::UnblockBasis;

/// One block of a party.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_parties_block_records", index(party_id))]
pub struct BlockRecord {
    /// The primary key.
    pub id: i64,
    /// The party.
    pub party_id: i64,
    /// The real reason code, e.g. `aml_concern`.
    pub reason_code: String,
    /// Whether the reason code was restricted when the party was blocked.
    pub restricted: bool,
    /// Why, in the blocker's words.
    pub note: String,
    /// The user who blocked, or whose action (e.g. `confirm_match`) caused
    /// a system block; `None` for a pure system run.
    pub blocked_by: Option<i64>,
    /// When the party was blocked.
    pub blocked_at: DateTime,
    /// The user who closed the record by unblocking.
    pub unblocked_by: Option<i64>,
    /// When the record was closed.
    pub unblocked_at: Option<DateTime>,
    /// Why it was closed.
    pub unblock_note: Option<String>,
    /// The statement a `sanctions` block was lifted on.
    pub unblock_basis: Option<UnblockBasis>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl BlockRecord {
    /// A new, open record for `party_id`, blocked now.
    pub fn open(
        party_id: i64,
        reason_code: &str,
        restricted: bool,
        note: &str,
        blocked_by: Option<i64>,
    ) -> BlockRecord {
        BlockRecord {
            party_id,
            reason_code: reason_code.to_owned(),
            restricted,
            note: note.trim().to_owned(),
            blocked_by,
            blocked_at: renox::db::now(),
            ..BlockRecord::default()
        }
    }

    /// Whether the record is still open (the block has not been lifted).
    pub fn is_open(&self) -> bool {
        self.unblocked_at.is_none()
    }

    /// The reason code others may see: the code, or `on_hold` when it is
    /// restricted.
    pub fn display_reason(&self) -> &str {
        wl_kernel::parties::display_block_reason(&self.reason_code, self.restricted)
    }

    /// The party's records, newest first. Every record, restricted ones
    /// included: filter with [`crate::visibility::visible_records`] before
    /// showing them to a user.
    pub async fn for_party<'c>(db: impl Executor<'c>, party_id: i64) -> Result<Vec<BlockRecord>> {
        BlockRecord::query()
            .where_(BlockRecord::PARTY_ID.eq(party_id))
            .order_by_desc(BlockRecord::BLOCKED_AT)
            .order_by_desc(BlockRecord::ID)
            .get(db)
            .await
    }

    /// Closes the record: unblocked by `user` now, with `note` and the
    /// `basis`, if any. Call `save` to store it.
    pub fn close(&mut self, user: Option<i64>, note: &str, basis: Option<UnblockBasis>) {
        self.unblocked_by = user;
        self.unblocked_at = Some(renox::db::now());
        self.unblock_note = Some(note.trim().to_owned());
        self.unblock_basis = basis;
    }

    /// The party's open records, oldest first.
    pub async fn open_for<'c>(db: impl Executor<'c>, party_id: i64) -> Result<Vec<BlockRecord>> {
        BlockRecord::open_for_party(db, party_id).await
    }

    /// The party's open records, oldest first.
    pub async fn open_for_party<'c>(
        db: impl Executor<'c>,
        party_id: i64,
    ) -> Result<Vec<BlockRecord>> {
        BlockRecord::query()
            .where_(BlockRecord::PARTY_ID.eq(party_id))
            .where_(BlockRecord::UNBLOCKED_AT.is_null())
            .order_by(BlockRecord::BLOCKED_AT)
            .order_by(BlockRecord::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_record_is_open_and_keeps_the_real_code() {
        let record = BlockRecord::open(7, "aml_concern", true, "  flagged  ", None);
        assert_eq!(record.party_id, 7);
        assert_eq!(record.reason_code, "aml_concern");
        assert!(record.restricted);
        assert_eq!(record.note, "flagged");
        assert_eq!(record.blocked_by, None);
        assert!(record.is_open());
        assert_eq!(record.display_reason(), "on_hold");
    }

    #[test]
    fn closing_a_record_stores_who_when_and_why() {
        let mut record = BlockRecord::open(7, "sanctions", true, "match", Some(3));
        record.close(
            Some(5),
            "  match found false ",
            Some(UnblockBasis::FalseMatch),
        );
        assert!(!record.is_open());
        assert_eq!(record.unblocked_by, Some(5));
        assert_eq!(record.unblock_note.as_deref(), Some("match found false"));
        assert_eq!(record.unblock_basis, Some(UnblockBasis::FalseMatch));
        assert_eq!(record.blocked_by, Some(3));
    }

    #[test]
    fn a_non_restricted_record_shows_its_code() {
        let record = BlockRecord::open(7, "non_payment", false, "3 bounced cheques", Some(3));
        assert_eq!(record.display_reason(), "non_payment");
        assert_eq!(record.blocked_by, Some(3));
    }
}
