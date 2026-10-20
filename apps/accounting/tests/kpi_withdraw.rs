//! Acceptance checks of #345 (BL-ACC-21): a published KPI pack is withdrawn
//! from the portal at once with a reason (*wrong client*, *wrong figures*);
//! the client users are told, without naming another company; a wrong
//! client opens a privacy incident task for the company admin listing who
//! of the receiving company opened the pack; *copy* makes a new draft for
//! the right client from the withdrawn pack.

mod support;

use accounting::kpi_packs::{WITHDRAWN_NOTICE, incident_key};
use accounting::{KpiMapping, KpiStatus};
use renox::auth::permissions;
use wl_kernel::COMPANY_ADMIN_ROLE;
use wl_kernel::notify::{self, NotifyChannel, NotifyRecipient};
use wl_kernel::tickets::Assignee;

use support::{OCTOBER, TB_OCTOBER, boot, register_portal_section};

/// The text the client reads for the notice `told`.
fn read(told: &wl_kernel::notify::NotifyRequest) -> String {
    let def = notify::notification_type(WITHDRAWN_NOTICE).expect("registered");
    let body = def
        .template_for(NotifyChannel::Email, "en")
        .expect("an English template")
        .body
        .clone();
    notify::fill(&body, &told.data)
}

/// Done when 1: October's pack for client A published to client B by
/// mistake; `withdraw` with *wrong client* removes it from B's portal, tells
/// B's users without naming A, and opens a privacy incident task for the
/// company admin listing who in B opened it.
#[renox::test]
async fn a_wrong_client_withdrawal_removes_the_pack_tells_the_client_and_opens_an_incident() {
    register_portal_section();
    let t = boot().await;
    permissions::define_role(t.db(), COMPANY_ADMIN_ROLE, &[])
        .await
        .unwrap();
    let admin = t.user_with("Ada", COMPANY_ADMIN_ROLE).await;
    let manager = t.manager().await;
    let a = t.client_named(" Alpha").await;
    let b = t.client_named(" Beta").await;
    let reader = t.portal_user("Bea", b.ubo, b.company).await;
    // The pack of A's figures went to B.
    let pack = t.kpi_published(&manager, b.company, TB_OCTOBER).await;
    let page = format!("/accounting/portal/kpi-packs/{}", pack.id);
    t.get(&reader, &page).await.assert_ok();
    t.get(&reader, &format!("/portal/{}/kpi_packs", b.company.0))
        .await
        .assert_ok()
        .assert_see(&pack.number());

    t.kpi_act(&manager, &pack, "withdraw", &[("reason", "wrong_client")])
        .await
        .assert_ok();

    let withdrawn = t.kpi_now(&pack).await;
    assert_eq!(withdrawn.status, KpiStatus::Withdrawn);
    assert_eq!(withdrawn.withdraw_reason.as_deref(), Some("wrong_client"));
    assert_eq!(withdrawn.withdrawn_by, Some(manager.id));
    // Gone from B's portal at once.
    t.get(&reader, &page).await.assert_not_found();
    let section = t
        .get(&reader, &format!("/portal/{}/kpi_packs", b.company.0))
        .await;
    section.assert_ok();
    assert!(
        !section.text().contains(&pack.number()),
        "{}",
        section.text()
    );
    // B's users are told, without naming A (or the period).
    let told = t.notices.of_type(WITHDRAWN_NOTICE);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].recipients, [NotifyRecipient::Party { id: b.ubo }]);
    let message = read(&told[0]);
    assert!(
        message.contains("withdrawn and must be disregarded"),
        "{message}"
    );
    for name in ["Alpha", "Beta", "2026-10", &pack.number()] {
        assert!(!message.contains(name), "{name} in {message}");
    }
    // The company admin has the incident task: the pack, the receiving
    // company and who of it opened the pack.
    let tasks = t.tickets.with_key(&incident_key(pack.id));
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee, Some(Assignee::user(admin.id)));
    assert!(
        tasks[0].title.contains(&pack.number()),
        "{}",
        tasks[0].title
    );
    let description = &tasks[0].description;
    assert!(
        description.contains("Gulf Trading Beta LLC"),
        "{description}"
    );
    assert!(description.contains("Bea"), "{description}");
    assert!(!description.contains("Alpha"), "{description}");
    let _ = a;
}

/// Done when 2: *wrong figures* opens no incident task, and tells the client
/// the period.
#[renox::test]
async fn a_wrong_figures_withdrawal_opens_no_incident() {
    let t = boot().await;
    let manager = t.manager().await;
    let client = t.client().await;
    let pack = t.kpi_published(&manager, client.company, TB_OCTOBER).await;

    t.kpi_act(&manager, &pack, "withdraw", &[("reason", "wrong_figures")])
        .await
        .assert_ok();

    assert_eq!(t.kpi_now(&pack).await.status, KpiStatus::Withdrawn);
    assert!(t.tickets.with_key(&incident_key(pack.id)).is_empty());
    let told = t.notices.of_type(WITHDRAWN_NOTICE);
    assert_eq!(told.len(), 1);
    let message = read(&told[0]);
    assert!(message.contains("2026-10-31"), "{message}");
    assert!(message.contains("wrong"), "{message}");
}

/// Done when 3: `withdraw` without a reason (or with another) is refused;
/// only `kpi.publish` withdraws; a superseded or draft pack cannot be
/// withdrawn.
#[renox::test]
async fn a_withdrawal_needs_a_reason_the_permission_and_a_published_pack() {
    let t = boot().await;
    let manager = t.manager().await;
    let accountant = t.accountant().await;
    let client = t.client().await;
    let v1 = t.kpi_published(&manager, client.company, TB_OCTOBER).await;

    t.kpi_act(&manager, &v1, "withdraw", &[])
        .await
        .assert_status(422);
    t.kpi_act(&manager, &v1, "withdraw", &[("reason", "changed my mind")])
        .await
        .assert_status(422);
    t.kpi_act(&accountant, &v1, "withdraw", &[("reason", "wrong_client")])
        .await
        .assert_forbidden();
    assert_eq!(t.kpi_now(&v1).await.status, KpiStatus::Published);
    assert!(t.notices.of_type(WITHDRAWN_NOTICE).is_empty());

    // v2 supersedes v1; neither v1 (superseded) nor a draft can be withdrawn.
    t.kpi_import(&manager, client.company, OCTOBER, TB_OCTOBER)
        .await
        .assert_ok();
    let v2 = t.kpi_version(client.company, 2).await;
    t.kpi_act(&manager, &v2, "withdraw", &[("reason", "wrong_client")])
        .await
        .assert_status(422);
    t.kpi_act(&manager, &v2, "publish", &[]).await.assert_ok();
    assert_eq!(t.kpi_now(&v1).await.status, KpiStatus::Superseded);
    t.kpi_act(&manager, &v1, "withdraw", &[("reason", "wrong_client")])
        .await
        .assert_status(422);
}

/// Done when 4: after a *wrong client* withdrawal, *copy* for client A
/// creates a `draft` for A's October with the same figures, categories and
/// commentary; the withdrawn pack stays `withdrawn` and staff still read it.
#[renox::test]
async fn copy_makes_a_draft_for_the_right_client_and_leaves_the_withdrawn_pack() {
    let t = boot().await;
    let manager = t.manager().await;
    let a = t.client_named(" Alpha").await;
    let b = t.client_named(" Beta").await;
    let wrong = t.kpi_published(&manager, b.company, TB_OCTOBER).await;
    t.kpi_act(&manager, &wrong, "withdraw", &[("reason", "wrong_client")])
        .await
        .assert_ok();
    // Only a withdrawn pack is copied.
    let live = t.kpi_published(&manager, a.company, TB_OCTOBER).await;
    t.kpi_act(
        &manager,
        &live,
        "copy",
        &[("client", &a.company.0.to_string())],
    )
    .await
    .assert_status(422);
    // Only a user with `kpi.manage` copies.
    let outsider = t.user_with_grants("Olga", &[]).await;
    t.kpi_act(
        &outsider,
        &wrong,
        "copy",
        &[("client", &a.company.0.to_string())],
    )
    .await
    .assert_forbidden();

    t.kpi_act(
        &manager,
        &wrong,
        "copy",
        &[("client", &a.company.0.to_string())],
    )
    .await
    .assert_ok();

    let copies = t.kpi_packs(a.company).await;
    let draft = copies
        .iter()
        .find(|p| p.status == KpiStatus::Draft)
        .expect("a draft for A");
    assert_eq!(draft.period_end, wrong.period_end);
    assert_eq!(draft.version, 2, "A's first pack is v1");
    assert_eq!(draft.commentary, wrong.commentary);
    assert_eq!(draft.copied_from_id, Some(wrong.id));
    let original = wrong.figures(t.db()).await.unwrap();
    let copied = draft.figures(t.db()).await.unwrap();
    assert_eq!(copied.len(), original.len());
    for (o, c) in original.iter().zip(&copied) {
        assert_eq!(
            (&o.account_code, o.amount, &o.category),
            (&c.account_code, c.amount, &c.category)
        );
    }
    // A's saved mapping is kept (4 accounts), none is added twice.
    assert_eq!(
        KpiMapping::of_client(t.db(), a.company)
            .await
            .unwrap()
            .len(),
        4
    );
    // The withdrawn pack is unchanged and readable by staff.
    let still = t.kpi_now(&wrong).await;
    assert_eq!(still.status, KpiStatus::Withdrawn);
    t.get(&manager, &format!("/accounting/kpi-packs/{}", wrong.id))
        .await
        .assert_ok()
        .assert_see("Wrong client");
    // A second copy for A is refused while its draft stands.
    t.kpi_act(
        &manager,
        &wrong,
        "copy",
        &[("client", &a.company.0.to_string())],
    )
    .await
    .assert_status(422);
    // After the withdrawal B's period has no published pack: a new import is
    // a new draft version.
    t.kpi_import(&manager, b.company, OCTOBER, TB_OCTOBER)
        .await
        .assert_ok();
    let again = t.kpi_packs(b.company).await;
    assert_eq!(again.len(), 2);
    assert_eq!(again[1].status, KpiStatus::Draft);
    assert_eq!(
        again[1].commentary, None,
        "nothing is copied from a withdrawn pack"
    );
}
