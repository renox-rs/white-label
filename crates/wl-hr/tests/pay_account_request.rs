//! BL-HRM-19 (#209), `HRM.ACL.05`: a new pay account is requested by HR or by
//! the employee, checked for format and holder name, approved by another user,
//! and the worker is told by SMS on the number on record before the request,
//! naming the last 4 digits and the date it takes effect.

mod support;

use renox::prelude::*;
use wl_hr::{Employee, EmployeeStatus, PayAccount, PayAccountStatus};
use wl_kernel::notify::{NotifyChannel, NotifyRecipient, fill, notification_type};
use wl_kernel::tickets::Assignee;

use support::{Booted, nov, users};

/// A UAE IBAN that passes the mod-97 check, ending 1234.
const NEW_IBAN: &str = "AE550331234567890121234";
/// The IBAN of the old account, ending 4321.
const OLD_IBAN: &str = "AE100261234567890124321";
const CHANGE: &str = "hr.pay_account_change";

/// HR user A (who may also approve, to show the requester cannot), approver B
/// and employee E, Ravi Kumar, with the mobile number +971500000061 on record.
struct World {
    t: Booted,
    a: User,
    b: User,
    e: Employee,
}

async fn world() -> World {
    world_on(support::boot().await).await
}

async fn world_on(t: Booted) -> World {
    let a = t.user_with("Hana", "hr_pay").await;
    let b = t.user_with("Basel", "pay_approver").await;
    let e = t.employee_in(61, EmployeeStatus::Active).await;
    t.parties.name(61, "Ravi Kumar");
    t.parties.mobile(61, "+971500000061");
    World { t, a, b, e }
}

/// The form fields of a bank account request for `iban`.
fn fields(iban: &str) -> Vec<(&str, &str)> {
    vec![
        ("channel", "bank_account"),
        ("account_number", iban),
        ("routing_code", "123456789"),
        ("holder_name", "Ravi Kumar"),
    ]
}

/// A asks for `NEW_IBAN` and the account is returned.
async fn requested(w: &World) -> PayAccount {
    w.t.request_account(&w.a, &w.e, &fields(NEW_IBAN))
        .await
        .assert_status(303);
    w.t.accounts_of(&w.e).await.remove(0)
}

/// AC1: HR user A requests a new IBAN for E: the account is
/// `pending_approval`; with an invalid IBAN it is refused and nothing is made.
#[renox::test]
async fn hrm19_ac1_a_valid_iban_is_pending_approval_and_an_invalid_one_is_refused() {
    let w = world().await;

    w.t.request_account(&w.a, &w.e, &fields("AE000331234567890121234"))
        .await
        .assert_status(422);
    assert!(w.t.accounts_of(&w.e).await.is_empty());

    let account = requested(&w).await;
    assert_eq!(account.status, PayAccountStatus::PendingApproval);
    assert_eq!(account.last4, "1234");
    assert_eq!(account.requested_by, Some(w.a.id));
    // The approver is told; the requester is not.
    let told = w.t.notices.of_type("hr.pay_account_requested");
    assert_eq!(told.len(), 1);
    assert_eq!(users(&told[0]), vec![w.b.id]);
    assert!(
        !format!("{:?}", told[0]).contains(NEW_IBAN),
        "no full number"
    );
}

/// AC2: A cannot approve; B approves on 2026-11-02; the SMS goes to E's number
/// on record before the request, naming "…1234" and 2026-11-07; the account is
/// `cooling`. A number changed after the request is never used.
#[renox::test]
async fn hrm19_ac2_the_requester_cannot_approve_and_the_sms_goes_to_the_old_number() {
    let w = world().await;
    let account = requested(&w).await;

    w.t.on(nov(2), w.t.pay_transition(&w.a, &account, "approve", ""))
        .await
        .assert_status(422);
    assert_eq!(
        w.t.account(&account).await.status,
        PayAccountStatus::PendingApproval
    );
    assert!(w.t.notices.of_type(CHANGE).is_empty());

    // The number is changed in the same breath: the notice still goes to the
    // one on record before the request.
    w.t.parties.mobile(61, "+971599999999");
    w.t.on(nov(2), w.t.pay_transition(&w.b, &account, "approve", ""))
        .await
        .assert_status(303);

    let account = w.t.account(&account).await;
    assert_eq!(account.status, PayAccountStatus::Cooling);
    assert_eq!(account.approved_by, Some(w.b.id));
    assert_eq!(account.cooling_ends_on, Some(nov(7)));
    let sms = w.t.notices.of_type(CHANGE);
    assert_eq!(sms.len(), 1);
    assert_eq!(
        sms[0].recipients,
        vec![NotifyRecipient::Address {
            channel: NotifyChannel::Sms,
            address: "+971500000061".into()
        }]
    );
    assert_eq!(sms[0].data["last4"], "1234");
    assert_eq!(sms[0].data["date"], "2026-11-07");
    let link = sms[0].data["link"].as_str().unwrap();
    assert!(link.contains(&format!("/hr/pay-accounts/{}/stop", account.id)));
    assert!(!sms[0].data.to_string().contains(NEW_IBAN));
    assert!(account.notice_undelivered_at.is_none());

    // The text the worker reads: the registered SMS template filled with the
    // data of the request names the last 4 digits and the date.
    let kind = notification_type(CHANGE).expect("the type is registered");
    let template = kind
        .template_for(NotifyChannel::Sms, "en")
        .expect("an SMS template");
    let text = fill(&template.body, &sms[0].data);
    assert!(text.contains("1234"), "{text}");
    assert!(text.contains("2026-11-07"), "{text}");
    assert!(text.contains(link), "{text}");
    assert!(!text.contains(NEW_IBAN), "{text}");
}

/// AC3: a holder name that differs from the employee's is refused without an
/// explanation; with "name on card is the short form" it is accepted and the
/// approver sees it, and only the last 4 digits.
#[renox::test]
async fn hrm19_ac3_a_name_mismatch_needs_an_explanation_the_approver_sees() {
    let w = world().await;
    let mut form = fields(NEW_IBAN);
    form[3] = ("holder_name", "R. Kumar");

    w.t.request_account(&w.a, &w.e, &form)
        .await
        .assert_status(422);
    assert!(w.t.accounts_of(&w.e).await.is_empty());

    form.push(("mismatch_explanation", "name on card is the short form"));
    w.t.request_account(&w.a, &w.e, &form)
        .await
        .assert_status(303);
    let account = w.t.accounts_of(&w.e).await.remove(0);
    assert_eq!(
        account.mismatch_explanation.as_deref(),
        Some("name on card is the short form")
    );

    w.t.app.acting_as(&w.b);
    w.t.app
        .get(&format!("/hr/pay-accounts/{}", account.id))
        .await
        .assert_ok()
        .assert_see("name on card is the short form")
        .assert_see("…1234")
        .assert_dont_see(NEW_IBAN);
    // Case and spacing do not make a mismatch.
    w.t.parties.name(62, "Mona Said");
    let f = w.t.employee_in(62, EmployeeStatus::Active).await;
    let mut other = fields("AE560331234567890125678");
    other[3] = ("holder_name", "  mona   SAID ");
    w.t.request_account(&w.a, &f, &other)
        .await
        .assert_status(303);
}

/// AC4: a card number with letters, or the routing code "12345", is refused.
#[renox::test]
async fn hrm19_ac4_a_card_with_letters_and_a_short_routing_code_are_refused() {
    let w = world().await;
    let card = |number: &'static str| {
        vec![
            ("channel", "salary_card"),
            ("account_number", number),
            ("routing_code", "123456789"),
            ("holder_name", "Ravi Kumar"),
        ]
    };

    w.t.request_account(&w.a, &w.e, &card("4111 1111 1111 111A"))
        .await
        .assert_status(422);
    let mut short = fields(NEW_IBAN);
    short[2] = ("routing_code", "12345");
    w.t.request_account(&w.a, &w.e, &short)
        .await
        .assert_status(422);
    assert!(w.t.accounts_of(&w.e).await.is_empty());

    w.t.request_account(&w.a, &w.e, &card("4111 1111 1111 1111"))
        .await
        .assert_status(303);
    assert_eq!(w.t.accounts_of(&w.e).await[0].last4, "1111");
}

/// AC5: E has no mobile number on record: `approve` makes the account
/// `cooling`, no SMS is sent, *notice undelivered* is set and HR has a task.
#[renox::test]
async fn hrm19_ac5_without_a_number_on_record_the_notice_is_undelivered_and_hr_has_a_task() {
    let w = world().await;
    let hr = w.t.user_with("Hira", "hr").await;
    let f = w.t.employee_in(63, EmployeeStatus::Active).await;
    w.t.parties.name(63, "Sara Noor");
    let mut form = fields(NEW_IBAN);
    form[3] = ("holder_name", "Sara Noor");
    w.t.request_account(&w.a, &f, &form)
        .await
        .assert_status(303);
    let account = w.t.accounts_of(&f).await.remove(0);

    w.t.on(nov(2), w.t.pay_transition(&w.b, &account, "approve", ""))
        .await
        .assert_status(303);

    let account = w.t.account(&account).await;
    assert_eq!(account.status, PayAccountStatus::Cooling);
    assert!(account.notice_undelivered_at.is_some());
    assert!(w.t.notices.of_type(CHANGE).is_empty(), "no SMS is sent");
    let tasks = w.t.tasks.with_key("hr.pay_account_notice:");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee, Some(Assignee::user(hr.id)));
}

/// AC5 with the real wl-ticketing: the HR task is a ticket of the `task`
/// category about the account, assigned to the HR holder, and the opener finds
/// it again by its source key while it is open.
#[renox::test]
async fn hrm19_ac5_the_hr_task_is_a_real_ticket_for_the_hr_holder() {
    let w = world_on(support::boot_with_ticketing().await).await;
    let hr = w.t.user_with("Hira", "hr").await;
    let f = w.t.employee_in(63, EmployeeStatus::Active).await;
    w.t.parties.name(63, "Sara Noor");
    let mut form = fields(NEW_IBAN);
    form[3] = ("holder_name", "Sara Noor");
    w.t.request_account(&w.a, &f, &form)
        .await
        .assert_status(303);
    let account = w.t.accounts_of(&f).await.remove(0);

    w.t.on(nov(2), w.t.pay_transition(&w.b, &account, "approve", ""))
        .await
        .assert_status(303);

    let account = w.t.account(&account).await;
    assert!(account.notice_undelivered_at.is_some());
    let tickets = w.t.tickets().await;
    assert_eq!(tickets.len(), 1);
    let (ticket, category) = &tickets[0];
    assert_eq!(category, "task");
    assert_eq!(ticket.assignee_user_id, Some(hr.id));
    assert_eq!(
        wl_kernel::Subject::new(&ticket.subject_type, ticket.subject_id),
        account.subject()
    );
    assert!(ticket.title.contains("Sara Noor"));
    let key = ticket.source_key.clone().unwrap();
    assert!(key.starts_with("hr.pay_account_notice:"));
    let found = wl_kernel::tickets::task_creator(w.t.state())
        .find_open(w.t.state(), &key)
        .await
        .unwrap();
    assert_eq!(found.map(|t| t.id.0), Some(ticket.id));
}

/// AC6: B rejects with a reason: `rejected`, A is notified; without a reason it
/// is refused. A new request is possible afterwards, not while one is open.
#[renox::test]
async fn hrm19_ac6_a_rejection_needs_a_reason_and_tells_the_requester() {
    let w = world().await;
    let account = requested(&w).await;
    w.t.request_account(&w.a, &w.e, &fields(OLD_IBAN))
        .await
        .assert_status(422);

    w.t.pay_transition(&w.b, &account, "reject", "")
        .await
        .assert_status(422);
    w.t.pay_transition(&w.b, &account, "reject", "IBAN belongs to someone else")
        .await
        .assert_status(303);

    let account = w.t.account(&account).await;
    assert_eq!(account.status, PayAccountStatus::Rejected);
    assert_eq!(
        account.reject_reason.as_deref(),
        Some("IBAN belongs to someone else")
    );
    let told = w.t.notices.of_type("hr.pay_account_rejected");
    assert_eq!(told.len(), 1);
    assert_eq!(users(&told[0]), vec![w.a.id]);
    assert_eq!(told[0].data["reason"], "IBAN belongs to someone else");
    w.t.request_account(&w.a, &w.e, &fields(OLD_IBAN))
        .await
        .assert_status(303);
}

/// The employee asks in the portal: the requester is the employee, and any
/// holder of `pay_accounts.approve` may approve. A user who is no employee is
/// refused.
#[renox::test]
async fn hrm19_the_employee_requests_in_the_portal_and_any_approver_may_approve() {
    let w = world().await;
    let (employee, account_user) = w.t.ravi().await;
    w.t.parties.name(50, "Ravi Kumar");
    w.t.parties.mobile(50, "+971500000050");
    let stranger = w.t.user("Sam").await;

    w.t.app.acting_as(&stranger);
    w.t.app
        .post("/hr/portal/pay-accounts", &fields(NEW_IBAN))
        .await
        .assert_status(403);
    w.t.app.acting_as(&account_user);
    w.t.app
        .post("/hr/portal/pay-accounts", &fields(NEW_IBAN))
        .await
        .assert_status(303);

    let account = w.t.accounts_of(&employee).await.remove(0);
    assert!(account.requested_by_employee);
    assert_eq!(account.requested_by, Some(account_user.id));
    w.t.pay_transition(&w.b, &account, "approve", "")
        .await
        .assert_status(303);
    assert_eq!(
        w.t.account(&account).await.status,
        PayAccountStatus::Cooling
    );
}

/// A request needs `employees.manage` for the employee: a manager who only
/// views employees is refused.
#[renox::test]
async fn hrm19_a_user_without_employees_manage_cannot_request() {
    let w = world().await;
    let manager = w.t.user_with("Mia", "manager").await;

    w.t.request_account(&manager, &w.e, &fields(NEW_IBAN))
        .await
        .assert_status(403);

    assert!(w.t.accounts_of(&w.e).await.is_empty());
}
