//! `HRM.RUL.13`, the midday break on marked hours (wl-attendance §5.1 step 2,
//! §5.5 point 2, issue #180): in the season at an outdoor location the break
//! of a crew marking is pre-filled with the window; a block marked without it,
//! or times that cross the window, flag `midday_break`.
//!
//! The dates are in 2027: the clock only goes forward, and 2026's season is
//! over. The season is the same every year (15 June to 15 September).

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_attendance::flags::{self, Pending};
use wl_attendance::midday::{self, MiddayWindow};
use wl_attendance::service::{self, NewRecord};
use wl_attendance::{AttendanceStatus, BreakSpan, FlagType, WorkLocation};

use support::{boot, dubai, users_of};

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

/// Monday 12 July 2027, in the season.
fn july() -> NaiveDate {
    d(2027, 7, 12)
}

fn block(
    employee: wl_kernel::workforce::EmployeeId,
    location: &WorkLocation,
    date: NaiveDate,
    by: i64,
) -> NewRecord {
    NewRecord::supervisor(
        employee,
        location.id,
        dubai(date, 7, 0),
        Some(dubai(date, 17, 0)),
        by,
    )
}

/// In the season at an outdoor location the break is pre-filled with the
/// window; out of the season and indoors it is not.
#[renox::test]
async fn att03_the_crew_marking_prefills_the_break_in_the_season_outdoors() {
    let t = boot().await;
    let outdoor = t
        .location_of(WorkLocation::named("Site 1").outdoors())
        .await;
    let indoor = t.location("Office").await;
    let prefilled = midday::prefill(t.state(), outdoor.id, july())
        .await
        .unwrap();
    assert_eq!(prefilled, Some(BreakSpan::hm((12, 30), (15, 0))));
    // The boundaries are inside the season, the days around it are not.
    for (date, inside) in [
        (d(2027, 6, 14), false),
        (d(2027, 6, 15), true),
        (d(2027, 9, 15), true),
        (d(2027, 9, 16), false),
        (d(2027, 10, 1), false),
    ] {
        let found = midday::prefill(t.state(), outdoor.id, date).await.unwrap();
        assert_eq!(found.is_some(), inside, "{date}");
    }
    assert_eq!(
        midday::prefill(t.state(), indoor.id, july()).await.unwrap(),
        None
    );
    assert!(matches!(
        midday::prefill(t.state(), 999, july()).await,
        Err(renox::prelude::Error::NotFound)
    ));
}

/// 07:00 to 17:00 with the pre-filled break kept: no flag. With the break
/// removed, every record of the crew is flagged and the supervisor and the HSE
/// officer are told.
#[renox::test]
async fn att03_a_block_without_the_break_is_flagged_and_with_it_is_not() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let hse = t.user_with("Hamza", "hse").await;
    let a = t.employee(1, &[]);
    let b = t.employee(2, &[]);
    let c = t.employee(3, &[]);
    let site = t
        .location_of(WorkLocation::named("Site 1").outdoors())
        .await;
    t.on(july(), async {
        let kept = service::record(
            t.state(),
            block(a, &site, july(), boss.id).breaks(vec![BreakSpan::hm((12, 30), (15, 0))]),
        )
        .await
        .unwrap();
        assert_eq!(kept.status, AttendanceStatus::Recorded);
        assert!(t.flags_of(&kept).await.is_empty());
        assert!(t.notices.all().is_empty());

        for employee in [b, c] {
            let record = service::record(t.state(), block(employee, &site, july(), boss.id))
                .await
                .unwrap();
            assert_eq!(record.status, AttendanceStatus::Flagged);
            assert_eq!(t.flags_of(&record).await, ["midday_break"]);
        }
        let told = t.notices.flagged("midday_break");
        assert_eq!(told.len(), 2);
        for request in &told {
            let users = users_of(request);
            assert!(users.contains(&boss.id), "the supervisor");
            assert!(users.contains(&hse.id), "the HSE officer");
        }
        // The record is saved whole and the flag is not one that holds it for
        // the reviewer.
        assert!(!FlagType::MiddayBreak.holds_for_review());
        let saved = t.records_of(b).await.remove(0);
        assert_eq!(saved.check_out, Some(dubai(july(), 17, 0)));
    })
    .await;
}

/// The marking route carries the break: kept (12:30 to 15:00) there is no
/// flag, removed (no break fields) every record of the block is flagged and the
/// supervisor and the HSE officer are told, a half break is refused.
#[renox::test]
async fn att03_the_marking_route_keeps_or_removes_the_break() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let hse = t.user_with("Hamza", "hse").await;
    let a = t.employee(1, &[]);
    let b = t.employee(2, &[]);
    let site = t
        .location_of(WorkLocation::named("Site 1").outdoors())
        .await;
    let form = |employee: i64, extra: &[(&'static str, &'static str)]| {
        let mut form = vec![
            ("employee_id", employee.to_string()),
            ("location_id", site.id.to_string()),
            ("check_in", "2027-07-12T07:00".to_owned()),
            ("check_out", "2027-07-12T17:00".to_owned()),
        ];
        form.extend(extra.iter().map(|(k, v)| (*k, (*v).to_owned())));
        form
    };
    let (tt, bb) = (&t, &boss);
    let send = move |form: Vec<(&'static str, String)>| async move {
        let pairs: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
        tt.app.acting_as(bb);
        tt.app.htmx().post("/attendance/mark", &pairs).await
    };
    t.on(july(), async {
        // Kept: the pre-filled break goes with the marking.
        send(form(a.0, &[("break_from", "12:30"), ("break_to", "15:00")]))
            .await
            .assert_ok();
        let kept = t.records_of(a).await.remove(0);
        assert_eq!(kept.status, AttendanceStatus::Recorded);
        assert_eq!(kept.breaks.0, vec![BreakSpan::hm((12, 30), (15, 0))]);
        assert!(t.notices.all().is_empty());

        // A break with one time only is refused and saves nothing.
        send(form(b.0, &[("break_from", "12:30")]))
            .await
            .assert_status(422);
        send(form(b.0, &[("break_from", "15:00"), ("break_to", "12:30")]))
            .await
            .assert_status(422);
        assert!(t.records_of(b).await.is_empty());

        // Removed: flagged, and the supervisor and the HSE officer are told.
        send(form(b.0, &[])).await.assert_ok();
        let removed = t.records_of(b).await.remove(0);
        assert_eq!(removed.status, AttendanceStatus::Flagged);
        assert!(removed.breaks.0.is_empty());
        assert_eq!(t.flags_of(&removed).await, ["midday_break"]);
        let told = t.notices.flagged("midday_break");
        assert_eq!(told.len(), 1);
        let users = users_of(&told[0]);
        assert!(users.contains(&boss.id) && users.contains(&hse.id));
    })
    .await;
}

/// The crew list page of a location and day shows the clash with the other
/// location and the pre-filled midday break; only a marker sees it.
#[renox::test]
async fn att01_att03_the_crew_page_shows_the_clashes_and_the_prefilled_break() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let nobody = t.user_with("Nadia", "reviewer").await;
    let a = t.employee(1, &[]);
    let b = t.employee(2, &[]);
    let site = t
        .location_of(WorkLocation::named("Site 1").outdoors())
        .await;
    let other = t.location("Site 2").await;
    let office = t.location("Office").await;
    t.on(july(), async {
        // A is already marked at Site 2 from 07:00 to 12:00.
        service::record(
            t.state(),
            NewRecord::supervisor(
                a,
                other.id,
                dubai(july(), 7, 0),
                Some(dubai(july(), 12, 0)),
                boss.id,
            ),
        )
        .await
        .unwrap();
        let path = |location: i64, extra: &str| {
            format!(
                "/attendance/crew?location_id={location}&date=2027-07-12&employees={},{}{extra}",
                a.0, b.0
            )
        };

        t.app.acting_as(&nobody);
        t.app.get(&path(site.id, "")).await.assert_forbidden();

        t.app.acting_as(&boss);
        let page = t.app.get(&path(site.id, "")).await;
        page.assert_ok();
        let body = page.text();
        assert!(body.contains("Site 1"), "the location: {body}");
        assert!(body.contains("Already marked at Site 2"), "{body}");
        assert!(
            body.contains("Midday break 12:30 to 15:00"),
            "the pre-filled break: {body}"
        );
        assert!(body.contains("No clash"), "B is free: {body}");

        // Hours that do not meet the other record show no clash.
        let later = t
            .app
            .get(&path(site.id, "&from=2027-07-12T13:00&to=2027-07-12T17:00"))
            .await;
        later.assert_ok();
        assert!(!later.text().contains("Already marked at Site 2"));

        // Indoors there is no break to pre-fill.
        let indoor = t.app.get(&path(office.id, "")).await;
        indoor.assert_ok();
        assert!(indoor.text().contains("No midday break applies"));
    })
    .await;
}

/// A break 12:30 to 14:00 leaves 14:00 to 15:00 worked inside the window.
#[renox::test]
async fn att03_a_partial_break_is_flagged() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let site = t
        .location_of(WorkLocation::named("Site 1").outdoors())
        .await;
    t.on(july(), async {
        let record = service::record(
            t.state(),
            block(a, &site, july(), boss.id).breaks(vec![BreakSpan::hm((12, 30), (14, 0))]),
        )
        .await
        .unwrap();
        assert_eq!(record.status, AttendanceStatus::Flagged);
        assert_eq!(t.flags_of(&record).await, ["midday_break"]);
        let note = wl_attendance::AttendanceFlag::of_record(t.db(), record.id)
            .await
            .unwrap()
            .remove(0)
            .note;
        assert!(
            note.contains("60 minutes") && note.contains("12:30"),
            "{note}"
        );
    })
    .await;
}

/// Out of the season, or at an indoor location, there is no flag.
#[renox::test]
async fn att03_out_of_season_and_indoors_nothing_is_flagged() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let b = t.employee(2, &[]);
    let c = t.employee(3, &[]);
    let site = t
        .location_of(WorkLocation::named("Site 1").outdoors())
        .await;
    let office = t.location("Office").await;
    // The day before the season starts.
    t.on(d(2027, 6, 14), async {
        let before = service::record(t.state(), block(a, &site, d(2027, 6, 14), boss.id))
            .await
            .unwrap();
        assert_eq!(before.status, AttendanceStatus::Recorded);
    })
    .await;
    // Indoors, in the season.
    t.on(july(), async {
        let indoors = service::record(t.state(), block(b, &office, july(), boss.id))
            .await
            .unwrap();
        assert_eq!(indoors.status, AttendanceStatus::Recorded);
        assert!(t.flags_of(&indoors).await.is_empty());
    })
    .await;
    // 1 October, a Friday.
    t.on(d(2027, 10, 1), async {
        let after = service::record(t.state(), block(c, &site, d(2027, 10, 1), boss.id))
            .await
            .unwrap();
        assert_eq!(after.status, AttendanceStatus::Recorded);
    })
    .await;
    assert!(t.flagged.all().is_empty());
    assert!(t.notices.all().is_empty());
}

/// The first and the last day of the season are in it.
#[renox::test]
async fn att03_the_season_boundaries_are_inclusive() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let b = t.employee(2, &[]);
    let site = t
        .location_of(WorkLocation::named("Site 1").outdoors())
        .await;
    t.on(d(2027, 6, 15), async {
        let first = service::record(t.state(), block(a, &site, d(2027, 6, 15), boss.id))
            .await
            .unwrap();
        assert_eq!(first.status, AttendanceStatus::Flagged);
    })
    .await;
    t.on(d(2027, 9, 15), async {
        let last = service::record(t.state(), block(b, &site, d(2027, 9, 15), boss.id))
            .await
            .unwrap();
        assert_eq!(last.status, AttendanceStatus::Flagged);
    })
    .await;
}

/// The window is read per scope: a scope with a window of its own is checked
/// against it, and the default is not applied to it.
#[renox::test]
async fn att03_the_window_is_read_per_scope() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let b = t.employee(2, &[]);
    let project = support::project(5);
    let mut window = MiddayWindow::new(
        Some(&project),
        d(2027, 7, 1),
        d(2027, 8, 31),
        (13, 0),
        (16, 0),
    );
    window.save(t.db()).await.unwrap();
    let scoped = t
        .location_of(WorkLocation::named("Site 5").in_scope(&project).outdoors())
        .await;
    let other = t
        .location_of(WorkLocation::named("Site 6").outdoors())
        .await;
    // The window of the project is 13:00 to 16:00.
    assert_eq!(
        midday::prefill(t.state(), scoped.id, july()).await.unwrap(),
        Some(BreakSpan::hm((13, 0), (16, 0)))
    );
    assert_eq!(
        midday::prefill(t.state(), other.id, july()).await.unwrap(),
        Some(BreakSpan::hm((12, 30), (15, 0)))
    );
    t.on(july(), async {
        // 07:00 to 12:45 is outside 13:00 to 16:00 and inside 12:30 to 15:00.
        let morning = |employee, location: &WorkLocation| {
            NewRecord::supervisor(
                employee,
                location.id,
                dubai(july(), 7, 0),
                Some(dubai(july(), 12, 45)),
                boss.id,
            )
        };
        let in_scope = service::record(t.state(), morning(a, &scoped))
            .await
            .unwrap();
        assert_eq!(in_scope.status, AttendanceStatus::Recorded);
        let default = service::record(t.state(), morning(b, &other))
            .await
            .unwrap();
        assert_eq!(default.status, AttendanceStatus::Flagged);
    })
    .await;
}

/// A record already flagged `midday_break` is not flagged again; a check-out
/// closes an open record and runs the check.
#[renox::test]
async fn att03_a_flag_is_held_once_and_a_check_out_runs_the_check() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let site = t
        .location_of(WorkLocation::named("Site 1").outdoors())
        .await;
    t.on(july(), async {
        let open = service::record(
            t.state(),
            NewRecord::supervisor(a, site.id, dubai(july(), 7, 0), None, boss.id),
        )
        .await
        .unwrap();
        assert_eq!(open.status, AttendanceStatus::Recorded, "no span yet");
        let closed = service::check_out(t.state(), open.id, dubai(july(), 17, 0))
            .await
            .unwrap();
        assert_eq!(closed.status, AttendanceStatus::Flagged);
        assert_eq!(t.flags_of(&closed).await, ["midday_break"]);

        let mut tx = t.db().begin().await.unwrap();
        let mut pending = Pending::default();
        let added = flags::add_in(&mut tx, closed.id, FlagType::MiddayBreak, "", &mut pending)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert!(!added && pending.is_empty());
        assert_eq!(t.flagged.all().len(), 1);
    })
    .await;
}
