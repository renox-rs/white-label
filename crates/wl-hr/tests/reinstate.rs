//! BL-HRM-08 (#198): `reinstate` returns an absconded employee to `active`
//! when the absence report is withdrawn or cancelled. HR enters the day the
//! worker returned and the withdrawal reference; another user with
//! `employees.exit.approve` approves (`HRM.ACL.07`). The draft or approved
//! settlement and the cancellation case are cancelled, the held lines are
//! released through `HRM.RUL.12` and `EmployeeReinstated` is emitted.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_hr::{
    AbsenceReport, Employee, EmployeeStatus, FinalSettlement, LineStatus, PayrollLine, PayrollRun,
    PayrollRunStatus, SettlementStatus, VisaCase, VisaCaseStatus,
};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::workforce::workforce;

use support::{Booted, nov};

/// The fields of the report request: "MOHRE-777" of 2026-11-09.
fn report() -> Vec<(&'static str, &'static str)> {
    vec![
        ("transition", "report_absconding"),
        ("reason", "absent since 2026-11-01"),
        ("report_number", "MOHRE-777"),
        ("report_date", "2026-11-09"),
    ]
}

/// The fields of a reinstatement: the reference "WD-55" and the return date
/// 2026-11-20.
fn reinstate() -> Vec<(&'static str, &'static str)> {
    vec![
        ("transition", "reinstate"),
        ("reason", "the worker came back"),
        ("reference", "WD-55"),
        ("return_date", "2026-11-20"),
    ]
}

/// `user` posts `fields` for `e` on `day`.
async fn post(
    t: &Booted,
    day: NaiveDate,
    user: &User,
    e: &Employee,
    fields: &[(&str, &str)],
) -> renox::testing::TestResponse {
    t.on(day, async {
        t.app.acting_as(user);
        t.app
            .htmx()
            .post(&format!("/hr/employees/{}/exits", e.id), fields)
            .await
    })
    .await
}

/// The MD approves the request pending about `e` on `day`.
async fn approve(t: &Booted, md: &User, e: &Employee, day: NaiveDate) {
    let request = ApprovalRequest::pending_about(t.db(), &e.subject())
        .await
        .unwrap()
        .remove(0);
    t.on(day, approvals::approve(t.state(), request.id, md, ""))
        .await
        .unwrap();
}

/// E is absent since 1 Nov, reported as absconded on 9 Nov (the real request
/// and approval), with a November line of AED 3,000 held for the report, a
/// draft settlement and the cancellation case the report opened.
struct World {
    t: Booted,
    hr: User,
    md: User,
    e: Employee,
    held: PayrollLine,
}

async fn world(from: EmployeeStatus) -> World {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let e = t.employee_in(60, from).await;
    t.open_case(&e, nov(4), nov(1)).await;
    t.attempt(&e, nov(4)).await;
    t.attempt(&e, nov(6)).await;
    let run = t.payroll_run("2026-11", PayrollRunStatus::Calculated).await;
    let held = t.payroll_line(&run, &e, 300_000, 300_000).await;
    post(&t, nov(9), &hr, &e, &report()).await.assert_ok();
    approve(&t, &md, &e, nov(9)).await;
    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Absconded);
    let held = PayrollLine::find_or_404(t.db(), held.id).await.unwrap();
    assert_eq!(held.status, LineStatus::Held);
    World { t, hr, md, e, held }
}

impl World {
    async fn settlements(&self) -> Vec<FinalSettlement> {
        FinalSettlement::of_employee(self.t.db(), self.e.id)
            .await
            .unwrap()
    }

    async fn cancellations(&self) -> Vec<VisaCase> {
        VisaCase::cancellations_of(self.t.db(), self.e.id)
            .await
            .unwrap()
    }
}

/// AC1: E `absconded` with a held November line, the draft settlement and the
/// *cancellation* case `open`: reinstated with the reference "WD-55" and the
/// return date 2026-11-20, E is `active`, the settlement and the case are
/// `cancelled`, `HRM.RUL.12` proposes an off-cycle run for the held line and
/// `EmployeeReinstated` is emitted.
#[renox::test]
async fn hrm08_ac1_a_reinstatement_cancels_the_settlement_and_the_case_and_releases_the_pay() {
    let w = world(EmployeeStatus::Active).await;
    // The November run was sent and paid meanwhile, so the held line is
    // proposed into an off-cycle run (a run not yet sent takes it back).
    renox::db::sql("UPDATE wl_hr_payroll_runs SET status = 'paid' WHERE id = ?")
        .bind(w.held.run_id)
        .execute(w.t.db())
        .await
        .unwrap();
    assert_eq!(w.settlements().await[0].status, SettlementStatus::Draft);
    assert_eq!(w.cancellations().await[0].status, VisaCaseStatus::Open);
    let account = w.t.user("Ravi").await;
    renox::db::sql("UPDATE wl_hr_employees SET user_id = ? WHERE id = ?")
        .bind(account.id)
        .bind(w.e.id)
        .execute(w.t.db())
        .await
        .unwrap();
    assert!(
        workforce(w.t.state())
            .access_disabled(w.t.db(), account.id)
            .await
            .unwrap()
    );

    post(&w.t, nov(20), &w.hr, &w.e, &reinstate())
        .await
        .assert_ok();
    assert_eq!(w.t.employee(&w.e).await.status, EmployeeStatus::Absconded);
    approve(&w.t, &w.md, &w.e, nov(20)).await;

    assert_eq!(w.t.employee(&w.e).await.status, EmployeeStatus::Active);
    assert_eq!(w.settlements().await[0].status, SettlementStatus::Cancelled);
    assert_eq!(w.cancellations().await[0].status, VisaCaseStatus::Cancelled);
    let report = AbsenceReport::query()
        .where_(AbsenceReport::EMPLOYEE_ID.eq(w.e.id))
        .first(w.t.db())
        .await
        .unwrap()
        .expect("the report");
    assert_eq!(report.status, "withdrawn");
    assert_eq!(report.withdrawal_reference.as_deref(), Some("WD-55"));
    assert_eq!(report.returned_on, Some(nov(20)));
    // `HRM.RUL.12`: an off-cycle run is proposed for the held line.
    let off = PayrollRun::query()
        .where_(PayrollRun::RUN_TYPE.eq("off_cycle"))
        .get(w.t.db())
        .await
        .unwrap();
    assert_eq!(off.len(), 1);
    assert_eq!(off[0].status, PayrollRunStatus::Draft);
    let lines = PayrollLine::query()
        .where_(PayrollLine::RUN_ID.eq(off[0].id))
        .get(w.t.db())
        .await
        .unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].source_line_id, Some(w.held.id));
    let events = w.t.reinstated.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].employee, w.e.employee_id());
    assert_eq!(events[0].return_date, nov(20));
    // The portal works again.
    assert!(
        !workforce(w.t.state())
            .access_disabled(w.t.db(), account.id)
            .await
            .unwrap()
    );
}

/// AC2: without the reference it is refused; without the return date, or with
/// one in the future, it is refused. Nothing is requested.
#[renox::test]
async fn hrm08_ac2_without_the_reference_or_with_a_bad_return_date_it_is_refused() {
    let w = world(EmployeeStatus::Active).await;
    let bad: [Vec<(&str, &str)>; 4] = [
        vec![
            ("transition", "reinstate"),
            ("reason", "came back"),
            ("return_date", "2026-11-20"),
        ],
        vec![
            ("transition", "reinstate"),
            ("reason", "came back"),
            ("reference", "WD-55"),
        ],
        vec![
            ("transition", "reinstate"),
            ("reason", "came back"),
            ("reference", "WD-55"),
            ("return_date", "2026-11-21"),
        ],
        vec![
            ("transition", "reinstate"),
            ("reason", ""),
            ("reference", "WD-55"),
            ("return_date", "2026-11-20"),
        ],
    ];
    for fields in bad {
        post(&w.t, nov(20), &w.hr, &w.e, &fields)
            .await
            .assert_status(422);
    }
    assert!(
        ApprovalRequest::pending_about(w.t.db(), &w.e.subject())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(w.t.employee(&w.e).await.status, EmployeeStatus::Absconded);
    assert!(w.t.reinstated.all().is_empty());
}

/// AC3: E absconded from `notice` (last working day 2026-11-30): after
/// reinstatement E is `active` with no last working day, and the notice is not
/// resumed.
#[renox::test]
async fn hrm08_ac3_an_employee_who_absconded_from_notice_is_active_without_a_last_day() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let e = t.employee_in(60, EmployeeStatus::Notice).await;
    renox::db::sql("UPDATE wl_hr_employees SET last_working_day = ? WHERE id = ?")
        .bind(nov(30))
        .bind(e.id)
        .execute(t.db())
        .await
        .unwrap();
    // The notice drafted the settlement; the report relabels it.
    let settlement = t
        .settlement_in(&e, SettlementStatus::Draft, |s| {
            s.last_working_day = Some(nov(30));
        })
        .await;
    t.open_case(&e, nov(4), nov(1)).await;
    t.attempt(&e, nov(4)).await;
    t.attempt(&e, nov(6)).await;
    post(&t, nov(9), &hr, &e, &report()).await.assert_ok();
    approve(&t, &md, &e, nov(9)).await;
    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Absconded);
    assert_eq!(t.employee(&e).await.last_working_day, Some(nov(30)));

    post(&t, nov(20), &hr, &e, &reinstate()).await.assert_ok();
    approve(&t, &md, &e, nov(20)).await;

    let after = t.employee(&e).await;
    assert_eq!(after.status, EmployeeStatus::Active);
    assert_eq!(after.last_working_day, None);
    assert_eq!(
        t.settlement(&settlement).await.status,
        SettlementStatus::Cancelled
    );
}

/// AC4: with a *cancellation* case `completed`, it is refused.
#[renox::test]
async fn hrm08_ac4_a_completed_cancellation_case_refuses_the_reinstatement() {
    let w = world(EmployeeStatus::Active).await;
    renox::db::sql("UPDATE wl_hr_visa_cases SET status = 'completed' WHERE employee_id = ?")
        .bind(w.e.id)
        .execute(w.t.db())
        .await
        .unwrap();

    let refused = post(&w.t, nov(20), &w.hr, &w.e, &reinstate()).await;

    refused.assert_status(422);
    assert!(refused.text().contains("completed"), "{}", refused.text());
    assert_eq!(w.t.employee(&w.e).await.status, EmployeeStatus::Absconded);
    assert_eq!(w.settlements().await[0].status, SettlementStatus::Draft);
}

/// AC5: the settlement `approved` and nothing paid: it is `cancelled` on
/// reinstatement. With an AED 500 payment recorded against it, `paid` or
/// `disputed`, reinstatement is refused (the worker is rehired as a new
/// employee).
#[renox::test]
async fn hrm08_ac5_an_approved_settlement_is_cancelled_and_a_paid_one_refuses() {
    let w = world(EmployeeStatus::Active).await;
    let settlement = w.settlements().await.remove(0);
    renox::db::sql("UPDATE wl_hr_final_settlements SET status = 'approved' WHERE id = ?")
        .bind(settlement.id)
        .execute(w.t.db())
        .await
        .unwrap();
    w.t.paid_against(&settlement, 50_000).await;

    let refused = post(&w.t, nov(20), &w.hr, &w.e, &reinstate()).await;
    refused.assert_status(422);
    assert!(
        refused
            .text()
            .contains("A payment is recorded against the settlement"),
        "{}",
        refused.text()
    );
    for (status, message) in [
        ("paid", "Settlement already paid"),
        ("disputed", "Settlement is disputed"),
    ] {
        renox::db::sql("UPDATE wl_hr_final_settlements SET status = ? WHERE id = ?")
            .bind(status)
            .bind(settlement.id)
            .execute(w.t.db())
            .await
            .unwrap();
        let refused = post(&w.t, nov(20), &w.hr, &w.e, &reinstate()).await;
        refused.assert_status(422);
        assert!(refused.text().contains(message), "{}", refused.text());
    }
    assert_eq!(w.t.employee(&w.e).await.status, EmployeeStatus::Absconded);

    // Nothing paid: an approved settlement is cancelled with the reinstatement.
    let w = world(EmployeeStatus::Active).await;
    let settlement = w.settlements().await.remove(0);
    renox::db::sql("UPDATE wl_hr_final_settlements SET status = 'approved' WHERE id = ?")
        .bind(settlement.id)
        .execute(w.t.db())
        .await
        .unwrap();
    post(&w.t, nov(20), &w.hr, &w.e, &reinstate())
        .await
        .assert_ok();
    approve(&w.t, &w.md, &w.e, nov(20)).await;
    assert_eq!(w.t.employee(&w.e).await.status, EmployeeStatus::Active);
    assert_eq!(w.settlements().await[0].status, SettlementStatus::Cancelled);
}
