//! BL-LOG-17 (#315): a returned remittance transfer and the control of a
//! change of the client's bank account, 03 §3.3 `LOG.ACL.13`, §4.1 row *Bank
//! returns the remittance transfer*, §7.1 rows `return_payment` and `repay`.
//! The payments are the real wl-payments; the confirmation is the signed link
//! the client's known contact receives.

mod support;

use logistics::account::{self, AccountChange, AccountChangeStatus};
use logistics::notices::{ACCOUNT_CONFIRM, STAFF_NOTICE};
use logistics::reweigh::ClientTerms;
use logistics::{PaymentReturn, Remittance, RemittanceStatus, statement};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::payments::{self, Direction, PaymentId, PaymentStatus};
use wl_payments::payment::Payment;

use support::parties::KNOWN_EMAIL;
use support::{SOUQ, World, boot_with_payments};

const OLD: &str = "AE070331234567890123456";
const NEW: &str = "AE460260001234567890123";

/// R-12: AED 4,000 of COD for Souq Express, drafted, approved by Bashar and
/// paid by Amal with the reference TRF-1.
async fn paid() -> (World, Remittance) {
    let w = boot_with_payments().await;
    w.cod_terms(SOUQ, 0, Some(OLD)).await;
    w.delivered(SOUQ, "ORD-1", "4000").await;
    w.rule().await;
    let draft = w.the_remittance_of(SOUQ).await;
    w.post(
        &w.bashar,
        &format!("/remittances/{}/approve", draft.id),
        &[],
    )
    .await
    .assert_status(303);
    w.post(
        &w.amal,
        &format!("/remittances/{}/pay", draft.id),
        &[("payment_reference", "TRF-1")],
    )
    .await
    .assert_status(303);
    let paid = w.remittance(&draft).await;
    assert_eq!(paid.status, RemittanceStatus::Paid);
    assert_eq!(paid.net, 400_000);
    (w, paid)
}

async fn returned(w: &World, remittance: &Remittance) {
    w.post(
        &w.amal,
        &format!("/remittances/{}/return-payment", remittance.id),
        &[("advice", "BANK-ADV-77"), ("reason", "account closed")],
    )
    .await
    .assert_status(303);
}

async fn the_account(w: &World) -> Option<String> {
    logistics::reweigh::terms(w.db(), SOUQ)
        .await
        .unwrap()
        .bank_account
}

/// The path and query of the confirmation link the client's contact got.
fn confirmation_path(w: &World) -> String {
    let sent = w.notices.of_type(ACCOUNT_CONFIRM);
    let link = sent.last().expect("the link was sent").data["link"]
        .as_str()
        .expect("a link")
        .to_owned();
    let at = link.find("/remittances").expect("the link names the route");
    link[at..].to_owned()
}

/// The client opens the link of its e-mail and presses the button: opening
/// only shows the page, the button posts to the same signed address.
async fn confirm_by_link(w: &World) {
    let before = the_change(w).await.status;
    let path = confirmation_path(w);
    w.get(&w.noor, &path)
        .await
        .assert_ok()
        .assert_see("Confirm the new bank account");
    assert_eq!(
        the_change(w).await.status,
        before,
        "opening the link changes nothing"
    );
    w.post(&w.noor, &path, &[]).await.assert_ok();
}

async fn the_change(w: &World) -> AccountChange {
    AccountChange::query()
        .order_by_desc(AccountChange::ID)
        .first(w.db())
        .await
        .unwrap()
        .expect("a change was asked")
}

async fn ask(w: &World, by: &User, account: &str) -> renox::testing::TestResponse {
    w.post(
        by,
        "/remittances/accounts",
        &[("client_party_id", "501"), ("account", account)],
    )
    .await
}

/// AC1: given remittance R-12 `paid` AED 4,000 and a bank return advice
/// attached, `return_payment` makes it `payment_returned` and the client is
/// asked to confirm its bank account.
#[renox::test]
async fn a_returned_transfer_makes_the_remittance_payment_returned_and_asks_the_client() {
    let (w, r12) = paid().await;
    let voucher = r12.payment_id.expect("a voucher");

    // The advice is needed, and so is the permission.
    let path = format!("/remittances/{}/return-payment", r12.id);
    w.post(&w.amal, &path, &[("advice", " "), ("reason", "closed")])
        .await
        .assert_status(422)
        .assert_see("return advice");
    w.post(&w.dispatcher, &path, &[("advice", "BANK-ADV-77")])
        .await
        .assert_forbidden();
    assert_eq!(w.remittance_status(&r12).await, RemittanceStatus::Paid);

    returned(&w, &r12).await;
    let back = w.remittance(&r12).await;
    assert_eq!(back.status, RemittanceStatus::PaymentReturned);
    // The advice is kept with the return.
    let returns: Vec<PaymentReturn> = back.returns(w.db()).await.unwrap();
    assert_eq!(returns.len(), 1);
    assert_eq!(returns[0].advice, "BANK-ADV-77");
    assert_eq!(returns[0].reason, "account closed");
    assert_eq!(returns[0].bank_account.as_deref(), Some(OLD));
    assert_eq!(returns[0].payment_reference.as_deref(), Some("TRF-1"));
    // The voucher is reversed in the payments module as a bank return.
    let payment = payments::payment_recorder(w.state())
        .find(w.db(), PaymentId(voucher))
        .await
        .unwrap()
        .expect("the voucher exists");
    assert_eq!(payment.status, PaymentStatus::Reversed);
    // The client is told and asked to confirm its bank account.
    let told = w.notices.to_clients();
    let notice = told
        .iter()
        .find(|request| {
            request.data["title"]
                .as_str()
                .is_some_and(|t| t.contains("returned"))
        })
        .expect("the client is told");
    assert!(
        notice
            .recipients
            .contains(&NotifyRecipient::Party { id: SOUQ })
    );
    assert!(
        notice.data["body"]
            .as_str()
            .is_some_and(|body| body.contains("confirm your bank account"))
    );
    // It is paid once, so it cannot be returned again before `repay`.
    w.post(&w.amal, &path, &[("advice", "BANK-ADV-78")])
        .await
        .assert_status(422);
}

/// AC2: given accountant A enters a new IBAN, it is pending; after
/// accountant B approves and the client confirms from its known e-mail, it
/// is active. A cannot approve.
#[renox::test]
async fn a_new_account_is_active_after_a_second_user_approves_and_the_client_confirms() {
    let (w, _r12) = paid().await;
    ask(&w, &w.amal, NEW).await.assert_status(303);
    let change = the_change(&w).await;
    assert_eq!(change.status, AccountChangeStatus::Pending);
    assert_eq!(change.account, NEW);
    assert_eq!(change.requested_by, w.amal.id);
    assert_eq!(change.contact, KNOWN_EMAIL);
    // The link went to the e-mail on record.
    let sent = w.notices.of_type(ACCOUNT_CONFIRM);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].recipients,
        vec![NotifyRecipient::Address {
            channel: wl_kernel::notify::NotifyChannel::Email,
            address: KNOWN_EMAIL.to_owned(),
        }]
    );
    assert_eq!(the_account(&w).await.as_deref(), Some(OLD));

    let approve = format!("/remittances/accounts/{}/approve", change.id);
    w.post(&w.amal, &approve, &[])
        .await
        .assert_status(422)
        .assert_see("You asked for this change");
    w.post(&w.dispatcher, &approve, &[])
        .await
        .assert_forbidden();
    w.post(&w.bashar, &approve, &[]).await.assert_status(303);
    assert_eq!(
        the_change(&w).await.status,
        AccountChangeStatus::Approved,
        "approval alone does not activate"
    );
    assert_eq!(the_account(&w).await.as_deref(), Some(OLD));

    confirm_by_link(&w).await;
    let active = the_change(&w).await;
    assert_eq!(active.status, AccountChangeStatus::Active);
    assert_eq!(active.approved_by, Some(w.bashar.id));
    assert!(active.confirmed_at.is_some());
    assert_eq!(the_account(&w).await.as_deref(), Some(NEW));
}

/// LOG.ACL.13: a mail scanner or a prefetch opens (GET) the link of the
/// e-mail: nothing is confirmed. The altered link is refused, also by POST.
#[renox::test]
async fn opening_the_confirmation_link_does_not_confirm_the_account() {
    let (w, _r12) = paid().await;
    ask(&w, &w.amal, NEW).await.assert_status(303);
    let path = confirmation_path(&w);
    for _ in 0..2 {
        w.get(&w.noor, &path).await.assert_ok();
    }
    let change = the_change(&w).await;
    assert_eq!(change.status, AccountChangeStatus::Pending);
    assert_eq!(change.confirmed_at, None);
    assert_eq!(the_account(&w).await.as_deref(), Some(OLD));

    let altered = path.replace("signature=", "signature=0");
    w.get(&w.noor, &altered).await.assert_status(403);
    w.post(&w.noor, &altered, &[]).await.assert_status(403);
    assert_eq!(the_change(&w).await.status, AccountChangeStatus::Pending);

    w.post(&w.noor, &path, &[]).await.assert_ok();
    assert_eq!(the_change(&w).await.status, AccountChangeStatus::Confirmed);
}

/// The two conditions are met in either order.
#[renox::test]
async fn the_client_may_confirm_before_the_approval() {
    let (w, _r12) = paid().await;
    ask(&w, &w.amal, NEW).await.assert_status(303);
    let change = the_change(&w).await;
    confirm_by_link(&w).await;
    assert_eq!(the_change(&w).await.status, AccountChangeStatus::Confirmed);
    assert_eq!(the_account(&w).await.as_deref(), Some(OLD));
    w.post(
        &w.bashar,
        &format!("/remittances/accounts/{}/approve", change.id),
        &[],
    )
    .await
    .assert_status(303);
    assert_eq!(the_change(&w).await.status, AccountChangeStatus::Active);
    assert_eq!(the_account(&w).await.as_deref(), Some(NEW));
}

/// A client user in the portal may ask for the change; the approval and the
/// confirmation are still needed, and the link still goes to the contact on
/// record.
#[renox::test]
async fn a_client_user_asks_for_the_change_in_the_portal() {
    let (w, _r12) = paid().await;
    // Another client's user cannot ask for Souq Express.
    ask(&w, &w.farah, NEW).await.assert_forbidden();
    ask(&w, &w.noor, NEW).await.assert_status(303);
    let change = the_change(&w).await;
    assert_eq!(change.via, "portal");
    assert_eq!(change.requested_by, w.noor.id);
    assert_eq!(change.contact, KNOWN_EMAIL);
    // The client user cannot approve (it holds no `remittances.approve`).
    w.post(
        &w.noor,
        &format!("/remittances/accounts/{}/approve", change.id),
        &[],
    )
    .await
    .assert_forbidden();
    assert_eq!(the_account(&w).await.as_deref(), Some(OLD));
}

/// AC3: `repay` with a new reference before the account is active is
/// refused; after activation it makes R-12 `paid` and the statement shows the
/// new reference.
#[renox::test]
async fn repay_waits_for_the_new_account_and_then_shows_the_new_reference() {
    let (w, r12) = paid().await;
    returned(&w, &r12).await;
    ask(&w, &w.amal, NEW).await.assert_status(303);
    let change = the_change(&w).await;
    let repay = format!("/remittances/{}/repay", r12.id);

    // The old account is the one that was returned, and the new one is not
    // active.
    w.post(&w.amal, &repay, &[("payment_reference", "TRF-2")])
        .await
        .assert_status(422)
        .assert_see("returned from the bank account on file");
    assert_eq!(
        w.remittance_status(&r12).await,
        RemittanceStatus::PaymentReturned
    );
    // Approved alone is not active either.
    w.post(
        &w.bashar,
        &format!("/remittances/accounts/{}/approve", change.id),
        &[],
    )
    .await
    .assert_status(303);
    w.post(&w.amal, &repay, &[("payment_reference", "TRF-2")])
        .await
        .assert_status(422);

    confirm_by_link(&w).await;
    // The reference must be a new one, and the permission is needed.
    w.post(&w.amal, &repay, &[("payment_reference", "TRF-1")])
        .await
        .assert_status(422)
        .assert_see("new payment reference");
    w.post(&w.dispatcher, &repay, &[("payment_reference", "TRF-2")])
        .await
        .assert_forbidden();
    w.post(&w.amal, &repay, &[("payment_reference", "TRF-2")])
        .await
        .assert_status(303);

    let again = w.remittance(&r12).await;
    assert_eq!(again.status, RemittanceStatus::Paid);
    assert_eq!(again.payment_reference.as_deref(), Some("TRF-2"));
    assert_eq!(again.bank_account.as_deref(), Some(NEW));
    // The new payout is a voucher of the net to the new account.
    let vouchers: Vec<Payment> = Payment::query()
        .order_by(Payment::ID)
        .get(w.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|payment| payment.direction == Direction::Out)
        .collect();
    assert_eq!(vouchers.len(), 2);
    assert_eq!(vouchers[1].amount, 400_000);
    assert_eq!(vouchers[1].bank_account.as_deref(), Some(NEW));
    assert_eq!(vouchers[1].status, PaymentStatus::Received);
    // The statement shows the new reference; the earlier version stays.
    let history = statement::history(w.db(), r12.id).await.unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].payment_reference, "TRF-1");
    assert_eq!(history[1].payment_reference, "TRF-2");
    w.get(&w.amal, &format!("/remittances/{}/statement", r12.id))
        .await
        .assert_ok()
        .assert_see("TRF-2");
}

/// Repaying to the same account is allowed when the client's confirmation
/// that it is correct is recorded; and a return can come again, each keeping
/// its advice.
#[renox::test]
async fn the_same_account_is_repaid_on_the_clients_confirmation_and_may_come_back_again() {
    let (w, r12) = paid().await;
    returned(&w, &r12).await;
    let repay = format!("/remittances/{}/repay", r12.id);
    w.post(&w.amal, &repay, &[("payment_reference", "TRF-2")])
        .await
        .assert_status(422);

    let confirm = format!("/remittances/{}/confirm-account", r12.id);
    w.post(&w.amal, &confirm, &[("note", " ")])
        .await
        .assert_status(422);
    w.post(&w.dispatcher, &confirm, &[("note", "called")])
        .await
        .assert_forbidden();
    w.post(
        &w.amal,
        &confirm,
        &[("note", "Client e-mail 2026-11-03 10:15")],
    )
    .await
    .assert_status(303);
    w.post(&w.amal, &repay, &[("payment_reference", "TRF-2")])
        .await
        .assert_status(303);
    let again = w.remittance(&r12).await;
    assert_eq!(again.status, RemittanceStatus::Paid);
    assert_eq!(again.bank_account.as_deref(), Some(OLD));
    assert_eq!(
        again.account_confirmed_at, None,
        "the confirmation is used up"
    );

    // It comes back a second time: the second advice is kept beside the first.
    w.post(
        &w.amal,
        &format!("/remittances/{}/return-payment", r12.id),
        &[("advice", "BANK-ADV-78"), ("reason", "bank issue")],
    )
    .await
    .assert_status(303);
    let advices: Vec<String> = w
        .remittance(&r12)
        .await
        .returns(w.db())
        .await
        .unwrap()
        .into_iter()
        .map(|returned| returned.advice)
        .collect();
    assert_eq!(
        advices,
        vec!["BANK-ADV-77".to_owned(), "BANK-ADV-78".to_owned()]
    );
}

/// AC4: given only B's approval and no client confirmation, the next
/// remittance's `pay` uses the old account.
#[renox::test]
async fn without_the_clients_confirmation_the_next_payment_goes_to_the_old_account() {
    let (w, _r12) = paid().await;
    ask(&w, &w.amal, NEW).await.assert_status(303);
    let change = the_change(&w).await;
    w.post(
        &w.bashar,
        &format!("/remittances/accounts/{}/approve", change.id),
        &[],
    )
    .await
    .assert_status(303);

    w.delivered(SOUQ, "ORD-2", "100").await;
    w.rule().await;
    let next = w
        .remittances_of(SOUQ)
        .await
        .into_iter()
        .find(|r| r.status == RemittanceStatus::Draft)
        .expect("the next draft");
    w.post(&w.bashar, &format!("/remittances/{}/approve", next.id), &[])
        .await
        .assert_status(303);
    w.post(
        &w.amal,
        &format!("/remittances/{}/pay", next.id),
        &[("payment_reference", "TRF-3")],
    )
    .await
    .assert_status(303);
    assert_eq!(w.remittance(&next).await.bank_account.as_deref(), Some(OLD));
}

/// AC5: given a change requested and approved but not confirmed in 14 days,
/// it lapses: the confirmation link is refused and A is told; a second
/// request made while the first was pending was refused.
#[renox::test]
async fn a_change_not_approved_and_confirmed_in_14_days_lapses() {
    let (w, _r12) = paid().await;
    ask(&w, &w.amal, NEW).await.assert_status(303);
    let first = the_change(&w).await;
    // Three days later it is still pending: a second request is refused.
    let requested = renox::db::now() - TimeDelta::days(3);
    let expires = requested + TimeDelta::days(account::LAPSE_DAYS);
    AccountChange::query()
        .where_(AccountChange::ID.eq(first.id))
        .update(
            w.db(),
            &[("requested_at", &requested), ("expires_at", &expires)],
        )
        .await
        .unwrap();
    ask(&w, &w.amal, "AE580260001111111111111")
        .await
        .assert_status(422)
        .assert_see("a bank account change is already pending");
    ask(&w, &w.bashar, "AE580260001111111111111")
        .await
        .assert_status(422);
    w.post(
        &w.bashar,
        &format!("/remittances/accounts/{}/approve", first.id),
        &[],
    )
    .await
    .assert_status(303);

    // Not confirmed by day 14: it lapses on the daily run.
    let lapsed = account::lapse_due(w.state(), requested + TimeDelta::days(14))
        .await
        .unwrap();
    assert_eq!(lapsed.len(), 1);
    let gone = the_change(&w).await;
    assert_eq!(gone.status, AccountChangeStatus::Lapsed);
    assert_eq!(the_account(&w).await.as_deref(), Some(OLD));
    w.get(&w.noor, &confirmation_path(&w))
        .await
        .assert_status(422)
        .assert_see("This link no longer works.");
    w.post(&w.noor, &confirmation_path(&w), &[])
        .await
        .assert_status(422)
        .assert_see("This link no longer works.");
    assert_eq!(the_account(&w).await.as_deref(), Some(OLD));
    // The requester is told.
    assert!(w.notices.users_of_type(STAFF_NOTICE).contains(&w.amal.id));
    // A new request is possible again.
    ask(&w, &w.amal, "AE580260001111111111111")
        .await
        .assert_status(303);
}

/// A rejected change is discarded and the requester is told; the requester
/// cannot reject its own; one who changed their mind withdraws it.
#[renox::test]
async fn a_rejected_or_withdrawn_change_is_discarded() {
    let (w, _r12) = paid().await;
    ask(&w, &w.amal, NEW).await.assert_status(303);
    let change = the_change(&w).await;
    let reject = format!("/remittances/accounts/{}/reject", change.id);
    w.post(&w.amal, &reject, &[("note", "x")])
        .await
        .assert_status(422);
    w.post(&w.bashar, &reject, &[("note", "Not the client's account")])
        .await
        .assert_status(303);
    assert_eq!(the_change(&w).await.status, AccountChangeStatus::Rejected);
    assert!(w.notices.users_of_type(STAFF_NOTICE).contains(&w.amal.id));
    // The link of a discarded change does not work.
    w.get(&w.noor, &confirmation_path(&w))
        .await
        .assert_status(422);
    assert_eq!(the_account(&w).await.as_deref(), Some(OLD));

    ask(&w, &w.amal, NEW).await.assert_status(303);
    let second = the_change(&w).await;
    let withdraw = format!("/remittances/accounts/{}/withdraw", second.id);
    w.post(&w.bashar, &withdraw, &[]).await.assert_status(422);
    w.post(&w.amal, &withdraw, &[]).await.assert_status(303);
    assert_eq!(the_change(&w).await.status, AccountChangeStatus::Withdrawn);
}

/// The IBAN is checked, and the account on file is not asked again.
#[renox::test]
async fn an_account_that_is_no_iban_or_is_the_one_on_file_is_refused() {
    let (w, _r12) = paid().await;
    ask(&w, &w.amal, "12345").await.assert_status(422);
    ask(&w, &w.amal, "ae07 0331 2345 6789 0123 456")
        .await
        .assert_status(422)
        .assert_see("on file already");
    assert_eq!(
        ClientTerms::query().count(w.db()).await.unwrap(),
        1,
        "nothing was changed"
    );
}

/// AC5: the daily 03:00 job is on the scheduler and lapses a change that was
/// not approved and confirmed within 14 days.
#[renox::test]
async fn the_scheduled_job_lapses_a_change_after_14_days() {
    let (w, _r12) = paid().await;
    ask(&w, &w.amal, NEW).await.assert_status(303);
    w.app.kernel().run_scheduled(account::TASK).await.unwrap();
    assert_eq!(
        the_change(&w).await.status,
        AccountChangeStatus::Pending,
        "not yet due"
    );
    w.app
        .travel(std::time::Duration::from_secs(14 * 24 * 3600 + 3_600));
    w.app
        .at_travelled_time(w.app.kernel().run_scheduled(account::TASK))
        .await
        .expect("the task is registered and runs");
    assert_eq!(the_change(&w).await.status, AccountChangeStatus::Lapsed);
    assert_eq!(the_account(&w).await.as_deref(), Some(OLD));
}
