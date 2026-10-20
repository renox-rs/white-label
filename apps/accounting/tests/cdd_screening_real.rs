//! Acceptance checks of #332 (BL-ACC-08, `PTY.RUL.02`): CDD `submit` and
//! `approve` are refused on a stale list or a screening run on one, and
//! `approve` on an open possible match, through the real routes with the
//! real wl-parties.

mod support;

use accounting::CddStatus;

use support::{Booted, Client, UAE, UN, boot, dubai};

/// A prepared CDD file of a fresh client, every party screened `clear` on
/// current lists.
async fn prepared(t: &Booted) -> (Client, accounting::CddFile) {
    let client = t.client().await;
    let file = t.cdd_prepared(&client).await;
    t.lists_current().await;
    t.screen_all_clear(&client).await;
    (client, file)
}

/// DW1: a file whose UBO screening carries the stale flag (UN list of
/// 2026-11-02) is not submitted; the refusal names the list and the date. A
/// party not screened is refused too.
#[renox::test]
async fn submit_is_refused_on_a_screening_run_on_a_stale_list() {
    let t = boot().await;
    t.set_clock(3, 9, 0);
    let (client, file) = prepared(&t).await;
    let preparer = t.accountant().await;
    let path = format!("/accounting/cdd/{}/submit", file.id);

    t.screen_on_stale(client.ubo, UN, dubai(2, 10, 0)).await;
    let res = t.post(&preparer, &path, &[]).await;
    res.assert_status(422);
    let text = res.text();
    assert!(
        text.contains("This CDD file cannot be submitted:"),
        "{text}"
    );
    assert!(
        text.contains("Screening list is out of date: UN Consolidated List of 2026-11-02"),
        "{text}"
    );
    assert!(!text.contains(UAE), "{text}");
    assert_eq!(t.cdd(&file).await.status, CddStatus::InProgress);

    // A clear re-run on the current list: the file is submitted.
    t.screen_clear(client.ubo).await;
    t.post(&preparer, &path, &[]).await.assert_ok();
    assert_eq!(t.cdd(&file).await.status, CddStatus::PendingApproval);

    // A party with no screening at all: "screening done" fails.
    let other = t.client_named(" B").await;
    let unscreened = t.cdd_prepared(&other).await;
    for party in [other.company, other.ubo, other.director] {
        t.screen_clear(party).await;
    }
    let res = t
        .post(
            &preparer,
            &format!("/accounting/cdd/{}/submit", unscreened.id),
            &[],
        )
        .await;
    res.assert_status(422);
    assert!(
        res.text()
            .contains("Karim Nasser B (Signatory) is not screened yet"),
        "{}",
        res.text()
    );
}

/// DW2: a file `pending_approval` and the UAE list stale since 2026-11-04
/// 09:00: the MLRO's `approve` is refused and the file stays; after the
/// update and a `clear` re-run it is approved.
#[renox::test]
async fn approve_is_refused_while_a_list_is_stale_until_the_update_and_a_rerun() {
    let t = boot().await;
    t.set_clock(3, 9, 0);
    let (client, file) = prepared(&t).await;
    let preparer = t.accountant().await;
    let mlro = t.mlro().await;
    t.post(
        &preparer,
        &format!("/accounting/cdd/{}/submit", file.id),
        &[],
    )
    .await
    .assert_ok();
    let approve = format!("/accounting/cdd/{}/approve", file.id);

    // 2026-11-04 10:00: the UN list is current, the UAE list was last updated
    // on 2026-11-03 09:00 and is stale since 2026-11-04 09:00.
    t.set_clock(4, 10, 0);
    t.lists_current().await;
    t.list_stale(UAE, dubai(3, 9, 0), dubai(4, 9, 0)).await;
    let res = t.post(&mlro, &approve, &[]).await;
    res.assert_status(422);
    let text = res.text();
    assert!(text.contains("This CDD file cannot be approved:"), "{text}");
    assert!(
        text.contains("Screening list is out of date: UAE Local Terrorist List of 2026-11-03"),
        "{text}"
    );
    assert_eq!(t.cdd(&file).await.status, CddStatus::PendingApproval);

    // The update, then the `clear` re-run.
    t.lists_current().await;
    t.screen_clear(client.company).await;
    t.post(&mlro, &approve, &[]).await.assert_ok();
    let approved = t.cdd(&file).await;
    assert_eq!(approved.status, CddStatus::Approved);
    assert_eq!(approved.approved_by, Some(mlro.id));
    // Rated low: the next review in three years.
    assert_eq!(
        approved.next_review_date.map(|d| d.to_string()).as_deref(),
        Some("2029-11-04")
    );
}

/// DW3: an open possible match on a director refuses `approve`; the MLRO is
/// told which party, the preparer sees only "Screening pending".
#[renox::test]
async fn approve_is_refused_on_an_open_match_and_the_preparer_sees_screening_pending() {
    let t = boot().await;
    let (client, file) = prepared(&t).await;
    let preparer = t.accountant().await;
    let mlro = t.mlro().await;
    t.post(
        &preparer,
        &format!("/accounting/cdd/{}/submit", file.id),
        &[],
    )
    .await
    .assert_ok();
    t.screen_possible_match(client.director).await;

    let res = t
        .post(&mlro, &format!("/accounting/cdd/{}/approve", file.id), &[])
        .await;
    res.assert_status(422);
    assert!(
        res.text()
            .contains("Possible match on Lina Saeed (Director): the MLRO decides it"),
        "{}",
        res.text()
    );
    assert_eq!(t.cdd(&file).await.status, CddStatus::PendingApproval);

    // The preparer's page: "Screening pending", nothing about the match.
    let page = t
        .get(&preparer, &format!("/accounting/cdd/{}", file.id))
        .await;
    page.assert_ok().assert_see("Screening pending");
    let text = page.text();
    assert!(!text.contains("Possible match"), "{text}");
    assert!(!text.contains("Omar H. Haddad"), "{text}");
    // The MLRO's page names the party.
    t.get(&mlro, &format!("/accounting/cdd/{}", file.id))
        .await
        .assert_ok()
        .assert_see("Possible match on Lina Saeed (Director)");
}
