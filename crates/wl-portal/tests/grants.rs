//! The access grant lifecycle (wl-portal §4, issue #44's base): invitation and
//! acceptance, self-registration, suspension and reinstatement, the end of a
//! relationship, closing and the terminal states, with who may do each.

mod support;

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_portal::permissions::{USERS_INVITE, USERS_MANAGE};
use wl_portal::{
    AccessGrant, GrantStatus, PortalAccess, PortalAccessActivated, PortalAccessClosed,
    PortalAccessReadOnly, PortalAccessSuspended,
};

use support::{SARA_PARTY, boot};

async fn status(t: &support::Booted, id: i64) -> GrantStatus {
    AccessGrant::find_or_404(t.db(), id).await.unwrap().status
}

/// Self-registration creates a grant directly `active` for the user's own
/// party and says so.
#[renox::test]
async fn a_self_registered_grant_is_active() {
    let t = boot().await;
    t.app.fake_events();
    let (sara, grant) = t.portal_user("Sara", 70, SARA_PARTY).await;
    assert_eq!(grant.status, GrantStatus::Active);
    assert_eq!(grant.user_id, Some(sara.id));
    assert_eq!(grant.source, "self_registration");
    assert_eq!(grant.role_keys(), ["primary_contact"]);
    assert_eq!(grant.party(), PartyId(SARA_PARTY));
    let found = PortalAccess::active_grant(t.db(), sara.id, PartyId(SARA_PARTY), grant.valid_from)
        .await
        .unwrap();
    assert_eq!(found.map(|g| g.id), Some(grant.id));
    assert!(
        PortalAccess::active_grant(t.db(), sara.id, PartyId(99), grant.valid_from)
            .await
            .unwrap()
            .is_none()
    );
    let seen = t.app.emitted::<PortalAccessActivated>();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].user, sara.id);
}

/// An invitation (`portal_users.invite`) is `invited` until the invitee
/// accepts; it can be revoked while open; only staff with the permission
/// invite.
#[renox::test]
async fn an_invitation_is_accepted_or_revoked() {
    let t = boot().await;
    let inviter = t.user("Ines", &[USERS_INVITE]).await;
    let nobody = t.user("Nora", &[]).await;
    let form = [
        ("person", "70"),
        ("party", "70"),
        ("roles", "primary_contact,viewer"),
    ];
    t.post_as(&nobody, "/test/grants", &form)
        .await
        .assert_forbidden();
    let created = t.post_as(&inviter, "/test/grants", &form).await;
    created.assert_ok();
    let id: i64 = created.text().parse().unwrap();
    let invited = AccessGrant::find_or_404(t.db(), id).await.unwrap();
    assert_eq!(invited.status, GrantStatus::Invited);
    assert_eq!(invited.user_id, None);
    assert_eq!(invited.role_keys(), ["primary_contact", "viewer"]);

    let invitee = t.user("Ivan", &[]).await;
    let accepted = PortalAccess::accept(t.state(), id, invitee.id)
        .await
        .unwrap();
    assert_eq!(accepted.status, GrantStatus::Active);
    assert_eq!(accepted.user_id, Some(invitee.id));
    // An accepted invitation cannot be revoked or accepted again.
    t.post_as(&inviter, &format!("/test/grants/{id}/revoke"), &[])
        .await
        .assert_status(422);
    assert!(
        PortalAccess::accept(t.state(), id, invitee.id)
            .await
            .is_err()
    );

    let second = t.post_as(&inviter, "/test/grants", &form).await;
    let second: i64 = second.text().parse().unwrap();
    t.post_as(&nobody, &format!("/test/grants/{second}/revoke"), &[])
        .await
        .assert_forbidden();
    t.post_as(&inviter, &format!("/test/grants/{second}/revoke"), &[])
        .await
        .assert_ok();
    assert_eq!(status(&t, second).await, GrantStatus::Revoked);

    let third = t.post_as(&inviter, "/test/grants", &form).await;
    let third: i64 = third.text().parse().unwrap();
    assert_eq!(
        PortalAccess::expire(t.state(), third).await.unwrap().status,
        GrantStatus::Expired
    );
    assert!(PortalAccess::expire(t.state(), third).await.is_err());
}

/// Suspending needs `portal_users.manage` and a reason; reinstating returns
/// the grant to the state before: `active` or `read_only`.
#[renox::test]
async fn a_suspended_grant_goes_back_to_where_it_came_from() {
    let t = boot().await;
    t.app.fake_events();
    let (sara, grant) = t.portal_user("Sara", 70, SARA_PARTY).await;
    let staff = t.user("Stan", &[USERS_MANAGE]).await;
    let nobody = t.user("Nora", &[]).await;
    let suspend = format!("/test/grants/{}/suspend", grant.id);
    let reinstate = format!("/test/grants/{}/reinstate", grant.id);

    t.post_as(&nobody, &suspend, &[("reason", "dispute")])
        .await
        .assert_forbidden();
    t.post_as(&staff, &suspend, &[("reason", " ")])
        .await
        .assert_status(422);
    assert_eq!(status(&t, grant.id).await, GrantStatus::Active);

    t.post_as(&staff, &suspend, &[("reason", "dispute")])
        .await
        .assert_ok();
    let suspended = AccessGrant::find_or_404(t.db(), grant.id).await.unwrap();
    assert_eq!(suspended.status, GrantStatus::Suspended);
    assert_eq!(suspended.suspended_from.as_deref(), Some("active"));
    assert_eq!(suspended.suspend_reason.as_deref(), Some("dispute"));
    // A suspended grant gives no access.
    assert!(
        PortalAccess::active_grant(t.db(), sara.id, PartyId(SARA_PARTY), suspended.valid_from)
            .await
            .unwrap()
            .is_none()
    );
    t.post_as(&nobody, &reinstate, &[]).await.assert_forbidden();
    t.post_as(&staff, &reinstate, &[]).await.assert_ok();
    assert_eq!(status(&t, grant.id).await, GrantStatus::Active);

    // From `read_only`: end, suspend, reinstate.
    let end = format!("/test/grants/{}/end", grant.id);
    t.post_as(&staff, &end, &[("reason", "lease terminated")])
        .await
        .assert_ok();
    assert_eq!(status(&t, grant.id).await, GrantStatus::ReadOnly);
    t.post_as(&staff, &suspend, &[("reason", "security")])
        .await
        .assert_ok();
    assert_eq!(
        AccessGrant::find_or_404(t.db(), grant.id)
            .await
            .unwrap()
            .suspended_from
            .as_deref(),
        Some("read_only")
    );
    t.post_as(&staff, &reinstate, &[]).await.assert_ok();
    assert_eq!(status(&t, grant.id).await, GrantStatus::ReadOnly);

    assert_eq!(t.app.emitted::<PortalAccessSuspended>().len(), 2);
    assert_eq!(t.app.emitted::<PortalAccessReadOnly>().len(), 1);
}

/// `end` takes an `active` grant to `read_only`; `close` ends it for good,
/// from `read_only`, `active` or `suspended`; the system ends and closes too.
#[renox::test]
async fn ending_and_closing_a_grant() {
    let t = boot().await;
    t.app.fake_events();
    let staff = t.user("Stan", &[USERS_MANAGE]).await;
    let nobody = t.user("Nora", &[]).await;
    let (_, one) = t.portal_user("Sara", 70, SARA_PARTY).await;
    let (_, two) = t.portal_user("Tara", 71, 71).await;
    let (_, three) = t.portal_user("Uma", 72, 72).await;

    t.post_as(&nobody, &format!("/test/grants/{}/end", one.id), &[])
        .await
        .assert_forbidden();
    // The system: the relationship ended, then the close-after period.
    let ended = PortalAccess::end(t.state(), None, one.id, Some("lease terminated"))
        .await
        .unwrap();
    assert_eq!(ended.status, GrantStatus::ReadOnly);
    assert_eq!(ended.ended_reason.as_deref(), Some("lease terminated"));
    // Only an active grant ends.
    assert!(
        PortalAccess::end(t.state(), None, one.id, None)
            .await
            .is_err()
    );
    let closed = PortalAccess::close(t.state(), None, one.id, Some("90 days passed"))
        .await
        .unwrap();
    assert_eq!(closed.status, GrantStatus::Closed);
    // Closed is final.
    assert!(
        PortalAccess::close(t.state(), None, one.id, None)
            .await
            .is_err()
    );
    t.post_as(
        &staff,
        &format!("/test/grants/{}/suspend", one.id),
        &[("reason", "x")],
    )
    .await
    .assert_status(422);

    // Close an active one by hand, and a suspended one.
    t.post_as(&staff, &format!("/test/grants/{}/close", two.id), &[])
        .await
        .assert_ok();
    assert_eq!(status(&t, two.id).await, GrantStatus::Closed);
    t.post_as(
        &staff,
        &format!("/test/grants/{}/suspend", three.id),
        &[("reason", "x")],
    )
    .await
    .assert_ok();
    t.post_as(&staff, &format!("/test/grants/{}/close", three.id), &[])
        .await
        .assert_ok();
    assert_eq!(status(&t, three.id).await, GrantStatus::Closed);
    assert_eq!(t.app.emitted::<PortalAccessClosed>().len(), 3);
}
