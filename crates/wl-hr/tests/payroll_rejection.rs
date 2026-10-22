//! BL-HRM-12 (#202): the bank rejects the whole SIF file (`record_rejection`):
//! the run is `rejected`, every line of the file `failed`, the preparer and
//! the approver are told and `PayrollLinesFailed` follows; the preparer fixes
//! the cause and `resubmit`s a new SIF version for the same lines with the
//! same nets, and the earlier file stays (`HRM.WPS.01`).

mod support;

use renox::prelude::*;
use wl_hr::{
    EmployeeStatus, LineStatus, PayrollLine, PayrollRun, PayrollRunStatus, SifFile, establishment,
};

use support::{Booted, users};

const NET: i64 = 300_000;

/// A prepared the run, B approved it; E and F are paid in SIF version 1.
struct Desk {
    t: Booted,
    a: User,
    b: User,
    run: PayrollRun,
    lines: Vec<PayrollLine>,
    entity: i64,
}

async fn desk() -> Desk {
    let t = support::boot().await;
    let a = t.user_with("Pia", "payroll_prep").await;
    let b = t.user_with("Ali", "payroll_appr").await;
    let e = t.employee_in(61, EmployeeStatus::Active).await;
    let f = t.employee_in(62, EmployeeStatus::Active).await;
    let entity = t.entity_id().await;
    establishment::set(t.db(), entity, "EST-1001")
        .await
        .unwrap();
    // Prepared by A and approved by B through the routes, then sent.
    let run = t.payroll_run("2026-11", PayrollRunStatus::Draft).await;
    t.payroll_line(&run, &e, NET, NET).await;
    t.payroll_line(&run, &f, NET, NET).await;
    t.transition(&a, &run, "calculate", "")
        .await
        .assert_status(303);
    t.transition(&b, &run, "approve", "")
        .await
        .assert_status(303);
    wl_hr::payroll::submit(t.state(), run.id).await.unwrap();
    let run = t.run(&run).await;
    let lines = run.lines(t.db()).await.unwrap();
    Desk {
        t,
        a,
        b,
        run,
        lines,
        entity,
    }
}

async fn files(d: &Desk) -> Vec<SifFile> {
    SifFile::of_run(d.t.db(), d.run.id).await.unwrap()
}

/// AC1: a run `submitted` with SIF v1, A records rejection "funding" with the
/// notice: the run is `rejected`, all lines `failed`, `PayrollLinesFailed`
/// emitted, the preparer and the approver notified.
#[renox::test]
async fn hrm12_ac1_a_rejection_fails_every_line_and_tells_the_preparer_and_the_approver() {
    let d = desk().await;
    assert_eq!(d.run.status, PayrollRunStatus::Submitted);
    assert_eq!(files(&d).await.len(), 1);

    d.t.reject(&d.a, &d.run, "funding", "BANK-REJ-77")
        .await
        .assert_status(303);

    let run = d.t.run(&d.run).await;
    assert_eq!(run.status, PayrollRunStatus::Rejected);
    assert_eq!(run.rejection_reason.as_deref(), Some("funding"));
    assert_eq!(run.rejection_notice.as_deref(), Some("BANK-REJ-77"));
    for line in &d.lines {
        let line = d.t.line(line).await;
        assert_eq!(line.status, LineStatus::Failed);
        assert_eq!(line.failure_reason.as_deref(), Some("funding"));
    }
    let events = d.t.failed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].run.0, d.run.id);
    assert_eq!(events[0].lines.len(), 2);
    assert!(events[0].lines.iter().all(|l| l.reason == "funding"));
    let notices = d.t.notices.of_type("hr.payroll_failed");
    assert_eq!(notices.len(), 1);
    let mut told = vec![d.a.id, d.b.id];
    told.sort_unstable();
    assert_eq!(users(&notices[0]), told);
    assert_eq!(notices[0].data["count"], 2);
}

/// AC2: A resubmits: SIF v2 exists, v1 is kept as it was, the lines are
/// `included` again with version 2, the run is `submitted`.
#[renox::test]
async fn hrm12_ac2_resubmit_makes_a_new_version_for_the_same_lines_and_keeps_the_old_one() {
    let d = desk().await;
    let before = files(&d).await.remove(0);
    d.t.reject(&d.a, &d.run, "funding", "BANK-REJ-77")
        .await
        .assert_status(303);

    d.t.resubmit(&d.a, &d.run, "Account topped up")
        .await
        .assert_status(303);

    let run = d.t.run(&d.run).await;
    assert_eq!(run.status, PayrollRunStatus::Submitted);
    assert_eq!(run.sif_version, 2);
    assert_eq!(run.resubmit_note.as_deref(), Some("Account topped up"));
    let all = files(&d).await;
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].version, 1);
    assert_eq!(all[0].content, before.content, "v1 is kept as it was");
    assert_eq!(all[0].file_name, before.file_name);
    assert_eq!(all[1].version, 2);
    assert_eq!(all[1].line_count, 2);
    assert_eq!(all[1].total, 2 * NET);
    for line in &d.lines {
        let line = d.t.line(line).await;
        assert_eq!(line.status, LineStatus::Included);
        assert_eq!(line.failure_reason, None);
        assert_eq!(line.sif_version, Some(2));
    }
    // The same two lines, with the same nets, are in both versions.
    let mut tx = d.t.db().begin().await.unwrap();
    let v1: Vec<(i64, i64)> = all[0]
        .lines_in(&mut tx)
        .await
        .unwrap()
        .iter()
        .map(|l| (l.line_id, l.net_amount))
        .collect();
    let v2: Vec<(i64, i64)> = all[1]
        .lines_in(&mut tx)
        .await
        .unwrap()
        .iter()
        .map(|l| (l.line_id, l.net_amount))
        .collect();
    assert_eq!(v1, v2);
}

/// AC3: if a line's net changed, resubmit is refused; nothing is generated.
#[renox::test]
async fn hrm12_ac3_resubmit_is_refused_when_a_net_changed() {
    let d = desk().await;
    d.t.reject(&d.a, &d.run, "funding", "BANK-REJ-77")
        .await
        .assert_status(303);
    PayrollLine::query()
        .where_(PayrollLine::ID.eq(d.lines[0].id))
        .update(d.t.db(), &[("net_amount", &(NET + 100))])
        .await
        .unwrap();

    d.t.resubmit(&d.a, &d.run, "Account topped up")
        .await
        .assert_status(422);

    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Rejected);
    assert_eq!(files(&d).await.len(), 1);
    assert_eq!(d.t.line(&d.lines[0]).await.status, LineStatus::Failed);
}

/// AC4: without the note, resubmit is refused (for every reason).
#[renox::test]
async fn hrm12_ac4_resubmit_without_a_note_is_refused() {
    let d = desk().await;
    d.t.reject(&d.a, &d.run, "file_format", "BANK-REJ-78")
        .await
        .assert_status(303);

    d.t.resubmit(&d.a, &d.run, "   ").await.assert_status(422);
    d.t.resubmit(&d.a, &d.run, "").await.assert_status(422);

    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Rejected);
    assert_eq!(files(&d).await.len(), 1);
    // Other reasons are not checked by the system: with a note it goes.
    d.t.resubmit(&d.a, &d.run, "Rebuilt in the bank's layout")
        .await
        .assert_status(303);
    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Submitted);
}

/// AC5: reason `establishment ID` with v1 named "EST-1001": resubmit is
/// refused while the entity's ID is still "EST-1001"; after it is corrected to
/// "EST-1010", v2 is generated named with "EST-1010" and v1 keeps its name.
#[renox::test]
async fn hrm12_ac5_an_establishment_id_rejection_needs_the_id_to_change() {
    let d = desk().await;
    let v1 = files(&d).await.remove(0);
    assert!(v1.file_name.contains("EST-1001"), "{}", v1.file_name);
    assert_eq!(v1.establishment_id.as_deref(), Some("EST-1001"));
    d.t.reject(&d.a, &d.run, "establishment_id", "BANK-REJ-79")
        .await
        .assert_status(303);

    d.t.resubmit(&d.a, &d.run, "Checked the ID with MoHRE")
        .await
        .assert_status(422);
    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Rejected);
    assert_eq!(files(&d).await.len(), 1);

    establishment::set(d.t.db(), d.entity, "EST-1010")
        .await
        .unwrap();
    d.t.resubmit(&d.a, &d.run, "Checked the ID with MoHRE")
        .await
        .assert_status(303);

    let all = files(&d).await;
    assert_eq!(all.len(), 2);
    assert!(
        all[1].file_name.contains("EST-1010"),
        "{}",
        all[1].file_name
    );
    assert_eq!(all[1].establishment_id.as_deref(), Some("EST-1010"));
    assert!(all[0].file_name.contains("EST-1001"), "v1 keeps its name");
}

/// AC6: without the bank's notice, `record_rejection` is refused; so is an
/// unknown reason, and a user without `payroll.prepare`.
#[renox::test]
async fn hrm12_ac6_a_rejection_needs_the_notice_a_known_reason_and_the_preparer_right() {
    let d = desk().await;

    d.t.reject(&d.a, &d.run, "funding", "  ")
        .await
        .assert_status(422);
    d.t.reject(&d.a, &d.run, "weather", "BANK-REJ-77")
        .await
        .assert_status(422);
    d.t.reject(&d.b, &d.run, "funding", "BANK-REJ-77")
        .await
        .assert_status(403);

    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Submitted);
    for line in &d.lines {
        assert_eq!(d.t.line(line).await.status, LineStatus::Included);
    }
    assert!(d.t.failed.all().is_empty());
    assert!(d.t.notices.of_type("hr.payroll_failed").is_empty());
}

/// A held line was never in the file: the rejection leaves it held and
/// resubmit does not add it. `return` is not possible once the file was sent.
#[renox::test]
async fn hrm12_a_held_line_stays_out_and_a_rejected_run_cannot_be_returned() {
    let t = support::boot().await;
    let a = t.user_with("Pia", "payroll_prep").await;
    let b = t.user_with("Ali", "payroll_appr").await;
    let e = t.employee_in(61, EmployeeStatus::Active).await;
    let g = t.employee_in(63, EmployeeStatus::Active).await;
    let run = t.payroll_run("2026-11", PayrollRunStatus::Approved).await;
    let line = t.payroll_line(&run, &e, NET, NET).await;
    let held = t.payroll_line(&run, &g, NET, NET).await;
    PayrollLine::query()
        .where_(PayrollLine::ID.eq(held.id))
        .update(
            t.db(),
            &[
                ("status", &"held"),
                ("held_reason", &Some("absence report".to_owned())),
            ],
        )
        .await
        .unwrap();
    wl_hr::payroll::submit(t.state(), run.id).await.unwrap();

    t.reject(&a, &run, "other", "BANK-REJ-80")
        .await
        .assert_status(303);
    assert_eq!(t.line(&held).await.status, LineStatus::Held);
    assert_eq!(t.line(&line).await.status, LineStatus::Failed);
    t.transition(&b, &run, "return", "wrong OT")
        .await
        .assert_status(422);

    t.resubmit(&a, &run, "Resent").await.assert_status(303);
    let files = SifFile::of_run(t.db(), run.id).await.unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[1].line_count, 1, "the held line is not added");
    assert_eq!(t.line(&held).await.status, LineStatus::Held);
    assert_eq!(t.line(&held).await.sif_version, None);
}
