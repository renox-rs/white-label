//! The vendor document guard (`WRK.ACL.11`, wl-ticketing §4.1 `resolve`, §4.6
//! `activate`, issue #151): a ticket or planned visit in a category guarded
//! by a vendor document kind cannot be resolved while that document of the
//! assigned vendor is expired or missing, and a contract covering a guarded
//! category cannot be activated unless the vendor's document is valid on its
//! start date.
//!
//! Which categories are guarded is not decided here: it is the document
//! kind's `blocks` (wl-documents §5.6; 01: the Civil Defence approval blocks
//! `ticket.resolve` and `service_contract.activate`, with the condition
//! `category` in the fire and life safety categories, and is a legal block
//! with the override mode `never`, locked). This module registers the two
//! [`BlockSubjects`] providers that map a record to the vendor whose document
//! must be valid, and returns none when the record does not match the
//! entry's condition, so the guard answers *yes* for it:
//!
//! | Guarded | Record | Subject of the document |
//! |---|---|---|
//! | `ticket.resolve` | the ticket | its vendor party, when the assignee is one and its category matches the condition |
//! | `service_contract.activate` | the contract | its vendor party, when a covered category matches the condition |
//!
//! A staff assignee is never guarded. [`run`] is the call both transitions go
//! through: [`guarded_transition_as`](wl_kernel::documents::guarded_transition_as)
//! with the business date, so the guard `document_valid` and an approved
//! override (offered only for a kind whose mode is `with_approval`, never in
//! 01 until the lawyer's answer L-19 says otherwise) work as everywhere.
//! Without `wl-documents` it just runs the transition.

use std::future::Future;
use std::sync::{Mutex, PoisonError};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::documents::{
    BlockEntry, BlockSubjects, DocumentFuture, guarded_transition_as, register_block_subjects,
};
use wl_kernel::parties::directory;
use wl_kernel::{PartyId, Subject};

use crate::config::Category;
use crate::contract::ServiceContract;
use crate::ticket::Ticket;

/// The guarded transition `resolve` of a ticket.
pub const RESOLVE: &str = "ticket.resolve";

/// The guarded transition `activate` of a service contract.
pub const ACTIVATE: &str = "service_contract.activate";

/// The field the entries' conditions read: the category's key.
pub const CONDITION_FIELD: &str = "category";

/// The name of a vendor party in a refusal.
async fn vendor_label(state: &AppState, subject: &Subject) -> Result<Option<String>> {
    if subject.subject_type != wl_kernel::parties::PARTY_SUBJECT_TYPE {
        return Ok(None);
    }
    directory(state)
        .display_name(&state.db, PartyId(subject.subject_id), "en")
        .await
}

/// `ticket.resolve`: the assigned vendor, for a guarded category.
struct ResolveSubjects;

impl BlockSubjects for ResolveSubjects {
    fn subjects<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            let Some(ticket) = Ticket::find(&state.db, record_id).await? else {
                return Ok(Vec::new());
            };
            let Some(vendor) = ticket.assignee_party_id else {
                return Ok(Vec::new());
            };
            let category = Category::find(&state.db, ticket.category_id).await?;
            if !entry.applies_to(category.as_ref().map(|c| c.key.as_str())) {
                return Ok(Vec::new());
            }
            Ok(vec![PartyId(vendor).subject()])
        })
    }

    fn label<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
    ) -> DocumentFuture<'a, Option<String>> {
        Box::pin(vendor_label(state, subject))
    }

    fn scope<'a>(
        &'a self,
        state: &'a AppState,
        _entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Option<renox::auth::permissions::Scope>> {
        Box::pin(async move {
            let ticket = Ticket::find(&state.db, record_id).await?;
            match ticket {
                Some(ticket) => Ok(Some(
                    wl_kernel::scopes::scope_of(&state.db, &ticket.subject()).await?,
                )),
                None => Ok(None),
            }
        })
    }
}

/// `service_contract.activate`: the vendor, when a covered category is
/// guarded.
struct ActivateSubjects;

impl BlockSubjects for ActivateSubjects {
    fn subjects<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            let Some(contract) = ServiceContract::find(&state.db, record_id).await? else {
                return Ok(Vec::new());
            };
            let mut guarded = entry.condition.is_none();
            for row in contract.coverage(&state.db).await? {
                if guarded {
                    break;
                }
                let category = Category::find(&state.db, row.category_id).await?;
                guarded = entry.applies_to(category.as_ref().map(|c| c.key.as_str()));
            }
            if guarded {
                Ok(vec![contract.vendor().subject()])
            } else {
                Ok(Vec::new())
            }
        })
    }

    fn label<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
    ) -> DocumentFuture<'a, Option<String>> {
        Box::pin(vendor_label(state, subject))
    }
}

/// Runs `transition` as the guarded transition `guarded` of record
/// `record_id`, checking the vendor's documents on the business date `on`
/// first. A refusal answers 422 with one line per failing document and the
/// transition does not run; a transition that fails is returned as it is.
///
/// # Errors
///
/// `Failed` (422) with the refusal, or the transition's own error.
pub(crate) async fn run<F, Fut>(
    state: &AppState,
    guarded: &str,
    record_id: i64,
    on: NaiveDate,
    actor: Option<i64>,
    transition: F,
) -> std::result::Result<(), TransitionError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = std::result::Result<(), TransitionError>>,
{
    let failed: Mutex<Option<TransitionError>> = Mutex::new(None);
    let result = guarded_transition_as(state, guarded, record_id, on, actor, || async {
        match transition().await {
            Ok(()) => Ok(()),
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

/// Registers the two providers.
pub(crate) fn register() {
    register_block_subjects(RESOLVE, ResolveSubjects);
    register_block_subjects(ACTIVATE, ActivateSubjects);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guarded_transitions_are_entity_dot_transition() {
        for guarded in [RESOLVE, ACTIVATE] {
            let (entity, transition) = guarded.split_once('.').expect("entity.transition");
            assert!(!entity.is_empty() && !transition.is_empty());
        }
        assert_eq!(RESOLVE, "ticket.resolve");
        assert_eq!(ACTIVATE, "service_contract.activate");
    }

    #[test]
    fn registering_makes_both_transitions_guardable() {
        register();
        let guarded = wl_kernel::documents::guarded_transitions();
        assert!(guarded.iter().any(|g| g == RESOLVE));
        assert!(guarded.iter().any(|g| g == ACTIVATE));
    }
}
