//! Booting Accounting in a `TestApp` with auth, the kernel, the real
//! wl-parties (screening on: sanctions lists, screenings, blocks and their
//! restricted reasons) and the real wl-documents (KYC documents and their
//! one-use override), and recording fakes for the contracts that reach other
//! modules: the notifier, the deadline tasks and the subscriptions.
//!
//! Users per role ([`Booted::partner`], [`Booted::manager`],
//! [`Booted::mlro`], …) hold the permissions the host gives them; a client
//! company with a UBO, a director and a signatory ([`Booted::client`]), and
//! fixtures for an engagement in a given state ([`Booted::engagement_in`],
//! [`Booted::ready_engagement`]).

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use accounting::cdd_files::{self, Assessment};
use accounting::permissions as perm;
use accounting::{
    Accounting, CddFile, Engagement, EngagementStatus, NewEngagement, RiskRating, ScreenedRole,
    TaxProfile, VatPeriod,
};
use renox::audit::Audit;
use renox::auth::permissions;
use renox::auth::{Auth, Permissions};
use renox::chrono::{NaiveDate, TimeZone, Utc};
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::{TestApp, TestResponse};
use wl_documents::{Documents, KindSeed, NewDocument};
use wl_kernel::deadlines::{
    DeadlineFuture, DeadlineTaskId, DeadlineTaskStatus, DeadlineTaskSummary, DeadlineTasks,
    NewDeadlineTask, ReleasedTasks, provide_deadline_tasks,
};
use wl_kernel::documents::{BlockCondition, BlockMode};
use wl_kernel::invoicing::{
    InvoiceDraft, InvoiceFuture, InvoiceId, InvoiceStatus, InvoiceSummary, Invoicer,
    provide_invoicer,
};
use wl_kernel::notify::{
    MessageId, NoticeStatus, Notifier, NotifyFuture, NotifyRecipient, NotifyRequest,
    provide_notifier,
};
use wl_kernel::subscriptions::{
    PauseCause, PauseSubscription, StartSubscription, SubscriptionFuture, SubscriptionId,
    SubscriptionSummary, Subscriptions, provide_subscriptions,
};
use wl_kernel::tickets::{
    Assignee, OpenTicket, TaskCreator, TicketFuture, TicketId, TicketRef, provide_task_creator,
};
use wl_kernel::wip::{UnbilledTime, WipFuture, WipSummary, provide_unbilled_time};
use wl_kernel::{PartyId, PartyKind, Subject};
use wl_parties::{
    ListStatus, ListUsed, NewCandidate, NewParty, Party, SanctionsList, Screening, ScreeningRun,
    ScreeningTrigger, WlParties, record_run,
};

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// A client with parties and documents on.
const CLIENT: &str = r#"
[client]
name = "Falcon Ledger Accountants LLC"
app = "accounting"

[modules]
enabled = ["parties", "documents"]

[[legal_entities]]
code = "FLA"
name_en = "Falcon Ledger Accountants LLC"
trn = "100000000000003"
default = true
"#;

/// The guarded transition the KYC kinds block.
pub const ACTIVATE_GUARDED: &str = "engagement.activate";

/// The UN list's name, as wl-parties seeds it.
pub const UN: &str = "UN Consolidated List";
/// The UAE list's name, as wl-parties seeds it.
pub const UAE: &str = "UAE Local Terrorist List";

/// A date in 2026.
pub fn d(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, month, day).unwrap()
}

/// `day` November 2026 at `hour:minute` Asia/Dubai, as an instant.
pub fn dubai(day: u32, hour: u32, minute: u32) -> DateTime {
    Utc.with_ymd_and_hms(2026, 11, day, hour, minute, 0)
        .unwrap()
        - renox::chrono::Duration::hours(4)
}

// ---- the fakes -------------------------------------------------------------

/// What the notifier was asked for, and what it answers about the delivery
/// of a statutory notice: delivered when it was sent, unless the test made
/// it bounce ([`Recording::bounce`]); a recorded proof wins
/// ([`Recording::prove`]).
#[derive(Clone, Default)]
pub struct Recording {
    requests: Arc<Mutex<Vec<NotifyRequest>>>,
    delivered: Arc<Mutex<Vec<(Subject, String, DateTime)>>>,
    bounced: Arc<Mutex<Vec<Subject>>>,
    proofs: Arc<Mutex<Vec<(Subject, NaiveDate)>>>,
}

impl Recording {
    /// Every request.
    pub fn all(&self) -> Vec<NotifyRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// The requests of the notification type `key`.
    pub fn of_type(&self, key: &str) -> Vec<NotifyRequest> {
        self.all()
            .into_iter()
            .filter(|r| r.type_key == key)
            .collect()
    }

    /// The requests addressed to `party` (directly or one of its contacts):
    /// what would reach the client.
    pub fn to_party(&self, party: PartyId) -> Vec<NotifyRequest> {
        self.all()
            .into_iter()
            .filter(|r| {
                r.recipients.iter().any(|to| match to {
                    NotifyRecipient::Party { id } => *id == party,
                    NotifyRecipient::Contact { of, .. } => *of == party,
                    _ => false,
                })
            })
            .collect()
    }

    /// The messages about `subject` did not reach anyone: the e-mail
    /// bounced, so the notice is not delivered until a proof is recorded.
    pub fn bounce(&self, subject: &Subject) {
        self.bounced.lock().unwrap().push(subject.clone());
    }

    /// Staff recorded the proof that the notice about `subject` was
    /// delivered on `on`.
    pub fn prove(&self, subject: &Subject, on: NaiveDate) {
        self.proofs.lock().unwrap().push((subject.clone(), on));
    }
}

impl Notifier for Recording {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        let id = {
            self.delivered.lock().unwrap().push((
                request.subject.clone(),
                request.type_key.clone(),
                renox::db::now(),
            ));
            let mut all = self.requests.lock().unwrap();
            all.push(request);
            all.len() as i64
        };
        Box::pin(async move { Ok(vec![MessageId(id)]) })
    }

    fn notice_status<'a>(
        &'a self,
        _state: &'a AppState,
        subject: &'a Subject,
        type_key: &'a str,
    ) -> NotifyFuture<'a, NoticeStatus> {
        let proof = self
            .proofs
            .lock()
            .unwrap()
            .iter()
            .find(|(s, _)| s == subject)
            .map(|(_, on)| *on);
        let bounced = self.bounced.lock().unwrap().contains(subject);
        let delivered = self
            .delivered
            .lock()
            .unwrap()
            .iter()
            .find(|(s, key, _)| s == subject && key == type_key)
            .map(|(_, _, at)| *at);
        let status = match (proof, bounced, delivered) {
            (Some(delivered_on), _, _) => NoticeStatus::ProofRecorded { delivered_on },
            (None, false, Some(at)) => NoticeStatus::Delivered { at },
            _ => NoticeStatus::NotDelivered,
        };
        Box::pin(async move { Ok(status) })
    }
}

/// A deadline task in the [`TaskBook`].
#[derive(Debug, Clone)]
pub struct BookTask {
    /// The id.
    pub id: DeadlineTaskId,
    /// What was created.
    pub task: NewDeadlineTask,
    /// Whether it is held.
    pub held: bool,
    /// Its state.
    pub status: DeadlineTaskStatus,
}

impl BookTask {
    fn summary(&self) -> DeadlineTaskSummary {
        DeadlineTaskSummary {
            id: self.id,
            source: self.task.source.clone(),
            task_type: self.task.task_type.clone(),
            title: self.task.title.clone(),
            due_date: self.task.due_date,
            start_date: self.task.due_date - renox::chrono::Duration::days(self.task.lead_days),
            status: self.status,
            late: false,
            held: self.held,
            overdue_on: None,
            party: self.task.party,
            owner_user: self.task.owner_user,
            closed_reason: None,
            next: None,
        }
    }
}

/// The deadline tasks created, held and released.
#[derive(Clone, Default)]
pub struct TaskBook {
    tasks: Arc<Mutex<Vec<BookTask>>>,
    held_sources: Arc<Mutex<Vec<Subject>>>,
    holds: Arc<Mutex<Vec<(Subject, NaiveDate)>>>,
    releases: Arc<Mutex<Vec<(Subject, NaiveDate)>>>,
}

impl TaskBook {
    /// Every task created.
    pub fn all(&self) -> Vec<NewDeadlineTask> {
        self.tasks
            .lock()
            .unwrap()
            .iter()
            .map(|t| t.task.clone())
            .collect()
    }

    /// The tasks of `source`.
    pub fn of(&self, source: &Subject) -> Vec<NewDeadlineTask> {
        self.all()
            .into_iter()
            .filter(|t| &t.source == source)
            .collect()
    }

    /// The tasks of `source` with their held flag and state.
    pub fn book(&self, source: &Subject) -> Vec<BookTask> {
        self.tasks
            .lock()
            .unwrap()
            .iter()
            .filter(|t| &t.task.source == source)
            .cloned()
            .collect()
    }

    /// The sources held, with the day.
    pub fn holds(&self) -> Vec<(Subject, NaiveDate)> {
        self.holds.lock().unwrap().clone()
    }

    /// The sources released, with the day.
    pub fn releases(&self) -> Vec<(Subject, NaiveDate)> {
        self.releases.lock().unwrap().clone()
    }
}

impl DeadlineTasks for TaskBook {
    fn create<'a>(
        &'a self,
        _state: &'a AppState,
        task: NewDeadlineTask,
    ) -> DeadlineFuture<'a, Option<DeadlineTaskId>> {
        let held = self.held_sources.lock().unwrap().contains(&task.source);
        let id = {
            let mut all = self.tasks.lock().unwrap();
            let id = DeadlineTaskId(all.len() as i64 + 1);
            all.push(BookTask {
                id,
                task,
                held,
                status: DeadlineTaskStatus::Upcoming,
            });
            id
        };
        Box::pin(async move { Ok(Some(id)) })
    }

    fn of_subject<'a>(
        &'a self,
        _db: &'a Db,
        source: &'a Subject,
    ) -> DeadlineFuture<'a, Vec<DeadlineTaskSummary>> {
        let tasks = self.book(source).iter().map(BookTask::summary).collect();
        Box::pin(async move { Ok(tasks) })
    }

    fn hold<'a>(
        &'a self,
        _state: &'a AppState,
        source: &'a Subject,
        on: NaiveDate,
    ) -> DeadlineFuture<'a, Vec<DeadlineTaskSummary>> {
        {
            let mut sources = self.held_sources.lock().unwrap();
            if !sources.contains(source) {
                sources.push(source.clone());
            }
            for task in self.tasks.lock().unwrap().iter_mut() {
                if &task.task.source == source && !task.status.is_terminal() {
                    task.held = true;
                }
            }
            self.holds.lock().unwrap().push((source.clone(), on));
        }
        let tasks = self.book(source).iter().map(BookTask::summary).collect();
        Box::pin(async move { Ok(tasks) })
    }

    fn release<'a>(
        &'a self,
        _state: &'a AppState,
        source: &'a Subject,
        on: NaiveDate,
    ) -> DeadlineFuture<'a, ReleasedTasks> {
        let mut released = Vec::new();
        {
            self.held_sources.lock().unwrap().retain(|s| s != source);
            for task in self.tasks.lock().unwrap().iter_mut() {
                if &task.task.source == source && task.held {
                    task.held = false;
                    released.push(task.id);
                }
            }
            self.releases.lock().unwrap().push((source.clone(), on));
        }
        let answer = ReleasedTasks {
            released,
            ..ReleasedTasks::default()
        };
        Box::pin(async move { Ok(answer) })
    }
}

/// A staff task opened through the kernel's `TaskCreator`.
#[derive(Debug, Clone)]
pub struct StaffTask {
    /// The id.
    pub id: i64,
    /// What it is about.
    pub subject: Subject,
    /// The category.
    pub category: String,
    /// The title.
    pub title: String,
    /// The description.
    pub description: String,
    /// The source key.
    pub key: Option<String>,
    /// Who it is assigned to.
    pub assignee: Option<Assignee>,
    /// When it is due.
    pub due_at: Option<DateTime>,
    /// Whether it is still open.
    pub open: bool,
    /// The note it was completed with.
    pub note: String,
}

/// The staff tasks opened (renewal tasks, CDD update tasks, owner alerts).
#[derive(Clone, Default)]
pub struct TicketBook(pub Arc<Mutex<Vec<StaffTask>>>);

impl TicketBook {
    /// Every task, oldest first.
    pub fn all(&self) -> Vec<StaffTask> {
        self.0.lock().unwrap().clone()
    }

    /// The tasks whose source key starts with `prefix`.
    pub fn with_key(&self, prefix: &str) -> Vec<StaffTask> {
        self.all()
            .into_iter()
            .filter(|t| t.key.as_deref().is_some_and(|k| k.starts_with(prefix)))
            .collect()
    }
}

/// The ticket reference of task `id`.
fn ticket_ref(id: i64) -> TicketRef {
    TicketRef {
        id: TicketId(id),
        number: format!("TKT-{id}"),
    }
}

impl TaskCreator for TicketBook {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        let id = {
            let mut tasks = self.0.lock().unwrap();
            let id = tasks.len() as i64 + 1;
            tasks.push(StaffTask {
                id,
                subject: ticket.subject.clone(),
                category: ticket.category_key.clone(),
                title: ticket.title.clone(),
                description: ticket.description.clone(),
                key: ticket.source_key.clone(),
                assignee: ticket.assignee,
                due_at: ticket.due_at,
                open: true,
                note: String::new(),
            });
            id
        };
        Box::pin(async move { Ok(Some(ticket_ref(id))) })
    }

    fn find_open<'a>(
        &'a self,
        _state: &'a AppState,
        source_key: &'a str,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        let found = self
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|t| t.open && t.key.as_deref() == Some(source_key))
            .map(|t| ticket_ref(t.id));
        Box::pin(async move { Ok(found) })
    }

    fn complete<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        note: &'a str,
    ) -> TicketFuture<'a, bool> {
        let done = {
            let mut tasks = self.0.lock().unwrap();
            match tasks.iter_mut().find(|t| t.id == ticket.0 && t.open) {
                Some(task) => {
                    task.open = false;
                    task.note = note.to_owned();
                    true
                }
                None => false,
            }
        };
        Box::pin(async move { Ok(done) })
    }

    fn resolve_from_source<'a>(
        &'a self,
        state: &'a AppState,
        ticket: TicketId,
        note: &'a str,
    ) -> TicketFuture<'a, bool> {
        self.complete(state, ticket, note)
    }
}

/// The subscriptions started, paused and resumed.
#[derive(Clone, Default)]
pub struct SubscriptionBook {
    started: Arc<Mutex<Vec<StartSubscription>>>,
    pauses: Arc<Mutex<Vec<(Subject, PauseSubscription)>>>,
    resumes: Arc<Mutex<Vec<(Subject, PauseCause, NaiveDate)>>>,
}

impl SubscriptionBook {
    /// Every subscription started.
    pub fn all(&self) -> Vec<StartSubscription> {
        self.started.lock().unwrap().clone()
    }

    /// Every pause asked, with the agreement.
    pub fn pauses(&self) -> Vec<(Subject, PauseSubscription)> {
        self.pauses.lock().unwrap().clone()
    }

    /// Every resume asked, with the agreement.
    pub fn resumes(&self) -> Vec<(Subject, PauseCause, NaiveDate)> {
        self.resumes.lock().unwrap().clone()
    }
}

impl Subscriptions for SubscriptionBook {
    fn start<'a>(
        &'a self,
        _state: &'a AppState,
        start: StartSubscription,
    ) -> SubscriptionFuture<'a, Option<SubscriptionId>> {
        let id = {
            let mut all = self.started.lock().unwrap();
            all.push(start);
            all.len() as i64
        };
        Box::pin(async move { Ok(Some(SubscriptionId(id))) })
    }

    fn pause<'a>(
        &'a self,
        _state: &'a AppState,
        agreement: &'a Subject,
        pause: PauseSubscription,
    ) -> SubscriptionFuture<'a, Vec<SubscriptionSummary>> {
        self.pauses.lock().unwrap().push((agreement.clone(), pause));
        Box::pin(async { Ok(Vec::new()) })
    }

    fn resume<'a>(
        &'a self,
        _state: &'a AppState,
        agreement: &'a Subject,
        cause: PauseCause,
        on: NaiveDate,
    ) -> SubscriptionFuture<'a, Vec<SubscriptionSummary>> {
        self.resumes
            .lock()
            .unwrap()
            .push((agreement.clone(), cause, on));
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The overdue invoices of the parties, as wl-invoicing would answer.
#[derive(Clone, Default)]
pub struct InvoiceBook(pub Arc<Mutex<Vec<InvoiceSummary>>>);

impl InvoiceBook {
    /// `party` owes `balance` fils on an invoice overdue since `due`.
    pub fn overdue(&self, party: PartyId, number: &str, balance: i64, due: NaiveDate) {
        let mut all = self.0.lock().unwrap();
        let id = InvoiceId(all.len() as i64 + 1);
        all.push(InvoiceSummary {
            id,
            number: Some(number.to_owned()),
            status: InvoiceStatus::Overdue,
            party,
            legal_entity_id: None,
            currency: "AED".to_owned(),
            gross_total: balance,
            balance,
            due_date: Some(due),
            disputed: false,
            chasing_paused_until: None,
        });
    }
}

impl Invoicer for InvoiceBook {
    fn draft<'a>(
        &'a self,
        _db: &'a Db,
        _draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        Box::pin(async { Ok(None) })
    }

    fn find<'a>(&'a self, _db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        let found = self.0.lock().unwrap().iter().find(|i| i.id == id).cloned();
        Box::pin(async move { Ok(found) })
    }

    fn allocate<'a>(
        &'a self,
        _state: &'a AppState,
        _id: InvoiceId,
        _source: Subject,
        _amount: i64,
    ) -> InvoiceFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn overdue_of<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
    ) -> InvoiceFuture<'a, Vec<InvoiceSummary>> {
        let found = self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|i| i.party == party && i.status == InvoiceStatus::Overdue)
            .cloned()
            .collect();
        Box::pin(async move { Ok(found) })
    }
}

/// The unbilled time (WIP) of the engagements and the write-offs made, as
/// wl-attendance would answer.
#[derive(Clone, Default)]
pub struct WipBook {
    unbilled: Arc<Mutex<Vec<(Subject, i64)>>>,
    billed: Arc<Mutex<Vec<(Subject, i64)>>>,
    written_off: Arc<Mutex<Vec<(Subject, i64, String, Option<i64>)>>>,
}

impl WipBook {
    /// `agreement` has `fils` of unbilled time.
    pub fn set(&self, agreement: &Subject, fils: i64) {
        let mut all = self.unbilled.lock().unwrap();
        all.retain(|(s, _)| s != agreement);
        all.push((agreement.clone(), fils));
    }

    /// `agreement` has billed `fils` of time.
    pub fn billed(&self, agreement: &Subject, fils: i64) {
        let mut all = self.billed.lock().unwrap();
        all.retain(|(s, _)| s != agreement);
        all.push((agreement.clone(), fils));
    }

    /// The unbilled time of `agreement` now.
    pub fn unbilled_of(&self, agreement: &Subject) -> i64 {
        self.unbilled
            .lock()
            .unwrap()
            .iter()
            .find(|(s, _)| s == agreement)
            .map_or(0, |(_, fils)| *fils)
    }

    /// Every write-off: agreement, amount, reason, who.
    pub fn written_off(&self) -> Vec<(Subject, i64, String, Option<i64>)> {
        self.written_off.lock().unwrap().clone()
    }
}

impl UnbilledTime for WipBook {
    fn summary<'a>(
        &'a self,
        _state: &'a AppState,
        agreement: &'a Subject,
    ) -> WipFuture<'a, WipSummary> {
        let summary = WipSummary {
            unbilled: self.unbilled_of(agreement),
            billed: self
                .billed
                .lock()
                .unwrap()
                .iter()
                .find(|(s, _)| s == agreement)
                .map_or(0, |(_, fils)| *fils),
        };
        Box::pin(async move { Ok(summary) })
    }

    fn write_off<'a>(
        &'a self,
        _state: &'a AppState,
        agreement: &'a Subject,
        amount: i64,
        reason: &'a str,
        by: Option<i64>,
    ) -> WipFuture<'a, ()> {
        let left = self.unbilled_of(agreement) - amount;
        self.set(agreement, left.max(0));
        self.written_off
            .lock()
            .unwrap()
            .push((agreement.clone(), amount, reason.to_owned(), by));
        Box::pin(async { Ok(()) })
    }
}

/// Provides the fakes; with `real`, only the notifier (wl-ticketing and
/// wl-invoicing provide the rest).
struct Fakes {
    notices: Recording,
    tasks: TaskBook,
    subscriptions: SubscriptionBook,
    tickets: TicketBook,
    invoices: InvoiceBook,
    wip: WipBook,
    real: bool,
}

impl Module for Fakes {
    fn name(&self) -> &'static str {
        "fakes"
    }

    fn register(&self, app: &mut Registry) {
        provide_notifier(app, self.notices.clone());
        provide_unbilled_time(app, self.wip.clone());
        if !self.real {
            provide_deadline_tasks(app, self.tasks.clone());
            provide_subscriptions(app, self.subscriptions.clone());
            provide_task_creator(app, self.tickets.clone());
            provide_invoicer(app, self.invoices.clone());
        }
    }
}

/// The engagements section of the portal, as the host registers it
/// (`host/src/app/accounting_portal.rs`).
struct EngagementsSection;

impl wl_portal::PortalSection for EngagementsSection {
    fn title(&self) -> String {
        accounting::portal::title()
    }

    fn rows<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> wl_portal::SectionFuture<'a, Vec<wl_portal::SectionRow>> {
        Box::pin(async move {
            let rows = accounting::portal::rows(state, party).await?;
            Ok::<Vec<wl_portal::SectionRow>, Error>(
                rows.iter()
                    .map(|row| wl_portal::SectionRow::new(&row.label, &row.status))
                    .collect(),
            )
        })
    }
}

/// Registers the engagements section of the portal.
pub fn register_portal_section() {
    wl_portal::register_section(accounting::portal::SECTION, EngagementsSection);
}

mod returns;
pub use returns::Crew;

// ---- the roles -------------------------------------------------------------

/// The roles of the tests, with the permissions the host gives them.
pub const PARTNER: &str = perm::PARTNER_ROLE;
/// The engagement manager.
pub const MANAGER: &str = perm::MANAGER_ROLE;
/// The accountant.
pub const ACCOUNTANT: &str = perm::ACCOUNTANT_ROLE;
/// The MLRO and deputy.
pub const MLRO: &str = perm::MLRO_ROLE;
/// Reviews documents (accepts KYC documents).
pub const REVIEWER: &str = "document_reviewer";

/// wl-parties' and wl-documents' permissions the roles hold.
const PARTIES_VIEW: &str = wl_parties::permissions::VIEW;
const PARTIES_BLOCK: &str = wl_parties::permissions::BLOCK;
const DOCUMENTS_VIEW: &str = wl_documents::permissions::VIEW;
const DOCUMENTS_REVIEW: &str = wl_documents::permissions::REVIEW;
const BLOCK_OVERRIDE: &str = wl_documents::permissions::BLOCK_OVERRIDE;
const TICKETS_VIEW: &str = wl_ticketing::permissions::VIEW;
const TICKETS_WORK: &str = wl_ticketing::permissions::WORK;
const TASKS_VIEW: &str = wl_ticketing::permissions::TASKS_VIEW;
const TASKS_WORK: &str = wl_ticketing::permissions::TASKS_WORK;
const TASKS_REVIEW: &str = wl_ticketing::permissions::TASKS_REVIEW;
const TASKS_MANAGE: &str = wl_ticketing::permissions::TASKS_MANAGE;

async fn define_roles(db: &Db) {
    let roles: &[(&str, &[&str])] = &[
        (
            PARTNER,
            &[
                perm::ENGAGEMENTS_VIEW,
                perm::ENGAGEMENTS_MANAGE,
                perm::ENGAGEMENTS_END,
                perm::CDD_VIEW,
                perm::TAX_PROFILES_VIEW,
                perm::RETURNS_VIEW,
                perm::RETURNS_REVIEW,
                perm::RETURNS_FILE,
                perm::WIP_WRITE_OFF,
                PARTIES_VIEW,
            ],
        ),
        (
            MANAGER,
            &[
                perm::ENGAGEMENTS_VIEW,
                perm::ENGAGEMENTS_MANAGE,
                perm::CDD_VIEW,
                perm::CDD_PREPARE,
                perm::TAX_PROFILES_VIEW,
                perm::TAX_PROFILES_MANAGE,
                perm::RETURNS_VIEW,
                perm::RETURNS_REVIEW,
                perm::WIP_WRITE_OFF,
                PARTIES_VIEW,
                DOCUMENTS_VIEW,
                // As the host gives the 04 manager (A2).
                TICKETS_VIEW,
                TASKS_VIEW,
                TASKS_WORK,
                TASKS_REVIEW,
                TASKS_MANAGE,
            ],
        ),
        (
            ACCOUNTANT,
            &[
                perm::ENGAGEMENTS_VIEW,
                perm::CDD_VIEW,
                perm::CDD_PREPARE,
                perm::TAX_PROFILES_VIEW,
                perm::RETURNS_VIEW,
                perm::RETURNS_PREPARE,
            ],
        ),
        (
            MLRO,
            &[
                perm::ENGAGEMENTS_VIEW,
                perm::CDD_VIEW,
                perm::CDD_PREPARE,
                perm::CDD_APPROVE,
                perm::BLOCK_REASON_VIEW,
                PARTIES_VIEW,
                PARTIES_BLOCK,
                DOCUMENTS_VIEW,
                BLOCK_OVERRIDE,
                // As the host gives the MLRO: the CDD update task.
                TICKETS_VIEW,
                TICKETS_WORK,
                TASKS_VIEW,
                TASKS_WORK,
            ],
        ),
        (REVIEWER, &[DOCUMENTS_VIEW, DOCUMENTS_REVIEW]),
    ];
    for (role, grants) in roles {
        permissions::define_role(db, role, grants).await.unwrap();
    }
}

// ---- the booted app --------------------------------------------------------

/// A client company with the people the CDD file screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Client {
    /// The client company.
    pub company: PartyId,
    /// Its UBO.
    pub ubo: PartyId,
    /// Its director.
    pub director: PartyId,
    /// Its signatory.
    pub signatory: PartyId,
}

impl Client {
    /// Every screened party, the company first.
    pub fn parties(&self) -> [PartyId; 4] {
        [self.company, self.ubo, self.director, self.signatory]
    }
}

/// The app under test and its fakes.
pub struct Booted {
    /// The app.
    pub app: TestApp,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
    /// What the notifier was asked for.
    pub notices: Recording,
    /// The deadline tasks created.
    pub tasks: TaskBook,
    /// The subscriptions started.
    pub subscriptions: SubscriptionBook,
    /// The staff tasks opened.
    pub tickets: TicketBook,
    /// The overdue invoices the invoicer answers.
    pub invoices: InvoiceBook,
    /// The unbilled time (WIP) the time module would answer.
    pub wip: WipBook,
}

/// A client with parties, documents, ticketing and invoicing on (the real
/// modules of [`boot_real`]).
const CLIENT_REAL: &str = r#"
[client]
name = "Falcon Ledger Accountants LLC"
app = "accounting"

[modules]
enabled = ["parties", "documents", "ticketing", "invoicing"]

[[legal_entities]]
code = "FLA"
name_en = "Falcon Ledger Accountants LLC"
trn = "100000000000003"
default = true
"#;

/// Accounting booted with auth, the audit log, the kernel, the real
/// wl-parties (screening on) and wl-documents, the fakes and the roles; the
/// sanctions lists exist, current, and the KYC kinds are installed.
pub async fn boot() -> Booted {
    boot_with(false, false).await
}

/// [`boot`] with the real wl-ticketing (deadline tasks held, released and
/// marked overdue, the manager's staff tasks; as the host sets it for 04:
/// `ACC.RUL.04`, the manager role, the partner told) and the real
/// wl-invoicing (the retainer subscription paused and resumed) instead of
/// the fakes; the notifier is still the recording one.
pub async fn boot_real() -> Booted {
    boot_with(true, false).await
}

/// [`boot_real`] where the subscription's cycle invoices are issued by
/// themselves (as the host sets it for 04), so a cancel can credit the cycle
/// invoice of the retainer.
pub async fn boot_billing() -> Booted {
    boot_with(true, true).await
}

/// `module` with the 04 reasons for `close_missed`, as the host sets them.
fn missed_reasons(mut module: wl_ticketing::WlTicketing) -> wl_ticketing::WlTicketing {
    for (code, name, needs_text) in accounting::tasks::missed_reason_names() {
        module = if needs_text {
            module.missed_reason_with_text(code, &name)
        } else {
            module.missed_reason(code, &name)
        };
    }
    module
}

async fn boot_with(real: bool, billing: bool) -> Booted {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let fakes = Fakes {
        notices: Recording::default(),
        tasks: TaskBook::default(),
        subscriptions: SubscriptionBook::default(),
        tickets: TicketBook::default(),
        invoices: InvoiceBook::default(),
        wip: WipBook::default(),
        real,
    };
    let (notices, tasks, subscriptions, tickets, invoices, wip) = (
        fakes.notices.clone(),
        fakes.tasks.clone(),
        fakes.subscriptions.clone(),
        fakes.tickets.clone(),
        fakes.invoices.clone(),
        fakes.wip.clone(),
    );
    let mut app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(Audit)
        .module(WlParties::new().screening(true))
        .module(wl_documents::WlDocuments)
        .module(wl_portal::WlPortal);
    if real {
        app = app
            .module(missed_reasons(
                wl_ticketing::WlTicketing::new()
                    .overdue_rule_id(accounting::tasks::OVERDUE_RULE)
                    .task_manager_role(MANAGER)
                    .task_overdue_role(PARTNER)
                    .waive_started_tasks(false),
            ))
            .module(if billing {
                wl_invoicing::WlInvoicing::new()
                    .auto_issue_source(wl_kernel::subscriptions::SUBSCRIPTION_SUBJECT_TYPE)
            } else {
                wl_invoicing::WlInvoicing::new()
            });
    }
    let app = app.module(Accounting).module(fakes);
    let client = if real { CLIENT_REAL } else { CLIENT };
    let config = wl_kernel::ClientConfig::parse(client).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    define_roles(app.db()).await;
    if real {
        wl_ticketing::seed::install(app.db())
            .await
            .expect("the ticket categories seed");
    }
    let t = Booted {
        app,
        _views: views,
        notices,
        tasks,
        subscriptions,
        tickets,
        invoices,
        wip,
    };
    wl_parties::seed::lists(t.state())
        .await
        .expect("the sanctions lists seed");
    t.lists_current().await;
    t.install_kyc_kinds().await;
    t
}

impl Booted {
    /// The database.
    pub fn db(&self) -> &Db {
        self.app.db()
    }

    /// The app state.
    pub fn state(&self) -> &AppState {
        self.app.state()
    }

    /// Runs `work` at the app's (travelled) time.
    pub async fn run<F: std::future::Future>(&self, work: F) -> F::Output {
        self.app.at_travelled_time(work).await
    }

    /// The clock as the app sees it.
    pub async fn now(&self) -> DateTime {
        self.run(async { renox::db::now() }).await
    }

    /// Moves the clock to `day` November 2026 at `hour:minute` Asia/Dubai
    /// (forward from the real present).
    pub fn set_clock(&self, day: u32, hour: u32, minute: u32) {
        self.set_clock_on(d(11, day), hour, minute);
    }

    /// Moves the clock to `day` at `hour:minute` Asia/Dubai (forward from
    /// the real present).
    pub fn set_clock_on(&self, day: NaiveDate, hour: u32, minute: u32) {
        self.app.travel_back();
        let wanted = day
            .and_hms_opt(hour, minute, 0)
            .expect("a valid time")
            .and_utc()
            - renox::chrono::Duration::hours(4);
        let ahead = (wanted - Utc::now()).num_seconds();
        assert!(ahead > 0, "the fixed day is in the future");
        self.app
            .travel(std::time::Duration::from_secs(ahead.unsigned_abs()));
    }

    // ---- users ------------------------------------------------------------

    /// A user holding `role` everywhere.
    pub async fn user_with(&self, name: &str, role: &str) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(self.db(), name, &email, "password123")
            .await
            .unwrap();
        user.assign_role(self.db(), role).await.unwrap();
        user
    }

    /// A user holding exactly `grants` through a role of their own.
    pub async fn user_with_grants(&self, name: &str, grants: &[&str]) -> User {
        let role = format!("role_{}", name.to_lowercase());
        permissions::define_role(self.db(), &role, grants)
            .await
            .unwrap();
        self.user_with(name, &role).await
    }

    /// Pia, a partner (no `parties.block_reason.view`).
    pub async fn partner(&self) -> User {
        self.user_with("Pia", PARTNER).await
    }

    /// Amal, an engagement manager (no `parties.block_reason.view`).
    pub async fn manager(&self) -> User {
        self.user_with("Amal", MANAGER).await
    }

    /// A second engagement manager named `name`.
    pub async fn manager_named(&self, name: &str) -> User {
        self.user_with(name, MANAGER).await
    }

    /// Asha, an accountant: prepares CDD files.
    pub async fn accountant(&self) -> User {
        self.user_with("Asha", ACCOUNTANT).await
    }

    /// Mariam, the MLRO: approves CDD files and overrides, sees restricted
    /// reasons and possible matches.
    pub async fn mlro(&self) -> User {
        self.user_with("Mariam", MLRO).await
    }

    /// Rita, who accepts documents.
    pub async fn reviewer(&self) -> User {
        self.user_with("Rita", REVIEWER).await
    }

    /// A portal user `name` linked to the person `person`, acting for the
    /// client company `company` as its owner.
    pub async fn portal_user(&self, name: &str, person: PartyId, company: PartyId) -> User {
        let user = self.user_with_grants(name, &[]).await;
        wl_portal::PortalAccess::register(
            self.state(),
            user.id,
            wl_portal::NewGrant::new(person, company, "client", &["owner"]),
        )
        .await
        .expect("the portal grant is registered");
        user
    }

    // ---- requests ---------------------------------------------------------

    /// `POST path` as `user`, the way a sheet posts it (htmx).
    pub async fn post(&self, user: &User, path: &str, pairs: &[(&str, &str)]) -> TestResponse {
        self.app.acting_as(user);
        self.app.htmx().post(path, pairs).await
    }

    /// `GET path` as `user`.
    pub async fn get(&self, user: &User, path: &str) -> TestResponse {
        self.app.acting_as(user);
        self.app.get(path).await
    }

    // ---- parties ------------------------------------------------------------

    /// A party of `kind` named `name`.
    pub async fn party(&self, name: &str, kind: PartyKind) -> PartyId {
        let party = wl_parties::create(
            self.state(),
            NewParty {
                kind,
                legal_name_en: name.to_owned(),
                source: "test".to_owned(),
                ..NewParty::default()
            },
        )
        .await
        .expect("the party is created");
        PartyId(party.id)
    }

    /// The system blocks `party` with the reason `code` (as a confirmed
    /// match or the MLRO would); `PartyBlocked` follows after the commit.
    pub async fn block(&self, party: PartyId, code: &str) {
        let row = self.party_row(party).await;
        self.run(wl_parties::block_by_system(
            self.state(),
            &row,
            code,
            "reviewed by the MLRO",
            None,
        ))
        .await
        .expect("the party is blocked");
    }

    /// `user` lifts the block of `party` through wl-parties' route with
    /// `note` (a block that is not `sanctions` needs no evidence);
    /// `PartyUnblocked` follows after the commit.
    pub async fn unblock(&self, user: &User, party: PartyId, note: &str) {
        self.app.acting_as(user);
        let files: Vec<(&str, &str, &[u8])> = Vec::new();
        self.app
            .htmx()
            .post_multipart(
                &format!("/parties/{}/unblock", party.0),
                &[("note", note)],
                &files,
            )
            .await
            .assert_ok();
    }

    /// The wl-parties row of `party`.
    pub async fn party_row(&self, party: PartyId) -> Party {
        Party::find(self.db(), party.0).await.unwrap().unwrap()
    }

    /// Gulf Trading LLC with its UBO Omar Haddad, its director Lina Saeed
    /// and its signatory Karim Nasser. `tag` keeps the names apart when a test
    /// needs two clients.
    pub async fn client_named(&self, tag: &str) -> Client {
        Client {
            company: self
                .party(&format!("Gulf Trading{tag} LLC"), PartyKind::Company)
                .await,
            ubo: self
                .party(&format!("Omar Haddad{tag}"), PartyKind::Person)
                .await,
            director: self
                .party(&format!("Lina Saeed{tag}"), PartyKind::Person)
                .await,
            signatory: self
                .party(&format!("Karim Nasser{tag}"), PartyKind::Person)
                .await,
        }
    }

    /// The client of [`Booted::client_named`] without a tag.
    pub async fn client(&self) -> Client {
        self.client_named("").await
    }

    // ---- sanctions lists and screenings -----------------------------------

    /// Every sanctions list.
    pub async fn lists(&self) -> Vec<SanctionsList> {
        SanctionsList::all(self.db()).await.unwrap()
    }

    /// Every list was just updated (at the app's time): current.
    pub async fn lists_current(&self) {
        let now = self.now().await;
        for mut list in self.lists().await {
            list.last_updated_at = Some(now);
            list.status = ListStatus::Current;
            list.stale_since = None;
            list.last_error = None;
            list.save(self.db()).await.unwrap();
        }
    }

    /// The list `name` last updated at `updated` and stale since `since`.
    pub async fn list_stale(&self, name: &str, updated: DateTime, since: DateTime) {
        for mut list in self.lists().await {
            if list.name == name {
                list.last_updated_at = Some(updated);
                list.status = ListStatus::Stale;
                list.stale_since = Some(since);
                list.save(self.db()).await.unwrap();
            }
        }
    }

    /// The lists as a screening at the app's time would record them, the list
    /// `stale` (if any) flagged with its last update `stale_updated`.
    async fn lists_used(&self, stale: Option<(&str, DateTime)>) -> Vec<ListUsed> {
        let now = self.now().await;
        self.lists()
            .await
            .into_iter()
            .map(|list| {
                let flagged = stale.filter(|(name, _)| *name == list.name);
                ListUsed {
                    list_id: list.id,
                    name: list.name.clone(),
                    version: list.version.clone(),
                    last_updated_at: Some(flagged.map_or(now, |(_, at)| at)),
                    stale: flagged.is_some(),
                }
            })
            .collect()
    }

    /// Records a screening of `party` through wl-parties' `record_run`.
    async fn record(&self, party: PartyId, run: ScreeningRun) -> Screening {
        let row = self.party_row(party).await;
        self.run(record_run(self.state(), &row, run, None))
            .await
            .expect("the screening is recorded")
    }

    /// A `clear` screening of `party` on current lists.
    pub async fn screen_clear(&self, party: PartyId) -> Screening {
        let mut run = ScreeningRun::automatic(ScreeningTrigger::Onboarding, Vec::new());
        run.lists_used = self.lists_used(None).await;
        self.record(party, run).await
    }

    /// A `clear` screening of `party` that ran while `list` was stale (last
    /// updated at `updated`): it carries the stale flag.
    pub async fn screen_on_stale(
        &self,
        party: PartyId,
        list: &str,
        updated: DateTime,
    ) -> Screening {
        let mut run = ScreeningRun::automatic(ScreeningTrigger::Onboarding, Vec::new());
        run.lists_stale = true;
        run.lists_used = self.lists_used(Some((list, updated))).await;
        self.record(party, run).await
    }

    /// A screening of `party` that found a possible match (score 0.95 on the
    /// UN list), not decided yet.
    pub async fn screen_possible_match(&self, party: PartyId) -> Screening {
        let candidate = NewCandidate {
            name: "Omar H. Haddad".to_owned(),
            list_name: UN.to_owned(),
            score: 0.95,
            list_entry: Some("QDi.001".to_owned()),
        };
        let mut run = ScreeningRun::automatic(ScreeningTrigger::Onboarding, vec![candidate]);
        run.lists_used = self.lists_used(None).await;
        self.record(party, run).await
    }

    /// Every party of `client` screened `clear`.
    pub async fn screen_all_clear(&self, client: &Client) {
        for party in client.parties() {
            self.screen_clear(party).await;
        }
    }

    // ---- KYC documents ------------------------------------------------------

    /// The KYC kinds of `accounting::seed`, blocking `engagement.activate` for
    /// the roles each names, with the MLRO's override.
    pub async fn install_kyc_kinds(&self) {
        let seeds: Vec<KindSeed> = accounting::seed::KYC_KINDS
            .iter()
            .map(|kind| {
                KindSeed::new(kind.key, kind.name_en)
                    .name_ar(kind.name_ar)
                    .subject_type(wl_kernel::parties::PARTY_SUBJECT_TYPE)
                    .expires()
                    .look_ahead(kind.look_ahead_days)
                    .responsible_role(MLRO)
                    .blocks_when(
                        ACTIVATE_GUARDED,
                        BlockMode::Block,
                        BlockCondition::new("party_role", kind.roles.iter().copied()),
                    )
            })
            .collect();
        wl_documents::kinds::install_seeds(self.db(), &seeds)
            .await
            .expect("the KYC kinds install");
    }

    /// A document of `kind` about `party` expiring on `expiry`, accepted by
    /// `reviewer`.
    pub async fn kyc(&self, reviewer: &User, party: PartyId, kind: &str, expiry: NaiveDate) {
        let new = NewDocument::new(
            party.subject(),
            kind,
            Upload::new("scan.pdf", "application/pdf", b"%PDF-1.7 scan".to_vec()),
        )
        .expires_on(expiry);
        let document = self
            .run(Documents::attach(self.state(), None, new))
            .await
            .unwrap()
            .expect("the document is attached");
        self.post(reviewer, &format!("/documents/{}/accept", document.id), &[])
            .await
            .assert_ok();
    }

    /// The client's trade licence and its UBO's Emirates ID valid to the end
    /// of 2027, and the UBO's passport valid until `passport_until`.
    pub async fn kyc_with(&self, client: &Client, passport_until: NaiveDate) {
        let reviewer = self
            .user_with(&format!("Reviewer {}", client.company.0), REVIEWER)
            .await;
        let until = NaiveDate::from_ymd_opt(2027, 12, 31).unwrap();
        self.kyc(
            &reviewer,
            client.company,
            accounting::seed::TRADE_LICENCE,
            until,
        )
        .await;
        self.kyc(
            &reviewer,
            client.ubo,
            accounting::seed::PASSPORT,
            passport_until,
        )
        .await;
        self.kyc(&reviewer, client.ubo, accounting::seed::EMIRATES_ID, until)
            .await;
    }

    /// The client's KYC documents, all valid to the end of 2027.
    pub async fn kyc_valid(&self, client: &Client) {
        self.kyc_with(client, NaiveDate::from_ymd_opt(2027, 12, 31).unwrap())
            .await;
    }

    // ---- CDD files, tax profiles and engagements --------------------------

    /// A CDD file of `client` in progress with its UBO, director and
    /// signatory, rated low with the identity documents attached.
    pub async fn cdd_prepared(&self, client: &Client) -> CddFile {
        let file = cdd_files::open(self.db(), None, client.company)
            .await
            .unwrap();
        for (party, role) in [
            (client.ubo, ScreenedRole::Ubo),
            (client.director, ScreenedRole::Director),
            (client.signatory, ScreenedRole::Signatory),
        ] {
            cdd_files::add_party(self.state(), file.id, party, role)
                .await
                .unwrap();
        }
        cdd_files::assess(self.state(), file.id, Assessment::rated(RiskRating::Low))
            .await
            .unwrap()
    }

    /// The CDD file of `client` as stored now.
    pub async fn cdd(&self, file: &CddFile) -> CddFile {
        CddFile::find(self.db(), file.id).await.unwrap().unwrap()
    }

    /// A CDD file of `client` approved through the real routes: prepared,
    /// every party screened `clear` on current lists, submitted by an
    /// accountant and approved by the MLRO `mlro`.
    pub async fn cdd_approved(&self, client: &Client, mlro: &User) -> CddFile {
        let file = self.cdd_prepared(client).await;
        self.lists_current().await;
        self.screen_all_clear(client).await;
        let preparer = self
            .user_with(&format!("Preparer {}", file.id), ACCOUNTANT)
            .await;
        self.post(
            &preparer,
            &format!("/accounting/cdd/{}/submit", file.id),
            &[],
        )
        .await
        .assert_ok();
        self.post(mlro, &format!("/accounting/cdd/{}/approve", file.id), &[])
            .await
            .assert_ok();
        self.cdd(&file).await
    }

    /// A complete tax profile of `client`: VAT quarterly (first period ends
    /// 2026-03-31), financial year ending 31 December, effective 2026-01-01.
    pub async fn tax_profile(&self, client: &Client) -> TaxProfile {
        accounting::tax_profile::record(
            self.db(),
            None,
            TaxProfile {
                client_party_id: client.company.0,
                trn: Some("100000000000003".to_owned()),
                vat_registered: true,
                vat_period: VatPeriod::Quarterly,
                vat_first_period_end: Some(d(3, 31)),
                financial_year_end: Some(d(12, 31)),
                ct_registration_no: Some("CT-100200300".to_owned()),
                effective_date: d(1, 1),
                ..TaxProfile::default()
            },
        )
        .await
        .unwrap()
    }

    /// An `onboarding` retainer engagement of `client` (AED 5,000 a month).
    pub async fn engagement(&self, client: &Client) -> Engagement {
        accounting::engagements::create(
            self.state(),
            None,
            NewEngagement::retainer(client.company, "Bookkeeping and VAT", 500_000),
        )
        .await
        .unwrap()
    }

    /// An engagement of `client` inserted directly in `status` (a fixture),
    /// with the neutral `reason` when given.
    pub async fn engagement_in(
        &self,
        client: &Client,
        status: EngagementStatus,
        reason: Option<&str>,
    ) -> Engagement {
        let mut engagement = Engagement {
            client_party_id: client.company.0,
            title: "Bookkeeping and VAT".to_owned(),
            retainer_fee: 500_000,
            letter_signed: true,
            letter_signed_on: Some(d(10, 1)),
            reason_code: reason.map(str::to_owned),
            status,
            ..Engagement::default()
        };
        let mut tx = self.db().begin().await.unwrap();
        engagement
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        self.reload(&engagement).await
    }

    /// `engagement`, as stored now.
    pub async fn reload(&self, engagement: &Engagement) -> Engagement {
        Engagement::find(self.db(), engagement.id)
            .await
            .unwrap()
            .unwrap()
    }

    /// `user` records the signed letter of `engagement` on 2026-10-15.
    pub async fn sign_letter(&self, user: &User, engagement: &Engagement) {
        self.post(
            user,
            &format!("/accounting/engagements/{}/letter", engagement.id),
            &[("signed_on", "2026-10-15")],
        )
        .await
        .assert_ok();
    }

    /// Everything `activate` checks on the client, ready: its CDD file
    /// approved by `mlro` (every party screened `clear` on current lists),
    /// its tax profile complete, its KYC documents accepted with the UBO's
    /// passport valid until `passport_until`.
    pub async fn prepare_client(&self, client: &Client, mlro: &User, passport_until: NaiveDate) {
        self.cdd_approved(client, mlro).await;
        self.tax_profile(client).await;
        self.kyc_with(client, passport_until).await;
    }

    /// An `onboarding` engagement of `client` with its letter signed.
    pub async fn signed_engagement(&self, client: &Client) -> Engagement {
        let engagement = self.engagement(client).await;
        let manager = self
            .user_with(&format!("Letter {}", engagement.id), MANAGER)
            .await;
        self.sign_letter(&manager, &engagement).await;
        self.reload(&engagement).await
    }

    /// An `onboarding` engagement of `client` that passes every activation
    /// check: CDD approved by `mlro`, letter signed, tax profile complete,
    /// KYC documents valid, every party screened `clear` on current lists.
    pub async fn ready_engagement(&self, client: &Client, mlro: &User) -> Engagement {
        self.prepare_client(client, mlro, NaiveDate::from_ymd_opt(2027, 12, 31).unwrap())
            .await;
        self.signed_engagement(client).await
    }

    /// `user` runs `action` on `engagement` through its route, with `pairs`.
    pub async fn act(
        &self,
        user: &User,
        engagement: &Engagement,
        action: &str,
        pairs: &[(&str, &str)],
    ) -> TestResponse {
        self.post(
            user,
            &format!("/accounting/engagements/{}/{action}", engagement.id),
            pairs,
        )
        .await
    }

    /// `user` activates `engagement` through the route.
    pub async fn activate(&self, user: &User, engagement: &Engagement) -> TestResponse {
        self.act(user, engagement, "activate", &[]).await
    }

    /// The status of `engagement` now.
    pub async fn status(&self, engagement: &Engagement) -> EngagementStatus {
        self.reload(engagement).await.status
    }

    /// Runs the system transition `name` on `engagement` with `reason`.
    pub async fn system(&self, engagement: &Engagement, name: &str, reason: &str) {
        let current = self.reload(engagement).await;
        let _outcome = current
            .transition(name)
            .by_system()
            .reason(reason)
            .run(self.state())
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}
