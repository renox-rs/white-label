//! Acceptance checks of #325 (BL-ACC-01): engagement `activate` is refused on
//! an expired KYC document (with the MLRO's one-use override), a stale
//! screening, an open possible match or a blocked client, through the real
//! route with the real wl-parties and wl-documents.

mod support;

use accounting::documents::ACTIVATE_GUARDED;
use accounting::permissions as perm;
use accounting::seed::PASSPORT;
use accounting::{EngagementStatus, ScreenedRole};
use renox::prelude::*;
use wl_documents::{BlockOverride, BlockOverrideStatus, Documents};
use wl_kernel::approvals;

use support::{Booted, Client, UN, boot, d, dubai};

/// A user who activates engagements and sees restricted reasons and
/// matches (the MLRO's deputy acting as manager).
async fn deputy(t: &Booted) -> User {
    t.user_with_grants(
        "Dina",
        &[
            perm::ENGAGEMENTS_VIEW,
            perm::ENGAGEMENTS_MANAGE,
            perm::BLOCK_REASON_VIEW,
        ],
    )
    .await
}

/// The party of `client` that plays `role`.
fn party_of(client: &Client, role: ScreenedRole) -> wl_kernel::PartyId {
    match role {
        ScreenedRole::Client => client.company,
        ScreenedRole::Ubo => client.ubo,
        ScreenedRole::Director => client.director,
        ScreenedRole::Signatory => client.signatory,
    }
}

/// DW1: an onboarding engagement whose UBO passport expired on 2026-11-01 is
/// refused on 2026-11-03, naming the passport and its expiry.
#[renox::test]
async fn an_expired_ubo_passport_refuses_activation_naming_it() {
    let t = boot().await;
    t.set_clock(3, 9, 0);
    let mlro = t.mlro().await;
    let client = t.client().await;
    t.prepare_client(&client, &mlro, d(11, 1)).await;
    let engagement = t.signed_engagement(&client).await;
    let manager = t.manager().await;

    let res = t.activate(&manager, &engagement).await;
    res.assert_status(422);
    let text = res.text();
    assert!(text.contains("This engagement cannot be activated:"), "{text}");
    assert!(
        text.contains("Omar Haddad — Passport expired on 2026-11-01"),
        "{text}"
    );
    // Only the passport fails: the licence and the Emirates ID are valid.
    assert!(!text.contains("Trade licence"), "{text}");
    assert!(!text.contains("Emirates ID"), "{text}");
    assert_eq!(t.status(&engagement).await, EngagementStatus::Onboarding);
}

/// DW2 and DW3: manager A asks for an override with a reason; A cannot
/// approve it; MLRO M approves at 2026-11-03 10:00; A activates on
/// 2026-11-05 09:00; a second engagement of the same client cannot reuse it;
/// an override approved at the same time but unused is refused at
/// 2026-11-06 10:01.
#[renox::test]
async fn the_mlro_approves_a_one_use_override_valid_for_72_hours() {
    let t = boot().await;
    t.set_clock(3, 9, 0);
    let mlro = t.mlro().await;
    let client = t.client().await;
    t.prepare_client(&client, &mlro, d(11, 1)).await;
    let a = t.manager().await;
    let first = t.signed_engagement(&client).await;
    let second = t.signed_engagement(&client).await;
    let third = t.signed_engagement(&client).await;

    let ask = |id: i64| {
        Documents::request_override(
            t.state(),
            &a,
            PASSPORT,
            ACTIVATE_GUARDED,
            id,
            "Renewal appointment booked for 2026-11-10",
        )
    };
    // A reason is needed.
    assert!(
        t.run(Documents::request_override(
            t.state(),
            &a,
            PASSPORT,
            ACTIVATE_GUARDED,
            first.id,
            " ",
        ))
        .await
        .is_err()
    );
    let for_first = t.run(ask(first.id)).await.expect("the override is asked");
    assert_eq!(for_first.len(), 1, "one override: the UBO's passport");
    let for_third = t.run(ask(third.id)).await.expect("the override is asked");
    let first_request = for_first[0].approval_request_id.expect("an approval request");
    let third_request = for_third[0].approval_request_id.expect("an approval request");

    // DW3: A, the requester, cannot approve it.
    assert!(
        t.run(approvals::approve(t.state(), first_request, &a, ""))
            .await
            .is_err()
    );

    t.set_clock(3, 10, 0);
    for request in [first_request, third_request] {
        t.run(approvals::approve(t.state(), request, &mlro, ""))
            .await
            .expect("the MLRO approves");
    }
    let approved = BlockOverride::find(t.db(), for_first[0].id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approved.status, BlockOverrideStatus::Approved);

    // 2026-11-05 09:00: A activates the first engagement under the override.
    t.set_clock(5, 9, 0);
    t.lists_current().await;
    t.activate(&a, &first).await.assert_ok();
    assert_eq!(t.status(&first).await, EngagementStatus::Active);
    let used = BlockOverride::find(t.db(), for_first[0].id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(used.status, BlockOverrideStatus::Used);

    // The second engagement of the same client cannot reuse it.
    let res = t.activate(&a, &second).await;
    res.assert_status(422);
    assert!(res.text().contains("Passport expired on 2026-11-01"), "{}", res.text());
    assert_eq!(t.status(&second).await, EngagementStatus::Onboarding);

    // 2026-11-06 10:01: the third engagement's override, never used, is past
    // its 72 hours.
    t.set_clock(6, 10, 1);
    t.lists_current().await;
    let res = t.activate(&a, &third).await;
    res.assert_status(422);
    assert!(res.text().contains("Passport expired on 2026-11-01"), "{}", res.text());
    assert_eq!(t.status(&third).await, EngagementStatus::Onboarding);
}

/// DW4: with the UN list stale since 2026-11-02 06:00 (last updated
/// 2026-11-01 06:00), `activate` is refused naming the list and the date,
/// with no override offered; after the update and a `clear` re-run it
/// succeeds.
#[renox::test]
async fn a_stale_list_refuses_activation_until_the_update_and_a_clear_rerun() {
    let t = boot().await;
    t.set_clock(3, 9, 0);
    let mlro = t.mlro().await;
    let client = t.client().await;
    let engagement = t.ready_engagement(&client, &mlro).await;
    let manager = t.manager().await;

    t.list_stale(UN, dubai(1, 6, 0), dubai(2, 6, 0)).await;
    // The UBO's screening re-ran while the list was stale: it is flagged.
    t.screen_on_stale(client.ubo, UN, dubai(1, 6, 0)).await;
    let res = t.activate(&manager, &engagement).await;
    res.assert_status(422);
    let text = res.text();
    assert!(
        text.contains("Screening list is out of date: UN Consolidated List of 2026-11-01"),
        "{text}"
    );
    assert!(text.contains("wait for the list update"), "{text}");
    assert!(!text.to_lowercase().contains("override"), "{text}");
    assert_eq!(t.status(&engagement).await, EngagementStatus::Onboarding);

    // The list updates; the latest screening is still the flagged one.
    t.lists_current().await;
    let res = t.activate(&manager, &engagement).await;
    res.assert_status(422);
    assert!(res.text().contains(UN), "{}", res.text());

    // The `clear` re-run.
    t.screen_clear(client.ubo).await;
    t.activate(&manager, &engagement).await.assert_ok();
    assert_eq!(t.status(&engagement).await, EngagementStatus::Active);
}

/// DW5: an open possible match on a UBO, a director or a signatory refuses
/// `activate`; a manager without `parties.block_reason.view` sees only
/// "Screening pending", the MLRO's deputy sees the match.
#[renox::test]
async fn an_open_possible_match_refuses_activation_and_shows_screening_pending() {
    let t = boot().await;
    let mlro = t.mlro().await;
    let manager = t.manager().await;
    let dina = deputy(&t).await;

    for (tag, role, label) in [
        (" U", ScreenedRole::Ubo, "UBO"),
        (" D", ScreenedRole::Director, "Director"),
        (" S", ScreenedRole::Signatory, "Signatory"),
    ] {
        let client = t.client_named(tag).await;
        let engagement = t.ready_engagement(&client, &mlro).await;
        let party = party_of(&client, role);
        t.screen_possible_match(party).await;
        let name = t.party_row(party).await.display_name;

        let res = t.activate(&manager, &engagement).await;
        res.assert_status(422);
        let text = res.text();
        assert!(text.contains("Screening pending"), "{label}: {text}");
        assert!(!text.contains("Possible match"), "{label}: {text}");
        assert!(!text.contains(&name), "{label}: {text}");
        assert!(!text.contains("Omar H. Haddad"), "{label}: {text}");
        assert!(!text.to_lowercase().contains("override"), "{label}: {text}");

        let res = t.activate(&dina, &engagement).await;
        res.assert_status(422);
        assert!(
            res.text()
                .contains(&format!("Possible match on {name} ({label})")),
            "{label}: {}",
            res.text()
        );
        assert_eq!(t.status(&engagement).await, EngagementStatus::Onboarding);
    }
}

/// DW6: a blocked client's engagement is not activated, and a manager sees
/// "On hold" only; there is no exception path.
#[renox::test]
async fn a_blocked_client_refuses_activation_with_on_hold() {
    let t = boot().await;
    let mlro = t.mlro().await;
    let client = t.client().await;
    let engagement = t.ready_engagement(&client, &mlro).await;
    let manager = t.manager().await;
    let dina = deputy(&t).await;
    t.block(client.company, "sanctions").await;

    let res = t.activate(&manager, &engagement).await;
    res.assert_status(422);
    let text = res.text();
    assert!(text.contains("On hold"), "{text}");
    assert!(!text.to_lowercase().contains("sanction"), "{text}");
    assert!(!text.to_lowercase().contains("exception"), "{text}");
    assert_eq!(t.status(&engagement).await, EngagementStatus::Onboarding);

    // The deputy sees the restricted reason; still refused.
    let res = t.activate(&dina, &engagement).await;
    res.assert_status(422);
    assert!(res.text().contains("Sanctions"), "{}", res.text());
    assert_eq!(t.status(&engagement).await, EngagementStatus::Onboarding);
}
