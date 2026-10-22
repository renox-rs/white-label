//! BL-HRM-09 (#199): `record_death` records that an employee died in service.
//! HR attaches the death certificate and requests it; another user with
//! `employees.exit.approve` approves (`HRM.ACL.07`). The open payroll lines are
//! held and the salary owed up to the date of death moves into the final
//! settlement (payee the estate), unstarted leave is cancelled, every message
//! to the employee stops, the *cancellation* case opens, the HR tasks open and
//! `EmployeeDeceased` is emitted.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_hr::{
    Employee, EmployeeStatus, FinalSettlement, LeaveRequest, LeaveStatus, LineStatus, PayrollLine,
    PayrollRun, PayrollRunStatus, SettlementStatus, VisaCase, VisaCaseStatus, release,
};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::notify::{
    self, NotificationTypeDef, NotifyChannel, NotifyRecipient, NotifyRequest, register_type,
};
use wl_kernel::workforce::workforce;
use wl_kernel::{PartyId, Subject};
use wl_notify::{Message, MessageStatus};

use support::{Booted, nov};

/// The fields of a death on 2026-11-10 with the certificate "DC-9".
fn death() -> Vec<(&'static str, &'static str)> {
    vec![
        ("transition", "record_death"),
        ("reason", "died in service"),
        ("death_certificate", "DC-9"),
        ("death_date", "2026-11-10"),
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

/// E is `active`, with a November run `calculated` and a line of AED 3,000.
struct World {
    t: Booted,
    hr: User,
    md: User,
    e: Employee,
    line: PayrollLine,
}

async fn world() -> World {
    world_on(support::boot().await).await
}

/// [`world`] on the app `t`.
async fn world_on(t: Booted) -> World {
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let e = t.employee_in(60, EmployeeStatus::Active).await;
    let run = t.payroll_run("2026-11", PayrollRunStatus::Calculated).await;
    let line = t.payroll_line(&run, &e, 300_000, 300_000).await;
    World { t, hr, md, e, line }
}

impl World {
    /// HR asks on 2026-11-10 and the MD approves the same day.
    async fn die(&self) {
        post(&self.t, nov(10), &self.hr, &self.e, &death())
            .await
            .assert_ok();
        approve(&self.t, &self.md, &self.e, nov(10)).await;
    }

    async fn settlements(&self) -> Vec<FinalSettlement> {
        FinalSettlement::of_employee(self.t.db(), self.e.id)
            .await
            .unwrap()
    }
}

/// The status of leave request `id`.
async fn status_of(w: &World, id: i64) -> LeaveStatus {
    LeaveRequest::find_or_404(w.t.db(), id)
        .await
        .unwrap()
        .status
}

/// AC1: E `active`, death on 2026-11-10 with the certificate, when approved: E
/// is `deceased`, the November line is held with *death*, a settlement with
/// the payee *estate* includes the unpaid salary of 1-10 November, a
/// *cancellation* case from the death variant is open, the three HR tasks are
/// open and `EmployeeDeceased` is emitted.
#[renox::test]
async fn hrm09_ac1_an_approved_death_holds_the_pay_drafts_the_settlement_and_opens_the_work() {
    let w = world().await;
    post(&w.t, nov(10), &w.hr, &w.e, &death()).await.assert_ok();
    assert_eq!(w.t.employee(&w.e).await.status, EmployeeStatus::Active);
    assert!(
        w.settlements().await.is_empty(),
        "nothing before the approval"
    );
    approve(&w.t, &w.md, &w.e, nov(10)).await;

    let e = w.t.employee(&w.e).await;
    assert_eq!(e.status, EmployeeStatus::Deceased);
    assert_eq!(e.date_of_death, Some(nov(10)));
    let line = w.t.line(&w.line).await;
    assert_eq!(line.status, LineStatus::Held);
    assert_eq!(line.held_reason.as_deref(), Some("death"));
    let settlements = w.settlements().await;
    assert_eq!(settlements.len(), 1);
    let s = &settlements[0];
    assert_eq!(s.status, SettlementStatus::Draft);
    assert_eq!(s.payee, "estate");
    assert_eq!(s.reason, "death");
    assert_eq!(s.last_working_day, Some(nov(10)));
    // 10 of the 30 days of November.
    assert_eq!(s.unpaid_salary, 100_000);
    assert_eq!(s.net, 100_000);
    let cases = VisaCase::cancellations_of(w.t.db(), w.e.id).await.unwrap();
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].variant, "death");
    assert_eq!(cases[0].status, VisaCaseStatus::Open);
    let tasks = w.t.tasks.with_key("hr.death:");
    assert_eq!(tasks.len(), 3);
    for kind in ["visa_cancellation", "insurance_claim", "repatriation"] {
        assert!(
            tasks
                .iter()
                .any(|t| t.source_key.as_deref()
                    == Some(format!("hr.death:{}:{kind}", w.e.id).as_str())),
            "{kind}"
        );
    }
    let events = w.t.deceased.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].employee, w.e.employee_id());
    assert_eq!(events[0].date, nov(10));
}

/// AC2: a payslip notification queued for E is cancelled, and every message to
/// E (the party and the portal user) is stopped.
#[renox::test]
async fn hrm09_ac2_a_payslip_notification_queued_for_the_employee_is_cancelled() {
    let w = world().await;
    let account = w.t.user("Ravi").await;
    renox::db::sql("UPDATE wl_hr_employees SET user_id = ? WHERE id = ?")
        .bind(account.id)
        .bind(w.e.id)
        .execute(w.t.db())
        .await
        .unwrap();
    let payslip = NotifyRequest::new("hr.payslip", Subject::new("hr.payroll_line", w.line.id))
        .to(NotifyRecipient::Party { id: PartyId(60) });
    notify::notifier(w.t.state())
        .notify(w.t.state(), payslip)
        .await
        .unwrap();
    assert_eq!(w.t.notices.waiting_of_type("hr.payslip").len(), 1);

    w.die().await;

    assert!(w.t.notices.waiting_of_type("hr.payslip").is_empty());
    let stopped = w.t.notices.stopped();
    assert_eq!(stopped.len(), 1);
    assert_eq!(
        stopped[0].0,
        [
            NotifyRecipient::Party { id: PartyId(60) },
            NotifyRecipient::User { id: account.id }
        ]
    );
    // The portal is off as well.
    assert!(
        workforce(w.t.state())
            .access_disabled(w.t.db(), account.id)
            .await
            .unwrap()
    );
}

/// AC1 with the real wl-ticketing: the three HR tasks of the death are three
/// open tickets about E, one per kind (the source key `hr.death:<id>:<kind>`),
/// assigned to the HR user.
#[renox::test]
async fn hrm09_ac1_the_three_hr_tasks_are_real_tickets_for_hr() {
    let w = world_on(support::boot_with_ticketing().await).await;
    w.die().await;

    let tickets = w.t.tickets().await;
    assert_eq!(tickets.len(), 3);
    for kind in ["visa_cancellation", "insurance_claim", "repatriation"] {
        let key = format!("hr.death:{}:{kind}", w.e.id);
        let found: Vec<_> = tickets
            .iter()
            .filter(|(ticket, _)| ticket.source_key.as_deref() == Some(key.as_str()))
            .collect();
        assert_eq!(found.len(), 1, "{kind}");
        let (ticket, _) = found[0];
        assert!(ticket.is_open(), "{kind}");
        assert_eq!(ticket.assignee_user_id, Some(w.hr.id), "{kind}");
        assert_eq!(
            Subject::new(&ticket.subject_type, ticket.subject_id),
            w.e.subject(),
            "{kind}"
        );
    }
    assert_eq!(w.t.deceased.all().len(), 1);
}

/// The message `id` as wl-notify stores it.
async fn stored(t: &Booted, id: notify::MessageId) -> Message {
    Message::find(t.db(), id.0)
        .await
        .unwrap()
        .expect("the message is stored")
}

/// AC2 with the real wl-notify: a payslip message queued for E (to the party
/// and to the portal user) is cancelled when the death is approved (stored as
/// *suppressed* with the reason), and a message asked for afterwards is
/// suppressed at once and never sent.
#[renox::test]
async fn hrm09_ac2_with_the_real_notify_the_queued_payslip_is_stopped() {
    register_type(NotificationTypeDef::new("hr.payslip", "hr").template(
        NotifyChannel::Email,
        "en",
        Some("Your payslip"),
        "Your payslip is ready.",
    ));
    let w = world_on(support::boot_with_notify().await).await;
    w.t.parties.email(60, "ravi@example.com");
    let account = w.t.user("Ravi").await;
    renox::db::sql("UPDATE wl_hr_employees SET user_id = ? WHERE id = ?")
        .bind(account.id)
        .bind(w.e.id)
        .execute(w.t.db())
        .await
        .unwrap();
    let payslip = |recipient: NotifyRecipient| {
        NotifyRequest::new("hr.payslip", Subject::new("hr.payroll_line", w.line.id)).to(recipient)
    };
    let to_party = notify::notifier(w.t.state())
        .notify(
            w.t.state(),
            payslip(NotifyRecipient::Party { id: PartyId(60) }),
        )
        .await
        .unwrap();
    let to_user = notify::notifier(w.t.state())
        .notify(
            w.t.state(),
            payslip(NotifyRecipient::User { id: account.id }),
        )
        .await
        .unwrap();
    for id in to_party.iter().chain(&to_user) {
        let queued = stored(&w.t, *id).await;
        assert!(
            matches!(
                queued.status,
                MessageStatus::Queued | MessageStatus::Deferred
            ),
            "waiting: {:?} {:?}",
            queued.status,
            queued.suppress_reason
        );
    }

    w.die().await;

    for id in to_party.iter().chain(&to_user) {
        let stopped = stored(&w.t, *id).await;
        assert_eq!(stopped.status, MessageStatus::Suppressed);
        assert_eq!(
            stopped.suppress_reason.as_deref(),
            Some("employee_deceased")
        );
    }
    // Nothing new reaches E either.
    let later = notify::notifier(w.t.state())
        .notify(
            w.t.state(),
            payslip(NotifyRecipient::Party { id: PartyId(60) }),
        )
        .await
        .unwrap();
    let later = stored(&w.t, later[0]).await;
    assert_eq!(later.status, MessageStatus::Suppressed);
    assert_eq!(later.suppress_reason.as_deref(), Some("recipient_stopped"));
}

/// AC3: E's November line is marked *settled in the final settlement*; no
/// off-cycle run is proposed for it by `HRM.RUL.12`, whatever happens to the
/// run, and the line cannot be released by hand.
#[renox::test]
async fn hrm09_ac3_the_line_is_settled_in_the_settlement_and_never_released() {
    let w = world().await;
    w.die().await;

    let s = w.settlements().await.remove(0);
    let line = w.t.line(&w.line).await;
    assert_eq!(line.settled_in_settlement, Some(s.id));
    assert_eq!(line.settled_amount, 100_000);

    // The run is sent and paid; the daily job still proposes nothing.
    renox::db::sql("UPDATE wl_hr_payroll_runs SET status = 'paid' WHERE id = ?")
        .bind(line.run_id)
        .execute(w.t.db())
        .await
        .unwrap();
    w.t.on(nov(20), async {}).await;
    let report = release::run(w.t.state()).await.unwrap();
    assert_eq!(report.lines, 0);
    let off = PayrollRun::query()
        .where_(PayrollRun::RUN_TYPE.eq("off_cycle"))
        .get(w.t.db())
        .await
        .unwrap();
    assert!(off.is_empty());
    let refused = release::line_fixed(w.t.state(), line.id).await;
    assert!(matches!(
        refused,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
    assert_eq!(w.t.line(&w.line).await.status, LineStatus::Held);
}

/// AC4: `pay` of E's settlement without the estate's representative recorded
/// is refused; after HR records the representative and the document, it pays.
#[renox::test]
async fn hrm09_ac4_the_settlement_is_not_paid_without_the_estates_representative() {
    let w = world().await;
    let approver = w.t.user_with("Basma", "settle_appr").await;
    let payer = w.t.user_with("Pia", "settle_pay").await;
    w.die().await;
    let s = w.settlements().await.remove(0);
    w.t.settle_transition(&approver, &s, "approve")
        .await
        .assert_status(303);

    let pay = [
        ("amount", "1000"),
        ("method", "transfer"),
        ("reference", "TRF-1"),
    ];
    w.t.settle(&payer, &s, "pay", &pay).await.assert_status(422);
    assert_eq!(w.t.settlement(&s).await.status, SettlementStatus::Approved);

    // The representative is recorded on the settlement (a draft only takes
    // the figures again, so the approval above is not repeated here).
    renox::db::sql(
        "UPDATE wl_hr_final_settlements SET representative = ?, right_document = ? WHERE id = ?",
    )
    .bind("Sara Rashid (widow)")
    .bind("Court order 77/2026")
    .bind(s.id)
    .execute(w.t.db())
    .await
    .unwrap();
    w.t.settle(&payer, &s, "pay", &pay).await.assert_status(303);
    let paid = w.t.settlement(&s).await;
    assert_eq!(paid.status, SettlementStatus::Paid);
    assert_eq!(
        paid.acknowledgement_requested_on, None,
        "not for the estate"
    );
}

/// AC4 (record): HR's preparer records the representative with the figures of
/// the draft through the amounts route.
#[renox::test]
async fn hrm09_ac4_the_preparer_records_the_representative_on_the_draft() {
    let w = world().await;
    let preparer = w.t.user_with("Amal", "settle_prep").await;
    w.die().await;
    let s = w.settlements().await.remove(0);
    w.t.settle(
        &preparer,
        &s,
        "amounts",
        &[
            ("unpaid_salary", "1000"),
            ("representative", "Sara Rashid (widow)"),
            ("right_document", "Court order 77/2026"),
        ],
    )
    .await
    .assert_status(303);
    let s = w.t.settlement(&s).await;
    assert_eq!(s.representative.as_deref(), Some("Sara Rashid (widow)"));
    assert_eq!(s.right_document.as_deref(), Some("Court order 77/2026"));
    assert_eq!(s.payee, "estate");
}

/// AC5: without the certificate or the authority's notice, without the date of
/// death, or with a date of death in the future, it is refused and nothing is requested.
#[renox::test]
async fn hrm09_ac5_without_the_certificate_or_the_date_or_with_a_future_date_it_is_refused() {
    let w = world().await;
    let without = [
        ("transition", "record_death"),
        ("reason", "died in service"),
        ("death_date", "2026-11-10"),
    ];
    post(&w.t, nov(10), &w.hr, &w.e, &without)
        .await
        .assert_status(422);
    let undated = [
        ("transition", "record_death"),
        ("reason", "died in service"),
        ("death_certificate", "DC-9"),
    ];
    post(&w.t, nov(10), &w.hr, &w.e, &undated)
        .await
        .assert_status(422);
    let future = [
        ("transition", "record_death"),
        ("reason", "died in service"),
        ("death_certificate", "DC-9"),
        ("death_date", "2026-11-11"),
    ];
    post(&w.t, nov(10), &w.hr, &w.e, &future)
        .await
        .assert_status(422);
    let pending = ApprovalRequest::pending_about(w.t.db(), &w.e.subject())
        .await
        .unwrap();
    assert!(pending.is_empty());
    assert_eq!(w.t.employee(&w.e).await.status, EmployeeStatus::Active);
    assert!(w.t.deceased.all().is_empty());
}

/// Edge: from `notice` or `absconded` the existing settlement keeps its id and
/// gets the payee *estate* and the reason *death*; the lines held for the
/// absence report are held for the death; the open cancellation case becomes
/// the death variant.
#[renox::test]
async fn hrm09_an_existing_settlement_and_case_are_kept_and_changed() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let e = t.employee_in(60, EmployeeStatus::Absconded).await;
    let run = t.payroll_run("2026-11", PayrollRunStatus::Calculated).await;
    let line = t.payroll_line(&run, &e, 300_000, 300_000).await;
    renox::db::sql(
        "UPDATE wl_hr_payroll_lines SET status = 'held', held_reason = 'absence report' WHERE id = ?",
    )
    .bind(line.id)
    .execute(t.db())
    .await
    .unwrap();
    let settlement = t
        .settlement_in(&e, SettlementStatus::Draft, |s| {
            s.reason = "absconding".into();
            s.unpaid_salary = 50_000;
            s.net = 50_000;
        })
        .await;
    let case = t
        .visa_in(&e, "cancellation", VisaCaseStatus::Open, |c| {
            c.variant = "absconding".into();
        })
        .await;

    post(&t, nov(10), &hr, &e, &death()).await.assert_ok();
    approve(&t, &md, &e, nov(10)).await;

    let all = FinalSettlement::of_employee(t.db(), e.id).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, settlement.id);
    assert_eq!(
        (all[0].payee.as_str(), all[0].reason.as_str()),
        ("estate", "death")
    );
    assert_eq!(
        all[0].unpaid_salary, 150_000,
        "50,000 typed plus 100,000 owed"
    );
    assert_eq!(all[0].net, 150_000);
    let line = t.line(&line).await;
    assert_eq!(line.held_reason.as_deref(), Some("death"));
    assert_eq!(line.settled_in_settlement, Some(settlement.id));
    let cases = VisaCase::cancellations_of(t.db(), e.id).await.unwrap();
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].id, case.id);
    assert_eq!(cases[0].variant, "death");
}

/// Edge: leave requests `requested` or `approved` that have not started are
/// cancelled by the system; leave that has started and leave already over stay.
#[renox::test]
async fn hrm09_leave_that_has_not_started_is_cancelled() {
    let w = world().await;
    let leave = |kind: &str, from: u32, to: u32, status: LeaveStatus| LeaveRequest {
        employee_id: w.e.id,
        leave_type: kind.to_owned(),
        from_date: NaiveDate::from_ymd_opt(2026, 12, from).unwrap(),
        to_date: NaiveDate::from_ymd_opt(2026, 12, to).unwrap(),
        status,
        ..LeaveRequest::default()
    };
    let mut ids = Vec::new();
    for request in [
        leave("annual", 1, 10, LeaveStatus::Approved),
        leave("annual", 15, 20, LeaveStatus::Requested),
        leave("annual", 21, 25, LeaveStatus::Rejected),
    ] {
        ids.push(LeaveRequest::create(w.t.db(), request).await.unwrap().id);
    }
    let started = LeaveRequest::create(
        w.t.db(),
        LeaveRequest {
            employee_id: w.e.id,
            leave_type: "annual".into(),
            from_date: nov(5),
            to_date: nov(12),
            status: LeaveStatus::Approved,
            ..LeaveRequest::default()
        },
    )
    .await
    .unwrap();

    w.die().await;

    assert_eq!(status_of(&w, ids[0]).await, LeaveStatus::Cancelled);
    assert_eq!(status_of(&w, ids[1]).await, LeaveStatus::Cancelled);
    assert_eq!(status_of(&w, ids[2]).await, LeaveStatus::Rejected);
    assert_eq!(status_of(&w, started.id).await, LeaveStatus::Approved);
}

/// Edge: a death reported late (date of death 2026-11-04, approved on 11-10)
/// is allowed; a line already in a sent file stays; the salary owed counts to
/// the date of death.
#[renox::test]
async fn hrm09_a_late_report_counts_to_the_date_of_death_and_leaves_sent_lines() {
    let w = world().await;
    let sent =
        w.t.payroll_run("2026-10", PayrollRunStatus::Submitted)
            .await;
    let october = w.t.payroll_line(&sent, &w.e, 300_000, 300_000).await;
    let late = [
        ("transition", "record_death"),
        ("reason", "found on 10 November"),
        ("authority_notice", "N-5"),
        ("death_date", "2026-11-04"),
    ];
    post(&w.t, nov(10), &w.hr, &w.e, &late).await.assert_ok();
    approve(&w.t, &w.md, &w.e, nov(10)).await;

    assert_eq!(w.t.employee(&w.e).await.date_of_death, Some(nov(4)));
    let s = w.settlements().await.remove(0);
    // 4 of 30 days.
    assert_eq!(s.unpaid_salary, 40_000);
    let october = w.t.line(&october).await;
    assert_eq!(october.status, LineStatus::Included);
    assert_eq!(october.settled_in_settlement, None);
    assert_eq!(w.t.deceased.all()[0].date, nov(4));
}
