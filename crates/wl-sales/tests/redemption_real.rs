//! Redeeming points confirmed by a code sent through the real wl-notify
//! (BL-SAL-23, #142): the code is a real `send_otp` message that reaches the
//! SMS provider, the real `verify_otp` tells right, wrong, expired and
//! locked, and the redemption follows. Only the SMS provider is a stand-in.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use renox::prelude::*;
use wl_notify::channels::ProviderFuture;
use wl_notify::{ChannelProvider, Outgoing, SendError, WlNotify};
use wl_sales::{CounterSale, PointsKind, Redemption, RedemptionStatus, WlSales};

use support::floor::{Floor, account, movements};
use support::{Booted, boot_configured};

/// What the SMS provider was handed.
#[derive(Clone, Default)]
struct Phone(Arc<Mutex<Vec<Outgoing>>>);

impl Phone {
    /// The six digits of the last SMS.
    fn last_code(&self) -> String {
        let all = self.0.lock().unwrap();
        let body = &all.last().expect("an SMS was sent").body;
        body.chars().take_while(char::is_ascii_digit).collect()
    }

    fn count(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

struct PhoneProvider(Phone);

impl ChannelProvider for PhoneProvider {
    fn name(&self) -> &'static str {
        "test-sms"
    }

    fn channel(&self) -> wl_kernel::notify::NotifyChannel {
        wl_kernel::notify::NotifyChannel::Sms
    }

    fn available(&self, _state: &AppState) -> bool {
        true
    }

    fn send<'a>(
        &'a self,
        _state: &'a AppState,
        outgoing: &'a Outgoing,
    ) -> ProviderFuture<'a, std::result::Result<String, SendError>> {
        (self.0).0.lock().unwrap().push(outgoing.clone());
        Box::pin(async { Ok("sms-1".to_owned()) })
    }
}

async fn start() -> (Booted, Floor, Phone, wl_sales::LoyaltyAccount) {
    let phone = Phone::default();
    let notify = WlNotify::new().provider(PhoneProvider(phone.clone()));
    let t = boot_configured(WlSales::new(), notify).await;
    let floor = Floor::new(&t).await;
    floor.turn_on(&t, &[]).await;
    let m = floor.enrol(&t, 7, "0501234567").await;
    floor
        .ring_up(
            &t,
            &[("gift", "800.00", "1")],
            Some(&m.code),
            &[("cash", "840.00")],
        )
        .await;
    let m = account(&t, &m).await;
    assert_eq!(m.points_balance, 800);
    (t, floor, phone, m)
}

async fn ask(t: &Booted, sale: &CounterSale, points: &str) {
    t.app
        .post(
            &format!("/counter-sales/{}/redeem", sale.id),
            &[("points", points)],
        )
        .await
        .assert_status(303);
    t.app.run_all_jobs().await;
}

async fn enter(t: &Booted, sale: &CounterSale, code: &str, status: u16) {
    t.app
        .post(
            &format!("/counter-sales/{}/redeem/confirm", sale.id),
            &[("code", code)],
        )
        .await
        .assert_status(status);
}

fn wrong(code: &str) -> String {
    if code == "000000" { "111111" } else { "000000" }.to_owned()
}

async fn status_of(t: &Booted, sale: &CounterSale) -> RedemptionStatus {
    Redemption::of_sale(t.db(), &sale.subject()).await.unwrap()[0].status
}

#[renox::test]
async fn the_real_code_reaches_the_phone_and_confirms_the_redemption_on_the_sale() {
    let (t, floor, phone, m) = start().await;
    let sale = floor
        .open_cart(
            &t,
            &[("tee", "19.05", "1"), ("cap", "19.05", "1")],
            Some(&m.code),
        )
        .await;
    ask(&t, &sale, "800").await;
    // The real message reached M's mobile with a six-digit code.
    assert_eq!(phone.count(), 1);
    let sms = phone.0.lock().unwrap()[0].clone();
    assert_eq!(sms.address, "+971501234567");
    assert!(sms.body.contains("valid for 5 minutes"), "{}", sms.body);
    let code = phone.last_code();
    assert_eq!(code.len(), 6);

    // A wrong code counts down; the right one holds the points.
    enter(&t, &sale, &wrong(&code), 422).await;
    assert_eq!(status_of(&t, &sale).await, RedemptionStatus::Pending);
    enter(&t, &sale, &code, 303).await;
    assert_eq!(status_of(&t, &sale).await, RedemptionStatus::Held);
    t.app
        .post(
            &format!("/counter-sales/{}/complete", sale.id),
            &[("cash", "32.00")],
        )
        .await
        .assert_status(303);
    assert_eq!(status_of(&t, &sale).await, RedemptionStatus::Applied);
    let spent = movements(&t, &m, PointsKind::Redeem).await;
    assert_eq!(spent.iter().map(|x| x.points).sum::<i64>(), -800);
}

#[renox::test]
async fn three_wrong_real_codes_lock_the_redemption_and_an_old_code_is_expired() {
    let (t, floor, phone, m) = start().await;
    let locked = floor
        .open_cart(&t, &[("tee", "20.00", "1")], Some(&m.code))
        .await;
    ask(&t, &locked, "600").await;
    let code = phone.last_code();
    for _ in 0..3 {
        enter(&t, &locked, &wrong(&code), 422).await;
    }
    assert_eq!(status_of(&t, &locked).await, RedemptionStatus::Refused);
    // The right code is no good now, and no new code is sent for the sale.
    enter(&t, &locked, &code, 422).await;
    t.app
        .post(
            &format!("/counter-sales/{}/redeem", locked.id),
            &[("points", "600")],
        )
        .await
        .assert_status(422);
    t.app.run_all_jobs().await;
    assert_eq!(phone.count(), 1);

    // Six minutes after sending the code is expired; a new one works.
    let late = floor
        .open_cart(&t, &[("tee", "20.00", "1")], Some(&m.code))
        .await;
    ask(&t, &late, "600").await;
    let first = phone.last_code();
    t.app.travel(Duration::from_secs(6 * 60));
    enter(&t, &late, &first, 422).await;
    ask(&t, &late, "600").await;
    assert_eq!(phone.count(), 3);
    enter(&t, &late, &phone.last_code(), 303).await;
    assert_eq!(status_of(&t, &late).await, RedemptionStatus::Held);
}
