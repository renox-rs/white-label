//! The documents a deployment depends on (09 Manpower §4.5 step 3, §9
//! *Documents that block work*, BL-MPW-09, `DOC.RUL.02`, `MPW.ACL.07`).
//!
//! The document kinds say which deployment transitions they guard (their
//! `blocks`: `deployment.activate`, `deployment.resume`, `deployment.extend`);
//! this module tells wl-documents what that means for a deployment, through the
//! kernel's providers, and shows what the nightly look-ahead found. The guard
//! itself (`MPW.ACL.07`) is [`crate::guard`].
//!
//! - **`BlockSubjects`** (all three transitions): a deployment → the employee
//!   (`hr.employee:W`), whose visa, work permit, Emirates ID, health insurance
//!   and site licences (SIRA) must be valid. A kind's entry may carry a
//!   condition on the deployment's `role` ([`ROLE_FIELD`]): the SIRA licence
//!   applies to the security roles only, so for a cleaner the provider returns
//!   no subject and the guard answers *yes*. The *site scope* of the record is
//!   the deployment's site ([`BlockSubjects::scope`]): the licence is judged
//!   for, and an override is approved in, that site.
//! - **`FutureWork`** (`deployment.activate`): what the document will stop
//!   after its expiry date. `planned_from(employee, from)` lists the
//!   deployments of the employee `planned` with a start on or after `from`
//!   (the day after the expiry date) and those `active` or `suspended` with an
//!   end date on or after it, or none; `open_for` lists the deployments on
//!   site now, for an expired document. The records carry the client company
//!   as their scope, so wl-documents sends the holders of the kind's
//!   responsible role in that scope (the account manager, when the kind lists
//!   the role in `also_notify`) one message per document per night with the
//!   list, not one per deployment.
//!
//!   Decision: one provider, on `deployment.activate`. A kind that guards
//!   `resume` and `extend` as well would list the same deployment under each
//!   transition; the look-ahead is about the deployments, not the transition.
//! - **The flag.** `DocumentExpiring` carries the affected deployments; each
//!   gets a [`DeploymentDocumentFlag`] (once per deployment and document), shown
//!   in the deployments list as *document expiring <kind> <date>*. When a
//!   renewed document of the same kind is accepted for the employee
//!   (`DocumentAccepted` with a later expiry date, or none) the flags of that
//!   kind are cleared. A renewal rejected is not accepted, so the flag stays.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::documents::{
    BlockEntry, BlockSubjects, DocumentAccepted, DocumentExpiring, DocumentFuture, FutureWork,
    PlannedRecord, register_block_subjects, register_future_work, today,
};
use wl_kernel::parties::directory;
use wl_kernel::workforce::{EMPLOYEE_SUBJECT_TYPE, EmployeeId, workforce};

use crate::deployment::{Deployment, DeploymentStatus, SUBJECT_TYPE};
use crate::site::Site;
use crate::texts::english;

/// The guarded transition `activate` of a deployment (`MPW.ACL.07`).
pub const ACTIVATE_GUARDED: &str = "deployment.activate";

/// The guarded transition `resume` of a deployment.
pub const RESUME_GUARDED: &str = "deployment.resume";

/// The guarded transition `extend` of a deployment.
pub const EXTEND_GUARDED: &str = "deployment.extend";

/// The field the entries' conditions read: the deployment's role.
pub const ROLE_FIELD: &str = "role";

/// A document that will expire during a deployment.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "manpower_deployment_document_flags",
    unique(deployment_id, document_id),
    index(deployment_id)
)]
pub struct DeploymentDocumentFlag {
    /// The primary key.
    pub id: i64,
    /// The deployment.
    pub deployment_id: i64,
    /// The expiring document.
    pub document_id: i64,
    /// Its kind's key.
    pub kind: String,
    /// The day it expires; it is valid through it.
    pub expires_on: NaiveDate,
    /// The day a renewed document was accepted.
    pub cleared_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl DeploymentDocumentFlag {
    /// The open flags of deployment `deployment_id`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn open_for(
        db: impl renox::db::Executor<'_>,
        deployment_id: i64,
    ) -> Result<Vec<DeploymentDocumentFlag>> {
        DeploymentDocumentFlag::query()
            .where_(DeploymentDocumentFlag::DEPLOYMENT_ID.eq(deployment_id))
            .where_(DeploymentDocumentFlag::CLEARED_ON.is_null())
            .order_by(DeploymentDocumentFlag::ID)
            .get(db)
            .await
    }

    /// The text the deployments list shows: *document expiring <kind> <date>*.
    pub fn text(&self) -> String {
        let kind = self.kind.replace('_', " ");
        english(
            "manpower.deployment.document_expiring",
            &[("kind", &kind), ("date", &self.expires_on)],
        )
    }
}

/// A deployment's `BlockSubjects`: the employee, when the entry's condition on
/// the role matches.
struct DeploymentSubjects;

impl BlockSubjects for DeploymentSubjects {
    fn subjects<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            let Some(deployment) = Deployment::find(&state.db, record_id).await? else {
                return Ok(Vec::new());
            };
            if !entry.applies_to(Some(deployment.role.as_str())) {
                return Ok(Vec::new());
            }
            Ok(vec![Subject::new(
                EMPLOYEE_SUBJECT_TYPE,
                deployment.employee_id,
            )])
        })
    }

    fn label<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
    ) -> DocumentFuture<'a, Option<String>> {
        Box::pin(async move {
            if subject.subject_type != EMPLOYEE_SUBJECT_TYPE {
                return Ok(None);
            }
            let profile = workforce(state)
                .staff(state, EmployeeId(subject.subject_id))
                .await?;
            match profile {
                Some(profile) => {
                    directory(state)
                        .display_name(&state.db, profile.party, "en")
                        .await
                }
                None => Ok(None),
            }
        })
    }

    fn scope<'a>(
        &'a self,
        state: &'a AppState,
        _entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Option<renox::auth::permissions::Scope>> {
        Box::pin(async move {
            Ok(Deployment::find(&state.db, record_id)
                .await?
                .map(|deployment| deployment.site_scope()))
        })
    }
}

/// The deployments of the employee `subject` that match `entry`'s condition
/// on the role.
async fn deployments_of(
    state: &AppState,
    entry: &BlockEntry,
    subject: &Subject,
) -> Result<Vec<Deployment>> {
    if subject.subject_type != EMPLOYEE_SUBJECT_TYPE {
        return Ok(Vec::new());
    }
    Ok(Deployment::query()
        .where_(Deployment::EMPLOYEE_ID.eq(subject.subject_id))
        .order_by(Deployment::ID)
        .get(&state.db)
        .await?
        .into_iter()
        .filter(|deployment| entry.applies_to(Some(deployment.role.as_str())))
        .collect())
}

/// The planned record of `deployment`, dated `date`.
async fn planned(
    state: &AppState,
    deployment: &Deployment,
    date: NaiveDate,
) -> Result<PlannedRecord> {
    let site = Site::find(&state.db, deployment.site_id)
        .await?
        .map(|site| site.name)
        .unwrap_or_default();
    let ends = deployment
        .end_date
        .map_or_else(|| "-".to_owned(), |end| end.to_string());
    Ok(PlannedRecord {
        record: deployment.subject(),
        date,
        label: english(
            "manpower.deployment.document_label",
            &[
                ("role", &deployment.role),
                ("site", &site),
                ("from", &deployment.start_date),
                ("until", &ends),
            ],
        ),
        scope: Some(deployment.scope()),
    })
}

/// What a document will stop after its expiry date: the deployments planned to
/// start after it, and those on site that run past it.
struct DeploymentsAffected;

impl FutureWork for DeploymentsAffected {
    fn planned_from<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        subject: &'a Subject,
        from: NaiveDate,
    ) -> DocumentFuture<'a, Vec<PlannedRecord>> {
        Box::pin(async move {
            let mut found = Vec::new();
            for deployment in deployments_of(state, entry, subject).await? {
                let date = match deployment.status {
                    DeploymentStatus::Planned if deployment.start_date >= from => {
                        deployment.start_date
                    }
                    DeploymentStatus::Active | DeploymentStatus::Suspended
                        if deployment.end_date.is_none_or(|end| end >= from) =>
                    {
                        deployment.start_date.max(from)
                    }
                    _ => continue,
                };
                found.push(planned(state, &deployment, date).await?);
            }
            found.sort_by_key(|p| (p.date, p.record.subject_id));
            Ok(found)
        })
    }

    fn open_for<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        subject: &'a Subject,
    ) -> DocumentFuture<'a, Vec<PlannedRecord>> {
        Box::pin(async move {
            let mut found = Vec::new();
            for deployment in deployments_of(state, entry, subject).await? {
                if deployment.is_on_site() {
                    found.push(planned(state, &deployment, today()).await?);
                }
            }
            found.sort_by_key(|p| p.record.subject_id);
            Ok(found)
        })
    }
}

/// Registers the providers with the kernel.
pub(crate) fn register() {
    for guarded in [ACTIVATE_GUARDED, RESUME_GUARDED, EXTEND_GUARDED] {
        register_block_subjects(guarded, DeploymentSubjects);
    }
    register_future_work(ACTIVATE_GUARDED, DeploymentsAffected);
}

/// Flags the deployments `event` lists as affected, once per deployment and
/// document.
async fn flag_affected(state: &AppState, event: &DocumentExpiring) -> Result {
    for group in &event.affected {
        for record in &group.records {
            if record.record.subject_type != SUBJECT_TYPE {
                continue;
            }
            let deployment_id = record.record.subject_id;
            let known = DeploymentDocumentFlag::query()
                .where_(DeploymentDocumentFlag::DEPLOYMENT_ID.eq(deployment_id))
                .where_(DeploymentDocumentFlag::DOCUMENT_ID.eq(event.document_id.0))
                .first(&state.db)
                .await?
                .is_some();
            if known {
                continue;
            }
            DeploymentDocumentFlag::create(
                &state.db,
                DeploymentDocumentFlag {
                    deployment_id,
                    document_id: event.document_id.0,
                    kind: event.kind.clone(),
                    expires_on: event.expires_on,
                    ..DeploymentDocumentFlag::default()
                },
            )
            .await?;
        }
    }
    Ok(())
}

/// Clears the flags a renewed document answers: the employee's deployments'
/// open flags of its kind whose document expires before the new one does.
async fn clear_renewed(state: &AppState, event: &DocumentAccepted) -> Result {
    if event.subject.subject_type != EMPLOYEE_SUBJECT_TYPE {
        return Ok(());
    }
    let deployments = Deployment::query()
        .where_(Deployment::EMPLOYEE_ID.eq(event.subject.subject_id))
        .get(&state.db)
        .await?;
    for deployment in deployments {
        for mut flag in DeploymentDocumentFlag::open_for(&state.db, deployment.id).await? {
            let renewed = flag.kind == event.kind
                && event
                    .expires_on
                    .is_none_or(|expires| expires > flag.expires_on);
            if renewed {
                flag.cleared_on = Some(today());
                flag.save_only(&state.db, &["cleared_on"]).await?;
            }
        }
    }
    Ok(())
}

/// Listens for the nightly look-ahead and for the acceptance of a renewal.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(|event: DocumentExpiring, state: AppState| async move {
        flag_affected(&state, &event).await
    });
    app.listen(|event: DocumentAccepted, state: AppState| async move {
        clear_renewed(&state, &event).await
    });
}
