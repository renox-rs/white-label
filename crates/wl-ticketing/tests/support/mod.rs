//! Booting wl-ticketing in a `TestApp`, as `crates/wl-invoicing/tests/support`
//! boots wl-invoicing: a party directory of its own (the crate does not
//! depend on wl-parties), a notifier that records what it is asked for, the
//! kernel events recorded, and a test-only module whose routes act as the
//! logged-in user at a time the test chooses (a test cannot build an
//! `AuthUser`, and the real routes take the time of the request).

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::auth::{Permissions, permissions};
use renox::axum::extract::Form;
use renox::chrono::{NaiveDate, NaiveDateTime};
use renox::db::{DateTime, sql};
use renox::prelude::*;
use renox::testing::TestApp;
use serde::Deserialize;
use wl_kernel::notify::{MessageId, Notifier, NotifyFuture, NotifyRecipient, NotifyRequest};
use wl_kernel::parties::{PartyDirectory, PartyFuture, PartyKind, PartyStatus, PartySummary};
use wl_kernel::invoicing::{InvoiceDraft, InvoiceFuture, InvoiceId, InvoiceSummary, Invoicer};
use wl_kernel::tickets::{Assignee, OpenTicket, TicketAssigned, TicketClosed};
use wl_kernel::{BusinessCalendar, ClientConfig, PartyId, Subject};
use wl_ticketing::{
    AssignRequest, Assignment, Priority, Ticket, Tickets, WlTicketing, assign, permissions as perm,
    seed, work,
};

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// A client with the ticketing module (and the parties it needs) on.
pub const CLIENT: &str = r#"
[client]
name = "Palm Grove Properties LLC"
app = "property"

[modules]
enabled = ["parties", "ticketing"]

[[legal_entities]]
code = "PGP"
name_en = "Palm Grove Properties LLC"
trn = "100000000000003"
default = true
"#;

/// The vendor "CoolAir" (active).
pub const COOLAIR: PartyId = PartyId(5);
/// The code of CoolAir.
pub const COOLAIR_CODE: &str = "P-00005";
/// The vendor "Dusty Ducts" (blocked).
pub const DUSTY: PartyId = PartyId(6);
/// The code of Dusty Ducts.
pub const DUSTY_CODE: &str = "P-00006";

/// The parties the tickets are assigned to.
#[derive(Debug, Clone, Copy, Default)]
pub struct TestDirectory;

fn summary(party: PartyId) -> Option<PartySummary> {
    let (code, name, status) = match party {
        COOLAIR => (COOLAIR_CODE, "CoolAir", PartyStatus::Active),
        DUSTY => (DUSTY_CODE, "Dusty Ducts", PartyStatus::Blocked),
        _ => return None,
    };
    Some(PartySummary {
        id: party,
        code: code.to_owned(),
        kind: PartyKind::Company,
        display_name: name.to_owned(),
        status,
        display_block_reason: None,
    })
}

impl PartyDirectory for TestDirectory {
    fn find<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async move { Ok(summary(party)) })
    }

    fn find_by_code<'a>(
        &'a self,
        _db: &'a Db,
        code: &'a str,
    ) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async move {
            Ok([COOLAIR, DUSTY]
                .into_iter()
                .filter_map(summary)
                .find(|party| party.code == code))
        })
    }

    fn display_name<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
        _lang: &'a str,
    ) -> PartyFuture<'a, Option<String>> {
        Box::pin(async move { Ok(summary(party).map(|p| p.display_name)) })
    }

    fn status<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        Box::pin(async move { Ok(summary(party).map(|p| p.status)) })
    }
}

/// A notifier that records every request and queues nothing.
#[derive(Clone, Default)]
pub struct Recording(Arc<Mutex<Vec<NotifyRequest>>>);

impl Recording {
    /// Every request so far, oldest first.
    pub fn all(&self) -> Vec<NotifyRequest> {
        self.0.lock().unwrap().clone()
    }

    /// The requests of one notification type, oldest first.
    pub fn of_type(&self, type_key: &str) -> Vec<NotifyRequest> {
        self.all()
            .into_iter()
            .filter(|r| r.type_key == type_key)
            .collect()
    }

    /// The recipients of every request of `type_key`, oldest first.
    pub fn recipients(&self, type_key: &str) -> Vec<NotifyRecipient> {
        self.of_type(type_key)
            .into_iter()
            .flat_map(|r| r.recipients)
            .collect()
    }
}

impl Notifier for Recording {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        self.0.lock().unwrap().push(request);
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// One draft the recording invoicer was asked for.
#[derive(Debug, Clone)]
pub struct Drafted {
    /// The id it answered with.
    pub id: i64,
    /// What it was asked for, with the lines as replaced.
    pub draft: InvoiceDraft,
    /// Whether the test issued it (then it is not rewritten).
    pub issued: bool,
}

/// An invoicer that keeps the drafts it is given and replaces the lines of a
/// draft that is not issued.
#[derive(Clone, Default)]
pub struct Invoices(Arc<Mutex<Vec<Drafted>>>);

impl Invoices {
    /// Every draft so far, oldest first.
    pub fn all(&self) -> Vec<Drafted> {
        self.0.lock().unwrap().clone()
    }

    /// Marks draft `id` as issued.
    pub fn issue(&self, id: i64) {
        for drafted in self.0.lock().unwrap().iter_mut() {
            if drafted.id == id {
                drafted.issued = true;
            }
        }
    }
}

impl Invoicer for Invoices {
    fn draft<'a>(
        &'a self,
        _db: &'a Db,
        draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        let mut all = self.0.lock().unwrap();
        let id = 100 + i64::try_from(all.len()).unwrap();
        all.push(Drafted {
            id,
            draft,
            issued: false,
        });
        Box::pin(async move { Ok(Some(InvoiceId(id))) })
    }

    fn update_draft<'a>(
        &'a self,
        _db: &'a Db,
        id: InvoiceId,
        draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, bool> {
        let mut all = self.0.lock().unwrap();
        let changed = match all.iter_mut().find(|d| d.id == id.0 && !d.issued) {
            Some(found) => {
                found.draft.lines = draft.lines;
                true
            }
            None => false,
        };
        Box::pin(async move { Ok(changed) })
    }

    fn find<'a>(&'a self, _db: &'a Db, _id: InvoiceId) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        Box::pin(async { Ok(None) })
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
}

/// Everything of one type `E` emitted so far.
#[derive(Clone)]
pub struct Recorded<E>(Arc<Mutex<Vec<E>>>);

impl<E: Clone> Recorded<E> {
    fn new() -> Self {
        Recorded(Arc::new(Mutex::new(Vec::new())))
    }

    /// The events, oldest first.
    pub fn all(&self) -> Vec<E> {
        self.0.lock().unwrap().clone()
    }
}

fn record<E: Event + Clone>(app: App, into: &Recorded<E>) -> App {
    let into = into.clone();
    app.listen(move |event: E, _state: AppState| {
        let into = into.clone();
        async move {
            into.0.lock().unwrap().push(event);
            Ok(())
        }
    })
}

/// Asia/Dubai `date` (`2026-11-03`) at `hour:minute`, as an instant.
pub fn dubai(date: &str, hour: u32, minute: u32) -> DateTime {
    let local = date
        .parse::<NaiveDate>()
        .expect("a date")
        .and_hms_opt(hour, minute, 0)
        .expect("a time");
    BusinessCalendar::default().from_local(local)
}

/// What the harness routes take: the time of the action (Asia/Dubai
/// wall-clock time) and the parts of the action.
#[derive(Deserialize)]
struct ActForm {
    at: NaiveDateTime,
    #[serde(default)]
    user: Option<i64>,
    #[serde(default)]
    party: Option<i64>,
    #[serde(default)]
    priority: Option<Priority>,
    #[serde(default)]
    visit: Option<NaiveDateTime>,
    #[serde(default)]
    reason_code: Option<String>,
    #[serde(default)]
    reason_text: Option<String>,
    #[serde(default)]
    hold_reason: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

fn instant(local: NaiveDateTime) -> DateTime {
    BusinessCalendar::default().from_local(local)
}

async fn act_assign(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<ActForm>,
) -> Result<String> {
    let ticket = Ticket::find_or_404(&state.db, id).await?;
    let assignee = match (form.user, form.party) {
        (Some(id), None) => Assignee::User { id },
        (None, Some(id)) => Assignee::Party { id: PartyId(id) },
        _ => return Err(Error::NotFound),
    };
    let request = AssignRequest {
        assignee,
        priority: form.priority.unwrap_or(ticket.priority),
        visit_at: form.visit.map(instant),
        reason_code: form.reason_code,
        reason_text: form.reason_text,
    };
    match assign(&state, &user, &ticket, request, instant(form.at)).await {
        Ok(kind) => Ok(kind.as_str().to_owned()),
        Err(error) => Err(error.into_error()),
    }
}

async fn act(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, action)): Path<(i64, String)>,
    Form(form): Form<ActForm>,
) -> Result<String> {
    let ticket = Ticket::find_or_404(&state.db, id).await?;
    let at = instant(form.at);
    let done = match action.as_str() {
        "start" => work::start(&state, &user, &ticket, at).await,
        "hold" => {
            let key = form.hold_reason.as_deref().unwrap_or_default();
            work::hold(&state, &user, &ticket, key, at).await
        }
        "resume" => work::resume(&state, &user, &ticket, at).await,
        "resolve" => work::resolve(&state, &user, &ticket, at).await,
        "confirm" => work::confirm(&state, &user, &ticket).await,
        "close" => work::close(&state, &user, &ticket).await,
        "reopen" => work::reopen(&state, &user, &ticket, at).await,
        "cancel" => {
            let reason = form.reason.as_deref().unwrap_or_default();
            work::cancel(&state, &user, &ticket, reason).await
        }
        _ => return Err(Error::NotFound),
    };
    match done {
        Ok(()) => Ok("done".to_owned()),
        Err(error) => Err(error.into_error()),
    }
}

/// The test-only routes.
struct Harness;

impl Module for Harness {
    fn name(&self) -> &'static str {
        "ticketing-test-harness"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .post("/test/tickets/{id}/assign", act_assign)
            .post("/test/tickets/{id}/do/{action}", act)
    }
}

/// The app under test with what it emitted.
pub struct Booted {
    /// The app.
    pub app: TestApp,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
    /// The notices asked of the kernel's notifier.
    pub notices: Recording,
    /// `TicketAssigned`.
    pub assigned: Recorded<TicketAssigned>,
    /// `TicketClosed`.
    pub closed: Recorded<TicketClosed>,
    /// The drafts the app's invoicer was asked for.
    pub invoices: Invoices,
}

/// `2026-11-03 09:00` as the form field `2026-11-03T09:00:00`.
pub fn local(text: &str) -> String {
    format!("{}:00", text.replacen(' ', "T", 1))
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

    /// A user holding exactly `grants` (through a role of their own).
    pub async fn user(&self, name: &str, grants: &[&str]) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(self.db(), name, &email, "password123")
            .await
            .unwrap();
        let role = format!("role_{}", name.to_lowercase());
        permissions::define_role(self.db(), &role, grants)
            .await
            .unwrap();
        user.assign_role(self.db(), &role).await.unwrap();
        user
    }

    /// The maintenance coordinator: view, create, assign, work.
    pub async fn coordinator(&self) -> User {
        self.user(
            "Carla",
            &[perm::VIEW, perm::CREATE, perm::ASSIGN, perm::WORK],
        )
        .await
    }

    /// A technician: view and work.
    pub async fn technician(&self, name: &str) -> User {
        self.user(name, &[perm::VIEW, perm::WORK]).await
    }

    /// Opens a ticket on `property.unit` 12 in `category`, asked for by
    /// `requester` when there is one, through the kernel contract's
    /// implementation (at a fixed time).
    pub async fn open(&self, category: &str, requester: Option<&User>) -> Ticket {
        let mut request = OpenTicket::new(Subject::new("property.unit", 12), category, "Chiller");
        if let Some(requester) = requester {
            request = request.requester_user(requester.id);
        }
        let opened = Tickets::open_at(self.state(), request, dubai("2026-11-03", 8, 0))
            .await
            .expect("the ticket opens");
        self.ticket(opened.id.0).await
    }

    /// The ticket `id`, as stored now.
    pub async fn ticket(&self, id: i64) -> Ticket {
        Ticket::find_or_404(self.db(), id).await.unwrap()
    }

    /// `ticket`, as stored now.
    pub async fn reload(&self, ticket: &Ticket) -> Ticket {
        self.ticket(ticket.id).await
    }

    /// The assignment rows of `ticket`, oldest first.
    pub async fn history(&self, ticket: &Ticket) -> Vec<Assignment> {
        Assignment::history(self.db(), ticket.id).await.unwrap()
    }

    /// `by` assigns `ticket` at `at` (`2026-11-03 09:00`, Asia/Dubai) as
    /// `pairs` say (`user` or `party`, `priority`, `visit`, `reason_code`,
    /// `reason_text`; a visit time is as [`local`] writes it).
    pub async fn assign(
        &self,
        by: &User,
        ticket: &Ticket,
        at: &str,
        pairs: &[(&str, &str)],
    ) -> renox::testing::TestResponse {
        let at = local(at);
        let mut fields = vec![("at", at.as_str())];
        fields.extend_from_slice(pairs);
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(&format!("/test/tickets/{}/assign", ticket.id), &fields)
            .await
    }

    /// `by` runs `action` (`start`, `hold`, `resume`, `resolve`, `confirm`,
    /// `close`, `reopen` or `cancel`) on `ticket` at `at`.
    pub async fn run(
        &self,
        by: &User,
        ticket: &Ticket,
        action: &str,
        at: &str,
        pairs: &[(&str, &str)],
    ) -> renox::testing::TestResponse {
        let at = local(at);
        let mut fields = vec![("at", at.as_str())];
        fields.extend_from_slice(pairs);
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(&format!("/test/tickets/{}/do/{action}", ticket.id), &fields)
            .await
    }
}

/// wl-ticketing booted in an app with auth and the kernel, seeded.
pub async fn boot() -> Booted {
    boot_with(WlTicketing::new()).await
}

/// `module` booted in an app with auth and the kernel, seeded.
pub async fn boot_with(module: WlTicketing) -> Booted {
    boot_client(CLIENT, module).await
}

/// `module` booted for the client `client` (its `[modules]` decide which
/// platform categories the seeder adds), seeded as `db:seed` seeds.
pub async fn boot_client(client: &str, module: WlTicketing) -> Booted {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();

    let assigned = Recorded::new();
    let closed = Recorded::new();
    let notices = Recording::default();
    let directory: Arc<dyn PartyDirectory> = Arc::new(TestDirectory);
    let notifier: Arc<dyn Notifier> = Arc::new(notices.clone());
    let invoices = Invoices::default();
    let invoicer: Arc<dyn Invoicer> = Arc::new(invoices.clone());
    let app = App::new()
        .module(Auth::new())
        .module(renox::audit::Audit)
        .module(Permissions)
        .module(module)
        .module(Harness)
        .provide(directory)
        .provide(notifier)
        .provide(invoicer);
    let app = record(app, &assigned);
    let app = record(app, &closed);
    let config = ClientConfig::parse(client).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    // The ticket list reads the names of vendor parties from wl-parties'
    // table, which this app does not have (the crate does not depend on
    // wl-parties): a stand-in with the two vendors of `TestDirectory`.
    sql(
        r#"CREATE TABLE "wl_parties_parties" (id INTEGER PRIMARY KEY, display_name TEXT NOT NULL)"#,
    )
    .execute(app.db())
    .await
    .expect("the stand-in table");
    sql(r#"INSERT INTO "wl_parties_parties" (id, display_name) VALUES (5, 'CoolAir'), (6, 'Dusty Ducts')"#)
        .execute(app.db())
        .await
        .expect("the stand-in rows");
    if client == CLIENT {
        seed::install(app.db()).await.expect("the defaults seed");
    } else {
        app.kernel()
            .call("ticketing:install", Vec::<String>::new())
            .await
            .expect("ticketing:install runs");
    }
    Booted {
        app,
        _views: views,
        notices,
        assigned,
        closed,
        invoices,
    }
}
