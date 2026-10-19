//! Recording the proof of delivery of a statutory notice (#59 AC2, AC3, AC7):
//! the proof page and its multipart `POST`, the permission, the attachment,
//! the task and the event.

mod support;

use renox::prelude::Model;
use wl_kernel::notify::{NoticeStatus, ProofMethod, StatutoryNoticeProofRecorded};
use wl_kernel::tickets::TicketId;
use wl_notify::permissions as perm;
use wl_notify::{StatutoryNotice, notices};

use support::{TICKET_NOTICE, TOWER_A_TICKET, boot_statutory};

const PDF: &[u8] = b"%PDF-1.7 signed by Sara M.";

#[renox::test]
async fn the_proof_of_delivery_is_recorded_from_the_proof_page() {
    let t = boot_statutory().await;
    t.give_up_on_sara(TICKET_NOTICE, TOWER_A_TICKET, "TKT-2026-0042")
        .await;
    t.app.fake_events();
    let notice = StatutoryNotice::query()
        .first(t.db())
        .await
        .unwrap()
        .expect("a notice record");
    let url = format!("/statutory-notices/{}/proof", notice.id);
    let today = wl_kernel::documents::today();
    let day = |offset: i64| {
        (today + renox::chrono::Duration::days(offset))
            .format("%Y-%m-%d")
            .to_string()
    };

    // Permission: 403 for others, 404 for an unknown notice.
    let nobody = t.user("Nadia", &[perm::LOG_VIEW]).await;
    t.app.acting_as(&nobody);
    t.app.get(&url).await.assert_forbidden();
    t.app.get(&format!("{url}/file")).await.assert_forbidden();
    t.app
        .post_multipart(&url, &[("method", "call")], &[])
        .await
        .assert_forbidden();
    let clerk = t.user("Pia", &[perm::RECORD_PROOF]).await;
    t.app.acting_as(&clerk);
    t.app
        .get("/statutory-notices/9999/proof")
        .await
        .assert_not_found();

    // The form offers the type's methods and the banner says not delivered.
    t.app
        .get(&url)
        .await
        .assert_ok()
        .assert_view("wl-notify/notice_proof.html")
        .assert_see("Record proof of delivery")
        .assert_see("Notice not delivered")
        .assert_see("hand delivery")
        .assert_see("registered mail")
        .assert_see("TKT-2026-0042");

    // Refusals store nothing.
    macro_rules! post {
        ($fields:expr, $files:expr $(,)?) => {
            t.app.htmx().post_multipart(&url, $fields, $files)
        };
    }
    post!(&[("method", "courier"), ("delivered_on", &day(0))], &[])
        .await
        .assert_invalid("attachment");
    post!(&[("method", "call"), ("delivered_on", &day(0))], &[])
        .await
        .assert_invalid("note");
    let receipt = [("files", "receipt.pdf", PDF)];
    post!(
        &[("method", "courier"), ("delivered_on", &day(1))],
        &receipt,
    )
    .await
    .assert_invalid("delivered_on");
    post!(
        &[("method", "courier"), ("delivered_on", &day(-1))],
        &receipt,
    )
    .await
    .assert_invalid("delivered_on");
    post!(
        &[("method", "carrier_pigeon"), ("delivered_on", &day(0))],
        &receipt,
    )
    .await
    .assert_invalid("method");
    post!(
        &[("method", "courier"), ("delivered_on", &day(0))],
        &[("files", "virus.exe", b"MZ")],
    )
    .await
    .assert_invalid("attachment");
    let stored = StatutoryNotice::find(t.db(), notice.id)
        .await
        .unwrap()
        .unwrap();
    assert!(!stored.has_proof() && stored.attachment_key.is_none());
    assert!(t.tasks.completed().is_empty());
    assert_eq!(
        notices::status(t.db(), &notice.subject(), &notice.type_key)
            .await
            .unwrap(),
        NoticeStatus::NotDelivered
    );

    // The proof: hand delivery today with the signed note.
    post!(
        &[
            ("method", "hand_delivery"),
            ("delivered_on", &day(0)),
            ("note", "Handed to Sara M. at the door"),
        ],
        &[("files", "signed.pdf", PDF)],
    )
    .await
    .assert_ok();
    let stored = StatutoryNotice::find(t.db(), notice.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.proof_method, Some(ProofMethod::HandDelivery));
    assert_eq!(stored.delivered_on, Some(today));
    assert_eq!(stored.recorded_by, Some(clerk.id));
    assert_eq!(stored.attachment_name.as_deref(), Some("signed.pdf"));
    assert!(
        stored
            .attachment_key
            .as_deref()
            .is_some_and(|key| key.starts_with(&format!("notify/proofs/{}/", notice.id)))
    );
    assert_eq!(
        notices::status(t.db(), &notice.subject(), &notice.type_key)
            .await
            .unwrap(),
        NoticeStatus::ProofRecorded {
            delivered_on: today
        }
    );
    assert!(
        notices::undelivered(t.db(), &notice.subject())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        t.tasks.completed(),
        [(
            TicketId(stored.ticket_id.unwrap()),
            "Proof recorded".to_owned()
        )]
    );
    let events = t.app.emitted::<StatutoryNoticeProofRecorded>();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].notice, notice.id);
    assert_eq!(events[0].method, ProofMethod::HandDelivery);
    assert_eq!(events[0].delivered_on, today);

    // A second proof is a conflict and changes nothing.
    t.app
        .post_multipart(
            &url,
            &[
                ("method", "call"),
                ("delivered_on", &day(0)),
                ("note", "again"),
            ],
            &[],
        )
        .await
        .assert_status(409);
    assert_eq!(t.app.emitted::<StatutoryNoticeProofRecorded>().len(), 1);

    // The page reads the proof; the attachment goes to holders only.
    t.app
        .get(&url)
        .await
        .assert_ok()
        .assert_see("Proof already recorded")
        .assert_see("Download the attachment")
        .assert_dont_see("Notice not delivered");
    t.app
        .get(&format!("{url}/file"))
        .await
        .assert_ok()
        .assert_see("signed by Sara M.");
    t.app.acting_as(&nobody);
    t.app.get(&format!("{url}/file")).await.assert_forbidden();
}
