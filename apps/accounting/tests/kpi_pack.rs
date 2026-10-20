//! Acceptance checks of #344 (BL-ACC-20, `ACC.RUL.06`): a trial balance
//! imported again for a period that has a published pack makes a new draft
//! version that carries the commentary; the published pack shows as *being
//! revised* in the portal until the new version is published, which
//! supersedes it and tells the client; an import while a draft exists
//! recalculates it; `publish` names the accounts that are not mapped.

mod support;

use accounting::kpi_packs::{IMPORTED_NOTICE, PUBLISHED_NOTICE, commentary_key};
use accounting::{Engagement, EngagementStatus, KpiStatus};
use renox::prelude::*;
use wl_kernel::notify::NotifyRecipient;

use support::{
    OCTOBER, TB_OCTOBER, TB_OCTOBER_LATE, TB_OCTOBER_NEW_ACCOUNT, boot, register_portal_section,
};

/// The portal page of the section of the KPI packs of `company`.
fn section(company: i64) -> String {
    format!("/portal/{company}/kpi_packs")
}

/// Done when 1: October's pack v1 `published`; importing a new trial balance
/// for October creates v2 `draft` with v1's commentary (and the saved
/// mapping), and the portal shows v1 with "being revised". The client is
/// told nothing at the re-import.
#[renox::test]
async fn a_reimport_makes_a_draft_with_the_commentary_and_the_portal_says_being_revised() {
    register_portal_section();
    let t = boot().await;
    let manager = t.manager().await;
    let client = t.client().await;
    let finance = t.portal_user("Omar", client.ubo, client.company).await;
    let v1 = t.kpi_published(&manager, client.company, TB_OCTOBER).await;
    assert_eq!(v1.status, KpiStatus::Published);
    assert_eq!(v1.version, 1);
    let told_before = t.notices.of_type(PUBLISHED_NOTICE).len();

    t.kpi_import(&manager, client.company, OCTOBER, TB_OCTOBER_LATE)
        .await
        .assert_ok();

    let packs = t.kpi_packs(client.company).await;
    assert_eq!(packs.len(), 2);
    let v2 = &packs[1];
    assert_eq!((v2.version, v2.status), (2, KpiStatus::Draft));
    assert_eq!(v2.commentary.as_deref(), Some("Sales up on last month."));
    let figures = v2.figures(t.db()).await.unwrap();
    assert!(
        figures.iter().all(|f| f.category.is_some()),
        "the client's saved mapping is applied"
    );
    assert_eq!(
        figures
            .iter()
            .find(|f| f.account_code == "1000")
            .unwrap()
            .amount,
        700_000
    );
    // v1 is untouched.
    let v1_now = t.kpi_now(&v1).await;
    assert_eq!(v1_now.status, KpiStatus::Published);
    assert_eq!(
        v1_now
            .figures(t.db())
            .await
            .unwrap()
            .iter()
            .find(|f| f.account_code == "1000")
            .unwrap()
            .amount,
        500_000,
        "a published pack's figures never change"
    );
    // The client sees v1, being revised, and is not told.
    t.get(&finance, &section(client.company.0))
        .await
        .assert_ok()
        .assert_see(&v1.number())
        .assert_see("Being revised");
    t.get(&finance, &format!("/accounting/portal/kpi-packs/{}", v1.id))
        .await
        .assert_ok()
        .assert_see("This report is being revised");
    assert_eq!(t.notices.of_type(PUBLISHED_NOTICE).len(), told_before);
    // The draft is not in the portal.
    t.get(&finance, &format!("/accounting/portal/kpi-packs/{}", v2.id))
        .await
        .assert_not_found();
}

/// Done when 2: the manager publishes v2: v1 is `superseded`, the portal
/// shows v2 only, and the client is notified; staff see both versions, the
/// old one marked superseded.
#[renox::test]
async fn publishing_the_new_version_supersedes_the_old_and_tells_the_client() {
    register_portal_section();
    let t = boot().await;
    let manager = t.manager().await;
    let client = t.client().await;
    let finance = t.portal_user("Omar", client.ubo, client.company).await;
    let v1 = t.kpi_published(&manager, client.company, TB_OCTOBER).await;
    t.kpi_import(&manager, client.company, OCTOBER, TB_OCTOBER_LATE)
        .await
        .assert_ok();
    let v2 = t.kpi_version(client.company, 2).await;

    t.kpi_act(&manager, &v2, "publish", &[]).await.assert_ok();

    assert_eq!(t.kpi_now(&v1).await.status, KpiStatus::Superseded);
    assert_eq!(t.kpi_now(&v2).await.status, KpiStatus::Published);
    // The portal: v2 only.
    let page = t.get(&finance, &section(client.company.0)).await;
    page.assert_ok().assert_see(&v2.number());
    let body = page.text();
    assert!(!body.contains(&v1.number()), "{body}");
    assert!(!body.contains("Being revised"), "{body}");
    t.get(&finance, &format!("/accounting/portal/kpi-packs/{}", v1.id))
        .await
        .assert_not_found();
    t.get(&finance, &format!("/accounting/portal/kpi-packs/{}", v2.id))
        .await
        .assert_ok();
    // The client was told of v1 and of v2, at the publish.
    let told = t.notices.of_type(PUBLISHED_NOTICE);
    assert_eq!(told.len(), 2);
    assert_eq!(
        told[1].recipients,
        [NotifyRecipient::Party { id: client.ubo }]
    );
    // Staff see both versions, the old one marked superseded.
    t.get(&manager, &format!("/accounting/kpi-packs/{}", v2.id))
        .await
        .assert_ok()
        .assert_see("Superseded")
        .assert_see(&v1.number());
}

/// Done when 3: importing again while v2 is `draft` recalculates v2; no v3
/// exists.
#[renox::test]
async fn an_import_while_a_draft_exists_recalculates_it_in_place() {
    let t = boot().await;
    let manager = t.manager().await;
    let client = t.client().await;
    t.kpi_published(&manager, client.company, TB_OCTOBER).await;
    t.kpi_import(&manager, client.company, OCTOBER, TB_OCTOBER_LATE)
        .await
        .assert_ok();
    let v2 = t.kpi_version(client.company, 2).await;
    t.kpi_act(
        &manager,
        &v2,
        "commentary",
        &[("commentary", "Late entries in.")],
    )
    .await
    .assert_ok();

    let sales = |csv: &'static str| csv.replace("12000.00", "13000.00");
    t.kpi_import(&manager, client.company, OCTOBER, &sales(TB_OCTOBER_LATE))
        .await
        .assert_ok();

    let packs = t.kpi_packs(client.company).await;
    assert_eq!(packs.len(), 2, "no third version");
    let v2_now = t.kpi_now(&v2).await;
    assert_eq!(v2_now.status, KpiStatus::Draft);
    assert_eq!(v2_now.commentary.as_deref(), Some("Late entries in."));
    let figures = v2_now.figures(t.db()).await.unwrap();
    assert_eq!(figures.len(), 4, "the accounts are replaced, not added");
    assert_eq!(
        figures
            .iter()
            .find(|f| f.account_code == "4000")
            .unwrap()
            .amount,
        -1_300_000
    );
    // Another period, or another client, has its own packs.
    t.kpi_import(&manager, client.company, "2026-11", TB_OCTOBER)
        .await
        .assert_ok();
    assert_eq!(t.kpi_packs(client.company).await.len(), 2);
}

/// Done when 4: `publish` with an account that is not mapped is refused and
/// names it; mapping it lets the publish through.
#[renox::test]
async fn publish_is_refused_naming_the_unmapped_accounts() {
    let t = boot().await;
    let manager = t.manager().await;
    let client = t.client().await;
    t.kpi_published(&manager, client.company, TB_OCTOBER).await;
    t.kpi_import(&manager, client.company, OCTOBER, TB_OCTOBER_NEW_ACCOUNT)
        .await
        .assert_ok();
    let v2 = t.kpi_version(client.company, 2).await;

    let refused = t.kpi_act(&manager, &v2, "publish", &[]).await;
    refused.assert_status(422);
    let body = refused.text();
    assert!(body.contains("7000"), "{body}");
    assert!(!body.contains("1000"), "only the unmapped account: {body}");
    assert_eq!(t.kpi_now(&v2).await.status, KpiStatus::Draft);

    t.kpi_act(
        &manager,
        &v2,
        "map",
        &[("code", "7000"), ("category", "other_expenses")],
    )
    .await
    .assert_ok();
    t.kpi_act(&manager, &v2, "publish", &[]).await.assert_ok();
    assert_eq!(t.kpi_now(&v2).await.status, KpiStatus::Published);
}

/// Commentary is required when the template says so; a draft only is
/// edited; `publish` needs `kpi.publish`.
#[renox::test]
async fn publish_needs_the_commentary_and_the_permission() {
    let t = boot().await;
    let manager = t.manager().await;
    let accountant = t.accountant().await;
    let client = t.client().await;
    t.kpi_import(&accountant, client.company, OCTOBER, TB_OCTOBER)
        .await
        .assert_ok();
    let v1 = t.kpi_version(client.company, 1).await;
    t.kpi_map_all(&accountant, &v1).await;

    let refused = t.kpi_act(&manager, &v1, "publish", &[]).await;
    refused.assert_status(422);
    assert!(refused.text().contains("commentary"), "{}", refused.text());
    t.kpi_act(&accountant, &v1, "commentary", &[("commentary", "Done.")])
        .await
        .assert_ok();
    t.kpi_act(&accountant, &v1, "publish", &[])
        .await
        .assert_forbidden();
    t.kpi_act(&manager, &v1, "publish", &[]).await.assert_ok();
    // A published pack takes no commentary.
    t.kpi_act(&manager, &v1, "commentary", &[("commentary", "Again.")])
        .await
        .assert_status(422);
    // A user without the permission cannot import.
    let outsider = t.user_with_grants("Olga", &[]).await;
    t.kpi_import(&outsider, client.company, OCTOBER, TB_OCTOBER)
        .await
        .assert_forbidden();
}

/// `ACC.RUL.06`: an import tells the engagement manager to add the
/// commentary; without a named manager the manager role gets a task.
#[renox::test]
async fn an_import_tells_the_manager_to_add_the_commentary() {
    let t = boot().await;
    let manager = t.manager().await;
    let accountant = t.accountant().await;
    let client = t.client().await;

    // No engagement manager is named: a task for the manager role.
    t.kpi_import(&accountant, client.company, OCTOBER, TB_OCTOBER)
        .await
        .assert_ok();
    let v1 = t.kpi_version(client.company, 1).await;
    let tasks = t.tickets.with_key(&commentary_key(v1.id));
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].title.contains(&v1.number()), "{}", tasks[0].title);

    // With a named manager: a notice to them, for the recalculation too.
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    Engagement::query()
        .where_(Engagement::ID.eq(engagement.id))
        .update(t.db(), &[("manager_user_id", &Some(manager.id))])
        .await
        .unwrap();
    t.kpi_import(&accountant, client.company, OCTOBER, TB_OCTOBER_LATE)
        .await
        .assert_ok();
    let told = t.notices.of_type(IMPORTED_NOTICE);
    assert_eq!(told.len(), 1);
    assert_eq!(
        told[0].recipients,
        [NotifyRecipient::User { id: manager.id }]
    );
    assert_eq!(told[0].data["version"], 1, "the draft was recalculated");
}

/// A trial balance that cannot be read is refused naming its rows, and
/// nothing is stored.
#[renox::test]
async fn an_unreadable_trial_balance_is_refused_with_its_rows() {
    let t = boot().await;
    let manager = t.manager().await;
    let client = t.client().await;
    let bad = "code,name,debit,credit\n1000,Bank,abc,\n1000,Bank again,1,\n";
    let refused = t.kpi_import(&manager, client.company, OCTOBER, bad).await;
    refused.assert_status(422);
    assert!(refused.text().contains("Row 2"), "{}", refused.text());
    assert!(t.kpi_packs(client.company).await.is_empty());
}

/// Bug fix: the older version is superseded in the publish transaction, not
/// by the listener afterwards: when the listener fails (the notifier is
/// down) two `published` versions never coexist, and the portal serves only
/// the current version.
#[renox::test]
async fn the_older_version_is_superseded_in_the_publish_transaction() {
    register_portal_section();
    let t = boot().await;
    let manager = t.manager().await;
    let client = t.client().await;
    let finance = t.portal_user("Omar", client.ubo, client.company).await;
    let v1 = t.kpi_published(&manager, client.company, TB_OCTOBER).await;
    t.kpi_import(&manager, client.company, OCTOBER, TB_OCTOBER_LATE)
        .await
        .assert_ok();
    let v2 = t.kpi_version(client.company, 2).await;
    t.notices.fail(PUBLISHED_NOTICE);

    // The listener fails after the commit; the publish itself stood.
    let _ = t.kpi_act(&manager, &v2, "publish", &[]).await;

    assert_eq!(t.kpi_now(&v2).await.status, KpiStatus::Published);
    assert_eq!(t.kpi_now(&v1).await.status, KpiStatus::Superseded);
    let published = t
        .kpi_packs(client.company)
        .await
        .into_iter()
        .filter(|p| p.status == KpiStatus::Published)
        .count();
    assert_eq!(published, 1);
    t.get(&finance, &format!("/accounting/portal/kpi-packs/{}", v1.id))
        .await
        .assert_not_found();
    t.get(&finance, &format!("/accounting/portal/kpi-packs/{}", v2.id))
        .await
        .assert_ok();
}

/// Bug fix (`ACC.ACL.14`): the publish notice is an automatic message and
/// is not sent while the client's engagement reason is restricted; the pack
/// is published all the same.
#[renox::test]
async fn no_publish_notice_goes_out_while_the_engagement_reason_is_restricted() {
    let t = boot().await;
    let manager = t.manager().await;
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Suspended, Some("on_hold"))
        .await;
    t.suppress_messages(&engagement).await;

    let pack = t.kpi_published(&manager, client.company, TB_OCTOBER).await;

    assert_eq!(pack.status, KpiStatus::Published);
    assert!(t.notices.of_type(PUBLISHED_NOTICE).is_empty());
}
