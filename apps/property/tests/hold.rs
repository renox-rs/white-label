//! BL-PRP-15 (#265): while a tenant is on hold (`PTY.ACL.03`), no new lease,
//! renewal or amendment is submitted or activated for it, with the neutral
//! text; the active lease goes on (it keeps billing and holding over); the
//! deposit refund of a blocked party is an exception request (tested with the
//! deposit in wl-payments).

mod support;

use property::lease_machine::{EXPIRE, FLAG_RENEWAL};
use property::leases::NewVersion;
use property::{Lease, LeaseStatus, NewLease};
use renox::prelude::*;
use wl_kernel::block_guard::ActionKind;

use support::{AGENT, Booted, MANAGER, TENANT, aed, boot, day};

struct Scene {
    t: Booted,
    unit: property::Unit,
    agent: User,
    manager: User,
}

async fn scene() -> Scene {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    Scene {
        t,
        unit,
        agent,
        manager,
    }
}

impl Scene {
    /// An `active` lease due for renewal, and a renewal draft of it.
    async fn renewal_due(&self) -> (Lease, Lease) {
        let v1 = self
            .t
            .active_lease(&self.unit, &self.agent, &self.manager)
            .await;
        self.t.system(&v1, FLAG_RENEWAL).await;
        let v1 = self.t.reload(&v1).await;
        let v2 = property::leases::create_version(
            self.t.state(),
            Some(self.agent.id),
            v1.id,
            NewVersion::renewal(day(65), day(429), aed(100_000), aed(5_000), 4),
        )
        .await
        .unwrap();
        // The tenant's signed acceptance is attached (`PRP.ACL.17`).
        self.t.accept(&v2);
        (v1, v2)
    }
}

/// AC1: tenant T is blocked: the `submit` of a renewal version for T is
/// refused with the neutral text, and the version stays a draft.
#[renox::test]
async fn bl_prp_15_a_renewal_for_a_blocked_tenant_is_refused_with_the_neutral_text() {
    let s = scene().await;
    let (_v1, v2) = s.renewal_due().await;
    s.t.blocks.block(TENANT);

    let refused = s.t.submit(&s.agent, &v2).await;
    refused.assert_status(422);
    assert!(refused.text().contains("on hold"), "{}", refused.text());
    assert!(!refused.text().contains("Blocked:"), "{}", refused.text());
    assert_eq!(s.t.status(&v2).await, LeaseStatus::Draft);
    assert!(s.t.blocks.asked().contains(&(TENANT.0, ActionKind::Renew)));

    // Free again, it goes through.
    s.t.blocks.free(TENANT);
    s.t.submit(&s.agent, &v2).await.assert_ok();
    assert_eq!(s.t.status(&v2).await, LeaseStatus::PendingSignature);
}

/// AC2: a version in `pending_signature` before the block cannot be activated
/// after it; it stays `pending_signature`, and the version it replaces is
/// untouched.
#[renox::test]
async fn bl_prp_15_a_version_waiting_for_signature_cannot_be_activated_after_the_block() {
    let s = scene().await;
    let (v1, v2) = s.renewal_due().await;
    s.t.submit(&s.agent, &v2).await.assert_ok();
    s.t.blocks.block(TENANT);

    let refused = s.t.activate(&s.manager, &v2).await;
    refused.assert_status(422);
    assert!(refused.text().contains("on hold"), "{}", refused.text());
    assert_eq!(s.t.status(&v2).await, LeaseStatus::PendingSignature);
    assert_eq!(s.t.status(&v1).await, LeaseStatus::RenewalDue);
    assert!(s.t.blocks.asked().iter().filter(|(_, k)| *k == ActionKind::Renew).count() >= 2);

    s.t.blocks.free(TENANT);
    s.t.activate(&s.manager, &v2).await.assert_ok();
    assert_eq!(s.t.status(&v2).await, LeaseStatus::Active);
}

/// A new lease is a `start`, an amendment an `extend`: both are refused for a
/// blocked tenant.
#[renox::test]
async fn bl_prp_15_a_new_lease_and_an_amendment_are_refused_too() {
    let s = scene().await;
    let v1 = s
        .t
        .active_lease(&s.unit, &s.agent, &s.manager)
        .await;
    let other_unit = s.t.unit(&s.t.building_coded("TWR-B").await, "88").await;
    let new = NewLease::new(other_unit.id, TENANT, day(5), day(369), aed(90_000), 4);
    let lease = property::leases::create_draft(s.t.state(), None, new)
        .await
        .unwrap();
    let amendment = property::leases::create_version(
        s.t.state(),
        Some(s.agent.id),
        v1.id,
        NewVersion::amendment(day(0), v1.end_date, aed(95_000), 0, 2),
    )
    .await
    .unwrap();
    s.t.accept(&amendment);
    s.t.blocks.block(TENANT);

    let building_b = s.t.user_with("Omar", AGENT).await;
    let refused = s.t.submit(&building_b, &lease).await;
    refused.assert_status(422);
    assert!(refused.text().contains("on hold"), "{}", refused.text());
    assert_eq!(s.t.status(&lease).await, LeaseStatus::Draft);
    assert!(s.t.blocks.asked().contains(&(TENANT.0, ActionKind::Start)));

    let refused = s.t.submit(&s.agent, &amendment).await;
    refused.assert_status(422);
    assert!(refused.text().contains("on hold"), "{}", refused.text());
    assert!(s.t.blocks.asked().contains(&(TENANT.0, ActionKind::Extend)));
}

/// AC3: the blocked tenant's active lease continues: it keeps issuing
/// invoices, goes on to hold over when its term ends and is not ended or
/// paused.
#[renox::test]
async fn bl_prp_15_the_active_lease_of_a_blocked_tenant_goes_on() {
    let s = scene().await;
    let v1 = s
        .t
        .active_lease(&s.unit, &s.agent, &s.manager)
        .await;
    s.t.blocks.block(TENANT);

    let report = property::invoicing::run(s.t.state(), day(0)).await.unwrap();
    assert!(report.invoiced >= 1, "{report:?}");
    assert_eq!(report.failed, 0);
    assert_eq!(s.t.status(&v1).await, LeaseStatus::Active);

    // Its term ends: it holds over and the charges go on.
    s.t.set(&v1, &[("end_date", &day(-1))]).await;
    s.t.system(&v1, FLAG_RENEWAL).await;
    s.t.system(&v1, EXPIRE).await;
    assert_eq!(s.t.status(&v1).await, LeaseStatus::Expired);
}
