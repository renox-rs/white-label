//! Warning about work planned past a document's expiry: the acceptance
//! checks AC1-AC6 of #47.
//!
//! The guard reads the real clock, so the dates are relative to today
//! (`support::day`) and the checks hold at any real date. The planned
//! records have no table here; the checks assert that `check_planned`
//! writes nothing, so what a module saved stays saved.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_documents::Documents;
use wl_kernel::Subject;
use wl_kernel::documents::{
    self, DocumentGuard, NoDocuments, PlannedError, PlannedItem, PlannedWarning,
};

use support::{
    Booted, MONA_ATTENDANCE, PLUMBING_TICKET, PRACTITIONER, RAVI, RAVI_ATTENDANCE, TUTOR, boot,
    civil_defence, day, flagged_work_permit, practitioner_licence, ravi, residence_visa,
    tutor_clearance,
};

const ACTIVATE: &str = "deployment.activate";

async fn planned(
    t: &Booted,
    guarded: &str,
    items: &[PlannedItem],
) -> Result<Option<PlannedWarning>, PlannedError> {
    documents::guard(t.state())
        .check_planned(t.state(), guarded, items)
        .await
        .expect("the check runs")
}

/// The stored documents of `subject`, to see that a check writes nothing.
async fn snapshot(t: &Booted, subject: &Subject) -> String {
    let documents = Documents::of_subject(t.db(), subject)
        .await
        .expect("documents load");
    format!("{documents:?}")
}

/// AC1: one warning for Ravi's visa, which expires before the deployment.
#[renox::test]
async fn work_planned_past_the_expiry_gives_a_warning() {
    let t = boot().await;
    t.install(&[residence_visa()]).await;
    let rita = t.reviewer().await;
    let expiry = day(30);
    t.accepted(&rita, ravi(), "residence_visa", Some(expiry))
        .await;
    let before = snapshot(&t, &ravi()).await;

    let start = day(35);
    let warning = planned(&t, ACTIVATE, &[PlannedItem::new(RAVI, start)])
        .await
        .expect("a warning, not a refusal")
        .expect("a warning");

    assert_eq!(warning.guarded, ACTIVATE);
    assert_eq!(warning.lines.len(), 1);
    let line = &warning.lines[0];
    assert_eq!(line.subject, ravi());
    assert_eq!(line.label, "Ravi S.");
    assert_eq!(line.text, format!("Residence visa expires {expiry}"));
    assert_eq!(line.dates, vec![start]);
    assert!(
        warning
            .message()
            .contains(&format!("Residence visa expires {expiry}"))
    );
    assert_eq!(
        warning.message(),
        format!("Ravi S.: Residence visa expires {expiry}; planned: {start}")
    );
    assert_eq!(snapshot(&t, &ravi()).await, before);
}

/// AC2: the expiry date itself is still valid.
#[renox::test]
async fn work_starting_on_the_expiry_date_gives_no_warning() {
    let t = boot().await;
    t.install(&[residence_visa()]).await;
    let rita = t.reviewer().await;
    let expiry = day(30);
    t.accepted(&rita, ravi(), "residence_visa", Some(expiry))
        .await;

    let warning = planned(&t, ACTIVATE, &[PlannedItem::new(RAVI, expiry)]).await;
    assert_eq!(warning, Ok(None));
}

/// AC3: a bookable slot is never a warning; the booking refusal rests on
/// `is_valid`, which answers no.
#[renox::test]
async fn a_bookable_slot_is_not_warned_about() {
    let t = boot().await;
    t.install(&[practitioner_licence()]).await;
    let rita = t.reviewer().await;
    let practitioner = Subject::new("hr.employee", PRACTITIONER);
    t.accepted(
        &rita,
        practitioner.clone(),
        "practitioner_licence",
        Some(day(30)),
    )
    .await;

    let start = day(35);
    let error = planned(&t, "booking.book", &[PlannedItem::new(1, start)])
        .await
        .expect_err("a bookable slot is refused, not warned about");
    assert_eq!(
        error,
        PlannedError::BookableSlot {
            guarded: "booking.book".into()
        }
    );

    let validity = documents::guard(t.state())
        .is_valid(t.db(), &practitioner, "practitioner_licence", start)
        .await
        .unwrap();
    assert!(!validity.is_valid());
}

/// AC4: twelve weekly sessions give one warning with one line listing the
/// three dates past the clearance.
#[renox::test]
async fn a_batch_gives_one_warning_with_the_dates_past_the_expiry() {
    let t = boot().await;
    t.install(&[tutor_clearance()]).await;
    let rita = t.reviewer().await;
    let tutor = Subject::new("hr.employee", TUTOR);
    // Sessions 0..=8 start on or before the expiry; 9, 10 and 11 after it.
    let weeks: Vec<NaiveDate> = (0..12).map(|i| day(7 + 7 * i)).collect();
    let expiry = weeks[8];
    t.accepted(&rita, tutor.clone(), "tutor_clearance", Some(expiry))
        .await;
    let before = snapshot(&t, &tutor).await;

    let items: Vec<PlannedItem> = weeks
        .iter()
        .zip(1..)
        .map(|(on, id)| PlannedItem::new(id, *on))
        .collect();
    let warning = planned(&t, "session.start", &items)
        .await
        .unwrap()
        .expect("a warning");

    assert_eq!(warning.lines.len(), 1);
    let line = &warning.lines[0];
    assert_eq!(line.subject, tutor);
    assert_eq!(line.text, format!("Tutor clearance expires {expiry}"));
    assert_eq!(line.dates, weeks[9..].to_vec());
    assert_eq!(snapshot(&t, &tutor).await, before);
}

/// AC5: a missing document is listed for every date; flag entries and
/// entries whose condition does not match give nothing.
#[renox::test]
async fn missing_documents_are_listed_and_non_blocking_entries_are_not() {
    let t = boot().await;
    t.install(&[residence_visa(), flagged_work_permit(), civil_defence()])
        .await;

    let dates = [day(10), day(17)];
    let items: Vec<PlannedItem> = dates.iter().map(|on| PlannedItem::new(RAVI, *on)).collect();
    let warning = planned(&t, ACTIVATE, &items)
        .await
        .unwrap()
        .expect("a warning");
    assert_eq!(warning.lines.len(), 1);
    assert_eq!(warning.lines[0].text, "Residence visa missing");
    assert_eq!(warning.lines[0].dates, dates.to_vec());

    // Flag mode never stops work, so it is not warned about either.
    for attendance in [RAVI_ATTENDANCE, MONA_ATTENDANCE] {
        let warning = planned(
            &t,
            "attendance_record.accept",
            &[PlannedItem::new(attendance, day(10))],
        )
        .await;
        assert_eq!(warning, Ok(None));
    }

    // A plumbing ticket does not match the Civil Defence condition.
    let warning = planned(
        &t,
        "ticket.resolve",
        &[PlannedItem::new(PLUMBING_TICKET, day(10))],
    )
    .await;
    assert_eq!(warning, Ok(None));
}

/// AC6: the toast is a persistent warning with the message; without
/// wl-documents the check gives no warning.
#[renox::test]
async fn the_toast_carries_the_message_and_documents_off_gives_none() {
    let t = boot().await;
    t.install(&[residence_visa()]).await;
    let warning = planned(&t, ACTIVATE, &[PlannedItem::new(RAVI, day(10))])
        .await
        .unwrap()
        .expect("a warning");
    let toast = warning.toast();
    assert_eq!(toast.kind, renox::ToastKind::Warning);
    assert_eq!(toast.message, warning.message());
    assert_eq!(toast.duration, Some(0));

    let none = NoDocuments
        .check_planned(t.state(), ACTIVATE, &[PlannedItem::new(RAVI, day(10))])
        .await
        .unwrap();
    assert_eq!(none, Ok(None));
}
