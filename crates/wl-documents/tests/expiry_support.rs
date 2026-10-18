//! The test support of the expiry run (#48 part 5) works with
//! `expiry::run`: the recording notifier, the planned-visits provider, the
//! kinds, the reminder schedule and the roles in scopes. The acceptance
//! checks are `expiry.rs` (part 6).

mod support;

use wl_documents::expiry;
use wl_kernel::documents::DocumentExpiring;

use support::{
    boot, civil_defence_approval, day, ecas_certificate, plant1, plant2, safefire, tower_a,
    tower_b, valve,
};

/// Later visits of the vendor reach the coordinator of their own building.
#[renox::test]
async fn planned_visits_are_reported_to_the_coordinator_of_the_building() {
    let t = boot().await;
    t.install(&[civil_defence_approval()]).await;
    t.roles(&["coordinator"]).await;
    let anna = t.staff("Anna", "coordinator", Some(&tower_a())).await;
    let bela = t.staff("Bela", "coordinator", Some(&tower_b())).await;
    let rita = t.reviewer().await;
    t.accepted(&rita, safefire(), "civil_defence_approval", Some(day(30)))
        .await;
    t.plan.visit(tower_a(), day(35));
    t.plan.visit(tower_a(), day(60));
    // On the expiry date the document is still valid.
    t.plan.visit(tower_a(), day(30));

    let report = expiry::run(t.state(), day(0)).await.unwrap();

    assert_eq!(
        (
            report.scanned,
            report.events,
            report.messages,
            report.failed
        ),
        (1, 1, 1, 0),
        "{report:?}"
    );
    assert!(t.notices.to_user(bela.id).is_empty());
    let sent = t.notices.to_user(anna.id);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].type_key, expiry::run::NOTIFICATION_TYPE);
    assert_eq!(sent[0].data["items"].as_array().unwrap().len(), 2);
    t.app.assert_emitted::<DocumentExpiring>(|event| {
        event.days_left == 30 && event.affected.len() == 1 && event.affected[0].records.len() == 2
    });
}

/// A certificate with no scope and no schedule goes to every plant, once.
#[renox::test]
async fn without_a_schedule_the_reminder_goes_to_every_plant() {
    let t = boot().await;
    t.install(&[ecas_certificate()]).await;
    t.roles(&["qc_inspector"]).await;
    let pia = t.staff("Pia", "qc_inspector", Some(&plant1())).await;
    let quin = t.staff("Quin", "qc_inspector", Some(&plant2())).await;
    let rita = t.reviewer().await;
    t.accepted(&rita, valve(), "ecas_certificate", Some(day(30)))
        .await;

    expiry::run(t.state(), day(0)).await.unwrap();
    assert_eq!(t.notices.to_user(pia.id).len(), 1);
    assert_eq!(t.notices.to_user(quin.id).len(), 1);
    assert!(t.plan.visits().is_empty());

    // The 30-day reminder is logged: nothing more that night.
    t.notices.clear();
    expiry::run(t.state(), day(0)).await.unwrap();
    assert!(t.notices.sent().is_empty());
}

/// The schedule names the plant the reminder goes to.
#[renox::test]
async fn the_schedule_limits_the_reminder_to_its_plant() {
    let t = boot().await;
    t.install(&[ecas_certificate()]).await;
    t.roles(&["qc_inspector"]).await;
    let pia = t.staff("Pia", "qc_inspector", Some(&plant1())).await;
    let quin = t.staff("Quin", "qc_inspector", Some(&plant2())).await;
    let rita = t.reviewer().await;
    t.accepted(&rita, valve(), "ecas_certificate", Some(day(30)))
        .await;
    t.plan.schedule(vec![plant1()]);

    expiry::run(t.state(), day(0)).await.unwrap();

    assert_eq!(t.notices.to_user(pia.id).len(), 1);
    assert!(t.notices.to_user(quin.id).is_empty());
}
