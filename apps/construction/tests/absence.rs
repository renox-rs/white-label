//! BL-CON-15 (#294): `CON.RUL.08` opens an absence case for HR when a worker
//! assigned to a project has no attendance and no approved leave for 3
//! consecutive working days of the site, tells the supervisor, reminds HR at 7
//! days that `report_absconding` may be requested, and stops counting when the
//! worker is seen again.

mod support;

use construction::absence::{self, ABSENCE_CATEGORY, AbsenceCase};
use construction::{Project, ProjectStatus};
use renox::prelude::*;
use renox::state_machine::TransitionRecord;
use wl_attendance::WorkLocation;

use support::{Booted, boot, date, users_of};

/// A project with a site, a supervisor, an HR user and worker 1 assigned to
/// the project, present on Friday 2027-02-26 and Saturday 2027-02-27.
struct Scene {
    project: Project,
    site: WorkLocation,
    supervisor: User,
    hr: User,
}

async fn scene(t: &Booted, saturdays: bool) -> Scene {
    let pm = t.project_manager("Amal").await;
    let supervisor = t.named("Sami", "supervisor").await;
    let hr = t.named("Hana", "hr").await;
    let project = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    if saturdays {
        Project::query()
            .where_(Project::ID.eq(project.id))
            .update(t.db(), &[("works_saturdays", &true)])
            .await
            .unwrap();
    }
    let site = t.work_location(&project).await;
    t.staff.employ(1);
    t.staff.assign(1, project.subject());
    t.present(1, &site, date(2027, 2, 26)).await;
    t.present(1, &site, date(2027, 2, 27)).await;
    Scene {
        project,
        site,
        supervisor,
        hr,
    }
}

async fn run(t: &Booted, day: renox::chrono::NaiveDate) -> absence::Run {
    t.on(day, async {
        absence::run_on(t.state(), day).await.unwrap()
    })
    .await
}

fn absence_tickets(t: &Booted) -> Vec<support::Opened> {
    t.tasks
        .all()
        .into_iter()
        .filter(|opened| opened.ticket.category_key == ABSENCE_CATEGORY)
        .collect()
}

/// Done when 1: no attendance on Mon 2027-03-01, Tue and Wed and no leave: on
/// 2027-03-04 an absence ticket opens for HR and the supervisor is notified;
/// two days are not enough, and a second run opens nothing more.
#[renox::test]
async fn three_working_days_without_attendance_open_an_absence_case() {
    let t = boot().await;
    let s = scene(&t, false).await;

    assert!(run(&t, date(2027, 3, 3)).await.opened.is_empty());
    assert!(absence_tickets(&t).is_empty());

    let first = run(&t, date(2027, 3, 4)).await;
    assert_eq!(first.opened.len(), 1);
    let again = run(&t, date(2027, 3, 4)).await;
    assert!(again.opened.is_empty());

    let tickets = absence_tickets(&t);
    assert_eq!(tickets.len(), 1);
    assert!(tickets[0].ticket.title.contains("3 working days"));
    assert!(tickets[0].ticket.description.contains("2027-02-27"));
    let case = &AbsenceCase::query().get(t.db()).await.unwrap()[0];
    assert_eq!(case.employee_id, 1);
    assert_eq!(case.project_id, s.project.id);
    assert_eq!(case.last_present_on, date(2027, 2, 27));

    let told = t.notices.notices_titled("has not come for 3 working days");
    assert_eq!(told.len(), 1);
    assert_eq!(users_of(&told[0]), [s.supervisor.id]);

    // wl-hr is told, counting from the last attended day.
    let opened = t.staff.opened();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].employee.0, 1);
    assert_eq!(opened[0].opened_on, date(2027, 3, 4));
    assert_eq!(opened[0].last_present_on, case.last_present_on);
    assert_eq!(opened[0].case.subject_type, "ticketing.ticket");
}

/// Done when 2: on a Monday to Saturday site, still absent through Monday
/// 2027-03-08 (7 working days, Sunday 2027-03-07 not counted), HR is reminded
/// on 2027-03-09, once; nothing is reported automatically.
#[renox::test]
async fn hr_is_reminded_at_seven_working_days_and_the_sunday_does_not_count() {
    let t = boot().await;
    let s = scene(&t, true).await;

    for day in 4..=8 {
        let run = run(&t, date(2027, 3, day)).await;
        assert_eq!(run.opened.len(), usize::from(day == 4));
        assert!(run.reminded.is_empty(), "day {day}");
    }
    assert!(t.notices.notices_titled("report_absconding").is_empty());

    let nine = run(&t, date(2027, 3, 9)).await;
    assert_eq!(nine.reminded.len(), 1);
    assert!(run(&t, date(2027, 3, 9)).await.reminded.is_empty());
    assert!(run(&t, date(2027, 3, 10)).await.reminded.is_empty());
    let reminders = t.notices.notices_titled("report_absconding");
    assert_eq!(reminders.len(), 1);
    assert_eq!(users_of(&reminders[0]), [s.hr.id]);
    assert!(
        reminders[0].data["title"]
            .as_str()
            .unwrap()
            .contains("7 working days")
    );
    // One case only; the rule never reports a worker as absconded.
    assert_eq!(absence_tickets(&t).len(), 1);
}

/// Done when 3: approved leave on those days gives no case.
#[renox::test]
async fn a_worker_on_approved_leave_gets_no_case() {
    let t = boot().await;
    let _s = scene(&t, false).await;
    t.staff.approve_leave(1, date(2027, 3, 1), date(2027, 3, 3));

    let run = run(&t, date(2027, 3, 4)).await;
    assert!(run.opened.is_empty());
    assert!(absence_tickets(&t).is_empty());
    assert!(t.staff.opened().is_empty());
}

/// Done when 4: marked present on Friday 2027-03-05 the count stops: no
/// reminder at 7 days; the case stays open for HR; a new absence is a new
/// spell with its own case.
#[renox::test]
async fn attendance_recorded_again_stops_the_count() {
    let t = boot().await;
    let s = scene(&t, false).await;

    assert_eq!(run(&t, date(2027, 3, 4)).await.opened.len(), 1);
    t.present(1, &s.site, date(2027, 3, 5)).await;
    for day in 6..=10 {
        let run = run(&t, date(2027, 3, day)).await;
        assert!(
            run.opened.is_empty() && run.reminded.is_empty(),
            "day {day}"
        );
    }
    assert!(t.notices.notices_titled("report_absconding").is_empty());
    let case = AbsenceCase::query().get(t.db()).await.unwrap().remove(0);
    assert_eq!(case.ended_on, Some(date(2027, 3, 6)));
    // The ticket stays open: nothing cancelled or closed.
    assert!(absence_tickets(&t)[0].cancelled.is_none());

    // Absent again on 8, 9 and 10 March: a second spell, a second case.
    let second = run(&t, date(2027, 3, 11)).await;
    assert_eq!(second.opened.len(), 1);
    assert_eq!(absence_tickets(&t).len(), 2);
    assert_eq!(t.staff.opened().len(), 2);
    assert_eq!(t.staff.opened()[1].last_present_on, date(2027, 3, 5));
}

/// Edge case: days when the project is on hold do not count and do not break
/// the run; a worker who never attended gets no case.
#[renox::test]
async fn days_on_hold_do_not_count_and_a_worker_never_seen_gets_no_case() {
    let t = boot().await;
    let s = scene(&t, false).await;
    // Worker 2 never attended: they have not "stopped coming".
    t.staff.employ(2);
    t.staff.assign(2, s.project.subject());

    // On hold from Monday 1 March to Wednesday 3 March (resumed on the 4th).
    t.on(date(2027, 3, 1), async {
        TransitionRecord::new(Project::TABLE, s.project.id, "on_hold", "hold")
            .from("in_progress")
            .record(t.db())
            .await
            .unwrap();
    })
    .await;
    t.on(date(2027, 3, 4), async {
        TransitionRecord::new(Project::TABLE, s.project.id, "in_progress", "resume")
            .from("on_hold")
            .record(t.db())
            .await
            .unwrap();
    })
    .await;
    // Thursday 4 and Friday 5 count; the held days before them do not.
    let early = run(&t, date(2027, 3, 6)).await;
    assert!(early.opened.is_empty(), "two counted days");
    // Monday 8 March is the third counted day.
    let third = run(&t, date(2027, 3, 9)).await;
    assert_eq!(third.opened.len(), 1);
    assert_eq!(third.opened[0].employee_id, 1);
    assert_eq!(
        absence_tickets(&t).len(),
        1,
        "not the worker who never came"
    );
}
