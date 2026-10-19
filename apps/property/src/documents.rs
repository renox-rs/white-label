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
//!
//! Without `wl-documents` nothing is guarded and every document is valid.

use std::future::Future;
use std::sync::{Mutex, PoisonError};

use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::documents::{
    BlockEntry, BlockSubjects, DocumentFuture, guarded_transition_as, register_block_subjects,
};
use wl_kernel::parties::{PartyKind, directory};
use wl_kernel::{PartyId, Subject, documents::today};

use crate::lease::Lease;

/// The guarded transition `submit`.
pub const SUBMIT: &str = "lease.submit";
/// The guarded transition `activate`.
pub const ACTIVATE: &str = "lease.activate";
/// The guarded step of recording the Ejari or Tawtheeq registration.
pub const RECORD_EJARI: &str = "lease.record_ejari";

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
    let result = guarded_transition_as(state, transition_name, record_id, today(), actor, || async {
        match transition().await {
            Ok(done) => Ok(done),
            Err(error) => {
                let message = error.to_string();
                *failed.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
                Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message))
            }
        }
    })
    .await;
    if let Some(error) = failed.into_inner().unwrap_or_else(PoisonError::into_inner) {
        return Err(error);
    }
    result.map_err(TransitionError::Failed)
}

/// Registers the three providers.
pub(crate) fn register() {
    register_block_subjects(SUBMIT, SubmitSubjects);
    register_block_subjects(ACTIVATE, LeaseSubjects);
    register_block_subjects(RECORD_EJARI, LeaseSubjects);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guarded_transitions_are_entity_dot_transition() {
        for guarded in [SUBMIT, ACTIVATE, RECORD_EJARI] {
            let (entity, transition) = guarded.split_once('.').expect("entity.transition");
            assert_eq!(entity, "lease");
            assert!(!transition.is_empty());
        }
    }

    #[test]
    fn registering_makes_the_transitions_guardable() {
        register();
        let guarded = wl_kernel::documents::guarded_transitions();
        for name in [SUBMIT, ACTIVATE, RECORD_EJARI] {
            assert!(guarded.iter().any(|g| g == name), "{name}");
        }
    }

    #[test]
    fn a_party_kind_is_named_for_the_conditions() {
        assert_eq!(tenant_kind(PartyKind::Person), "person");
        assert_eq!(tenant_kind(PartyKind::Company), "company");
    }
}
