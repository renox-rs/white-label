//! Booting wl-notify in a `TestApp`, as `crates/wl-parties/tests/support`
//! boots wl-parties: scripted e-mail and SMS providers, give-up handlers
//! that count their calls, a party directory of its own (the crate does not
//! depend on wl-parties, #55 A14) and the kernel events recorded.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use renox::auth::permissions::Scope;
use renox::auth::{Permissions, permissions};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::automation::AutomationFailed;
use wl_kernel::notify::{
    self, Category, ContactKind, ContactPointInvalidated, ContactPurpose, ContactRelation,
    GiveUpReaction, MessageFailed, MessageGaveUp, MessageId, MessageSuppressed,
    NotificationTypeDef, NotifyChannel, NotifyRecipient, NotifyRequest, PartyContactPoint,
    PartyContacts, Urgency,
};
use wl_kernel::parties::{
    InvalidatedContact, PartyDirectory, PartyFuture, PartyStatus, PartySummary, RelatedContact,
};
use wl_kernel::tickets::{OpenTicket, TaskComment, TaskCreator, TicketFuture, TicketId, TicketRef};
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

[calendar]
holidays = ["2026-12-02"]
"#;

/// The reminder type of the plan: e-mail, then SMS.
pub const REMINDER: &str = "invoice.reminder";
/// A statutory type: a notice with legal effect.
pub const NOTICE: &str = "lease.notice";
/// An e-mail-only type that marks the contact point invalid and tells staff
/// (#58 AC1).
pub const INVOICE_ISSUED: &str = "invoice.issued";
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

/// Sara M.: an e-mail and a mobile (the staff task tests).
pub const SARA: PartyId = PartyId(5);
/// Sara's e-mail.
pub const SARA_EMAIL: &str = "sara@example.com";
/// Sara's mobile.
pub const SARA_MOBILE: &str = "+971501234567";
/// A type with a staff role (`coordinator`) and normal urgency.
pub const TICKET_UPDATE: &str = "ticket.update";
/// The same with urgency `urgent`.
pub const TICKET_EMERGENCY: &str = "ticket.emergency";
/// A type with no staff role.
pub const TICKET_DIGEST: &str = "ticket.digest";
/// The staff role of the ticket types.
pub const COORDINATOR: &str = "coordinator";
/// The ticket whose scope is Tower A.
pub const TOWER_A_TICKET: i64 = 42;
/// A ticket in Tower B.
pub const TOWER_B_TICKET: i64 = 43;
/// Another ticket in Tower A.
pub const TOWER_A_OTHER_TICKET: i64 = 44;
/// The subject text of the message, which a task must never carry.
pub const SECRET_SUBJECT: &str = "Your lift is out of order";
/// The body of the message, which a task must never carry.
pub const SECRET_BODY: &str = "The lift of tower A stops at 9 pm.";

/// Tower A.
pub fn tower_a() -> Scope {
    Scope::new("buildings", 1)
}

/// Tower B.
pub fn tower_b() -> Scope {
    Scope::new("buildings", 2)
}

/// Sami: a finance contact whose only address (e-mail) a test puts on the
/// suppression list.
pub const SAMI: PartyId = PartyId(10);
/// Sami's e-mail.
pub const SAMI_EMAIL: &str = "sami@lonesome.example";
/// Ali: the primary finance contact of Blue Dunes (e-mail and mobile).
pub const ALI: PartyId = PartyId(11);
/// Ali's e-mail.
pub const ALI_EMAIL: &str = "ali@bluedunes.example";
/// Ali's mobile.
pub const ALI_MOBILE: &str = "+971500000311";
/// Mona: the second finance contact of Blue Dunes; Arabic, a mobile only.
pub const MONA: PartyId = PartyId(12);
/// Mona's mobile.
pub const MONA_MOBILE: &str = "+971500000312";
/// Omar: the operations contact of Blue Dunes (never a finance contact).
pub const OMAR: PartyId = PartyId(13);
/// Zaid: a finance contact of Red Rock, another company.
pub const ZAID: PartyId = PartyId(14);
/// Lone Star LLC: finance contacts Rashid (primary) and Sami.
pub const LONE_STAR: PartyId = PartyId(40);
/// Rashid, the primary finance contact of Lone Star.
pub const RASHID: PartyId = PartyId(43);
/// Mixed Bag LLC: finance contacts Tala (primary), Sami and Nadia.
pub const MIXED_BAG: PartyId = PartyId(50);
/// Tala, the primary finance contact of Mixed Bag.
pub const TALA: PartyId = PartyId(41);
/// Nadia, the last finance contact of Mixed Bag.
pub const NADIA: PartyId = PartyId(42);
/// A student with one guardian.
pub const SOLO_STUDENT: PartyId = PartyId(20);
/// The only guardian of [`SOLO_STUDENT`].
pub const SOLO_GUARDIAN: PartyId = PartyId(21);
/// A student with two guardians.
pub const PAIR_STUDENT: PartyId = PartyId(22);
/// The primary guardian of [`PAIR_STUDENT`].
pub const FIRST_GUARDIAN: PartyId = PartyId(23);
/// The second guardian of [`PAIR_STUDENT`], not primary.
pub const SECOND_GUARDIAN: PartyId = PartyId(24);
/// A type for the guardians: `alternative_recipient` alone, staff role
/// `coordinator`.
pub const ABSENCE: &str = "student.absence";

/// The relationships of the test directory: to-party, relation, position,
/// from-party (a person), primary. (The real directory also drops ended
/// ones; that is tested in wl-parties.)
const RELATIONS: &[(PartyId, ContactRelation, Option<&str>, PartyId, bool)] = &[
    (
        BLUE_DUNES,
        ContactRelation::ContactOf,
        Some("finance"),
        MONA,
        false,
    ),
    (
        BLUE_DUNES,
        ContactRelation::ContactOf,
        Some("finance"),
        ALI,
        true,
    ),
    (
        BLUE_DUNES,
        ContactRelation::ContactOf,
        Some("operations"),
        OMAR,
        true,
    ),
    (
        RED_ROCK,
        ContactRelation::ContactOf,
        Some("finance"),
        ZAID,
        true,
    ),
    (
        LONE_STAR,
        ContactRelation::ContactOf,
        Some("finance"),
        RASHID,
        true,
    ),
    (
        LONE_STAR,
        ContactRelation::ContactOf,
        Some("finance"),
        SAMI,
        false,
    ),
    (
        MIXED_BAG,
        ContactRelation::ContactOf,
        Some("finance"),
        SAMI,
        false,
    ),
    (
        MIXED_BAG,
        ContactRelation::ContactOf,
        Some("finance"),
        NADIA,
        false,
    ),
    (
        MIXED_BAG,
        ContactRelation::ContactOf,
        Some("finance"),
        TALA,
        true,
    ),
    (
        SOLO_STUDENT,
        ContactRelation::GuardianOf,
        None,
        SOLO_GUARDIAN,
        true,
    ),
    (
        PAIR_STUDENT,
        ContactRelation::GuardianOf,
        None,
        SECOND_GUARDIAN,
        false,
    ),
    (
        PAIR_STUDENT,
        ContactRelation::GuardianOf,
        None,
        FIRST_GUARDIAN,
        true,
    ),
];

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
    notify::register_type(
        NotificationTypeDef::new(ABSENCE, "school")
            .channels(&[NotifyChannel::Email, NotifyChannel::Sms])
            .give_up(&[GiveUpReaction::AlternativeRecipient])
            .staff_role(COORDINATOR)
            .template(
                NotifyChannel::Email,
                "en",
                Some("Absence"),
                "Your child is absent.",
            )
            .template(NotifyChannel::Sms, "en", None, "Your child is absent."),
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
            )
            .template(
                NotifyChannel::Sms,
                "ar",
                None,
                "الفاتورة {{ number }} متأخرة.",
            ),
    );
    notify::register_type(
        NotificationTypeDef::new(INVOICE_ISSUED, "invoicing")
            .group("invoices")
            .give_up(&[MarkContactInvalid, StaffTask])
            .staff_role("collector")
            .template(
                NotifyChannel::Email,
                "en",
                Some("Invoice {{ number }}"),
                "Invoice {{ number }} is issued.",
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
    // A ticket is in the building its number says (the real resolver is
    // wl-ticketing's, which this crate does not depend on).
    wl_kernel::scopes::register("ticketing.ticket", |_db, id| {
        Box::pin(async move {
            Ok(match id {
                TOWER_A_TICKET | TOWER_A_OTHER_TICKET => tower_a(),
                TOWER_B_TICKET => tower_b(),
                _ => Scope::global(),
            })
        })
    });
    for (key, urgency, staff_role) in [
        (TICKET_UPDATE, Urgency::Normal, Some(COORDINATOR)),
        (TICKET_EMERGENCY, Urgency::Urgent, Some(COORDINATOR)),
        (TICKET_DIGEST, Urgency::Normal, None),
    ] {
        let mut def = NotificationTypeDef::new(key, "ticketing")
            .urgency(urgency)
            .channels(&[NotifyChannel::Email, NotifyChannel::Sms])
            .give_up(&[StaffTask])
            .template(
                NotifyChannel::Email,
                "en",
                Some(SECRET_SUBJECT),
                SECRET_BODY,
            )
            .template(NotifyChannel::Sms, "en", None, SECRET_BODY);
        if let Some(role) = staff_role {
            def = def.staff_role(role);
        }
        notify::register_type(def);
    }
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

/// One call that marked a contact point invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalidation {
    /// The contact point.
    pub point: i64,
    /// The since the handler gave.
    pub since: renox::db::DateTime,
    /// The reason the handler gave.
    pub reason: String,
}

/// The parties the tests send to. It records the contact points marked
/// invalid (the real status is wl-parties', #58 A1) and then reports them
/// `valid: false`, like the real directory.
#[derive(Debug, Clone, Default)]
pub struct TestDirectory {
    invalid: Arc<Mutex<Vec<Invalidation>>>,
}

impl TestDirectory {
    /// The points marked invalid, oldest first.
    pub fn invalidations(&self) -> Vec<Invalidation> {
        self.invalid.lock().unwrap().clone()
    }

    fn is_invalid(&self, point: i64) -> bool {
        self.invalid
            .lock()
            .unwrap()
            .iter()
            .any(|i| i.point == point)
    }
}

/// The parties whose points can be marked invalid.
const MARKABLE: [PartyId; 3] = [BLUE_DUNES, RED_ROCK, SARA];

fn point(id: i64, kind: ContactKind, address: &str) -> PartyContactPoint {
    PartyContactPoint {
        id,
        kind,
        address: address.to_owned(),
        primary: true,
        verified: true,
        valid: true,
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
        party: PartyId,
        _lang: &'a str,
    ) -> PartyFuture<'a, Option<String>> {
        let name = match party {
            SARA => Some("Sara M.".to_owned()),
            ALI => Some("Ali".to_owned()),
            MONA => Some("Mona".to_owned()),
            _ => None,
        };
        Box::pin(async move { Ok(name) })
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
        let both = |email: &str, mobile: &str, id: i64| {
            vec![
                point(id, ContactKind::Email, email),
                point(id + 1, ContactKind::Mobile, mobile),
            ]
        };
        let language = if party == MONA { "ar" } else { "en" };
        let points = match party {
            ALI => both(ALI_EMAIL, ALI_MOBILE, 111),
            MONA => vec![point(121, ContactKind::Mobile, MONA_MOBILE)],
            OMAR => both("omar@bluedunes.example", "+971500000313", 131),
            ZAID => both("zaid@redrock.example", "+971500000314", 141),
            SAMI => vec![point(101, ContactKind::Email, SAMI_EMAIL)],
            RASHID => both("rashid@lonestar.example", "+971500000343", 431),
            TALA => both("tala@mixedbag.example", "+971500000341", 411),
            NADIA => both("nadia@mixedbag.example", "+971500000342", 421),
            SOLO_GUARDIAN => both("solo@family.example", "+971500000321", 211),
            FIRST_GUARDIAN => both("first@family.example", "+971500000323", 231),
            SECOND_GUARDIAN => both("second@family.example", "+971500000324", 241),
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
            SARA => vec![
                point(51, ContactKind::Email, SARA_EMAIL),
                point(52, ContactKind::Mobile, SARA_MOBILE),
            ],
            DARK_MESA => vec![point(41, ContactKind::Email, DARK_MESA_EMAIL)],
            _ => return Box::pin(async { Ok(None) }),
        };
        let points: Vec<_> = points
            .into_iter()
            .map(|mut p| {
                p.valid = !self.is_invalid(p.id);
                p
            })
            .collect();
        Box::pin(async move {
            Ok(Some(PartyContacts {
                party,
                language: language.into(),
                preferred_channel: None,
                points,
            }))
        })
    }

    fn invalidate_contact<'a>(
        &'a self,
        db: &'a Db,
        point: i64,
        since: renox::db::DateTime,
        reason: &'a str,
    ) -> PartyFuture<'a, Option<InvalidatedContact>> {
        Box::pin(async move {
            if self.is_invalid(point) {
                return Ok(None);
            }
            for party in MARKABLE {
                let Some(contacts) = self.contacts(db, party).await? else {
                    continue;
                };
                if let Some(found) = contacts.points.iter().find(|p| p.id == point) {
                    self.invalid.lock().unwrap().push(Invalidation {
                        point,
                        since,
                        reason: reason.to_owned(),
                    });
                    return Ok(Some(InvalidatedContact {
                        party,
                        kind: found.kind,
                    }));
                }
            }
            Ok(None)
        })
    }

    fn related_contacts<'a>(
        &'a self,
        _db: &'a Db,
        of: PartyId,
        purpose: &'a ContactPurpose,
    ) -> PartyFuture<'a, Vec<RelatedContact>> {
        let mut found: Vec<RelatedContact> = RELATIONS
            .iter()
            .filter(|(to, relation, position, ..)| {
                *to == of
                    && *relation == purpose.relation
                    && (purpose.position.is_none() || *position == purpose.position.as_deref())
            })
            .map(|(_, _, position, from, primary)| RelatedContact {
                party: *from,
                position: position.map(str::to_owned),
                primary: *primary,
            })
            .collect();
        found.sort_by_key(|c| (!c.primary, c.party));
        Box::pin(async move { Ok(found) })
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

/// The tasks a [`Tasks`] creator opened.
#[derive(Debug, Default)]
struct TaskLog {
    opened: Vec<(TicketRef, OpenTicket)>,
    closed: Vec<TicketId>,
    comments: Vec<(TicketId, TaskComment)>,
}

/// A task creator that records what it is asked for, finds an open task by
/// its source key until [`Tasks::close`] is called for it.
#[derive(Clone, Default)]
pub struct Tasks(Arc<Mutex<TaskLog>>);

impl Tasks {
    /// The tasks opened, oldest first.
    pub fn opened(&self) -> Vec<OpenTicket> {
        let log = self.0.lock().unwrap();
        log.opened.iter().map(|(_, t)| t.clone()).collect()
    }

    /// The comments added: ticket and comment, oldest first.
    pub fn comments(&self) -> Vec<(TicketId, TaskComment)> {
        self.0.lock().unwrap().comments.clone()
    }

    /// Closes the task `id`: it is no longer found.
    pub fn close(&self, id: TicketId) {
        self.0.lock().unwrap().closed.push(id);
    }
}

impl TaskCreator for Tasks {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        let mut log = self.0.lock().unwrap();
        let id = log.opened.len() as i64 + 1;
        let reference = TicketRef {
            id: TicketId(id),
            number: format!("TKT-2026-{id:04}"),
        };
        log.opened.push((reference.clone(), ticket));
        Box::pin(async move { Ok(Some(reference)) })
    }

    fn find_open<'a>(
        &'a self,
        _state: &'a AppState,
        source_key: &'a str,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        let log = self.0.lock().unwrap();
        let found = log
            .opened
            .iter()
            .rev()
            .find(|(r, t)| {
                t.source_key.as_deref() == Some(source_key) && !log.closed.contains(&r.id)
            })
            .map(|(r, _)| r.clone());
        Box::pin(async move { Ok(found) })
    }

    fn comment<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        comment: TaskComment,
    ) -> TicketFuture<'a, ()> {
        self.0.lock().unwrap().comments.push((ticket, comment));
        Box::pin(async { Ok(()) })
    }
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
    /// The tasks asked of the recording task creator the app provides.
    pub tasks: Tasks,
    /// `MessageGaveUp`, as a requester module listening to it gets it.
    pub gave_up: Recorded<MessageGaveUp>,
    /// `MessageFailed`.
    pub failed: Recorded<MessageFailed>,
    /// `MessageSuppressed`.
    pub suppressed: Recorded<MessageSuppressed>,
    /// `AutomationFailed`.
    pub automation_failed: Recorded<AutomationFailed>,
    /// `ContactPointInvalidated`.
    pub invalidated: Recorded<ContactPointInvalidated>,
    /// The party directory the app uses; it records the invalidations.
    pub directory: TestDirectory,
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

    /// A `type_key` message about ticket `ticket` (labelled `label`) to
    /// Sara M., whose e-mail and SMS both fail for good (`550 mailbox
    /// unknown`, `504 timeout`); runs the jobs and returns the
    /// `MessageGaveUp` it ended in.
    pub async fn give_up_on_sara(&self, type_key: &str, ticket: i64, label: &str) -> MessageGaveUp {
        self.give_up_on(SARA, type_key, ticket, label).await
    }

    /// As [`give_up_on_sara`](Self::give_up_on_sara), for any `party`
    /// with an e-mail and a mobile.
    pub async fn give_up_on(
        &self,
        party: PartyId,
        type_key: &str,
        ticket: i64,
        label: &str,
    ) -> MessageGaveUp {
        self.email
            .then([Err(SendError::permanent("550", "mailbox unknown"))]);
        self.sms.then([Err(SendError::permanent("504", "timeout"))]);
        let before = self.gave_up.all().len();
        let request = NotifyRequest::new(type_key, Subject::new("ticketing.ticket", ticket))
            .subject_label(label)
            .to(NotifyRecipient::Party { id: party });
        let state = self.app.state();
        self.app
            .at_travelled_time(notify::notifier(state).notify(state, request))
            .await
            .expect("the notifier queues the message");
        self.app.run_all_jobs().await;
        let events = self.gave_up.all();
        assert_eq!(events.len(), before + 1, "the message gave up");
        events.last().cloned().expect("an event")
    }

    /// A user holding `role` in `scope` only; with `None`, everywhere.
    pub async fn holder(&self, name: &str, role: &str, scope: Option<&Scope>) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(self.db(), name, &email, "password123")
            .await
            .unwrap();
        permissions::define_role(self.db(), role, &[])
            .await
            .unwrap();
        match scope {
            Some(scope) => user.assign_role_in(self.db(), role, scope).await.unwrap(),
            None => user.assign_role(self.db(), role).await.unwrap(),
        }
        user
    }

    /// `user`'s database notifications of `kind`, newest first, as JSON text.
    pub async fn notices(&self, user: &User, kind: &str) -> Vec<String> {
        user.notifications(self.db(), 50)
            .await
            .unwrap()
            .into_iter()
            .filter(|n| n.kind == kind)
            .map(|n| n.data.to_string())
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
    boot_with(&[], true).await
}

/// As [`boot`], but the `staff_task` reaction is the module's own, with the
/// recording task creator behind it: what the end-to-end tests run.
pub async fn boot_staff() -> Booted {
    boot_with(&[GiveUpReaction::StaffTask], true).await
}

/// As [`boot`], but the `mark_contact_invalid` reaction is the module's own,
/// with the test directory behind it (#58).
pub async fn boot_invalid() -> Booted {
    boot_with(&[GiveUpReaction::MarkContactInvalid], true).await
}

/// As [`boot_staff`], with the module's own `alternative_recipient` too.
pub async fn boot_alternative() -> Booted {
    boot_with(
        &[
            GiveUpReaction::StaffTask,
            GiveUpReaction::AlternativeRecipient,
        ],
        true,
    )
    .await
}

/// As [`boot_staff`], with the `ticketing` module off: no task creator.
pub async fn boot_without_tickets() -> Booted {
    boot_with(&[GiveUpReaction::StaffTask], false).await
}

async fn boot_with(real: &[GiveUpReaction], ticketing: bool) -> Booted {
    register_types();
    let tasks = Tasks::default();
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
        if real.contains(&reaction) {
            continue;
        }
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
    let test_directory = TestDirectory::default();
    let directory: Arc<dyn PartyDirectory> = Arc::new(test_directory.clone());
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(module)
        .provide(directory);
    let app = if ticketing {
        let creator: Arc<dyn TaskCreator> = Arc::new(tasks.clone());
        app.provide(creator)
    } else {
        app
    };
    let app = record(app, &gave_up);
    let app = record(app, &failed);
    let app = record(app, &suppressed);
    let app = record(app, &automation_failed);
    let invalidated = Recorded::new();
    let app = record(app, &invalidated);
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
        tasks,
        gave_up,
        failed,
        suppressed,
        automation_failed,
        invalidated,
        directory: test_directory,
    }
}
