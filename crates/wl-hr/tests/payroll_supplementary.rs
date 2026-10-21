//! BL-HRM-17 (#207), `HRM.ACL.04` and `HRM.ACL.06`: an error found after a run
//! is `paid` is corrected by a supplementary run (wl-hr §3 *Payroll run*, §4
//! *Run types*, §5.6 step 1). The paid run is never edited; positive
//! differences are paid through the run's own SIF, negative ones are
//! recovered within the Art. 25 caps, from a settlement or as a receivable.

mod support;

use renox::prelude::*;
use wl_hr::{
    Adjustment, AdjustmentKind, Employee, EmployeeStatus, FinalSettlement, LineStatus, PayrollLine,
    PayrollRun, PayrollRunStatus, Receivable, SettlementStatus, SifFile,
};
use wl_kernel::Subject;

use support::Booted;

/// A wage of AED 3,000 in fils.
const WAGE: i64 = 300_000;

/// AED `dirhams` in fils.
fn aed(dirhams: i64) -> i64 {
    dirhams * 100
}

/// The October run R, `paid`: E was overpaid AED 900 (paid AED 3,900, should
/// be AED 3,000), F underpaid AED 200 (paid AED 1,000, should be AED 1,200).
/// A prepares (and holds every payroll permission), B approves.
struct World {
    t: Booted,
    a: User,
    b: User,
    e: Employee,
    f: Employee,
    r: PayrollRun,
    project_e: Subject,
    project_f: Subject,
}

async fn world() -> World {
    let t = support::boot().await;
    let a = t.user_with("Zed", "payroll_all").await;
    let b = t.user_with("Ali", "payroll_appr").await;
    let e = t.employee_in(101, EmployeeStatus::Active).await;
    let f = t.employee_in(102, EmployeeStatus::Active).await;
    let r = t.payroll_run("2026-10", PayrollRunStatus::Paid).await;
    let project_e = Subject::new("construction.project", 7);
    let project_f = Subject::new("construction.project", 8);
    t.paid_line(&r, &e, WAGE, aed(3_900), &project_e).await;
    t.paid_line(&r, &f, aed(1_000), aed(1_000), &project_f)
        .await;
    World {
        t,
        a,
        b,
        e,
        f,
        r,
        project_e,
        project_f,
    }
}

impl World {
    /// A creates a supplementary run correcting `run` with `rows`.
    async fn correct(
        &self,
        run: &PayrollRun,
        reason: &str,
        rows: &str,
    ) -> renox::testing::TestResponse {
        self.t.app.acting_as(&self.a);
        self.t
            .app
            .post(
                &format!("/hr/payroll/{}/supplementary", run.id),
                &[("reason", reason), ("corrections", rows)],
            )
            .await
    }

    /// The supplementary runs, oldest first.
    async fn supplementary(&self) -> Vec<PayrollRun> {
        PayrollRun::query()
            .where_(PayrollRun::RUN_TYPE.eq("supplementary"))
            .order_by(PayrollRun::ID)
            .get(self.t.db())
            .await
            .unwrap()
    }

    /// A calculates, B approves, the file is sent.
    async fn send(&self, run: &PayrollRun) {
        self.t
            .transition(&self.a, run, "calculate", "")
            .await
            .assert_status(303);
        self.t
            .transition(&self.b, run, "approve", "")
            .await
            .assert_status(303);
        wl_hr::payroll::submit(self.t.state(), run.id)
            .await
            .unwrap();
    }

    async fn adjustments_of(&self, employee: &Employee) -> Vec<Adjustment> {
        Adjustment::query()
            .where_(Adjustment::EMPLOYEE_ID.eq(employee.id))
            .order_by(Adjustment::ID)
            .get(self.t.db())
            .await
            .unwrap()
    }

    async fn lines(&self, run: &PayrollRun) -> Vec<PayrollLine> {
        run.lines(self.t.db()).await.unwrap()
    }
}

fn rows(e: &Employee, f: Option<&Employee>) -> String {
    let mut rows = format!("{}=3000.00=overtime", e.id);
    if let Some(f) = f {
        rows.push_str(&format!("\n{}=1200.00=allowance", f.id));
    }
    rows
}

/// AC1: R is `paid` with E overpaid AED 900 and F underpaid AED 200: A creates
/// a supplementary run with a reason; it has two lines, E -900 and F +200, and
/// R is unchanged.
#[renox::test]
async fn hrm17_ac1_a_supplementary_run_lists_only_the_differences_and_leaves_the_paid_run_alone() {
    let w = world().await;

    w.correct(&w.r, "Overtime miscounted", &rows(&w.e, Some(&w.f)))
        .await
        .assert_status(303);

    let runs = w.supplementary().await;
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(run.corrects, Some(w.r.id));
    assert_eq!(run.reason.as_deref(), Some("Overtime miscounted"));
    assert_eq!(run.period, "2026-10");
    assert_eq!(run.status, PayrollRunStatus::Draft);
    let lines = w.lines(run).await;
    let nets: Vec<(i64, i64)> = lines
        .iter()
        .map(|l| (l.employee_id, l.net_amount))
        .collect();
    assert_eq!(nets, vec![(w.e.id, -aed(900)), (w.f.id, aed(200))]);
    assert_eq!(lines[0].component.as_deref(), Some("overtime"));
    // The paid run, its lines and its totals are as they were.
    assert_eq!(w.t.run(&w.r).await.status, PayrollRunStatus::Paid);
    let original: Vec<(i64, i64, LineStatus)> = w
        .lines(&w.r)
        .await
        .iter()
        .map(|l| (l.employee_id, l.net_amount, l.status))
        .collect();
    assert_eq!(
        original,
        vec![
            (w.e.id, aed(3_900), LineStatus::Paid),
            (w.f.id, aed(1_000), LineStatus::Paid)
        ]
    );
    // An employee whose pay is right is not in it.
    w.t.app.acting_as(&w.a);
    w.t.app
        .get(&format!("/hr/payroll/{}", run.id))
        .await
        .assert_ok()
        .assert_see("Overtime miscounted");
}

/// AC2: B approves (A cannot); the SIF has only F's AED 200; on `confirm_paid`
/// the corrected payslips are published and E has an overpayment recovery of
/// AED 900 at AED 600 a month over two runs.
#[renox::test]
async fn hrm17_ac2_another_user_approves_the_bank_gets_only_the_positive_line_and_the_recovery_starts()
 {
    let w = world().await;
    w.correct(&w.r, "Overtime miscounted", &rows(&w.e, Some(&w.f)))
        .await
        .assert_status(303);
    let run = w.supplementary().await.remove(0);

    w.t.transition(&w.a, &run, "calculate", "")
        .await
        .assert_status(303);
    w.t.transition(&w.a, &run, "approve", "")
        .await
        .assert_status(422);
    w.t.transition(&w.b, &run, "approve", "")
        .await
        .assert_status(303);
    wl_hr::payroll::submit(w.t.state(), run.id).await.unwrap();

    let files = SifFile::of_run(w.t.db(), run.id).await.unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!((files[0].line_count, files[0].total), (1, aed(200)));
    let mut tx = w.t.db().begin().await.unwrap();
    let carried = files[0].lines_in(&mut tx).await.unwrap();
    assert_eq!(carried.len(), 1);
    assert_eq!(carried[0].employee_id, w.f.id);
    drop(tx);
    // Nothing is recovered before the correction is final.
    assert!(w.adjustments_of(&w.e).await.is_empty());

    w.t.confirm(&w.b, &run, "BANK-1").await.assert_status(303);

    assert_eq!(w.t.run(&run).await.status, PayrollRunStatus::Paid);
    for line in w.lines(&run).await {
        assert_eq!(line.status, LineStatus::Paid);
        assert!(line.payslip_published_at.is_some(), "{line:?}");
    }
    let recoveries = w.adjustments_of(&w.e).await;
    let plan: Vec<(String, i64, AdjustmentKind)> = recoveries
        .iter()
        .map(|a| (a.period.clone(), a.amount, a.kind))
        .collect();
    assert_eq!(
        plan,
        vec![
            (
                "2026-11".to_owned(),
                aed(600),
                AdjustmentKind::OverpaymentRecovery
            ),
            (
                "2026-12".to_owned(),
                aed(300),
                AdjustmentKind::OverpaymentRecovery
            ),
        ]
    );
    // F is underpaid, so nothing is recovered from F.
    assert!(w.adjustments_of(&w.f).await.is_empty());
}

/// AC3: the cost differences -900 and +200 are posted to R's cost objects.
#[renox::test]
async fn hrm17_ac3_the_cost_differences_are_posted_to_the_cost_objects_of_the_paid_run() {
    let w = world().await;
    w.correct(&w.r, "Overtime miscounted", &rows(&w.e, Some(&w.f)))
        .await
        .assert_status(303);
    let run = w.supplementary().await.remove(0);
    w.send(&run).await;
    assert!(w.t.costs.posted().is_empty());

    w.t.confirm(&w.b, &run, "BANK-1").await.assert_status(303);

    let posted: Vec<(Subject, i64, Subject)> =
        w.t.costs
            .posted()
            .into_iter()
            .map(|p| (p.cost_object, p.amount_aed, p.source))
            .collect();
    assert_eq!(
        posted,
        vec![
            (w.project_e.clone(), -aed(900), run.subject()),
            (w.project_f.clone(), aed(200), run.subject()),
        ]
    );
}

/// AC4: a supplementary run with only E's -AED 900 line: `submit` generates no
/// SIF and `confirm_paid` succeeds without a bank confirmation, creating E's
/// recovery.
#[renox::test]
async fn hrm17_ac4_a_run_with_only_negative_differences_sends_nothing_to_the_bank() {
    let w = world().await;
    w.correct(&w.r, "Overtime miscounted", &rows(&w.e, None))
        .await
        .assert_status(303);
    let run = w.supplementary().await.remove(0);
    w.send(&run).await;

    let sent = w.t.run(&run).await;
    assert_eq!(sent.status, PayrollRunStatus::Submitted);
    assert_eq!(sent.sif_version, 0);
    assert!(SifFile::of_run(w.t.db(), run.id).await.unwrap().is_empty());

    w.t.confirm(&w.b, &run, "").await.assert_status(303);

    assert_eq!(w.t.run(&run).await.status, PayrollRunStatus::Paid);
    let plan: Vec<i64> = w
        .adjustments_of(&w.e)
        .await
        .iter()
        .map(|a| a.amount)
        .collect();
    assert_eq!(plan, vec![aed(600), aed(300)]);
    let line = &w.lines(&run).await[0];
    assert_eq!(line.status, LineStatus::Paid);
    assert!(line.payslip_published_at.is_some());
}

/// AC5: G exited with an `approved` settlement and a -AED 300 difference: on
/// `confirm_paid` the AED 300 is added to G's settlement recoveries; given
/// H's settlement `paid`, a receivable of AED 300 is created on H's party
/// instead. Neither is recovered from a later run.
#[renox::test]
async fn hrm17_ac5_an_employee_who_left_is_recovered_from_the_settlement_or_owes_a_receivable() {
    let w = world().await;
    let g = w.t.employee_in(103, EmployeeStatus::Exited).await;
    let h = w.t.employee_in(104, EmployeeStatus::Exited).await;
    for (employee, status) in [
        (&g, SettlementStatus::Approved),
        (&h, SettlementStatus::Paid),
    ] {
        let mut settlement = FinalSettlement {
            employee_id: employee.id,
            reason: "resignation".into(),
            payee: "employee".into(),
            ..FinalSettlement::default()
        };
        let mut tx = w.t.db().begin().await.unwrap();
        settlement
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        w.t.paid_line(&w.r, employee, WAGE, aed(1_300), &w.project_e)
            .await;
    }
    let rows = format!("{}=1000.00=advance\n{}=1000.00=advance", g.id, h.id);
    w.correct(&w.r, "Advance deducted twice", &rows)
        .await
        .assert_status(303);
    let run = w.supplementary().await.remove(0);
    w.send(&run).await;

    w.t.confirm(&w.b, &run, "").await.assert_status(303);

    let settlements = FinalSettlement::of_employee(w.t.db(), g.id).await.unwrap();
    assert_eq!(settlements[0].recoveries, aed(300));
    assert!(
        Receivable::of_employee(w.t.db(), g.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(w.adjustments_of(&g).await.is_empty());

    let settlements = FinalSettlement::of_employee(w.t.db(), h.id).await.unwrap();
    assert_eq!(settlements[0].recoveries, 0);
    let owed = Receivable::of_employee(w.t.db(), h.id).await.unwrap();
    assert_eq!(owed.len(), 1);
    assert_eq!((owed[0].amount, owed[0].party_id), (aed(300), h.party_id));
    assert_eq!(owed[0].source_run_id, Some(run.id));
    assert!(w.adjustments_of(&h).await.is_empty());
}

/// Several supplementary runs on one paid run: each difference is against the
/// paid run plus the earlier corrections, and `corrects` always names the
/// original run, also when the correction is made from a paid supplementary
/// run.
#[renox::test]
async fn hrm17_a_second_correction_is_against_the_paid_run_plus_the_first_and_names_the_original() {
    let w = world().await;
    w.correct(&w.r, "First", &rows(&w.e, None))
        .await
        .assert_status(303);
    let first = w.supplementary().await.remove(0);
    w.send(&first).await;
    w.t.confirm(&w.b, &first, "").await.assert_status(303);

    // E should have had AED 2,800: AED 200 below the first correction.
    w.correct(&first, "Second", &format!("{}=2800.00=overtime", w.e.id))
        .await
        .assert_status(303);

    let runs = w.supplementary().await;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[1].corrects, Some(w.r.id));
    let line = &w.lines(&runs[1]).await[0];
    assert_eq!(line.net_amount, -aed(200));
}

/// Only a paid run is corrected, with a reason and at least one difference;
/// and only by `payroll.prepare`.
#[renox::test]
async fn hrm17_a_run_that_is_not_paid_a_blank_reason_or_no_difference_is_refused() {
    let w = world().await;
    let open =
        w.t.payroll_run("2026-11", PayrollRunStatus::Calculated)
            .await;
    w.correct(&open, "Too early", &rows(&w.e, None))
        .await
        .assert_status(422);
    w.correct(&w.r, "  ", &rows(&w.e, None))
        .await
        .assert_status(422);
    // E was paid AED 3,900: saying so is no difference.
    w.correct(&w.r, "Nothing wrong", &format!("{}=3900.00", w.e.id))
        .await
        .assert_status(422);
    let bad = w
        .correct(&w.r, "Bad row", "not a row")
        .await
        .status
        .as_u16();
    assert!((400..500).contains(&bad), "{bad}");
    assert!(w.supplementary().await.is_empty());

    w.t.app.acting_as(&w.b);
    w.t.app
        .post(
            &format!("/hr/payroll/{}/supplementary", w.r.id),
            &[("reason", "Not mine"), ("corrections", &rows(&w.e, None))],
        )
        .await
        .assert_status(403);
}

/// A positive difference held at the `submit` of the supplementary run (no
/// usable pay account) is paid later in an off-cycle run (`HRM.RUL.12`), not
/// in the supplementary run.
#[renox::test]
async fn hrm17_a_positive_line_held_at_submit_is_paid_in_an_off_cycle_run() {
    let w = world().await;
    w.correct(&w.r, "Overtime miscounted", &rows(&w.e, Some(&w.f)))
        .await
        .assert_status(303);
    let run = w.supplementary().await.remove(0);
    let f_line = w.lines(&run).await.remove(1);
    PayrollLine::query()
        .where_(PayrollLine::ID.eq(f_line.id))
        .update(
            w.t.db(),
            &[
                ("status", &LineStatus::Held.as_str().to_owned()),
                ("held_reason", &Some("closed account".to_owned())),
            ],
        )
        .await
        .unwrap();
    w.send(&run).await;
    assert!(SifFile::of_run(w.t.db(), run.id).await.unwrap().is_empty());
    w.t.confirm(&w.b, &run, "").await.assert_status(303);
    assert_eq!(w.t.line(&f_line).await.status, LineStatus::Held);

    wl_hr::release::line_fixed(w.t.state(), f_line.id)
        .await
        .unwrap();

    let off = PayrollRun::query()
        .where_(PayrollRun::RUN_TYPE.eq("off_cycle"))
        .get(w.t.db())
        .await
        .unwrap();
    assert_eq!(off.len(), 1);
    let lines = w.lines(&off[0]).await;
    assert_eq!(lines.len(), 1);
    assert_eq!(
        (lines[0].employee_id, lines[0].net_amount),
        (w.f.id, aed(200))
    );
    assert_eq!(w.t.line(&f_line).await.released_into, Some(off[0].id));
}
