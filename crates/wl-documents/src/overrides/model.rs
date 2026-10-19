//! The [`BlockOverride`] record (wl-documents §3 *Block override*).

use renox::auth::permissions::Scope;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// Where an override stands (§4.3).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlockOverrideStatus {
    /// Asked for; the approval is pending.
    #[default]
    Requested,
    /// Approved: the requester may use it once until `valid_until`.
    Approved,
    /// The approval ended without a grant.
    Refused,
    /// Used by the requester.
    Used,
}

/// The permission, once approved, to run one guarded transition on one
/// record although a kind blocks it. Its status moves only through the
/// machine ([`crate::overrides::machine`]).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_documents_block_overrides",
    state_machine,
    index(kind_id, subject_type, subject_id, guarded, record_id),
    index(requested_by, status)
)]
pub struct BlockOverride {
    /// The primary key.
    pub id: i64,
    /// The kind that blocks.
    #[model(references = "wl_documents_kinds")]
    pub kind_id: i64,
    /// The document subject's type, e.g. `hr.employee`.
    pub subject_type: String,
    /// The document subject's id.
    pub subject_id: i64,
    /// The guarded transition, `<entity>.<transition>`.
    pub guarded: String,
    /// The guarded record's id.
    pub record_id: i64,
    /// The record's scope type when requested; `None` for global.
    pub scope_type: Option<String>,
    /// The record's scope id when requested; `None` for global.
    pub scope_id: Option<String>,
    /// Why the override is asked for.
    pub reason: String,
    /// Who asked; only this user may use it.
    pub requested_by: i64,
    /// Who gave the last yes.
    pub approved_by: Option<i64>,
    /// When it was approved.
    pub approved_at: Option<DateTime>,
    /// The last instant it may be used.
    pub valid_until: Option<DateTime>,
    /// When it was used; empty while unused.
    pub used_at: Option<DateTime>,
    /// The kernel approval request.
    pub approval_request_id: Option<i64>,
    /// Where it stands.
    pub status: BlockOverrideStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl BlockOverride {
    /// The document subject the override is for.
    pub fn subject(&self) -> Subject {
        Subject::new(&self.subject_type, self.subject_id)
    }

    /// The scope stored when requested; global when none.
    pub fn scope(&self) -> Scope {
        match (&self.scope_type, &self.scope_id) {
            (Some(kind), Some(id)) => Scope::new(kind, id),
            _ => Scope::global(),
        }
    }

    /// Whether the override is approved, unused and still valid at `at`.
    pub fn usable_at(&self, at: DateTime) -> bool {
        self.status == BlockOverrideStatus::Approved
            && self.used_at.is_none()
            && self.valid_until.is_some_and(|until| at <= until)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::Duration;
    use renox::db::now;

    fn approved(until: DateTime) -> BlockOverride {
        BlockOverride {
            status: BlockOverrideStatus::Approved,
            valid_until: Some(until),
            ..Default::default()
        }
    }

    #[test]
    fn a_new_override_is_requested_and_global() {
        let o = BlockOverride::default();
        assert_eq!(o.status, BlockOverrideStatus::Requested);
        assert!(o.scope().is_global());
    }

    #[test]
    fn the_stored_scope_is_the_overrides_scope() {
        let o = BlockOverride {
            scope_type: Some("site".into()),
            scope_id: Some("7".into()),
            ..Default::default()
        };
        assert_eq!(o.scope(), Scope::new("site", 7));
    }

    #[test]
    fn an_override_is_usable_until_its_valid_until() {
        let at = now();
        let o = approved(at);
        assert!(o.usable_at(at));
        assert!(!o.usable_at(at + Duration::minutes(1)));
        let used = BlockOverride {
            used_at: Some(at),
            ..o
        };
        assert!(!used.usable_at(at));
        assert!(!BlockOverride::default().usable_at(at));
    }
}
