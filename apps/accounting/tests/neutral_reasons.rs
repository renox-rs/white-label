//! Acceptance checks of #331 (BL-ACC-07, `ACC.ACL.14`): a restricted
//! suspension or end shows only "On hold" or "Ended" to staff without
//! `parties.block_reason.view` and in the portal, no automatic message goes
//! to the client, and the MLRO's neutral template is recorded on the
//! engagement.

mod support;

use accounting::client_notices::{ClientNotice, NeutralTemplate, ON_HOLD_NOTICE};
use accounting::engagement_machine::AUTO_SUSPEND;
use accounting::{Engagement, EngagementStatus};
use renox::prelude::*;
use renox::state_machine::history;

use support::{Booted, Client, boot, register_portal_section};

/// What a restricted reason must never show to these viewers.
const RESTRICTED: [&str; 4] = ["sanctions", "Sanctions", "aml_concern", "AML"];

fn assert_neutral(what: &str, text: &str) {
    for word in RESTRICTED {
        assert!(!text.contains(word), "{what} shows `{word}`: {text}");
    }
}

/// An active engagement of a fresh client, and the client blocked for
/// `sanctions` by the system.
async fn blocked_for_sanctions(t: &Booted) -> (Client, Engagement) {
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    t.block(client.company, "sanctions").await;
    (client, engagement)
}

/// DW1: an engagement suspended by a `sanctions` block shows "On hold" to a
/// partner without `parties.block_reason.view` on the page, in the history
/// and in the export; the MLRO sees "sanctions" on the party's block record.
#[renox::test]
async fn a_sanctions_block_shows_on_hold_to_the_partner_and_sanctions_to_the_mlro() {
    let t = boot().await;
    let partner = t.partner().await;
    let mlro = t.mlro().await;
    let (client, engagement) = blocked_for_sanctions(&t).await;

    let suspended = t.reload(&engagement).await;
    assert_eq!(suspended.status, EngagementStatus::Suspended);
    assert_eq!(suspended.reason_code.as_deref(), Some("on_hold"));
    assert!(!suspended.may_message_client());

    // The page: the reason and the history row read "On hold".
    let page = t
        .get(
            &partner,
            &format!("/accounting/engagements/{}", engagement.id),
        )
        .await;
    page.assert_ok()
        .assert_see("On hold")
        .assert_see("Suspended");
    assert_neutral("the partner's page", &page.text());
    page.assert_dont_see(&format!("/parties/{}", client.company.0));

    // The history keeps the neutral code only.
    let rows = history(t.db(), Engagement::TABLE, engagement.id)
        .await
        .unwrap();
    let last = rows.last().unwrap();
    assert_eq!(last.transition, AUTO_SUSPEND);
    assert_eq!(last.reason.as_deref(), Some("on_hold"));

    // The export.
    let export = t.get(&partner, "/accounting/engagements/export").await;
    export.assert_ok();
    let csv = export.text();
    assert!(csv.contains(&engagement.number()), "{csv}");
    assert!(csv.contains("On hold"), "{csv}");
    assert_neutral("the export", &csv);

    // The list.
    let list = t.get(&partner, "/accounting/engagements").await;
    list.assert_ok().assert_see("On hold");
    assert_neutral("the list", &list.text());

    // The MLRO: the engagement shows the same neutral code and links to the
    // party, whose Block record names the real reason.
    t.get(&mlro, &format!("/accounting/engagements/{}", engagement.id))
        .await
        .assert_ok()
        .assert_see("On hold")
        .assert_see(&format!("/parties/{}", client.company.0));
    let record = t
        .get(&mlro, &format!("/parties/{}", client.company.0))
        .await;
    record.assert_ok();
    assert!(
        record.text().to_lowercase().contains("sanctions"),
        "the MLRO's party page: {}",
        record.text()
    );
}

/// DW2: the client user sees "On hold" in the portal, with no reason and no
/// block, and no e-mail goes to the client.
#[renox::test]
async fn the_client_user_sees_on_hold_in_the_portal_and_gets_no_message() {
    register_portal_section();
    let t = boot().await;
    let (client, engagement) = blocked_for_sanctions(&t).await;
    let owner = t.portal_user("Hana", client.ubo, client.company).await;

    let page = t
        .get(&owner, &format!("/portal/{}/engagements", client.company.0))
        .await;
    page.assert_ok()
        .assert_see(&engagement.number())
        .assert_see("On hold");
    let text = page.text();
    assert_neutral("the portal", &text);
    for word in ["Blocked", "on_hold", "Suspended", "Reason"] {
        assert!(!text.contains(word), "the portal shows `{word}`: {text}");
    }

    for party in client.parties() {
        assert!(
            t.notices.to_party(party).is_empty(),
            "a message went to {party}: {:?}",
            t.notices.to_party(party)
        );
    }
}

/// DW3: the MLRO sends the neutral "on hold" template; it is recorded on the
/// engagement and goes to the client. A partner may not send it.
#[renox::test]
async fn the_mlro_sends_the_neutral_template_and_it_is_recorded() {
    let t = boot().await;
    let partner = t.partner().await;
    let mlro = t.mlro().await;
    let (client, engagement) = blocked_for_sanctions(&t).await;
    let path = format!("/accounting/engagements/{}/client-notice", engagement.id);

    t.post(&partner, &path, &[("template", "on_hold")])
        .await
        .assert_forbidden();
    // The "ended" template does not fit a suspended engagement.
    t.post(&mlro, &path, &[("template", "ended")])
        .await
        .assert_status(422);
    t.post(&mlro, &path, &[("template", "on_hold")])
        .await
        .assert_ok();

    let recorded = ClientNotice::of_engagement(t.db(), engagement.id)
        .await
        .unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].template, NeutralTemplate::OnHold);
    assert_eq!(recorded[0].sent_by, mlro.id);

    let sent = t.notices.to_party(client.company);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].type_key, ON_HOLD_NOTICE);
    assert_eq!(sent[0].subject, engagement.subject());

    // The page shows what the client was told, to the partner too.
    let page = t
        .get(
            &partner,
            &format!("/accounting/engagements/{}", engagement.id),
        )
        .await;
    page.assert_ok()
        .assert_see("Your engagement is on hold, please contact your partner.")
        .assert_see("Mariam");
    assert_neutral("the partner's page", &page.text());
}

/// DW4: after a CDD exit the partner ends the engagement; non-MLRO staff and
/// the portal see "Ended", the exit reason stays with the MLRO, and nothing
/// goes to the client.
#[renox::test]
async fn an_end_after_a_cdd_exit_shows_ended_to_staff_and_the_portal() {
    register_portal_section();
    let t = boot().await;
    let partner = t.partner().await;
    let mlro = t.mlro().await;
    let manager = t.manager().await;
    let client = t.client().await;
    let file = t.cdd_approved(&client, &mlro).await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;

    t.post(
        &mlro,
        &format!("/accounting/cdd/{}/exit", file.id),
        &[("reason", "AML concern: unexplained transfers")],
    )
    .await
    .assert_ok();
    // Whatever reason the partner picks, the end after an exit is the
    // neutral "relationship ended".
    t.act(
        &partner,
        &engagement,
        "end",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
    let ended = t.reload(&engagement).await;
    assert_eq!(ended.status, EngagementStatus::Ended);
    assert_eq!(ended.reason_code.as_deref(), Some("relationship_ended"));
    assert!(!ended.may_message_client());

    for viewer in [&partner, &manager] {
        let page = t
            .get(
                viewer,
                &format!("/accounting/engagements/{}", engagement.id),
            )
            .await;
        page.assert_ok().assert_see("Ended");
        assert_neutral("the staff page", &page.text());
        page.assert_dont_see("unexplained transfers");
        let cdd = t.get(viewer, &format!("/accounting/cdd/{}", file.id)).await;
        cdd.assert_ok().assert_dont_see("unexplained transfers");
    }
    let export = t.get(&manager, "/accounting/engagements/export").await;
    assert!(export.text().contains("Ended"), "{}", export.text());
    assert!(!export.text().contains("unexplained"), "{}", export.text());
    // The MLRO sees the exit reason on the CDD file.
    t.get(&mlro, &format!("/accounting/cdd/{}", file.id))
        .await
        .assert_ok()
        .assert_see("AML concern: unexplained transfers");

    let owner = t.portal_user("Hana", client.ubo, client.company).await;
    let portal = t
        .get(&owner, &format!("/portal/{}/engagements", client.company.0))
        .await;
    portal.assert_ok().assert_see("Ended");
    assert_neutral("the portal", &portal.text());
    assert!(!portal.text().contains("unexplained"));

    for party in client.parties() {
        assert!(
            t.notices.to_party(party).is_empty(),
            "a message went to {party}"
        );
    }
}
