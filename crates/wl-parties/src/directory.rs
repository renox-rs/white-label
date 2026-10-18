//! The module's [`PartyDirectory`] (wl-parties §6): how other modules read
//! parties through the kernel. [`crate::WlParties`] provides it with
//! [`wl_kernel::parties::provide_directory`].
//!
//! Summaries carry the party's `hold_reason`, which is already the display
//! code (`on_hold` for a restricted reason), so no restricted code can leave
//! the module (`PTY.ACL.02`).

use renox::prelude::*;
use wl_kernel::parties::{PartyDirectory, PartyFuture};
use wl_kernel::{PartyId, PartyStatus, PartySummary};

use crate::party::Party;

/// Reads the `wl_parties_parties` table for the kernel contract.
#[derive(Debug, Clone, Copy, Default)]
pub struct Directory;

impl Party {
    /// What other modules see of this party: the display block reason only
    /// while it is blocked.
    pub fn summary(&self) -> PartySummary {
        PartySummary {
            id: self.party_id(),
            code: self.code.clone(),
            kind: self.kind,
            display_name: self.display_name.clone(),
            status: self.status,
            display_block_reason: match self.status {
                PartyStatus::Blocked => self
                    .hold_reason
                    .clone()
                    .filter(|reason| !reason.trim().is_empty()),
                _ => None,
            },
        }
    }
}

impl PartyDirectory for Directory {
    fn find<'a>(&'a self, db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async move { Ok(Party::find(db, party.0).await?.map(|p| p.summary())) })
    }

    fn find_by_code<'a>(
        &'a self,
        db: &'a Db,
        code: &'a str,
    ) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async move {
            let party = Party::query()
                .where_(Party::CODE.eq(code.trim()))
                .first(db)
                .await?;
            Ok(party.map(|p| p.summary()))
        })
    }

    fn display_name<'a>(
        &'a self,
        db: &'a Db,
        party: PartyId,
        lang: &'a str,
    ) -> PartyFuture<'a, Option<String>> {
        Box::pin(async move {
            let party = Party::find(db, party.0).await?;
            Ok(party.map(|p| p.name_in(lang).to_owned()))
        })
    }

    fn status<'a>(&'a self, db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        Box::pin(async move { Ok(Party::find(db, party.0).await?.map(|p| p.status)) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::PartyKind;

    fn party(status: PartyStatus, hold_reason: Option<&str>) -> Party {
        Party {
            id: 7,
            code: "P-00007".into(),
            kind: PartyKind::Company,
            display_name: "Al Noor Trading".into(),
            legal_name_en: "Al Noor Trading LLC".into(),
            status,
            hold_reason: hold_reason.map(str::to_owned),
            ..Default::default()
        }
    }

    #[test]
    fn an_active_party_has_no_display_reason() {
        let summary = party(PartyStatus::Active, None).summary();
        assert_eq!(summary.id, PartyId(7));
        assert_eq!(summary.code, "P-00007");
        assert_eq!(summary.kind, PartyKind::Company);
        assert_eq!(summary.display_name, "Al Noor Trading");
        assert_eq!(summary.status, PartyStatus::Active);
        assert_eq!(summary.display_block_reason, None);
    }

    #[test]
    fn a_blocked_party_shows_its_display_reason() {
        let summary = party(PartyStatus::Blocked, Some("on_hold")).summary();
        assert_eq!(summary.display_block_reason.as_deref(), Some("on_hold"));
        let summary = party(PartyStatus::Blocked, Some("non_payment")).summary();
        assert_eq!(summary.display_block_reason.as_deref(), Some("non_payment"));
    }

    #[test]
    fn a_stale_hold_reason_is_not_shown_once_active() {
        let summary = party(PartyStatus::Active, Some("non_payment")).summary();
        assert_eq!(summary.display_block_reason, None);
    }
}
