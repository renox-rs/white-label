//! BL-PRP-05 (#255): submit and activate renewal and amendment versions for
//! the sitting tenant. The unit guard exempts such versions, `activate` acts
//! per version kind, and a renewal or an amendment never changes the unit.

mod support;

use property::lease_machine::FLAG_RENEWAL;
use property::leases::NewVersion;
use property::{ChargeStatus, Lease, LeaseStatus, UnitStatus, VersionKind};
use renox::prelude::*;

use support::{AGENT, MANAGER, Real, aed, boot_with, day};

/// A renewal for the year after the active term ends, at `rent`.
fn renewal(rent: i64, deposit: i64) -> NewVersion {
    NewVersion::renewal(day(65), day(429), rent, deposit, 4)
}

/// A unit with an active lease v1 that is due for renewal, and the users.
struct Scene {
    t: support::Booted,
    unit: property::Unit,
    v1: Lease,
    agent: User,
    manager: User,
}

async fn scene() -> Scene {
    let t = boot_with(Real::DOCUMENTS).await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let v1 = t.active_lease(&unit, &agent, &manager).await;
    t.system(&v1, FLAG_RENEWAL).await;
    let v1 = t.reload(&v1).await;
    assert_eq!(v1.status, LeaseStatus::RenewalDue);
    Scene {
        t,
        unit,
        v1,
        agent,
        manager,
    }
}

/// AC1, AC2: a renewal draft for the sitting tenant passes the unit guard on
/// `submit`; a new lease for another tenant on the occupied unit does not.
#[renox::test]
async fn bl_prp_05_a_renewal_passes_the_unit_guard_and_a_new_lease_does_not() {
    let s = scene().await;
    let t = &s.t;
    assert_eq!(t.unit_now(&s.unit).await.status, UnitStatus::Occupied);

    let v2 = property::leases::create_version(
        t.state(),
        Some(s.agent.id),
        s.v1.id,
        renewal(aed(110_000), aed(5_000)),
    )
    .await
    .unwrap();
    assert_eq!((v2.version, v2.version_kind), (2, VersionKind::Renewal));
    assert_eq!(v2.root_id, s.v1.root_id);
    assert_eq!(v2.previous_id, Some(s.v1.id));
    assert_eq!(v2.tenant_party_id, s.v1.tenant_party_id);
    assert_eq!(v2.unit_id, s.unit.id);
    // The rent goes up: the tenant's signed acceptance is attached (`PRP.ACL.17`).
    t.attach_acceptance(&v2).await;
    t.submit(&s.agent, &v2).await.assert_ok();
    assert_eq!(t.status(&v2).await, LeaseStatus::PendingSignature);

    let other = property::NewLease::new(
        s.unit.id,
        support::COMPANY,
        day(65),
        day(429),
        aed(90_000),
        4,
    );
    let u = property::leases::create_draft(t.state(), None, other)
        .await
        .unwrap();
    let refused = t.submit(&s.agent, &u).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("not available"),
        "{}",
        refused.text()
    );
}

/// AC3: activating the renewal makes v1 `renewed` and v2 `active`; the unit
/// stays `occupied`; the charge schedule starts on v2's start date; an Ejari
/// task is created and `LeaseActivated` is emitted.
#[renox::test]
async fn bl_prp_05_activating_a_renewal_replaces_the_previous_version() {
    let s = scene().await;
    let t = &s.t;
    let v2 = property::leases::create_version(
        t.state(),
        Some(s.agent.id),
        s.v1.id,
        renewal(aed(110_000), aed(5_000)),
    )
    .await
    .unwrap();
    t.attach_acceptance(&v2).await;
    t.submit(&s.agent, &v2).await.assert_ok();
    t.activate(&s.manager, &v2).await.assert_ok();

    assert_eq!(t.status(&s.v1).await, LeaseStatus::Renewed);
    assert_eq!(t.status(&v2).await, LeaseStatus::Active);
    assert_eq!(t.unit_now(&s.unit).await.status, UnitStatus::Occupied);
    let versions = Lease::versions_of(t.db(), s.v1.root_id).await.unwrap();
    let active = versions
        .iter()
        .filter(|l| l.status == LeaseStatus::Active)
        .count();
    assert_eq!(active, 1, "never two versions active at once");

    let charges = t.reload(&v2).await.charges(t.db()).await.unwrap();
    assert_eq!(charges.len(), 4);
    assert!(charges.iter().all(|c| c.status == ChargeStatus::Scheduled));
    assert_eq!(charges[0].due_date, v2.start_date);

    let tasks = t.tasks.with_key(&format!("property.ejari:{}", v2.id));
    assert_eq!(tasks.len(), 1);
    assert!(
        tasks[0].title.contains("Renew the Ejari"),
        "{}",
        tasks[0].title
    );
    let events = t.activated.all();
    let last = events.last().expect("LeaseActivated");
    assert_eq!((last.lease, last.kind), (v2.id, VersionKind::Renewal));
    // An unchanged deposit is carried over: nothing asks the deposit to adjust.
    assert!(t.tasks.with_key("property.deposit_adjust").is_empty());
}

/// AC4: activating an amendment makes the previous version `amended`; the
/// unit is unchanged.
#[renox::test]
async fn bl_prp_05_activating_an_amendment_ends_the_previous_version_as_amended() {
    let t = boot_with(Real::DOCUMENTS).await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let v1 = t.active_lease(&unit, &agent, &manager).await;

    let v2 = property::leases::create_version(
        t.state(),
        Some(agent.id),
        v1.id,
        NewVersion::amendment(day(0), v1.end_date, aed(105_000), aed(5_000), 2),
    )
    .await
    .unwrap();
    assert_eq!(v2.version_kind, VersionKind::Amendment);
    assert_eq!(v2.effective_date, Some(day(0)));
    // An amendment against the tenant always needs the tenant's acceptance.
    t.attach_acceptance(&v2).await;
    t.submit(&agent, &v2).await.assert_ok();
    t.activate(&manager, &v2).await.assert_ok();

    assert_eq!(t.status(&v1).await, LeaseStatus::Amended);
    assert_eq!(t.status(&v2).await, LeaseStatus::Active);
    assert_eq!(t.unit_now(&unit).await.status, UnitStatus::Occupied);
    let tasks = t.tasks.with_key(&format!("property.ejari:{}", v2.id));
    assert!(
        tasks[0].title.contains("Update the Ejari"),
        "{}",
        tasks[0].title
    );
}

/// An amendment needs its effective date, within the term of the version it
/// changes and not before the first day of this month.
#[renox::test]
async fn bl_prp_05_an_amendments_effective_date_is_checked() {
    let t = boot_with(Real::DOCUMENTS).await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let v1 = t.active_lease(&unit, &agent, &manager).await;

    let mut missing = NewVersion::amendment(day(0), v1.end_date, aed(105_000), 0, 2);
    missing.effective_date = None;
    assert!(
        property::leases::create_version(t.state(), None, v1.id, missing)
            .await
            .is_err()
    );

    for (effective, why) in [
        (day(-40), "before the first day of this month"),
        (day(200), "after the end of the term"),
    ] {
        let draft = property::leases::create_version(
            t.state(),
            None,
            v1.id,
            NewVersion::amendment(effective, day(300), aed(105_000), 0, 2),
        )
        .await
        .unwrap();
        let refused = t.submit(&agent, &draft).await;
        refused.assert_status(422);
        assert_eq!(t.status(&draft).await, LeaseStatus::Draft, "{why}");
        // Only one version in progress at a time: cancel this draft to go on.
        t.set(&draft, &[("status", &LeaseStatus::Cancelled)]).await;
    }
}

/// A renewal needs the previous version due for renewal (or holding over); an
/// amendment also accepts an active one.
#[renox::test]
async fn bl_prp_05_the_previous_version_must_be_in_a_state_renew_or_amend_accepts() {
    let t = boot_with(Real::DOCUMENTS).await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let v1 = t.active_lease(&unit, &agent, &manager).await;

    let v2 = property::leases::create_version(t.state(), None, v1.id, renewal(aed(110_000), 0))
        .await
        .unwrap();
    let refused = t.submit(&agent, &v2).await;
    refused.assert_status(422);
    assert!(refused.text().contains("active"), "{}", refused.text());
    assert_eq!(t.status(&v2).await, LeaseStatus::Draft);
}

/// AC5: a second renewal draft for the same lease is refused while one is in
/// progress (a draft or waiting for signature).
#[renox::test]
async fn bl_prp_05_only_one_renewal_or_amendment_is_in_progress_per_lease() {
    let s = scene().await;
    let t = &s.t;
    let first = property::leases::create_version(
        t.state(),
        None,
        s.v1.id,
        renewal(aed(110_000), aed(5_000)),
    )
    .await
    .unwrap();
    let draft_twice = property::leases::create_version(
        t.state(),
        None,
        s.v1.id,
        renewal(aed(120_000), aed(5_000)),
    )
    .await;
    assert!(matches!(
        draft_twice,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
    t.attach_acceptance(&first).await;
    t.submit(&s.agent, &first).await.assert_ok();
    let while_pending = property::leases::create_version(
        t.state(),
        None,
        s.v1.id,
        NewVersion::amendment(day(0), s.v1.end_date, aed(95_000), 0, 2),
    )
    .await;
    assert!(while_pending.is_err());
    // The page offers no new version while one is in progress.
    t.app.acting_as(&s.agent);
    t.app
        .get(&format!("/property/leases/{}", s.v1.id))
        .await
        .assert_ok()
        .assert_dont_see("Renew or amend");
}

/// AC6: if v1 was terminated meanwhile, activating v2 is refused with the
/// reason; nothing changes.
#[renox::test]
async fn bl_prp_05_a_previous_version_that_moved_refuses_the_activation() {
    let s = scene().await;
    let t = &s.t;
    let v2 = property::leases::create_version(
        t.state(),
        None,
        s.v1.id,
        renewal(aed(110_000), aed(5_000)),
    )
    .await
    .unwrap();
    t.attach_acceptance(&v2).await;
    t.submit(&s.agent, &v2).await.assert_ok();
    // The previous version ended meanwhile (a bulk update is the fixture).
    t.set(&s.v1, &[("status", &LeaseStatus::Terminated)]).await;

    let refused = t.activate(&s.manager, &v2).await;
    refused.assert_status(422);
    assert!(refused.text().contains("terminated"), "{}", refused.text());
    assert_eq!(t.status(&v2).await, LeaseStatus::PendingSignature);
    assert_eq!(t.status(&s.v1).await, LeaseStatus::Terminated);
    assert_eq!(t.unit_now(&s.unit).await.status, UnitStatus::Occupied);
    let charges = t.reload(&v2).await.charges(t.db()).await.unwrap();
    assert!(charges.iter().all(|c| c.status == ChargeStatus::Planned));
}

/// A renewal whose deposit differs asks the deposit to adjust (system); when
/// it cannot, the accountant gets a task.
#[renox::test]
async fn bl_prp_05_a_changed_deposit_asks_the_deposit_to_adjust() {
    let s = scene().await;
    let t = &s.t;
    let v2 = property::leases::create_version(
        t.state(),
        None,
        s.v1.id,
        renewal(aed(110_000), aed(6_000)),
    )
    .await
    .unwrap();
    t.attach_acceptance(&v2).await;
    t.submit(&s.agent, &v2).await.assert_ok();
    t.activate(&s.manager, &v2).await.assert_ok();
    assert_eq!(t.status(&v2).await, LeaseStatus::Active);
    // The test book holds no deposit to adjust, so the accountant is asked to.
    let tasks = t
        .tasks
        .with_key(&format!("property.deposit_adjust:{}", v2.id));
    assert_eq!(tasks.len(), 1);
    assert!(
        tasks[0].title.contains("AED 6,000.00"),
        "{}",
        tasks[0].title
    );
}
