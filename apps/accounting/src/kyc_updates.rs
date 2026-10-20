//! The CDD update task when a KYC document of a client being served expires
//! (04 §9 *Document blocks*, last paragraph; BL-ACC-12).
//!
//! An expired trade licence or UBO passport of a client with an `active` or
//! `suspended` engagement does not suspend the engagement and does not move
//! the CDD file. Instead, on `DocumentExpired` (wl-documents `DOC.RUL.03`,
//! the day after the expiry date) for a KYC kind ([`crate::seed::KYC_KINDS`])
//! whose subject is the client company or a UBO of the client's current CDD
//! file, the compliance officer gets a **CDD update task**, due 14 days after
//! the expiry: one per document and client (a UBO of two clients gives two
//! tasks), found again by its source key, so a second event for the same
//! document opens nothing. The engagement's manager is told, as the task's
//! watcher.
//!
//! On `DocumentAccepted` of the same kind and subject with an expiry date that
//! has not passed (the renewal), the open task is completed.
//!
//! An `onboarding` engagement gets no task: its activation is refused by the
//! document guard instead (BL-ACC-01). The returns flag at filing is
//! BL-ACC-19.
//!
//! Decision: wl-ticketing has no watchers, so the manager's watch is a notice
//! ([`CDD_UPDATE_NOTICE`]) when the task opens, and the task's description
//! names the engagement. Decision: the task is about the client's current
//! CDD file (the engagement when the client has none) and is a staff task in
//! the `task` category, assigned to the first holder of the MLRO role (else
//! the company admin), through the kernel's
//! [`role_tasks`](wl_kernel::role_tasks).

use renox::chrono::{Duration, NaiveDate};
use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::calendar::BusinessCalendar;
use wl_kernel::documents::{DocumentAccepted, DocumentExpired, today};
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest, notifier};
use wl_kernel::parties::{PARTY_SUBJECT_TYPE, directory};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::{TicketRef, task_creator};
use wl_kernel::{PartyId, Subject};

use crate::cdd::{CddFile, CddParty, ScreenedRole};
use crate::engagement::{Engagement, EngagementStatus};
use crate::permissions::MLRO_ROLE;
use crate::seed::KYC_KINDS;
use crate::texts::english;

/// Days after the expiry date the CDD update task is due (§9).
pub const UPDATE_DUE_DAYS: i64 = 14;

/// The engagement's manager is told a CDD update task was opened.
pub const CDD_UPDATE_NOTICE: &str = "accounting.cdd_update";

/// The engagement states whose client gets a CDD update task.
const SERVED: [EngagementStatus; 2] = [EngagementStatus::Active, EngagementStatus::Suspended];

/// The key the task of `client` for the document of `kind` about `holder`
/// is found by while it is open:
/// `accounting.cdd_update:<client>:parties.party:<holder>:<kind>`.
pub fn source_key(client: PartyId, holder: PartyId, kind: &str) -> String {
    format!("accounting.cdd_update:{}:{}:{kind}", client.0, holder.subject())
}

/// The day the task is due: 14 days after `expires_on`.
pub fn due_date(expires_on: NaiveDate) -> NaiveDate {
    expires_on + Duration::days(UPDATE_DUE_DAYS)
}

/// 09:00 Asia/Dubai on `day`, the task's due time.
fn due_at(day: NaiveDate) -> DateTime {
    let morning = day.and_hms_opt(9, 0, 0).unwrap_or_default();
    BusinessCalendar::default().from_local(morning)
}

/// Whether `kind` is a client KYC kind.
pub fn is_kyc_kind(kind: &str) -> bool {
    KYC_KINDS.iter().any(|k| k.key == kind)
}

/// The party a document is about, when it is about a party.
fn holder_of(subject: &Subject) -> Option<PartyId> {
    (subject.subject_type == PARTY_SUBJECT_TYPE).then_some(PartyId(subject.subject_id))
}

/// The clients `holder` holds KYC documents for: itself when it has an
/// engagement, and every client whose current CDD file names it as a UBO.
///
/// # Errors
///
/// The database's errors.
pub async fn clients_of(db: &Db, holder: PartyId) -> Result<Vec<PartyId>> {
    let mut clients = Vec::new();
    if !Engagement::of_client(db, holder).await?.is_empty() {
        clients.push(holder);
    }
    let rows = CddParty::query()
        .where_(CddParty::PARTY_ID.eq(holder.0))
        .where_(CddParty::ROLE.eq(ScreenedRole::Ubo))
        .order_by(CddParty::ID)
        .get(db)
        .await?;
    for row in rows {
        let Some(file) = CddFile::find(db, row.cdd_file_id).await? else {
            continue;
        };
        let client = file.client();
        let current = CddFile::current(db, client).await?;
        if current.is_some_and(|c| c.id == file.id) && !clients.contains(&client) {
            clients.push(client);
        }
    }
    Ok(clients)
}

/// The client's engagement being served (`active` or `suspended`), the
/// oldest first; `None` when it has none.
async fn served_engagement(db: &Db, client: PartyId) -> Result<Option<Engagement>> {
    Engagement::query()
        .where_(Engagement::CLIENT_PARTY_ID.eq(client.0))
        .where_(Engagement::STATUS.is_in(SERVED))
        .order_by(Engagement::ID)
        .first(db)
        .await
}

/// The kind's English name (`Trade licence`), else its key.
fn kind_name(kind: &str) -> String {
    KYC_KINDS
        .iter()
        .find(|k| k.key == kind)
        .map_or_else(|| kind.to_owned(), |k| k.name_en.to_owned())
}

/// The party's display name, else its number.
async fn name_of(state: &AppState, party: PartyId) -> Result<String> {
    Ok(directory(state)
        .display_name(&state.db, party, "en")
        .await?
        .unwrap_or_else(|| format!("#{}", party.0)))
}

/// Opens the CDD update task of `client` for `event`, unless one is open
/// under the same key. Returns the task when it was opened now.
async fn open_for(
    state: &AppState,
    client: PartyId,
    holder: PartyId,
    engagement: &Engagement,
    event: &DocumentExpired,
) -> Result<Option<TicketRef>> {
    let key = source_key(client, holder, &event.kind);
    if task_creator(state).find_open(state, &key).await?.is_some() {
        return Ok(None);
    }
    let subject = match CddFile::current(&state.db, client).await? {
        Some(file) => file.subject(),
        None => engagement.subject(),
    };
    let kind = kind_name(&event.kind);
    let holder_name = name_of(state, holder).await?;
    let client_name = name_of(state, client).await?;
    let due = due_date(event.expires_on);
    let title = english(
        "accounting.cdd_update.title",
        &[("kind", &kind), ("party", &holder_name)],
    );
    let description = english(
        "accounting.cdd_update.description",
        &[
            ("kind", &kind),
            ("party", &holder_name),
            ("client", &client_name),
            ("expired", &event.expires_on),
            ("engagement", &engagement.number()),
        ],
    );
    let task = RoleTask::new(subject, MLRO_ROLE, &title, &key)
        .description(&description)
        .due_at(due_at(due));
    let Some(opened) = role_tasks::open(state, task).await? else {
        return Ok(None);
    };
    if let Some(manager) = engagement.manager_user_id {
        let request = NotifyRequest::new(CDD_UPDATE_NOTICE, engagement.subject())
            .to_user(manager)
            .data(json!({
                "number": engagement.number(),
                "kind": kind,
                "party": holder_name,
                "client": client_name,
                "expired": event.expires_on.to_string(),
                "due": due.to_string(),
                "task": opened.number,
            }));
        notifier(state).notify(state, request).await?;
    }
    Ok(Some(opened))
}

/// `DocumentExpired`: a CDD update task for each client being served that
/// the document's holder holds KYC documents for. Nothing for another kind,
/// a document not about a party, or a client with no `active` or
/// `suspended` engagement. No engagement or CDD file changes state. Returns
/// the tasks opened now.
///
/// # Errors
///
/// The database's, the task creator's and the notifier's errors.
pub async fn on_expired(state: &AppState, event: &DocumentExpired) -> Result<Vec<TicketRef>> {
    let mut opened = Vec::new();
    if !is_kyc_kind(&event.kind) {
        return Ok(opened);
    }
    let Some(holder) = holder_of(&event.subject) else {
        return Ok(opened);
    };
    for client in clients_of(&state.db, holder).await? {
        let Some(engagement) = served_engagement(&state.db, client).await? else {
            continue;
        };
        if let Some(task) = open_for(state, client, holder, &engagement, event).await? {
            opened.push(task);
        }
    }
    Ok(opened)
}

/// `DocumentAccepted`: a renewal (an expiry date that has not passed)
/// completes the open CDD update task of each client of the holder for that
/// kind. Returns how many were completed.
///
/// # Errors
///
/// The database's and the task creator's errors.
pub async fn on_accepted(state: &AppState, event: &DocumentAccepted) -> Result<usize> {
    if !is_kyc_kind(&event.kind) {
        return Ok(0);
    }
    let Some(holder) = holder_of(&event.subject) else {
        return Ok(0);
    };
    if event.expires_on.is_some_and(|expiry| expiry < today()) {
        return Ok(0);
    }
    let creator = task_creator(state);
    let note = english(
        "accounting.cdd_update.completed",
        &[("kind", &kind_name(&event.kind))],
    );
    let mut completed = 0;
    for client in clients_of(&state.db, holder).await? {
        let key = source_key(client, holder, &event.kind);
        let Some(task) = creator.find_open(state, &key).await? else {
            continue;
        };
        if creator.complete(state, task.id, &note).await? {
            completed += 1;
        }
    }
    Ok(completed)
}

/// The notice the engagement's manager gets.
fn notification_type() -> NotificationTypeDef {
    let subject = english("accounting.cdd_update.notice_subject", &[]);
    let body = english("accounting.cdd_update.notice_body", &[]);
    NotificationTypeDef::new(CDD_UPDATE_NOTICE, "accounting")
        .group("compliance")
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// Registers the notice and the listeners.
pub(crate) fn register(app: &mut Registry) {
    notify::register_type(notification_type());
    app.listen(|event: DocumentExpired, state: AppState| async move {
        on_expired(&state, &event).await.map(|_| ())
    });
    app.listen(|event: DocumentAccepted, state: AppState| async move {
        on_accepted(&state, &event).await.map(|_| ())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_task_is_due_fourteen_days_after_the_expiry() {
        let expiry = NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();
        assert_eq!(due_date(expiry), NaiveDate::from_ymd_opt(2026, 11, 24).unwrap());
        assert_eq!(
            due_at(due_date(expiry)).to_rfc3339(),
            "2026-11-24T05:00:00+00:00"
        );
    }

    #[test]
    fn the_key_names_the_client_the_holder_and_the_kind() {
        assert_eq!(
            source_key(PartyId(3), PartyId(4), "passport"),
            "accounting.cdd_update:3:parties.party:4:passport"
        );
        assert!(is_kyc_kind("trade_licence"));
        assert!(!is_kyc_kind("lift_certificate"));
        assert_eq!(holder_of(&Subject::new("accounting.engagement", 4)), None);
        assert_eq!(holder_of(&PartyId(4).subject()), Some(PartyId(4)));
    }
}
