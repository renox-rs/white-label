//! BL-HRM-11 (#201): the approver returns a `calculated` or `approved` run to
//! `draft` with a reason and comments on the lines to fix (`return`); the
//! approval is cleared, the preparer is told and `PayrollRunReturned` follows.
//! Once the file was sent `return` is not offered (`HRM.ACL.04` stays: the
//! returner is never the preparer).

mod support;

use renox::prelude::*;
use wl_hr::{EmployeeStatus, PayrollLine, PayrollRun, PayrollRunStatus};

use support::{Booted, users};

const WAGE: i64 = 300_000;

/// A: prepares. B: approves and returns. E: the employee whose line is wrong.
struct Desk {
    t: Booted,
    a: User,
    b: User,
    run: PayrollRun,
    line: PayrollLine,
    other_line: PayrollLine,
}

async fn desk() -> Desk {
    let t = support::boot().await;
    let a = t.user_with("Pia", "payroll_prep").await;
    let b = t.user_with("Ali", "payroll_appr").await;
    let e = t.employee_in(61, EmployeeStatus::Active).await;
    let other = t.employee_in(62, EmployeeStatus::Active).await;
    let run = t.payroll_run("2026-11", PayrollRunStatus::Draft).await;
    let line = t.payroll_line(&run, &e, WAGE, WAGE).await;
    let other_line = t.payroll_line(&run, &other, WAGE, WAGE).await;
    Desk {
        t,
        a,
        b,
        run,
        line,
        other_line,
    }
}

/// A calculates and B approves.
async fn approved(d: &Desk) {
    d.t.transition(&d.a, &d.run, "calculate", "")
        .await
        .assert_status(303);
    d.t.transition(&d.b, &d.run, "approve", "")
        .await
        .assert_status(303);
    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Approved);
}

/// AC1: a run `approved` by B, B returns it with reason "wrong OT" and a
/// comment on E's line: the run is `draft`, the approver cleared, the preparer
/// A notified with the reason and the comment, `PayrollRunReturned` emitted,
/// and no line or file deleted.
#[renox::test]
async fn hrm11_ac1_the_approver_returns_an_approved_run_with_a_reason_and_a_line_comment() {
    let d = desk().await;
    approved(&d).await;
    assert_eq!(d.t.run(&d.run).await.approved_by, Some(d.b.id));

    d.t.comment(&d.b, &d.line, "Overtime of 12 h, not 21 h")
        .await
        .assert_status(303);
    d.t.transition(&d.b, &d.run, "return", "wrong OT")
        .await
        .assert_status(303);

    let run = d.t.run(&d.run).await;
    assert_eq!(run.status, PayrollRunStatus::Draft);
    assert_eq!(run.approved_by, None, "the approver id is not kept");
    assert_eq!(run.return_reason.as_deref(), Some("wrong OT"));
    assert_eq!(run.prepared_by, Some(d.a.id));
    // The lines stay, with the comment on E's line only.
    let lines = run.lines(d.t.db()).await.unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(
        d.t.line(&d.line).await.approver_comment.as_deref(),
        Some("Overtime of 12 h, not 21 h")
    );
    assert_eq!(d.t.line(&d.other_line).await.approver_comment, None);

    // The preparer sees the comment on the line of the draft run page.
    d.t.app.acting_as(&d.a);
    d.t.app
        .get(&format!("/hr/payroll/{}", d.run.id))
        .await
        .assert_ok()
        .assert_see("Overtime of 12 h, not 21 h");

    // The preparer is told.
    let notices = d.t.notices.of_type("hr.payroll_returned");
    assert_eq!(notices.len(), 1);
    assert_eq!(users(&notices[0]), vec![d.a.id]);
    assert_eq!(notices[0].data["reason"], "wrong OT");
    assert!(
        notices[0].data["comments"]
            .as_str()
            .unwrap()
            .contains("Overtime of 12 h, not 21 h")
    );
    // The event.
    let events = d.t.returned.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].run.0, d.run.id);
    assert_eq!(events[0].period, "2026-11");
    assert_eq!(events[0].reason, "wrong OT");
}

/// AC1, from `calculated` too (the other state `return` leaves), and the next
/// `calculate` takes the comments away and goes on with the same lines.
#[renox::test]
async fn hrm11_a_calculated_run_can_be_returned_and_calculate_answers_the_comments() {
    let d = desk().await;
    d.t.transition(&d.a, &d.run, "calculate", "")
        .await
        .assert_status(303);
    d.t.comment(&d.b, &d.line, "Wrong rate")
        .await
        .assert_status(303);
    d.t.transition(&d.b, &d.run, "return", "rate")
        .await
        .assert_status(303);
    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Draft);

    d.t.transition(&d.a, &d.run, "calculate", "")
        .await
        .assert_status(303);
    assert_eq!(d.t.line(&d.line).await.approver_comment, None);
    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Calculated);
    assert_eq!(
        d.t.run(&d.run).await.lines(d.t.db()).await.unwrap().len(),
        2
    );
}

/// AC2: without a comment on a line it is refused: the run stays `approved`,
/// nothing is sent. A reason is needed too.
#[renox::test]
async fn hrm11_ac2_a_return_without_a_line_comment_or_a_reason_is_refused() {
    let d = desk().await;
    approved(&d).await;

    d.t.transition(&d.b, &d.run, "return", "wrong OT")
        .await
        .assert_status(422);
    let run = d.t.run(&d.run).await;
    assert_eq!(run.status, PayrollRunStatus::Approved);
    assert_eq!(run.approved_by, Some(d.b.id), "the approval stands");
    assert!(d.t.returned.all().is_empty());
    assert!(d.t.notices.of_type("hr.payroll_returned").is_empty());

    d.t.comment(&d.b, &d.line, "Check this")
        .await
        .assert_status(303);
    d.t.transition(&d.b, &d.run, "return", "  ")
        .await
        .assert_status(422);
    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Approved);

    // A blank comment takes the comment away again.
    d.t.comment(&d.b, &d.line, " ").await.assert_status(303);
    d.t.transition(&d.b, &d.run, "return", "wrong OT")
        .await
        .assert_status(422);
}

/// AC3: from `submitted` `return` is not offered, and it is refused.
#[renox::test]
async fn hrm11_ac3_a_submitted_run_cannot_be_returned() {
    let d = desk().await;
    approved(&d).await;

    // The approved run offers it on its page.
    d.t.app.acting_as(&d.b);
    d.t.app
        .get(&format!("/hr/payroll/{}", d.run.id))
        .await
        .assert_ok()
        .assert_see("Return");

    wl_hr::payroll::submit(d.t.state(), d.run.id).await.unwrap();
    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Submitted);
    d.t.comment(&d.b, &d.line, "Too late")
        .await
        .assert_status(422);
    d.t.transition(&d.b, &d.run, "return", "wrong OT")
        .await
        .assert_status(422);
    d.t.app.acting_as(&d.b);
    d.t.app
        .get(&format!("/hr/payroll/{}", d.run.id))
        .await
        .assert_ok()
        .assert_dont_see("Return");
    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Submitted);
}

/// The preparer never returns their own run, and only an approver comments.
#[renox::test]
async fn hrm11_the_preparer_and_users_without_payroll_approve_cannot_return_or_comment() {
    let d = desk().await;
    let all = d.t.user_with("Zed", "payroll_all").await;
    d.t.transition(&all, &d.run, "calculate", "")
        .await
        .assert_status(303);

    // Zed prepared it: he may not comment on it or return it (`HRM.ACL.04`).
    d.t.comment(&all, &d.line, "Mine").await.assert_status(422);
    assert_eq!(d.t.line(&d.line).await.approver_comment, None);
    d.t.transition(&all, &d.run, "return", "wrong OT")
        .await
        .assert_status(422);

    // Pia holds no `payroll.approve`.
    d.t.comment(&d.a, &d.line, "Mine").await.assert_status(403);
    d.t.transition(&d.a, &d.run, "return", "wrong OT")
        .await
        .assert_status(403);
    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Calculated);
}
