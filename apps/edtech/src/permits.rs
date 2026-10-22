//! An expiring or expired centre permit or course approval (10 §4.2 row *The
//! course approval or centre permit expires*, §7.2 `EDU.RUL.13`, §9, BL-EDU-07).
//!
//! On `DocumentExpiring` (the kind's look-ahead is 60 days, so from 60 days
//! ahead) and on `DocumentExpired`, for the centre permit or a course approval,
//! the rule:
//!
//! - lists the `open` and `running` batches the document covers: those of the
//!   course (an approval), or those of the campuses of the legal entity (a
//!   permit);
//! - tells the academic coordinator, the centre manager and the owner **in one
//!   message** which batches are affected, saying that no new batch opens and no
//!   certificate is issued until it is renewed ([`crate::documents`] holds the
//!   guards) and that a running batch is not stopped: the centre manager decides
//!   (`⚠ open: authority (AU-25)`);
//! - opens the renewal task for the kind's responsible role, due on the expiry
//!   date: the centre manager for the permit (of the campus, when the entity's
//!   campuses are known), the academic coordinator for the approval (of the
//!   campuses of the course's batches).
//!
//! It stops no running batch, offers no override and changes nothing else.
//!
//! Each occurrence is one **compliance-critical** rule run (`PLT.RUL.04`): the
//! key is the kind, the expiry date and the phase (`expiring`, `expired`), so a
//! later reminder of the same expiry runs nothing again. A run whose message
//! fails is retried by the daily run ([`retry`]) up to five attempts, and then the
//! owner and the company admin are alerted by the kernel. A permit renewed
//! before the *expired* phase runs is not announced.

use std::collections::BTreeSet;

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::automation::{RuleRun, RuleRunSpec, RuleRunStatus, run_rule};
use wl_kernel::calendar::BusinessCalendar;
use wl_kernel::compliance::CriticalRule;
use wl_kernel::documents::{DocumentExpired, DocumentExpiring, guard, today};
use wl_kernel::role_tasks;
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, task_creator};
use wl_kernel::{COMPANY_ADMIN_ROLE, ClientConfig, ScopeEntity, Subject};

use crate::batch::{Batch, BatchStatus};
use crate::course::{COURSE_SUBJECT_TYPE, Course};
use crate::documents::{LEGAL_ENTITY_SUBJECT_TYPE, permit_subject};
use crate::notices;
use crate::permissions;
use crate::seed::{CENTRE_PERMIT_KIND, COURSE_APPROVAL_KIND};
use crate::texts::english;

/// The rule id.
pub const RULE: &str = "EDU.RUL.13";

/// The days before the expiry the rule starts (`DOC.RUL.02` look-ahead of the
/// kinds).
pub const LOOK_AHEAD_DAYS: i64 = 60;

/// The scheduled task that tries the failed runs again.
pub const RETRY_TASK: &str = "edtech:permits";

/// Which moment of the expiry a run is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    /// Sixty days ahead or less.
    Expiring,
    /// The day after the expiry date.
    Expired,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Phase::Expiring => "expiring",
            Phase::Expired => "expired",
        }
    }

    fn parse(text: &str) -> Option<Phase> {
        match text {
            "expiring" => Some(Phase::Expiring),
            "expired" => Some(Phase::Expired),
            _ => None,
        }
    }
}

/// Whether `kind` is one of the two this rule watches.
pub fn watched(kind: &str) -> bool {
    kind == CENTRE_PERMIT_KIND || kind == COURSE_APPROVAL_KIND
}

/// The role responsible for renewing `kind` (§9).
pub fn responsible_role(kind: &str) -> &'static str {
    if kind == CENTRE_PERMIT_KIND {
        permissions::CENTRE_MANAGER_ROLE
    } else {
        permissions::COORDINATOR_ROLE
    }
}

/// The owner role of this deployment: `[roles] owner`, else the company admins.
fn owner_role(state: &AppState) -> String {
    state
        .provided::<std::sync::Arc<ClientConfig>>()
        .map(|config| config.owner_role().to_owned())
        .unwrap_or_else(|| COMPANY_ADMIN_ROLE.to_owned())
}

/// The key of the run for `kind` expiring on `expires_on` in `phase`.
fn key(kind: &str, expires_on: NaiveDate, phase: Phase) -> String {
    format!("{kind}:{expires_on}:{}", phase.as_str())
}

/// 09:00 Asia/Dubai on `day`, as an instant: the due time of a task due on a day.
fn due_morning(day: NaiveDate) -> Option<renox::db::DateTime> {
    day.and_hms_opt(9, 0, 0)
        .map(|local| BusinessCalendar::default().from_local(local))
}

/// The scope of a campus subject: its registered scope, else a scope of its own
/// type and id (as wl-scheduling does for a branch with no resolver).
async fn campus_scope(db: &Db, campus: &Subject) -> Result<Scope> {
    let registered = wl_kernel::scopes::scope_of(db, campus).await?;
    Ok(if registered.is_global() {
        Scope::new(&campus.subject_type, campus.subject_id)
    } else {
        registered
    })
}

/// The `open` and `running` batches `subject`'s document of `kind` covers.
///
/// # Errors
///
/// The database's errors.
pub async fn affected_batches(
    state: &AppState,
    subject: &Subject,
    kind: &str,
) -> Result<Vec<Batch>> {
    let live = Batch::query()
        .where_(Batch::STATUS.is_in([BatchStatus::Open, BatchStatus::Running]))
        .order_by(Batch::ID)
        .get(&state.db)
        .await?;
    let mut found = Vec::new();
    for batch in live {
        let covered = if kind == COURSE_APPROVAL_KIND {
            subject.subject_type == COURSE_SUBJECT_TYPE && batch.course_id == subject.subject_id
        } else {
            subject.subject_type == LEGAL_ENTITY_SUBJECT_TYPE
                && permit_subject(&state.db, batch.campus().as_ref()).await? == *subject
        };
        if covered {
            found.push(batch);
        }
    }
    Ok(found)
}

/// The campuses (scopes) the document concerns: those of the affected batches,
/// and, for a permit, those linked to its legal entity.
async fn scopes_of(state: &AppState, subject: &Subject, batches: &[Batch]) -> Result<Vec<Scope>> {
    let mut found: Vec<Scope> = Vec::new();
    for batch in batches {
        if let Some(campus) = batch.campus() {
            let scope = campus_scope(&state.db, &campus).await?;
            if !found.contains(&scope) {
                found.push(scope);
            }
        }
    }
    if subject.subject_type == LEGAL_ENTITY_SUBJECT_TYPE {
        for link in ScopeEntity::query()
            .where_(ScopeEntity::LEGAL_ENTITY_ID.eq(subject.subject_id))
            .order_by(ScopeEntity::ID)
            .get(&state.db)
            .await?
        {
            let scope = Scope::new(&link.scope_type, link.scope_id);
            if !found.contains(&scope) {
                found.push(scope);
            }
        }
    }
    Ok(found)
}

/// The holders of `role` in the campuses `scopes` (and everywhere they hold it),
/// once each; in no campus, the global holders.
async fn holders(state: &AppState, role: &str, scopes: &[Scope]) -> Result<Vec<i64>> {
    let mut ids = BTreeSet::new();
    if scopes.is_empty() {
        for user in users_with_role_in(&state.db, role, &Scope::global()).await? {
            ids.insert(user.id);
        }
    }
    for scope in scopes {
        for user in users_with_role_in(&state.db, role, scope).await? {
            ids.insert(user.id);
        }
    }
    Ok(ids.into_iter().collect())
}

/// The name of the document's subject: the centre's legal name, or the course.
async fn subject_name(state: &AppState, subject: &Subject) -> String {
    if subject.subject_type == COURSE_SUBJECT_TYPE
        && let Ok(Some(course)) = Course::find(&state.db, subject.subject_id).await
    {
        return course.title();
    }
    if subject.subject_type == LEGAL_ENTITY_SUBJECT_TYPE
        && let Ok(Some(entity)) = wl_kernel::LegalEntity::find(&state.db, subject.subject_id).await
    {
        return entity.name_en;
    }
    subject.to_string()
}

/// One batch of the message: `BAT-0007 · open · 2026-11-30 to 2027-03-15`.
fn batch_line(batch: &Batch) -> String {
    english(
        "edtech.permits.batch_line",
        &[
            ("batch", &batch.number()),
            ("status", &batch.status.as_str()),
            ("from", &batch.start_date),
            ("until", &batch.end_date),
        ],
    )
}

/// What one run does: the message to the three roles and the renewal task.
async fn announce(
    state: &AppState,
    subject: &Subject,
    kind: &str,
    expires_on: NaiveDate,
    phase: Phase,
) -> Result {
    // A renewal accepted before the run: the document is valid again.
    if phase == Phase::Expired
        && guard(state)
            .is_valid(&state.db, subject, kind, today())
            .await?
            .is_valid()
    {
        return Ok(());
    }
    let batches = affected_batches(state, subject, kind).await?;
    let scopes = scopes_of(state, subject, &batches).await?;
    let name = subject_name(state, subject).await;
    let what = english(
        if kind == CENTRE_PERMIT_KIND {
            "edtech.permits.kind.centre_permit"
        } else {
            "edtech.permits.kind.course_approval"
        },
        &[],
    );
    // The message: one, to the coordinator, the centre manager and the owner.
    let mut users = BTreeSet::new();
    for role in [
        permissions::COORDINATOR_ROLE.to_owned(),
        permissions::CENTRE_MANAGER_ROLE.to_owned(),
        owner_role(state),
    ] {
        users.extend(holders(state, &role, &scopes).await?);
    }
    let users: Vec<i64> = users.into_iter().collect();
    let lines = if batches.is_empty() {
        english("edtech.permits.no_batches", &[])
    } else {
        batches.iter().map(batch_line).collect::<Vec<_>>().join("\n")
    };
    let params: [(&str, &dyn std::fmt::Display); 4] = [
        ("what", &what),
        ("name", &name),
        ("date", &expires_on),
        ("batches", &lines),
    ];
    let (title_key, body_key) = match phase {
        Phase::Expiring => ("edtech.permits.expiring_title", "edtech.permits.expiring_body"),
        Phase::Expired => ("edtech.permits.expired_title", "edtech.permits.expired_body"),
    };
    notices::tell_users(
        state,
        subject.clone(),
        &name,
        &users,
        &english(title_key, &params),
        &english(body_key, &params),
    )
    .await?;
    // The renewal task, due on the expiry date, for the kind's responsible role.
    let role = responsible_role(kind);
    let creator = task_creator(state);
    let task_key = format!(
        "edtech.permit_renewal:{}:{}:{kind}:{expires_on}",
        subject.subject_type, subject.subject_id
    );
    if creator.find_open(state, &task_key).await?.is_some() {
        return Ok(());
    }
    let title = english(
        "edtech.permits.task_title",
        &[("what", &what), ("name", &name), ("date", &expires_on)],
    );
    let description = english(
        "edtech.permits.task_body",
        &[("what", &what), ("name", &name), ("date", &expires_on)],
    );
    let mut ticket = OpenTicket::new(subject.clone(), TASK_CATEGORY, &title)
        .description(&description)
        .source_key(&task_key);
    if let Some(due) = due_morning(expires_on) {
        ticket = ticket.due_at(due);
    }
    let holder = match holders(state, role, &scopes).await?.first() {
        Some(user) => Some(*user),
        None => role_tasks::assignee(&state.db, role).await?,
    };
    if let Some(user) = holder {
        ticket = ticket.assign_to(Assignee::user(user));
    }
    creator.open(state, ticket).await?;
    Ok(())
}

/// One run of the rule for `subject`'s `kind` expiring on `expires_on`.
async fn run(
    state: &AppState,
    subject: &Subject,
    kind: &str,
    expires_on: NaiveDate,
    phase: Phase,
) -> Result {
    let spec = RuleRunSpec::new(RULE, subject.clone(), key(kind, expires_on, phase))
        .compliance_critical();
    let (subject, kind) = (subject.clone(), kind.to_owned());
    run_rule(state, spec, move |state: AppState| async move {
        announce(&state, &subject, &kind, expires_on, phase).await
    })
    .await?;
    Ok(())
}

/// `DocumentExpiring` for a centre permit or a course approval: from 60 days
/// ahead, the one message and the renewal task.
///
/// # Errors
///
/// The database's errors (a failing step is retried, not returned).
pub async fn on_expiring(state: &AppState, event: &DocumentExpiring) -> Result {
    if !watched(&event.kind) || event.days_left > LOOK_AHEAD_DAYS {
        return Ok(());
    }
    run(state, &event.subject, &event.kind, event.expires_on, Phase::Expiring).await
}

/// `DocumentExpired` for a centre permit or a course approval: the same on the
/// day after the expiry date.
///
/// # Errors
///
/// The database's errors (a failing step is retried, not returned).
pub async fn on_expired(state: &AppState, event: &DocumentExpired) -> Result {
    if !watched(&event.kind) {
        return Ok(());
    }
    run(state, &event.subject, &event.kind, event.expires_on, Phase::Expired).await
}

/// Tries again the runs that failed and wait (the scheduled 02:00 run): five
/// attempts, then the kernel alerts the owner and the company admin.
///
/// # Errors
///
/// The first error of the runs.
pub async fn retry(state: &AppState) -> Result {
    let waiting = RuleRun::query()
        .where_(RuleRun::RULE.eq(RULE))
        .where_(RuleRun::STATUS.eq(RuleRunStatus::Retrying))
        .order_by(RuleRun::ID)
        .get(&state.db)
        .await?;
    let mut failure = None;
    for waiting_run in waiting {
        let parts: Vec<&str> = waiting_run.occurrence_key.splitn(3, ':').collect();
        let [kind, date, phase] = parts.as_slice() else {
            continue;
        };
        let (Ok(expires_on), Some(phase)) = (date.parse::<NaiveDate>(), Phase::parse(phase)) else {
            continue;
        };
        let subject = Subject::new(&waiting_run.subject_type, waiting_run.subject_id);
        if let Err(error) = run(state, &subject, kind, expires_on, phase).await {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

/// Marks `EDU.RUL.13` compliance-critical, owned by the owner (`PLT.RUL.04`).
pub(crate) fn register() {
    CriticalRule::new(RULE)
        .owner_role(permissions::OWNER_ROLE)
        .register();
}

/// Registers the listeners and the daily retry.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(|event: DocumentExpiring, state: AppState| async move {
        on_expiring(&state, &event).await
    });
    app.listen(|event: DocumentExpired, state: AppState| async move {
        on_expired(&state, &event).await
    });
    app.schedule()
        .daily_at("02:00", RETRY_TASK, |state: AppState| async move {
            retry(&state).await
        })
        .timezone("Asia/Dubai");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_phases_are_stored_in_the_key_and_read_back() {
        let day = NaiveDate::from_ymd_opt(2027, 1, 31).unwrap();
        assert_eq!(
            key(COURSE_APPROVAL_KIND, day, Phase::Expiring),
            "course_approval:2027-01-31:expiring"
        );
        assert_eq!(Phase::parse("expired"), Some(Phase::Expired));
        assert_eq!(Phase::parse("later"), None);
    }

    #[test]
    fn the_kinds_have_their_responsible_roles() {
        assert!(watched(CENTRE_PERMIT_KIND) && watched(COURSE_APPROVAL_KIND));
        assert!(!watched("police_clearance"));
        assert_eq!(
            responsible_role(CENTRE_PERMIT_KIND),
            permissions::CENTRE_MANAGER_ROLE
        );
        assert_eq!(
            responsible_role(COURSE_APPROVAL_KIND),
            permissions::COORDINATOR_ROLE
        );
    }
}
