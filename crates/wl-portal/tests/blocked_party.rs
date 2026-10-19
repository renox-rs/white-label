//! BL-POR-01 (#44), `PTY.ACL.02`: while the party a portal user acts for is
//! blocked, the portal refuses new business and money out with one neutral
//! text, logs the refusal for compliance staff only, and shows nothing else:
//! no block, no status, no reason. History and payments work, grants stay
//! as they are, and on unblock the actions work again.

mod support;

use renox::prelude::*;
use wl_kernel::{PartyBlocked, PartyId, PartyUnblocked};
use wl_portal::{AccessGrant, GrantStatus, PortalAccess, RefusalLog};

use support::{ACME_ONE, ACME_TWO, CHILD, GUARDIAN, SARA_PARTY, boot};

const NEUTRAL_EN: &str = "This request cannot be completed online. Please contact us.";
const NEUTRAL_AR: &str = "هذا الطلب لا يمكن إتمامه عبر الإنترنت. يرجى التواصل معنا.";

/// AC1 to AC3, AC5: Sara M.'s party is blocked (`non_payment`); she pays an
/// open invoice, her deposit release request is refused with the neutral
/// text, a maintenance request is accepted as usual.
#[renox::test]
async fn por01_a_blocked_tenant_pays_but_cannot_start_new_business_or_move_money_out() {
    let t = boot().await;
    let (sara, _) = t.portal_user("Sara", 70, SARA_PARTY).await;
    t.blocks.block(SARA_PARTY);

    t.act(&sara, "pay_invoice", SARA_PARTY, "en").await.assert_ok();
    let refused = t.act(&sara, "deposit_release_request", SARA_PARTY, "en").await;
    refused.assert_status(422);
    assert!(refused.text().contains(NEUTRAL_EN), "{}", refused.text());
    t.act(&sara, "maintenance_request", SARA_PARTY, "en")
        .await
        .assert_ok();
    assert_eq!(
        t.done.all(),
        ["pay_invoice:70", "maintenance_request:70"],
        "the refused request created nothing"
    );

    // Her request is logged once, with no reason.
    let log = RefusalLog::latest(t.db(), 10).await.unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].user_id, sara.id);
    assert_eq!(log[0].party_id, SARA_PARTY);
    assert_eq!(log[0].action, "deposit_release_request");
    assert!(log[0].created_at.is_some());
}

/// AC4 and AC6: every action of the closed list is refused with exactly the
/// same text, whatever the reason, in English and in Arabic; no page shows
/// "On hold", a status or a reason.
#[renox::test]
async fn por01_every_refused_action_shows_the_same_text_and_never_a_reason() {
    let t = boot().await;
    let (sara, _) = t.portal_user("Sara", 70, SARA_PARTY).await;
    t.blocks.block(SARA_PARTY);
    for key in [
        "new_booking",
        "new_order",
        "sign_agreement",
        "refund_request",
        "deposit_release_request",
        "cancel_with_refund",
    ] {
        let en = t.act(&sara, key, SARA_PARTY, "en").await;
        en.assert_status(422);
        assert!(en.text().contains(NEUTRAL_EN), "{key}: {}", en.text());
        for secret in ["On hold", "on_hold", "Sanctions", "Blocked:"] {
            assert!(!en.text().contains(secret), "{key}: {secret}");
        }
        let ar = t.act(&sara, key, SARA_PARTY, "ar").await;
        ar.assert_status(422);
        assert!(ar.text().contains(NEUTRAL_AR), "{key}: {}", ar.text());
    }
    assert!(t.done.all().is_empty());
    assert_eq!(RefusalLog::latest(t.db(), 100).await.unwrap().len(), 12);
    assert_eq!(wl_portal::neutral(&t.state().lang("en")), NEUTRAL_EN);
    assert_eq!(wl_portal::neutral(&t.state().lang("ar")), NEUTRAL_AR);
}

/// The other actions are not new business and are never refused because of
/// the block: a complaint, a message, a document upload, a profile change,
/// cancelling without a refund.
#[renox::test]
async fn por01_the_other_actions_work_as_usual() {
    let t = boot().await;
    let (sara, _) = t.portal_user("Sara", 70, SARA_PARTY).await;
    t.blocks.block(SARA_PARTY);
    for key in [
        "pay_invoice",
        "maintenance_request",
        "complaint",
        "message",
        "document_upload",
        "profile_change",
        "cancel_without_refund",
    ] {
        t.act(&sara, key, SARA_PARTY, "en").await.assert_ok();
    }
    assert_eq!(t.done.all().len(), 7);
    assert!(RefusalLog::latest(t.db(), 10).await.unwrap().is_empty());
}

/// AC5 and AC7: the grant stays `active` and no message is queued when the
/// party is blocked; after `PartyUnblocked` the deposit-release request is
/// accepted.
#[renox::test]
async fn por01_blocking_changes_no_grant_and_sends_nothing_and_unblocking_restores_the_actions() {
    let t = boot().await;
    let (sara, grant) = t.portal_user("Sara", 70, SARA_PARTY).await;

    t.blocks.block(SARA_PARTY);
    t.state()
        .emit(PartyBlocked {
            party: PartyId(SARA_PARTY),
            display_reason: "on_hold".to_owned(),
        })
        .await
        .unwrap();
    let after = AccessGrant::find_or_404(t.db(), grant.id).await.unwrap();
    assert_eq!(after.status, GrantStatus::Active);
    assert_eq!(after.suspend_reason, None);
    assert!(t.notices.all().is_empty(), "{:?}", t.notices.all());
    t.act(&sara, "deposit_release_request", SARA_PARTY, "en")
        .await
        .assert_status(422);
    assert!(t.notices.all().is_empty());

    t.blocks.unblock(SARA_PARTY);
    t.state()
        .emit(PartyUnblocked {
            party: PartyId(SARA_PARTY),
            display_reason: "on_hold".to_owned(),
        })
        .await
        .unwrap();
    t.act(&sara, "deposit_release_request", SARA_PARTY, "en")
        .await
        .assert_ok();
    assert_eq!(t.done.all(), ["deposit_release_request:70"]);
    let grants = PortalAccess::grants_for(t.db(), sara.id).await.unwrap();
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].status, GrantStatus::Active);
}

/// The acting-for party of the current grant is checked: a group contact with
/// a grant per member company is refused for the blocked company only, and a
/// guardian acting for a blocked dependant is refused for the dependant, not
/// for themselves.
#[renox::test]
async fn por01_only_the_party_the_user_acts_for_is_checked() {
    let t = boot().await;
    let cfo = t.user("Carla", &[]).await;
    for company in [ACME_ONE, ACME_TWO] {
        PortalAccess::register(
            t.state(),
            cfo.id,
            wl_portal::NewGrant::new(PartyId(60), PartyId(company), "client", &["finance_contact"]),
        )
        .await
        .unwrap();
    }
    t.blocks.block(ACME_ONE);
    t.act(&cfo, "new_order", ACME_ONE, "en").await.assert_status(422);
    t.act(&cfo, "new_order", ACME_TWO, "en").await.assert_ok();

    let guardian = t.user("Gus", &[]).await;
    for party in [GUARDIAN, CHILD] {
        PortalAccess::register(
            t.state(),
            guardian.id,
            wl_portal::NewGrant::new(PartyId(GUARDIAN), PartyId(party), "guardian", &["guardian"]),
        )
        .await
        .unwrap();
    }
    t.blocks.block(CHILD);
    t.act(&guardian, "new_booking", CHILD, "en")
        .await
        .assert_status(422);
    t.act(&guardian, "new_booking", GUARDIAN, "en")
        .await
        .assert_ok();
}

/// Without an `active` grant for the party there is no access at all: not
/// even the neutral text, and nothing is logged.
#[renox::test]
async fn por01_a_user_without_an_active_grant_for_the_party_is_forbidden() {
    let t = boot().await;
    let (sara, grant) = t.portal_user("Sara", 70, SARA_PARTY).await;
    t.blocks.block(SARA_PARTY);
    // Another party: no grant.
    t.act(&sara, "new_booking", ACME_ONE, "en").await.assert_forbidden();
    // A suspended grant gives nothing.
    let staff = t.user("Stan", &[wl_portal::permissions::USERS_MANAGE]).await;
    t.post_as(
        &staff,
        &format!("/test/grants/{}/suspend", grant.id),
        &[("reason", "dispute")],
    )
    .await
    .assert_ok();
    t.act(&sara, "pay_invoice", SARA_PARTY, "en").await.assert_forbidden();
    assert!(RefusalLog::latest(t.db(), 10).await.unwrap().is_empty());
}

/// AC2: the refusal log is for staff with `parties.block_reason.view` only:
/// not for the party's own user and not for other staff.
#[renox::test]
async fn por01_the_refusal_log_is_visible_to_the_mlro_only() {
    let t = boot().await;
    let (sara, _) = t.portal_user("Sara", 70, SARA_PARTY).await;
    t.blocks.block(SARA_PARTY);
    t.act(&sara, "deposit_release_request", SARA_PARTY, "en")
        .await
        .assert_status(422);

    let mlro = t
        .user("Mlro", &[wl_portal::permissions::BLOCK_REASON_VIEW])
        .await;
    let other = t.user("Olga", &[wl_portal::permissions::USERS_MANAGE]).await;
    t.app.acting_as(&mlro);
    let page = t.app.get("/portal/block-refusals").await;
    page.assert_ok();
    page.assert_see("Sara");
    page.assert_see("Deposit release request");
    page.assert_see(&format!("#{SARA_PARTY}"));
    t.app.acting_as(&other);
    t.app.get("/portal/block-refusals").await.assert_forbidden();
    t.app.acting_as(&sara);
    t.app.get("/portal/block-refusals").await.assert_forbidden();
}
