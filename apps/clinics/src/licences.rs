//! `CLN.RUL.01`: a practitioner's licence or branch privileges expired (05
//! §4.5, §7.2; BL-CLN-12).
//!
//! The rule scans nothing. The block is wl-hr's (`HRM.RUL.10`): from the day
//! after the expiry `qualification_valid` answers *no*, and wl-scheduling,
//! which asks it for the `required_qualifications` of every service type
//! ([`crate::seed::REQUIRED_QUALIFICATIONS`]), refuses a new booking of that
//! practitioner at that branch with no override, refuses their check-in and
//! start, and puts the bookings already made in conflict (`SCH.RUL.05`).
//! What 05 adds is the telling: [`on_expired`] reacts to the kernel's
//! [`QualificationExpired`] for the two kinds ([`KINDS`]) with **one message
//! per person**: the clinic managers of the branch(es) the qualification
//! covered and the owner / medical director, each naming the practitioner,
//! the branch and the number of appointments affected.
//!
//! Decisions:
//!
//! - *Affected appointments* are the bookings of the event's `affected` list
//!   that lie in a branch the qualification covered (a qualification with no
//!   branch covers every one), each counted once even when it is blocked at
//!   both `booking.book` and `booking.check_in`.
//! - *One owner message per expiry.* The document kinds are not marked
//!   compliance-critical in wl-documents (`host/src/app/clinics_seed.rs`), so
//!   `DOC.RUL.03` does not tell the owner a second time; this rule does, and
//!   `CLN.RUL.01` and `DOC.RUL.03` are both compliance-critical rules owned
//!   by the owner role, so a run that fails five times alerts the owner
//!   (`PLT.RUL.04`).
//! - The run is a kernel rule run per employee, kind and expiry date: a second
//!   `QualificationExpired` for the same expiry does nothing, and a run that
//!   failed half way is tried again without telling anybody twice
//!   ([`LicenceAlert`]).
//! - Nothing is cancelled: the appointments stay booked for the front desk to
//!   reassign or reschedule.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::chrono::NaiveDate;
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::automation::{RuleRunSpec, RunOutcome, run_rule};
use wl_kernel::compliance::CriticalRule;
use wl_kernel::notify::{
    self, GROUP_DOCUMENTS, NotificationTypeDef, NotifyChannel, NotifyRequest, notifier,
};
use wl_kernel::parties::directory;
use wl_kernel::scheduling::BOOKING_SUBJECT_TYPE;
use wl_kernel::workforce::{EmployeeId, QualificationExpired, workforce};
use wl_kernel::{COMPANY_ADMIN_ROLE, ClientConfig, Subject, scopes};

use crate::permissions::{CLINIC_MANAGER_ROLE, OWNER_ROLE};
use crate::seed::{LICENCE_KIND, PRIVILEGES_KIND};
use crate::texts::english;

/// The rule.
pub const RULE: &str = "CLN.RUL.01";
/// The notification type of the message.
pub const LICENCE_EXPIRED: &str = "clinics.licence_expired";
/// The document kinds the rule is about.
pub const KINDS: [&str; 2] = [LICENCE_KIND, PRIVILEGES_KIND];

/// Someone told that a licence expired, once per employee, kind, expiry date
/// and person.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "clinics_licence_alerts",
    unique(employee_id, kind, expires_on, recipient_user_id)
)]
pub struct LicenceAlert {
    /// The primary key.
    pub id: i64,
    /// The practitioner's employee record.
    pub employee_id: i64,
    /// The document kind that expired.
    pub kind: String,
    /// The expiry date; the document was valid through it.
    pub expires_on: NaiveDate,
    /// The user who was told.
    pub recipient_user_id: i64,
    /// How many appointments the message named.
    pub appointments: i64,
    /// When the message was sent.
    pub sent_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl LicenceAlert {
    /// The people told about this expiry.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_expiry(
        db: &Db,
        employee: i64,
        kind: &str,
        expires_on: NaiveDate,
    ) -> Result<Vec<LicenceAlert>> {
        LicenceAlert::query()
            .where_(LicenceAlert::EMPLOYEE_ID.eq(employee))
            .where_(LicenceAlert::KIND.eq(kind))
            .where_(LicenceAlert::EXPIRES_ON.eq(expires_on))
            .order_by(LicenceAlert::ID)
            .get(db)
            .await
    }
}

/// What a run told.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Told {
    /// The users who got the message in this run, by id.
    pub users: Vec<i64>,
    /// The appointments the message named.
    pub appointments: usize,
}

/// The scope of a branch subject: the one the app registered for its type,
/// else the subject's own type and id.
async fn scope_for(db: &Db, subject: &Subject) -> Result<Scope> {
    let registered = scopes::scope_of(db, subject).await?;
    Ok(if registered.is_global() {
        Scope::new(&subject.subject_type, subject.subject_id)
    } else {
        registered
    })
}

/// The branches a qualification covered, as scopes; global when it covered
/// every one.
async fn branches(db: &Db, event: &QualificationExpired) -> Result<Vec<Scope>> {
    if event.scope.is_empty() {
        return Ok(vec![Scope::global()]);
    }
    let mut found = Vec::with_capacity(event.scope.len());
    for subject in &event.scope {
        found.push(scope_for(db, subject).await?);
    }
    Ok(found)
}

/// The owner role of this deployment: `[roles] owner`, else the company
/// administrators.
fn owner_role(state: &AppState) -> String {
    state
        .provided::<Arc<ClientConfig>>()
        .map(|config| config.owner_role().to_owned())
        .unwrap_or_else(|| COMPANY_ADMIN_ROLE.to_owned())
}

/// The users to tell: the clinic managers of the branches and the owner,
/// once each, by id.
pub async fn recipients(state: &AppState, event: &QualificationExpired) -> Result<Vec<i64>> {
    let mut users = BTreeSet::new();
    for scope in branches(&state.db, event).await? {
        for user in users_with_role_in(&state.db, CLINIC_MANAGER_ROLE, &scope).await? {
            users.insert(user.id);
        }
    }
    for user in users_with_role_in(&state.db, &owner_role(state), &Scope::global()).await? {
        users.insert(user.id);
    }
    Ok(users.into_iter().collect())
}

/// How many appointments the expiry affects: the bookings of the event's
/// `affected` list that lie in a branch the qualification covered, each once.
///
/// # Errors
///
/// The database's errors.
pub async fn affected(state: &AppState, event: &QualificationExpired) -> Result<Vec<Subject>> {
    let covered = branches(&state.db, event).await?;
    let any = covered.iter().any(Scope::is_global);
    let mut seen = BTreeSet::new();
    let mut found = Vec::new();
    for work in &event.affected {
        if work.record.subject_type != BOOKING_SUBJECT_TYPE
            || !seen.insert(work.record.subject_id)
        {
            continue;
        }
        if !any {
            let at = scopes::scope_of(&state.db, &work.record).await?;
            if !covered.contains(&at) {
                continue;
            }
        }
        found.push(work.record.clone());
    }
    Ok(found)
}

/// The practitioner's name for staff: the party's display name, else
/// `employee 7`.
async fn practitioner(state: &AppState, employee: EmployeeId) -> String {
    let profile = workforce(state).staff(state, employee).await.ok().flatten();
    if let Some(profile) = profile
        && let Ok(Some(name)) = directory(state)
            .display_name(&state.db, profile.party, "en")
            .await
    {
        return name;
    }
    format!("employee {}", employee.0)
}

/// The names of the branches, comma separated.
async fn branch_names(db: &Db, covered: &[Scope]) -> Result<String> {
    let mut names = Vec::new();
    for scope in covered {
        names.push(scopes::label_of(db, scope).await?);
    }
    Ok(names.join(", "))
}

/// The kind as staff read it: `practitioner licence`.
fn kind_text(kind: &str) -> String {
    kind.replace('_', " ")
}

/// `20 Nov 2026`.
fn date_text(date: NaiveDate) -> String {
    date.format("%d %b %Y").to_string()
}

/// Tells the clinic managers and the owner that the qualification of `event`
/// expired. Each person is told once per expiry: a row of [`LicenceAlert`] is
/// written before the message and taken back if the message cannot be sent.
async fn tell(state: &AppState, event: &QualificationExpired) -> Result<Told> {
    let covered = branches(&state.db, event).await?;
    let bookings = affected(state, event).await?;
    let users = recipients(state, event).await?;
    let name = practitioner(state, event.employee).await;
    let mut told = Told {
        users: Vec::new(),
        appointments: bookings.len(),
    };
    for user in users {
        let mut alert = LicenceAlert {
            employee_id: event.employee.0,
            kind: event.kind.clone(),
            expires_on: event.expires_on,
            recipient_user_id: user,
            appointments: i64::try_from(bookings.len()).unwrap_or(i64::MAX),
            sent_at: now(),
            ..LicenceAlert::default()
        };
        match alert.save(&state.db).await {
            Ok(_) => {}
            // Told in an earlier attempt of this run.
            Err(error) if error.is_unique_violation() => continue,
            Err(error) => return Err(error),
        }
        let request = NotifyRequest::new(LICENCE_EXPIRED, event.employee.subject())
            .subject_label(&name)
            .to_user(user)
            .data(json!({
                "practitioner": name,
                "kind": kind_text(&event.kind),
                "branches": branch_names(&state.db, &covered).await?,
                "expires_on": date_text(event.expires_on),
                "count": bookings.len(),
            }));
        if let Err(error) = notifier(state).notify(state, request).await {
            LicenceAlert::query()
                .where_(LicenceAlert::ID.eq(alert.id))
                .delete(&state.db)
                .await?;
            return Err(error);
        }
        told.users.push(user);
    }
    Ok(told)
}

/// `QualificationExpired` of a practitioner licence or branch privileges:
/// runs the rule once for the practitioner, kind and expiry date. Any other
/// kind does nothing.
///
/// # Errors
///
/// The database's errors; a failed run is tried again with a backoff and,
/// after five attempts, alerts the owner (`PLT.RUL.04`).
pub async fn on_expired(
    state: &AppState,
    event: &QualificationExpired,
) -> Result<Option<(RunOutcome, Told)>> {
    if !KINDS.contains(&event.kind.as_str()) {
        return Ok(None);
    }
    let key = format!(
        "{RULE}:{}:{}:{}",
        event.employee.0, event.kind, event.expires_on
    );
    let spec = RuleRunSpec::new(RULE, event.employee.subject(), key);
    let told = Arc::new(Mutex::new(Told::default()));
    let out = Arc::clone(&told);
    let event = event.clone();
    let outcome = run_rule(state, spec, move |state: AppState| async move {
        let done = tell(&state, &event).await?;
        *out.lock().unwrap_or_else(PoisonError::into_inner) = done;
        Ok(())
    })
    .await?;
    let told = told.lock().unwrap_or_else(PoisonError::into_inner).clone();
    Ok(Some((outcome, told)))
}

/// The notification type of the message.
pub fn notification_types() -> Vec<NotificationTypeDef> {
    let subject = english("clinics.licences.subject", &[]);
    let body = english("clinics.licences.body", &[]);
    vec![
        NotificationTypeDef::new(LICENCE_EXPIRED, "clinics")
            .group(GROUP_DOCUMENTS)
            .template(NotifyChannel::Email, "en", Some(&subject), &body),
    ]
}

/// Registers the notification type, the listener and the two compliance-
/// critical rules owned by the owner / medical director.
pub(crate) fn register(app: &mut Registry) {
    for def in notification_types() {
        notify::register_type(def);
    }
    CriticalRule::new(RULE).owner_role(OWNER_ROLE).register();
    CriticalRule::new("DOC.RUL.03")
        .owner_role(OWNER_ROLE)
        .register();
    app.listen(|event: QualificationExpired, state: AppState| async move {
        on_expired(&state, &event).await.map(|_| ())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_is_about_the_licence_and_the_privileges() {
        assert_eq!(KINDS, ["practitioner_licence", "branch_privileges"]);
        assert_eq!(kind_text("practitioner_licence"), "practitioner licence");
    }

    #[test]
    fn the_date_reads_day_month_year() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 30).unwrap();
        assert_eq!(date_text(day), "30 Nov 2026");
    }
}
