//! BL-ACC-23 (#347), rules and integrations, through the real host of the 04
//! Accounting client: the compliance-critical rules are marked, a failed or
//! stopped critical run alerts the partner role (the owner role) and the
//! company admin once each, a failed list download with a successful run
//! alerts only the MLRO, and the connections with a manual fallback exist
//! (04 §7.2, §8).

mod support;

use accounting::seed::CRITICAL_RULES;
use host::roles::{ADMIN, COMPLIANCE, PARTNER};
use renox::prelude::*;
use renox::testing::TestApp;
use support::{accounting_booted, messages_to, person};
use wl_kernel::automation::{MAX_ATTEMPTS, RuleRun, RuleRunSpec, run_rule};
use wl_kernel::compliance::service::NOTIFICATION_TYPE as OWNER_ALERT;
use wl_kernel::compliance::is_critical;
use wl_kernel::integrations::{ConnectionStatus, IntegrationConnection, go_down, report_failure};
use wl_kernel::Subject;

const STALE: &str = "party.sanctions_list.stale";

/// Fails the `PTY.RUL.02` run for good: the lists cannot be saved, five
/// attempts, each when the one before is due again.
async fn fail_the_list_run(app: &TestApp) {
    renox::db::sql(
        "CREATE TRIGGER refuse_lists BEFORE UPDATE ON wl_parties_sanctions_lists WHEN OLD.name = 'UN Consolidated List' \
         BEGIN SELECT RAISE(ABORT, 'refused for the test'); END",
    )
    .execute(app.db())
    .await
    .unwrap();
    for _ in 0..MAX_ATTEMPTS {
        wl_parties::freshness::run(app.state()).await.unwrap();
        RuleRun::query()
            .update(
                app.db(),
                &[(
                    "next_attempt_at",
                    &Some(renox::db::now() - renox::chrono::Duration::hours(1)),
                )],
            )
            .await
            .unwrap();
    }
    renox::db::sql("DROP TRIGGER refuse_lists")
        .execute(app.db())
        .await
        .unwrap();
}

/// Done when 4: a failed `PTY.RUL.02` run alerts the partner and the company
/// admin once each; a failed list download with a successful run gives only
/// the MLRO alert; a stopped run alerts them once each too.
#[renox::test]
async fn a_failed_or_stopped_critical_run_alerts_the_partner_and_the_admin_once_each() {
    let app = accounting_booted().await;
    for rule in CRITICAL_RULES {
        assert!(is_critical(rule), "{rule}");
    }
    assert!(!is_critical("ACC.RUL.06"), "a KPI import is not critical");
    let partner = person(&app, "Pia", PARTNER).await;
    let admin = person(&app, "Ada", ADMIN).await;
    let mlro = person(&app, "Mariam", COMPLIANCE).await;

    // The run itself fails: one owner alert each, once.
    fail_the_list_run(&app).await;
    assert_eq!(messages_to(&app, OWNER_ALERT, &partner).await.len(), 1);
    assert_eq!(messages_to(&app, OWNER_ALERT, &admin).await.len(), 1);
    assert!(messages_to(&app, OWNER_ALERT, &mlro).await.is_empty());

    // Two hours later a failed download with a successful run: the MLRO
    // only, no new owner alert.
    app.travel(std::time::Duration::from_secs(2 * 3600));
    app.at_travelled_time(wl_parties::freshness::run(app.state()))
        .await
        .unwrap();
    assert!(!messages_to(&app, STALE, &mlro).await.is_empty());
    assert_eq!(messages_to(&app, OWNER_ALERT, &partner).await.len(), 1);
    assert_eq!(messages_to(&app, OWNER_ALERT, &admin).await.len(), 1);

    // A stopped run (a chain too deep) alerts them once each as well.
    let spec = RuleRunSpec::new("PTY.RUL.02", Subject::new("parties.sanctions_list", 99), "stopped")
        .compliance_critical()
        .chain_depth(4);
    run_rule(app.state(), spec, |_state| async { Ok(()) })
        .await
        .unwrap();
    assert_eq!(messages_to(&app, OWNER_ALERT, &partner).await.len(), 2);
    assert_eq!(messages_to(&app, OWNER_ALERT, &admin).await.len(), 2);
}

/// Done when 5 (connections): the sanctions download and the e-mail
/// connections exist with their manual fallback and responsible role; when
/// e-mail goes down the fallback is on the connection.
#[renox::test]
async fn the_connections_with_a_manual_fallback_are_registered() {
    let app = accounting_booted().await;
    let sanctions = IntegrationConnection::by_key(app.db(), "sanctions.download")
        .await
        .unwrap()
        .expect("the sanctions download connection");
    assert_eq!(sanctions.owner_role, COMPLIANCE);
    assert!(sanctions.fallback_en.contains("by hand"));
    let email = IntegrationConnection::by_key(app.db(), "email")
        .await
        .unwrap()
        .expect("the e-mail connection");
    assert!(email.fallback_en.contains("visible in the portal"));
    assert_eq!(email.status, ConnectionStatus::Up);

    report_failure(app.state(), "email", "SMTP unreachable")
        .await
        .unwrap();
    let failing = IntegrationConnection::by_key(app.db(), "email")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failing.status, ConnectionStatus::Degraded);
    app.travel(std::time::Duration::from_secs(16 * 60));
    app.at_travelled_time(go_down(app.state(), &failing))
        .await
        .unwrap();
    let down = IntegrationConnection::by_key(app.db(), "email")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(down.status, ConnectionStatus::Down);
    // EmaraTax and goAML are manual: not connections.
    for key in ["emaratax", "goaml"] {
        assert!(IntegrationConnection::by_key(app.db(), key).await.unwrap().is_none());
    }
}
