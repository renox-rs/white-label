//! The document guard hook of engagement `activate` (04 §9 *Document
//! blocks*, `DOC.ACL.02`, BL-ACC-01).
//!
//! Which documents `activate` needs is not decided here: it is the `blocks`
//! of the document kinds (wl-documents §5.6), which 04 seeds
//! ([`crate::seed::KYC_KINDS`]): the client company's trade licence, and the
//! passport and Emirates ID of each UBO. This module registers the
//! [`BlockSubjects`] provider that maps an engagement to those parties:
//! `BlockSubjects`: an engagement → the client company and its UBOs (from
//! its current CDD file). An entry's condition on [`PARTY_ROLE_FIELD`]
//! (`client`, `ubo`) picks which of them must hold the kind; an entry with no
//! condition needs it from all of them. A missing document fails like an
//! expired one.
//!
//! An expired KYC document may be overridden once, within 72 hours of the
//! MLRO's approval, by the manager who asked (wl-documents' one-use
//! override). Without `wl-documents` nothing is guarded.

use renox::prelude::*;
use wl_kernel::documents::{BlockEntry, BlockSubjects, DocumentFuture, register_block_subjects};
use wl_kernel::parties::{PARTY_SUBJECT_TYPE, directory};
use wl_kernel::{PartyId, Subject};

use crate::cdd::{CddFile, ScreenedRole};
use crate::engagement::Engagement;

/// The guarded transition `activate` of an engagement.
pub const ACTIVATE_GUARDED: &str = "engagement.activate";

/// The field the KYC entries' conditions read: the role of the party
/// (`client` or `ubo`).
pub const PARTY_ROLE_FIELD: &str = "party_role";

/// The client company and the UBOs of `engagement`, with their role.
///
/// # Errors
///
/// The database's errors.
pub async fn kyc_parties(db: &Db, engagement: &Engagement) -> Result<Vec<(PartyId, ScreenedRole)>> {
    let mut parties = vec![(engagement.client(), ScreenedRole::Client)];
    if let Some(file) = CddFile::current(db, engagement.client()).await? {
        for row in file.parties(db).await? {
            if row.role == ScreenedRole::Ubo && !parties.contains(&(row.party(), row.role)) {
                parties.push((row.party(), row.role));
            }
        }
    }
    Ok(parties)
}

/// `engagement.activate`: the client company and its UBOs, as the entry's
/// condition on their role says.
struct ActivateSubjects;

impl BlockSubjects for ActivateSubjects {
    fn subjects<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            let Some(engagement) = Engagement::find(&state.db, record_id).await? else {
                return Ok(Vec::new());
            };
            Ok(kyc_parties(&state.db, &engagement)
                .await?
                .into_iter()
                .filter(|(_, role)| entry.applies_to(Some(role.as_str())))
                .map(|(party, _)| party.subject())
                .collect::<Vec<Subject>>())
        })
    }

    fn label<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
    ) -> DocumentFuture<'a, Option<String>> {
        Box::pin(async move {
            if subject.subject_type != PARTY_SUBJECT_TYPE {
                return Ok(None);
            }
            directory(state)
                .display_name(&state.db, PartyId(subject.subject_id), "en")
                .await
        })
    }
}

/// Registers the provider.
pub(crate) fn register() {
    register_block_subjects(ACTIVATE_GUARDED, ActivateSubjects);
}
