//! BL-HRM-21 (#211), `HRM.ACL.05`: the worker stops a change of pay account
//! during the cooling from the signed link of the SMS or the portal; payroll
//! waives the cooling only with an MD's approval and a reason; an SMS that
//! gave up keeps the account cooling and gives HR a task until the worker is
//! reached.

mod support;

use renox::prelude::*;
use wl_hr::pay_account;
use wl_hr::{Employee, EmployeeStatus, PayAccount, PayAccountStatus};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::notify::{MessageGaveUp, MessageId, NotifyChannel, NotifyRecipient};
use wl_kernel::tickets::{Assignee, CONFIDENTIAL_TASK_CATEGORY};

use support::{Booted, nov, users};

const NEW_IBAN: &str = "AE550331234567890121234";
const OLD_IBAN: &str = "AE100261234567890124321";
const CHANGE: &str = "hr.pay_account_change";

/// A (HR) asks for the new account, B approves it, C is the payroll user who
/// waives the cooling, M and N are MDs, and H is the HR task holder. E, Ravi
/// Kumar, is paid into the old account.
struct World {
    t: Booted,
    a: User,
    b: User,
    c: User,
    md: User,
    other_md: User,
    hr: User,
    e: Employee,
    old: PayAccount,
}

async fn world() -> World {
    world_on(support::boot().await).await
}

async fn world_on(t: Booted) -> World {
    let a = t.user_with("Hana", "hr_pay").await;
    let b = t.user_with("Basel", "pay_approver").await;
    let c = t.user_with("Cara", "payroll_appr").await;
    let md = t.user_with("Mazen", "owner").await;
    let other_md = t.user_with("Nadia", "owner").await;
    let hr = t.user_with("Hira", "hr").await;
    let e = t.employee_in(61, EmployeeStatus::Active).await;
    t.parties.name(61, "Ravi Kumar");
    t.parties.mobile(61, "+971500000061");
    let old = t
        .pay_account_in(&e, PayAccountStatus::Active, OLD_IBAN)
        .await;
    World {
        t,
        a,
        b,
        c,
        md,
        other_md,
        hr,
        e,
        old,
    }
}

/// `user` asks for `NEW_IBAN` for `employee` and B approves it on 2026-11-02:
/// it cools until the 7th.
async fn cooling_by(w: &World, user: &User, employee: &Employee) -> PayAccount {
    let form = [
        ("channel", "bank_account"),
        ("account_number", NEW_IBAN),
        ("routing_code", "123456789"),
        ("holder_name", "Ravi Kumar"),
    ];
    w.t.request_account(user, employee, &form)
        .await
        .assert_status(303);
    let account = w.t.accounts_of(employee).await.pop().unwrap();
    w.t.on(nov(2), w.t.pay_transition(&w.b, &account, "approve", ""))
        .await
        .assert_status(303);
    w.t.account(&account).await
}

async fn cooling(w: &World) -> PayAccount {
    cooling_by(w, &w.a, &w.e).await
}

/// The path and query of the link in the SMS.
fn stop_path(w: &World) -> String {
    let sms = w.t.notices.of_type(CHANGE);
    let link = sms[0].data["link"].as_str().unwrap().to_owned();
    let rest = link.split_once("://").map_or(link.as_str(), |(_, r)| r);
    rest[rest.find('/').unwrap()..].to_owned()
}

/// The clock to `day`, then the daily activation job.
async fn job_on(w: &World, day: renox::chrono::NaiveDate) {
    w.t.on(day, async {}).await;
    w.t.app.run_scheduled(pay_account::TASK).await.unwrap();
}

/// The SMS of the account gave up.
async fn gave_up(w: &World, account: &PayAccount) {
    w.t.state()
        .emit(MessageGaveUp {
            message: MessageId(1),
            type_key: CHANGE.into(),
            subject: account.subject(),
            subject_label: None,
            recipient: NotifyRecipient::Address {
                channel: NotifyChannel::Sms,
                address: "+971500000061".into(),
            },
            gave_up_at: renox::db::now(),
            contact_point: None,
            channels_tried: Vec::new(),
            statutory: false,
            reactions: Vec::new(),
            alternative_of: None,
            contact_of: None,
        })
        .await
        .unwrap();
}

/// The approval requests about `account` that are pending.
async fn pending(w: &World, account: &PayAccount) -> Vec<ApprovalRequest> {
    ApprovalRequest::pending_about(w.t.db(), &account.subject())
        .await
        .unwrap()
}

/// AC1: given a cooling account, when E taps the stop link, it is `rejected`,
/// the old account stays active, a security ticket exists and the MD is told.
/// The link works once.
#[renox::test]
async fn hrm21_ac1_the_worker_stops_the_change_from_the_link_once() {
    let w = world().await;
    let new = cooling(&w).await;
    let path = stop_path(&w);
    w.t.app.logout();

    w.t.on(nov(3), w.t.app.get(&path))
        .await
        .assert_ok()
        .assert_see("Stop the change")
        .assert_see("…1234");
    assert_eq!(w.t.account(&new).await.status, PayAccountStatus::Cooling);
    w.t.on(nov(3), w.t.app.post(&path, &[]))
        .await
        .assert_ok()
        .assert_see("The change was stopped");

    let stopped = w.t.account(&new).await;
    assert_eq!(stopped.status, PayAccountStatus::Rejected);
    assert!(stopped.stopped_by_worker);
    assert_eq!(w.t.account(&w.old).await.status, PayAccountStatus::Active);
    let tickets = w.t.tasks.with_key("hr.pay_account_stopped:");
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].category_key, CONFIDENTIAL_TASK_CATEGORY);
    assert_eq!(tickets[0].assignee, Some(Assignee::user(w.md.id)));
    let told = w.t.notices.of_type("hr.pay_account_stopped");
    assert_eq!(told.len(), 1);
    assert_eq!(users(&told[0]), vec![w.md.id, w.other_md.id]);

    // The link works once: a second tap changes nothing.
    w.t.on(nov(3), w.t.app.post(&path, &[]))
        .await
        .assert_ok()
        .assert_see("can no longer be stopped");
    assert_eq!(w.t.tasks.with_key("hr.pay_account_stopped:").len(), 1);
    // The employee's job later finds nothing to activate.
    job_on(&w, nov(7)).await;
    assert_eq!(w.t.account(&w.old).await.status, PayAccountStatus::Active);
}

/// AC1 with the real wl-ticketing: the security ticket is a ticket of the
/// confidential category about the stopped account, assigned to the MD.
#[renox::test]
async fn hrm21_ac1_the_security_task_is_a_real_confidential_ticket_for_the_md() {
    let w = world_on(support::boot_with_ticketing().await).await;
    let new = cooling(&w).await;
    let path = stop_path(&w);
    w.t.app.logout();
    w.t.on(nov(3), w.t.app.post(&path, &[]))
        .await
        .assert_ok()
        .assert_see("The change was stopped");

    let tickets = w.t.tickets().await;
    assert_eq!(tickets.len(), 1);
    let (ticket, category) = &tickets[0];
    assert_eq!(category, CONFIDENTIAL_TASK_CATEGORY);
    assert!(ticket.confidential);
    assert_eq!(ticket.assignee_user_id, Some(w.md.id));
    assert_eq!(
        wl_kernel::Subject::new(&ticket.subject_type, ticket.subject_id),
        new.subject()
    );
    assert!(ticket.title.contains("Ravi Kumar"));
    assert!(ticket.title.contains("1234"));
    assert!(ticket.is_open());

    // A second tap changes nothing, and no second ticket is opened.
    w.t.on(nov(3), w.t.app.post(&path, &[]))
        .await
        .assert_ok()
        .assert_see("can no longer be stopped");
    assert_eq!(w.t.tickets().await.len(), 1);
}

/// The link is signed and expires with the cooling period.
#[renox::test]
async fn hrm21_a_tampered_or_expired_link_is_refused() {
    let w = world().await;
    let new = cooling(&w).await;
    let path = stop_path(&w);
    w.t.app.logout();

    w.t.on(nov(3), w.t.app.post(&format!("{path}0"), &[]))
        .await
        .assert_status(403);
    w.t.on(nov(8), w.t.app.post(&path, &[]))
        .await
        .assert_status(403);

    assert_eq!(w.t.account(&new).await.status, PayAccountStatus::Cooling);
    assert!(w.t.tasks.with_key("hr.pay_account_stopped:").is_empty());
}

/// The employee stops the change in the portal; another user cannot. HR with
/// `employees.manage` can stop it too.
#[renox::test]
async fn hrm21_the_employee_stops_in_the_portal_and_hr_can_stop_too() {
    let w = world().await;
    let (employee, user) = w.t.ravi().await;
    w.t.parties.name(50, "Ravi Kumar");
    w.t.parties.mobile(50, "+971500000050");
    let form = [
        ("channel", "bank_account"),
        ("account_number", NEW_IBAN),
        ("routing_code", "123456789"),
        ("holder_name", "Ravi Kumar"),
    ];
    w.t.app.acting_as(&user);
    w.t.app
        .post("/hr/portal/pay-accounts", &form)
        .await
        .assert_status(303);
    let account = w.t.accounts_of(&employee).await.remove(0);
    w.t.on(nov(2), w.t.pay_transition(&w.b, &account, "approve", ""))
        .await
        .assert_status(303);
    let path = format!("/hr/portal/pay-accounts/{}/stop", account.id);

    let stranger = w.t.user("Sam").await;
    w.t.app.acting_as(&stranger);
    w.t.app.post(&path, &[]).await.assert_status(403);
    w.t.app.acting_as(&user);
    w.t.app.post(&path, &[]).await.assert_status(303);
    assert_eq!(
        w.t.account(&account).await.status,
        PayAccountStatus::Rejected
    );
    assert!(w.t.account(&account).await.stopped_by_worker);

    // HR stops another one.
    let new = cooling(&w).await;
    w.t.pay_transition(&w.a, &new, "stop", "")
        .await
        .assert_status(303);
    let stopped = w.t.account(&new).await;
    assert_eq!(stopped.status, PayAccountStatus::Rejected);
    assert_eq!(stopped.stopped_by, Some(w.a.id));
    assert!(!stopped.stopped_by_worker);
    assert_eq!(w.t.tasks.with_key("hr.pay_account_stopped:").len(), 2);
}

/// AC2: `waive_cooling` by payroll user C with the reason "failed WPS line,
/// verified EID" creates an approval request; while it is pending, or after M
/// rejects, the account stays `cooling`; when M approves it is `active`.
#[renox::test]
async fn hrm21_ac2_a_waiver_needs_an_md_and_a_reason() {
    let w = world().await;
    let new = cooling(&w).await;
    let route = format!("/hr/pay-accounts/{}/waive", new.id);

    // Without a reason, or by a user without payroll.approve: refused.
    w.t.app.acting_as(&w.c);
    w.t.app
        .post(&route, &[("reason", "  ")])
        .await
        .assert_status(422);
    w.t.app.acting_as(&w.a);
    w.t.app
        .post(&route, &[("reason", "failed WPS line")])
        .await
        .assert_status(403);
    assert!(pending(&w, &new).await.is_empty());

    w.t.app.acting_as(&w.c);
    w.t.app
        .post(&route, &[("reason", "failed WPS line, verified EID")])
        .await
        .assert_status(303);
    let requests = pending(&w, &new).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, "hr.waive_cooling");
    assert_eq!(requests[0].requester_id, w.c.id);
    assert_eq!(
        requests[0].payload.0["reason"],
        "failed WPS line, verified EID"
    );
    assert_eq!(w.t.account(&new).await.status, PayAccountStatus::Cooling);
    assert!(w.t.pay_changed.all().is_empty());

    // The MD says no: it stays cooling and a new waiver can be asked.
    approvals::reject(w.t.state(), requests[0].id, &w.md, "verify in person first")
        .await
        .unwrap();
    assert_eq!(w.t.account(&new).await.status, PayAccountStatus::Cooling);
    w.t.app.acting_as(&w.c);
    w.t.app
        .post(&route, &[("reason", "worker verified in person")])
        .await
        .assert_status(303);

    // The MD says yes.
    let request = pending(&w, &new).await.remove(0);
    approvals::approve(w.t.state(), request.id, &w.md, "")
        .await
        .unwrap();
    let active = w.t.account(&new).await;
    assert_eq!(active.status, PayAccountStatus::Active);
    assert_eq!(
        active.waive_reason.as_deref(),
        Some("worker verified in person")
    );
    assert_eq!(w.t.account(&w.old).await.status, PayAccountStatus::Retired);
    assert_eq!(w.t.pay_changed.all().len(), 1);
}

/// The user who asks for the waiver (C) cannot approve it, not even as an MD.
#[renox::test]
async fn hrm21_the_user_who_asks_never_approves_the_waiver() {
    let w = world().await;
    let new = cooling(&w).await;
    let both = w.t.user_with("Carim", "payroll_appr").await;
    both.assign_role(w.t.db(), "owner").await.unwrap();
    w.t.app.acting_as(&both);
    w.t.app
        .post(
            &format!("/hr/pay-accounts/{}/waive", new.id),
            &[("reason", "failed WPS line")],
        )
        .await
        .assert_status(303);
    let request = pending(&w, &new).await.remove(0);

    let refused = approvals::approve(w.t.state(), request.id, &both, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)));
    assert_eq!(w.t.account(&new).await.status, PayAccountStatus::Cooling);
    approvals::approve(w.t.state(), request.id, &w.md, "")
        .await
        .unwrap();
    assert_eq!(w.t.account(&new).await.status, PayAccountStatus::Active);
}

/// AC3: M requested the pay account change: M cannot approve the waiver, MD N
/// can.
#[renox::test]
async fn hrm21_ac3_the_requester_of_the_change_cannot_approve_its_waiver() {
    let w = world().await;
    let m = w.t.user_with("Basel2", "hr_md").await;
    m.assign_role(w.t.db(), "owner").await.unwrap();
    let new = cooling_by(&w, &m, &w.e).await;
    assert_eq!(new.requested_by, Some(m.id));
    w.t.app.acting_as(&w.c);
    w.t.app
        .post(
            &format!("/hr/pay-accounts/{}/waive", new.id),
            &[("reason", "failed WPS line, verified EID")],
        )
        .await
        .assert_status(303);
    let request = pending(&w, &new).await.remove(0);

    let refused = approvals::approve(w.t.state(), request.id, &m, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)));
    assert_eq!(w.t.account(&new).await.status, PayAccountStatus::Cooling);

    approvals::approve(w.t.state(), request.id, &w.other_md, "")
        .await
        .unwrap();
    assert_eq!(w.t.account(&new).await.status, PayAccountStatus::Active);
}

/// AC4: `MessageGaveUp` for the SMS: on the cooling end date the account stays
/// `cooling` and HR has a task; after HR records the worker reached, it
/// activates at once.
#[renox::test]
async fn hrm21_ac4_a_message_that_gave_up_keeps_the_account_cooling_until_the_worker_is_reached() {
    let w = world().await;
    let new = cooling(&w).await;

    gave_up(&w, &new).await;

    assert!(w.t.account(&new).await.notice_undelivered_at.is_some());
    let tasks = w.t.tasks.with_key("hr.pay_account_notice:");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee, Some(Assignee::user(w.hr.id)));
    job_on(&w, nov(7)).await;
    job_on(&w, nov(8)).await;
    assert_eq!(w.t.account(&new).await.status, PayAccountStatus::Cooling);
    assert_eq!(w.t.account(&w.old).await.status, PayAccountStatus::Active);

    // Only a user who manages the employee records it, with how and when.
    let route = format!("/hr/pay-accounts/{}/reached", new.id);
    w.t.app.acting_as(&w.b);
    w.t.app
        .post(&route, &[("by", "Hana"), ("method", "call")])
        .await
        .assert_status(403);
    w.t.app.acting_as(&w.a);
    w.t.app
        .post(&route, &[("by", "Hana"), ("method", "telepathy")])
        .await
        .assert_status(422);
    w.t.app
        .post(
            &route,
            &[("by", "Hana"), ("method", "call"), ("on", "2030-01-01")],
        )
        .await
        .assert_status(422);
    assert_eq!(w.t.account(&new).await.status, PayAccountStatus::Cooling);
    w.t.app
        .post(&route, &[("by", "Hana"), ("method", "call")])
        .await
        .assert_status(303);

    let active = w.t.account(&new).await;
    assert_eq!(active.status, PayAccountStatus::Active);
    assert_eq!(active.reached_by.as_deref(), Some("Hana"));
    assert_eq!(active.reached_method.as_deref(), Some("call"));
    assert_eq!(active.reached_recorded_by, Some(w.a.id));
    assert_eq!(w.t.account(&w.old).await.status, PayAccountStatus::Retired);
    assert_eq!(w.t.pay_changed.all().len(), 1);
}

/// AC4 with the real wl-ticketing: the HR task is a real ticket for the HR
/// holder, found by its source key, so a second give-up opens none.
#[renox::test]
async fn hrm21_ac4_the_hr_task_is_a_real_ticket_found_by_its_source_key() {
    let w = world_on(support::boot_with_ticketing().await).await;
    let new = cooling(&w).await;

    gave_up(&w, &new).await;
    gave_up(&w, &new).await;

    let tickets = w.t.tickets().await;
    assert_eq!(tickets.len(), 1, "the same open task is used");
    let (ticket, category) = &tickets[0];
    assert_eq!(category, "task");
    assert!(!ticket.confidential);
    assert_eq!(ticket.assignee_user_id, Some(w.hr.id));
    assert_eq!(
        wl_kernel::Subject::new(&ticket.subject_type, ticket.subject_id),
        new.subject()
    );
    assert!(ticket.title.contains("1234"));
    assert!(
        ticket
            .source_key
            .as_deref()
            .is_some_and(|key| key.starts_with("hr.pay_account_notice:"))
    );
}

/// AC5: the worker reached is recorded before the cooling end date: the
/// account activates on the end date, not earlier.
#[renox::test]
async fn hrm21_ac5_reaching_the_worker_early_activates_on_the_end_date_not_before() {
    let w = world().await;
    let new = cooling(&w).await;
    gave_up(&w, &new).await;

    w.t.on(nov(3), async {}).await;
    w.t.app.acting_as(&w.a);
    w.t.app
        .post(
            &format!("/hr/pay-accounts/{}/reached", new.id),
            &[("by", "Hana"), ("method", "visit")],
        )
        .await
        .assert_status(303);
    assert_eq!(w.t.account(&new).await.status, PayAccountStatus::Cooling);
    job_on(&w, nov(6)).await;
    assert_eq!(w.t.account(&new).await.status, PayAccountStatus::Cooling);

    job_on(&w, nov(7)).await;

    assert_eq!(w.t.account(&new).await.status, PayAccountStatus::Active);
}

/// A give-up of the e-mail copy, or of another message, does not say the
/// worker was not told.
#[renox::test]
async fn hrm21_only_the_give_up_of_the_sms_makes_the_notice_undelivered() {
    let w = world().await;
    let new = cooling(&w).await;

    w.t.state()
        .emit(MessageGaveUp {
            message: MessageId(2),
            type_key: CHANGE.into(),
            subject: new.subject(),
            subject_label: None,
            recipient: NotifyRecipient::Address {
                channel: NotifyChannel::Email,
                address: "ravi@example.com".into(),
            },
            gave_up_at: renox::db::now(),
            contact_point: None,
            channels_tried: Vec::new(),
            statutory: false,
            reactions: Vec::new(),
            alternative_of: None,
            contact_of: None,
        })
        .await
        .unwrap();

    assert!(w.t.account(&new).await.notice_undelivered_at.is_none());
    assert!(w.t.tasks.with_key("hr.pay_account_notice:").is_empty());
    job_on(&w, nov(7)).await;
    assert_eq!(w.t.account(&new).await.status, PayAccountStatus::Active);
}
