//! A recalled lot traced to patients, and the neutral notice they get (05
//! §4.6 *Supplier, manufacturer or regulator recalls a lot*, `CLN.ACL.01`,
//! `STK.ACL.05`; BL-CLN-14).
//!
//! The recall is wl-inventory's: its owner opens it, every lot in scope is
//! held, and its trace follows the movements of the lots. Three things are
//! the clinic's:
//!
//! - [`VisitParties`], the app's `TracePartyResolver`: a visit
//!   (`clinics.visit:N`, the cost object of the consumable's movements) is
//!   its patient, or, for a dependant, the guardian (`PTY.REL.01`).
//! - [`trace`], the list the clinic manager and the medical director read:
//!   patient name, visit date and branch only, never the visit reason or a
//!   code, with the lines of *possible* use marked *lot uncertain*. Each view
//!   is audited. [`send`] is the clinic manager's: one notice per patient per
//!   recall listing every visit date, through the kernel's notifier, with the
//!   text of [`notice_types`]: the clinic's name, its phone number and "please
//!   call us about a product used in your visit on <date>", never the product,
//!   the reason or anything clinical.
//! - The call task. A notice that gives up (`MessageGaveUp`) is the
//!   `clinics.recall_notice` going back to the front desk of the visit's
//!   branch: wl-notify's give-up reaction opens the call task for the holders
//!   of the receptionist role in the scope of the notice (the branch of the
//!   first visit it covers); [`on_gave_up`] takes the lines back to
//!   *pending*; [`record_call`] records the outcome of the call and marks the
//!   lines *sent*.
//!
//! Decisions:
//!
//! - The clinic sends its own notice rather than the recall's *patient*
//!   template (which names the product and the reason): it tells the recall
//!   which lines it told through the kernel's `LotTrace::set_notice`.
//! - The clinic phone number is `phone` in the `[clinics]` section of
//!   `client.toml`; without it the text says "call us" with no number.
//! - A line whose party the resolver does not know (a visit with no patient)
//!   is listed and waived or handled in the recall, never noticed here.
//! - AU-20 is open: no authority notification is built; the recall report of
//!   wl-inventory can be exported.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::db::Json;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::StateMachine;
use wl_kernel::inventory::{
    RECALL_SUBJECT_TYPE, TraceFuture, TraceMark, TraceParty, TracePartyResolver, lot_trace,
    provide_trace_party_resolver,
};
use wl_kernel::notify::{
    self, Category, MessageGaveUp, NotificationTypeDef, NotifyChannel, NotifyRequest, Urgency,
};
use wl_kernel::scheduling::BOOKING_SUBJECT_TYPE;
use wl_kernel::{ClientConfig, PartyId, Subject, scopes};

use crate::authorisations::addressees;
use crate::db::refused;
use crate::permissions::RECEPTIONIST_ROLE;
use crate::recall_notice::{
    CALLED, GIVE_UP, NOTICE_TYPE, NoticeInput, RECALL_NOTICE_SUBJECT_TYPE, RecallNotice,
    RecallNoticeStatus,
};
use crate::texts::english;
use crate::visit::{VISIT_SUBJECT_TYPE, Visit};

/// The audit action of a view of the trace list (`CLN.ACL.01`).
pub const VIEWED: &str = "clinics.recall_viewed";

/// The audit action of a notice sent to a patient (`CLN.ACL.01`): who told
/// whom, never what the recall was about.
pub const SENT: &str = "clinics.recall_notice_sent";

/// The audit action of a notice shown on the call list of the front desk
/// (`CLN.ACL.01`: the list shows a patient's name and visit days).
pub const CALL_VIEWED: &str = "clinics.recall_call_viewed";

/// The Arabic subject of the notice.
const SUBJECT_AR: &str = "رسالة من {{ clinic }}";

/// The Arabic body of the notice.
const BODY_AR: &str =
    "{{ clinic }}: يرجى الاتصال بنا{{ contact_ar }} بخصوص منتج استُخدم خلال زيارتكم بتاريخ {{ dates }}.";

/// The Arabic way to give the phone number, ahead of its digits.
const CALL_US_ON_AR: &str = " على الرقم ";

/// The app's `TracePartyResolver`: a visit is its patient, or the guardian of
/// a dependant.
#[derive(Debug, Clone, Copy, Default)]
pub struct VisitParties;

impl TracePartyResolver for VisitParties {
    fn party<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
    ) -> TraceFuture<'a, TraceParty> {
        Box::pin(async move {
            if subject.subject_type != VISIT_SUBJECT_TYPE {
                return Ok(TraceParty::Unknown);
            }
            let Some(visit) = Visit::find(&state.db, subject.subject_id).await? else {
                return Ok(TraceParty::Unknown);
            };
            let Some(patient) = visit.patient_party_id else {
                return Ok(TraceParty::Unknown);
            };
            Ok(addressees(state, PartyId(patient))
                .await
                .first()
                .copied()
                .map_or(TraceParty::Unknown, TraceParty::Party))
        })
    }
}

/// One line of the trace list: who, which day, which branch. Never why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceRow {
    /// The line of the recall, `inventory.recall_line:7`.
    pub line: Subject,
    /// The visit.
    pub visit: i64,
    /// The patient.
    pub patient: Option<i64>,
    /// The party the notice goes to: the patient, or the guardian.
    pub addressee: i64,
    /// The day of the visit (Asia/Dubai).
    pub on: Option<NaiveDate>,
    /// The branch, as a label.
    pub branch: String,
    /// How sure the trace is about the line.
    pub mark: TraceMark,
    /// `pending`, `sent` or `waived`.
    pub notice: String,
}

impl TraceRow {
    /// Whether the line says *lot uncertain* (a pending issue or a possible
    /// use) rather than proves the lot.
    pub fn is_uncertain(&self) -> bool {
        self.mark != TraceMark::Confirmed
    }
}

/// The branch scope of a visit: the scope of its booking.
async fn branch_scope(db: &Db, visit: &Visit) -> Result<renox::auth::permissions::Scope> {
    scopes::scope_of(db, &Subject::new(BOOKING_SUBJECT_TYPE, visit.booking_id)).await
}

/// The lines of recall `recall` that are visits, oldest visit first. The
/// parties come from the resolver wl-inventory called when it traced.
///
/// # Errors
///
/// The database's and the trace's errors.
pub async fn trace(state: &AppState, recall: i64) -> Result<Vec<TraceRow>> {
    let holders = lot_trace(state)
        .holders(state, &Subject::new(RECALL_SUBJECT_TYPE, recall))
        .await?;
    let mut rows = Vec::new();
    for holder in holders {
        let Some(subject) = holder
            .subject
            .as_ref()
            .filter(|subject| subject.subject_type == VISIT_SUBJECT_TYPE)
        else {
            continue;
        };
        let Some(visit) = Visit::find(&state.db, subject.subject_id).await? else {
            continue;
        };
        let scope = branch_scope(&state.db, &visit).await?;
        rows.push(TraceRow {
            line: holder.line.clone(),
            visit: visit.id,
            patient: visit.patient_party_id,
            addressee: holder.party.map_or(0, |party| party.0),
            on: Some(visit.visit_date).or(holder.on),
            branch: scopes::label_of(&state.db, &scope).await?,
            mark: holder.mark,
            notice: holder.notice.clone(),
        });
    }
    rows.sort_by_key(|row| (row.on, row.visit));
    Ok(rows)
}

/// Records that `user` looked at the trace list of recall `recall`
/// (`CLN.ACL.01`: every view of health data is audited).
///
/// # Errors
///
/// The database's errors.
pub async fn audit_view(state: &AppState, user: &AuthUser, recall: i64) -> Result {
    renox::audit::record(
        &state.db,
        renox::audit::Entry::new(VIEWED)
            .user(user.id)
            .subject("inventory.recall", recall),
    )
    .await
}

/// Records that `user` looked at the notices `notices` on the call list, one
/// entry for each (`CLN.ACL.01`).
///
/// # Errors
///
/// The database's errors.
pub async fn audit_calls_view(state: &AppState, user: &AuthUser, notices: &[i64]) -> Result {
    for notice in notices {
        renox::audit::record(
            &state.db,
            renox::audit::Entry::new(CALL_VIEWED)
                .user(user.id)
                .subject(RECALL_NOTICE_SUBJECT_TYPE, *notice),
        )
        .await?;
    }
    Ok(())
}

/// The clinic's name and the way to reach it, for the text of a notice: the
/// English phrase, and the Arabic one (the Arabic text does not take an
/// English phrase).
fn clinic_contact(state: &AppState) -> (String, String, String) {
    let Some(config) = state.provided::<Arc<ClientConfig>>() else {
        return (
            String::new(),
            english("clinics.recall.call_us", &[]),
            String::new(),
        );
    };
    let phone = config
        .section("clinics")
        .and_then(|section| section.get("phone"))
        .and_then(|phone| phone.as_str())
        .map(str::trim)
        .filter(|phone| !phone.is_empty());
    let contact = match phone {
        Some(phone) => english("clinics.recall.call_us_on", &[("phone", &phone)]),
        None => english("clinics.recall.call_us", &[]),
    };
    let contact_ar = phone.map_or_else(String::new, |phone| format!("{CALL_US_ON_AR}{phone}"));
    (config.client.name.clone(), contact, contact_ar)
}

/// The patients still to be told, by the party the notice goes to: their
/// visits, lines and days.
#[derive(Debug, Default)]
struct Group {
    patient: Option<i64>,
    visits: BTreeSet<i64>,
    lines: Vec<Subject>,
    dates: BTreeSet<NaiveDate>,
}

/// Sends the notice of recall `recall_id`: one message to each party that
/// has visits among the lines still `pending`, listing all their visit days.
/// Returns the notices made; none when everyone was told.
///
/// # Errors
///
/// A refusal when the recall has no notice to send; the notifier's errors (a
/// notice that failed is removed and its lines stay `pending`).
pub async fn send(state: &AppState, by: &AuthUser, recall_id: i64) -> Result<Vec<RecallNotice>> {
    let rows = trace(state, recall_id).await?;
    let mut groups: BTreeMap<i64, Group> = BTreeMap::new();
    for row in rows
        .iter()
        .filter(|row| row.notice == "pending" && row.addressee != 0)
    {
        let group = groups.entry(row.addressee).or_default();
        group.patient = group.patient.or(row.patient);
        group.visits.insert(row.visit);
        group.lines.push(row.line.clone());
        group.dates.extend(row.on);
    }
    if groups.is_empty() {
        return Err(refused(english("clinics.recall.problem.nothing", &[])));
    }
    let (clinic, contact, contact_ar) = clinic_contact(state);
    let trace_service = lot_trace(state);
    let mut made = Vec::new();
    for (party, group) in groups {
        let dates = group
            .dates
            .iter()
            .map(NaiveDate::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let mut notice = RecallNotice {
            recall_id,
            party_id: party,
            patient_party_id: group.patient,
            visit_ids: Json(group.visits.iter().copied().collect()),
            line_ids: Json(group.lines.iter().map(|line| line.subject_id).collect()),
            visit_dates: dates.clone(),
            sent_by: Some(by.id),
            sent_at: Some(renox::db::now()),
            ..RecallNotice::default()
        };
        notice.save(&state.db).await?;
        let request = NotifyRequest::new(NOTICE_TYPE, notice.subject())
            .subject_label(&english("clinics.recall.label", &[]))
            .to_party(PartyId(party))
            .data(json!({
                "clinic": clinic,
                "contact": contact,
                "contact_ar": contact_ar,
                "dates": dates
            }));
        if let Err(error) = notify::notifier(state).notify(state, request).await {
            RecallNotice::query()
                .where_(RecallNotice::ID.eq(notice.id))
                .delete(&state.db)
                .await?;
            return Err(error);
        }
        for line in &group.lines {
            trace_service
                .set_notice(state, line, true, Some(by.id))
                .await?;
        }
        // Who told whom, and when: the notice names the recall and the
        // number of visits, nothing of the patient's health.
        renox::audit::record(
            &state.db,
            renox::audit::Entry::new(SENT)
                .user(by.id)
                .subject(RECALL_NOTICE_SUBJECT_TYPE, notice.id)
                .data(json!({ "recall": recall_id, "visits": group.visits.len() })),
        )
        .await?;
        made.push(notice);
    }
    Ok(made)
}

/// `MessageGaveUp` of a recall notice: the notice is `gave_up` and its lines
/// go back to `pending` (the call task is wl-notify's give-up reaction, for
/// the front desk of the notice's branch).
///
/// # Errors
///
/// The database's and the machine's errors.
pub async fn on_gave_up(state: &AppState, event: &MessageGaveUp) -> Result {
    if event.type_key != NOTICE_TYPE || event.subject.subject_type != RECALL_NOTICE_SUBJECT_TYPE {
        return Ok(());
    }
    let Some(notice) = RecallNotice::find(&state.db, event.subject.subject_id).await? else {
        return Ok(());
    };
    if notice.status != RecallNoticeStatus::Sent {
        return Ok(());
    }
    let _outcome = notice
        .transition(GIVE_UP)
        .by_system()
        .input(&NoticeInput {
            at: Some(event.gave_up_at),
            ..NoticeInput::default()
        })
        .run(state)
        .await
        .map_err(renox::state_machine::TransitionError::into_error)?;
    for line in notice.line_ids.0.iter() {
        lot_trace(state)
            .set_notice(
                state,
                &Subject::new(wl_kernel::inventory::RECALL_LINE_SUBJECT_TYPE, *line),
                false,
                None,
            )
            .await?;
    }
    Ok(())
}

/// The scope a notice lies in: the branch of the first visit it covers.
pub async fn notice_scope(
    db: &Db,
    notice: &RecallNotice,
) -> Result<renox::auth::permissions::Scope> {
    for visit in notice.visit_ids.0.iter() {
        if let Some(visit) = Visit::find(db, *visit).await? {
            return branch_scope(db, &visit).await;
        }
    }
    Ok(renox::auth::permissions::Scope::global())
}

/// The front desk records the outcome of the call to the patient of notice
/// `id`: the notice is `called` and its lines are `sent`.
///
/// # Errors
///
/// `Forbidden` without `recalls.call` in the notice's branch; a refusal for a
/// blank outcome or a notice that did not give up.
pub async fn record_call(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    outcome: &str,
) -> Result<RecallNotice> {
    let notice = RecallNotice::find_or_404(&state.db, id).await?;
    let scope = notice_scope(&state.db, &notice).await?;
    if !by.has_permission_in(crate::permissions::RECALLS_CALL, &scope) {
        return Err(Error::Forbidden);
    }
    let _outcome = notice
        .transition(CALLED)
        .by_system()
        .input(&NoticeInput {
            at: Some(renox::db::now()),
            outcome: Some(outcome.to_owned()),
            by: Some(by.id),
        })
        .run(state)
        .await
        .map_err(renox::state_machine::TransitionError::into_error)?;
    for line in notice.line_ids.0.iter() {
        lot_trace(state)
            .set_notice(
                state,
                &Subject::new(wl_kernel::inventory::RECALL_LINE_SUBJECT_TYPE, *line),
                true,
                Some(by.id),
            )
            .await?;
    }
    RecallNotice::find_or_404(&state.db, id).await
}

/// The notification type of the notice: urgent, no group default, the front
/// desk of the branch on give-up. The text names the clinic and asks the
/// patient to call; no product, no reason, nothing clinical.
pub fn notice_types() -> Vec<NotificationTypeDef> {
    let subject = english("clinics.recall.subject", &[]);
    let body = english("clinics.recall.body", &[]);
    vec![
        NotificationTypeDef::new(NOTICE_TYPE, "clinics")
            .group("clinics")
            .category(Category::Transactional)
            .urgency(Urgency::Urgent)
            .staff_role(RECEPTIONIST_ROLE)
            .template(NotifyChannel::Email, "en", Some(&subject), &body)
            .template(NotifyChannel::Email, "ar", Some(SUBJECT_AR), BODY_AR),
    ]
}

/// Registers the resolver, the notification type, the scope of a notice and
/// the listener of the give-up.
pub(crate) fn register(app: &mut Registry) {
    provide_trace_party_resolver(app, VisitParties);
    for def in notice_types() {
        notify::register_type(def);
    }
    crate::recall_notice::register();
    scopes::register(RECALL_NOTICE_SUBJECT_TYPE, |db, id| {
        Box::pin(async move {
            match RecallNotice::find(db, id).await? {
                Some(notice) => notice_scope(db, &notice).await,
                None => Ok(renox::auth::permissions::Scope::global()),
            }
        })
    });
    app.listen(
        |event: MessageGaveUp, state: AppState| async move { on_gave_up(&state, &event).await },
    );
}
