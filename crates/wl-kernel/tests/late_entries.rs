//! Checks of #28 part 3: entering a record after the fact. The entities are
//! test-only: a check-in (business time `checked_in_at`, parent subject
//! `booking`) and a ticket (business time `reported_at`, response due 4
//! hours after it). Times are relative to now; the exact limits are checked
//! in the unit tests of `late_entries::assess`.

use renox::auth::{Permissions, permissions};
use renox::chrono::Duration;
use renox::db::{DateTime, Migration, now, sql};
use renox::prelude::*;
use renox::serde_json::json;
use renox::testing::TestApp;
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, ApprovalEnd, ApprovalRequest, ApprovalStatus};
use wl_kernel::late_entries::{self, Entered, EntrySpec};
use wl_kernel::machines::BoxFuture;
use wl_kernel::{
    ClientConfig, LateEntry, LateEntryDef, LateEntryRecorded, LateEntryStatus, Limits, NewRecord,
    Period, RuleRun, Subject,
};

const CHECK_IN: &str = "bookings.check_in";
const TICKET: &str = "ticketing.ticket";

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_check_ins")]
struct CheckIn {
    id: i64,
    occurred_at: DateTime,
    entered_at: DateTime,
    late: bool,
    late_reason: Option<String>,
    outage_reference: Option<String>,
    late_entry_id: Option<i64>,
    approval_request_id: Option<i64>,
    booking_id: Option<i64>,
    room: i64,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_tickets")]
struct Ticket {
    id: i64,
    reported_at: DateTime,
    entered_at: DateTime,
    response_due: DateTime,
    late: bool,
}

impl Ticket {
    /// Overdue from the second after the response is due.
    fn overdue_at(&self, at: DateTime) -> bool {
        at > self.response_due
    }
}

const UP: &str = r#"
CREATE TABLE "test_check_ins" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    occurred_at TEXT NOT NULL,
    entered_at TEXT NOT NULL,
    late INTEGER NOT NULL DEFAULT 0,
    late_reason TEXT,
    outage_reference TEXT,
    late_entry_id INTEGER,
    approval_request_id INTEGER,
    booking_id INTEGER,
    room INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE "test_tickets" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    reported_at TEXT NOT NULL,
    entered_at TEXT NOT NULL,
    response_due TEXT NOT NULL,
    late INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE "test_periods" (mode TEXT NOT NULL);
"#;

const DOWN: &str = r#"
DROP TABLE "test_periods";
DROP TABLE "test_tickets";
DROP TABLE "test_check_ins";
"#;

const MIGRATIONS: &[Migration] = &[Migration::new(
    "20261019100099_create_test_late_entries",
    UP,
    Some(DOWN),
)];

fn create_check_in<'a>(
    tx: &'a mut renox::db::Transaction,
    new: &'a NewRecord,
) -> BoxFuture<'a, Result<i64>> {
    Box::pin(async move {
        if let Some(id) = new.existing {
            CheckIn::where_eq("id", id)
                .update(
                    &mut *tx,
                    &[
                        ("occurred_at", &new.occurred_at),
                        ("entered_at", &new.entered_at),
                        ("late", &new.late),
                        ("late_reason", &new.reason),
                        ("late_entry_id", &new.late_entry_id),
                    ],
                )
                .await?;
            return Ok(id);
        }
        let row = CheckIn::create(
            &mut *tx,
            CheckIn {
                occurred_at: new.occurred_at,
                entered_at: new.entered_at,
                late: new.late,
                late_reason: new.reason.clone(),
                outage_reference: new.outage_reference.clone(),
                late_entry_id: new.late_entry_id,
                approval_request_id: new.approval_request_id,
                booking_id: new.parent.as_ref().map(|p| p.subject_id),
                room: new.payload["room"].as_i64().unwrap_or_default(),
                ..Default::default()
            },
        )
        .await?;
        Ok(row.id)
    })
}

fn create_ticket<'a>(
    tx: &'a mut renox::db::Transaction,
    new: &'a NewRecord,
) -> BoxFuture<'a, Result<i64>> {
    Box::pin(async move {
        // The response is due 4 hours after the business time.
        let row = Ticket::create(
            &mut *tx,
            Ticket {
                reported_at: new.occurred_at,
                entered_at: new.entered_at,
                response_due: new.occurred_at + Duration::hours(4),
                late: new.late,
                ..Default::default()
            },
        )
        .await?;
        Ok(row.id)
    })
}

fn period<'a>(
    tx: &'a mut renox::db::Transaction,
    _occurred_at: DateTime,
) -> BoxFuture<'a, Result<Period>> {
    Box::pin(async move {
        let mode = sql("SELECT mode FROM test_periods LIMIT 1")
            .scalar_optional::<String>(&mut *tx)
            .await?;
        Ok(match mode.as_deref() {
            Some("closed") => Period::Closed,
            Some("locked") => Period::Locked("The timesheet of October is locked.".into()),
            _ => Period::Open,
        })
    })
}

struct Bookings;

impl Module for Bookings {
    fn name(&self) -> &'static str {
        "test-bookings"
    }

    fn migrations(&self) -> &'static [Migration] {
        MIGRATIONS
    }

    fn register(&self, app: &mut Registry) {
        app.model::<CheckIn>().model::<Ticket>();
        LateEntryDef::of(CHECK_IN, create_check_in)
            .business_time("checked_in_at")
            .parent("booking")
            .period(period)
            .register();
        LateEntryDef::of(TICKET, create_ticket)
            .business_time("reported_at")
            .register();
    }
}

const CLIENT: &str = r#"
[client]
name = "Acme Properties LLC"
app = "property"

[modules]
enabled = []

[[legal_entities]]
code = "ACME"
name_en = "Acme Properties LLC"
default = true
"#;

struct World {
    app: TestApp,
    /// Enters records; may not approve.
    anna: User,
    /// Holds `late_entries.approve`.
    boris: User,
}

async fn world() -> World {
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(
        App::new()
            .module(Auth::new())
            .module(Permissions)
            .module(Bookings),
        config,
    )
    .expect("the kernel applies");
    let app = TestApp::new(app).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    app.fake_events();

    let db = app.db();
    permissions::define_role(
        db,
        "receptionist",
        &[wl_kernel::permissions::APPROVALS_VIEW],
    )
    .await
    .unwrap();
    permissions::define_role(
        db,
        "manager",
        &[wl_kernel::permissions::LATE_ENTRIES_APPROVE],
    )
    .await
    .unwrap();
    let user = |name: &'static str, role: &'static str| async move {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(db, name, &email, "password123")
            .await
            .unwrap();
        user.assign_role(db, role).await.unwrap();
        user
    };
    World {
        anna: user("Anna", "receptionist").await,
        boris: user("Boris", "manager").await,
        app,
    }
}

impl World {
    /// A check-in whose work happened `ago` ago.
    fn check_in(&self, ago: Duration) -> EntrySpec {
        EntrySpec::new(CHECK_IN, self.anna.id, now() - ago)
            .parent(Subject::new("bookings.booking", 2))
            .payload(json!({ "room": 4 }))
    }

    async fn enter(&self, spec: EntrySpec) -> Result<Entered> {
        late_entries::enter(self.app.state(), spec).await
    }

    async fn check_ins(&self) -> Vec<CheckIn> {
        CheckIn::query()
            .order_by(CheckIn::ID)
            .get(self.app.db())
            .await
            .unwrap()
    }

    async fn entries(&self) -> Vec<LateEntry> {
        LateEntry::query()
            .order_by(LateEntry::ID)
            .get(self.app.db())
            .await
            .unwrap()
    }

    async fn request(&self) -> ApprovalRequest {
        ApprovalRequest::query()
            .where_(ApprovalRequest::SUBJECT_TYPE.eq("kernel.late_entry"))
            .order_by_desc(ApprovalRequest::ID)
            .first(self.app.db())
            .await
            .unwrap()
            .expect("an approval request")
    }

    async fn set_period(&self, mode: &str) {
        sql("INSERT INTO test_periods (mode) VALUES (?)")
            .bind(mode)
            .execute(self.app.db())
            .await
            .unwrap();
    }
}

#[track_caller]
fn assert_refused<T: std::fmt::Debug>(result: Result<T>) -> String {
    match result {
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)) => message,
        other => panic!("expected 422, got {other:?}"),
    }
}

/// AC1: an entry 15 minutes after the business time is saved without a flag.
#[renox::test]
async fn an_entry_within_thirty_minutes_is_saved_without_a_flag() {
    let w = world().await;
    let entered = w.enter(w.check_in(Duration::minutes(15))).await.unwrap();

    assert!(matches!(entered, Entered::OnTime { .. }));
    let rows = w.check_ins().await;
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].late && rows[0].late_entry_id.is_none());
    assert!(w.entries().await.is_empty());
    w.app.assert_not_emitted::<LateEntryRecorded>();
}

/// AC2: 90 minutes late asks for a reason; without one the save is refused;
/// with "router down" it is saved, flagged, and the event is emitted.
#[renox::test]
async fn a_late_entry_needs_a_reason_and_is_flagged() {
    let w = world().await;
    assert_refused(w.enter(w.check_in(Duration::minutes(90))).await);
    assert_refused(
        w.enter(w.check_in(Duration::minutes(90)).reason("   "))
            .await,
    );
    assert!(w.check_ins().await.is_empty());
    assert!(w.entries().await.is_empty());

    let entered = w
        .enter(
            w.check_in(Duration::minutes(90))
                .reason("router down")
                .outage_reference("outage at branch 2, 08:30-10:00"),
        )
        .await
        .unwrap();
    let Entered::Recorded { record_id, entry } = entered else {
        panic!("expected a recorded late entry");
    };
    let rows = w.check_ins().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, record_id);
    assert!(rows[0].late);
    assert_eq!(rows[0].late_reason.as_deref(), Some("router down"));
    assert_eq!(rows[0].late_entry_id, Some(entry.id));
    // The record keeps the business time and the entry time apart.
    assert!(rows[0].entered_at - rows[0].occurred_at > Duration::minutes(89));

    assert_eq!(entry.status, LateEntryStatus::Recorded);
    assert_eq!(entry.record(), Some(Subject::new(CHECK_IN, record_id)));
    assert_eq!(
        entry.outage_reference.as_deref(),
        Some("outage at branch 2, 08:30-10:00")
    );
    assert!(entry.minutes_late >= 90);
    w.app.assert_emitted::<LateEntryRecorded>(|e| {
        e.subject == Subject::new(CHECK_IN, record_id)
            && e.reason == "router down"
            && e.user_id == w.anna.id
            && e.approval_request_id.is_none()
    });
}

/// AC3: 26 hours late goes to a holder of `late_entries.approve`; the
/// requester cannot approve; on approval the record exists with the flag and
/// the approval id.
#[renox::test]
async fn an_entry_beyond_24_hours_is_created_only_on_approval() {
    let w = world().await;
    let entered = w
        .enter(w.check_in(Duration::hours(26)).reason("power cut"))
        .await
        .unwrap();
    let Entered::Pending {
        entry,
        approval_request_id,
    } = entered
    else {
        panic!("expected a pending late entry");
    };
    assert_eq!(entry.status, LateEntryStatus::AwaitingApproval);
    assert!(w.check_ins().await.is_empty(), "created only on approval");
    w.app.assert_not_emitted::<LateEntryRecorded>();

    let request = w.request().await;
    assert_eq!(request.id, approval_request_id);
    assert_eq!(request.status, ApprovalStatus::Pending);
    assert_eq!(request.subject(), entry.subject());
    assert_eq!(request.requester_id, w.anna.id);
    assert_eq!(
        request.steps.0[0].permission.as_deref(),
        Some("late_entries.approve")
    );

    let state = w.app.state();
    match approvals::approve(state, request.id, &w.anna, "").await {
        Err(Error::Forbidden) => {}
        other => panic!("the requester approved her own entry: {other:?}"),
    }
    assert!(w.check_ins().await.is_empty());

    let decided = approvals::approve(state, request.id, &w.boris, "ok")
        .await
        .unwrap()
        .expect("the last step ends the request");
    assert_eq!(decided.decision, ApprovalEnd::Approved);

    let rows = w.check_ins().await;
    assert_eq!(rows.len(), 1);
    assert!(rows[0].late);
    assert_eq!(rows[0].approval_request_id, Some(request.id));
    let stored = w.entries().await.remove(0);
    assert_eq!(stored.status, LateEntryStatus::Recorded);
    assert_eq!(stored.record_id, Some(rows[0].id));
    w.app.assert_emitted::<LateEntryRecorded>(|e| {
        e.subject == Subject::new(CHECK_IN, rows[0].id)
            && e.approval_request_id == Some(request.id)
            && e.reason == "power cut"
    });
}

/// AC3: on reject nothing is created.
#[renox::test]
async fn a_rejected_late_entry_creates_nothing() {
    let w = world().await;
    w.enter(w.check_in(Duration::hours(26)).reason("power cut"))
        .await
        .unwrap();
    let request = w.request().await;

    approvals::reject(w.app.state(), request.id, &w.boris, "not credible")
        .await
        .unwrap();

    assert!(w.check_ins().await.is_empty());
    let stored = w.entries().await.remove(0);
    assert_eq!(stored.status, LateEntryStatus::Discarded);
    assert_eq!(stored.record_id, None);
    w.app.assert_not_emitted::<LateEntryRecorded>();
}

/// AC4: a closed period the module allows to reopen needs approval even two
/// hours late; a period it forbids refuses the save and stores nothing.
#[renox::test]
async fn a_closed_period_needs_approval_and_a_locked_one_refuses() {
    let w = world().await;
    w.set_period("closed").await;
    let entered = w
        .enter(w.check_in(Duration::hours(2)).reason("paper sheet"))
        .await
        .unwrap();
    assert!(entered.is_pending());
    assert!(w.check_ins().await.is_empty());

    sql("DELETE FROM test_periods")
        .execute(w.app.db())
        .await
        .unwrap();
    w.set_period("locked").await;
    let message = assert_refused(
        w.enter(w.check_in(Duration::hours(2)).reason("paper sheet"))
            .await,
    );
    assert_eq!(message, "The timesheet of October is locked.");
    assert_eq!(w.entries().await.len(), 1, "no request for the refused one");
    assert!(w.check_ins().await.is_empty());
}

/// AC5: a ticket entered 3 hours after it was reported has its response due
/// 4 hours after the business time, not after the entry time.
#[renox::test]
async fn the_business_time_decides_the_sla() {
    let w = world().await;
    let reported = now() - Duration::hours(3);
    let spec = EntrySpec::new(TICKET, w.anna.id, reported).reason("power cut");
    let Entered::Recorded { record_id, .. } = w.enter(spec).await.unwrap() else {
        panic!("expected a recorded late entry");
    };
    let ticket = Ticket::find(w.app.db(), record_id).await.unwrap().unwrap();

    assert!(ticket.late);
    assert_eq!(ticket.response_due, ticket.reported_at + Duration::hours(4));
    assert!(ticket.response_due < ticket.entered_at + Duration::hours(4));
    assert!(!ticket.overdue_at(ticket.response_due));
    assert!(ticket.overdue_at(ticket.response_due + Duration::seconds(1)));
}

/// AC6: M = 60 minutes stops flagging a 45-minute delay of that entity only.
#[renox::test]
async fn the_limit_is_set_per_entity() {
    let w = world().await;
    let sixty = Limits {
        flag_after_minutes: 60,
        approval_after_hours: 24,
    };
    late_entries::set_limits(w.app.db(), CHECK_IN, sixty)
        .await
        .unwrap();

    let entered = w.enter(w.check_in(Duration::minutes(45))).await.unwrap();
    assert!(matches!(entered, Entered::OnTime { .. }));
    // The ticket entity still has M = 30.
    let ticket = EntrySpec::new(TICKET, w.anna.id, now() - Duration::minutes(45));
    assert_refused(w.enter(ticket).await);
}

/// AC7: the no-show run of 09:15 on booking B is not undone; the late
/// check-in (business time 09:05, entered 10:30) lists it as "already fired,
/// review" and the event carries it.
#[renox::test]
async fn the_runs_already_fired_are_listed_for_review() {
    let w = world().await;
    let booking = Subject::new("bookings.booking", 2);
    let no_show = RuleRun::create(
        w.app.db(),
        RuleRun {
            rule: "BKG.RUL.02".into(),
            subject_type: booking.subject_type.clone(),
            subject_id: booking.subject_id,
            occurrence_key: "no-show".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    // 85 minutes ago the work happened; 75 minutes ago the run fired.
    let fired_at = now() - Duration::minutes(75);
    RuleRun::where_eq("id", no_show.id)
        .update(w.app.db(), &[("created_at", &Some(fired_at))])
        .await
        .unwrap();

    let entered = w
        .enter(w.check_in(Duration::minutes(85)).reason("router down"))
        .await
        .unwrap();
    let Entered::Recorded { entry, .. } = entered else {
        panic!("expected a recorded late entry");
    };

    assert_eq!(entry.fired_runs.0.len(), 1);
    assert_eq!(entry.fired_runs.0[0].run_id, no_show.id);
    assert_eq!(entry.fired_runs.0[0].rule, "BKG.RUL.02");
    assert_eq!(entry.fired_runs.0[0].subject, booking);
    // The kernel did not touch the run.
    let still = RuleRun::find(w.app.db(), no_show.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still.status, no_show.status);
    w.app.assert_emitted::<LateEntryRecorded>(|e| {
        e.fired_runs.len() == 1 && e.fired_runs[0].rule == "BKG.RUL.02"
    });
}

/// §5.13 step 2: changing the business time of an existing record is
/// checked the same way, against the time of the edit.
#[renox::test]
async fn changing_the_business_time_of_a_record_is_checked_too() {
    let w = world().await;
    let Entered::OnTime { record_id } = w.enter(w.check_in(Duration::minutes(5))).await.unwrap()
    else {
        panic!("expected an on-time entry");
    };

    let edit = |ago| w.check_in(ago).existing(record_id);
    assert_refused(w.enter(edit(Duration::hours(3))).await);
    let entered = w
        .enter(edit(Duration::hours(3)).reason("typo"))
        .await
        .unwrap();
    assert_eq!(entered.record_id(), Some(record_id));

    let rows = w.check_ins().await;
    assert_eq!(rows.len(), 1, "the record was changed, not added");
    assert!(rows[0].late && rows[0].late_reason.as_deref() == Some("typo"));
}

/// An entity that did not opt in takes no late entries.
#[renox::test]
async fn an_entity_that_did_not_opt_in_is_refused() {
    let w = world().await;
    let spec = EntrySpec::new("bookings.unknown", w.anna.id, now());
    match w.enter(spec).await {
        Err(Error::BadRequest(_)) => {}
        other => panic!("expected 400, got {other:?}"),
    }
}
