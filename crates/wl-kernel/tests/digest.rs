//! Acceptance checks of #33: the daily digest of voids and late entries
//! (`PLT.RUL.06`).
//!
//! The records are test-only: receipts (voided; their scope is the building,
//! found by a scope resolver) and check-ins (late entries; the check-in of a
//! booking). The Void records and the late entries are stored with times
//! inside the period of 4 November 2026: 07:00 on the 3rd to 07:00 on the
//! 4th, Asia/Dubai. Managers hold `late_entries.approve` in a building.

use std::sync::{Arc, Mutex};

use renox::auth::permissions::Scope;
use renox::auth::{Permissions, permissions};
use renox::chrono::{Duration, NaiveDate};
use renox::db::{DateTime, Json, Migration, sql};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::digest::{self, RecordInfo};
use wl_kernel::machines::BoxFuture;
use wl_kernel::notify::{
    MessageId, Notifier, NotifyFuture, NotifyRecipient, NotifyRequest, provide_notifier,
};
use wl_kernel::{
    ClientConfig, FiredRun, LateEntry, LateEntryStatus, Subject, VoidRecord, scopes,
};

const BUILDINGS: &str = "test_buildings";
const TOWER_A: i64 = 1;
const TOWER_B: i64 = 2;

const UP: &str = r#"
CREATE TABLE "test_receipts" (id INTEGER PRIMARY KEY AUTOINCREMENT, building_id INTEGER NOT NULL);
CREATE TABLE "test_check_ins" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT NOT NULL,
    confidential INTEGER NOT NULL DEFAULT 0
);
"#;

const DOWN: &str = r#"
DROP TABLE "test_check_ins";
DROP TABLE "test_receipts";
"#;

const MIGRATIONS: &[Migration] = &[Migration::new(
    "20261019100399_create_test_digest",
    UP,
    Some(DOWN),
)];

fn date() -> NaiveDate {
    "2026-11-04".parse().expect("a date")
}

fn at(text: &str) -> DateTime {
    text.parse().expect("an instant")
}

/// The scope of a receipt: its building.
fn receipt_scope<'a>(db: &'a Db, id: i64) -> BoxFuture<'a, Result<Scope>> {
    Box::pin(async move {
        let building = sql("SELECT building_id FROM test_receipts WHERE id = ?")
            .bind(id)
            .scalar::<i64>(db)
            .await?;
        Ok(Scope::new(BUILDINGS, building))
    })
}

fn building_name<'a>(_db: &'a Db, id: &'a str) -> BoxFuture<'a, Result<Option<String>>> {
    let name = match id {
        "1" => Some("Tower A".to_owned()),
        "2" => Some("Tower B".to_owned()),
        _ => None,
    };
    Box::pin(async move { Ok(name) })
}

fn check_in_info<'a>(
    tx: &'a mut renox::db::Transaction,
    id: i64,
) -> BoxFuture<'a, Result<RecordInfo>> {
    Box::pin(async move {
        let row = sql("SELECT number, confidential FROM test_check_ins WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        Ok(match row {
            Some(row) => {
                let info = RecordInfo::numbered(&row.try_get::<String>("number")?);
                if row.try_get::<i64>("confidential")? == 1 {
                    info.confidential()
                } else {
                    info
                }
            }
            None => RecordInfo::default(),
        })
    })
}

/// A recording notifier.
#[derive(Clone, Default)]
struct Notices(Arc<Mutex<Vec<NotifyRequest>>>);

impl Notices {
    /// The digests sent to user `id`.
    fn to(&self, id: i64) -> Vec<NotifyRequest> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| {
                r.type_key == digest::NOTIFICATION_TYPE
                    && r.recipients.contains(&NotifyRecipient::User { id })
            })
            .cloned()
            .collect()
    }

    fn total(&self) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.type_key == digest::NOTIFICATION_TYPE)
            .count()
    }
}

impl Notifier for Notices {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(request);
            Ok(Vec::new())
        })
    }
}

struct Site {
    notices: Notices,
}

impl Module for Site {
    fn name(&self) -> &'static str {
        "test-site"
    }

    fn migrations(&self) -> &'static [Migration] {
        MIGRATIONS
    }

    fn register(&self, app: &mut Registry) {
        provide_notifier(app, self.notices.clone());
        scopes::register("payments.receipt", receipt_scope);
        scopes::register_label(BUILDINGS, building_name);
        digest::register_info("bookings.check_in", check_in_info);
        digest::register_info("bookings.booking", |_tx, _id| {
            Box::pin(async { Ok(RecordInfo::numbered("B")) })
        });
        digest::register_rule_title("BKG.RUL.02", "no-show rule");
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
    notices: Notices,
    vera: User,
    anna: User,
    /// Manager of Tower A.
    manager_a: User,
    /// Manager of Tower B.
    manager_b: User,
    /// Manager of both towers.
    both: User,
    /// Holds a manager-like role in Tower A, without `late_entries.approve`.
    supervisor: User,
}

async fn world() -> World {
    let notices = Notices::default();
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(
        App::new()
            .module(Auth::new())
            .module(Permissions)
            .module(Site {
                notices: notices.clone(),
            }),
        config,
    )
    .expect("the kernel applies");
    let app = TestApp::new(app).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");

    let db = app.db();
    permissions::define_role(db, "manager", &[wl_kernel::permissions::LATE_ENTRIES_APPROVE])
        .await
        .unwrap();
    permissions::define_role(db, "supervisor", &[wl_kernel::permissions::APPROVALS_VIEW])
        .await
        .unwrap();
    let user = |name: &'static str| async move {
        let email = format!("{}@example.com", name.to_lowercase());
        User::register(db, name, &email, "password123").await.unwrap()
    };
    let tower = |id: i64| Scope::new(BUILDINGS, id);
    let world = World {
        vera: user("Vera").await,
        anna: user("Anna").await,
        manager_a: user("Mia").await,
        manager_b: user("Max").await,
        both: user("Bea").await,
        supervisor: user("Sam").await,
        notices,
        app,
    };
    let db = world.app.db();
    world
        .manager_a
        .assign_role_in(db, "manager", &tower(TOWER_A))
        .await
        .unwrap();
    world
        .manager_b
        .assign_role_in(db, "manager", &tower(TOWER_B))
        .await
        .unwrap();
    for id in [TOWER_A, TOWER_B] {
        world.both.assign_role_in(db, "manager", &tower(id)).await.unwrap();
    }
    world
        .supervisor
        .assign_role_in(db, "supervisor", &tower(TOWER_A))
        .await
        .unwrap();
    world
}

impl World {
    /// A receipt of `building` voided at `when` (UTC) with `reason`.
    async fn void(&self, building: i64, number: &str, when: &str, reason: &str) {
        let db = self.app.db();
        sql("INSERT INTO test_receipts (building_id) VALUES (?)")
            .bind(building)
            .execute(db)
            .await
            .unwrap();
        let id = sql("SELECT MAX(id) FROM test_receipts")
            .scalar::<i64>(db)
            .await
            .unwrap();
        VoidRecord::create(
            db,
            VoidRecord {
                subject_type: "payments.receipt".into(),
                subject_id: id,
                number: Some(number.to_owned()),
                series_key: Some("receipt".into()),
                state_before: "issued".into(),
                reason: reason.to_owned(),
                voided_by: self.vera.id,
                effects: Json(Vec::new()),
                at: at(when),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    /// A check-in of `building`, entered late at `entered` (UTC) with
    /// `reason`, whose booking had `runs` fired.
    async fn late(
        &self,
        building: i64,
        number: &str,
        confidential: bool,
        entered: &str,
        reason: &str,
        runs: Vec<FiredRun>,
    ) {
        let db = self.app.db();
        sql("INSERT INTO test_check_ins (number, confidential) VALUES (?, ?)")
            .bind(number)
            .bind(i64::from(confidential))
            .execute(db)
            .await
            .unwrap();
        let id = sql("SELECT MAX(id) FROM test_check_ins")
            .scalar::<i64>(db)
            .await
            .unwrap();
        let entered_at = at(entered);
        LateEntry::create(
            db,
            LateEntry {
                subject_type: "bookings.check_in".into(),
                record_id: Some(id),
                parent_type: Some("bookings.booking".into()),
                parent_id: Some(2),
                scope_kind: BUILDINGS.into(),
                scope_id: building.to_string(),
                occurred_at: entered_at - Duration::minutes(85),
                entered_at,
                minutes_late: 85,
                reason: reason.to_owned(),
                entered_by: self.anna.id,
                fired_runs: Json(runs),
                status: LateEntryStatus::Recorded,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    async fn run(&self) -> digest::Report {
        digest::run_digest(self.app.state(), date())
            .await
            .expect("the digest runs")
    }
}

fn no_show() -> FiredRun {
    FiredRun {
        run_id: 1,
        rule: "BKG.RUL.02".into(),
        subject: Subject::new("bookings.booking", 2),
        // 09:15 Dubai.
        at: at("2026-11-03T05:15:00Z"),
    }
}

/// AC1 and AC2: 2 voids and 3 late entries in Tower A: its manager gets one
/// digest listing the 5 records with user and reason, and under the check-in
/// of booking B the no-show of 09:15.
#[renox::test]
async fn the_manager_gets_one_digest_with_the_five_records_and_the_runs_to_review() {
    let w = world().await;
    w.void(TOWER_A, "RCP-ACME-2026-0042", "2026-11-03T10:20:00Z", "duplicate").await;
    w.void(TOWER_A, "RCP-ACME-2026-0043", "2026-11-03T11:00:00Z", "wrong party").await;
    w.late(TOWER_A, "CI-1", false, "2026-11-03T06:30:00Z", "router down", vec![no_show()]).await;
    w.late(TOWER_A, "CI-2", false, "2026-11-03T07:00:00Z", "power cut", Vec::new()).await;
    w.late(TOWER_A, "CI-3", false, "2026-11-03T08:00:00Z", "paper sheet", Vec::new()).await;

    let report = w.run().await;
    assert_eq!(report.sent, 2, "Mia, and Bea who manages both towers");

    let sent = w.notices.to(w.manager_a.id);
    assert_eq!(sent.len(), 1, "one digest");
    let text = sent[0].data["digest"].as_str().unwrap().to_owned();
    assert!(text.contains("Tower A"), "{text}");
    assert_eq!(text.lines().filter(|l| l.starts_with("- ")).count(), 5, "{text}");
    for expected in [
        "receipt RCP-ACME-2026-0042: voided by Vera, reason: duplicate",
        "receipt RCP-ACME-2026-0043: voided by Vera, reason: wrong party",
        "check in CI-1: entered by Anna, reason: router down",
        "check in CI-2: entered by Anna, reason: power cut",
        "check in CI-3: entered by Anna, reason: paper sheet",
        "already fired, review: no-show rule, booking B, 09:15",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
    // The run sits under its late entry.
    let first = text.find("check in CI-1").unwrap();
    let review = text.find("already fired, review").unwrap();
    let second = text.find("check in CI-2").unwrap();
    assert!(first < review && review < second, "{text}");
}

/// AC3: a manager-like role without `late_entries.approve` in Tower A gets
/// no digest. AC4: the manager of Tower B, with nothing in Tower B, gets
/// nothing.
#[renox::test]
async fn only_the_holders_of_the_permission_in_the_scope_with_something_get_one() {
    let w = world().await;
    w.void(TOWER_A, "RCP-ACME-2026-0042", "2026-11-03T10:20:00Z", "duplicate").await;
    w.run().await;

    assert!(w.notices.to(w.supervisor.id).is_empty());
    assert!(w.notices.to(w.manager_b.id).is_empty());
    assert_eq!(w.notices.to(w.manager_a.id).len(), 1);
    assert_eq!(w.notices.total(), 2, "Tower A's manager and the one of both");
}

/// AC5: running the rule again for the same date sends nothing new.
#[renox::test]
async fn a_second_run_for_the_same_date_sends_nothing() {
    let w = world().await;
    w.void(TOWER_A, "RCP-ACME-2026-0042", "2026-11-03T10:20:00Z", "duplicate").await;
    assert_eq!(w.run().await.sent, 2);

    let again = w.run().await;
    assert_eq!((again.sent, again.skipped, again.failed), (0, 2, 0));
    assert_eq!(w.notices.total(), 2);
}

/// A manager of several scopes gets one digest with a section per scope.
#[renox::test]
async fn a_manager_of_two_towers_gets_one_digest_with_two_sections() {
    let w = world().await;
    w.void(TOWER_B, "RCP-ACME-2026-0050", "2026-11-03T10:20:00Z", "duplicate").await;
    w.late(TOWER_A, "CI-1", false, "2026-11-03T06:30:00Z", "router down", Vec::new()).await;
    w.run().await;

    let both = w.notices.to(w.both.id);
    assert_eq!(both.len(), 1, "one digest a day");
    let text = both[0].data["digest"].as_str().unwrap().to_owned();
    let a = text.find("Tower A").expect("Tower A");
    let b = text.find("Tower B").expect("Tower B");
    assert!(a < b, "{text}");
    assert!(text.contains("CI-1") && text.contains("RCP-ACME-2026-0050"), "{text}");

    // The towers' own managers see only their tower.
    let only_a = w.notices.to(w.manager_a.id)[0].data["digest"].as_str().unwrap().to_owned();
    assert!(only_a.contains("CI-1") && !only_a.contains("RCP-ACME-2026-0050"), "{only_a}");
    let only_b = w.notices.to(w.manager_b.id)[0].data["digest"].as_str().unwrap().to_owned();
    assert!(only_b.contains("RCP-ACME-2026-0050") && !only_b.contains("CI-1"), "{only_b}");
}

/// The period is 07:00 the day before up to, not including, 07:00 on the
/// date.
#[renox::test]
async fn only_the_records_of_the_period_are_listed() {
    let w = world().await;
    // 07:00 Dubai on the 3rd is 03:00 UTC; the 4th, 03:00 UTC.
    w.void(TOWER_A, "RCP-IN-FROM", "2026-11-03T03:00:00Z", "at the start").await;
    w.void(TOWER_A, "RCP-BEFORE", "2026-11-03T02:59:59Z", "too early").await;
    w.void(TOWER_A, "RCP-AT-UNTIL", "2026-11-04T03:00:00Z", "at the end").await;
    w.void(TOWER_A, "RCP-LAST", "2026-11-04T02:59:59Z", "last second").await;
    w.run().await;

    let text = w.notices.to(w.manager_a.id)[0].data["digest"].as_str().unwrap().to_owned();
    assert!(text.contains("RCP-IN-FROM") && text.contains("RCP-LAST"), "{text}");
    assert!(!text.contains("RCP-BEFORE") && !text.contains("RCP-AT-UNTIL"), "{text}");
}

/// Content follows field visibility: a late entry on a confidential record
/// shows only its type and number.
#[renox::test]
async fn a_confidential_record_shows_only_its_type_and_number() {
    let w = world().await;
    w.late(TOWER_A, "CI-SECRET", true, "2026-11-03T06:30:00Z", "private matter", vec![no_show()])
        .await;
    w.run().await;

    let text = w.notices.to(w.manager_a.id)[0].data["digest"].as_str().unwrap().to_owned();
    assert!(text.contains("- check in CI-SECRET"), "{text}");
    assert!(!text.contains("Anna") && !text.contains("private matter"), "{text}");
    assert!(!text.contains("already fired"), "{text}");
}

/// Entries that are not recorded (waiting for approval, or discarded) are not
/// listed; an empty period sends nothing.
#[renox::test]
async fn an_entry_that_waits_for_approval_is_not_listed_and_an_empty_day_sends_nothing() {
    let w = world().await;
    let db = w.app.db();
    LateEntry::create(
        db,
        LateEntry {
            subject_type: "bookings.check_in".into(),
            scope_kind: BUILDINGS.into(),
            scope_id: TOWER_A.to_string(),
            occurred_at: at("2026-11-02T04:00:00Z"),
            entered_at: at("2026-11-03T06:30:00Z"),
            reason: "power cut".into(),
            entered_by: w.anna.id,
            status: LateEntryStatus::AwaitingApproval,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let report = w.run().await;
    assert_eq!((report.sent, report.skipped, report.failed), (0, 0, 0));
    assert_eq!(w.notices.total(), 0);
}

/// The digest is a daily task at 07:00 Asia/Dubai.
#[renox::test]
async fn the_task_runs_daily_at_seven_in_dubai() {
    let w = world().await;
    assert_eq!(digest::TASK, "digest:voids-late");
    assert_eq!(digest::TASK_AT, "07:00");
    assert_eq!(digest::TASK_ZONE, "Asia/Dubai");
    // Today's period is empty here: the task runs and sends nothing.
    w.app
        .kernel()
        .run_scheduled(digest::TASK)
        .await
        .expect("the task runs");
    assert_eq!(w.notices.total(), 0);
}
