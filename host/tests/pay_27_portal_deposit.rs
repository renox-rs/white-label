//! BL-PAY-27 (#466) in the real Property host: the tenant sees her deposit,
//! the published deductions and the refund status in the portal, and disputes
//! lines while the window is open, also with a read-only grant after
//! move-out.
//!
//! The deductions are published on 2 November 2026 when the clock is before
//! that day (the clock travels there); on a later run they are published
//! today and the window is checked from that day, the same 14 days.

mod support;

use host::roles::{ACCOUNTANT, MEMBER};
use property::UnitUse;
use renox::chrono::{Days, NaiveDate, NaiveDateTime};
use renox::prelude::*;
use renox::testing::TestApp;
use support::*;
use wl_kernel::documents::today;
use wl_kernel::{BusinessCalendar, PartyId};
use wl_payments::Deposit;
use wl_payments::deposit_deductions::{DepositDeduction, DepositDispute, gate_of};
use wl_portal::{NewGrant, PortalAccess};

/// Moves the clock to noon of `day` (Asia/Dubai), which is ahead of it.
fn travel_to(app: &TestApp, day: NaiveDate) {
    let local = NaiveDateTime::new(
        day,
        renox::chrono::NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
    );
    let ahead = (BusinessCalendar::default().from_local(local) - renox::db::now()).num_seconds();
    assert!(ahead > 0, "{day} is ahead of the clock");
    app.travel(std::time::Duration::from_secs(ahead.unsigned_abs()));
}

/// The setting: a deposit of AED 10,000 for Sara's lease, settling, with
/// her portal grant. Returns the tenant, the deposit, Sara, her grant and
/// the accountant.
async fn settling(app: &TestApp) -> (PartyId, Deposit, User, i64, User) {
    let accountant = person(app, "Amal", ACCOUNTANT).await;
    let party = tenant(app, "Sara Haddad").await;
    let unit = unit_of(app, "TWR-D", "803", UnitUse::Residential).await;
    let lease = draft_lease(app, &unit, party, -300).await;
    let deposit = settling_deposit(app, &accountant, party, &lease, 1_000_000).await;
    let sara = person(app, "Sara", MEMBER).await;
    let grant = PortalAccess::register(
        app.state(),
        sara.id,
        NewGrant::new(party, party, "tenant", &["primary_contact"]),
    )
    .await
    .unwrap();
    (party, deposit, sara, grant.id, accountant)
}

/// Publishes cleaning AED 800 and wall repair AED 1,200 on 2 November 2026
/// (or today, on a later run); returns the publication day.
async fn publish(app: &TestApp, accountant: &User, deposit: &Deposit) -> NaiveDate {
    let november = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
    if today() < november {
        travel_to(app, november);
    }
    app.acting_as(accountant);
    app.htmx()
        .post(
            &format!("/deposits/{}/deductions", deposit.id),
            &[
                (
                    "lines",
                    "Cleaning | 800.00 | move-out photos 1-4\nWall repair | 1200.00 | move-out photos 5-9",
                ),
                ("forfeiture_reason", ""),
            ],
        )
        .await
        .assert_ok();
    DepositDeduction::latest(app.db(), deposit.id)
        .await
        .unwrap()
        .expect("published")
        .published_on
}

fn until(day: NaiveDate) -> String {
    format!("until {}", day.format("%-d %b %Y"))
}

fn page(party: PartyId) -> String {
    format!("/portal/{}/deposit", party.0)
}

async fn wall_line(app: &TestApp, deposit: &Deposit) -> i64 {
    let latest = DepositDeduction::latest(app.db(), deposit.id)
        .await
        .unwrap()
        .unwrap();
    latest
        .lines(app.db())
        .await
        .unwrap()
        .into_iter()
        .find(|l| l.description == "Wall repair")
        .expect("the wall repair line")
        .id
}

async fn dispute_wall(app: &TestApp, party: PartyId, deposit: &Deposit, version: i32) -> u16 {
    let line = wall_line(app, deposit).await;
    let key = format!("line_{line}");
    let version = version.to_string();
    app.htmx()
        .post(
            &format!("/portal/{}/deposit/{}/dispute", party.0, deposit.id),
            &[
                ("version", version.as_str()),
                (key.as_str(), "on"),
                ("comment", "pre-existing damage"),
            ],
        )
        .await
        .status
        .as_u16()
}

#[renox::test]
async fn sara_sees_the_lines_the_window_and_the_refund_in_review_then_disputes() {
    let app = booted().await;
    let (party, deposit, sara, _, accountant) = settling(&app).await;
    let published = publish(&app, &accountant, &deposit).await;
    let window_end = published + Days::new(14);

    app.acting_as(&sara);
    app.get(&page(party))
        .await
        .assert_ok()
        .assert_see("AED 10,000.00")
        .assert_see("Cleaning")
        .assert_see("AED 800.00")
        .assert_see("Wall repair")
        .assert_see("AED 1,200.00")
        .assert_see("move-out photos 5-9")
        .assert_see(&until(window_end))
        .assert_see("In review")
        .assert_see("Send the dispute");

    assert_eq!(dispute_wall(&app, party, &deposit, 1).await, 200);
    let open = DepositDispute::open_of_deposit(app.db(), deposit.id)
        .await
        .unwrap();
    assert_eq!(open.len(), 1, "the dispute flag is set");
    let gate = app
        .at_travelled_time(gate_of(app.state(), &deposit))
        .await
        .unwrap();
    assert_eq!(gate.open_disputes, 1, "the refund waits");
    // The disputed line shows as such.
    app.get(&page(party))
        .await
        .assert_ok()
        .assert_see("Disputed")
        .assert_see("In review");
    app.travel_back();
}

#[renox::test]
async fn the_day_after_the_window_the_dispute_action_is_gone() {
    let app = booted().await;
    let (party, deposit, sara, _, accountant) = settling(&app).await;
    let published = publish(&app, &accountant, &deposit).await;
    // The last day counts; the next day the window is closed.
    app.travel_back();
    travel_to(&app, published + Days::new(15));
    app.acting_as(&sara);
    app.get(&page(party))
        .await
        .assert_ok()
        .assert_see("Wall repair")
        .assert_dont_see("Send the dispute");
    assert_eq!(dispute_wall(&app, party, &deposit, 1).await, 422);
    app.travel_back();
}

#[renox::test]
async fn with_a_read_only_grant_after_move_out_she_can_still_dispute() {
    let app = booted().await;
    let (party, deposit, sara, grant, accountant) = settling(&app).await;
    publish(&app, &accountant, &deposit).await;
    app.at_travelled_time(PortalAccess::end(
        app.state(),
        None,
        grant,
        Some("lease terminated"),
    ))
    .await
    .unwrap();
    app.acting_as(&sara);
    app.get(&page(party))
        .await
        .assert_ok()
        .assert_see("read-only")
        .assert_see("Send the dispute");
    assert_eq!(dispute_wall(&app, party, &deposit, 1).await, 200);
    assert_eq!(
        DepositDispute::open_of_deposit(app.db(), deposit.id)
            .await
            .unwrap()
            .len(),
        1
    );
    app.travel_back();
}

#[renox::test]
async fn a_revision_shows_version_two_and_a_new_window() {
    let app = booted().await;
    let (party, deposit, sara, _, accountant) = settling(&app).await;
    let published = publish(&app, &accountant, &deposit).await;
    app.acting_as(&sara);
    assert_eq!(dispute_wall(&app, party, &deposit, 1).await, 200);

    // Three days later staff revise the wall repair to AED 600.
    app.travel_back();
    let revised_on = published + Days::new(3);
    travel_to(&app, revised_on);
    let line = wall_line(&app, &deposit).await;
    let changes = format!("{line} | 600.00");
    app.acting_as(&accountant);
    app.htmx()
        .post(
            &format!("/deposits/{}/answer", deposit.id),
            &[
                ("response", "Partly agreed: the repair was smaller"),
                ("evidence", "inspection report"),
                ("changes", changes.as_str()),
            ],
        )
        .await
        .assert_ok();
    let latest = DepositDeduction::latest(app.db(), deposit.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest.version, 2);

    app.acting_as(&sara);
    app.get(&page(party))
        .await
        .assert_ok()
        .assert_see("Version 2")
        .assert_see("AED 600.00")
        .assert_see(&until(revised_on + Days::new(14)))
        .assert_dont_see("Disputed");
    app.travel_back();
}

#[renox::test]
async fn another_party_sees_nothing_and_cannot_dispute() {
    let app = booted().await;
    let (party, deposit, _, _, accountant) = settling(&app).await;
    publish(&app, &accountant, &deposit).await;
    let other = tenant(&app, "Omar Saleh").await;
    let stranger = person(&app, "Stranger", MEMBER).await;
    PortalAccess::register(
        app.state(),
        stranger.id,
        NewGrant::new(other, other, "tenant", &["primary_contact"]),
    )
    .await
    .unwrap();
    app.acting_as(&stranger);
    app.get(&page(party)).await.assert_forbidden();
    app.get(&page(other))
        .await
        .assert_ok()
        .assert_dont_see("Wall repair");
    let line = wall_line(&app, &deposit).await;
    let key = format!("line_{line}");
    let refused = app
        .htmx()
        .post(
            &format!("/portal/{}/deposit/{}/dispute", other.0, deposit.id),
            &[
                ("version", "1"),
                (key.as_str(), "on"),
                ("comment", "not mine"),
            ],
        )
        .await;
    assert_eq!(refused.status.as_u16(), 404);
    app.travel_back();
}
