//! One-time codes through the kernel's `Notifier` (wl-notify §6 `send_otp`
//! and `verify_otp`, §7; the base of BL-SAL-23, #142): the code reaches the
//! provider as a message, is valid for 5 minutes and 3 wrong tries, is told
//! `valid`, `wrong`, `expired` or `locked`, never confirms another record, and
//! a code that cannot be sent says so at once.

mod support;

use std::time::Duration;

use wl_kernel::Subject;
use wl_kernel::notify::{self, NotifyChannel, NotifyRecipient, OtpRequest, OtpSent, OtpVerdict};
use wl_notify::{OtpCode, OtpStatus};

use support::{Booted, boot, boot_providers};

const MOBILE: &str = "+971501234567";

fn sale(id: i64) -> Subject {
    Subject::new("sales.counter_sale", id)
}

fn to_mobile() -> NotifyRecipient {
    NotifyRecipient::Address {
        channel: NotifyChannel::Sms,
        address: MOBILE.to_owned(),
    }
}

fn request(subject: Subject) -> OtpRequest {
    OtpRequest::new("loyalty_redemption", subject, to_mobile())
}

/// Sends a code and returns the digits the provider was handed.
async fn send(t: &Booted, subject: Subject) -> String {
    let state = t.app.state();
    let before = t.sms.sent().len();
    let sent = t
        .app
        .at_travelled_time(notify::notifier(state).send_otp(state, request(subject)))
        .await
        .expect("the notifier sends the code");
    assert!(matches!(sent, OtpSent::Sent { .. }), "{sent:?}");
    t.app.run_all_jobs().await;
    let all = t.sms.sent();
    assert_eq!(all.len(), before + 1, "one SMS carries the code");
    let sms = all.last().unwrap();
    assert_eq!(sms.address, MOBILE);
    let code: String = sms.body.chars().take_while(char::is_ascii_digit).collect();
    assert_eq!(code.len(), 6, "{}", sms.body);
    assert!(sms.body.contains("valid for 5 minutes"), "{}", sms.body);
    code
}

async fn verify(t: &Booted, subject: &Subject, typed: &str) -> OtpVerdict {
    let state = t.app.state();
    t.app
        .at_travelled_time(notify::notifier(state).verify_otp(
            state,
            "loyalty_redemption",
            subject,
            typed,
        ))
        .await
        .expect("the notifier checks the code")
}

/// A wrong code never equals the one sent.
fn wrong_of(code: &str) -> String {
    if code == "000000" { "111111" } else { "000000" }.to_owned()
}

#[renox::test]
async fn the_right_code_is_valid_once_and_a_wrong_one_counts_down() {
    let t = boot().await;
    let code = send(&t, sale(7)).await;
    assert_eq!(
        verify(&t, &sale(7), &wrong_of(&code)).await,
        OtpVerdict::Wrong { tries_left: 2 }
    );
    assert_eq!(verify(&t, &sale(7), &code).await, OtpVerdict::Valid);
    // Used up: it cannot confirm twice.
    assert_eq!(verify(&t, &sale(7), &code).await, OtpVerdict::Expired);
    let row = OtpCode::latest(t.db(), "loyalty_redemption", &sale(7))
        .await
        .unwrap()
        .unwrap();
    assert_eq!((row.status, row.tries), (OtpStatus::Used, 1));
}

#[renox::test]
async fn three_wrong_codes_void_the_code_even_for_the_right_one() {
    let t = boot().await;
    let code = send(&t, sale(7)).await;
    let wrong = wrong_of(&code);
    assert_eq!(
        verify(&t, &sale(7), &wrong).await,
        OtpVerdict::Wrong { tries_left: 2 }
    );
    assert_eq!(
        verify(&t, &sale(7), &wrong).await,
        OtpVerdict::Wrong { tries_left: 1 }
    );
    assert_eq!(verify(&t, &sale(7), &wrong).await, OtpVerdict::Locked);
    assert_eq!(verify(&t, &sale(7), &code).await, OtpVerdict::Locked);
    let row = OtpCode::latest(t.db(), "loyalty_redemption", &sale(7))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, OtpStatus::Locked);
}

#[renox::test]
async fn a_code_expires_after_five_minutes_and_a_new_one_can_be_sent() {
    let t = boot().await;
    let code = send(&t, sale(7)).await;
    t.app.travel(Duration::from_secs(4 * 60));
    assert_eq!(
        verify(&t, &sale(7), &wrong_of(&code)).await,
        OtpVerdict::Wrong { tries_left: 2 }
    );
    t.app.travel(Duration::from_secs(2 * 60));
    assert_eq!(verify(&t, &sale(7), &code).await, OtpVerdict::Expired);
    // An expired code may be sent again; the new one works.
    let fresh = send(&t, sale(7)).await;
    assert_eq!(verify(&t, &sale(7), &fresh).await, OtpVerdict::Valid);
}

#[renox::test]
async fn a_code_confirms_only_the_record_it_was_sent_for() {
    let t = boot().await;
    let code = send(&t, sale(7)).await;
    assert_eq!(verify(&t, &sale(8), &code).await, OtpVerdict::Expired);
    assert_eq!(verify(&t, &sale(7), &code).await, OtpVerdict::Valid);
}

#[renox::test]
async fn a_new_code_replaces_the_live_one() {
    let t = boot().await;
    let first = send(&t, sale(7)).await;
    let second = send(&t, sale(7)).await;
    if first != second {
        assert_eq!(
            verify(&t, &sale(7), &first).await,
            OtpVerdict::Wrong { tries_left: 2 }
        );
    }
    assert_eq!(verify(&t, &sale(7), &second).await, OtpVerdict::Valid);
}

#[renox::test]
async fn a_code_that_cannot_be_sent_says_so_at_once_and_keeps_nothing() {
    let t = boot().await;
    let state = t.app.state();
    // No channel of the type can reach a WhatsApp number.
    let to = NotifyRecipient::Address {
        channel: NotifyChannel::Whatsapp,
        address: MOBILE.to_owned(),
    };
    let sent = notify::notifier(state)
        .send_otp(state, OtpRequest::new("loyalty_redemption", sale(7), to))
        .await
        .unwrap();
    match sent {
        OtpSent::NotSent { text } => assert!(text.starts_with("Code could not be sent"), "{text}"),
        other => panic!("expected NotSent, got {other:?}"),
    }
    t.app.run_all_jobs().await;
    assert!(t.sms.sent().is_empty() && t.email.sent().is_empty());
    assert_eq!(verify(&t, &sale(7), "123456").await, OtpVerdict::Expired);
}

#[renox::test]
async fn the_validity_and_the_wrong_tries_are_settings_of_the_module() {
    let t = boot_providers(
        None,
        |m| m.otp_wrong_tries(2).otp_validity_minutes(10),
        |app| app,
    )
    .await;
    let state = t.app.state();
    let sent = notify::notifier(state)
        .send_otp(state, request(sale(7)))
        .await
        .unwrap();
    let OtpSent::Sent { expires_at, .. } = sent else {
        panic!("expected Sent");
    };
    assert!(expires_at > renox::db::now() + renox::chrono::TimeDelta::minutes(9));
    assert_eq!(
        verify(&t, &sale(7), "000000").await,
        OtpVerdict::Wrong { tries_left: 1 }
    );
    // The code is random: "000000" may even be right once in a million.
    let second = verify(&t, &sale(7), "111111").await;
    assert!(matches!(second, OtpVerdict::Locked | OtpVerdict::Valid));
}
