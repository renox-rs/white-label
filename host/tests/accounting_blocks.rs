//! BL-ACC-23 (#347), screening and blocks, through the real host of the 04
//! Accounting client: a sanctions list older than 24 hours refuses CDD
//! `submit` until the MLRO uploads the list by hand, a confirmed match on a
//! UBO blocks the client company and the partner sees *On hold* (04 §4.6,
//! §7.1 *Block effect*, §7.2 `PTY.RUL.02`, §8).
//!
//! The refund of a prepaid retainer to a blocked client waits for the MLRO's
//! exception (`PTY.ACL.03`): wl-payments and wl-invoicing do not ask the block
//! guard for it yet (see `DEFERRED-batch3.md`).

mod support;

use accounting::cdd::{RiskRating, ScreenedRole};
use accounting::cdd_files::{self, Assessment};
use accounting::{Engagement, EngagementStatus};
use host::roles::{ACCOUNTANT, COMPLIANCE, PARTNER};
use renox::auth::permissions::Scope;
use renox::chrono::Duration;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{accounting_booted, company, person, tenant};
use wl_kernel::block_effects::{BlockEffect, RuleSource, effect_for};
use wl_kernel::{PartyId, PartyStatus};
use wl_parties::{
    ListStatus, ListUsed, NewCandidate, Party, SanctionsList, Screening, ScreeningRun,
    ScreeningTrigger, record_run,
};

const UN: &str = "UN Consolidated List";

/// A prepared CDD file: the client company with a UBO, a director and a
/// signatory, rated low, identity documents attached.
async fn prepared(app: &TestApp, tag: &str) -> (PartyId, PartyId, accounting::CddFile) {
    let company = company(app, &format!("Gulf Trading {tag} LLC")).await;
    let ubo = tenant(app, &format!("Omar Haddad {tag}")).await;
    let file = cdd_files::open(app.db(), None, company).await.unwrap();
    cdd_files::add_party(app.state(), file.id, ubo, ScreenedRole::Ubo)
        .await
        .unwrap();
    let mut assessment = Assessment::rated(RiskRating::Low);
    assessment.identity_documents_attached = true;
    let file = cdd_files::assess(app.state(), file.id, assessment)
        .await
        .unwrap();
    (company, ubo, file)
}

/// The list `name`, as stored now.
async fn list(app: &TestApp, name: &str) -> SanctionsList {
    SanctionsList::all(app.db())
        .await
        .unwrap()
        .into_iter()
        .find(|l| l.name == name)
        .unwrap_or_else(|| panic!("the list {name}"))
}

/// Every list updated `hours` ago except `old`, which was updated
/// `old_hours` ago; the freshness check then judges them.
async fn age_lists(app: &TestApp, old: &str, old_hours: i64, hours: i64) {
    let now = renox::db::now();
    for mut l in SanctionsList::all(app.db()).await.unwrap() {
        let age = if l.name == old { old_hours } else { hours };
        l.last_updated_at = Some(now - Duration::hours(age));
        l.last_error = None;
        // The module's own judgement with the 04 limit of 24 hours (no list
        // feed is wired yet, so the hourly download would always fail).
        if l.is_stale_at(now, 24) {
            l.status = ListStatus::Stale;
            l.stale_since = Some(now);
        } else {
            l.status = ListStatus::Current;
            l.stale_since = None;
        }
        l.save(app.db()).await.unwrap();
    }
}

/// Done when 2: with the UN list 25 hours old, CDD `submit` is refused naming
/// it; 23 hours old it is not stale; the MLRO can upload the list by hand.
#[renox::test]
async fn a_list_older_than_24_hours_refuses_cdd_submit_until_the_mlro_uploads_it() {
    let app = accounting_booted().await;
    let preparer = person(&app, "Asha", ACCOUNTANT).await;
    let mlro = person(&app, "Mariam", COMPLIANCE).await;
    let (_company, _ubo, file) = prepared(&app, "A").await;
    let submit = format!("/accounting/cdd/{}/submit", file.id);

    // 23 hours old: still current.
    age_lists(&app, UN, 23, 1).await;
    assert_eq!(list(&app, UN).await.status, ListStatus::Current);
    app.acting_as(&preparer);
    let fresh = app.htmx().post(&submit, &[]).await;
    fresh.assert_status(422);
    assert!(
        !fresh.text().contains("out of date"),
        "a list 23 hours old is not stale: {}",
        fresh.text()
    );

    // 25 hours old: stale; submit is refused and names the list.
    age_lists(&app, UN, 25, 1).await;
    let stale = list(&app, UN).await;
    assert_eq!(stale.status, ListStatus::Stale);
    app.acting_as(&preparer);
    let refused = app.htmx().post(&submit, &[]).await;
    refused.assert_status(422);
    assert!(
        refused
            .text()
            .contains("Screening list is out of date: UN Consolidated List"),
        "{}",
        refused.text()
    );

    // The MLRO uploads the official list by hand; the list is current.
    app.acting_as(&mlro);
    app.htmx()
        .post_multipart(
            &format!("/parties/screening/lists/{}/upload", stale.id),
            &[
                ("source_url", "https://example.org/un-list"),
                ("list_date", &wl_kernel::documents::today().to_string()),
            ],
            &[(
                "file",
                "un.xml",
                b"<list><entry><name>Omar K.</name></entry></list>",
            )],
        )
        .await
        .assert_ok();
    let uploaded = list(&app, UN).await;
    assert_eq!(uploaded.status, ListStatus::Current);
    assert_eq!(
        uploaded.update_source,
        Some(wl_parties::UpdateSource::Manual)
    );
    // The accountant cannot upload one.
    app.acting_as(&preparer);
    let forbidden = app
        .htmx()
        .post_multipart(
            &format!("/parties/screening/lists/{}/upload", stale.id),
            &[("list_date", &wl_kernel::documents::today().to_string())],
            &[("file", "un.xml", b"<list></list>")],
        )
        .await;
    assert!(
        forbidden.status.as_u16() == 403 || forbidden.status.as_u16() == 404,
        "{}",
        forbidden.status
    );
}

/// An `active` engagement of `company`, run by a fixture.
async fn active_engagement(app: &TestApp, company: PartyId) -> Engagement {
    let mut engagement = Engagement {
        client_party_id: company.0,
        title: "Bookkeeping and VAT".to_owned(),
        retainer_fee: 500_000,
        letter_signed: true,
        status: EngagementStatus::Active,
        ..Engagement::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    engagement
        .insert_in_state(&mut tx, EngagementStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    Engagement::find_or_404(app.db(), engagement.id)
        .await
        .unwrap()
}

/// A possible match on the UN list for `party`, recorded as the screening
/// run would.
async fn possible_match(app: &TestApp, party: PartyId) -> Screening {
    let candidate = NewCandidate {
        name: "Omar H. Haddad".to_owned(),
        list_name: UN.to_owned(),
        score: 0.95,
        list_entry: Some("QDi.001".to_owned()),
    };
    let mut run = ScreeningRun::automatic(ScreeningTrigger::Onboarding, vec![candidate]);
    run.lists_used = SanctionsList::all(app.db())
        .await
        .unwrap()
        .into_iter()
        .map(|l| ListUsed {
            list_id: l.id,
            name: l.name.clone(),
            version: l.version.clone(),
            last_updated_at: Some(renox::db::now()),
            stale: false,
        })
        .collect();
    let row = Party::find(app.db(), party.0).await.unwrap().unwrap();
    record_run(app.state(), &row, run, None).await.unwrap()
}

/// Done when 3: a confirmed match on a UBO blocks the client company (the
/// related-person block is on), the engagement goes *On hold*, and a partner
/// sees *On hold* and nothing of the reason.
#[renox::test]
async fn a_confirmed_match_on_a_ubo_blocks_the_company_and_the_partner_sees_on_hold() {
    let app = accounting_booted().await;
    let mlro = person(&app, "Mariam", COMPLIANCE).await;
    let deputy = person(&app, "Dina", COMPLIANCE).await;
    let partner = person(&app, "Pia", PARTNER).await;
    let (company, ubo, _file) = prepared(&app, "A").await;
    let engagement = active_engagement(&app, company).await;
    let screening = possible_match(&app, ubo).await;

    // The first reviewer confirms; the second approves (wl-parties §7).
    app.acting_as(&mlro);
    app.post(&format!("/probe/confirm/{}", screening.id), &[])
        .await
        .assert_ok();
    let request = wl_kernel::ApprovalRequest::query()
        .where_(wl_kernel::ApprovalRequest::SUBJECT_TYPE.eq("parties.screening"))
        .where_(wl_kernel::ApprovalRequest::SUBJECT_ID.eq(screening.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("the confirmation waits for the second reviewer");
    wl_kernel::approvals::approve(app.state(), request.id, &deputy, "ok")
        .await
        .unwrap();

    let ubo_row = Party::find(app.db(), ubo.0).await.unwrap().unwrap();
    let company_row = Party::find(app.db(), company.0).await.unwrap().unwrap();
    assert_eq!(ubo_row.status, PartyStatus::Blocked);
    assert_eq!(
        company_row.status,
        PartyStatus::Blocked,
        "the related-person block reaches the client company"
    );
    let suspended = Engagement::find_or_404(app.db(), engagement.id)
        .await
        .unwrap();
    assert_eq!(suspended.status, EngagementStatus::Suspended);
    assert_eq!(suspended.reason_code.as_deref(), Some("on_hold"));

    // The partner sees On hold, not the reason, on the engagement.
    app.acting_as(&partner);
    let page = app
        .get(&format!("/accounting/engagements/{}", engagement.id))
        .await;
    page.assert_ok().assert_see("On hold");
    let html = page.text().to_lowercase();
    assert!(!html.contains("sanction"), "no reason for the partner");
}

/// §7.1 *Block effect*: blocking a client pauses its engagement and its
/// retainer, and the MLRO decides whether they end.
#[renox::test]
async fn the_block_effect_of_an_engagement_and_its_retainer_is_pause_decided_by_the_mlro() {
    let _app = accounting_booted().await;
    for kind in ["engagement", "subscription"] {
        let rule = effect_for(kind, &Scope::global());
        assert_eq!(rule.effect, BlockEffect::Pause, "{kind}");
        assert_eq!(rule.decider_role.as_deref(), Some(COMPLIANCE), "{kind}");
        assert_eq!(rule.source, RuleSource::Vertical, "{kind}");
    }
}

/// The sanctions lists the 04 client screens against are seeded: the UN
/// Consolidated List (XML) and the UAE Local Terrorist List.
#[renox::test]
async fn the_lists_of_the_04_client_are_seeded() {
    let app = accounting_booted().await;
    let names: Vec<String> = SanctionsList::all(app.db())
        .await
        .unwrap()
        .into_iter()
        .map(|l| l.name)
        .collect();
    assert!(names.contains(&UN.to_owned()), "{names:?}");
    assert!(
        names.contains(&"UAE Local Terrorist List".to_owned()),
        "{names:?}"
    );
}
