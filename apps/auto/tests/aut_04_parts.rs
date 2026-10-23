//! BL-AUT-04 (#400): the legal parts supply period and the substitute vehicle
//! (`AUT.RUL.16`, compliance-critical): the wait for parts records its start and
//! its legal supply date; the daily rule tells the advisor, the customer and
//! the branch manager once each, asks for a substitute vehicle on warranty
//! repairs, and alerts the owner when a run fails or is missing.

mod support;

use std::time::Duration;

use auto::job_card_machine::PARTS_ARRIVED;
use auto::job_cards::{self, Delivery};
use auto::parts_delays::{self, DELAY_TYPE, NOTICE_TYPE, RULE};
use auto::parts_wait::PartsWait;
use auto::substitute::{self, Substitute};
use auto::{JobCard, JobCardStatus, PartLine, supply};
use renox::auth::permissions;
use renox::chrono::{Duration as Days, NaiveDate};
use renox::db::Json;
use renox::prelude::*;
use support::{Garage, Messages, at, boot, day, run_as_system};
use wl_kernel::compliance::{AlertKind, ComplianceAlert, check_missed, occurrence_key};
use wl_kernel::documents::today;
use wl_kernel::notify::{NotifyChannel, NotifyRecipient, NotifyRequest};
use wl_kernel::{COMPANY_ADMIN_ROLE, Subject};

/// The users a request is for.
fn users_of(request: &NotifyRequest) -> Vec<i64> {
    request
        .recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect()
}

/// A user holding `role` (and the grants), registered once.
async fn holder(t: &Garage, name: &str, role: &str, grants: &[&str]) -> User {
    let email = format!("{}@example.com", name.to_lowercase());
    if let Some(found) = User::find_by_email(t.db(), &email).await.unwrap() {
        return found;
    }
    permissions::define_role(t.db(), role, grants)
        .await
        .unwrap();
    let user = User::register(t.db(), name, &email, "password123")
        .await
        .unwrap();
    user.assign_role(t.db(), role).await.unwrap();
    user
}

/// The branch manager Basma.
async fn manager(t: &Garage) -> User {
    holder(
        t,
        "Basma",
        auto::permissions::BRANCH_MANAGER_ROLE,
        &[auto::permissions::JOB_CARDS_APPROVE],
    )
    .await
}

/// JC-40: `in_progress`, advised by Sami, with an approved part from `source`
/// not issued yet.
async fn jc(t: &Garage, source: &str) -> (JobCard, PartLine) {
    let vehicle = t.hilux().await;
    let sami = t.advisor_user().await;
    let card = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(t.db(), &[("advisor_user_id", &Some(sami.id))])
        .await
        .unwrap();
    let part = t
        .approved_part(&card, "Gearbox mount", None, 1_000, 30_000)
        .await;
    PartLine::query()
        .where_(PartLine::ID.eq(part.id))
        .update(t.db(), &[("source", &source.to_owned())])
        .await
        .unwrap();
    (t.reload(card.id).await, t.part_now(part.id).await)
}

/// The branch manager sets the import period to 14 days.
async fn imports_take_14_days(t: &Garage) {
    let basma = manager(t).await;
    let auth = t.auth(&basma).await;
    supply::set_period(t.state(), &auth, "import", 14)
        .await
        .unwrap();
}

/// Moves the clock to the Asia/Dubai instant `to`.
fn go_to(t: &Garage, to: renox::db::DateTime) {
    t.app.travel_back();
    let ahead = (to - renox::db::now()).to_std().expect("ahead");
    t.app.travel(Duration::from_secs(ahead.as_secs() + 1));
}

/// JC-40 enters `awaiting_parts` for an imported part with a 14-day period: the
/// legal date is the start plus 14 days (not editable); two days before, the
/// advisor and the customer are told; on the legal date the branch manager is.
/// Each step once.
#[renox::test]
async fn aut_04_the_legal_date_is_the_start_plus_the_period_and_the_notices_fire_once() {
    let t = boot().await;
    imports_take_14_days(&t).await;
    let (card, _) = jc(&t, "import").await;
    let basma = manager(&t).await;
    let sami = t.advisor_user().await;
    let tareq = t.technician("Tareq").await;

    let waiting = job_cards::wait_parts(t.state(), &tareq, card.id)
        .await
        .unwrap();

    let legal = today() + Days::days(14);
    assert_eq!(waiting.status, JobCardStatus::AwaitingParts);
    assert_eq!(waiting.legal_supply_date, Some(legal));
    assert!(waiting.parts_wait_started_at.is_some());
    let wait = PartsWait::open_of(t.db(), card.id).await.unwrap().unwrap();
    assert_eq!((wait.legal_date, wait.source.as_str()), (legal, "import"));

    // Before the N days: nothing.
    parts_delays::run_daily(t.state(), legal - Days::days(3))
        .await
        .unwrap();
    assert!(t.messages.sent().is_empty());

    // Two days before: the advisor and the customer.
    parts_delays::run_daily(t.state(), legal - Days::days(2))
        .await
        .unwrap();
    let notices = t.messages.of_type(NOTICE_TYPE);
    assert_eq!(notices.len(), 1);
    assert!(Messages::told_user(&notices[0], sami.id));
    let delays = t.messages.of_type(DELAY_TYPE);
    assert_eq!(delays.len(), 1);
    assert!(Messages::told_party(&delays[0], 7));
    let sms = Messages::text(&delays[0], NotifyChannel::Sms);
    assert!(sms.contains("we will confirm the date"), "{sms}");
    assert!(sms.contains("Dubai A 12345"), "{sms}");
    assert_eq!(
        t.tickets.with_key("auto.parts_date:").len(),
        1,
        "no date was entered: the advisor gets a task"
    );

    // Again, and the day after: nothing more; the manager is not told yet.
    parts_delays::run_daily(t.state(), legal - Days::days(2))
        .await
        .unwrap();
    parts_delays::run_daily(t.state(), legal - Days::days(1))
        .await
        .unwrap();
    assert_eq!(t.messages.sent().len(), 2);
    assert_eq!(t.tickets.with_key("auto.parts_date:").len(), 1);

    // On the legal date: the branch manager.
    parts_delays::run_daily(t.state(), legal).await.unwrap();
    let notices = t.messages.of_type(NOTICE_TYPE);
    assert_eq!(notices.len(), 2);
    assert_eq!(users_of(&notices[1]), [basma.id]);
    parts_delays::run_daily(t.state(), legal + Days::days(1))
        .await
        .unwrap();
    assert_eq!(t.messages.sent().len(), 3, "each step fires once");
}

/// The new date the customer is told is the expected arrival date the purchaser
/// entered on the part; with it the advisor has no task to find it out.
#[renox::test]
async fn aut_04_the_customer_is_told_the_expected_date_of_the_part() {
    let t = boot().await;
    let (card, part) = jc(&t, "stock").await;
    let tareq = t.technician("Tareq").await;
    let sami = t.advisor().await;
    job_cards::wait_parts(t.state(), &tareq, card.id)
        .await
        .unwrap();
    let expected = day(9);
    auto::parts::set_expected(t.state(), &sami, part.id, Some(expected))
        .await
        .unwrap();

    parts_delays::run_daily(t.state(), today() + Days::days(5))
        .await
        .unwrap();

    let delays = t.messages.of_type(DELAY_TYPE);
    assert_eq!(delays.len(), 1);
    let sms = Messages::text(&delays[0], NotifyChannel::Sms);
    assert!(sms.contains(&expected.to_string()), "{sms}");
    assert!(t.tickets.with_key("auto.parts_date:").is_empty());
}

/// A customer with no mobile number is not texted: the advisor gets a task to
/// call.
#[renox::test]
async fn aut_04_a_customer_without_a_mobile_number_gets_a_call_from_the_advisor() {
    let t = boot().await;
    let (card, _) = jc(&t, "stock").await;
    t.people.set_no_mobile(7, true);
    let tareq = t.technician("Tareq").await;
    job_cards::wait_parts(t.state(), &tareq, card.id)
        .await
        .unwrap();

    parts_delays::run_daily(t.state(), today() + Days::days(5))
        .await
        .unwrap();

    assert!(t.messages.of_type(DELAY_TYPE).is_empty());
    let calls = t.tickets.with_key("auto.call_customer:");
    assert_eq!(calls.len(), 1);
    assert!(calls[0].ticket.title.contains("Mona Hassan"));
    assert_eq!(
        t.messages.of_type(NOTICE_TYPE).len(),
        1,
        "the advisor is told"
    );
}

/// Parts from a 7-day source and a 30-day source: the legal date uses 7 days.
#[renox::test]
async fn aut_04_the_strictest_source_gives_the_legal_date() {
    let t = boot().await;
    let (card, _) = jc(&t, "stock").await;
    t.approved_part(&card, "Turbo", None, 1_000, 90_000).await;
    PartLine::query()
        .where_(PartLine::JOB_CARD_ID.eq(card.id))
        .where_(PartLine::DESCRIPTION.eq("Turbo"))
        .update(t.db(), &[("source", &"import".to_owned())])
        .await
        .unwrap();
    let tareq = t.technician("Tareq").await;
    // A source that is not one is refused when the part is drafted.
    let sami = t.advisor().await;
    let bad = job_cards::add_part(
        t.state(),
        &sami,
        card.id,
        job_cards::NewPart::new("Wiper", 1_000, 500).from_source("moon"),
    )
    .await
    .unwrap_err();
    assert!(
        bad.to_string().contains("where the part comes from"),
        "{bad}"
    );

    let waiting = job_cards::wait_parts(t.state(), &tareq, card.id)
        .await
        .unwrap();

    assert_eq!(waiting.legal_supply_date, Some(today() + Days::days(7)));
    let wait = PartsWait::open_of(t.db(), card.id).await.unwrap().unwrap();
    assert_eq!(wait.source, "stock");
}

/// Waiting needs a parts request: an approved part not issued yet.
#[renox::test]
async fn aut_04_waiting_for_parts_needs_a_parts_request() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    let tareq = t.technician("Tareq").await;
    let refused = job_cards::wait_parts(t.state(), &tareq, card.id)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("parts request"), "{refused}");
    assert_eq!(t.reload(card.id).await.status, JobCardStatus::InProgress);
}

/// A wait that ends starts no step; each wait is its own row and the earlier
/// ones stay in history.
#[renox::test]
async fn aut_04_each_wait_has_its_own_dates_and_an_ended_wait_fires_nothing() {
    let t = boot().await;
    let (card, _) = jc(&t, "stock").await;
    let tareq = t.technician("Tareq").await;
    let parked = job_cards::wait_parts(t.state(), &tareq, card.id)
        .await
        .unwrap();
    let resumed = run_as_system(&t, &parked, PARTS_ARRIVED, Default::default())
        .await
        .unwrap();
    assert_eq!(resumed.status, JobCardStatus::InProgress);
    parts_delays::run_daily(t.state(), today() + Days::days(30))
        .await
        .unwrap();
    assert!(
        t.messages.sent().is_empty(),
        "the wait ended: nothing fires"
    );

    job_cards::wait_parts(t.state(), &tareq, card.id)
        .await
        .unwrap();
    let waits = PartsWait::of_job_card(t.db(), card.id).await.unwrap();
    assert_eq!(waits.len(), 2);
    assert!(
        waits[0].ended_at.is_some(),
        "the earlier wait stays in history"
    );
    assert!(waits[1].ended_at.is_none());
}

/// JC-41 (warranty ticket linked) `awaiting_parts` and JC-42 (warranty ticket)
/// still `in_progress` with no parts wait, both checked in 8 days ago: the
/// advisor has a substitute vehicle task, once; a job card with no warranty
/// ticket, and a job card that is `ready`, get none.
#[renox::test]
async fn aut_04_a_warranty_repair_gets_a_substitute_vehicle_task_after_seven_days() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let eight_days_ago = at(day(-8), 9, 0);
    let mut cards = Vec::new();
    for status in [
        JobCardStatus::AwaitingParts,
        JobCardStatus::InProgress,
        JobCardStatus::InProgress,
        JobCardStatus::Ready,
    ] {
        let card = t.card_in(&vehicle, status).await;
        JobCard::query()
            .where_(JobCard::ID.eq(card.id))
            .update(t.db(), &[("created_at", &Some(eight_days_ago))])
            .await
            .unwrap();
        cards.push(card);
    }
    // 41, 42 and the ready one have a warranty ticket; the third has none.
    for index in [0, 1, 3] {
        t.tickets.add(
            &cards[index].subject(),
            "warranty",
            50 + index as i64,
            "TKT-W",
            "open",
        );
    }
    let six_days_ago = at(day(-6), 9, 0);
    let young = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    JobCard::query()
        .where_(JobCard::ID.eq(young.id))
        .update(t.db(), &[("created_at", &Some(six_days_ago))])
        .await
        .unwrap();
    t.tickets
        .add(&young.subject(), "warranty", 60, "TKT-Y", "open");

    parts_delays::run_daily(t.state(), today()).await.unwrap();

    let tasks = t.tickets.with_key("auto.substitute:");
    let mut subjects: Vec<Subject> = tasks.iter().map(|o| o.ticket.subject.clone()).collect();
    subjects.sort_by_key(|s| s.subject_id);
    assert_eq!(subjects, [cards[0].subject(), cards[1].subject()]);
    assert_eq!(
        tasks[0].ticket.source_key.as_deref(),
        Some(substitute::task_key(cards[0].id).as_str())
    );
    // Once per job card.
    parts_delays::run_daily(t.state(), today() + Days::days(1))
        .await
        .unwrap();
    assert_eq!(t.tickets.with_key("auto.substitute:").len(), 2);
    // The sixth day: not yet; the seventh: yes.
    parts_delays::run_daily(t.state(), today() + Days::days(1))
        .await
        .unwrap();
    assert_eq!(t.tickets.with_key("auto.substitute:").len(), 2);
    parts_delays::run_daily(t.state(), dubai_plus(six_days_ago, 7))
        .await
        .unwrap();
    assert_eq!(t.tickets.with_key("auto.substitute:").len(), 3);
}

/// The day `days` after the Asia/Dubai date of `from`.
fn dubai_plus(from: renox::db::DateTime, days: i64) -> NaiveDate {
    wl_kernel::documents::dubai_date(from) + Days::days(days)
}

/// Recording the substitute (plate "Dubai A 12345" from a day) closes the task;
/// `deliver` is refused until "to" is filled. A refusal with a reason closes the
/// task too; a refusal without a reason closes nothing.
#[renox::test]
async fn aut_04_the_substitute_closes_the_task_and_blocks_the_delivery_until_it_is_back() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let card = t.card_in(&vehicle, JobCardStatus::Ready).await;
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(
            t.db(),
            &[
                ("created_at", &Some(at(day(-8), 9, 0))),
                (
                    "qc_checks",
                    &Json(
                        auto::job_card::QC_KEYS
                            .iter()
                            .map(|key| auto::job_card::QcCheck {
                                key: (*key).to_owned(),
                                done: true,
                            })
                            .collect::<Vec<_>>(),
                    ),
                ),
            ],
        )
        .await
        .unwrap();
    // The warranty task is for a job card not yet ready: ask for it directly.
    t.tickets
        .add(&card.subject(), "warranty", 70, "TKT-W", "open");
    let row = substitute::request(t.state(), &t.reload(card.id).await)
        .await
        .unwrap();
    assert!(row.is_open(), "asked for and not answered");
    let blocked = job_cards::deliver(
        t.state(),
        &sami,
        card.id,
        Delivery {
            delivered_to: "Mona Hassan".to_owned(),
            ..Delivery::default()
        },
    )
    .await
    .unwrap_err();
    assert!(
        blocked.to_string().contains("substitute vehicle"),
        "{blocked}"
    );

    let lent = substitute::record(t.state(), &sami, card.id, "Dubai A 12345", day(-1), None)
        .await
        .unwrap();
    assert_eq!(lent.plate.as_deref(), Some("Dubai A 12345"));
    let still = job_cards::deliver(
        t.state(),
        &sami,
        card.id,
        Delivery {
            delivered_to: "Mona Hassan".to_owned(),
            ..Delivery::default()
        },
    )
    .await
    .unwrap_err();
    assert!(still.to_string().contains("substitute vehicle"), "{still}");
    let early = substitute::return_vehicle(t.state(), &sami, card.id, day(-3))
        .await
        .unwrap_err();
    assert!(early.to_string().contains("before the day"), "{early}");
    substitute::return_vehicle(t.state(), &sami, card.id, day(0))
        .await
        .unwrap();
    let delivered = job_cards::deliver(
        t.state(),
        &sami,
        card.id,
        Delivery {
            delivered_to: "Mona Hassan".to_owned(),
            ..Delivery::default()
        },
    )
    .await
    .expect("the car is back: delivered");
    assert_eq!(delivered.status, JobCardStatus::Delivered);
    assert!(
        Substitute::of_job_card(t.db(), card.id)
            .await
            .unwrap()
            .unwrap()
            .to_date
            .is_some()
    );
}

/// The task is closed only by the record or the refusal with a reason.
#[renox::test]
async fn aut_04_only_a_record_or_a_refusal_with_a_reason_closes_the_task() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let card = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(t.db(), &[("created_at", &Some(at(day(-8), 9, 0)))])
        .await
        .unwrap();
    t.tickets
        .add(&card.subject(), "warranty", 71, "TKT-W", "open");
    parts_delays::run_daily(t.state(), today()).await.unwrap();
    let key = substitute::task_key(card.id);
    assert_eq!(t.tickets.with_key(&key).len(), 1);
    assert!(t.tickets.with_key(&key)[0].is_open());

    let blank = substitute::refuse(t.state(), &sami, card.id, "  ")
        .await
        .unwrap_err();
    assert!(blank.to_string().contains("reason"), "{blank}");
    assert!(
        t.tickets.with_key(&key)[0].is_open(),
        "an offer by word closes nothing"
    );

    substitute::refuse(t.state(), &sami, card.id, "The customer has a second car")
        .await
        .unwrap();
    let task = &t.tickets.with_key(&key)[0];
    assert!(!task.is_open());
    assert_eq!(
        task.completed.as_deref(),
        Some("The customer has a second car")
    );
    assert!(
        !Substitute::of_job_card(t.db(), card.id)
            .await
            .unwrap()
            .unwrap()
            .is_open()
    );
}

/// A run of `AUT.RUL.16` that ends `failed` (the notifier is down) alerts the
/// owner and the company admin once, after the fifth failure, and opens an
/// owner task; the step is done once the notifier works again.
#[renox::test]
async fn aut_04_a_failed_run_alerts_the_owner_and_the_company_admin() {
    let t = boot().await;
    let owner = holder(&t, "Omar Owner", auto::permissions::OWNER_ROLE, &[]).await;
    let admin = holder(&t, "Aisha Admin", COMPANY_ADMIN_ROLE, &[]).await;
    imports_take_14_days(&t).await;
    let (card, _) = jc(&t, "import").await;
    let tareq = t.technician("Tareq").await;
    job_cards::wait_parts(t.state(), &tareq, card.id)
        .await
        .unwrap();
    let legal = today() + Days::days(14);
    t.messages.fail(true);
    t.messages
        .always_works(wl_kernel::compliance::service::NOTIFICATION_TYPE);
    assert!(wl_kernel::compliance::is_critical(RULE));

    for wait in [60_u64, 300, 900, 3600] {
        t.app
            .at_travelled_time(parts_delays::run_daily(t.state(), legal - Days::days(2)))
            .await
            .unwrap();
        assert!(
            ComplianceAlert::open(t.db()).await.unwrap().is_empty(),
            "no alert before the fifth failure"
        );
        t.app.travel(Duration::from_secs(wait));
    }
    t.app
        .at_travelled_time(parts_delays::run_daily(t.state(), legal - Days::days(2)))
        .await
        .unwrap();

    let alerts: Vec<_> = ComplianceAlert::open(t.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.rule == RULE)
        .collect();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0].kind, AlertKind::Failed);
    assert_eq!(alerts[0].owner_role, auto::permissions::OWNER_ROLE);
    let told = t
        .messages
        .of_type(wl_kernel::compliance::service::NOTIFICATION_TYPE);
    assert_eq!(told.len(), 1);
    let mut recipients = users_of(&told[0]);
    recipients.sort_unstable();
    let mut wanted = vec![owner.id, admin.id];
    wanted.sort_unstable();
    assert_eq!(recipients, wanted, "the owner and the company admin");
    assert!(
        t.tickets
            .tasks()
            .iter()
            .any(|task| task.assignee_user() == Some(owner.id)),
        "the owner has a task"
    );

    // The notifier works again: the next day's run does the step, once.
    t.messages.fail(false);
    parts_delays::run_daily(t.state(), legal - Days::days(1))
        .await
        .unwrap();
    assert_eq!(t.messages.of_type(NOTICE_TYPE).len(), 1);
}

/// A day with no run: at 08:30 the owner is not alerted; from 09:00 (60 minutes
/// after 08:00) the kernel's check raises the alert for that day's occurrence.
#[renox::test]
async fn aut_04_a_missed_run_alerts_the_owner_from_the_09_00_check() {
    let t = boot().await;
    let owner = holder(&t, "Omar Owner", auto::permissions::OWNER_ROLE, &[]).await;
    let tomorrow = day(1);
    let occurrence = occurrence_key(RULE, tomorrow);
    let missed = |alerts: &[ComplianceAlert]| {
        alerts
            .iter()
            .filter(|a| a.occurrence_key == occurrence)
            .count()
    };

    go_to(&t, at(tomorrow, 8, 30));
    t.app
        .at_travelled_time(check_missed(t.state()))
        .await
        .unwrap();
    assert_eq!(missed(&ComplianceAlert::open(t.db()).await.unwrap()), 0);

    go_to(&t, at(tomorrow, 9, 0));
    t.app
        .at_travelled_time(check_missed(t.state()))
        .await
        .unwrap();
    let alerts = ComplianceAlert::open(t.db()).await.unwrap();
    assert_eq!(missed(&alerts), 1, "{alerts:?}");
    assert!(alerts.iter().all(|a| a.kind == AlertKind::Missed));
    assert_eq!(alerts[0].owner_role, auto::permissions::OWNER_ROLE);
    assert!(
        t.messages.sent().iter().any(|r| r.type_key
            == wl_kernel::compliance::service::NOTIFICATION_TYPE
            && users_of(r).contains(&owner.id)),
        "the owner is told"
    );
}

/// JC-40 with no run on the 13th and 14th: the run of the 15th sends the N-day
/// notice and then the legal-date notice to the branch manager.
#[renox::test]
async fn aut_04_a_late_run_fires_every_step_whose_day_has_passed_in_order() {
    let t = boot().await;
    imports_take_14_days(&t).await;
    let (card, _) = jc(&t, "import").await;
    let basma = manager(&t).await;
    let tareq = t.technician("Tareq").await;
    job_cards::wait_parts(t.state(), &tareq, card.id)
        .await
        .unwrap();
    let legal = today() + Days::days(14);

    parts_delays::run_daily(t.state(), legal).await.unwrap();

    let sent = t.messages.sent();
    let types: Vec<&str> = sent.iter().map(|r| r.type_key.as_str()).collect();
    assert_eq!(types, [NOTICE_TYPE, DELAY_TYPE, NOTICE_TYPE]);
    assert_eq!(users_of(&sent[2]), [basma.id], "last: the branch manager");
}

/// The job card shows the wait's start and the legal supply date, in words, and
/// no field to edit them.
#[renox::test]
async fn aut_04_the_page_shows_the_start_and_the_legal_date_and_no_field_to_edit_them() {
    let t = boot().await;
    let (card, _) = jc(&t, "stock").await;
    let tareq = t.technician("Tareq").await;
    let waiting = job_cards::wait_parts(t.state(), &tareq, card.id)
        .await
        .unwrap();
    t.app.acting_as(&t.advisor_user().await);
    let page = t.app.get(&format!("/auto/job-cards/{}", card.id)).await;
    page.assert_ok()
        .assert_see("Legal supply date")
        .assert_see("Waiting for parts since")
        .assert_see(&waiting.legal_supply_date.unwrap().to_string())
        .assert_dont_see("legal_supply_date");
}
