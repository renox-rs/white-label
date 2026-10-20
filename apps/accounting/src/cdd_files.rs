//! The CDD file services (04 §7.1 *CDD file*): open a file, list the parties
//! it screens, rate it, and run its transitions, each gathering what the
//! guards need first.

use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::PartyId;
use wl_kernel::documents::today;

use crate::cdd::{CddFile, CddParty, CddStatus, RiskRating, ScreenedRole};
use crate::cdd_machine::{APPROVE, CddInput, EXIT, FLAG_REVIEW, REJECT, REOPEN, SUBMIT};
use crate::db::{refused, set_cdd_file};
use crate::permissions;
use crate::texts::english;

/// The file `id`, or 404.
async fn load(state: &AppState, id: i64) -> Result<CddFile> {
    CddFile::find_or_404(&state.db, id).await
}

/// Opens an `in_progress` CDD file for `client`, with the client company
/// as its first screened party.
///
/// # Errors
///
/// The database's errors.
pub async fn open(db: &Db, by: Option<i64>, client: PartyId) -> Result<CddFile> {
    let mut tx = db.begin().await?;
    let mut file = CddFile {
        client_party_id: client.0,
        created_by: by,
        ..CddFile::default()
    };
    file.insert(&mut tx).await?;
    add_party_in(&mut tx, &file, client, ScreenedRole::Client).await?;
    tx.commit().await?;
    Ok(file)
}

/// The screened parties of `client`'s current CDD file, with their role:
/// the client company first, then its UBOs, directors and signatories in the
/// order they were added. Without a file, the client company alone.
///
/// # Errors
///
/// The database's errors.
pub async fn screened_parties(db: &Db, client: PartyId) -> Result<Vec<(PartyId, ScreenedRole)>> {
    let mut parties = vec![(client, ScreenedRole::Client)];
    if let Some(file) = CddFile::current(db, client).await? {
        for row in file.parties(db).await? {
            let entry = (row.party(), row.role);
            if !parties.contains(&entry) {
                parties.push(entry);
            }
        }
    }
    Ok(parties)
}

/// Adds `party` as `role` to `file`'s screened parties; adding it twice in
/// the same role does nothing.
async fn add_party_in(
    tx: &mut Transaction,
    file: &CddFile,
    party: PartyId,
    role: ScreenedRole,
) -> Result {
    let exists = CddParty::query()
        .where_(CddParty::CDD_FILE_ID.eq(file.id))
        .where_(CddParty::PARTY_ID.eq(party.0))
        .where_(CddParty::ROLE.eq(role))
        .exists(&mut *tx)
        .await?;
    if !exists {
        let mut row = CddParty {
            cdd_file_id: file.id,
            party_id: party.0,
            role,
            ..CddParty::default()
        };
        row.insert(&mut *tx).await?;
    }
    Ok(())
}

/// Adds `party` as a UBO, director or signatory of the client to file `id`.
///
/// # Errors
///
/// A 422 once the file is approved or closed; the database's errors.
pub async fn add_party(state: &AppState, id: i64, party: PartyId, role: ScreenedRole) -> Result<CddFile> {
    let file = load(state, id).await?;
    if !matches!(file.status, CddStatus::InProgress) {
        return Err(refused(english("accounting.problems.cdd_locked", &[])));
    }
    let mut tx = state.db.begin().await?;
    add_party_in(&mut tx, &file, party, role).await?;
    tx.commit().await?;
    load(state, id).await
}

/// What the preparer records on the file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Assessment {
    /// The risk rating.
    pub risk_rating: RiskRating,
    /// The risk factors.
    pub risk_factors: String,
    /// The EDD notes.
    pub edd_notes: String,
    /// Whether the identity documents are attached.
    pub identity_documents_attached: bool,
}

impl Assessment {
    /// An assessment rated `rating`, with the identity documents attached.
    pub fn rated(rating: RiskRating) -> Self {
        Assessment {
            risk_rating: rating,
            identity_documents_attached: true,
            ..Assessment::default()
        }
    }
}

/// Records `assessment` on file `id` while it is `in_progress`.
///
/// # Errors
///
/// A 422 once submitted; the database's errors.
pub async fn assess(state: &AppState, id: i64, assessment: Assessment) -> Result<CddFile> {
    let file = load(state, id).await?;
    if file.status != CddStatus::InProgress {
        return Err(refused(english("accounting.problems.cdd_locked", &[])));
    }
    let mut tx = state.db.begin().await?;
    set_cdd_file(
        &mut tx,
        id,
        &[
            ("risk_rating", &assessment.risk_rating),
            ("risk_factors", &assessment.risk_factors),
            ("edd_notes", &assessment.edd_notes),
            (
                "identity_documents_attached",
                &assessment.identity_documents_attached,
            ),
        ],
    )
    .await?;
    tx.commit().await?;
    load(state, id).await
}

/// The checks of `submit` on the file itself, one English line each: UBOs
/// identified, identity documents attached, the risk rated.
///
/// # Errors
///
/// The database's errors.
pub async fn submit_readiness(db: &Db, file: &CddFile) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    let ubos = file
        .parties(db)
        .await?
        .iter()
        .filter(|p| p.role == ScreenedRole::Ubo)
        .count();
    if ubos == 0 {
        problems.push(english("accounting.cdd.no_ubos", &[]));
    }
    if !file.identity_documents_attached {
        problems.push(english("accounting.cdd.no_documents", &[]));
    }
    if file.risk_rating == RiskRating::Unrated {
        problems.push(english("accounting.cdd.not_rated", &[]));
    }
    Ok(problems)
}

/// Runs `transition` on `file` as `by` with `input`.
async fn run(
    state: &AppState,
    by: &AuthUser,
    file: &CddFile,
    transition: &str,
    input: &CddInput,
) -> Result<CddFile> {
    let _outcome = file
        .transition(transition)
        .by(by)
        .input(input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    load(state, file.id).await
}

/// A 422 with `problems` under the heading `heading`.
fn refusal(heading: &str, problems: &[String]) -> Error {
    let mut lines = vec![english(heading, &[])];
    lines.extend(problems.iter().map(|p| format!("- {p}")));
    refused(lines.join("\n"))
}

/// `by` (the preparer) submits file `id` for approval.
///
/// # Errors
///
/// `Forbidden` without `cdd.prepare`; a 422 listing every failing check.
pub async fn submit(state: &AppState, by: &AuthUser, id: i64) -> Result<CddFile> {
    if !by.has_permission(permissions::CDD_PREPARE) {
        return Err(Error::Forbidden);
    }
    let file = load(state, id).await?;
    let on = today();
    let mut problems = Vec::new();
    if file.status == CddStatus::InProgress {
        problems = submit_readiness(&state.db, &file).await?;
        if !problems.is_empty() {
            return Err(refusal("accounting.cdd.submit_refused", &problems));
        }
    }
    let input = CddInput {
        problems,
        on: Some(on),
        reason: None,
    };
    run(state, by, &file, SUBMIT, &input).await
}

/// `by` (the MLRO) approves file `id`: the next review date follows the
/// rating.
///
/// # Errors
///
/// `Forbidden` without `cdd.approve`; a 422 listing every failing check.
pub async fn approve(state: &AppState, by: &AuthUser, id: i64) -> Result<CddFile> {
    if !by.has_permission(permissions::CDD_APPROVE) {
        return Err(Error::Forbidden);
    }
    let file = load(state, id).await?;
    let input = CddInput {
        on: Some(today()),
        ..CddInput::default()
    };
    run(state, by, &file, APPROVE, &input).await
}

/// `by` (the MLRO) rejects file `id` with `reason` (kept for the MLRO
/// only); the client's onboarding engagements are declined.
///
/// # Errors
///
/// As the transition; a 422 without a reason.
pub async fn reject(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<CddFile> {
    let file = load(state, id).await?;
    let input = CddInput {
        on: Some(today()),
        reason: Some(reason.to_owned()),
        ..CddInput::default()
    };
    run(state, by, &file, REJECT, &input).await
}

/// `by` reopens file `id` to collect more.
///
/// # Errors
///
/// As the transition.
pub async fn reopen(state: &AppState, by: &AuthUser, id: i64) -> Result<CddFile> {
    let file = load(state, id).await?;
    run(state, by, &file, REOPEN, &CddInput::default()).await
}

/// `by` (the MLRO) exits the client of file `id` with `reason` (kept for the
/// MLRO only). Its effects (the partner's task to end the engagements) come
/// with BL-ACC-09.
///
/// # Errors
///
/// As the transition; a 422 without a reason.
pub async fn exit(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<CddFile> {
    let file = load(state, id).await?;
    let input = CddInput {
        on: Some(today()),
        reason: Some(reason.to_owned()),
        ..CddInput::default()
    };
    run(state, by, &file, EXIT, &input).await
}

/// The system flags file `id` for its periodic review (`approved` only).
///
/// # Errors
///
/// As the transition.
pub async fn flag_review(state: &AppState, id: i64, on: NaiveDate) -> Result<CddFile> {
    let file = load(state, id).await?;
    let _outcome = file
        .transition(FLAG_REVIEW)
        .by_system()
        .input(&CddInput {
            on: Some(on),
            ..CddInput::default()
        })
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    load(state, id).await
}
