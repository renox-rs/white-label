//! The manual list upload base (wl-parties §5.4, issue #42 part 1): the new
//! columns, the upload log and the settings.

mod support;

use renox::prelude::Model;

use wl_parties::list_upload::UploadSettings;
use wl_parties::{
    ListFormat, ListParsers, SanctionsList, SanctionsListUpload, UploadOutcome, WlParties, seed,
};

use support::{boot, boot_with};

#[renox::test]
async fn a_seeded_list_is_xml_with_no_uploader_and_no_source_url() {
    let t = boot_with(WlParties::new().screening(true), false).await;
    seed::lists(t.app.state()).await.unwrap();
    for list in SanctionsList::all(t.db()).await.unwrap() {
        assert_eq!(list.format, ListFormat::Xml);
        assert!(list.updated_by.is_none() && list.source_url.is_none());
    }
}

#[renox::test]
async fn a_list_keeps_its_format_uploader_and_source_url() {
    let t = boot_with(WlParties::new().screening(true), false).await;
    let mut list = SanctionsList {
        name: "Local list (PDF)".into(),
        source: "Authority".into(),
        format: ListFormat::Pdf,
        updated_by: Some(7),
        source_url: Some("https://example.org/list".into()),
        ..SanctionsList::default()
    };
    list.insert(t.db()).await.unwrap();
    let back = SanctionsList::all(t.db()).await.unwrap().remove(0);
    assert_eq!(back.format, ListFormat::Pdf);
    assert_eq!(back.updated_by, Some(7));
    assert_eq!(back.source_url.as_deref(), Some("https://example.org/list"));
}

#[renox::test]
async fn accepted_and_refused_uploads_are_logged_newest_first() {
    let t = boot_with(WlParties::new().screening(true), false).await;
    let mut list = SanctionsList {
        name: "UN".into(),
        source: "UN".into(),
        ..SanctionsList::default()
    };
    list.insert(t.db()).await.unwrap();
    let refused = SanctionsListUpload {
        list_id: list.id,
        uploaded_by: 3,
        uploaded_at: renox::db::now(),
        file_name: "bad.xml".into(),
        byte_size: 12,
        outcome: UploadOutcome::Refused,
        refusal_reason: Some("the file has no entries".into()),
        ..SanctionsListUpload::default()
    };
    let accepted = SanctionsListUpload {
        source_url: Some("https://example.org/list".into()),
        list_date: Some("2026-11-03".into()),
        file_ref: Some("parties/lists/1/list.xml".into()),
        outcome: UploadOutcome::Accepted,
        refusal_reason: None,
        file_name: "list.xml".into(),
        ..refused.clone()
    };
    let mut refused = refused;
    refused.insert(t.db()).await.unwrap();
    let mut accepted = accepted;
    accepted.insert(t.db()).await.unwrap();
    let log = SanctionsListUpload::for_list(t.db(), list.id)
        .await
        .unwrap();
    let names: Vec<_> = log.iter().map(|u| u.file_name.as_str()).collect();
    assert_eq!(names, ["list.xml", "bad.xml"]);
    assert_eq!(log[0].outcome, UploadOutcome::Accepted);
    assert_eq!(log[1].outcome, UploadOutcome::Refused);
    assert!(log[1].source_url.is_none() && log[1].list_date.is_none());
    assert!(
        SanctionsListUpload::for_list(t.db(), list.id + 1)
            .await
            .unwrap()
            .is_empty()
    );
}

#[renox::test]
async fn manual_upload_is_provided_on_unless_the_app_turns_it_off() {
    let on = boot().await;
    assert!(UploadSettings::of(on.app.state()).enabled);
    let off = boot_with(WlParties::new().manual_upload(false), false).await;
    assert!(!UploadSettings::of(off.app.state()).enabled);
    assert!(off.app.state().provided::<ListParsers>().is_some());
}
