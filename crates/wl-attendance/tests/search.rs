//! Timesheets and work locations in the search index and the Related panel
//! (BL-AGT-11): each type's document is produced and kept current by the
//! timesheet's transitions, and the relations are produced (a timesheet's
//! adjustments, a work location's attendance records); attendance records
//! themselves are never indexed one by one.

mod support;

use wl_attendance::timesheet::{self, Consumer};
use wl_attendance::{AttendanceStatus, Timesheet, TimesheetStatus};
use wl_kernel::Subject;
use wl_kernel::attendance::{ATTENDANCE_SUBJECT_TYPE, WORK_LOCATION_SUBJECT_TYPE};
use wl_kernel::related::{self, RelatedGroup};
use wl_kernel::scopes::Reach;
use wl_kernel::search::{self, Access};
use wl_kernel::timesheets::TIMESHEET_SUBJECT_TYPE;

use support::{at, boot, nov};

fn count(groups: &[RelatedGroup], name: &str) -> Option<u64> {
    groups.iter().find(|g| g.name == name).map(|g| g.count)
}

#[renox::test]
async fn timesheets_and_work_locations_have_documents_and_relations() {
    let t = boot().await;
    let site = t.location("Al Quoz site").await;
    let worker = t.employee(7, &[]);
    let record = t
        .fixture(
            worker,
            &site,
            at(3, 7, 0),
            Some(at(3, 15, 0)),
            AttendanceStatus::Accepted,
        )
        .await;
    let sheet = t
        .sheet_in(
            Timesheet::regular(worker, nov(1), nov(30)),
            TimesheetStatus::Approved,
        )
        .await;
    let mut correction = Timesheet::regular(worker, nov(1), nov(30));
    correction.kind = wl_attendance::timesheet::TimesheetKind::Adjustment;
    correction.adjusts_id = Some(sheet.id);
    correction.reason = Some("Missed overtime".to_owned());
    let correction = t.sheet_in(correction, TimesheetStatus::Open).await;
    // The fixtures were inserted as the apps do; the rebuild brings them in.
    search::rebuild(t.db()).await.unwrap();

    let sheet_subject = Subject::new(TIMESHEET_SUBJECT_TYPE, sheet.id);
    let document = search::document_of(t.db(), &sheet_subject)
        .await
        .unwrap()
        .expect("the timesheet is indexed");
    assert_eq!(document.title, "Timesheet 2026-11-01 to 2026-11-30");
    assert_eq!(document.status.as_deref(), Some("approved"));
    let correction_document =
        search::document_of(t.db(), &Subject::new(TIMESHEET_SUBJECT_TYPE, correction.id))
            .await
            .unwrap()
            .unwrap();
    assert!(
        correction_document
            .title
            .starts_with("Timesheet adjustment")
    );

    // `lock` (a transition) writes the new status into the document.
    timesheet::lock(t.state(), sheet.id, Consumer::Payroll, "PAY-2026-11")
        .await
        .unwrap();
    let document = search::document_of(t.db(), &sheet_subject)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(document.status.as_deref(), Some("locked"));

    let site_subject = Subject::new(WORK_LOCATION_SUBJECT_TYPE, site.id);
    let document = search::document_of(t.db(), &site_subject)
        .await
        .unwrap()
        .expect("the work location is indexed");
    assert_eq!(document.title, "Al Quoz site");
    // Attendance records are reached through Related, never indexed.
    assert!(
        search::document_of(t.db(), &Subject::new(ATTENDANCE_SUBJECT_TYPE, record.id))
            .await
            .unwrap()
            .is_none()
    );

    let access = Access::with(
        1,
        [(
            wl_attendance::permissions::VIEW.to_owned(),
            Reach::everything(),
        )],
    );
    let groups = related::related(t.db(), &access, &sheet_subject, 1)
        .await
        .unwrap();
    assert_eq!(count(&groups, "adjustments"), Some(1));
    let groups = related::related(t.db(), &access, &site_subject, 1)
        .await
        .unwrap();
    assert_eq!(count(&groups, "records"), Some(1));
}
