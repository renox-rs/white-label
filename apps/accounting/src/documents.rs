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
//! The same kinds guard the filing of a return (`return.file`, flag mode): an
//! expired client KYC document does not stop a return being filed, it flags
//! the return for the MLRO ([`flag_expired`]). `BlockSubjects`: a return →
//! the client company and its UBOs, as for `activate`.
//!
//! An expired KYC document may be overridden once, within 72 hours of the
//! MLRO's approval, by the manager who asked (wl-documents' one-use
//! override). Without `wl-documents` nothing is guarded.

use renox::prelude::*;
use wl_kernel::documents::{
    BlockEntry, BlockSubjects, DocumentFuture, RecordFlag, guard, register_block_subjects,
};
use wl_kernel::parties::{PARTY_SUBJECT_TYPE, directory};
use wl_kernel::{PartyId, Subject};

use crate::cdd::{CddFile, ScreenedRole};
use crate::engagement::Engagement;
use crate::tax_return::Return;

/// The guarded transition `activate` of an engagement.
pub const ACTIVATE_GUARDED: &str = "engagement.activate";

/// The guarded transition `file` of a return.
pub const RETURN_FILE_GUARDED: &str = "return.file";

/// The record type the flags of a return are kept under.
pub const RETURN_RECORD_TYPE: &str = "return";

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

/// `return.file`: the client company and its UBOs, as the entry's condition
/// on their role says.
struct ReturnSubjects;

impl BlockSubjects for ReturnSubjects {
    fn subjects<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            let Some(ret) = Return::find(&state.db, record_id).await? else {
                return Ok(Vec::new());
            };
            let Some(engagement) = Engagement::find(&state.db, ret.engagement_id).await? else {
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
        ActivateSubjects.label(state, subject)
    }
}

/// Registers the providers.
pub(crate) fn register() {
    register_block_subjects(ACTIVATE_GUARDED, ActivateSubjects);
    register_block_subjects(RETURN_FILE_GUARDED, ReturnSubjects);
}

/// After `ret` was filed: the client KYC documents judged on the filing
/// date; every one that is expired or missing flags the return for the MLRO
/// (flag mode: the filing already ran). Answers the flags written now.
///
/// # Errors
///
/// The documents module's and the database's errors.
pub async fn flag_expired(state: &AppState, ret: &Return) -> Result<usize> {
    let on = ret.filed_on.unwrap_or_else(wl_kernel::documents::today);
    let documents = guard(state);
    match documents
        .check(state, RETURN_FILE_GUARDED, ret.id, on)
        .await?
    {
        Ok(outcome) if !outcome.flags.is_empty() => Ok(documents
            .record_flags(&state.db, RETURN_FILE_GUARDED, ret.id, &outcome.flags)
            .await?
            .len()),
        _ => Ok(0),
    }
}

/// The flags of return `id`, oldest first, reviewed or not.
///
/// # Errors
///
/// The documents module's errors.
pub async fn flags_of(state: &AppState, id: i64) -> Result<Vec<RecordFlag>> {
    guard(state)
        .flags_for(&state.db, RETURN_RECORD_TYPE, id)
        .await
}
