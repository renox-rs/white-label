//! BL-POR-02 (#461) and BL-POR-03 (#462) in the real Property host: the
//! tenant audience and its portal roles are seeded and listed in the portal
//! settings; a terminated lease (`LeaseTerminated`, `LeaseForceTerminated`)
//! turns the tenant's grant read-only when no other lease of theirs runs,
//! and the daily close-after timer closes it on day 91 (`POR.ACL.01`).

mod support;

use host::roles::{ADMIN, MEMBER};
use renox::chrono::{NaiveDate, NaiveDateTime};
use renox::prelude::*;
use renox::testing::TestApp;
use support::{booted, draft_lease, person, tenant, unit_of};
use wl_kernel::{BusinessCalendar, PartyId};
use wl_portal::{AccessGrant, GrantStatus, NewGrant, PortalAccess, PortalRole};

/// Moves the clock to noon of `day` (Asia/Dubai, ahead of the real clock).
fn travel_to(app: &TestApp, day: &str) {
    let local = NaiveDateTime::parse_from_str(&format!("{day} 12:00"), "%Y-%m-%d %H:%M").unwrap();
    let ahead = (BusinessCalendar::default().from_local(local) - renox::db::now()).num_seconds();
    assert!(ahead > 0, "{day} is ahead of the clock");
    app.travel_back();
    app.travel(std::time::Duration::from_secs(ahead.unsigned_abs()));
}

/// A lease of a new unit for `tenant`, put straight in `status` (the lease
/// lifecycle is the property app's and tested there; here only the state
/// the portal listener reads matters).
async fn lease_in(app: &TestApp, code: &str, tenant: PartyId, status: &str) -> property::Lease {
    let unit = unit_of(app, code, "101", property::UnitUse::Residential).await;
    let lease = draft_lease(app, &unit, tenant, -30).await;
    renox::db::sql("UPDATE property_leases SET status = ? WHERE id = ?")
        .bind(status)
        .bind(lease.id)
        .execute(app.db())
        .await
        .unwrap();
    property::Lease::find_or_404(app.db(), lease.id)
        .await
        .unwrap()
}

fn terminated(lease: &property::Lease) -> property::LeaseTerminated {
    property::LeaseTerminated {
        lease: lease.id,
        root: lease.root_id,
        unit: lease.unit_id,
        tenant: lease.tenant(),
        move_out: NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
        rent_due_until: None,
        credited: 0,
    }
}

/// A portal user `name` with an `active` tenant grant for `party`.
async fn portal_user(app: &TestApp, name: &str, party: PartyId) -> (User, AccessGrant) {
    let user = person(app, name, MEMBER).await;
    let grant = PortalAccess::register(
        app.state(),
        user.id,
        NewGrant::new(party, party, "tenant", &["primary_contact"]),
    )
    .await
    .unwrap();
    (user, grant)
}

async fn status(app: &TestApp, grant: &AccessGrant) -> GrantStatus {
    AccessGrant::find_or_404(app.db(), grant.id)
        .await
        .unwrap()
        .status
}

/// BL-POR-02: the seed stores the tenant audience with `primary_contact`
/// and `finance_contact` and their permissions (01 §3.1); the company admin
/// sees them in the portal settings.
#[renox::test]
async fn por02_the_tenant_roles_are_seeded_and_listed_for_the_admin() {
    let app = booted().await;
    let roles = PortalRole::of_audience(app.db(), "tenant").await.unwrap();
    let of = |key: &str| {
        roles
            .iter()
            .find(|r| r.key == key)
            .unwrap_or_else(|| panic!("{key}"))
            .permission_keys()
    };
    assert_eq!(
        of("primary_contact"),
        [
            "portal.leases.view",
            "portal.invoices.view",
            "portal.invoices.pay",
            "portal.documents.view",
            "portal.requests.create",
            "portal.users.manage",
        ]
    );
    assert_eq!(
        of("finance_contact"),
        ["portal.invoices.view", "portal.invoices.pay"]
    );
    assert_eq!(
        wl_portal::audiences::close_after_days(app.db(), "tenant")
            .await
            .unwrap(),
        90
    );
    let admin = person(&app, "Hana", ADMIN).await;
    app.acting_as(&admin);
    let page = app.get("/portal/roles").await;
    page.assert_ok();
    assert!(page.text().contains("finance_contact"));
}

/// BL-POR-03 Done when 1 and 2: Sara's only lease is terminated on
/// 2026-12-31: her grant turns `read_only` (the home shows the notice); on
/// 2027-03-31 it is still `read_only`; on 2027-04-01 the daily timer closes
/// it.
#[renox::test]
async fn por03_the_last_lease_terminated_makes_the_grant_read_only_then_closed() {
    let app = booted().await;
    let party = tenant(&app, "Sara M.").await;
    let lease = lease_in(&app, "PT1", party, "terminated").await;
    let (sara, grant) = portal_user(&app, "Sara", party).await;

    travel_to(&app, "2026-12-31");
    app.at_travelled_time(app.state().emit(terminated(&lease)))
        .await
        .unwrap();
    let ended = AccessGrant::find_or_404(app.db(), grant.id).await.unwrap();
    assert_eq!(ended.status, GrantStatus::ReadOnly);
    assert_eq!(ended.ended_on, NaiveDate::from_ymd_opt(2026, 12, 31));
    app.acting_as(&sara);
    let home = app.get(&format!("/portal/{}", party.0)).await;
    home.assert_ok();
    assert!(
        home.text()
            .contains("Your access is read-only since 31 Dec 2026.")
    );

    travel_to(&app, "2027-03-31");
    app.run_scheduled(wl_portal::CLOSE_TASK).await.unwrap();
    assert_eq!(status(&app, &grant).await, GrantStatus::ReadOnly);

    travel_to(&app, "2027-04-01");
    app.run_scheduled(wl_portal::CLOSE_TASK).await.unwrap();
    assert_eq!(status(&app, &grant).await, GrantStatus::Closed);
    app.travel_back();
}

/// BL-POR-03 Done when 3: a tenant with a second active lease keeps an
/// `active` grant when one lease terminates.
#[renox::test]
async fn por03_a_second_running_lease_keeps_the_grant_active() {
    let app = booted().await;
    let party = tenant(&app, "Khalid R.").await;
    let ended = lease_in(&app, "PT2", party, "terminated").await;
    let _running = lease_in(&app, "PT3", party, "active").await;
    let (_khalid, grant) = portal_user(&app, "Khalid", party).await;
    app.state().emit(terminated(&ended)).await.unwrap();
    assert_eq!(status(&app, &grant).await, GrantStatus::Active);
}

/// BL-POR-03 Done when 4: a `force_terminated` lease ends the grant the same
/// way.
#[renox::test]
async fn por03_a_force_terminated_lease_ends_the_grant() {
    let app = booted().await;
    let party = tenant(&app, "Yusuf A.").await;
    let lease = lease_in(&app, "PT4", party, "terminated").await;
    let (_yusuf, grant) = portal_user(&app, "Yusuf", party).await;
    app.state()
        .emit(property::LeaseForceTerminated {
            lease: lease.id,
            root: lease.root_id,
            unit: lease.unit_id,
            tenant: party,
            reason: "abandoned".into(),
            move_out: NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            requested_by: None,
            evidence_cheques: 0,
            closed_cases: 0,
        })
        .await
        .unwrap();
    let ended = AccessGrant::find_or_404(app.db(), grant.id).await.unwrap();
    assert_eq!(ended.status, GrantStatus::ReadOnly);
    assert_eq!(
        ended.ended_reason.as_deref(),
        Some("lease force-terminated")
    );
}
