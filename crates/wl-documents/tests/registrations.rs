//! BL-DOC-10 (#54): the seeded kinds for registrations and certifications:
//! on or off per vertical, with English and Arabic names, expiry and
//! reminders to the responsible role, and no `blocks`.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_documents::documents::{AttachReason, NewDocument};
use wl_documents::expiry::{self, ReminderLog};
use wl_documents::registrations::{self, HALAL, ISO, OTHER, PRODUCT, TRADEMARK, VerticalSettings};
use wl_documents::{
    Document, DocumentKind, DocumentStatus, Documents, Sensitivity, permissions as perm,
};
use wl_kernel::{ClientConfig, Subject};

use support::{Booted, boot, day};

fn config(app: &str, owner: Option<&str>) -> ClientConfig {
    let roles = owner.map_or_else(String::new, |o| format!("\n[roles]\nowner = \"{o}\"\n"));
    ClientConfig::parse(&format!(
        "[client]\nname = \"Acme LLC\"\napp = \"{app}\"\n{roles}"
    ))
    .expect("a valid client.toml")
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn entity() -> Subject {
    Subject::new(registrations::LEGAL_ENTITY_SUBJECT, 1)
}

/// Attaches a numbered document of `kind` expiring on `expiry` and has
/// `reviewer` accept it: these kinds have a number.
async fn accepted(
    t: &Booted,
    reviewer: &User,
    subject: Subject,
    kind: &str,
    expiry: NaiveDate,
) -> Document {
    let mut new = NewDocument::new(subject, kind, support::pdf("scan.pdf")).expires_on(expiry);
    new.number = Some("REG-2026-001".into());
    let document = Documents::attach(t.state(), None, new)
        .await
        .unwrap()
        .expect("the document is attached");
    t.accept(reviewer, &document).await
}

async fn install(t: &Booted, app: &str, owner: Option<&str>, settings: &VerticalSettings) {
    registrations::install_for(t.db(), &config(app, owner), settings)
        .await
        .expect("the registration kinds install");
}

/// AC1: a new 06 client has Trademark registration, Halal certificate and
/// Other certification on, with English and Arabic names, class `normal`,
/// not visible in the portal and without blocks; ISO certificate is
/// available but off.
#[renox::test]
async fn docexp01_a_new_06_client_has_three_kinds_on_and_iso_off() {
    let t = boot().await;
    install(&t, "fnb", None, &VerticalSettings::new()).await;

    for key in [TRADEMARK, HALAL, OTHER] {
        let kind = t.kind(key).await;
        assert!(kind.enabled, "{key} is on");
        assert!(!kind.name_en.is_empty() && !kind.name_ar.is_empty());
        assert_eq!(kind.sensitivity, Sensitivity::Normal);
        assert!(!kind.portal_upload);
        assert!(kind.blocks(t.db()).await.unwrap().is_empty());
        assert!(!kind.is_blocking(t.db()).await.unwrap());
        assert!(!kind.legal_block);
    }
    let iso = t.kind(ISO).await;
    assert!(!iso.enabled);
    assert_eq!(iso.name_en, "ISO certificate");
    assert!(!t.kind(PRODUCT).await.enabled);
    assert_eq!(t.kind(TRADEMARK).await.reminder_offsets.0, [180, 90, 30]);
    assert_eq!(t.kind(HALAL).await.reminder_offsets.0, [90, 30, 7]);

    // Seeding again keeps what the client changed.
    wl_documents::set_enabled(t.db(), ISO, true).await.unwrap();
    install(&t, "fnb", None, &VerticalSettings::new()).await;
    assert!(t.kind(ISO).await.enabled);
}

/// A kind that is off takes no new document; the ones it has stay.
#[renox::test]
async fn docexp01_a_kind_that_is_off_takes_no_new_documents() {
    let t = boot().await;
    install(&t, "fnb", None, &VerticalSettings::new()).await;
    let reviewer = t.reviewer().await;

    let refused = Documents::attach(
        t.state(),
        None,
        NewDocument::new(entity(), ISO, support::pdf("iso.pdf")),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(refused.reason, AttachReason::KindOff);

    let held = accepted(&t, &reviewer, entity(), TRADEMARK, day(200)).await;
    wl_documents::set_enabled(t.db(), TRADEMARK, false)
        .await
        .unwrap();
    let refused = Documents::attach(
        t.state(),
        None,
        NewDocument::new(entity(), TRADEMARK, support::pdf("tm.pdf")),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(refused.reason, AttachReason::KindOff);
    assert_eq!(t.reload(&held).await.status, DocumentStatus::Accepted);
}

/// AC2: a trademark expiring 2027-06-30 and no vertical override: the
/// holders of the business owner role are reminded on 2027-01-01,
/// 2027-04-01 and 2027-05-31.
#[renox::test]
async fn docexp01_a_trademark_reminds_the_owner_at_180_90_and_30_days() {
    let t = boot().await;
    install(&t, "fnb", Some("md"), &VerticalSettings::new()).await;
    t.roles(&["md"]).await;
    let owner = t.staff("Mona", "md", None).await;
    let reviewer = t.reviewer().await;
    let mark = accepted(&t, &reviewer, entity(), TRADEMARK, date(2027, 6, 30)).await;
    assert_eq!(mark.status, DocumentStatus::Accepted);

    for (night, expected) in [
        (date(2026, 12, 31), 0),
        (date(2027, 1, 1), 1),
        (date(2027, 1, 2), 1),
        (date(2027, 4, 1), 2),
        (date(2027, 5, 30), 2),
        (date(2027, 5, 31), 3),
    ] {
        expiry::run(t.state(), night).await.unwrap();
        assert_eq!(t.notices.to_user(owner.id).len(), expected, "on {night}");
    }
}

/// AC3: an expired Halal certificate on an item refuses no transition: the
/// kind has no blocks, the run opens no renewal task and no expiry notice
/// lists blocked records.
#[renox::test]
async fn docexp01_an_expired_halal_certificate_on_an_item_blocks_nothing() {
    let t = boot().await;
    install(&t, "fnb", None, &VerticalSettings::new()).await;
    let reviewer = t.reviewer().await;
    let item = Subject::new(registrations::ITEM_SUBJECT, 12);
    let halal = accepted(&t, &reviewer, item, HALAL, day(-1)).await;

    let report = expiry::run(t.state(), day(0)).await.unwrap();
    assert_eq!((report.failed, report.tasks), (0, 0), "{report:?}");
    assert!(t.tasks.opened().is_empty());
    let kind = t.kind(HALAL).await;
    assert!(kind.blocks(t.db()).await.unwrap().is_empty());
    assert!(
        wl_documents::blocks_for(t.db(), "ticket.resolve")
            .await
            .unwrap()
            .iter()
            .all(|entry| entry.kind != HALAL)
    );
    assert_eq!(t.reload(&halal).await.status, DocumentStatus::Accepted);
}

/// AC3, through a guarded transition: resolving a fire alarm ticket checks
/// the vendor's Civil Defence approval; an expired Halal certificate on an
/// item and on a supplier changes nothing about the answer, while an
/// expired Civil Defence approval is still refused (and does not name Halal).
#[renox::test]
async fn docexp01_an_expired_halal_certificate_does_not_refuse_a_guarded_transition() {
    let t = boot().await;
    install(&t, "fnb", None, &VerticalSettings::new()).await;
    t.install(&[support::civil_defence()]).await;
    let reviewer = t.reviewer().await;
    let item = Subject::new(registrations::ITEM_SUBJECT, 12);
    accepted(&t, &reviewer, item, HALAL, day(-30)).await;
    accepted(
        &t,
        &reviewer,
        Subject::new("parties.party", 31),
        HALAL,
        day(-1),
    )
    .await;
    let approval = t
        .accepted(
            &reviewer,
            support::safefire(),
            "civil_defence_approval",
            Some(day(10)),
        )
        .await;
    let guard = wl_kernel::documents::guard(t.state());
    let today = day(0);

    let allowed = guard
        .check(t.state(), "ticket.resolve", support::FIRE_TICKET, today)
        .await
        .expect("the check runs");
    assert!(
        allowed.is_ok(),
        "the expired Halal certificates block nothing"
    );

    // The control: once the Civil Defence approval has expired the same
    // transition is refused, and the refusal is about that approval only.
    let refused = guard
        .check(t.state(), "ticket.resolve", support::FIRE_TICKET, day(11))
        .await
        .expect("the check runs")
        .expect_err("an expired Civil Defence approval refuses");
    assert!(
        refused.message().contains("Civil Defence approval"),
        "{}",
        refused.message()
    );
    assert!(!refused.message().to_lowercase().contains("halal"));
    assert_eq!(t.reload(&approval).await.status, DocumentStatus::Accepted);
}

/// AC4: a 06 client whose vertical seeds the storekeeper as the Halal
/// certificate's responsible role: a certificate on supplier "Fresh Meat
/// Trading" covering "Chicken breast" reminds the storekeeper 90, 30 and 7
/// days before expiry, and not the owner.
#[renox::test]
async fn docexp01_a_vertical_role_is_reminded_for_a_supplier_certificate() {
    let t = boot().await;
    let settings = VerticalSettings::new().role(HALAL, "storekeeper");
    install(&t, "fnb", Some("md"), &settings).await;
    assert_eq!(
        t.kind(HALAL).await.responsible_role.as_deref(),
        Some("storekeeper")
    );
    t.roles(&["md", "storekeeper"]).await;
    let owner = t.staff("Mona", "md", None).await;
    let store = t.staff("Sami", "storekeeper", None).await;
    let reviewer = t.reviewer().await;

    let supplier = Subject::new("parties.party", 31);
    let mut new = NewDocument::new(supplier, HALAL, support::pdf("halal.pdf"))
        .expires_on(day(90))
        .note("Covers: Chicken breast");
    new.number = Some("HAL-2026-77".into());
    new.issuing_authority = Some("Emirates Halal Body".into());
    let document = Documents::attach(t.state(), None, new)
        .await
        .unwrap()
        .expect("a supplier certificate is attached");
    t.accept(&reviewer, &document).await;

    for (night, expected) in [(day(0), 1), (day(60), 2), (day(83), 3)] {
        expiry::run(t.state(), night).await.unwrap();
        assert_eq!(t.notices.to_user(store.id).len(), expected, "{night}");
    }
    assert!(t.notices.to_user(owner.id).is_empty());
}

/// AC5: a 07 client with no override: an expiring ISO certificate reminds
/// the QC inspectors.
#[renox::test]
async fn docexp01_an_iso_certificate_reminds_the_qc_inspectors_in_manufacturing() {
    let t = boot().await;
    let settings = registrations::settings_of("manufacturing");
    install(&t, "manufacturing", Some("md"), &settings).await;
    let iso = t.kind(ISO).await;
    assert!(iso.enabled);
    assert_eq!(iso.responsible_role.as_deref(), Some("qc_inspector"));
    t.roles(&["md", "qc_inspector"]).await;
    let owner = t.staff("Mona", "md", None).await;
    let qc = t.staff("Qadir", "qc_inspector", None).await;
    let reviewer = t.reviewer().await;
    accepted(&t, &reviewer, entity(), ISO, day(30)).await;

    expiry::run(t.state(), day(0)).await.unwrap();
    assert_eq!(t.notices.to_user(qc.id).len(), 1);
    assert!(t.notices.to_user(owner.id).is_empty());
}

/// AC6: correcting a trademark's expiry date from 2027-06-30 to 2027-09-30
/// reschedules its reminders and is audited.
#[renox::test]
async fn docexp01_correcting_the_expiry_date_reschedules_reminders_and_is_audited() {
    let t = boot().await;
    install(&t, "fnb", Some("md"), &VerticalSettings::new()).await;
    t.roles(&["md"]).await;
    let owner = t.staff("Mona", "md", None).await;
    let reviewer = t.reviewer().await;
    let manager = t.user("Omar", &[perm::MANAGE]).await;
    let mark = accepted(&t, &reviewer, entity(), TRADEMARK, date(2027, 6, 30)).await;

    expiry::run(t.state(), date(2027, 1, 1)).await.unwrap();
    assert_eq!(t.notices.to_user(owner.id).len(), 1);
    assert_eq!(
        ReminderLog::offsets_for(t.db(), mark.id).await.unwrap(),
        [180]
    );

    t.app.acting_as(&manager);
    t.app
        .htmx()
        .post(
            &format!("/documents/{}", mark.id),
            &[
                ("title", mark.title.as_str()),
                ("expiry_date", "2027-09-30"),
            ],
        )
        .await
        .assert_ok();
    let edited = t.reload(&mark).await;
    assert_eq!(edited.expiry_date, Some(date(2027, 9, 30)));
    assert!(
        ReminderLog::offsets_for(t.db(), mark.id)
            .await
            .unwrap()
            .is_empty()
    );

    // Nothing is due on the old schedule; the first reminder of the new
    // date comes 180 days before it.
    expiry::run(t.state(), date(2027, 4, 1)).await.unwrap();
    assert_eq!(t.notices.to_user(owner.id).len(), 1);
    expiry::run(t.state(), date(2027, 4, 3)).await.unwrap();
    assert_eq!(t.notices.to_user(owner.id).len(), 2);

    let entries = renox::audit::for_subject(t.db(), Document::TABLE, mark.id, 10)
        .await
        .unwrap();
    let updated = entries
        .iter()
        .find(|e| e.action == "document.updated")
        .expect("the correction is audited");
    assert_eq!(updated.user_id, Some(manager.id));
    assert_eq!(updated.data["old"]["expiry_date"], "2027-06-30");
    assert_eq!(updated.data["new"]["expiry_date"], "2027-09-30");
}

/// The kind page turns a kind on and off for `document_kinds.manage`.
#[renox::test]
async fn docexp01_the_kind_page_turns_a_kind_on() {
    let t = boot().await;
    install(&t, "fnb", None, &VerticalSettings::new()).await;
    let admin = t.kind_admin().await;
    t.app.acting_as(&admin);
    t.app
        .get(&format!("/document-kinds/{ISO}"))
        .await
        .assert_ok()
        .assert_see("Off");
    t.app
        .htmx()
        .post(
            &format!("/document-kinds/{ISO}/enabled"),
            &[("enabled", "1")],
        )
        .await
        .assert_ok();
    assert!(
        DocumentKind::by_key(t.db(), ISO)
            .await
            .unwrap()
            .unwrap()
            .enabled
    );
    t.app
        .htmx()
        .post(
            &format!("/document-kinds/{ISO}/enabled"),
            &[("enabled", "0")],
        )
        .await
        .assert_ok();
    assert!(!t.kind(ISO).await.enabled);

    let nobody = t.user("Nadia", &[perm::VIEW]).await;
    t.app.acting_as(&nobody);
    t.app
        .htmx()
        .post(
            &format!("/document-kinds/{ISO}/enabled"),
            &[("enabled", "1")],
        )
        .await
        .assert_status(403);
}
