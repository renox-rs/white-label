//! Checks of #28 part 5: the back office of late entries. The entry form's
//! partial asks for a reason only when the entry is late, and the card on
//! the record lists the rule runs "already fired, review". The entity is a
//! test-only check-in.

use renox::auth::{Permissions, permissions};
use renox::chrono::Duration;
use renox::db::{DateTime, Migration, now};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::testing::TestApp;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::dubai_text;
use wl_kernel::late_entries::{self, Entered, EntrySpec, web};
use wl_kernel::machines::BoxFuture;
use wl_kernel::{ClientConfig, LateEntryDef, Limits, NewRecord, Period, RuleRun, Subject};

const CHECK_IN: &str = "bookings.check_in";

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_check_ins")]
struct CheckIn {
    id: i64,
    occurred_at: DateTime,
}

const UP: &str = r#"
CREATE TABLE "test_check_ins" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    occurred_at TEXT NOT NULL
);
"#;

const DOWN: &str = r#"DROP TABLE "test_check_ins";"#;

const MIGRATIONS: &[Migration] = &[Migration::new(
    "20261019100098_create_test_late_check_ins",
    UP,
    Some(DOWN),
)];

fn create<'a>(
    tx: &'a mut renox::db::Transaction,
    new: &'a NewRecord,
) -> BoxFuture<'a, Result<i64>> {
    Box::pin(async move {
        let row = CheckIn::create(
            &mut *tx,
            CheckIn {
                occurred_at: new.occurred_at,
                ..Default::default()
            },
        )
        .await?;
        Ok(row.id)
    })
}

/// `GET /late-fields/{minutes}`: the fields for an entry whose work happened
/// that many minutes ago.
async fn fields(lang: Lang, Path(minutes): Path<i64>) -> Json<Value> {
    let found = late_entries::assess(
        Limits::default(),
        now() - Duration::minutes(minutes),
        now(),
        &Period::Open,
    );
    Json(web::fields_for(&found, &lang).unwrap_or(Value::Null))
}

/// `GET /check-ins/{id}/card`: the late-entry card of a check-in.
async fn card(State(state): State<AppState>, lang: Lang, Path(id): Path<i64>) -> Result<Json<Value>> {
    let card = web::card_for(&state.db, &Subject::new(CHECK_IN, id), &lang).await?;
    Ok(Json(card.unwrap_or(Value::Null)))
}

struct Bookings;

impl Module for Bookings {
    fn name(&self) -> &'static str {
        "test-bookings"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .get("/late-fields/{minutes}", fields)
            .get("/check-ins/{id}/card", card)
    }

    fn migrations(&self) -> &'static [Migration] {
        MIGRATIONS
    }

    fn register(&self, app: &mut Registry) {
        app.model::<CheckIn>();
        LateEntryDef::of(CHECK_IN, create)
            .business_time("checked_in_at")
            .parent("booking")
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

async fn world() -> (TestApp, User) {
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
    let db = app.db();
    permissions::define_role(db, "receptionist", &[wl_kernel::permissions::APPROVALS_VIEW])
        .await
        .unwrap();
    let anna = User::register(db, "Anna", "anna@example.com", "password123")
        .await
        .unwrap();
    anna.assign_role(db, "receptionist").await.unwrap();
    (app, anna)
}

#[renox::test]
async fn the_fields_are_asked_for_only_when_the_entry_is_late() {
    let (app, anna) = world().await;
    app.acting_as(&anna);

    app.get("/late-fields/10")
        .await
        .assert_json(Value::Null);

    let late = app.get("/late-fields/90").await;
    late.assert_ok();
    late.assert_json_path("labels.reason", "Reason it is late");
    late.assert_json_path("labels.outage", "Outage reference");

    // Beyond 24 hours the notice says that a manager must approve.
    let approval = app.get("/late-fields/1560").await;
    approval.assert_ok();
    assert!(
        approval.text().contains("A manager must approve"),
        "{}",
        approval.text()
    );
}

#[renox::test]
async fn the_card_lists_the_runs_already_fired() {
    let (app, anna) = world().await;
    app.acting_as(&anna);
    let booking = Subject::new("bookings.booking", 2);
    let no_show = RuleRun::create(
        app.db(),
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
    let fired_at = now() - Duration::minutes(75);
    RuleRun::where_eq("id", no_show.id)
        .update(app.db(), &[("created_at", &Some(fired_at))])
        .await
        .unwrap();

    let spec = EntrySpec::new(CHECK_IN, anna.id, now() - Duration::minutes(85))
        .parent(booking.clone())
        .reason("router down")
        .outage_reference("outage at branch 2");
    let Entered::Recorded { record_id, .. } =
        late_entries::enter(app.state(), spec).await.unwrap()
    else {
        panic!("expected a recorded late entry");
    };

    let res = app.get(&format!("/check-ins/{record_id}/card")).await;
    res.assert_ok();
    res.assert_json_path("status_label", "Recorded");
    res.assert_json_path("reason", "router down");
    res.assert_json_path("outage_reference", "outage at branch 2");
    res.assert_json_path("labels.review", "Already fired, review");
    res.assert_json_path(
        "runs.0.text",
        format!(
            "BKG.RUL.02 on bookings.booking:2 at {}",
            dubai_text(fired_at)
        ),
    );

    // A record that was not entered late has no card.
    app.get("/check-ins/999/card").await.assert_json(Value::Null);
}

#[renox::test]
async fn the_kernel_templates_compile() {
    let (app, _) = world().await;
    app.assert_views_compile();
}
