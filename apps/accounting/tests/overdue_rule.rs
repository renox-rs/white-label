//! Acceptance check DW4 of #337 (BL-ACC-13): `ACC.RUL.04` is
//! compliance-critical, so when its nightly run fails (five attempts) the
//! partner role and the company admin get the owner alert: with the real
//! job (wl-ticketing's `mark_overdue`, run under `ACC.RUL.04` as the host
//! sets it), made to fail by taking its table away, and with a run of the
//! rule whose body fails. The overdue, late and missed lifecycle itself
//! (DW1-DW3) is checked on wl-ticketing set as the 04 host sets it, in
//! `crates/wl-ticketing/tests/acc_overdue.rs`.

mod support;

use std::time::Duration;

use accounting::tasks::{MISSED_CLIENT_FILED_ITSELF, MISSED_REASONS, OVERDUE_RULE, missed_reason};
use renox::auth::permissions;
use renox::prelude::*;
use wl_kernel::COMPANY_ADMIN_ROLE;
use wl_kernel::automation::{RuleRunSpec, RunOutcome, run_rule};
use wl_kernel::compliance::{ComplianceAlert, is_critical, occurrence_key, schedule_subject};
use wl_kernel::notify::NotifyRecipient;

use support::{Booted, PARTNER, boot, boot_real, d};

/// One attempt of the run of 2026-11-29, failing.
async fn failing_attempt(t: &Booted) -> RunOutcome {
    let spec = RuleRunSpec::new(
        OVERDUE_RULE,
        schedule_subject(),
        occurrence_key(OVERDUE_RULE, d(11, 29)),
    );
    t.run(run_rule(t.state(), spec, |_state: AppState| async {
        Err::<(), Error>(Error::Status(
            StatusCode::INTERNAL_SERVER_ERROR,
            "the task store is unavailable".to_owned(),
        ))
    }))
    .await
    .unwrap()
}

/// Whether a request of the owner alert went to `user`.
fn alerted(t: &Booted, user: i64) -> bool {
    t.notices.all().iter().any(|r| {
        r.type_key == "kernel.compliance_alert"
            && r.recipients.contains(&NotifyRecipient::User { id: user })
    })
}

/// DW4: a failed `ACC.RUL.04` run alerts the partner role and the company
/// admin, once, owned by the partner role.
#[renox::test]
async fn bl_acc_13_a_failed_overdue_run_alerts_the_partner_and_the_admin() {
    let t = boot().await;
    assert!(is_critical(OVERDUE_RULE));
    let partner = t.partner().await;
    permissions::define_role(t.db(), COMPANY_ADMIN_ROLE, &[])
        .await
        .unwrap();
    let admin = t.user_with("Ada", COMPANY_ADMIN_ROLE).await;

    for wait in [60_u64, 300, 900, 3600] {
        assert_eq!(failing_attempt(&t).await, RunOutcome::Retrying);
        assert!(!alerted(&t, partner.id), "not before the last attempt");
        t.app.travel(Duration::from_secs(wait));
    }
    assert_eq!(failing_attempt(&t).await, RunOutcome::Failed);

    let alerts: Vec<ComplianceAlert> = ComplianceAlert::open(t.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.rule == OVERDUE_RULE)
        .collect();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0].owner_role, PARTNER);
    assert!(alerted(&t, partner.id), "the partner role is told");
    assert!(alerted(&t, admin.id), "the company admin is told");
}

/// Renames wl-ticketing's deadline task table, so that the overdue job's
/// query fails, or back.
async fn tasks_table_off(t: &Booted, off: bool) {
    let (from, to) = if off {
        (
            "wl_ticketing_deadline_tasks",
            "wl_ticketing_deadline_tasks_off",
        )
    } else {
        (
            "wl_ticketing_deadline_tasks_off",
            "wl_ticketing_deadline_tasks",
        )
    };
    renox::db::sql(format!("ALTER TABLE \"{from}\" RENAME TO \"{to}\""))
        .execute(t.db())
        .await
        .unwrap();
}

/// DW4 with the real job: the nightly `mark_overdue` of 2026-11-29 (the
/// 04 client's `ACC.RUL.04`) fails on each of its five attempts; after the
/// last the partner role and the company admin are alerted, once; a later
/// run of the same night does nothing more.
#[renox::test]
async fn bl_acc_13_the_real_overdue_job_failing_alerts_the_partner_and_the_admin() {
    let t = boot_real().await;
    let partner = t.partner().await;
    permissions::define_role(t.db(), COMPANY_ADMIN_ROLE, &[])
        .await
        .unwrap();
    let admin = t.user_with("Ada", COMPANY_ADMIN_ROLE).await;

    tasks_table_off(&t, true).await;
    for wait in [60_u64, 300, 900, 3600, 0] {
        t.run(wl_ticketing::deadlines::mark_overdue(t.state(), d(11, 29)))
            .await
            .expect("a failed attempt is recorded, not raised");
        if wait > 0 {
            assert!(!alerted(&t, partner.id), "not before the last attempt");
            t.app.travel(Duration::from_secs(wait));
        }
    }
    tasks_table_off(&t, false).await;

    let alerts: Vec<ComplianceAlert> = ComplianceAlert::open(t.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.rule == OVERDUE_RULE)
        .collect();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0].owner_role, PARTNER);
    assert!(alerted(&t, partner.id), "the partner role is told");
    assert!(alerted(&t, admin.id), "the company admin is told");

    // The night's run is over: it does not run again.
    t.run(wl_ticketing::deadlines::mark_overdue(t.state(), d(11, 29)))
        .await
        .unwrap();
    let alerts = ComplianceAlert::open(t.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.rule == OVERDUE_RULE)
        .count();
    assert_eq!(alerts, 1);
}

/// `close_missed` takes a reason from the 04 list; *client filed itself*
/// reads as such, and *other* needs a text.
#[test]
fn bl_acc_13_the_missed_reasons_are_the_04_list() {
    assert_eq!(MISSED_REASONS.len(), 4);
    assert_eq!(
        missed_reason(MISSED_CLIENT_FILED_ITSELF, None).as_deref(),
        Some("Client filed itself")
    );
    assert_eq!(missed_reason("other", None), None);
}
