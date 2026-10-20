//! Class A gaps of the batch 3 audit (#338, #343, #347), through the real host
//! of the 04 Accounting client and its real routes: the approval of a tax
//! profile change through `/approvals/{id}/approve`, return filing by the
//! client or a tax agent with the expired-licence flag and the MLRO's
//! approval for a blocked client, and the block that pauses the retainer and
//! holds the tasks.

mod support;

use accounting::tax_profile_change::TaxProfileChange;
use accounting::{
    Engagement, EngagementStatus, Return, ReturnStatus, ReturnType, TaxProfile, VatPeriod,
};
use host::roles::{ACCOUNTANT, COMPLIANCE, ENGAGEMENT_MANAGER, PARTNER, TAX_SPECIALIST};
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{accounting_booted, block, company, person};
use wl_kernel::PartyId;
use wl_kernel::approvals;
use wl_kernel::deadlines::{NewDeadlineTask, deadline_tasks};
use wl_kernel::documents::today;
use wl_kernel::subscriptions::SubscriptionStatus;

fn date(text: &str) -> NaiveDate {
    text.parse().expect("a date")
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

/// The VAT profile of `company` with the quarters ending in January.
async fn january_profile(app: &TestApp, company: PartyId) -> TaxProfile {
    accounting::tax_profile::record(
        app.db(),
        None,
        TaxProfile {
            client_party_id: company.0,
            trn: Some("100000000000003".to_owned()),
            vat_registered: true,
            vat_period: VatPeriod::Quarterly,
            vat_first_period_end: Some(date("2026-01-31")),
            financial_year_end: Some(date("2026-12-31")),
            ct_registration_no: Some("CT-100200300".to_owned()),
            effective_date: date("2026-01-01"),
            ..TaxProfile::default()
        },
    )
    .await
    .unwrap()
}

/// The accountant (or `by`) asks for the quarters ending in February from
/// 2027-02-01.
async fn ask_for_february(app: &TestApp, by: &User, company: PartyId) {
    app.acting_as(by);
    app.htmx()
        .post(
            &format!("/accounting/clients/{}/tax-profile/change", company.0),
            &[
                ("trn", "100000000000003"),
                ("vat_registered", "1"),
                ("vat_period", "quarterly"),
                ("vat_first_period_end", "2026-02-28"),
                ("financial_year_end", "2026-12-31"),
                ("ct_registration_no", "CT-100200300"),
                ("effective_date", "2027-02-01"),
                ("note", "The FTA moved the stagger"),
            ],
        )
        .await
        .assert_ok();
}

/// `ACC.ACL.13` in the host: an accountant's change waits; the accountant
/// holds `tax_profiles.manage` only and cannot approve; a manager who asked
/// cannot approve their own change; a partner approves through the kernel's
/// route and the new version is in force from its date.
#[renox::test]
async fn a_tax_profile_change_is_approved_through_the_approvals_route() {
    let app = accounting_booted().await;
    let accountant = person(&app, "Asha", ACCOUNTANT).await;
    let manager = person(&app, "Mona", ENGAGEMENT_MANAGER).await;
    let partner = person(&app, "Pia", PARTNER).await;
    let client = company(&app, "Gulf Trading A LLC").await;
    january_profile(&app, client).await;

    ask_for_february(&app, &accountant, client).await;
    let change = TaxProfileChange::pending(app.db(), client)
        .await
        .unwrap()
        .expect("the change waits");
    let request = approvals::pending_for(app.db(), &change.subject())
        .await
        .unwrap()
        .remove(0);
    let approve = format!("/approvals/{}/approve", request.id);

    // Not the accountant, who may only ask.
    app.acting_as(&accountant);
    app.htmx()
        .post(&approve, &[("comment", "ok")])
        .await
        .assert_forbidden();
    assert_eq!(
        TaxProfile::versions(app.db(), client).await.unwrap().len(),
        1
    );
    // The partner decides it.
    app.acting_as(&partner);
    app.htmx()
        .post(&approve, &[("comment", "Per the FTA notice")])
        .await
        .assert_ok();
    let versions = TaxProfile::versions(app.db(), client).await.unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[1].approved_by, Some(partner.id));
    assert_eq!(versions[1].effective_date, date("2027-02-01"));
    assert_eq!(
        TaxProfile::current(app.db(), client, date("2027-03-01"))
            .await
            .unwrap()
            .unwrap()
            .vat_first_period_end,
        Some(date("2026-02-28"))
    );

    // A manager who asked for a change cannot approve it; another can.
    app.acting_as(&manager);
    app.htmx()
        .post(
            &format!("/accounting/clients/{}/tax-profile/change", client.0),
            &[
                ("trn", "100000000000003"),
                ("vat_registered", "1"),
                ("vat_period", "quarterly"),
                ("vat_first_period_end", "2026-02-28"),
                ("financial_year_end", "2026-12-31"),
                ("ct_registration_no", "CT-555"),
                ("effective_date", "2027-06-01"),
            ],
        )
        .await
        .assert_ok();
    let second = TaxProfileChange::pending(app.db(), client)
        .await
        .unwrap()
        .expect("the second change waits");
    let request = approvals::pending_for(app.db(), &second.subject())
        .await
        .unwrap()
        .remove(0);
    app.acting_as(&manager);
    app.htmx()
        .post(&format!("/approvals/{}/approve", request.id), &[])
        .await
        .assert_forbidden();
    app.acting_as(&partner);
    app.htmx()
        .post(&format!("/approvals/{}/approve", request.id), &[])
        .await
        .assert_ok();
    assert_eq!(
        TaxProfile::versions(app.db(), client).await.unwrap().len(),
        3
    );
}

/// A VAT return of `engagement` inserted `approved`, for a period ending
/// `period_end`.
async fn approved_return(app: &TestApp, engagement: &Engagement, period_end: &str) -> Return {
    let mut ret = Return {
        client_party_id: engagement.client_party_id,
        engagement_id: engagement.id,
        return_type: ReturnType::Vat,
        period_end: date(period_end),
        due_date: today() + Days::new(10),
        version: 1,
        figures: Some("Output VAT 12,000; input VAT 4,000".to_owned()),
        workpapers_attached: true,
        status: ReturnStatus::Approved,
        ..Return::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    ret.insert_in_state(&mut tx, ReturnStatus::Approved, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    Return::find_or_404(app.db(), ret.id).await.unwrap()
}

/// `by` files `ret` through the route as the client (its EmaraTax user), or
/// as a tax agent.
async fn file(
    app: &TestApp,
    by: &User,
    ret: &Return,
    by_client: bool,
) -> renox::testing::TestResponse {
    let on = today().to_string();
    let mut pairs = vec![
        ("acknowledgement", "FTA-ACK-4100"),
        ("filed_on", on.as_str()),
    ];
    if by_client {
        pairs.push(("by_client", "1"));
        pairs.push(("client_user", "falcon.finance"));
    }
    app.acting_as(by);
    app.htmx()
        .post(&format!("/accounting/returns/{}/file", ret.id), &pairs)
        .await
}

/// A KYC document `key` of `party`, expiring `expires`.
async fn kyc_document(app: &TestApp, party: PartyId, key: &str, expires: NaiveDate) {
    let kind = wl_documents::DocumentKind::by_key(app.db(), key)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the kind {key}"));
    wl_documents::Document::create(
        app.db(),
        wl_documents::Document {
            subject_type: "parties.party".into(),
            subject_id: party.0,
            kind_id: kind.id,
            title: key.into(),
            expiry_date: Some(expires),
            current_version: 1,
            status: wl_documents::DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap();
}

/// `ACC.ACL.15` in the host (no tax agent registration exists in wl-hr yet):
/// a tax agent is refused (nothing to override); the client's own filing with its
/// EmaraTax user is recorded; an expired trade licence flags the return for
/// the MLRO and does not stop it.
#[renox::test]
async fn a_return_is_filed_by_the_client_and_an_expired_licence_only_flags_it() {
    let app = accounting_booted().await;
    let filer = person(&app, "Tariq", TAX_SPECIALIST).await;
    let mlro = person(&app, "Mariam", COMPLIANCE).await;
    let client = company(&app, "Gulf Trading A LLC").await;
    let engagement = active_engagement(&app, client).await;
    let first = approved_return(&app, &engagement, "2026-06-30").await;

    // As a tax agent: no valid registration, and no override is offered.
    let refused = file(&app, &filer, &first, false).await;
    refused.assert_status(422);
    refused.assert_see("cannot file as a registered tax agent");
    assert_eq!(
        Return::find_or_404(app.db(), first.id)
            .await
            .unwrap()
            .status,
        ReturnStatus::Approved
    );

    // As the client, with an expired trade licence: filed, and flagged.
    kyc_document(&app, client, "trade_licence", today() - Days::new(1)).await;
    file(&app, &filer, &first, true).await.assert_ok();
    let filed = Return::find_or_404(app.db(), first.id).await.unwrap();
    assert_eq!(filed.status, ReturnStatus::Filed);
    assert!(filed.filed_by_client);
    let flags = accounting::documents::flags_of(app.state(), first.id)
        .await
        .unwrap();
    assert_eq!(flags.len(), 1, "{flags:?}");
    app.acting_as(&mlro);
    app.get(&format!("/accounting/returns/{}", first.id))
        .await
        .assert_ok()
        .assert_see("KYC document expired");
    app.acting_as(&filer);
    app.get(&format!("/accounting/returns/{}", first.id))
        .await
        .assert_ok()
        .assert_dont_see("KYC document expired");
}

/// For a blocked client the filing waits for the MLRO: the filer reads only
/// *On hold — MLRO approval needed* (no code, note or label), cannot approve,
/// and the MLRO approves through the kernel's route, for that return only.
#[renox::test]
async fn a_blocked_clients_return_waits_for_the_mlro_through_the_approvals_route() {
    let app = accounting_booted().await;
    let filer = person(&app, "Tariq", TAX_SPECIALIST).await;
    let mlro = person(&app, "Mariam", COMPLIANCE).await;
    let client = company(&app, "Gulf Trading B LLC").await;
    let engagement = active_engagement(&app, client).await;
    let first = approved_return(&app, &engagement, "2026-06-30").await;
    let second = approved_return(&app, &engagement, "2026-09-30").await;
    block(&app, client).await;

    let response = file(&app, &filer, &first, true).await;
    response.assert_ok();
    for leak in ["non_payment", "Non-payment", "3 bounced cheques"] {
        assert!(!response.text().contains(leak), "{leak}");
    }
    app.acting_as(&filer);
    let page = app.get(&format!("/accounting/returns/{}", first.id)).await;
    page.assert_ok()
        .assert_see("On hold — MLRO approval needed");
    for leak in ["non_payment", "Non-payment", "3 bounced cheques"] {
        assert!(!page.text().contains(leak), "{leak}");
    }
    let request = approvals::pending_for(app.db(), &first.subject())
        .await
        .unwrap()
        .remove(0);
    let approve = format!("/approvals/{}/approve", request.id);
    app.acting_as(&filer);
    app.htmx().post(&approve, &[]).await.assert_forbidden();
    assert_eq!(
        Return::find_or_404(app.db(), first.id)
            .await
            .unwrap()
            .status,
        ReturnStatus::Approved
    );

    app.acting_as(&mlro);
    app.htmx()
        .post(&approve, &[("comment", "Cleared")])
        .await
        .assert_ok();
    assert_eq!(
        Return::find_or_404(app.db(), first.id)
            .await
            .unwrap()
            .status,
        ReturnStatus::Filed
    );
    // The approval does not carry to the other return.
    file(&app, &filer, &second, true).await.assert_ok();
    assert_eq!(
        Return::find_or_404(app.db(), second.id)
            .await
            .unwrap()
            .status,
        ReturnStatus::Approved
    );
    assert_eq!(
        approvals::pending_for(app.db(), &second.subject())
            .await
            .unwrap()
            .len(),
        1
    );
}

/// §7.1 *Block effect* through the real effects: blocking the client pauses
/// the engagement and its retainer (wl-invoicing's subscription) with the
/// neutral reason, and holds the engagement's deadline tasks.
#[renox::test]
async fn a_block_pauses_the_engagement_the_retainer_and_holds_the_tasks() {
    let app = accounting_booted().await;
    let client = company(&app, "Gulf Trading C LLC").await;
    let engagement = active_engagement(&app, client).await;
    let subscription = accounting::retainer::start(app.state(), &engagement, today())
        .await
        .unwrap()
        .expect("the retainer starts");
    let task = deadline_tasks(app.state())
        .create(
            app.state(),
            NewDeadlineTask::new(
                engagement.subject(),
                "vat_return",
                "VAT return",
                today() + Days::new(30),
            )
            .party(client),
        )
        .await
        .unwrap()
        .expect("ticketing is on");
    let status_of = || async {
        wl_invoicing::Subscription::find_or_404(app.db(), subscription.0)
            .await
            .unwrap()
            .status
    };
    assert_eq!(status_of().await, SubscriptionStatus::Active);

    block(&app, client).await;

    assert_eq!(status_of().await, SubscriptionStatus::Paused);
    let suspended = Engagement::find_or_404(app.db(), engagement.id)
        .await
        .unwrap();
    assert_eq!(suspended.status, EngagementStatus::Suspended);
    assert_eq!(suspended.reason_code.as_deref(), Some("on_hold"));
    let held = deadline_tasks(app.state())
        .find(app.db(), task)
        .await
        .unwrap()
        .expect("the task");
    assert!(held.held, "the engagement's tasks are held");
}

/// A verified client user acting for `company` as an owner, with the person
/// party `name`.
async fn client_user(app: &TestApp, name: &str, company: PartyId) -> (User, PartyId) {
    let party = support::tenant(app, name).await;
    let user = person(app, name, host::roles::MEMBER).await;
    wl_portal::PortalAccess::register(
        app.state(),
        user.id,
        wl_portal::NewGrant::new(party, company, "client", &["owner"]),
    )
    .await
    .unwrap();
    (user, party)
}

const TB_OCTOBER: &str = "code,name,debit,credit\n1000,Bank,5000.00,\n4000,Sales,,10000.00\n5000,Cost of sales,4000.00,\n6000,Rent,1000.00\n";
const TB_LATE: &str = "code,name,debit,credit\n1000,Bank,7000.00,\n4000,Sales,,12000.00\n5000,Cost of sales,4000.00,\n6000,Rent,1000.00\n";

/// `by` imports `csv` for October 2026 of `company`.
async fn import(app: &TestApp, by: &User, company: PartyId, csv: &str) {
    app.acting_as(by);
    app.htmx()
        .post(
            "/accounting/kpi-packs/import",
            &[
                ("client", &company.0.to_string()),
                ("period", "2026-10"),
                ("csv", csv),
            ],
        )
        .await
        .assert_ok();
}

/// The pack of `company` in `version`.
async fn pack(app: &TestApp, company: PartyId, version: i64) -> accounting::KpiPack {
    accounting::KpiPack::query()
        .where_(accounting::KpiPack::CLIENT_PARTY_ID.eq(company.0))
        .where_(accounting::KpiPack::VERSION.eq(version))
        .first(app.db())
        .await
        .unwrap()
        .expect("that version exists")
}

/// `by` maps, comments and publishes `pack`.
async fn publish(app: &TestApp, by: &User, pack: &accounting::KpiPack, map: bool) {
    app.acting_as(by);
    if map {
        for (code, category) in [
            ("1000", "assets"),
            ("4000", "revenue"),
            ("5000", "cost_of_sales"),
            ("6000", "operating_expenses"),
        ] {
            app.htmx()
                .post(
                    &format!("/accounting/kpi-packs/{}/map", pack.id),
                    &[("code", code), ("category", category)],
                )
                .await
                .assert_ok();
        }
        app.htmx()
            .post(
                &format!("/accounting/kpi-packs/{}/commentary", pack.id),
                &[("commentary", "Sales up on last month.")],
            )
            .await
            .assert_ok();
    }
    app.htmx()
        .post(&format!("/accounting/kpi-packs/{}/publish", pack.id), &[])
        .await
        .assert_ok();
}

/// #344 in the host: the client reads the published pack in the portal, a
/// re-import shows it as being revised while the draft stays hidden, the new
/// version supersedes the old (one published version only) and each publish
/// reaches the client through the real wl-notify; another company's user gets
/// 404 on the pack.
#[renox::test]
async fn the_portal_shows_the_latest_published_pack_and_says_when_it_is_being_revised() {
    let app = accounting_booted().await;
    let manager = person(&app, "Mona", ENGAGEMENT_MANAGER).await;
    let client = company(&app, "Gulf Trading A LLC").await;
    let other = company(&app, "Gulf Trading B LLC").await;
    let (omar, omar_party) = client_user(&app, "Omar Haddad", client).await;
    let (stranger, _) = client_user(&app, "Sami Rahal", other).await;
    let section = format!("/portal/{}/kpi_packs", client.0);

    import(&app, &manager, client, TB_OCTOBER).await;
    let v1 = pack(&app, client, 1).await;
    publish(&app, &manager, &v1, true).await;
    app.run_all_jobs().await;
    assert_eq!(
        support::messages_to_party(&app, "accounting.kpi_published", omar_party)
            .await
            .len(),
        1,
        "the publish reaches the client through wl-notify"
    );
    app.acting_as(&omar);
    app.get(&section).await.assert_ok().assert_see(&v1.number());
    app.get(&format!("/accounting/portal/kpi-packs/{}", v1.id))
        .await
        .assert_ok();
    app.acting_as(&stranger);
    app.get(&format!("/accounting/portal/kpi-packs/{}", v1.id))
        .await
        .assert_not_found();

    // The re-import: v1 stays, being revised; the draft is hidden.
    import(&app, &manager, client, TB_LATE).await;
    let v2 = pack(&app, client, 2).await;
    app.acting_as(&omar);
    app.get(&section)
        .await
        .assert_ok()
        .assert_see(&v1.number())
        .assert_see("Being revised");
    app.get(&format!("/accounting/portal/kpi-packs/{}", v2.id))
        .await
        .assert_not_found();

    // Publishing v2: only v2 is served.
    publish(&app, &manager, &v2, false).await;
    app.run_all_jobs().await;
    let published = accounting::KpiPack::query()
        .where_(accounting::KpiPack::CLIENT_PARTY_ID.eq(client.0))
        .where_(accounting::KpiPack::STATUS.eq(accounting::KpiStatus::Published))
        .count(app.db())
        .await
        .unwrap();
    assert_eq!(published, 1);
    app.acting_as(&omar);
    let page = app.get(&section).await;
    page.assert_ok().assert_see(&v2.number());
    assert!(!page.text().contains(&v1.number()));
    assert!(!page.text().contains("Being revised"));
    app.get(&format!("/accounting/portal/kpi-packs/{}", v1.id))
        .await
        .assert_not_found();
    app.get(&format!("/accounting/portal/kpi-packs/{}", v2.id))
        .await
        .assert_ok();
    assert_eq!(
        support::messages_to_party(&app, "accounting.kpi_published", omar_party)
            .await
            .len(),
        2
    );
}
