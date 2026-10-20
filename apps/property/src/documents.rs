//! The document guard hooks of the lease (`PRP.LSE.02`, 01 Property §7.1
//! `submit` and `activate`, §7.2 *Document kinds*).
//!
//! Which documents a transition needs is not decided here: it is the
//! `blocks` of the document kinds (wl-documents §5.6), which 01 seeds
//! (BL-PRP-27): the tenant's identity or company documents on `submit`, the
//! signed tenancy contract on `activate`, the Ejari certificate when the
//! registration is recorded. This module registers the three
//! [`BlockSubjects`] providers that map a lease to the subjects whose
//! documents must be valid, and [`guarded`] runs a lease transition through
//! the guard, as the other verticals do.
//!
//! | Guarded | Record | Subject of the document |
//! |---|---|---|
//! | [`SUBMIT`] `lease.submit` | the lease | its tenant party (entries may be conditioned on `tenant_kind`: `person` or `company`) |
//! | [`ACTIVATE`] `lease.activate` | the lease | the lease version itself (the signed contract) |
//! | [`RECORD_EJARI`] `lease.record_ejari` | the lease | the lease version itself (the Ejari or Tawtheeq certificate) |
//! | [`AGREEMENT_ACTIVATE`] `agreement.activate` | the management agreement | the agreement itself (the signed agreement) |
//!
//! Without `wl-documents` nothing is guarded and every document is valid.

use std::future::Future;
use std::sync::{Mutex, PoisonError};

use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::documents::{
    BlockEntry, BlockRefusal, BlockSubjects, DocumentFuture, guarded_transition_as,
    register_block_subjects,
};
use wl_kernel::parties::{PartyKind, directory};
use wl_kernel::{PartyId, Subject, documents::today};

use crate::agreement::ManagementAgreement;
use crate::building::{BUILDING_SUBJECT_TYPE, Building};
use crate::lease::Lease;
use crate::texts;

/// The guarded transition `submit`.
pub const SUBMIT: &str = "lease.submit";
/// The guarded transition `activate`.
pub const ACTIVATE: &str = "lease.activate";
/// The guarded step of recording the Ejari or Tawtheeq registration.
pub const RECORD_EJARI: &str = "lease.record_ejari";

/// The guarded transition `activate` of a management agreement (the signed
/// agreement, BL-PRP-22).
pub const AGREEMENT_ACTIVATE: &str = "agreement.activate";

/// The field the entries' conditions read on `lease.submit`: whether the
/// tenant is a `person` or a `company`.
pub const CONDITION_FIELD: &str = "tenant_kind";

/// The text of a party kind as the entries' conditions name it.
pub fn tenant_kind(kind: PartyKind) -> &'static str {
    match kind {
        PartyKind::Person => "person",
        PartyKind::Company => "company",
    }
}

/// `lease.submit`: the tenant, when the entry's condition accepts its kind.
struct SubmitSubjects;

impl BlockSubjects for SubmitSubjects {
    fn subjects<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            let Some(lease) = Lease::find(&state.db, record_id).await? else {
                return Ok(Vec::new());
            };
            let tenant = lease.tenant();
            let kind = directory(state)
                .find(&state.db, tenant)
                .await?
                .map(|party| tenant_kind(party.kind));
            if !entry.applies_to(kind) {
                return Ok(Vec::new());
            }
            Ok(vec![tenant.subject()])
        })
    }

    fn label<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
    ) -> DocumentFuture<'a, Option<String>> {
        Box::pin(async move {
            if subject.subject_type != wl_kernel::parties::PARTY_SUBJECT_TYPE {
                return Ok(None);
            }
            directory(state)
                .display_name(&state.db, PartyId(subject.subject_id), "en")
                .await
        })
    }

    fn scope<'a>(
        &'a self,
        state: &'a AppState,
        _entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Option<renox::auth::permissions::Scope>> {
        Box::pin(async move {
            Ok(Lease::find(&state.db, record_id)
                .await?
                .map(|lease| lease.scope()))
        })
    }
}

/// `lease.activate` and `lease.record_ejari`: the lease version itself.
struct LeaseSubjects;

impl BlockSubjects for LeaseSubjects {
    fn subjects<'a>(
        &'a self,
        state: &'a AppState,
        _entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            Ok(Lease::find(&state.db, record_id)
                .await?
                .map(|lease| vec![lease.subject()])
                .unwrap_or_default())
        })
    }

    fn scope<'a>(
        &'a self,
        state: &'a AppState,
        _entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Option<renox::auth::permissions::Scope>> {
        Box::pin(async move {
            Ok(Lease::find(&state.db, record_id)
                .await?
                .map(|lease| lease.scope()))
        })
    }
}

/// `agreement.activate`: the agreement itself (the signed contract).
struct AgreementSubjects;

impl BlockSubjects for AgreementSubjects {
    fn subjects<'a>(
        &'a self,
        state: &'a AppState,
        _entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            Ok(ManagementAgreement::find(&state.db, record_id)
                .await?
                .map(|agreement| vec![agreement.subject()])
                .unwrap_or_default())
        })
    }
}

/// `building.certify`: the building itself (its compliance certificates).
/// Nothing runs this transition: the entry exists so that `DOC.RUL.03` takes
/// an expiring certificate up, and the entry's mode is `flag`, never `block`
/// (BL-PRP-27; leasing is not blocked by a certificate).
struct BuildingSubjects;

impl BlockSubjects for BuildingSubjects {
    fn subjects<'a>(
        &'a self,
        state: &'a AppState,
        _entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            Ok(Building::find(&state.db, record_id)
                .await?
                .map(|building| vec![Subject::new(BUILDING_SUBJECT_TYPE, building.id)])
                .unwrap_or_default())
        })
    }
}

/// Runs `transition` as the guarded transition `transition_name` of lease
/// `record_id`, after checking the documents on the business date. A refusal
/// answers 422 with one line per failing document and `transition` does not
/// run; a transition that fails is returned as it is. A flag-mode failure is
/// recorded on the lease after the transition.
///
/// # Errors
///
/// `Failed` (422) with the refusal, or the transition's own error.
pub(crate) async fn guarded<T, F, Fut>(
    state: &AppState,
    transition_name: &str,
    record_id: i64,
    actor: Option<i64>,
    transition: F,
) -> std::result::Result<T, TransitionError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = std::result::Result<T, TransitionError>>,
{
    let failed: Mutex<Option<TransitionError>> = Mutex::new(None);
    let result = guarded_transition_as(
        state,
        transition_name,
        record_id,
        today(),
        actor,
        || async {
            match transition().await {
                Ok(done) => Ok(done),
                Err(error) => {
                    let message = error.to_string();
                    *failed.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
                    Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message))
                }
            }
        },
    )
    .await;
    if let Some(error) = failed.into_inner().unwrap_or_else(PoisonError::into_inner) {
        return Err(error);
    }
    // The transition did not run: the guard refused. Say it in words (what
    // the tenant needs on file); the guard's own line, with the renewing role
    // and the override mode, goes to the log.
    if let Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, machine)) = &result
        && let Ok(Err(refusal)) = wl_kernel::documents::guard(state)
            .check_as(state, transition_name, record_id, today(), actor)
            .await
    {
        eprintln!("documents: {transition_name} on lease {record_id} refused: {machine}");
        return Err(TransitionError::Failed(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            human_refusal(&refusal),
        )));
    }
    result.map_err(TransitionError::Failed)
}

/// Who a document subject is, for the sentence of a refusal.
fn who(subject: &Subject) -> &'static str {
    match subject.subject_type.as_str() {
        wl_kernel::parties::PARTY_SUBJECT_TYPE => "tenant",
        crate::lease::LEASE_SUBJECT_TYPE => "lease",
        crate::agreement::AGREEMENT_SUBJECT_TYPE => "agreement",
        BUILDING_SUBJECT_TYPE => "building",
        _ => "record",
    }
}

/// The name of a kind in a sentence: lower case, unless it is a proper name
/// or an acronym (`Emirates ID`, `TRN certificate`).
fn in_sentence(name: &str) -> String {
    if name.chars().skip(1).any(char::is_uppercase) {
        return name.to_owned();
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `a, b and c`.
fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// A refusal of a guarded lease transition as a person reads it: what the
/// tenant (or the lease) needs on file, and what is wrong with a document
/// that is there ("Residence visa expired on 2026-11-30"). The guard's own
/// line, with the role that renews and the override mode, is for the logs.
pub fn human_refusal(refusal: &BlockRefusal) -> String {
    let mut subjects: Vec<&Subject> = Vec::new();
    for failure in &refusal.failures {
        if !subjects.contains(&&failure.subject) {
            subjects.push(&failure.subject);
        }
    }
    let mut sentences: Vec<String> = Vec::new();
    for subject in subjects {
        let mut names: Vec<String> = Vec::new();
        let mut problems: Vec<String> = Vec::new();
        for failure in refusal.failures.iter().filter(|f| &f.subject == subject) {
            // "Residence visa expired on 2026-11-30; renew: ...": the part
            // before the first semicolon says what is wrong.
            let problem = failure.message.split("; ").next().unwrap_or_default();
            let name = [" missing", " rejected", " not accepted yet"]
                .iter()
                .find_map(|tail| problem.strip_suffix(tail))
                .or_else(|| problem.split_once(" expired on ").map(|(name, _)| name))
                .unwrap_or(problem);
            names.push(in_sentence(name));
            if problem != format!("{name} missing") {
                problems.push(problem.to_owned());
            }
        }
        sentences.push(texts::english(
            "property.documents.needs",
            &[
                (
                    "who",
                    &texts::english(&format!("property.documents.who_{}", who(subject)), &[]),
                ),
                ("what", &list(&names)),
            ],
        ));
        sentences.extend(problems.into_iter().map(|p| format!("{p}.")));
    }
    sentences.push(texts::english("property.documents.then", &[]));
    sentences.join(" ")
}

/// What `guarded` (a lease transition) still waits for on lease `record_id`:
/// the refusal the guard would give now, or `None` when the documents are in
/// order. Nothing is claimed or recorded.
pub(crate) async fn missing(
    state: &AppState,
    guarded: &str,
    record_id: i64,
) -> Result<Option<BlockRefusal>> {
    Ok(wl_kernel::documents::guard(state)
        .check_as(state, guarded, record_id, today(), None)
        .await?
        .err())
}

/// Registers the providers.
pub(crate) fn register() {
    register_block_subjects(AGREEMENT_ACTIVATE, AgreementSubjects);
    register_block_subjects(SUBMIT, SubmitSubjects);
    register_block_subjects(ACTIVATE, LeaseSubjects);
    register_block_subjects(RECORD_EJARI, LeaseSubjects);
    register_block_subjects(crate::seed::CERTIFY, BuildingSubjects);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guarded_transitions_are_entity_dot_transition() {
        for guarded in [SUBMIT, ACTIVATE, RECORD_EJARI, AGREEMENT_ACTIVATE] {
            let (entity, transition) = guarded.split_once('.').expect("entity.transition");
            assert!(entity == "lease" || entity == "agreement");
            assert!(!transition.is_empty());
        }
    }

    fn failure(subject: Subject, message: &str) -> wl_kernel::documents::BlockFailure {
        use wl_kernel::documents::{BlockMode, InvalidReason, OverrideMode, Validity};
        wl_kernel::documents::BlockFailure {
            entry: BlockEntry {
                kind: "k".into(),
                guarded: SUBMIT.into(),
                mode: BlockMode::Block,
                override_mode: OverrideMode::WithApproval,
                condition: None,
                locked: false,
            },
            subject,
            validity: Validity::Invalid {
                reason: InvalidReason::Missing,
                document: None,
                expired_on: None,
                pending: None,
            },
            message: message.into(),
        }
    }

    #[test]
    fn a_refusal_is_said_in_words_and_keeps_the_guard_line_out_of_them() {
        let tenant = Subject::new(wl_kernel::parties::PARTY_SUBJECT_TYPE, 12);
        let refusal = BlockRefusal {
            guarded: SUBMIT.into(),
            record_id: 5,
            failures: vec![
                failure(
                    tenant.clone(),
                    "Emirates ID missing; renew: property_manager; override: with approval",
                ),
                failure(
                    tenant.clone(),
                    "Passport not accepted yet; override: with approval",
                ),
                failure(
                    tenant,
                    "Residence visa expired on 2026-11-30; renew: PRO; override: not allowed",
                ),
            ],
        };
        let said = human_refusal(&refusal);
        assert!(
            said.starts_with(
                "The tenant needs a valid Emirates ID, passport and residence visa on file. \
                 Passport not accepted yet. Residence visa expired on 2026-11-30."
            ),
            "{said}"
        );
        assert!(
            !said.contains("override") && !said.contains("renew:"),
            "{said}"
        );

        let lease = BlockRefusal {
            guarded: ACTIVATE.into(),
            record_id: 5,
            failures: vec![failure(
                Subject::new(crate::lease::LEASE_SUBJECT_TYPE, 5),
                "Signed tenancy contract missing; override: not allowed",
            )],
        };
        assert!(
            human_refusal(&lease)
                .starts_with("The lease needs a valid signed tenancy contract on file.")
        );
    }

    #[test]
    fn registering_makes_the_transitions_guardable() {
        register();
        let guarded = wl_kernel::documents::guarded_transitions();
        for name in [
            SUBMIT,
            ACTIVATE,
            RECORD_EJARI,
            AGREEMENT_ACTIVATE,
            crate::seed::CERTIFY,
        ] {
            assert!(guarded.iter().any(|g| g == name), "{name}");
        }
    }

    #[test]
    fn a_party_kind_is_named_for_the_conditions() {
        assert_eq!(tenant_kind(PartyKind::Person), "person");
        assert_eq!(tenant_kind(PartyKind::Company), "company");
    }
}
