//! BL-AUT-04 (#400, `AUT.RUL.16`) against the REAL wl-ticketing, wl-notify and
//! wl-parties of the host: the notices of a wait for parts (advisor, customer
//! SMS, branch manager), the substitute vehicle task of a warranty repair on a
//! real warranty ticket and its closing, and the alert of a day with no run.
//! The app's own tests (`apps/auto/tests/aut_04_parts.rs`) use stand-ins.

mod support;

use std::time::Duration;

use auto::job_cards::{self, NewLabour, NewPart};
use auto::parts_delays::{self, DELAY_TYPE, NOTICE_TYPE, RULE};
use auto::substitute::{self, Substitute};
use auto::{JobCard, JobCardStatus, supply};
use host::roles::{BRANCH_MANAGER, OWNER, SERVICE_ADVISOR, TECHNICIAN};
use renox::chrono::{Duration as Days, TimeDelta};
use renox::prelude::*;
use support::autofx::{World, boot, inspected_card, sent_of};
use support::tenant_with_mobile;
use wl_kernel::calendar::BusinessCalendar;
use wl_kernel::compliance::{AlertKind, ComplianceAlert, check_missed, occurrence_key};
use wl_kernel::documents::today;
use wl_kernel::tickets::{OpenTicket, task_creator};

/// The tasks of ticketing whose source key starts with `prefix`.
async fn tasks(w: &World, prefix: &str) -> Vec<wl_ticketing::Ticket> {
    wl_ticketing::Ticket::query()
        .get(w.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|t| {
            t.source_key
                .as_deref()
                .is_some_and(|k| k.starts_with(prefix))
        })
        .collect()
}

/// Moves the clock to the Asia/Dubai instant `to`.
fn go_to(w: &World, to: renox::db::DateTime) {
    w.app.travel_back();
    let ahead = (to - renox::db::now()).to_std().expect("ahead");
    w.app.travel(Duration::from_secs(ahead.as_secs() + 1));
}

/// JC-40 for a gearbox mount imported (14 days), a warranty ticket on it, check-in
/// 8 days ago: two days before the legal date the advisor and the customer
/// (SMS on a real mobile) are told and the advisor has a task for the date; on
/// the legal date the branch manager is; each once (points 1, 7). The warranty
/// repair gets the real substitute vehicle task on the 7th day (points 2, 3),
/// which the record of the car closes, and the delivery waits until it is back
/// (point 4).
#[renox::test]
async fn aut_04_real_the_notices_the_substitute_task_and_its_closing_run_on_real_tickets() {
    let w = boot().await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let tareq = w.staff("Tareq", TECHNICIAN).await;
    let (basma_user, basma) = w.staff("Basma", BRANCH_MANAGER).await;
    let owner = tenant_with_mobile(&w.app, "Mona Hassan", "+971501234567").await;
    supply::set_period(w.state(), &basma, "import", 14)
        .await
        .unwrap();
    let card = inspected_card(
        &w,
        &sami,
        &tareq,
        owner,
        "12345",
        0,
        vec![NewLabour::hourly("Fit", 1_000, 20_000)],
        false,
    )
    .await;
    job_cards::add_part(
        w.state(),
        &tareq.1,
        card.id,
        NewPart::new("Gearbox mount", 1_000, 30_000).from_source("import"),
    )
    .await
    .unwrap();
    job_cards::send_estimate(w.state(), &sami, card.id, job_cards::Promise::default())
        .await
        .unwrap();
    let round = job_cards::pending_round(w.db(), card.id)
        .await
        .unwrap()
        .unwrap();
    job_cards::approve_by_link(w.state(), &round.token, "Mona Hassan", None, true)
        .await
        .unwrap();
    let waiting = job_cards::wait_parts(w.state(), &tareq.1, card.id)
        .await
        .unwrap();
    assert_eq!(waiting.status, JobCardStatus::AwaitingParts);
    let legal = today() + Days::days(14);
    assert_eq!(waiting.legal_supply_date, Some(legal));
    let eight_days_ago = renox::db::now() - TimeDelta::days(8);
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(w.db(), &[("created_at", &Some(eight_days_ago))])
        .await
        .unwrap();
    task_creator(w.state())
        .open(
            w.state(),
            OpenTicket::new(card.subject(), "warranty", "Gearbox mount rattles"),
        )
        .await
        .unwrap()
        .expect("ticketing is on");

    parts_delays::run_daily(w.state(), legal - Days::days(3))
        .await
        .unwrap();
    assert!(
        sent_of(&w, NOTICE_TYPE).await.is_empty(),
        "nothing before N"
    );
    // The scheduled task `auto:parts-delays` (08:00 Asia/Dubai) is the same run:
    // twelve days on, it sends the notice of the day.
    assert_eq!(
        (parts_delays::TASK_AT, parts_delays::TASK_ZONE),
        ("08:00", "Asia/Dubai")
    );
    w.app.travel(Duration::from_secs(12 * 24 * 3600));
    w.app.run_scheduled(parts_delays::TASK).await.unwrap();
    w.app.travel_back();
    assert_eq!(
        sent_of(&w, NOTICE_TYPE).await.len(),
        1,
        "the task ran the rule"
    );
    assert_eq!(sent_of(&w, DELAY_TYPE).await.len(), 1);
    assert_eq!(
        tasks(&w, &substitute::task_key(card.id)).await.len(),
        1,
        "7 days after check-in a warranty repair gets the substitute task"
    );
    parts_delays::run_daily(w.state(), legal - Days::days(2))
        .await
        .unwrap();
    parts_delays::run_daily(w.state(), legal - Days::days(2))
        .await
        .unwrap();
    let notices = sent_of(&w, NOTICE_TYPE).await;
    assert_eq!(notices.len(), 1, "the advisor, once");
    assert_eq!(notices[0].user_id, Some(sami.id));
    let delays = sent_of(&w, DELAY_TYPE).await;
    assert_eq!(delays.len(), 1, "the customer, once");
    assert!(
        delays[0].rendered_body.contains("12345"),
        "{}",
        delays[0].rendered_body
    );
    assert_eq!(format!("{:?}", delays[0].channel), "Sms");
    assert_eq!(
        tasks(&w, "auto.parts_date:").await.len(),
        1,
        "no date entered: the advisor has a task"
    );
    parts_delays::run_daily(w.state(), legal).await.unwrap();
    let notices = sent_of(&w, NOTICE_TYPE).await;
    assert_eq!(notices.len(), 2);
    assert_eq!(
        notices[1].user_id,
        Some(basma_user.id),
        "the branch manager"
    );
    parts_delays::run_daily(w.state(), legal + Days::days(1))
        .await
        .unwrap();
    assert_eq!(sent_of(&w, NOTICE_TYPE).await.len(), 2, "each step once");
    assert_eq!(tasks(&w, &substitute::task_key(card.id)).await.len(), 1);

    // The task closes only with the record of the car; the delivery waits.
    let open = |w: &World| {
        let state = w.state().clone();
        let key = substitute::task_key(card.id);
        async move {
            task_creator(&state)
                .find_open(&state, &key)
                .await
                .unwrap()
                .is_some()
        }
    };
    assert!(open(&w).await);
    let row = Substitute::of_job_card(w.db(), card.id)
        .await
        .unwrap()
        .expect("asked for");
    assert!(row.is_open());
    substitute::record(w.state(), &sami, card.id, "Dubai B 777", today(), None)
        .await
        .unwrap();
    assert!(!open(&w).await, "the record closed the real task");
    let still = Substitute::of_job_card(w.db(), card.id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        still.is_open(),
        "lent and not back: still stops the delivery"
    );
    substitute::return_vehicle(w.state(), &sami, card.id, today())
        .await
        .unwrap();
    assert!(
        !Substitute::of_job_card(w.db(), card.id)
            .await
            .unwrap()
            .unwrap()
            .is_open()
    );
}

/// A day on which `AUT.RUL.16` did not run: at 08:30 nothing, from 09:00 the
/// kernel's check alerts the Owner / GM with a real message and a real task
/// (point 6, missed run).
#[renox::test]
async fn aut_04_real_a_missed_run_alerts_the_owner_with_a_message_and_a_task() {
    let w = boot().await;
    let (owner_user, _) = w.staff("Omar", OWNER).await;
    let tomorrow = today() + Days::days(1);
    let occurrence = occurrence_key(RULE, tomorrow);
    let at = |h, m| {
        BusinessCalendar::default().from_local(tomorrow.and_hms_opt(h, m, 0).expect("a time"))
    };
    let missed = |alerts: &[ComplianceAlert]| {
        alerts
            .iter()
            .filter(|a| a.occurrence_key == occurrence)
            .count()
    };

    go_to(&w, at(8, 30));
    w.app
        .at_travelled_time(check_missed(w.state()))
        .await
        .unwrap();
    assert_eq!(missed(&ComplianceAlert::open(w.db()).await.unwrap()), 0);

    go_to(&w, at(9, 0));
    w.app
        .at_travelled_time(check_missed(w.state()))
        .await
        .unwrap();
    let alerts = ComplianceAlert::open(w.db()).await.unwrap();
    assert_eq!(missed(&alerts), 1, "{alerts:?}");
    let ours = alerts
        .iter()
        .find(|a| a.occurrence_key == occurrence)
        .unwrap();
    assert_eq!(ours.kind, AlertKind::Missed);
    let told = sent_of(&w, wl_kernel::compliance::service::NOTIFICATION_TYPE).await;
    assert!(
        told.iter().any(|m| m.user_id == Some(owner_user.id)),
        "the owner is told for real"
    );
    assert!(
        wl_ticketing::Ticket::query()
            .get(w.db())
            .await
            .unwrap()
            .iter()
            .any(|t| t.assignee_user_id == Some(owner_user.id)),
        "the owner has a real task"
    );
}

/// The run of `AUT.RUL.16` fails (the notice cannot be stored): after the fifth
/// attempt the Owner / GM and the company admin are told through the real
/// notifier and the owner has a real task; the rerun by the admin closes both
/// and the notice goes out (point 6, failed run).
#[renox::test]
async fn aut_04_real_a_failed_run_alerts_the_owner_and_the_admin_and_a_rerun_closes_it() {
    use host::roles::ADMIN;
    use renox::state_machine::StateMachine;
    use wl_kernel::automation::{MAX_ATTEMPTS, RuleRun, transitions};
    use wl_kernel::compliance::schedule_subject;

    let w = boot().await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let tareq = w.staff("Tareq", TECHNICIAN).await;
    let (_, basma) = w.staff("Basma", BRANCH_MANAGER).await;
    let (owner_user, _) = w.staff("Omar", OWNER).await;
    let (admin_user, admin) = w.staff("Ada", ADMIN).await;
    let owner = tenant_with_mobile(&w.app, "Mona Hassan", "+971501234567").await;
    supply::set_period(w.state(), &basma, "import", 14)
        .await
        .unwrap();
    let card = inspected_card(
        &w,
        &sami,
        &tareq,
        owner,
        "12345",
        0,
        vec![NewLabour::hourly("Fit", 1_000, 20_000)],
        false,
    )
    .await;
    job_cards::add_part(
        w.state(),
        &tareq.1,
        card.id,
        NewPart::new("Gearbox mount", 1_000, 30_000).from_source("import"),
    )
    .await
    .unwrap();
    job_cards::send_estimate(w.state(), &sami, card.id, job_cards::Promise::default())
        .await
        .unwrap();
    let round = job_cards::pending_round(w.db(), card.id)
        .await
        .unwrap()
        .unwrap();
    job_cards::approve_by_link(w.state(), &round.token, "Mona Hassan", None, true)
        .await
        .unwrap();
    job_cards::wait_parts(w.state(), &tareq.1, card.id)
        .await
        .unwrap();
    let on = today() + Days::days(12);

    renox::db::sql(
        "CREATE TRIGGER refuse_notice BEFORE INSERT ON wl_notify_messages WHEN NEW.type_key = 'auto.parts_notice' \
         BEGIN SELECT RAISE(ABORT, 'refused for the test'); END",
    )
    .execute(w.db())
    .await
    .unwrap();
    for attempt in 0..MAX_ATTEMPTS {
        parts_delays::run_daily(w.state(), on).await.unwrap();
        if attempt + 1 < MAX_ATTEMPTS {
            assert!(
                ComplianceAlert::open(w.db())
                    .await
                    .unwrap()
                    .iter()
                    .all(|a| a.rule != RULE),
                "no alert before the fifth failure"
            );
        }
        RuleRun::query()
            .update(
                w.db(),
                &[(
                    "next_attempt_at",
                    &Some(renox::db::now() - TimeDelta::hours(1)),
                )],
            )
            .await
            .unwrap();
    }
    let alerts: Vec<_> = ComplianceAlert::open(w.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.rule == RULE)
        .collect();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0].kind, AlertKind::Failed);
    let told = sent_of(&w, wl_kernel::compliance::service::NOTIFICATION_TYPE).await;
    let recipients: Vec<_> = told.iter().filter_map(|m| m.user_id).collect();
    assert!(
        recipients.contains(&owner_user.id) && recipients.contains(&admin_user.id),
        "the owner and the company admin: {recipients:?}"
    );
    let owner_task = || async {
        wl_ticketing::Ticket::query()
            .get(w.db())
            .await
            .unwrap()
            .into_iter()
            .find(|t| t.assignee_user_id == Some(owner_user.id))
            .expect("the owner has a task")
    };
    assert!(!matches!(
        owner_task().await.status,
        wl_ticketing::TicketStatus::Closed | wl_ticketing::TicketStatus::Resolved
    ));

    renox::db::sql("DROP TRIGGER refuse_notice")
        .execute(w.db())
        .await
        .unwrap();
    let failed =
        RuleRun::find_occurrence(w.db(), RULE, &schedule_subject(), &occurrence_key(RULE, on))
            .await
            .unwrap()
            .expect("the run is kept");
    let _outcome = failed
        .transition(transitions::RERUN)
        .by(&admin)
        .run(w.state())
        .await
        .unwrap();
    parts_delays::run_daily(w.state(), on).await.unwrap();
    assert!(
        ComplianceAlert::open(w.db())
            .await
            .unwrap()
            .into_iter()
            .all(|a| a.rule != RULE),
        "the rerun closed the alert"
    );
    assert!(matches!(
        owner_task().await.status,
        wl_ticketing::TicketStatus::Closed | wl_ticketing::TicketStatus::Resolved
    ));
    assert_eq!(
        sent_of(&w, NOTICE_TYPE).await.len(),
        1,
        "the notice went out"
    );
}
