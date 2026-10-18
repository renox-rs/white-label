//! Booting wl-notify in a `TestApp`, as `crates/wl-parties/tests/support`
//! boots wl-parties: scripted e-mail and SMS providers, give-up handlers
//! that count their calls, a party directory of its own (the crate does not
//! depend on wl-parties, #55 A14) and the kernel events recorded.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use renox::auth::{Permissions, permissions};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::automation::AutomationFailed;
use wl_kernel::notify::{
    self, Category, ContactKind, GiveUpReaction, MessageFailed, MessageGaveUp, MessageId,
    MessageSuppressed, NotificationTypeDef, NotifyChannel, NotifyRecipient, NotifyRequest,
    PartyContactPoint, PartyContacts,
};
use wl_kernel::parties::{PartyDirectory, PartyFuture, PartyStatus, PartySummary};
use wl_kernel::{ClientConfig, PartyId, Subject};
use wl_notify::channels::ProviderFuture;
use wl_notify::reactions::HandlerFuture;
use wl_notify::{
    ChannelProvider, GiveUpContext, GiveUpHandler, Message, Outgoing, SendError, StepOutcome,
    WlNotify,
};

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// A client with the notify module (and the parties it needs) on.
pub const CLIENT: &str = r#"
[client]
name = "Palm Grove Properties LLC"
app = "property"

[modules]
enabled = ["parties", "notify"]

[[legal_entities]]
code = "PGP"
name_en = "Palm Grove Properties LLC"
default = true
"#;

/// The reminder type of the plan: e-mail, then SMS.
pub const REMINDER: &str = "invoice.reminder";
/// A statutory type: a notice with legal effect.
pub const NOTICE: &str = "lease.notice";
/// An e-mail-only type with no give-up reaction.
pub const RECEIPT: &str = "receipt.issued";

/// Blue Dunes LLC: an e-mail and a mobile.
pub const BLUE_DUNES: PartyId = PartyId(1);
/// Quiet Sands LLC: no contact point at all.
pub const QUIET_SANDS: PartyId = PartyId(2);
/// Red Rock Trading LLC: an e-mail only.
pub const RED_ROCK: PartyId = PartyId(3);

/// Dark Mesa LLC: an e-mail, and on hold (blocked).
pub const DARK_MESA: PartyId = PartyId(4);
/// A type that announces a pause (`block_effects::announces_pause`).
pub const PAUSE_NOTICE: &str = "subscription.paused";
/// Dark Mesa's e-mail.
pub const DARK_MESA_EMAIL: &str = "ops@darkmesa.example";

/// Blue Dunes' e-mail and its contact point.
pub const BLUE_DUNES_EMAIL: (&str, i64) = ("accounts@bluedunes.example", 11);
/// Blue Dunes' mobile and its contact point.
pub const BLUE_DUNES_MOBILE: (&str, i64) = ("+971500000301", 12);
/// Red Rock's e-mail.
pub const RED_ROCK_EMAIL: &str = "ap@redrock.example";

/// The test types, registered with the kernel before every boot (the
/// registry is process-wide; registering a key again replaces it).
pub fn register_types() {
    wl_kernel::block_effects::mark_announces_pause(PAUSE_NOTICE);
    notify::register_type(
        NotificationTypeDef::new(PAUSE_NOTICE, "subscriptions").template(
            NotifyChannel::Email,
            "en",
            Some("Your subscription is paused"),
            "Your subscription is paused.",
        ),
    );
    use GiveUpReaction::{AlternativeRecipient, MarkContactInvalid, StaffTask, StatutoryNotice};
    notify::register_type(
        NotificationTypeDef::new(REMINDER, "invoicing")
            .group("invoices")
            .category(Category::Reminder)
            .channels(&[NotifyChannel::Email, NotifyChannel::Sms])
            .give_up(&[AlternativeRecipient, MarkContactInvalid, StaffTask])
            .staff_role("collector")
            .template(
                NotifyChannel::Email,
                "en",
                Some("Invoice {{ number }} is overdue"),
                "Please pay invoice {{ number }}.",
            )
            .template(
                NotifyChannel::Sms,
                "en",
                None,
                "Invoice {{ number }} is overdue.",
            ),
    );
    notify::register_type(
        NotificationTypeDef::new(NOTICE, "property")
            .group("leases")
            .statutory(true)
            .channels(&[NotifyChannel::Email, NotifyChannel::Sms])
            .give_up(&[StatutoryNotice, MarkContactInvalid])
            .template(
                NotifyChannel::Email,
                "en",
                Some("Notice for lease {{ number }}"),
                "The terms of lease {{ number }} change.",
            ),
    );
    notify::register_type(
        NotificationTypeDef::new(RECEIPT, "payments")
            .give_up(&[GiveUpReaction::None])
            .template(
                NotifyChannel::Email,
                "en",
                Some("Receipt {{ number }}"),
                "We received your payment.",
            ),
    );
}

/// The parties the tests send to.
#[derive(Debug, Clone, Copy, Default)]
pub struct TestDirectory;

fn point(id: i64, kind: ContactKind, address: &str) -> PartyContactPoint {
    PartyContactPoint {
        id,
        kind,
        address: address.to_owned(),
        primary: true,
        verified: true,
    }
}

impl PartyDirectory for TestDirectory {
    fn find<'a>(&'a self, _db: &'a Db, _party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async { Ok(None) })
    }

    fn find_by_code<'a>(
        &'a self,
        _db: &'a Db,
        _code: &'a str,
    ) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async { Ok(None) })
    }

    fn display_name<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _lang: &'a str,
    ) -> PartyFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }

    fn status<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        let status = if party == DARK_MESA {
            PartyStatus::Blocked
        } else {
            PartyStatus::Active
        };
        Box::pin(async move { Ok(Some(status)) })
    }

    fn contacts<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
    ) -> PartyFuture<'a, Option<PartyContacts>> {
        let points = match party {
            BLUE_DUNES => vec![
                point(BLUE_DUNES_EMAIL.1, ContactKind::Email, BLUE_DUNES_EMAIL.0),
                point(
                    BLUE_DUNES_MOBILE.1,
                    ContactKind::Mobile,
                    BLUE_DUNES_MOBILE.0,
                ),
            ],
            QUIET_SANDS => Vec::new(),
            RED_ROCK => vec![point(31, ContactKind::Email, RED_ROCK_EMAIL)],
            DARK_MESA => vec![point(41, ContactKind::Email, DARK_MESA_EMAIL)],
            _ => return Box::pin(async { Ok(None) }),
        };
        Box::pin(async move {
            Ok(Some(PartyContacts {
                party,
                language: "en".into(),
                preferred_channel: None,
                points,
            }))
        })
    }
}

/// What a scripted provider answers to one send.
pub type Reply = std::result::Result<String, SendError>;

/// The answers of a scripted provider, in turn, and what it was given.
/// With no answer left, a send is accepted.
#[derive(Clone, Default)]
pub struct Script {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    sent: Arc<Mutex<Vec<Outgoing>>>,
}

impl Script {
    /// Answers the next sends with `replies`, in turn.
    pub fn then(&self, replies: impl IntoIterator<Item = Reply>) -> &Self {
        self.replies.lock().unwrap().extend(replies);
        self
    }

    /// Every send handed to the provider, oldest first.
    pub fn sent(&self) -> Vec<Outgoing> {
        self.sent.lock().unwrap().clone()
    }

    fn next(&self, outgoing: &Outgoing) -> Reply {
        let mut sent = self.sent.lock().unwrap();
        sent.push(outgoing.clone());
        let n = sent.len();
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(format!("{}-{n}", outgoing.channel)))
    }
}

/// A provider that answers from its [`Script`].
pub struct Scripted {
    channel: NotifyChannel,
    script: Script,
}

impl ChannelProvider for Scripted {
    fn name(&self) -> &'static str {
        match self.channel {
            NotifyChannel::Email => "scripted-email",
            NotifyChannel::Sms => "scripted-sms",
            NotifyChannel::Whatsapp => "scripted-whatsapp",
        }
    }

    fn channel(&self) -> NotifyChannel {
        self.channel
    }

    fn available(&self, _state: &AppState) -> bool {
        true
    }

    fn send<'a>(
        &'a self,
        _state: &'a AppState,
        outgoing: &'a Outgoing,
    ) -> ProviderFuture<'a, std::result::Result<String, SendError>> {
        let reply = self.script.next(outgoing);
        Box::pin(async move { reply })
    }
}

/// A give-up handler that records each call (reaction and message) and
/// fails while `failing` is set.
pub struct Counting {
    reaction: GiveUpReaction,
    calls: Arc<Mutex<Vec<(GiveUpReaction, MessageId)>>>,
    failing: Arc<AtomicBool>,
}

impl GiveUpHandler for Counting {
    fn reaction(&self) -> GiveUpReaction {
        self.reaction
    }

    fn run<'a>(
        &'a self,
        _state: &'a AppState,
        ctx: &'a GiveUpContext,
    ) -> HandlerFuture<'a, StepOutcome> {
        self.calls
            .lock()
            .unwrap()
            .push((self.reaction, ctx.message));
        let failing = self.failing.load(Ordering::SeqCst);
        Box::pin(async move {
            if failing {
                return Err(renox::Error::Internal(renox::anyhow::anyhow!(
                    "the task service is down"
                )));
            }
            Ok(StepOutcome::new(
                "tested",
                renox::serde_json::to_value(ctx).unwrap(),
            ))
        })
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

/// The app under test with its scripts and what it emitted.
pub struct Booted {
    /// The app.
    pub app: TestApp,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
    /// The e-mail provider's answers.
    pub email: Script,
    /// The SMS provider's answers.
    pub sms: Script,
    calls: Arc<Mutex<Vec<(GiveUpReaction, MessageId)>>>,
    failing: Arc<AtomicBool>,
    /// `MessageGaveUp`, as a requester module listening to it gets it.
    pub gave_up: Recorded<MessageGaveUp>,
    /// `MessageFailed`.
    pub failed: Recorded<MessageFailed>,
    /// `MessageSuppressed`.
    pub suppressed: Recorded<MessageSuppressed>,
    /// `AutomationFailed`.
    pub automation_failed: Recorded<AutomationFailed>,
}

impl Booted {
    /// The database.
    pub fn db(&self) -> &Db {
        self.app.db()
    }

    /// The handler calls so far: reaction and message, in call order.
    pub fn calls(&self) -> Vec<(GiveUpReaction, MessageId)> {
        self.calls.lock().unwrap().clone()
    }

    /// Makes every give-up handler fail (or work again).
    pub fn handlers_fail(&self, failing: bool) {
        self.failing.store(failing, Ordering::SeqCst);
    }

    /// Asks the module, through the kernel, for a `type_key` message about
    /// invoice 7 to `recipient`.
    pub async fn notify(&self, type_key: &str, recipient: NotifyRecipient) -> Vec<MessageId> {
        let request = NotifyRequest::new(type_key, Subject::new("invoicing.invoice", 7))
            .to(recipient)
            .data(renox::serde_json::json!({ "number": "INV-PGP-2026-0007" }));
        let state = self.app.state();
        self.app
            .at_travelled_time(notify::notifier(state).notify(state, request))
            .await
            .expect("the notifier queues the message")
    }

    /// Asks for a `type_key` message to the party `party`.
    pub async fn notify_party(&self, type_key: &str, party: PartyId) -> Vec<MessageId> {
        self.notify(type_key, NotifyRecipient::Party { id: party })
            .await
    }

    /// The message `id`, as stored now.
    pub async fn message(&self, id: MessageId) -> Message {
        Message::find(self.db(), id.0)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("no message {id}"))
    }

    /// Every message, oldest first.
    pub async fn messages(&self) -> Vec<Message> {
        Message::query()
            .order_by(Message::ID)
            .get(self.db())
            .await
            .unwrap()
    }

    /// Runs the queue now, at the travelled time.
    pub async fn run_jobs(&self) -> usize {
        self.app.run_jobs().await
    }

    /// Moves the clock `by` forward, then runs the jobs due.
    pub async fn after(&self, by: Duration) -> usize {
        self.app.travel(by);
        self.app.run_jobs().await
    }

    /// The names of the transitions message `id` went through, in order.
    pub async fn history(&self, id: MessageId) -> Vec<String> {
        renox::state_machine::history(self.db(), Message::TABLE, id.0)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.transition)
            .collect()
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
}

/// wl-notify with scripted providers and counting handlers, booted and
/// seeded (the registered types installed).
pub async fn boot() -> Booted {
    register_types();
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();

    let email = Script::default();
    let sms = Script::default();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let failing = Arc::new(AtomicBool::new(false));
    let mut module = WlNotify::new()
        .provider(Scripted {
            channel: NotifyChannel::Email,
            script: email.clone(),
        })
        .provider(Scripted {
            channel: NotifyChannel::Sms,
            script: sms.clone(),
        });
    for reaction in [
        GiveUpReaction::StaffTask,
        GiveUpReaction::AlternativeRecipient,
        GiveUpReaction::MarkContactInvalid,
        GiveUpReaction::StatutoryNotice,
    ] {
        module = module.reaction(Counting {
            reaction,
            calls: Arc::clone(&calls),
            failing: Arc::clone(&failing),
        });
    }

    let gave_up = Recorded::new();
    let failed = Recorded::new();
    let suppressed = Recorded::new();
    let automation_failed = Recorded::new();
    let directory: Arc<dyn PartyDirectory> = Arc::new(TestDirectory);
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(module)
        .provide(directory);
    let app = record(app, &gave_up);
    let app = record(app, &failed);
    let app = record(app, &suppressed);
    let app = record(app, &automation_failed);
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    app.kernel().seed().await.expect("the seeders run");
    Booted {
        app,
        _views: views,
        email,
        sms,
        calls,
        failing,
        gave_up,
        failed,
        suppressed,
        automation_failed,
    }
}
