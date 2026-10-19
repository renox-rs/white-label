//! The approvals waiting for a person (UX §2.1 "Needs your action", the
//! *Approvals* source): the pending requests whose current step the person
//! may decide, and a registry through which a module names the page of its
//! subject (the kernel does not know the modules' routes).

use std::collections::BTreeMap;
use std::sync::{PoisonError, RwLock};

use renox::prelude::*;
use serde::Serialize;

use super::notify;
use super::request::{ApprovalRequest, ApprovalStatus};
use crate::texts;

/// A module's page for a subject: its id to the URL.
pub type SubjectLink = fn(i64) -> String;

static LINKS: RwLock<BTreeMap<String, SubjectLink>> = RwLock::new(BTreeMap::new());

/// Registers the page of a module's subject type, e.g.
/// `register_subject_link("documents.block_override", |id| format!("/documents/overrides/{id}"))`.
/// A second registration of the same type replaces the first.
pub fn register_subject_link(subject_type: &str, link: SubjectLink) {
    LINKS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(subject_type.to_owned(), link);
}

/// The page of the subject `subject_type:id`, when its module registered one.
pub fn subject_url(subject_type: &str, id: i64) -> Option<String> {
    let link = *LINKS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(subject_type)?;
    Some(link(id))
}

/// The pending requests whose current step `user` may decide, oldest first
/// (the step's approvers hold its role or permission in the subject's scope;
/// with SoD on, the requester is not one).
pub async fn awaiting(db: &Db, user: &User) -> Result<Vec<ApprovalRequest>> {
    let pending = ApprovalRequest::query()
        .where_(ApprovalRequest::STATUS.eq(ApprovalStatus::Pending))
        .order_by(ApprovalRequest::ID)
        .get(db)
        .await?;
    let mut mine = Vec::new();
    for request in pending {
        let approvers = notify::step_approvers(db, &request).await?;
        if approvers.iter().any(|approver| approver.id == user.id) {
            mine.push(request);
        }
    }
    Ok(mine)
}

/// One approval waiting for the user, as the dashboard lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AwaitingItem {
    /// The pending request.
    pub request_id: i64,
    /// What to decide, in the viewer's language.
    pub title: String,
    /// The record the request is about: its type.
    pub subject_type: String,
    /// The record's id.
    pub subject_id: i64,
    /// The subject's page, when its module registered one.
    pub url: Option<String>,
}

/// The dashboard's approvals for `user` ([`awaiting`]), with the subject's
/// page from the registry.
pub async fn awaiting_items(db: &Db, user: &User, lang: &Lang) -> Result<Vec<AwaitingItem>> {
    Ok(awaiting(db, user)
        .await?
        .into_iter()
        .map(|request| AwaitingItem {
            request_id: request.id,
            title: texts::t_with(
                lang,
                "wl_kernel.approvals.awaiting.item",
                &[("action", &request.action)],
            ),
            url: subject_url(&request.subject_type, request.subject_id),
            subject_type: request.subject_type,
            subject_id: request.subject_id,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registered_link_names_the_subjects_page() {
        register_subject_link("test.awaiting_thing", |id| format!("/things/{id}"));
        assert_eq!(
            subject_url("test.awaiting_thing", 7).as_deref(),
            Some("/things/7")
        );
        assert_eq!(subject_url("test.awaiting_unknown", 7), None);
    }
}
