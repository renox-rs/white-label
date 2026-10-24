//! BL-POR-03 (#462), `POR.ACL.01`: when the vertical's contract ends the
//! grant turns `read_only` (pages and documents still read, write actions
//! refused with the read-only notice, the deposit deductions dispute kept),
//! and the daily close-after timer closes it on day 91, ending the user's
//! sessions.

mod support;

use renox::chrono::NaiveDate;
use renox::db::sql;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_portal::{
    AccessGrant, GrantStatus, PortalAccess, PortalAccessClosed, PortalAccessReadOnly,
};

use support::{SARA_PARTY, boot};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

async fn reload(t: &support::Booted, grant: &AccessGrant) -> AccessGrant {
    AccessGrant::find_or_404(t.db(), grant.id).await.unwrap()
}

async fn sessions_revoked_at(t: &support::Booted, user: &User) -> i64 {
    sql("SELECT sessions_revoked_at FROM users WHERE id = ?")
        .bind(user.id)
        .scalar::<i64>(t.db())
        .await
        .unwrap()
}

/// Done when 1: Sara's only lease is terminated on 2026-12-31: her grant is
/// `read_only` and `PortalAccessReadOnly` is emitted; she still opens her
/// invoices and her documents (with the notice), and *Pay*, *Dispute* and
/// *New request* are refused with the read-only notice. The deposit
/// deductions dispute stays allowed.
#[renox::test]
async fn por03_a_terminated_lease_makes_the_grant_read_only() {
    let t = boot().await;
    t.with_tenant_portal().await;
    let (sara, grant) = t
        .tenant_user("Sara", SARA_PARTY, &["primary_contact"])
        .await;
    t.app.fake_events();
    let ended = PortalAccess::contract_ended(
        t.state(),
        PartyId(SARA_PARTY),
        "tenant",
        day(2026, 12, 31),
        "lease terminated",
    )
    .await
    .unwrap();
    assert_eq!(ended.len(), 1);
    let grant = reload(&t, &grant).await;
    assert_eq!(grant.status, GrantStatus::ReadOnly);
    assert_eq!(grant.ended_on, Some(day(2026, 12, 31)));
    assert_eq!(grant.ended_reason.as_deref(), Some("lease terminated"));
    t.app
        .assert_emitted::<PortalAccessReadOnly>(|e| e.grant == grant.id);

    let notice = "Your access is read-only since 31 Dec 2026.";
    for key in ["invoices", "documents", "lease", "requests"] {
        let page = t.get_as(&sara, &format!("/portal/{SARA_PARTY}/{key}")).await;
        page.assert_ok();
        assert!(page.text().contains(notice), "{key}");
    }
    let home = t.get_as(&sara, &format!("/portal/{SARA_PARTY}")).await;
    home.assert_ok();
    assert!(home.text().contains(notice));
    // A module's download reads through the readable grants.
    assert_eq!(
        PortalAccess::readable_grants(t.db(), sara.id, PartyId(SARA_PARTY), today())
            .await
            .unwrap()
            .len(),
        1
    );

    for key in ["pay_invoice", "dispute", "maintenance_request", "message"] {
        let refused = t.act(&sara, key, SARA_PARTY, "en").await;
        refused.assert_forbidden();
        assert!(refused.text().contains(notice), "{key}");
    }
    t.act(&sara, "deposit_deduction_dispute", SARA_PARTY, "en")
        .await
        .assert_ok();
    assert_eq!(
        t.done.all(),
        [format!("deposit_deduction_dispute:{SARA_PARTY}")]
    );
    // Nothing is reopened by a second event.
    let again = PortalAccess::contract_ended(
        t.state(),
        PartyId(SARA_PARTY),
        "tenant",
        day(2027, 1, 5),
        "lease terminated",
    )
    .await
    .unwrap();
    assert!(again.is_empty());
    assert_eq!(reload(&t, &grant).await.ended_on, Some(day(2026, 12, 31)));
}

/// Done when 2: on 2027-03-31 the grant is still `read_only`; on 2027-04-01
/// (day 91) it is `closed`, `PortalAccessClosed` is emitted and her sessions
/// end.
#[renox::test]
async fn por03_the_grant_closes_on_day_91() {
    let t = boot().await;
    t.with_tenant_portal().await;
    let (sara, grant) = t
        .tenant_user("Sara", SARA_PARTY, &["primary_contact"])
        .await;
    PortalAccess::contract_ended(
        t.state(),
        PartyId(SARA_PARTY),
        "tenant",
        day(2026, 12, 31),
        "lease terminated",
    )
    .await
    .unwrap();
    assert_eq!(reload(&t, &grant).await.closes_on(90), Some(day(2027, 4, 1)));
    assert_eq!(sessions_revoked_at(&t, &sara).await, 0);
    t.app.fake_events();

    for on in [day(2027, 1, 1), day(2027, 3, 31)] {
        let closed = PortalAccess::close_due(t.state(), on).await.unwrap();
        assert!(closed.is_empty(), "{on}");
        assert_eq!(reload(&t, &grant).await.status, GrantStatus::ReadOnly);
    }
    let closed = PortalAccess::close_due(t.state(), day(2027, 4, 1))
        .await
        .unwrap();
    assert_eq!(closed.len(), 1);
    let grant = reload(&t, &grant).await;
    assert_eq!(grant.status, GrantStatus::Closed);
    assert_eq!(
        grant.ended_reason.as_deref(),
        Some(wl_portal::CLOSE_AFTER_REASON)
    );
    t.app
        .assert_emitted::<PortalAccessClosed>(|e| e.grant == grant.id);
    assert!(sessions_revoked_at(&t, &sara).await > 0);
}

/// The period is the audience's: a 60-day audience closes on day 61.
#[renox::test]
async fn por03_the_period_is_the_audiences() {
    let t = boot().await;
    t.with_tenant_portal().await;
    wl_portal::audiences::seed(
        t.db(),
        &wl_portal::AudienceDef {
            key: "worker",
            label: "Workers",
            close_after_days: 60,
            roles: &[],
        },
    )
    .await
    .unwrap();
    let user = t.user("Wasim", &[]).await;
    let grant = PortalAccess::register(
        t.state(),
        user.id,
        wl_portal::NewGrant::new(
            PartyId(SARA_PARTY),
            PartyId(SARA_PARTY),
            "worker",
            &["worker"],
        ),
    )
    .await
    .unwrap();
    PortalAccess::contract_ended(
        t.state(),
        PartyId(SARA_PARTY),
        "worker",
        day(2026, 11, 10),
        "worker exited",
    )
    .await
    .unwrap();
    assert!(
        PortalAccess::close_due(t.state(), day(2027, 1, 9))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        PortalAccess::close_due(t.state(), day(2027, 1, 10))
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(reload(&t, &grant).await.status, GrantStatus::Closed);
}

/// A grant suspended at the event stays suspended; `reinstate` returns it
/// to `read_only`. A grant of another audience is not touched.
#[renox::test]
async fn por03_a_suspended_grant_is_reinstated_to_read_only() {
    let t = boot().await;
    t.with_tenant_portal().await;
    let staff = t
        .user("Mona", &[wl_portal::permissions::USERS_MANAGE])
        .await;
    let (_sara, grant) = t
        .tenant_user("Sara", SARA_PARTY, &["primary_contact"])
        .await;
    let owner_user = t.user("Faisal", &[]).await;
    let owner_grant = PortalAccess::register(
        t.state(),
        owner_user.id,
        wl_portal::NewGrant::new(
            PartyId(SARA_PARTY),
            PartyId(SARA_PARTY),
            "owner",
            &["owner"],
        ),
    )
    .await
    .unwrap();
    t.post_as(
        &staff,
        &format!("/test/grants/{}/suspend", grant.id),
        &[("reason", "dispute")],
    )
    .await
    .assert_ok();
    let ended = PortalAccess::contract_ended(
        t.state(),
        PartyId(SARA_PARTY),
        "tenant",
        day(2026, 12, 31),
        "lease terminated",
    )
    .await
    .unwrap();
    assert!(ended.is_empty());
    assert_eq!(reload(&t, &grant).await.status, GrantStatus::Suspended);
    assert_eq!(reload(&t, &owner_grant).await.status, GrantStatus::Active);
    t.post_as(&staff, &format!("/test/grants/{}/reinstate", grant.id), &[])
        .await
        .assert_ok();
    let back = reload(&t, &grant).await;
    assert_eq!(back.status, GrantStatus::ReadOnly);
    assert_eq!(back.ended_on, Some(day(2026, 12, 31)));
}
