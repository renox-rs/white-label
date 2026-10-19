//! The late-entry base (wl-kernel §5.13; #28 part 2): the limits per entity,
//! the late entry record and the rule runs listed for review.

use renox::auth::Permissions;
use renox::db::{DateTime, Json, now};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::late_entries::{self, fired};
use wl_kernel::{
    ClientConfig, FiredRun, LateEntry, LateEntryStatus, Limits, RuleRun, Subject,
};

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

async fn app() -> TestApp {
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(App::new().module(Auth::new()).module(Permissions), config)
        .expect("the kernel applies");
    TestApp::new(app).await
}

fn at(text: &str) -> DateTime {
    text.parse().expect("an instant")
}

#[renox::test]
async fn an_entity_without_a_row_uses_30_minutes_and_24_hours() {
    let app = app().await;
    let limits = late_entries::limits_for(app.db(), "bookings.check_in")
        .await
        .unwrap();
    assert_eq!(limits, Limits::default());
}

#[renox::test]
async fn the_limits_are_set_per_entity() {
    let app = app().await;
    let db = app.db();
    let sixty = Limits {
        flag_after_minutes: 60,
        approval_after_hours: 24,
    };
    late_entries::set_limits(db, "bookings.check_in", sixty)
        .await
        .unwrap();
    assert_eq!(
        late_entries::limits_for(db, "bookings.check_in").await.unwrap(),
        sixty
    );
    // Another entity keeps the defaults.
    assert_eq!(
        late_entries::limits_for(db, "ticketing.ticket").await.unwrap(),
        Limits::default()
    );
    // Setting them again changes the row, it does not add one.
    let twelve = Limits {
        flag_after_minutes: 45,
        approval_after_hours: 12,
    };
    late_entries::set_limits(db, "bookings.check_in", twelve)
        .await
        .unwrap();
    assert_eq!(
        late_entries::limits_for(db, "bookings.check_in").await.unwrap(),
        twelve
    );
    assert_eq!(
        late_entries::LateEntryLimits::query().count(db).await.unwrap(),
        1
    );
}

#[renox::test]
async fn limits_that_cannot_work_are_refused() {
    let app = app().await;
    for (minutes, hours) in [(0, 24), (-5, 24), (90, 1)] {
        let error = late_entries::set_limits(
            app.db(),
            "bookings.check_in",
            Limits {
                flag_after_minutes: minutes,
                approval_after_hours: hours,
            },
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _)),
            "{minutes} minutes, {hours} hours"
        );
    }
}

#[renox::test]
async fn a_late_entry_keeps_both_times_and_finds_its_record() {
    let app = app().await;
    let entry = LateEntry::create(
        app.db(),
        LateEntry {
            subject_type: "bookings.check_in".into(),
            record_id: Some(8),
            parent_type: Some("bookings.booking".into()),
            parent_id: Some(2),
            occurred_at: at("2026-11-03T05:05:00Z"),
            entered_at: at("2026-11-03T06:30:00Z"),
            minutes_late: 85,
            reason: "router down".into(),
            entered_by: 7,
            payload: Json(renox::serde_json::json!({ "room": 4 })),
            status: LateEntryStatus::Recorded,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let found = LateEntry::of_record(app.db(), "bookings.check_in", 8)
        .await
        .unwrap()
        .expect("found by its record");
    assert_eq!(found.id, entry.id);
    assert_eq!(found.occurred_at, at("2026-11-03T05:05:00Z"));
    assert_eq!(found.entered_at, at("2026-11-03T06:30:00Z"));
    assert_eq!(found.payload.0["room"], 4);
    assert_eq!(found.parent(), Some(Subject::new("bookings.booking", 2)));
    assert!(
        LateEntry::of_record(app.db(), "bookings.check_in", 9)
            .await
            .unwrap()
            .is_none()
    );
}

async fn run(app: &TestApp, rule: &str, subject: &Subject, key: &str, when: &str) -> RuleRun {
    let run = RuleRun::create(
        app.db(),
        RuleRun {
            rule: rule.into(),
            subject_type: subject.subject_type.clone(),
            subject_id: subject.subject_id,
            occurrence_key: key.into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    RuleRun::where_eq("id", run.id)
        .update(app.db(), &[("created_at", &Some(at(when)))])
        .await
        .unwrap();
    run
}

/// §5.13 step 5: the no-show of 09:15 (05:15 UTC) on booking B lies between
/// the business time 09:05 and the entry time 10:30; the runs outside the
/// window, or about other records, do not.
#[renox::test]
async fn the_runs_between_the_two_times_on_the_record_or_its_parent_are_listed() {
    let app = app().await;
    let booking = Subject::new("bookings.booking", 2);
    let other = Subject::new("bookings.booking", 3);
    let check_in = Subject::new("bookings.check_in", 8);
    let no_show = run(&app, "BKG.RUL.02", &booking, "b2", "2026-11-03T05:15:00Z").await;
    let on_record = run(&app, "BKG.RUL.05", &check_in, "c8", "2026-11-03T05:40:00Z").await;
    run(&app, "BKG.RUL.02", &booking, "early", "2026-11-03T05:04:59Z").await;
    run(&app, "BKG.RUL.02", &booking, "late", "2026-11-03T06:30:01Z").await;
    run(&app, "BKG.RUL.02", &other, "o3", "2026-11-03T05:15:00Z").await;

    let mut tx = app.db().begin().await.unwrap();
    let found = fired::find(
        &mut tx,
        "bookings.check_in",
        Some(8),
        Some(&booking),
        at("2026-11-03T05:05:00Z"),
        at("2026-11-03T06:30:00Z"),
    )
    .await
    .unwrap();
    assert_eq!(
        found,
        vec![
            FiredRun {
                run_id: no_show.id,
                rule: "BKG.RUL.02".into(),
                subject: booking.clone(),
                at: at("2026-11-03T05:15:00Z"),
            },
            FiredRun {
                run_id: on_record.id,
                rule: "BKG.RUL.05".into(),
                subject: check_in.clone(),
                at: at("2026-11-03T05:40:00Z"),
            },
        ]
    );

    // An entity that names no parent only looks at runs on itself.
    let alone = fired::find(
        &mut tx,
        "bookings.check_in",
        Some(8),
        None,
        at("2026-11-03T05:05:00Z"),
        at("2026-11-03T06:30:00Z"),
    )
    .await
    .unwrap();
    assert_eq!(alone.len(), 1);
    assert_eq!(alone[0].subject, check_in);
    let nothing = fired::find(&mut tx, "bookings.check_in", None, None, now(), now())
        .await
        .unwrap();
    assert!(nothing.is_empty());
}
