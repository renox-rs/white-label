//! The `upload_list_file` service (wl-parties §5.4, issue #42 part 2): the
//! permission, stale-only, dates, storage, the transaction, the upload log
//! and the event. A test-only route calls it as the signed-in user
//! (`acting_as`). The full acceptance checks are in part 5.

mod support;

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_parties::list_upload::UploadError;
use wl_parties::{
    ListStatus, SanctionsList, SanctionsListUpdated, SanctionsListUpload, UpdateSource,
    UploadOutcome, UploadRequest, UploadResult, WlParties, permissions as perm, seed,
    upload_list_file,
};

use support::{Booted, boot_custom, set_dubai_clock};

const UAE: &str = "UAE Local Terrorist List";
const XML: &[u8] = b"<list><entry><name>Omar K.</name></entry></list>";
const URL: &str = "https://example.org/uae-list";

#[derive(Deserialize, Serialize, Validate)]
struct UploadForm {
    #[validate(required)]
    file: Option<Upload>,
    source_url: String,
    list_date: String,
}

async fn upload_route(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Valid(form): Valid<UploadForm>,
) -> Result<Response> {
    let request = UploadRequest {
        file: form.file.expect("a file"),
        source_url: form.source_url,
        list_date: form.list_date,
    };
    match upload_list_file(&state, &user, id, request).await {
        Ok(UploadResult::NewVersion(v)) => Ok(format!("new:{v}").into_response()),
        Ok(UploadResult::SameVersion) => Ok("same".into_response()),
        Err(UploadError::Forbidden) => Ok(StatusCode::FORBIDDEN.into_response()),
        Err(UploadError::NotFound) => Ok(StatusCode::NOT_FOUND.into_response()),
        Err(UploadError::NotStale) => Ok(StatusCode::CONFLICT.into_response()),
        Err(UploadError::Refused(why)) => {
            Ok((StatusCode::UNPROCESSABLE_ENTITY, why.to_string()).into_response())
        }
        Err(UploadError::Failed(e)) => Err(e),
    }
}

struct UploadRoute;

impl Module for UploadRoute {
    fn name(&self) -> &'static str {
        "upload-route"
    }

    fn routes(&self) -> Routes {
        Routes::new().post("/t/lists/{id}/upload", upload_route)
    }
}

async fn setup(module: WlParties) -> Booted {
    let t = boot_custom(module.screening(true), false, |app| app.module(UploadRoute)).await;
    seed::lists(t.app.state()).await.unwrap();
    // Today is 2026-11-05 in Dubai.
    set_dubai_clock(&t, 5, 10);
    t.app.fake_events();
    t
}

async fn reviewer(t: &Booted) -> User {
    t.user("Mariam", &[perm::VIEW, perm::SCREENING_REVIEW])
        .await
}

async fn uae(t: &Booted) -> SanctionsList {
    SanctionsList::all(t.db())
        .await
        .unwrap()
        .into_iter()
        .find(|l| l.name == UAE)
        .unwrap()
}

async fn post(
    t: &Booted,
    user: &User,
    list: &SanctionsList,
    date: &str,
    url: &str,
    bytes: &[u8],
) -> renox::testing::TestResponse {
    t.app.acting_as(user);
    t.app
        .post_multipart(
            &format!("/t/lists/{}/upload", list.id),
            &[("source_url", url), ("list_date", date)],
            &[("file", "uae.xml", bytes)],
        )
        .await
}

async fn log(t: &Booted, list: &SanctionsList) -> Vec<SanctionsListUpload> {
    SanctionsListUpload::for_list(t.db(), list.id)
        .await
        .unwrap()
}

#[renox::test]
async fn a_stale_list_takes_a_valid_file_and_becomes_current() {
    let t = setup(WlParties::new()).await;
    let user = reviewer(&t).await;
    let list = uae(&t).await;
    assert_eq!(list.status, ListStatus::Stale);

    post(&t, &user, &list, "2026-11-03", URL, XML)
        .await
        .assert_ok()
        .assert_see("new:2026-11-03");

    let after = uae(&t).await;
    assert_eq!(after.status, ListStatus::Current);
    assert_eq!(after.version.as_deref(), Some("2026-11-03"));
    assert_eq!(after.update_source, Some(UpdateSource::Manual));
    assert_eq!(after.updated_by, Some(user.id));
    assert_eq!(after.source_url.as_deref(), Some(URL));
    assert!(after.stale_since.is_none() && after.last_error.is_none());
    assert!(after.last_updated_at.is_some());
    assert_eq!(t.app.emitted::<SanctionsListUpdated>().len(), 1);
    t.app
        .assert_emitted::<SanctionsListUpdated>(|e| e.version == "2026-11-03" && e.list == list.id);

    let rows = log(&t, &list).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, UploadOutcome::Accepted);
    assert_eq!(rows[0].uploaded_by, user.id);
    assert_eq!(rows[0].list_date.as_deref(), Some("2026-11-03"));
    assert_eq!(rows[0].byte_size, XML.len() as i64);
    let key = rows[0].file_ref.clone().unwrap();
    assert!(key.starts_with(&format!("parties/lists/{}/uploads/", list.id)));
    assert!(t.app.state().storage.exists(&key).await.unwrap());
}

#[renox::test]
async fn an_unreadable_file_is_refused_and_logged_without_a_change() {
    let t = setup(WlParties::new()).await;
    let user = reviewer(&t).await;
    let list = uae(&t).await;

    let res = post(&t, &user, &list, "2026-11-03", URL, b"<list><entry></list>").await;
    res.assert_status(422);
    res.assert_see("the file cannot be read");
    post(&t, &user, &list, "2026-11-03", URL, b"<list/>")
        .await
        .assert_see("the file has no entries");

    let after = uae(&t).await;
    assert_eq!(after.status, ListStatus::Stale);
    assert!(after.version.is_none() && after.updated_by.is_none());
    assert!(t.app.emitted::<SanctionsListUpdated>().is_empty());
    let rows = log(&t, &list).await;
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.outcome == UploadOutcome::Refused));
    assert!(rows.iter().all(|r| r.file_ref.is_none()));
    assert!(rows[1].refusal_reason.as_deref().unwrap().contains("read"));
}

#[renox::test]
async fn an_older_date_is_refused_and_the_same_date_is_the_same_version() {
    let t = setup(WlParties::new()).await;
    let user = reviewer(&t).await;
    let list = uae(&t).await;
    post(&t, &user, &list, "2026-11-03", URL, XML)
        .await
        .assert_ok();
    // The list goes stale again (a failed download).
    let mut stale = uae(&t).await;
    stale.status = ListStatus::Stale;
    stale.stale_since = Some(renox::db::now());
    stale.last_error = Some("source unreachable".into());
    stale.save(t.db()).await.unwrap();

    let res = post(&t, &user, &list, "2026-11-02", URL, XML).await;
    res.assert_status(422);
    res.assert_see("older than the current version");
    assert_eq!(uae(&t).await.status, ListStatus::Stale);

    post(&t, &user, &list, "2026-11-03", URL, XML)
        .await
        .assert_ok()
        .assert_see("same");
    let after = uae(&t).await;
    assert_eq!(after.status, ListStatus::Current);
    assert_eq!(after.version.as_deref(), Some("2026-11-03"));
    assert!(after.stale_since.is_none() && after.last_error.is_none());
    // Only the first upload emitted.
    assert_eq!(t.app.emitted::<SanctionsListUpdated>().len(), 1);
    let outcomes: Vec<_> = log(&t, &list).await.iter().map(|r| r.outcome).collect();
    assert_eq!(
        outcomes,
        [
            UploadOutcome::Accepted,
            UploadOutcome::Refused,
            UploadOutcome::Accepted
        ]
    );
}

#[renox::test]
async fn a_future_date_and_missing_details_are_refused() {
    let t = setup(WlParties::new()).await;
    let user = reviewer(&t).await;
    let list = uae(&t).await;
    post(&t, &user, &list, "2026-11-06", URL, XML)
        .await
        .assert_status(422)
        .assert_see("future");
    post(&t, &user, &list, "", URL, XML)
        .await
        .assert_see("list date is required");
    post(&t, &user, &list, "2026-11-03", "", XML)
        .await
        .assert_see("source URL is required");
    assert_eq!(uae(&t).await.status, ListStatus::Stale);
    assert!(t.app.emitted::<SanctionsListUpdated>().is_empty());
    assert_eq!(log(&t, &list).await.len(), 3);
}

#[renox::test]
async fn a_current_list_refuses_an_upload() {
    let t = setup(WlParties::new()).await;
    let user = reviewer(&t).await;
    let list = uae(&t).await;
    post(&t, &user, &list, "2026-11-03", URL, XML)
        .await
        .assert_ok();
    post(&t, &user, &list, "2026-11-04", URL, XML)
        .await
        .assert_status(409);
    assert_eq!(uae(&t).await.version.as_deref(), Some("2026-11-03"));
    let rows = log(&t, &list).await;
    assert_eq!(rows[0].outcome, UploadOutcome::Refused);
    assert_eq!(
        rows[0].refusal_reason.as_deref(),
        Some("the list is current; no upload needed")
    );
}

#[renox::test]
async fn a_format_without_a_parser_and_a_disabled_setting_refuse() {
    let t = setup(WlParties::new()).await;
    let user = reviewer(&t).await;
    let mut pdf = SanctionsList {
        name: "Local list (PDF)".into(),
        source: "Authority".into(),
        format: wl_parties::ListFormat::Pdf,
        ..SanctionsList::default()
    };
    pdf.insert(t.db()).await.unwrap();
    post(&t, &user, &pdf, "2026-11-03", URL, XML)
        .await
        .assert_status(422)
        .assert_see("Record manual check");
    assert_eq!(log(&t, &pdf).await.len(), 1);

    let off = setup(WlParties::new().manual_upload(false)).await;
    let user = reviewer(&off).await;
    let list = uae(&off).await;
    post(&off, &user, &list, "2026-11-03", URL, XML)
        .await
        .assert_status(422)
        .assert_see("turned off");
    assert_eq!(uae(&off).await.status, ListStatus::Stale);
}

#[renox::test]
async fn without_screening_review_nothing_happens_and_nothing_is_logged() {
    let t = setup(WlParties::new()).await;
    let list = uae(&t).await;
    let viewer = t
        .user("Bilal", &[perm::VIEW, perm::BLOCK_REASON_VIEW])
        .await;
    post(&t, &viewer, &list, "2026-11-03", URL, XML)
        .await
        .assert_status(403);
    assert_eq!(uae(&t).await.status, ListStatus::Stale);
    assert!(log(&t, &list).await.is_empty());
    assert!(t.app.emitted::<SanctionsListUpdated>().is_empty());
}

#[renox::test]
async fn an_unknown_list_is_not_found() {
    let t = setup(WlParties::new()).await;
    let user = reviewer(&t).await;
    let ghost = SanctionsList {
        id: 99_999,
        ..SanctionsList::default()
    };
    post(&t, &user, &ghost, "2026-11-03", URL, XML)
        .await
        .assert_status(404);
}
