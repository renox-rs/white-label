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
    DeadlineFuture, DeadlineTaskId, DeadlineTasks, NewDeadlineTask, provide_deadline_tasks,
};
use wl_kernel::documents::{BlockCondition, BlockMode};
use wl_kernel::notify::{
    MessageId, Notifier, NotifyFuture, NotifyRecipient, NotifyRequest, provide_notifier,
};
use wl_kernel::subscriptions::{
    StartSubscription, SubscriptionFuture, SubscriptionId, Subscriptions, provide_subscriptions,
};
use wl_kernel::tickets::{
    Assignee, OpenTicket, TaskCreator, TicketFuture, TicketId, TicketRef, provide_task_creator,
};
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

/// What the notifier was asked for.
#[derive(Clone, Default)]
pub struct Recording(pub Arc<Mutex<Vec<NotifyRequest>>>);

impl Recording {
    /// Every request.
    pub fn all(&self) -> Vec<NotifyRequest> {
        self.0.lock().unwrap().clone()
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
}

impl Notifier for Recording {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        let id = {
            let mut all = self.0.lock().unwrap();
            all.push(request);
            all.len() as i64
        };
        Box::pin(async move { Ok(vec![MessageId(id)]) })
    }
}

/// The deadline tasks created.
#[derive(Clone, Default)]
pub struct TaskBook(pub Arc<Mutex<Vec<NewDeadlineTask>>>);

impl TaskBook {
    /// Every task created.
    pub fn all(&self) -> Vec<NewDeadlineTask> {
        self.0.lock().unwrap().clone()
    }

    /// The tasks of `source`.
    pub fn of(&self, source: &Subject) -> Vec<NewDeadlineTask> {
        self.all()
            .into_iter()
            .filter(|t| &t.source == source)
            .collect()
    }
}

impl DeadlineTasks for TaskBook {
    fn create<'a>(
        &'a self,
        _state: &'a AppState,
        task: NewDeadlineTask,
    ) -> DeadlineFuture<'a, Option<DeadlineTaskId>> {
        let id = {
            let mut all = self.0.lock().unwrap();
            all.push(task);
            all.len() as i64
        };
        Box::pin(async move { Ok(Some(DeadlineTaskId(id))) })
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

/// The subscriptions started.
#[derive(Clone, Default)]
pub struct SubscriptionBook(pub Arc<Mutex<Vec<StartSubscription>>>);

impl SubscriptionBook {
    /// Every subscription started.
    pub fn all(&self) -> Vec<StartSubscription> {
        self.0.lock().unwrap().clone()
    }
}

impl Subscriptions for SubscriptionBook {
    fn start<'a>(
        &'a self,
        _state: &'a AppState,
        start: StartSubscription,
    ) -> SubscriptionFuture<'a, Option<SubscriptionId>> {
        let id = {
            let mut all = self.0.lock().unwrap();
            all.push(start);
            all.len() as i64
        };
        Box::pin(async move { Ok(Some(SubscriptionId(id))) })
    }
}

/// Provides the fakes.
struct Fakes {
    notices: Recording,
    tasks: TaskBook,
    subscriptions: SubscriptionBook,
    tickets: TicketBook,
}

impl Module for Fakes {
    fn name(&self) -> &'static str {
        "fakes"
    }

    fn register(&self, app: &mut Registry) {
        provide_notifier(app, self.notices.clone());
        provide_deadline_tasks(app, self.tasks.clone());
        provide_subscriptions(app, self.subscriptions.clone());
        provide_task_creator(app, self.tickets.clone());
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
                PARTIES_VIEW,
                DOCUMENTS_VIEW,
            ],
        ),
        (
            ACCOUNTANT,
            &[
                perm::ENGAGEMENTS_VIEW,
                perm::CDD_VIEW,
                perm::CDD_PREPARE,
                perm::TAX_PROFILES_VIEW,
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
}

/// Accounting booted with auth, the audit log, the kernel, the real
/// wl-parties (screening on) and wl-documents, the fakes and the roles; the
/// sanctions lists exist, current, and the KYC kinds are installed.
pub async fn boot() -> Booted {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let fakes = Fakes {
        notices: Recording::default(),
        tasks: TaskBook::default(),
        subscriptions: SubscriptionBook::default(),
        tickets: TicketBook::default(),
    };
    let (notices, tasks, subscriptions, tickets) = (
        fakes.notices.clone(),
        fakes.tasks.clone(),
        fakes.subscriptions.clone(),
        fakes.tickets.clone(),
    );
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(Audit)
        .module(WlParties::new().screening(true))
        .module(wl_documents::WlDocuments)
        .module(wl_portal::WlPortal)
        .module(Accounting)
        .module(fakes);
    let config = wl_kernel::ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    define_roles(app.db()).await;
    let t = Booted {
        app,
        _views: views,
        notices,
        tasks,
        subscriptions,
        tickets,
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
        self.app.travel_back();
        let wanted = dubai(day, hour, minute);
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
    pub async fn screen_on_stale(&self, party: PartyId, list: &str, updated: DateTime) -> Screening {
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
        self.kyc(&reviewer, client.company, accounting::seed::TRADE_LICENCE, until)
            .await;
        self.kyc(&reviewer, client.ubo, accounting::seed::PASSPORT, passport_until)
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
        self.post(&preparer, &format!("/accounting/cdd/{}/submit", file.id), &[])
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
