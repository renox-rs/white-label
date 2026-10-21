//! What waits on the projects, for the projects list and the dashboard: IPAs
//! the client has not certified, guarantees about to expire and variations
//! the client has not answered. Every figure counts the projects the person
//! may see (`projects.view` or `projects.manage` in the project).

use renox::chrono::{Duration, NaiveDate};
use renox::prelude::*;

use crate::guarantee::{Guarantee, GuaranteeStatus};
use crate::ipa::{Ipa, IpaStatus};
use crate::permissions;
use crate::project::{Project, ProjectStatus};
use crate::variation::{Variation, VariationStatus};

/// The days after which an IPA or a variation the client has not answered
/// needs a nudge (`CON.RUL.09` asks the project manager to chase at 14).
pub const WAIT_DAYS: i64 = 14;

/// The days before the expiry of a guarantee that it is listed
/// (`CON.RUL.11` asks the bank for an extension 60 days before).
pub const EXPIRY_DAYS: i64 = 60;

/// The projects `user` may see, by code.
///
/// # Errors
///
/// A database error.
pub async fn visible_projects(db: &Db, user: &AuthUser) -> Result<Vec<Project>> {
    Ok(Project::query()
        .order_by(Project::CODE)
        .get(db)
        .await?
        .into_iter()
        .filter(|project| {
            let scope = project.scope();
            user.has_permission_in(permissions::PROJECTS_VIEW, &scope)
                || user.has_permission_in(permissions::PROJECTS_MANAGE, &scope)
        })
        .collect())
}

/// The visible projects that are in progress.
pub fn in_progress(projects: &[Project]) -> Vec<&Project> {
    projects
        .iter()
        .filter(|project| project.status == ProjectStatus::InProgress)
        .collect()
}

/// The ids of `projects`.
pub fn ids_of(projects: &[Project]) -> Vec<i64> {
    projects.iter().map(|project| project.id).collect()
}

/// The IPAs of `projects` submitted more than [`WAIT_DAYS`] days before
/// `today` that the client has not certified.
///
/// # Errors
///
/// A database error.
pub async fn ipas_waiting(db: &Db, projects: &[i64], today: NaiveDate) -> Result<Vec<Ipa>> {
    if projects.is_empty() {
        return Ok(Vec::new());
    }
    let cutoff = today - Duration::days(WAIT_DAYS);
    Ok(Ipa::query()
        .where_(Ipa::PROJECT_ID.is_in(projects.to_vec()))
        .where_(Ipa::STATUS.eq(IpaStatus::Submitted))
        .order_by(Ipa::ID)
        .get(db)
        .await?
        .into_iter()
        .filter(|ipa| ipa.submitted_on.is_some_and(|day| day < cutoff))
        .collect())
}

/// The active guarantees of `projects` that expire from `today` up to
/// [`EXPIRY_DAYS`] days later.
///
/// # Errors
///
/// A database error.
pub async fn guarantees_expiring(
    db: &Db,
    projects: &[i64],
    today: NaiveDate,
) -> Result<Vec<Guarantee>> {
    if projects.is_empty() {
        return Ok(Vec::new());
    }
    let last = today + Duration::days(EXPIRY_DAYS);
    Ok(Guarantee::query()
        .where_(Guarantee::PROJECT_ID.is_in(projects.to_vec()))
        .where_(Guarantee::STATUS.eq(GuaranteeStatus::Active))
        .order_by(Guarantee::ID)
        .get(db)
        .await?
        .into_iter()
        .filter(|guarantee| {
            guarantee
                .expires_on
                .is_some_and(|day| day >= today && day <= last)
        })
        .collect())
}

/// The variations of `projects` submitted more than [`WAIT_DAYS`] days before
/// `today` that the client has not answered.
///
/// # Errors
///
/// A database error.
pub async fn variations_waiting(
    db: &Db,
    projects: &[i64],
    today: NaiveDate,
) -> Result<Vec<Variation>> {
    if projects.is_empty() {
        return Ok(Vec::new());
    }
    let cutoff = today - Duration::days(WAIT_DAYS);
    Ok(Variation::query()
        .where_(Variation::PROJECT_ID.is_in(projects.to_vec()))
        .where_(Variation::STATUS.eq(VariationStatus::Submitted))
        .order_by(Variation::ID)
        .get(db)
        .await?
        .into_iter()
        .filter(|variation| variation.submitted_on.is_some_and(|day| day < cutoff))
        .collect())
}

/// Whether `count` of something is worth a warning badge.
pub fn count_of<T>(list: &[T], matching: impl Fn(&T) -> bool) -> usize {
    list.iter().filter(|item| matching(item)).count()
}
