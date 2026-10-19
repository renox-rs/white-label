//! The back office sheets of issue #42 part 4: *Upload official file* on the
//! screening page and *Record manual check* on the party page, their routes,
//! and the permission gates.

mod support;

use renox::prelude::Model;

use wl_parties::{
    ListFormat, ListStatus, SanctionsList, Screening, UpdateSource, WlParties, permissions as perm,
    seed,
};

use support::{Booted, boot_custom, set_dubai_clock};

const UAE: &str = "UAE Local Terrorist List";
const XML: &[u8] = b"<list><entry><name>Omar K.</name></entry></list>";
const URL: &str = "https://example.org/uae-list";

async fn setup() -> (Booted, SanctionsList) {
    let t = boot_custom(WlParties::new().screening(true), true, |app| app).await;
    seed::lists(t.app.state()).await.unwrap();
    set_dubai_clock(&t, 5, 10);
    let mut pdf = SanctionsList {
        name: "Local list (PDF)".into(),
        source: "https://example.org".into(),
        format: ListFormat::Pdf,
        status: ListStatus::Current,
        ..SanctionsList::default()
    };
    pdf.insert(t.db()).await.unwrap();
    let uae = SanctionsList::all(t.db())
        .await
        .unwrap()
        .into_iter()
        .find(|l| l.name == UAE)
        .unwrap();
    (t, uae)
}

async fn upload(
    t: &Booted,
    list: &SanctionsList,
    date: &str,
    url: &str,
) -> renox::testing::TestResponse {
    t.app
        .htmx()
        .post_multipart(
            &format!("/parties/screening/lists/{}/upload", list.id),
            &[("source_url", url), ("list_date", date)],
            &[("file", "uae.xml", XML)],
        )
        .await
}

async fn check(t: &Booted, party_id: i64, fields: &[(&str, &str)]) -> renox::testing::TestResponse {
    t.app
        .htmx()
        .post(&format!("/parties/{party_id}/manual-check"), fields)
        .await
}

#[renox::test]
async fn the_upload_button_is_for_screening_review_only() {
    let (t, uae) = setup().await;
    let button = format!("upload-list-{}", uae.id);

    let reviewer = t
        .user("Mariam", &[perm::VIEW, perm::SCREENING_REVIEW])
        .await;
    t.app.acting_as(&reviewer);
    t.app
        .get("/parties/screening")
        .await
        .assert_ok()
        .assert_see("Upload official file")
        .assert_see(&button);

    for (name, grant) in [
        ("Rana", perm::SCREENING_RUN),
        ("Blake", perm::BLOCK_REASON_VIEW),
    ] {
        let user = t.user(name, &[grant]).await;
        t.app.acting_as(&user);
        t.app
            .get("/parties/screening")
            .await
            .assert_ok()
            .assert_dont_see("Upload official file");
        upload(&t, &uae, "2026-11-03", URL).await.assert_status(403);
    }
}

#[renox::test]
async fn an_upload_makes_the_list_current_and_shows_its_source() {
    let (t, uae) = setup().await;
    let user = t
        .user("Mariam", &[perm::VIEW, perm::SCREENING_REVIEW])
        .await;
    t.app.acting_as(&user);

    upload(&t, &uae, "2026-13-45", URL).await.assert_status(422);
    upload(&t, &uae, "2026-11-03", "").await.assert_status(422);
    upload(&t, &uae, "2026-11-03", URL).await.assert_ok();

    let list = SanctionsList::find(t.db(), uae.id).await.unwrap().unwrap();
    assert_eq!(list.status, ListStatus::Current);
    assert_eq!(list.update_source, Some(UpdateSource::Manual));
    assert_eq!(list.updated_by, Some(user.id));
    t.app
        .get("/parties/screening")
        .await
        .assert_ok()
        .assert_see("Manual upload")
        .assert_see(URL)
        .assert_see("2026-11-03")
        .assert_see("Mariam");
    // Current now: a second upload is refused.
    upload(&t, &uae, "2026-11-04", URL).await.assert_status(422);
}

#[renox::test]
async fn the_manual_check_action_and_route_need_screening_review() {
    let (t, _) = setup().await;
    let party = t.party("Al Noor Trading LLC").await;
    let pdf = SanctionsList::all(t.db())
        .await
        .unwrap()
        .into_iter()
        .find(|l| l.format == ListFormat::Pdf)
        .unwrap();
    let path = format!("/parties/{}", party.id);
    let id = pdf.id.to_string();

    let plain = t.user("Pia", &[perm::VIEW, perm::BLOCK_REASON_VIEW]).await;
    t.app.acting_as(&plain);
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_dont_see("Record manual check");
    check(
        &t,
        party.id,
        &[
            ("list_id", &id),
            ("list_date", "2026-11-03"),
            ("result", "no_match"),
        ],
    )
    .await
    .assert_status(403);
    assert!(
        Screening::for_party(t.db(), party.id)
            .await
            .unwrap()
            .is_empty()
    );

    let mlro = t
        .user("Mariam", &[perm::VIEW, perm::SCREENING_REVIEW])
        .await;
    t.app.acting_as(&mlro);
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("Record manual check")
        .assert_see("Local list (PDF)");
    // A candidate without a list entry is refused in the sheet.
    check(
        &t,
        party.id,
        &[
            ("list_id", &id),
            ("list_date", "2026-11-03"),
            ("result", "candidate"),
            ("name", "Omar K."),
        ],
    )
    .await
    .assert_status(422);
    check(
        &t,
        party.id,
        &[
            ("list_id", &id),
            ("list_date", "2026-11-03"),
            ("result", "no_match"),
        ],
    )
    .await
    .assert_ok();
    let runs = Screening::for_party(t.db(), party.id).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].reviewer_user, Some(mlro.id));
}
