//! BL-ACC-23 (#347), documents, through the real host of the 04 Accounting
//! client: the KYC kinds block engagement `activate` with the MLRO's override
//! and flag `return.file`, their expiry goes to the compliance officer, the
//! registration kinds go to the partner, and the look-ahead lists the work
//! due after an expiry (04 §9 *Document blocks*, §7.2 `DOC.RUL.01` to
//! `DOC.RUL.03`).
//!
//! The FTA tax agent registration (a qualification of the firm's staff,
//! `HRM.QUA.01`) waits for the Qualification records of wl-hr (see
//! `DEFERRED-batch3.md`).

mod support;

use accounting::cdd::ScreenedRole;
use accounting::engagement::NewEngagement;
use accounting::{Engagement, Return, ReturnType, cdd_files, engagements};
use host::roles::{COMPLIANCE, PARTNER};
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::testing::TestApp;
use support::{accounting_booted, company, entity, person, tenant};
use wl_documents::expiry;
use wl_documents::{Document, DocumentKind, DocumentStatus};
use wl_kernel::PartyId;
use wl_kernel::documents::{self, BlockMode, OverrideMode, today};
use wl_notify::Message;

/// A client company with a UBO, an open CDD file listing the UBO, and an
/// engagement in onboarding.
async fn client(app: &TestApp, tag: &str) -> (PartyId, PartyId, Engagement) {
    let company = company(app, &format!("Gulf Trading {tag} LLC")).await;
    let ubo = tenant(app, &format!("Omar Haddad {tag}")).await;
    let file = cdd_files::open(app.db(), None, company).await.unwrap();
    cdd_files::add_party(app.state(), file.id, ubo, ScreenedRole::Ubo)
        .await
        .unwrap();
    let engagement = engagements::create(
        app.state(),
        None,
        NewEngagement::retainer(company, "Bookkeeping and VAT", 500_000),
    )
    .await
    .unwrap();
    (company, ubo, engagement)
}

/// An accepted document of kind `key` about `party`, valid through `expires`.
async fn document(app: &TestApp, party: PartyId, key: &str, expires: NaiveDate) {
    let kind = DocumentKind::by_key(app.db(), key)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the kind {key}"));
    Document::create(
        app.db(),
        Document {
            subject_type: "parties.party".into(),
            subject_id: party.0,
            kind_id: kind.id,
            title: key.into(),
            expiry_date: Some(expires),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap();
}

/// The KYC documents of the client company and its UBO, all valid for a year.
async fn valid_documents(app: &TestApp, company: PartyId, ubo: PartyId) {
    let year = today() + Days::new(365);
    document(app, company, "trade_licence", year).await;
    document(app, ubo, "passport", year).await;
    document(app, ubo, "emirates_id", year).await;
}

/// The three KYC kinds block `engagement.activate` for the parties whose role
/// they name, with the MLRO's override, and flag `return.file`; the MLRO
/// renews them, 30 days of look-ahead, compliance-critical.
#[renox::test]
async fn the_kyc_kinds_block_activation_flag_the_filing_and_belong_to_the_mlro() {
    let app = accounting_booted().await;
    for (key, roles) in [
        ("trade_licence", vec!["client"]),
        ("passport", vec!["ubo"]),
        ("emirates_id", vec!["ubo"]),
    ] {
        let kind = DocumentKind::by_key(app.db(), key)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{key} is seeded"));
        assert_eq!(kind.responsible_role.as_deref(), Some(COMPLIANCE), "{key}");
        assert_eq!(kind.look_ahead_days, 30, "{key}");
        assert!(kind.compliance_critical, "{key}");
        assert!(!kind.legal_block, "{key}: the MLRO can override");
        assert_eq!(kind.override_mode, OverrideMode::WithApproval, "{key}");
        let mut blocks = kind.blocks(app.db()).await.unwrap();
        blocks.sort_by(|a, b| a.guarded.cmp(&b.guarded));
        let seen: Vec<(&str, BlockMode)> =
            blocks.iter().map(|b| (b.guarded.as_str(), b.mode)).collect();
        assert_eq!(
            seen,
            [
                ("engagement.activate", BlockMode::Block),
                ("return.file", BlockMode::Flag)
            ],
            "{key}"
        );
        for block in &blocks {
            let condition = block.condition().expect("a condition on the party role");
            assert_eq!(condition.values, roles, "{key} {}", block.guarded);
        }
    }
}

/// Done when 1 (first and second parts): activating with an expired UBO
/// passport is refused with an MLRO override offered; the client's valid
/// documents let the same engagement through; the licence and the Emirates ID
/// are not the ones named.
#[renox::test]
async fn an_expired_ubo_passport_refuses_activation_with_an_override_offered() {
    let app = accounting_booted().await;
    let (company, ubo, engagement) = client(&app, "A").await;
    let yesterday = today() - Days::new(1);
    let year = today() + Days::new(365);
    document(&app, company, "trade_licence", year).await;
    document(&app, ubo, "emirates_id", year).await;
    document(&app, ubo, "passport", yesterday).await;
    let state = app.state();

    let refused = documents::guard(state)
        .check(state, "engagement.activate", engagement.id, today())
        .await
        .unwrap()
        .unwrap_err();
    let message = refused.message();
    assert!(
        message.contains(&format!("Passport expired on {yesterday}")),
        "{message}"
    );
    assert!(
        message.contains("override: with approval"),
        "the MLRO's override is offered: {message}"
    );
    assert!(!message.contains("Trade licence"), "{message}");
    assert!(!message.contains("Emirates ID"), "{message}");

    // A valid passport lets the same engagement through.
    let (company, ubo, engagement) = client(&app, "B").await;
    valid_documents(&app, company, ubo).await;
    assert!(
        documents::guard(state)
            .check(state, "engagement.activate", engagement.id, today())
            .await
            .unwrap()
            .is_ok()
    );
}

/// Done when 1 (third part): filing with an expired trade licence is not
/// refused; the return is flagged for the MLRO.
#[renox::test]
async fn an_expired_trade_licence_flags_the_filing_and_does_not_stop_it() {
    let app = accounting_booted().await;
    let (company, ubo, engagement) = client(&app, "A").await;
    let year = today() + Days::new(365);
    document(&app, company, "trade_licence", today() - Days::new(1)).await;
    document(&app, ubo, "passport", year).await;
    document(&app, ubo, "emirates_id", year).await;
    let ret = Return::create(
        app.db(),
        Return {
            client_party_id: company.0,
            engagement_id: engagement.id,
            return_type: ReturnType::Vat,
            period_end: today() - Days::new(30),
            due_date: today() + Days::new(5),
            version: 1,
            ..Return::default()
        },
    )
    .await
    .unwrap();
    let state = app.state();

    let outcome = documents::guard(state)
        .check(state, "return.file", ret.id, today())
        .await
        .unwrap()
        .expect("the filing is not refused");
    assert_eq!(outcome.flags.len(), 1, "one flag: the trade licence");
    assert!(
        outcome.flags[0].label.contains("Trade licence"),
        "{:?}",
        outcome.flags[0]
    );
    // Flagged for the MLRO after the filing.
    let flagged = accounting::documents::flag_expired(state, &ret).await.unwrap();
    assert_eq!(flagged, 1);
    assert_eq!(
        accounting::documents::flags_of(state, ret.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// The users told by the expiry reminder of `kind`, with the days left.
async fn told(app: &TestApp) -> Vec<(i64, i64)> {
    let mut messages = Message::query()
        .where_(Message::TYPE_KEY.eq("documents.expiring"))
        .order_by(Message::ID)
        .get(app.db())
        .await
        .unwrap();
    messages.retain(|m| m.user_id.is_some());
    messages
        .iter()
        .map(|m| {
            (
                m.user_id.unwrap(),
                m.data.0["days_left"].as_i64().expect("days left"),
            )
        })
        .collect()
}

/// Done when 1 (fourth part): the expiry reminder of a client trade licence
/// goes to the MLRO, and that of a trademark registration of the firm to the
/// partners; each 30 days before the expiry; no one else is told.
#[renox::test]
async fn the_expiry_reminders_go_to_the_mlro_and_the_partners() {
    let app = accounting_booted().await;
    let mlro = person(&app, "Mariam", COMPLIANCE).await;
    let partner = person(&app, "Pia", PARTNER).await;
    let (company, _ubo, _engagement) = client(&app, "A").await;
    let in_30 = today() + Days::new(30);
    document(&app, company, "trade_licence", in_30).await;

    expiry::run(app.state(), today()).await.unwrap();
    let licence = told(&app).await;
    assert!(licence.contains(&(mlro.id, 30)), "{licence:?}");
    assert!(
        licence.iter().all(|(user, _)| *user != partner.id),
        "the partner is not the licence's responsible role: {licence:?}"
    );

    // The firm's trademark registration expires in 90 days: the partner.
    let firm = entity(&app, "FLA").await;
    let kind = DocumentKind::by_key(app.db(), "trademark_registration")
        .await
        .unwrap()
        .expect("the registration kind");
    Document::create(
        app.db(),
        Document {
            subject_type: "kernel.legal_entity".into(),
            subject_id: firm.id,
            kind_id: kind.id,
            title: "Falcon Ledger trademark".into(),
            expiry_date: Some(today() + Days::new(90)),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    expiry::run(app.state(), today()).await.unwrap();
    let all = told(&app).await;
    assert!(all.contains(&(partner.id, 90)), "{all:?}");
    assert!(
        !all.contains(&(mlro.id, 90)),
        "the MLRO does not renew trademarks: {all:?}"
    );
}

/// The registration kinds: *Trademark registration* and *Other certification*
/// are on, with the partner as responsible role, the reminders of the spec and
/// no `blocks`; the kinds an accounting firm does not use stay off.
#[renox::test]
async fn the_registration_kinds_are_on_with_the_partner_and_block_nothing() {
    let app = accounting_booted().await;
    let trademark = DocumentKind::by_key(app.db(), "trademark_registration")
        .await
        .unwrap()
        .unwrap();
    assert!(trademark.enabled && trademark.blocks(app.db()).await.unwrap().is_empty());
    assert_eq!(trademark.reminder_offsets.0, [180, 90, 30]);
    assert_eq!(trademark.responsible_role.as_deref(), Some(PARTNER));
    assert_eq!(trademark.subject_types.0[0], "kernel.legal_entity");
    let other = DocumentKind::by_key(app.db(), "other_certification")
        .await
        .unwrap()
        .unwrap();
    assert!(other.enabled && other.blocks(app.db()).await.unwrap().is_empty());
    assert_eq!(other.reminder_offsets.0, [90, 30, 7]);
    assert_eq!(other.responsible_role.as_deref(), Some(PARTNER));
    for off in ["halal_certificate", "iso_certificate", "product_registration"] {
        let kind = DocumentKind::by_key(app.db(), off).await.unwrap().unwrap();
        assert!(!kind.enabled, "{off} stays off for 04");
    }
}

/// `FutureWork` (§9): the returns due after the expiry date are listed, and
/// no others; the onboarding engagement is listed for `activate`.
#[renox::test]
async fn the_look_ahead_lists_the_returns_and_engagements_after_the_expiry() {
    let app = accounting_booted().await;
    let (company, ubo, engagement) = client(&app, "A").await;
    let expires = today() + Days::new(20);
    let year = today() + Days::new(365);
    document(&app, company, "trade_licence", expires).await;
    document(&app, ubo, "passport", year).await;
    document(&app, ubo, "emirates_id", year).await;
    let ret = |due: u64| Return {
        client_party_id: company.0,
        engagement_id: engagement.id,
        return_type: ReturnType::Vat,
        period_end: today() - Days::new(30),
        due_date: today() + Days::new(due),
        version: 1,
        ..Return::default()
    };
    let before = Return::create(app.db(), ret(10)).await.unwrap();
    let after = Return::create(app.db(), ret(25)).await.unwrap();
    let state = app.state();

    let entries = wl_documents::blocks_for(app.db(), "return.file")
        .await
        .unwrap();
    let entry = entries
        .iter()
        .find(|e| e.kind == "trade_licence")
        .expect("the trade licence entry");
    let work = documents::future_work("return.file").expect("the provider is registered");
    let planned = work
        .planned_from(state, entry, &company.subject(), expires + Days::new(1))
        .await
        .unwrap();
    let ids: Vec<i64> = planned.iter().map(|p| p.record.subject_id).collect();
    assert_eq!(ids, [after.id], "only the return due after the expiry");
    assert_ne!(before.id, after.id);
    // The UBO's passport is not the one that expires: nothing is listed for
    // the licence against the UBO.
    let none = work
        .planned_from(state, entry, &ubo.subject(), expires + Days::new(1))
        .await
        .unwrap();
    assert!(none.is_empty(), "the licence entry applies to the company");

    let activate = wl_documents::blocks_for(app.db(), "engagement.activate")
        .await
        .unwrap();
    let entry = activate
        .iter()
        .find(|e| e.kind == "trade_licence")
        .expect("the trade licence entry");
    let work = documents::future_work("engagement.activate").expect("the provider is registered");
    let planned = work
        .planned_from(state, entry, &company.subject(), today())
        .await
        .unwrap();
    assert_eq!(planned.len(), 1);
    assert_eq!(planned[0].record, engagement.subject());
}
