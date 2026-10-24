//! BL-MFG-12 (#393) with the real dispatch, invoicing, documents and tickets:
//! `MFG.RUL.08` warns the accountant at day 60 of an export dispatch with no
//! exit evidence, flags it *5% VAT due* at day 90 with one task that names the
//! zero-rated invoices, stops when evidence is attached, keeps the flag and
//! tells the accountant when evidence comes late, runs once for a day, and
//! alerts the owner when it fails or does not run.

mod manufacturing_support;

use host::roles::{ACCOUNTANT, ADMIN, STOREKEEPER};
use manufacturing::exports::{self, BILL_OF_LADING, ExportWatch, task_key};
use manufacturing_support::{
    Floor, boot, browse, dispatch_of, document, floor, messages_to, person, post, stocked,
};
use renox::chrono::Duration;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_documents::DocumentStatus;
use wl_inventory::Dispatch;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory, invoicer};
use wl_kernel::tickets::task_creator;
use wl_kernel::{PartyId, PartyKind, Subject};
use wl_parties::{CompanyDetails, NewParty, create};

async fn customer(app: &TestApp) -> PartyId {
    create(
        app.state(),
        NewParty {
            kind: PartyKind::Company,
            legal_name_en: "Gulf Export FZE".into(),
            company: Some(CompanyDetails::default()),
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .party_id()
}

/// DN-5: a dispatch of panels to an export customer that left today, and the
/// issued invoice that bills it at the zero rate for AED 40,000.
async fn exported(app: &TestApp, f: &Floor, company: PartyId) -> (Dispatch, String) {
    let keeper = person(app, "Karim", STOREKEEPER, None).await;
    let admin = person(app, "Ada", ADMIN, None).await;
    let panels = stocked(app, "EXP-PANEL", &f.finished, 100).await;
    let dispatch = dispatch_of(app, &panels, &f.finished, 50, company.0, 10).await;
    post(
        app,
        &keeper,
        &format!("/dispatches/{}/pick", dispatch.id),
        &[],
    )
    .await
    .assert_ok();
    post(
        app,
        &keeper,
        &format!("/dispatches/{}/dispatch", dispatch.id),
        &[("handover", "courier"), ("tracking_number", "TRK-5")],
    )
    .await
    .assert_ok();
    let lines = wl_inventory::dispatch::lines_of(app.db(), dispatch.id)
        .await
        .unwrap();
    let mut draft = InvoiceDraft::new(company, dispatch.subject());
    for line in &lines {
        draft = draft.line(
            DraftLine::new("Panels", 4_000_000, VatCategory::ZeroRated).origin(line.subject()),
        );
    }
    let id = invoicer(app.state())
        .draft(app.db(), draft)
        .await
        .unwrap()
        .expect("invoicing is on");
    let issued = post(app, &admin, &format!("/invoices/{}/issue", id.0), &[]).await;
    assert_eq!(issued.status.as_u16(), 303, "{}", issued.text());
    let invoice = wl_invoicing::Invoice::find_or_404(app.db(), id.0)
        .await
        .unwrap();
    let dispatch = Dispatch::find_or_404(app.db(), dispatch.id).await.unwrap();
    (dispatch, invoice.number.unwrap_or_default())
}

async fn watch_of(app: &TestApp, dispatch: &Dispatch) -> ExportWatch {
    ExportWatch::query()
        .where_(ExportWatch::DISPATCH_ID.eq(dispatch.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("the dispatch is watched")
}

fn after(days: i64) -> renox::chrono::NaiveDate {
    today() + Duration::days(days)
}

/// Done when 1, 2 and 6: DN-5 left today with INV zero-rated AED 40,000 and no
/// evidence. On day 60 the accountant is told; on day 90 it is flagged *5% VAT
/// due* and the accountant has one task that names the invoice and its amount;
/// running the rule again on day 90 creates no second task and no second
/// notice; the exports page says *5% VAT due*.
#[renox::test]
async fn the_accountant_is_warned_at_day_60_and_given_one_task_at_day_90() {
    let app = boot().await;
    let f = floor(&app).await;
    let accountant = person(&app, "Alia", ACCOUNTANT, None).await;
    let company = customer(&app).await;
    let (dispatch, invoice) = exported(&app, &f, company).await;
    let notice = manufacturing::notices::NOTICE;

    // Day 59: nothing. Day 60: the warning.
    let early = exports::run(app.state(), after(59), 60, 90).await.unwrap();
    assert_eq!(early, exports::Done::default());
    assert!(messages_to(&app, notice, &accountant).await.is_empty());
    let day60 = exports::run(app.state(), after(60), 60, 90).await.unwrap();
    assert_eq!((day60.warned, day60.flagged), (1, 0));
    let told = messages_to(&app, notice, &accountant).await;
    assert_eq!(told.len(), 1, "the accountant is warned once");
    assert!(
        format!("{:?}", told[0]).contains("DSP"),
        "the warning names the dispatch: {:?}",
        told[0]
    );
    assert!(!watch_of(&app, &dispatch).await.vat_due());

    // Day 90: flagged, with the task.
    let day90 = exports::run(app.state(), after(90), 60, 90).await.unwrap();
    assert_eq!((day90.warned, day90.flagged), (0, 1));
    let watch = watch_of(&app, &dispatch).await;
    assert!(watch.vat_due());
    assert!(watch.invoices.contains(&invoice), "{}", watch.invoices);
    assert!(
        watch.invoices.contains("AED 40,000.00"),
        "{}",
        watch.invoices
    );
    let creator = task_creator(app.state());
    let task = creator
        .find_open(app.state(), &task_key(dispatch.id))
        .await
        .unwrap()
        .expect("the accountant has a task");
    let ticket = wl_ticketing::Ticket::find_or_404(app.db(), task.id.0)
        .await
        .unwrap();
    assert!(
        ticket.description.contains(&invoice),
        "{}",
        ticket.description
    );
    assert!(ticket.description.contains("AED 40,000.00"));
    assert_eq!(ticket.assignee_user_id, Some(accountant.id));

    // The same day again changes nothing.
    let again = exports::run(app.state(), after(90), 60, 90).await.unwrap();
    assert_eq!(again, exports::Done::default());
    assert_eq!(ExportWatch::query().count(app.db()).await.unwrap(), 1);
    let tasks = wl_ticketing::Ticket::query()
        .where_(wl_ticketing::Ticket::SUBJECT_TYPE.eq("inventory.dispatch"))
        .count(app.db())
        .await
        .unwrap();
    assert_eq!(tasks, 1, "one task");

    // The page, as the accountant.
    let page = browse(&app, &accountant, "/manufacturing/exports").await;
    page.assert_ok();
    // The "None" choice of a grid filter is the grid's own label, not a value.
    let html = page
        .text()
        .replace("<option value=\"\">None</option>", "")
        .replace("<option value=\"\" selected>None</option>", "");
    for word in ["5% VAT due", "Attach evidence", invoice.as_str()] {
        assert!(html.contains(word), "the page says `{word}`: {html}");
    }
    assert!(!html.contains("None"), "{html}");
}

/// Done when 3: a bill of lading attached before day 60 stops the rule: no
/// warning, no flag, no task at day 60 or 90.
#[renox::test]
async fn evidence_attached_stops_the_rule() {
    let app = boot().await;
    let f = floor(&app).await;
    let accountant = person(&app, "Alia", ACCOUNTANT, None).await;
    let company = customer(&app).await;
    let (dispatch, _invoice) = exported(&app, &f, company).await;
    document(
        &app,
        &dispatch.subject(),
        BILL_OF_LADING,
        None,
        DocumentStatus::Accepted,
    )
    .await;

    let day60 = exports::run(app.state(), after(60), 60, 90).await.unwrap();
    let day90 = exports::run(app.state(), after(90), 60, 90).await.unwrap();

    assert_eq!(day60, exports::Done::default());
    assert_eq!(day90, exports::Done::default());
    assert!(
        messages_to(&app, manufacturing::notices::NOTICE, &accountant)
            .await
            .is_empty()
    );
    assert!(!watch_of(&app, &dispatch).await.vat_due());
    assert!(
        task_creator(app.state())
            .find_open(app.state(), &task_key(dispatch.id))
            .await
            .unwrap()
            .is_none()
    );
}

/// A late run fires the steps whose day has passed, 60 and then 90 in the same
/// run; evidence attached after the flag keeps the flag and tells the
/// accountant once that it came after the correction.
#[renox::test]
async fn a_late_run_fires_both_steps_and_late_evidence_keeps_the_flag() {
    let app = boot().await;
    let f = floor(&app).await;
    let accountant = person(&app, "Alia", ACCOUNTANT, None).await;
    let company = customer(&app).await;
    let (dispatch, _invoice) = exported(&app, &f, company).await;

    let late = exports::run(app.state(), after(95), 60, 90).await.unwrap();
    assert_eq!((late.warned, late.flagged), (1, 1));

    document(
        &app,
        &dispatch.subject(),
        "airway_bill",
        None,
        DocumentStatus::Accepted,
    )
    .await;
    let received = exports::run(app.state(), after(96), 60, 90).await.unwrap();
    assert_eq!(received.late, 1);
    let again = exports::run(app.state(), after(97), 60, 90).await.unwrap();
    assert_eq!(again.late, 0, "told once");

    assert!(
        watch_of(&app, &dispatch).await.vat_due(),
        "nothing is reversed"
    );
    let told = messages_to(&app, manufacturing::notices::NOTICE, &accountant).await;
    assert!(
        told.iter()
            .any(|m| format!("{m:?}").contains("Evidence received after correction")),
        "{told:?}"
    );
}

/// Done when 4 and 5: the rule is compliance-critical. When the daily run fails
/// on its five attempts the company admin and the owner role get the alert and
/// a task; the rerun that succeeds closes it. A day with no run is found by the
/// check for missed runs, once.
#[renox::test]
async fn a_failed_or_missed_run_alerts_the_owner() {
    use wl_kernel::automation::RunOutcome;
    use wl_kernel::compliance::service::{check_missed, recipients};
    use wl_kernel::compliance::{AlertStatus, ComplianceAlert};
    let app = boot().await;
    let f = floor(&app).await;
    let admin = person(&app, "Adam", ADMIN, None).await;
    let company = customer(&app).await;
    let _ = exported(&app, &f, company).await;
    let rule = wl_kernel::compliance::rule(exports::RULE).expect("registered");
    assert!(
        matches!(rule.schedule(), Some(wl_kernel::compliance::Schedule::DailyAt(at)) if at.as_str() == exports::RUN_AT),
        "the daily run is watched"
    );

    // The rule cannot store what it follows: the run fails.
    renox::db::sql("DROP TABLE manufacturing_export_watches")
        .execute(app.db())
        .await
        .unwrap();
    let day = today();
    for wait in [60_u64, 300, 900, 3600] {
        let outcome = app
            .at_travelled_time(exports::run_daily(app.state(), day))
            .await
            .unwrap();
        assert_eq!(outcome, RunOutcome::Retrying);
        app.travel(std::time::Duration::from_secs(wait));
    }
    let outcome = app
        .at_travelled_time(exports::run_daily(app.state(), day))
        .await
        .unwrap();
    assert_eq!(outcome, RunOutcome::Failed);

    let alerts = ComplianceAlert::query().get(app.db()).await.unwrap();
    assert_eq!(alerts.len(), 1, "one alert");
    assert_eq!(alerts[0].rule, exports::RULE);
    let told = recipients(app.db(), &alerts[0].owner_role).await.unwrap();
    assert!(told.contains(&admin.id), "the company admin is alerted");
    assert!(alerts[0].ticket_id.is_some(), "the owner role has a task");

    // The table is back and the run is queued again: it succeeds and closes
    // the alert.
    renox::db::sql(
        "CREATE TABLE manufacturing_export_watches (id INTEGER PRIMARY KEY AUTOINCREMENT, dispatch_id INTEGER NOT NULL, number TEXT NOT NULL, customer_id INTEGER, dispatched_on TEXT NOT NULL, warned_on TEXT, flagged_on TEXT, evidence_on TEXT, late_told INTEGER NOT NULL DEFAULT 0, task_id INTEGER, invoices TEXT NOT NULL DEFAULT '', created_at TEXT, updated_at TEXT)",
    )
    .execute(app.db())
    .await
    .unwrap();
    let run = wl_kernel::automation::RuleRun::query()
        .where_(wl_kernel::automation::RuleRun::RULE.eq(exports::RULE))
        .first(app.db())
        .await
        .unwrap()
        .expect("the run");
    // An operator's `rerun` queues the failed run again.
    wl_kernel::automation::RuleRun::query()
        .where_(wl_kernel::automation::RuleRun::ID.eq(run.id))
        .update(
            app.db(),
            &[
                ("status", &"queued"),
                ("attempts", &0_i64),
                ("next_attempt_at", &Option::<renox::db::DateTime>::None),
            ],
        )
        .await
        .unwrap();
    let outcome = app
        .at_travelled_time(exports::run_daily(app.state(), day))
        .await
        .unwrap();
    assert_eq!(outcome, RunOutcome::Succeeded);
    let closed = ComplianceAlert::find_or_404(app.db(), alerts[0].id)
        .await
        .unwrap();
    assert_eq!(closed.status, AlertStatus::Closed);

    // A night that never ran is found by the check, once.
    let before = ComplianceAlert::query().count(app.db()).await.unwrap();
    app.travel(std::time::Duration::from_secs(3 * 24 * 3600));
    let found = app
        .at_travelled_time(check_missed(app.state()))
        .await
        .unwrap();
    assert!(found >= 1, "a missed night is found");
    let after_first = ComplianceAlert::query().count(app.db()).await.unwrap();
    assert!(after_first > before);
    let again = app
        .at_travelled_time(check_missed(app.state()))
        .await
        .unwrap();
    assert_eq!(again, 0, "the same occurrence is not alerted twice");
}

/// The customer's contract note is written by the accountant and shown on the
/// task of the flag.
#[renox::test]
async fn the_task_shows_the_contract_note_of_the_customer() {
    let app = boot().await;
    let f = floor(&app).await;
    let accountant = person(&app, "Alia", ACCOUNTANT, None).await;
    let company = customer(&app).await;
    let (dispatch, _invoice) = exported(&app, &f, company).await;
    post(
        &app,
        &accountant,
        "/manufacturing/exports/notes",
        &[
            ("party_id", &company.0.to_string()),
            ("note", "VAT is recharged to the customer"),
        ],
    )
    .await
    .assert_status(303);

    exports::run(app.state(), after(90), 60, 90).await.unwrap();

    let task = task_creator(app.state())
        .find_open(app.state(), &task_key(dispatch.id))
        .await
        .unwrap()
        .expect("a task");
    let ticket = wl_ticketing::Ticket::find_or_404(app.db(), task.id.0)
        .await
        .unwrap();
    assert!(
        ticket
            .description
            .contains("VAT is recharged to the customer"),
        "{}",
        ticket.description
    );
    let _ = Subject::new("inventory.dispatch", dispatch.id);
}
