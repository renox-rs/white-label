//! Customer segments (BL-SAL-25, #144): the rules, the members worked out
//! when a segment is used, blocked parties left out, the editor with its
//! preview count, the clinical items it does not offer and the audited
//! export. The retention campaigns are in `retention.rs`.

mod support;

use renox::chrono::Duration;
use renox::db::now;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::parties::PartyStatus;
use wl_kernel::segments::{SegmentId, Segments};
use wl_sales::permissions as perm;
use wl_sales::segments::{self, SalesSegments};
use wl_sales::{MatchMode, Rule, SalesConfig, Segment, SegmentRefused, WlSales};

use support::floor::Floor;
use support::retention::{Stage, sale_at};
use support::{Booted, boot_configured};

/// S of the plan: no sale for 60 days and more than AED 500 in 12 months.
async fn lapsed(t: &Booted) -> Segment {
    Segment::of(
        "Lapsed",
        MatchMode::All,
        &[
            Rule::NoSaleForDays { days: 60 },
            Rule::SpendOver {
                amount: 50_000,
                days: 365,
            },
        ],
    )
    .store(t.db(), &SalesConfig::default())
    .await
    .unwrap()
    .unwrap()
}

#[renox::test]
async fn a_segment_picks_the_parties_that_meet_its_rules_and_never_a_blocked_one() {
    let stage = Stage::default();
    let t = boot_configured(WlSales::new(), stage.clone()).await;
    let at = now();
    // P1: last sale 90 days ago, AED 600 in the year: meets both rules.
    sale_at(&t, 1, at - Duration::days(90), 60_000, "tee").await;
    // P2: bought 30 days ago: not lapsed.
    sale_at(&t, 2, at - Duration::days(30), 90_000, "tee").await;
    // P3: lapsed but spent only AED 200.
    sale_at(&t, 3, at - Duration::days(100), 20_000, "tee").await;
    // P4: like P1, but blocked.
    sale_at(&t, 4, at - Duration::days(90), 60_000, "tee").await;
    stage.parties.directory.set(4, PartyStatus::Blocked);
    // P5: two sales that add up to AED 600; the last one is 90 days ago.
    sale_at(&t, 5, at - Duration::days(200), 30_000, "tee").await;
    sale_at(&t, 5, at - Duration::days(90), 30_000, "tee").await;
    // P6: like P1 but archived; P7 merged.
    sale_at(&t, 6, at - Duration::days(90), 60_000, "tee").await;
    sale_at(&t, 7, at - Duration::days(90), 60_000, "tee").await;
    stage.parties.directory.set(6, PartyStatus::Archived);
    stage.parties.directory.set(7, PartyStatus::Merged);

    let s = lapsed(&t).await;
    let members = segments::members_at(t.state(), &s, at).await.unwrap();
    assert_eq!(members, vec![PartyId(1), PartyId(5)]);

    // The module's `Segments` answers the same for wl-notify.
    let asked = SalesSegments
        .members_in(t.state(), SegmentId(s.id), at)
        .await
        .unwrap();
    assert_eq!(asked, members);
    // An unknown segment has no members.
    let none = SalesSegments
        .members_in(t.state(), SegmentId(9_999), at)
        .await
        .unwrap();
    assert!(none.is_empty());
}

#[renox::test]
async fn any_means_one_rule_is_enough_and_a_segment_is_worked_out_again_each_time() {
    let stage = Stage::default();
    let t = boot_configured(WlSales::new(), stage).await;
    let at = now();
    sale_at(&t, 1, at - Duration::days(90), 10_000, "tee").await;
    sale_at(&t, 2, at - Duration::days(3), 10_000, "mug").await;
    let s = Segment::of(
        "Lapsed or mug",
        MatchMode::Any,
        &[
            Rule::NoSaleForDays { days: 60 },
            Rule::BoughtItem {
                item: "MUG".into(),
                days: Some(30),
            },
        ],
    )
    .store(t.db(), &SalesConfig::default())
    .await
    .unwrap()
    .unwrap();
    let members = segments::members_at(t.state(), &s, at).await.unwrap();
    assert_eq!(members, vec![PartyId(1), PartyId(2)]);

    // A new lapsed customer is a member at once: no list is kept.
    sale_at(&t, 3, at - Duration::days(120), 10_000, "tee").await;
    let members = segments::members_at(t.state(), &s, at).await.unwrap();
    assert_eq!(members, vec![PartyId(1), PartyId(2), PartyId(3)]);
}

#[renox::test]
async fn enrolled_members_with_points_and_the_rest_are_told_apart() {
    let stage = Stage::default();
    let t = boot_configured(WlSales::new(), stage).await;
    let floor = Floor::new(&t).await;
    floor.turn_on(&t, &[]).await;
    let m = floor.enrol(&t, 7, "0501234567").await;
    floor.earn(&t, &m, 800).await;
    sale_at(&t, 8, now() - Duration::days(5), 10_000, "tee").await;

    let enrolled = Segment::of(
        "Members",
        MatchMode::All,
        &[
            Rule::EnrolledInLoyalty { enrolled: true },
            Rule::LoyaltyBalanceAtLeast { points: 500 },
        ],
    )
    .store(t.db(), &SalesConfig::default())
    .await
    .unwrap()
    .unwrap();
    let members = segments::members_at(t.state(), &enrolled, now())
        .await
        .unwrap();
    assert_eq!(members, vec![PartyId(7)]);

    let others = Segment::of(
        "Not members",
        MatchMode::All,
        &[Rule::EnrolledInLoyalty { enrolled: false }],
    )
    .store(t.db(), &SalesConfig::default())
    .await
    .unwrap()
    .unwrap();
    let members = segments::members_at(t.state(), &others, now())
        .await
        .unwrap();
    assert_eq!(members, vec![PartyId(8)]);
}

#[renox::test]
async fn a_segment_without_rules_or_with_bad_numbers_is_not_saved() {
    let t = boot_configured(WlSales::new(), Stage::default()).await;
    let config = SalesConfig::default();
    let empty = Segment::of("Everybody", MatchMode::All, &[])
        .store(t.db(), &config)
        .await
        .unwrap();
    assert_eq!(empty.unwrap_err(), SegmentRefused::NoRules);
    let bad = Segment::of(
        "Bad",
        MatchMode::All,
        &[Rule::NoSaleForDays { days: 0 }],
    )
    .store(t.db(), &config)
    .await
    .unwrap();
    assert!(matches!(bad.unwrap_err(), SegmentRefused::BadRule(_)));
    let first = lapsed(&t).await;
    let again = Segment::of("Lapsed", MatchMode::Any, &[Rule::NoSaleForDays { days: 5 }])
        .store(t.db(), &config)
        .await
        .unwrap();
    assert_eq!(again.unwrap_err(), SegmentRefused::NameTaken);
    assert_eq!(Segment::find(t.db(), first.id).await.unwrap().unwrap().rules().len(), 2);
}

async fn staff(t: &Booted) -> User {
    t.user("Mia", &[perm::SEGMENTS_MANAGE]).await
}

#[renox::test]
async fn the_editor_saves_a_segment_and_previews_its_size_as_an_estimate() {
    let stage = Stage::default();
    let t = boot_configured(WlSales::new(), stage).await;
    let at = now();
    sale_at(&t, 1, at - Duration::days(90), 60_000, "tee").await;
    sale_at(&t, 2, at - Duration::days(95), 70_000, "tee").await;
    let mia = staff(&t).await;
    t.app.acting_as(&mia);

    t.app.get("/segments").await.assert_ok().assert_see("Customer segments");
    t.app.get("/segments/new").await.assert_ok();
    t.app
        .post(
            "/segments",
            &[
                ("name", "Lapsed"),
                ("match_mode", "all"),
                ("no_sale_days", "60"),
                ("spend_amount", "500"),
                ("spend_days", "365"),
            ],
        )
        .await
        .assert_status(303);
    let s = Segment::query()
        .where_(Segment::NAME.eq("Lapsed".to_owned()))
        .first(t.db())
        .await
        .unwrap()
        .expect("the segment was saved");
    assert_eq!(s.match_mode, MatchMode::All);
    assert_eq!(
        s.rules(),
        vec![
            Rule::NoSaleForDays { days: 60 },
            Rule::SpendOver {
                amount: 50_000,
                days: 365
            }
        ]
    );
    assert_eq!(s.last_member_count, None);

    t.app
        .post(&format!("/segments/{}/preview", s.id), &[("x", "")])
        .await
        .assert_status(303);
    let s = Segment::find(t.db(), s.id).await.unwrap().unwrap();
    assert_eq!(s.last_member_count, Some(2));
    t.app
        .get(&format!("/segments/{}", s.id))
        .await
        .assert_ok()
        .assert_see("Lapsed")
        .assert_see("2");

    // A second segment with the name, and one with no answer, are refused.
    t.app
        .htmx()
        .post("/segments", &[("name", "Lapsed"), ("no_sale_days", "30")])
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post("/segments", &[("name", "Nothing")])
        .await
        .assert_status(422);
    t.app.get("/segments").await.assert_see("Lapsed");

    let nobody = t.user("Nobody", &[]).await;
    t.app.acting_as(&nobody);
    t.app.get("/segments").await.assert_forbidden();
}

#[renox::test]
async fn the_editor_does_not_offer_clinical_items_by_default() {
    let stage = Stage::default();
    let t = boot_configured(WlSales::new().clinical_items(&["XRAY"]), stage).await;
    let at = now();
    sale_at(&t, 1, at - Duration::days(5), 10_000, "XRAY").await;
    sale_at(&t, 1, at - Duration::days(5), 10_000, "SHAMPOO").await;
    let mia = staff(&t).await;
    t.app.acting_as(&mia);
    t.app
        .get("/segments/new")
        .await
        .assert_ok()
        .assert_see("SHAMPOO")
        .assert_dont_see("XRAY");
    // A rule on the clinical item is refused too.
    t.app
        .htmx()
        .post("/segments", &[("name", "Scans"), ("item", "XRAY")])
        .await
        .assert_status(422);
    assert!(
        Segment::query()
            .where_(Segment::NAME.eq("Scans".to_owned()))
            .first(t.db())
            .await
            .unwrap()
            .is_none()
    );
    t.app
        .post("/segments", &[("name", "Shampoo"), ("item", "SHAMPOO")])
        .await
        .assert_status(303);

    // With the setting on, the item is offered.
    let stage = Stage::default();
    let t = boot_configured(
        WlSales::new()
            .clinical_items(&["XRAY"])
            .segment_clinical_rules(true),
        stage,
    )
    .await;
    sale_at(&t, 1, now() - Duration::days(5), 10_000, "XRAY").await;
    let mia = staff(&t).await;
    t.app.acting_as(&mia);
    t.app.get("/segments/new").await.assert_ok().assert_see("XRAY");
}

#[renox::test]
async fn exporting_the_members_needs_the_sensitive_export_permission_and_is_audited() {
    let stage = Stage::default();
    let t = boot_configured(WlSales::new(), stage).await;
    sale_at(&t, 1, now() - Duration::days(90), 60_000, "tee").await;
    let s = lapsed(&t).await;

    let mia = staff(&t).await;
    t.app.acting_as(&mia);
    t.app
        .get(&format!("/segments/{}/export", s.id))
        .await
        .assert_forbidden();
    assert!(
        renox::audit::for_subject(t.db(), Segment::TABLE, s.id, 10)
            .await
            .unwrap()
            .is_empty()
    );

    let boss = t
        .user("Boss", &[perm::SEGMENTS_MANAGE, perm::SEGMENTS_EXPORT])
        .await;
    t.app.acting_as(&boss);
    let csv = t.app.get(&format!("/segments/{}/export", s.id)).await;
    csv.assert_ok().assert_see("party_id,name").assert_see("Party 1");
    let log = renox::audit::for_subject(t.db(), Segment::TABLE, s.id, 10)
        .await
        .unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].action, "sales.segment_exported");
}
