//! BL-MPW-07 (#421), `MPW.ACL.16`, `FIN.RAT.01` and `FIN.USG.01`: rate card
//! versions, accepted by the client, price the hours of a work date; a version
//! dated in the past needs the MD's approval after the client accepts and
//! never reprices invoiced hours or locked timesheets; month rates are split
//! per version; multipliers and minimum hours come from the version in force.

mod support;

use renox::prelude::*;
use wl_invoicing::rate_card::{RateCardLine, RateCardVersion, Unit, VersionStatus};
use wl_invoicing::rate_cards::{self, LineInput};
use wl_invoicing::rate_pricing::{self, MonthWork, Work};
use wl_invoicing::usage::{self, UsageRecord};
use wl_kernel::approvals;
use wl_kernel::rate_cards::rate_cards as kernel_cards;

use support::rates::{
    accept_as, accept_signed, account_manager, card, client_user, contract, d, guard, md, on,
    reload, sheet, version,
};
use support::{BLUE_DUNES, boot};

/// 8 hours of guard on the day shift on `day`.
fn shift(day: &str) -> Work {
    Work::new(contract(), "guard", "day", d(day), "normal", 8_000)
}

/// The price in fils of `work`, or `None` when nothing prices it.
async fn amount_of(t: &support::Booted, work: &Work) -> Option<i64> {
    rate_pricing::price(t.db(), work)
        .await
        .unwrap()
        .map(|priced| priced.amount)
}

/// A usage record as stored now.
async fn stored(t: &support::Booted, record: &UsageRecord) -> UsageRecord {
    UsageRecord::find_or_404(t.db(), record.id).await.unwrap()
}

/// AC1: with V1 (guard AED 25 an hour) and V2 (AED 27) effective 2026-12-01,
/// hours worked on 2026-11-30 are priced at 25 and those on 2026-12-01 at 27.
#[renox::test]
async fn hours_are_priced_by_the_version_in_force_on_the_work_date() {
    let t = boot().await;
    let staff = account_manager(&t, "Maya").await;
    let card = card(&t).await;
    on(&t, "2026-11-15", async {
        let v1 = version(&t, &card, "2026-11-01", guard(25)).await;
        accept_signed(&t, &v1, &staff).await;
        let v2 = version(&t, &card, "2026-12-01", guard(27)).await;
        accept_signed(&t, &v2, &staff).await;

        assert_eq!(amount_of(&t, &shift("2026-11-30")).await, Some(20_000));
        assert_eq!(amount_of(&t, &shift("2026-12-01")).await, Some(21_600));
        assert_eq!(amount_of(&t, &shift("2026-12-20")).await, Some(21_600));
        // Before the first version nothing prices the day.
        assert_eq!(amount_of(&t, &shift("2026-10-31")).await, None);

        // As usage records for the billing run, at the version's own rate.
        let before = rate_pricing::record_hours(t.db(), &sheet(1), &shift("2026-11-30"), None)
            .await
            .unwrap()
            .unwrap();
        let after = rate_pricing::record_hours(t.db(), &sheet(2), &shift("2026-12-01"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!((before.unit_price, before.quantity_milli), (2_500, 8_000));
        assert_eq!((after.unit_price, after.quantity_milli), (2_700, 8_000));
        assert_eq!(before.draft_line().net(), 20_000);
        assert_eq!(after.draft_line().net(), 21_600);
    })
    .await;
}

/// AC2: editing a line of accepted V1 is refused and the user is offered a new
/// version; so is adding a line to it.
#[renox::test]
async fn editing_a_line_of_an_accepted_version_is_refused_and_a_new_version_is_offered() {
    let t = boot().await;
    let staff = account_manager(&t, "Maya").await;
    let card = card(&t).await;
    on(&t, "2026-11-15", async {
        let v1 = version(&t, &card, "2026-11-01", guard(25)).await;
        // While pending the line may still be changed.
        let line = v1.lines(t.db()).await.unwrap().remove(0);
        t.post_as(
            &format!("/rate-card-lines/{}", line.id),
            &staff,
            &[("role", "guard"), ("shift_type", "day"), ("rate", "26")],
        )
        .await
        .assert_status(303);
        assert_eq!(
            RateCardLine::find_or_404(t.db(), line.id)
                .await
                .unwrap()
                .rate,
            2_600
        );
        accept_signed(&t, &v1, &staff).await;

        let refused = t
            .post_as(
                &format!("/rate-card-lines/{}", line.id),
                &staff,
                &[("role", "guard"), ("shift_type", "day"), ("rate", "30")],
            )
            .await;
        refused.assert_status(422);
        assert!(refused.text().contains("new version"), "{}", refused.text());
        assert_eq!(
            RateCardLine::find_or_404(t.db(), line.id)
                .await
                .unwrap()
                .rate,
            2_600,
            "the rate did not change"
        );
        let added = t
            .post_as(
                &format!("/rate-card-versions/{}/lines", v1.id),
                &staff,
                &[("role", "cleaner"), ("shift_type", "day"), ("rate", "18")],
            )
            .await;
        added.assert_status(422);
        assert!(added.text().contains("new version"));
        assert_eq!(v1.lines(t.db()).await.unwrap().len(), 1);
    })
    .await;
}

/// AC3 and AC4: V2 made on 2026-12-10 with effective date 2026-12-01 needs the
/// MD after the client accepts; until then December hours price at V1. After
/// the approval the unlocked December hours price at 27, while the timesheet
/// locked on 2026-12-08 for 12-01 to 12-07 and the hours already invoiced keep
/// AED 25.
#[renox::test]
async fn a_version_dated_in_the_past_waits_for_the_md_and_never_reprices_locked_or_invoiced_hours()
{
    let t = boot().await;
    let staff = account_manager(&t, "Maya").await;
    let client = client_user(&t, "Client").await;
    let mona = md(&t, "Mona").await;
    let card = card(&t).await;
    on(&t, "2026-12-10", async {
        let v1 = version(&t, &card, "2026-11-01", guard(25)).await;
        assert!(!v1.past_dated, "the first version is a start, not a change");
        accept_signed(&t, &v1, &staff).await;

        // The 3 December hours are invoiced before the new version.
        let invoiced = rate_pricing::record_hours(t.db(), &sheet(9), &shift("2026-12-03"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(usage::bill(t.db(), BLUE_DUNES).await.unwrap().len(), 1);
        let locked_one = rate_pricing::record_hours(t.db(), &sheet(7), &shift("2026-12-01"), None)
            .await
            .unwrap()
            .unwrap();
        let locked_two = rate_pricing::record_hours(t.db(), &sheet(7), &shift("2026-12-02"), None)
            .await
            .unwrap()
            .unwrap();
        let open = rate_pricing::record_hours(t.db(), &sheet(8), &shift("2026-12-08"), None)
            .await
            .unwrap()
            .unwrap();
        t.sheets.lock(7);

        let v2 = version(&t, &card, "2026-12-01", guard(27)).await;
        assert!(v2.past_dated);
        // The client accepts in the portal: the MD is asked.
        accept_as(&t, &v2, &client, &[]).await.assert_status(303);
        let waiting = reload(&t, &v2).await;
        assert_eq!(waiting.status, VersionStatus::AwaitingApproval);
        assert_eq!(waiting.accepted_by, Some(client.id));
        let request = approvals::pending_for(t.db(), &waiting.subject())
            .await
            .unwrap()
            .remove(0);
        // Before the approval December prices at V1.
        assert_eq!(amount_of(&t, &shift("2026-12-05")).await, Some(20_000));
        assert_eq!(stored(&t, &open).await.unit_price, 2_500);
        // The client who accepted cannot approve; the MD can.
        assert!(
            approvals::approve(t.state(), request.id, &client, "")
                .await
                .is_err()
        );
        approvals::approve(t.state(), request.id, &mona, "")
            .await
            .unwrap();

        let accepted = reload(&t, &v2).await;
        assert_eq!(accepted.status, VersionStatus::Accepted);
        assert_eq!(accepted.approved_by, Some(mona.id));
        assert_eq!(amount_of(&t, &shift("2026-12-05")).await, Some(21_600));
        assert_eq!(stored(&t, &open).await.unit_price, 2_700, "unlocked hours");
        assert_eq!(
            stored(&t, &locked_one).await.unit_price,
            2_500,
            "locked sheet"
        );
        assert_eq!(
            stored(&t, &locked_two).await.unit_price,
            2_500,
            "locked sheet"
        );
        assert_eq!(
            stored(&t, &invoiced).await.unit_price,
            2_500,
            "invoiced hours"
        );
    })
    .await;
}

/// AC3 (the other answer): when the MD rejects, the version is not used and
/// December keeps pricing at V1.
#[renox::test]
async fn a_version_the_md_rejects_is_never_used() {
    let t = boot().await;
    let staff = account_manager(&t, "Maya").await;
    let mona = md(&t, "Mona").await;
    let card = card(&t).await;
    on(&t, "2026-12-10", async {
        let v1 = version(&t, &card, "2026-11-01", guard(25)).await;
        accept_signed(&t, &v1, &staff).await;
        let v2 = version(&t, &card, "2026-12-01", guard(27)).await;
        accept_signed(&t, &v2, &staff).await;
        let waiting = reload(&t, &v2).await;
        assert_eq!(waiting.status, VersionStatus::AwaitingApproval);
        let request = approvals::pending_for(t.db(), &waiting.subject())
            .await
            .unwrap()
            .remove(0);
        approvals::reject(t.state(), request.id, &mona, "Not agreed with the client")
            .await
            .unwrap();
        let rejected = reload(&t, &v2).await;
        assert_eq!(rejected.status, VersionStatus::Rejected);
        assert_eq!(amount_of(&t, &shift("2026-12-05")).await, Some(20_000));
        // The date is free again for a new version.
        rate_cards::new_version(t.db(), None, card.id, d("2026-12-01"), false)
            .await
            .unwrap();
    })
    .await;
}

/// AC5: a version pending the client's acceptance prices nothing, and the
/// contract the verticals ask does not see it.
#[renox::test]
async fn a_version_pending_client_acceptance_prices_nothing() {
    let t = boot().await;
    let staff = account_manager(&t, "Maya").await;
    let card = card(&t).await;
    on(&t, "2026-11-15", async {
        let v1 = version(&t, &card, "2026-11-01", guard(25)).await;
        let source = contract();
        let asked = kernel_cards(t.state());
        assert_eq!(amount_of(&t, &shift("2026-11-20")).await, None);
        assert!(
            rate_pricing::record_hours(t.db(), &sheet(1), &shift("2026-11-20"), None)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            asked
                .accepted_version(t.state(), &source, d("2026-11-20"))
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            !asked
                .covers(t.state(), &source, "guard", "day", d("2026-11-20"))
                .await
                .unwrap()
        );

        accept_signed(&t, &v1, &staff).await;
        let found = asked
            .accepted_version(t.state(), &source, d("2026-11-20"))
            .await
            .unwrap()
            .expect("the accepted version");
        assert_eq!((found.version, found.effective_from), (1, d("2026-11-01")));
        assert!(
            asked
                .covers(t.state(), &source, "Guard", "day", d("2026-11-20"))
                .await
                .unwrap()
        );
        assert!(
            !asked
                .covers(t.state(), &source, "cleaner", "day", d("2026-11-20"))
                .await
                .unwrap()
        );
        assert_eq!(amount_of(&t, &shift("2026-11-20")).await, Some(20_000));
    })
    .await;
}

/// AC6: with V1 at AED 3,100 a month and V2 at AED 3,410 effective 2026-12-11,
/// December bills 3,100 x 10 / 31 = AED 1,000.00 and 3,410 x 21 / 31 =
/// AED 2,310.00, one line per version.
#[renox::test]
async fn a_month_rate_is_split_pro_rata_per_version() {
    let t = boot().await;
    let staff = account_manager(&t, "Maya").await;
    let card = card(&t).await;
    on(&t, "2026-11-15", async {
        let month = |rate: i64| LineInput::new("guard", "day", Unit::Month, rate * 100);
        let v1 = version(&t, &card, "2026-11-01", month(3_100)).await;
        accept_signed(&t, &v1, &staff).await;
        let v2 = version(&t, &card, "2026-12-11", month(3_410)).await;
        accept_signed(&t, &v2, &staff).await;

        let december = MonthWork {
            source: contract(),
            role: "guard".into(),
            shift_type: "day".into(),
            year: 2026,
            month: 12,
        };
        let lines = rate_pricing::price_month(t.db(), &december).await.unwrap();
        let seen: Vec<_> = lines
            .iter()
            .map(|l| (l.version.version, l.from, l.to, l.days, l.amount))
            .collect();
        assert_eq!(
            seen,
            [
                (1, d("2026-12-01"), d("2026-12-10"), 10, 100_000),
                (2, d("2026-12-11"), d("2026-12-31"), 21, 231_000),
            ]
        );
        let records = rate_pricing::record_month(
            t.db(),
            &wl_kernel::Subject::new("manpower.client_contract", 4),
            &december,
            None,
        )
        .await
        .unwrap();
        let nets: Vec<_> = records.iter().map(|r| r.draft_line().net()).collect();
        assert_eq!(nets, [100_000, 231_000]);
        // An hourly price of a month line is not a thing.
        assert_eq!(amount_of(&t, &shift("2026-12-15")).await, None);
    })
    .await;
}

/// AC7: with V2 at AED 27 an hour, an overtime multiplier 1.25 and a minimum of
/// 4 hours per shift, 2 overtime hours on 2026-12-02 bill 2 x 27 x 1.25 =
/// AED 67.50 and a 3-hour shift on 2026-12-03 bills 4 x 27 = AED 108.00; the
/// version in force on the work date gives the multiplier.
#[renox::test]
async fn multipliers_and_minimum_hours_are_those_of_the_version_in_force() {
    let t = boot().await;
    let staff = account_manager(&t, "Maya").await;
    let card = card(&t).await;
    on(&t, "2026-11-15", async {
        let v1 = version(&t, &card, "2026-11-01", guard(25).overtime(150)).await;
        accept_signed(&t, &v1, &staff).await;
        let v2 = version(
            &t,
            &card,
            "2026-12-01",
            guard(27).overtime(125).minimum_hours_milli(4_000),
        )
        .await;
        accept_signed(&t, &v2, &staff).await;

        let overtime = Work::new(
            contract(),
            "guard",
            "day",
            d("2026-12-02"),
            "overtime",
            2_000,
        );
        assert_eq!(amount_of(&t, &overtime).await, Some(6_750));
        let short = Work::new(contract(), "guard", "day", d("2026-12-03"), "normal", 3_000);
        assert_eq!(amount_of(&t, &short).await, Some(10_800));
        // The minimum is not applied to overtime, and V1 has its own 1.50.
        let old = Work::new(
            contract(),
            "guard",
            "day",
            d("2026-11-20"),
            "overtime",
            2_000,
        );
        assert_eq!(amount_of(&t, &old).await, Some(7_500));
        let old_short = Work::new(contract(), "guard", "day", d("2026-11-20"), "normal", 3_000);
        assert_eq!(amount_of(&t, &old_short).await, Some(7_500));

        let record = rate_pricing::record_hours(t.db(), &sheet(5), &overtime, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!((record.quantity_milli, record.unit_price), (2_000, 3_375));
        assert_eq!(record.draft_line().net(), 6_750);
        let record = rate_pricing::record_hours(t.db(), &sheet(5), &short, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!((record.quantity_milli, record.unit_price), (4_000, 2_700));
        assert_eq!(record.draft_line().net(), 10_800);
    })
    .await;
}

/// `MPW.ACL.16`: two versions of a contract never share an effective date; a
/// rate that is not whole fils is rounded once, half up, after the
/// multiplying.
#[renox::test]
async fn two_versions_never_share_a_date_and_an_amount_is_rounded_once() {
    let t = boot().await;
    let staff = account_manager(&t, "Maya").await;
    let card = card(&t).await;
    on(&t, "2026-11-15", async {
        let v1 = version(
            &t,
            &card,
            "2026-11-01",
            LineInput::new("guard", "day", Unit::Hour, 2_750).overtime(115),
        )
        .await;
        accept_signed(&t, &v1, &staff).await;
        let refused = rate_cards::new_version(t.db(), None, card.id, d("2026-11-01"), false).await;
        assert!(refused.is_err(), "the date is taken");
        let overtime = Work::new(
            contract(),
            "guard",
            "day",
            d("2026-11-20"),
            "overtime",
            1_000,
        );
        // 27.50 x 1.15 = 31.625: 31.63, in one line.
        assert_eq!(amount_of(&t, &overtime).await, Some(3_163));
        let record = rate_pricing::record_hours(t.db(), &sheet(5), &overtime, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.draft_line().net(), 3_163);
    })
    .await;
}

/// `DOC.SIG.01`: only the client's portal users with the role `approver` or
/// `owner`, or staff who record the signed acceptance, accept a version.
#[renox::test]
async fn only_the_client_or_staff_with_the_signed_document_accept_a_version() {
    let t = boot().await;
    let staff = account_manager(&t, "Maya").await;
    let stranger = t.user("Sam", &[]).await;
    let viewer = t.user("Vic", &[]).await;
    t.portal.grant(&viewer, BLUE_DUNES, &["viewer"]);
    let client = client_user(&t, "Client").await;
    let card = card(&t).await;
    on(&t, "2026-11-15", async {
        let v1 = version(&t, &card, "2026-11-01", guard(25)).await;
        accept_as(&t, &v1, &stranger, &[]).await.assert_forbidden();
        accept_as(&t, &v1, &viewer, &[]).await.assert_forbidden();
        // Staff must name the signed document.
        accept_as(&t, &v1, &staff, &[]).await.assert_status(422);
        assert_eq!(reload(&t, &v1).await.status, VersionStatus::Pending);
        accept_as(&t, &v1, &client, &[]).await.assert_status(303);
        let accepted: RateCardVersion = reload(&t, &v1).await;
        assert_eq!(accepted.status, VersionStatus::Accepted);
        assert_eq!(accepted.accepted_by, Some(client.id));
        assert_eq!(accepted.accepted_document, None);
        // Accepted once; and a version without lines cannot be accepted.
        accept_as(&t, &v1, &client, &[]).await.assert_status(422);
        let empty = rate_cards::new_version(t.db(), None, card.id, d("2027-01-01"), false)
            .await
            .unwrap();
        accept_as(&t, &empty, &client, &[]).await.assert_status(422);
    })
    .await;
}
