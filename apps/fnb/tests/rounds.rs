//! BL-FNB-17 (#381), 06 F&B §4.6, §7.2 `FNB.RUL.03` and `FNB.RUL.04`,
//! `PLT.RUL.04`: the scheduled rounds of an outlet. The opening checklist is
//! opened an hour before opening and checked at opening, a unit with no
//! reading for four hours is named to the kitchen manager, and a planned run
//! that never happened is found by the kernel's check and alerts the
//! operations director; the late run closes that alert. Everything runs
//! through the scheduled jobs on the travelled clock, with the readings
//! recorded through the real route.

mod support;

use fnb::permissions as perm;
use fnb::{ColdUnit, Outlet, rounds};
use renox::prelude::*;
use wl_inventory::Location;
use wl_kernel::compliance::{AlertKind, AlertStatus, ComplianceAlert};

use support::ops::{boot_ops, day_after, dubai_at, go_to};
use support::{World, make_user};

const NOTICE: &str = "fnb.staff_notice";

/// A fridge or freezer of `outlet` that logs temperatures.
async fn unit(w: &World, outlet: &Outlet, code: &str, name: &str) -> ColdUnit {
    let mut location = Location::new(code, name);
    location.save(w.db()).await.unwrap();
    let mut unit = ColdUnit {
        code: code.to_owned(),
        name: name.to_owned(),
        kind: "fridge".to_owned(),
        outlet_id: Some(outlet.id),
        location_id: location.id,
        max_tenths: Some(50),
        active: true,
        ..ColdUnit::default()
    };
    unit.save(w.db()).await.unwrap();
    unit
}

/// The scheduled job of the rounds, run on the travelled clock.
async fn job(w: &World) {
    w.app
        .at_travelled_time(w.app.run_scheduled(rounds::TASK))
        .await
        .expect("the rounds job runs");
}

/// The kernel's check for missed runs, run on the travelled clock.
async fn check_missed(w: &World) {
    w.app
        .at_travelled_time(w.app.run_scheduled(wl_kernel::compliance::service::TASK))
        .await
        .expect("the check runs");
}

/// `logger` reads 4 degrees on `unit` at the clock of the app.
async fn read(w: &World, logger: &User, unit: &ColdUnit) {
    w.app
        .at_travelled_time(w.post(
            logger,
            &format!("/cold-units/{}/readings", unit.id),
            &[("temperature", "4")],
        ))
        .await
        .assert_status(303);
}

/// The titles of the staff notices so far.
fn titles(w: &World) -> Vec<String> {
    w.notices
        .of_type(NOTICE)
        .iter()
        .filter_map(|r| r.data["title"].as_str().map(str::to_owned))
        .collect()
}

struct Staff {
    manager: User,
    logger: User,
}

async fn staff(w: &World) -> Staff {
    Staff {
        manager: make_user(
            w.db(),
            "Mounir K",
            &[perm::COLD_UNITS_MANAGE, perm::TEMPERATURE_VIEW],
        )
        .await,
        logger: make_user(
            w.db(),
            "Lena",
            &[perm::TEMPERATURE_LOG, perm::TEMPERATURE_VIEW],
        )
        .await,
    }
}

/// `FNB.RUL.04`: Marina (open 11:00 to 02:00) has a fridge read at 13:00 and a
/// freezer never read. The check at 15:00 names the freezer only; the one at
/// 19:00 names both; nothing is named twice for the same planned run.
#[renox::test]
async fn a_unit_with_no_reading_for_four_hours_is_named_to_the_kitchen_manager() {
    let (w, o) = boot_ops().await;
    let s = staff(&w).await;
    let fridge = unit(&w, &o.marina, "M-F1", "Fridge 1").await;
    unit(&w, &o.marina, "M-Z1", "Freezer 1").await;

    go_to(&w, dubai_at(1, "13:00")).await;
    read(&w, &s.logger, &fridge).await;
    go_to(&w, dubai_at(1, "15:05")).await;
    job(&w).await;

    let gap: Vec<_> = w
        .notices
        .of_type(NOTICE)
        .into_iter()
        .filter(|r| {
            r.data["title"]
                .as_str()
                .is_some_and(|t| t.contains("No temperature reading"))
        })
        .collect();
    assert_eq!(gap.len(), 1, "{:?}", titles(&w));
    let body = gap[0].data["body"].as_str().unwrap();
    assert!(body.contains("Freezer 1"), "{body}");
    assert!(!body.contains("Fridge 1"), "{body}");
    assert!(
        w.notices.users_of_type(NOTICE).contains(&s.manager.id),
        "the kitchen manager is told"
    );

    // Running the job again changes nothing: the run exists.
    job(&w).await;
    assert_eq!(
        titles(&w)
            .iter()
            .filter(|t| t.contains("No temperature reading"))
            .count(),
        1
    );

    go_to(&w, dubai_at(1, "19:05")).await;
    job(&w).await;
    let latest = w
        .notices
        .of_type(NOTICE)
        .into_iter()
        .rfind(|r| {
            r.data["title"]
                .as_str()
                .is_some_and(|t| t.contains("No temperature reading"))
        })
        .unwrap();
    let body = latest.data["body"].as_str().unwrap();
    assert!(
        body.contains("Fridge 1") && body.contains("Freezer 1"),
        "{body}"
    );
}

/// `FNB.RUL.03`: Sharjah (opens 10:00) has a unit read before opening, Marina
/// (opens 11:00) one read and one not. The checklists are opened an hour
/// before opening; at opening Sharjah's is complete and Marina's names the
/// freezer to the kitchen manager.
#[renox::test]
async fn the_opening_checklist_is_opened_an_hour_before_and_checked_at_opening() {
    let (w, o) = boot_ops().await;
    let s = staff(&w).await;
    let s_unit = unit(&w, &o.sharjah, "S-F1", "Sharjah fridge").await;
    let m_fridge = unit(&w, &o.marina, "M-F1", "Fridge 1").await;
    unit(&w, &o.marina, "M-Z1", "Freezer 1").await;
    let day = day_after(1);

    go_to(&w, dubai_at(1, "09:05")).await;
    job(&w).await;
    let sharjah = rounds::OpeningChecklist::of_day(w.db(), o.sharjah.id, day)
        .await
        .unwrap()
        .expect("Sharjah's checklist is open an hour before 10:00");
    assert!(sharjah.completed_at.is_none());
    assert!(
        rounds::OpeningChecklist::of_day(w.db(), o.marina.id, day)
            .await
            .unwrap()
            .is_none(),
        "Marina's opens at 10:00"
    );
    assert!(
        titles(&w)
            .iter()
            .any(|t| t.contains("opening checklist of Sharjah Kitchen is open")),
        "{:?}",
        titles(&w)
    );
    assert!(w.notices.users_of_type(NOTICE).contains(&s.logger.id));

    go_to(&w, dubai_at(1, "09:30")).await;
    read(&w, &s.logger, &s_unit).await;
    go_to(&w, dubai_at(1, "10:05")).await;
    job(&w).await;
    let sharjah = rounds::OpeningChecklist::of_day(w.db(), o.sharjah.id, day)
        .await
        .unwrap()
        .unwrap();
    assert!(sharjah.completed_at.is_some(), "every unit was read");
    assert!(sharjah.notified_at.is_none());
    assert!(
        rounds::OpeningChecklist::of_day(w.db(), o.marina.id, day)
            .await
            .unwrap()
            .is_some(),
        "Marina's checklist is open"
    );

    go_to(&w, dubai_at(1, "10:30")).await;
    read(&w, &s.logger, &m_fridge).await;
    go_to(&w, dubai_at(1, "11:05")).await;
    job(&w).await;
    let marina = rounds::OpeningChecklist::of_day(w.db(), o.marina.id, day)
        .await
        .unwrap()
        .unwrap();
    assert!(marina.completed_at.is_none());
    assert!(marina.notified_at.is_some());
    assert_eq!(marina.missing, "Freezer 1");
    let missed: Vec<_> = w
        .notices
        .of_type(NOTICE)
        .into_iter()
        .filter(|r| {
            r.data["title"]
                .as_str()
                .is_some_and(|t| t.contains("opens without its checklist done"))
        })
        .collect();
    assert_eq!(missed.len(), 1);
    assert!(
        missed[0].data["body"]
            .as_str()
            .unwrap()
            .contains("Freezer 1")
    );
    assert!(w.notices.users_of_type(NOTICE).contains(&s.manager.id));
}

/// A planned run that never happened: the job did not run at 15:00, so the
/// kernel's check at 16:10 finds the run of Marina's 15:00 round missed and
/// alerts the operations director; the late run of the job closes the alert.
#[renox::test]
async fn a_round_that_never_ran_is_found_missed_and_the_late_run_closes_the_alert() {
    let (w, o) = boot_ops().await;
    staff(&w).await;
    unit(&w, &o.marina, "M-Z1", "Freezer 1").await;
    let day = day_after(1);

    go_to(&w, dubai_at(1, "16:10")).await;
    check_missed(&w).await;

    let alerts = ComplianceAlert::open(w.db()).await.unwrap();
    let marina_round = alerts
        .iter()
        .find(|a| {
            a.rule == rounds::READINGS_RULE
                && a.occurrence_key
                    .contains(&format!(":marina:readings:{day}T11:00"))
        })
        .unwrap_or_else(|| panic!("no alert for the 15:00 round: {alerts:?}"));
    assert_eq!(marina_round.kind, AlertKind::Missed);
    assert_eq!(marina_round.owner_role, "operations_director");
    assert!(
        alerts
            .iter()
            .any(|a| a.rule == rounds::OPENING_RULE && a.kind == AlertKind::Missed),
        "the opening checklists missed their runs too"
    );
    let id = marina_round.id;

    // The job runs late: the round is run, so the alert closes.
    job(&w).await;
    let closed = ComplianceAlert::find_or_404(w.db(), id).await.unwrap();
    assert_eq!(closed.status, AlertStatus::Closed);
}
