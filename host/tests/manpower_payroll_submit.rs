//! BL-HRM-12 (#202), BL-HRM-13 (#203), WPS: a payroll run walked through the
//! routes as users in the seeded host, with no state forced: HR calculates, the
//! MD approves, HR submits (`POST /hr/payroll/{id}/transitions/submit`; refused
//! while a paid line has no active WPS pay account), records a partly failed
//! file, fixes the failed line through a new pay account, regenerates, and the
//! MD confirms payment.

mod manpower_support;
mod support;

use host::roles::{HR, OWNER};
use manpower_support::{
    DeliveringSms, boot_seeded_with, draft_run, run_transition, salary, worker_with_mobile,
};
use renox::prelude::*;
use renox::testing::TestApp;
use support::person;
use wl_hr::pay_account::{self, PayAccount, PayAccountStatus};
use wl_hr::sif::SifFile;
use wl_hr::{LineStatus, PayrollLine, PayrollRun, PayrollRunStatus};
use wl_kernel::workforce::EmployeeId;

const IBAN: &str = "AE550331234567890121234";
const IBAN_NEW: &str = "AE560331234567890125678";

async fn agency() -> TestApp {
    let sms = DeliveringSms::default();
    boot_seeded_with(host::Tuning::default().notify(move |n| n.provider(sms))).await
}

/// HR asks for a pay account for `who`, the MD approves it, and after the 5
/// seeded days of cooling the daily job activates it.
async fn active_account(app: &TestApp, hr: &User, md: &User, who: (EmployeeId, &str), iban: &str) {
    let (who, holder) = who;
    app.acting_as(hr);
    app.htmx()
        .post(
            &format!("/hr/employees/{}/pay-accounts", who.0),
            &[
                ("channel", "bank_account"),
                ("account_number", iban),
                ("routing_code", "123456789"),
                ("holder_name", holder),
            ],
        )
        .await
        .assert_status(303);
    let account = PayAccount::of_employee(app.db(), who.0)
        .await
        .unwrap()
        .pop()
        .expect("the requested account");
    app.acting_as(md);
    app.htmx()
        .post(
            &format!("/hr/pay-accounts/{}/transitions/approve", account.id),
            &[("reason", "")],
        )
        .await
        .assert_status(303);
    app.run_all_jobs().await;
    app.travel(std::time::Duration::from_secs(6 * 24 * 3600));
    app.at_travelled_time(async {
        app.run_scheduled(pay_account::TASK).await.unwrap();
    })
    .await;
    let active = PayAccount::find_or_404(app.db(), account.id).await.unwrap();
    assert_eq!(active.status, PayAccountStatus::Active);
}

async fn run_of(app: &TestApp, id: i64) -> PayrollRun {
    PayrollRun::find_or_404(app.db(), id).await.unwrap()
}

async fn lines_of(app: &TestApp, id: i64) -> Vec<PayrollLine> {
    PayrollLine::query()
        .where_(PayrollLine::RUN_ID.eq(id))
        .order_by(PayrollLine::ID)
        .get(app.db())
        .await
        .unwrap()
}

/// AC (route): the approved run is submitted by the preparer on
/// `POST /hr/payroll/{id}/transitions/submit`, file version 1 carries both
/// lines; a partial result fails one; the new pay account of that worker
/// fixes it when it becomes active; `regenerate` sends version 2 with that
/// line only; the MD confirms payment and the run is `paid`.
#[renox::test]
async fn a_run_is_walked_from_calculate_to_paid_through_the_routes() {
    let app = agency().await;
    let hr = person(&app, "Hana", HR).await;
    let md = person(&app, "Mazen", OWNER).await;
    let w1 = worker_with_mobile(&app, "Wasim Khan", "+971501110001").await;
    let w2 = worker_with_mobile(&app, "Wajid Ali", "+971501110002").await;
    for w in [w1, w2] {
        salary(&app, w, 300_000).await;
    }
    active_account(&app, &hr, &md, (w1, "Wasim Khan"), IBAN).await;
    active_account(&app, &hr, &md, (w2, "Wajid Ali"), IBAN).await;
    let run = draft_run(&app, "2026-11").await;

    run_transition(&app, &hr, &run, "calculate")
        .await
        .assert_status(303);
    run_transition(&app, &md, &run, "approve")
        .await
        .assert_status(303);
    assert_eq!(
        run_of(&app, run.id).await.status,
        PayrollRunStatus::Approved
    );

    // The approver does not prepare: the run is submitted by HR.
    run_transition(&app, &md, &run, "submit")
        .await
        .assert_status(403);
    run_transition(&app, &hr, &run, "submit")
        .await
        .assert_status(303);
    let sent = run_of(&app, run.id).await;
    assert_eq!(sent.status, PayrollRunStatus::Submitted);
    assert_eq!(sent.sif_version, 1);
    let files = SifFile::of_run(app.db(), run.id).await.unwrap();
    assert_eq!((files.len(), files[0].line_count), (1, 2));

    // The bank pays W1's line and fails W2's.
    let lines = lines_of(&app, run.id).await;
    let failing = lines.iter().find(|l| l.employee_id == w2.0).unwrap();
    support_post(
        &app,
        &hr,
        &format!("/hr/payroll/{}/partial", run.id),
        &[
            ("result_file", "result-v1.csv"),
            ("failed", &format!("{}=invalid IBAN", failing.id)),
        ],
    )
    .await;
    assert_eq!(
        run_of(&app, run.id).await.status,
        PayrollRunStatus::PartiallyRejected
    );
    let lines = lines_of(&app, run.id).await;
    assert_eq!(
        lines.iter().find(|l| l.employee_id == w1.0).unwrap().status,
        LineStatus::Paid
    );
    assert_eq!(
        lines.iter().find(|l| l.id == failing.id).unwrap().status,
        LineStatus::Failed
    );

    // Not regenerated while the failed line is not fixed.
    run_transition(&app, &hr, &run, "regenerate")
        .await
        .assert_status(422);

    // HR enters the right account for W2; once active the line is fixed.
    active_account(&app, &hr, &md, (w2, "Wajid Ali"), IBAN_NEW).await;
    let fixed = lines_of(&app, run.id).await;
    assert_eq!(
        fixed.iter().find(|l| l.id == failing.id).unwrap().status,
        LineStatus::Included
    );
    run_transition(&app, &hr, &run, "regenerate")
        .await
        .assert_status(303);
    let resent = run_of(&app, run.id).await;
    assert_eq!(resent.status, PayrollRunStatus::Submitted);
    assert_eq!(resent.sif_version, 2);
    let files = SifFile::of_run(app.db(), run.id).await.unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[1].line_count, 1);

    support_post(
        &app,
        &md,
        &format!("/hr/payroll/{}/confirm", run.id),
        &[("confirmation", "bank-ref-v2")],
    )
    .await;
    assert_eq!(run_of(&app, run.id).await.status, PayrollRunStatus::Paid);
    assert!(
        lines_of(&app, run.id)
            .await
            .iter()
            .all(|l| l.status == LineStatus::Paid)
    );
}

async fn support_post(app: &TestApp, by: &User, path: &str, form: &[(&str, &str)]) {
    manpower_support::post(app, by, path, form)
        .await
        .assert_status(303);
}

/// A line to be paid for a worker without an active WPS pay account is refused
/// at `submit` with the employee listed, the run stays `approved` with no file,
/// and the run page shows the button disabled with the reason; with the
/// account the same call succeeds.
#[renox::test]
async fn submit_is_refused_while_a_paid_line_has_no_active_wps_account() {
    let app = agency().await;
    let hr = person(&app, "Hana", HR).await;
    let md = person(&app, "Mazen", OWNER).await;
    let w1 = worker_with_mobile(&app, "Wasim Khan", "+971501110001").await;
    let w2 = worker_with_mobile(&app, "Wajid Ali", "+971501110002").await;
    for w in [w1, w2] {
        salary(&app, w, 300_000).await;
    }
    active_account(&app, &hr, &md, (w1, "Wasim Khan"), IBAN).await;
    let run = draft_run(&app, "2026-11").await;
    run_transition(&app, &hr, &run, "calculate")
        .await
        .assert_status(303);
    run_transition(&app, &md, &run, "approve")
        .await
        .assert_status(303);

    let refused = run_transition(&app, &hr, &run, "submit").await;
    refused.assert_status(422);
    let number = wl_hr::Employee::find_or_404(app.db(), w2.0)
        .await
        .unwrap()
        .employee_number;
    refused.assert_see(&number);
    let after = run_of(&app, run.id).await;
    assert_eq!(
        (after.status, after.sif_version),
        (PayrollRunStatus::Approved, 0)
    );
    assert!(SifFile::of_run(app.db(), run.id).await.unwrap().is_empty());

    app.acting_as(&hr);
    app.get(&format!("/hr/payroll/{}", run.id))
        .await
        .assert_see(&number);

    active_account(&app, &hr, &md, (w2, "Wajid Ali"), IBAN_NEW).await;
    run_transition(&app, &hr, &run, "submit")
        .await
        .assert_status(303);
    assert_eq!(
        run_of(&app, run.id).await.status,
        PayrollRunStatus::Submitted
    );
}
