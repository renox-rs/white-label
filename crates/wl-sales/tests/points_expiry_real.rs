//! The expiry reminder through the real wl-notify (BL-SAL-24, #143,
//! `SAL.RUL.08`): the reminder reaches the member's e-mail, one that gives up
//! for real is shown on the account while the lot still expires, and a member
//! who opted out gets none. Only the e-mail provider, the party directory and
//! the clock are stand-ins.

mod support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use renox::chrono::{Duration, NaiveDate};
use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::documents::today;
use wl_kernel::notify::NotifyChannel;
use wl_notify::channels::ProviderFuture;
use wl_notify::suppressions::{self, SuppressionReason};
use wl_notify::{ChannelProvider, Clock, Outgoing, SendError, WlNotify};
use wl_sales::expiry::TASK;
use wl_sales::{LoyaltyAccount, PointsKind, PointsMovement, WlSales};

use support::floor::{Floor, account, movements};
use support::retention::Parties;
use support::{Booted, boot_pair};

/// What the e-mail provider was handed, and whether it refuses to send.
#[derive(Clone, Default)]
struct Mailbox {
    sent: Arc<Mutex<Vec<Outgoing>>>,
    broken: Arc<AtomicBool>,
}

impl Mailbox {
    fn to(&self, address: &str) -> usize {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|o| o.address == address)
            .count()
    }

    fn count(&self) -> usize {
        self.sent.lock().unwrap().len()
    }
}

struct MailProvider(Mailbox);

impl ChannelProvider for MailProvider {
    fn name(&self) -> &'static str {
        "test-mail"
    }

    fn channel(&self) -> NotifyChannel {
        NotifyChannel::Email
    }

    fn available(&self, _state: &AppState) -> bool {
        true
    }

    fn send<'a>(
        &'a self,
        _state: &'a AppState,
        outgoing: &'a Outgoing,
    ) -> ProviderFuture<'a, std::result::Result<String, SendError>> {
        let result = if (self.0).broken.load(Ordering::SeqCst) {
            Err(SendError::permanent("550", "mailbox unknown"))
        } else {
            (self.0).sent.lock().unwrap().push(outgoing.clone());
            Ok("mail-1".to_owned())
        };
        Box::pin(async move { result })
    }
}

/// The party directory (with e-mail addresses) and a clock fixed at noon in
/// Dubai, inside every send window.
#[derive(Clone)]
struct Fakes {
    parties: Parties,
    noon: DateTime,
}

impl Module for Fakes {
    fn name(&self) -> &'static str {
        "real_fakes"
    }

    fn register(&self, app: &mut Registry) {
        self.parties.register(app);
        app.provide(Clock::fixed(self.noon));
    }
}

const MARIA: &str = "maria@example.com";
const OMAR: &str = "omar@example.com";

async fn world() -> (Booted, Floor, Mailbox, Parties) {
    let mailbox = Mailbox::default();
    let notify = WlNotify::new().provider(MailProvider(mailbox.clone()));
    let noon = (today().and_hms_opt(12, 0, 0).unwrap() - Duration::hours(4)).and_utc();
    let parties = Parties::default();
    parties.directory.set_email(7, MARIA);
    parties.directory.set_email(8, OMAR);
    let fakes = Fakes {
        parties: parties.clone(),
        noon,
    };
    let t = boot_pair(WlSales::new(), notify, fakes).await;
    let floor = Floor::new(&t).await;
    floor.turn_on(&t, &[]).await;
    (t, floor, mailbox, parties)
}

/// A member with a lot of `points` whose last day is `last`.
async fn member(
    t: &Booted,
    floor: &Floor,
    party: i64,
    mobile: &str,
    points: i64,
    last: NaiveDate,
) -> LoyaltyAccount {
    let m = floor.enrol(t, party, mobile).await;
    floor.earn(t, &m, points).await;
    let lot = movements(t, &m, PointsKind::Earn).await.remove(0);
    let last = Some(last);
    PointsMovement::query()
        .where_(PointsMovement::ID.eq(lot.id))
        .update(t.db(), &[("lot_expires_on", &last)])
        .await
        .unwrap();
    account(t, &m).await
}

/// Runs the real daily task for `on` and lets the queue send what it asked.
async fn run(t: &Booted, on: NaiveDate) {
    t.app
        .kernel()
        .call(TASK, vec!["--date".to_owned(), on.to_string()])
        .await
        .expect("the expiry task runs");
    t.app.run_all_jobs().await;
}

#[renox::test]
async fn the_reminder_reaches_the_members_mailbox_once_and_the_lot_expires_after_its_day() {
    let (t, floor, mailbox, _parties) = world().await;
    let last = today() + Duration::days(20);
    let m = member(&t, &floor, 7, "0501234567", 300, last).await;

    run(&t, today()).await;
    assert_eq!(mailbox.to(MARIA), 1);
    let body = mailbox.sent.lock().unwrap()[0].body.clone();
    assert!(body.contains("300"), "{body}");
    assert!(account(&t, &m).await.expiry_reminder_failed_at.is_none());
    // One reminder per lot: the next run sends no second one.
    run(&t, today() + Duration::days(1)).await;
    assert_eq!(mailbox.to(MARIA), 1);

    run(&t, last + Duration::days(1)).await;
    let gone = movements(&t, &m, PointsKind::Expire).await;
    assert_eq!(gone.iter().map(|x| x.points).sum::<i64>(), -300);
}

#[renox::test]
async fn a_reminder_that_really_gave_up_shows_on_the_account_and_the_lot_still_expires() {
    let (t, floor, mailbox, _parties) = world().await;
    let last = today() + Duration::days(20);
    let m = member(&t, &floor, 7, "0501234567", 300, last).await;
    mailbox.broken.store(true, Ordering::SeqCst);

    run(&t, today()).await;
    // Nothing was delivered; wl-notify gave up and said so.
    assert_eq!(mailbox.count(), 0);
    assert!(account(&t, &m).await.expiry_reminder_failed_at.is_some());
    t.app.acting_as(&floor.cora);
    t.app
        .get("/loyalty")
        .await
        .assert_ok()
        .assert_see("Reminder not delivered");

    // The lot expires on its date all the same.
    run(&t, last + Duration::days(1)).await;
    let gone = movements(&t, &m, PointsKind::Expire).await;
    assert_eq!(gone.iter().map(|x| x.points).sum::<i64>(), -300);
    assert_eq!(account(&t, &m).await.points_balance, 0);
}

#[renox::test]
async fn a_member_who_opted_out_gets_no_reminder_but_the_others_do() {
    let (t, floor, mailbox, _parties) = world().await;
    let last = today() + Duration::days(20);
    let maria = member(&t, &floor, 7, "0501234567", 300, last).await;
    let omar = member(&t, &floor, 8, "0507654321", 200, last).await;
    // Omar opted out of e-mail.
    let mut tx = t.db().begin().await.unwrap();
    suppressions::add(
        &mut tx,
        NotifyChannel::Email,
        OMAR,
        SuppressionReason::OptOut,
        Some("test"),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    run(&t, today()).await;
    assert_eq!(mailbox.to(MARIA), 1);
    assert_eq!(mailbox.to(OMAR), 0);
    assert_eq!(mailbox.count(), 1);
    // Not receiving it is not a failure of delivery on the account, and his
    // points expire like hers.
    assert!(account(&t, &omar).await.expiry_reminder_failed_at.is_none());
    run(&t, last + Duration::days(1)).await;
    assert_eq!(account(&t, &omar).await.points_balance, 0);
    assert_eq!(account(&t, &maria).await.points_balance, 0);
}
