//! The screening back office page (wl-parties §6, issue #40 part 5):
//! lists with version, last update and status, the latest screenings with
//! the stale badge, and the permission check.

mod support;

use wl_parties::{
    SanctionsList, ScreeningTrigger, WlParties, permissions as perm, screening, seed,
};

use support::boot_with;

const AL_NOOR: &str = "Al Noor Trading LLC";

#[renox::test]
async fn the_page_shows_lists_and_stale_screenings_to_the_three_permissions() {
    let t = boot_with(WlParties::new().screening(true), true).await;
    seed::lists(t.app.state()).await.unwrap();
    let party = t.party(AL_NOOR).await;
    screening::run(t.app.state(), &party, ScreeningTrigger::Manual)
        .await
        .unwrap();
    let list = SanctionsList::all(t.db()).await.unwrap().remove(0);

    for (name, grant) in [
        ("Rana", perm::SCREENING_RUN),
        ("Rev", perm::SCREENING_REVIEW),
        ("Mlro", perm::BLOCK_REASON_VIEW),
    ] {
        let user = t.user(name, &[grant]).await;
        t.app.acting_as(&user);
        t.app
            .get("/parties/screening")
            .await
            .assert_ok()
            .assert_see(&list.name)
            .assert_see("Stale")
            .assert_see("Stale list")
            .assert_see(&party.code);
    }

    let other = t.user("Pia", &[perm::VIEW]).await;
    t.app.acting_as(&other);
    t.app.get("/parties/screening").await.assert_forbidden();
}
