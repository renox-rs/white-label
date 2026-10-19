//! BL-DOC-09 (#53), `DOC.RUL.04`: SOP acknowledgements are requested when a
//! version is published and when a reader joins or takes a role, reminded and
//! escalated daily, replaced by a newer version, cancelled when the reader
//! leaves, and recorded by the reader (own session) or in person; the kernel
//! contract `SopAcknowledgements` answers the onboarding.

mod support;

use renox::prelude::*;
use wl_documents::acknowledgements::run::run;
use wl_documents::{AckStatus, Document, Sop, SopAcknowledgement, sops::kind_seeds, permissions as perm};
use wl_kernel::documents::{SopStaff, sop_acknowledgements};
use wl_kernel::notify::{NotifyRecipient, NotifyRequest};
use wl_kernel::workforce::{
    EmployeeAssignmentChanged, EmployeeExited, EmployeeId, OnboardingStarted, OnboardingId,
};
use wl_kernel::{PartyId, SopId, Subject};

use support::{Booted, Options, boot_options, day, site_worker};

const REQUESTED: &str = "documents.sop_ack_requested";
const REQUESTED_IN_PERSON: &str = "documents.sop_ack_requested_in_person";
const REMINDER: &str = "documents.sop_ack_reminder";
const ESCALATED: &str = "documents.sop_ack_escalated";
const IN_PERSON_RECORDED: &str = "documents.sop_ack_in_person_recorded";

const ALI: i64 = 101;
const SUNIL: i64 = 102;
const OMAR: i64 = 103;
const BILAL: i64 = 104;

/// 12 site workers read SOP-HSE-01: Ali and Bilal have accounts, Sunil has
/// none, Omar is on leave until `day(30)`; Mira manages them all, Hana is the
/// HSE officer (owner role) who manages SOPs.
struct World {
    t: Booted,
    hana: User,
    mira: User,
    ali: User,
    bilal: User,
    sop: Sop,
}

async fn world() -> World {
    let t = boot_options(Options {
        fake_events: false,
        ..Options::default()
    })
    .await;
    t.install(&kind_seeds()).await;
    t.roles(&["hse_officer"]).await;
    let hana = t.user("Hana", &[perm::SOPS_MANAGE, perm::VIEW]).await;
    hana.assign_role(t.db(), "hse_officer").await.unwrap();
    let mira = t.user("Mira", &[]).await;
    let ali = t.user("Ali", &[]).await;
    let bilal = t.user("Bilal", &[]).await;
    for id in 101..=112 {
        let account = match id {
            ALI => Some(ali.id),
            BILAL => Some(bilal.id),
            _ => None,
        };
        let mut worker = site_worker(id, account);
        worker.managers = vec![mira.id];
        if id == OMAR {
            worker.on_leave = true;
            worker.returns_on = Some(day(30));
        }
        t.staff.add(worker);
    }
    t.app.acting_as(&hana);
    let created = t
        .app
        .post(
            "/test/sops",
            &[
                ("code", "SOP-HSE-01"),
                ("title", "Working at height"),
                ("owner_role", "hse_officer"),
            ],
        )
        .await;
    created.assert_ok();
    let sop = Sop::find_or_404(t.db(), created.text().parse().unwrap())
        .await
        .unwrap();
    t.app
        .post(
            &format!("/test/sops/{}/readers", sop.id),
            &[("readers", "site_worker@sites:1")],
        )
        .await
        .assert_ok();
    World {
        t,
        hana,
        mira,
        ali,
        bilal,
        sop,
    }
}

impl World {
    /// Uploads a version as Hana and has her accept it (the owner role).
    async fn publish(&self) -> Document {
        self.t.app.acting_as(&self.hana);
        let uploaded = self
            .t
            .app
            .post(
                &format!("/test/sops/{}/versions", self.sop.id),
                &[("file_name", "working-at-height.pdf")],
            )
            .await;
        uploaded.assert_ok();
        let version = Document::find_or_404(self.t.db(), uploaded.text().parse().unwrap())
            .await
            .unwrap();
        self.t
            .app
            .post(&format!("/test/sop-versions/{}/accept", version.id), &[])
            .await
            .assert_ok();
        self.t.reload(&version).await
    }

    /// The newest acknowledgement of `employee` for `version`.
    async fn ack(&self, employee: i64, version: &Document) -> SopAcknowledgement {
        SopAcknowledgement::query()
            .where_(SopAcknowledgement::EMPLOYEE_ID.eq(employee))
            .where_(SopAcknowledgement::DOCUMENT_ID.eq(version.id))
            .order_by_desc(SopAcknowledgement::ID)
            .first(self.t.db())
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("no acknowledgement of {employee}"))
    }

    async fn reload(&self, ack: &SopAcknowledgement) -> SopAcknowledgement {
        SopAcknowledgement::find_or_404(self.t.db(), ack.id)
            .await
            .unwrap()
    }

    /// How many acknowledgements are in `status`.
    async fn count(&self, status: AckStatus) -> u64 {
        SopAcknowledgement::query()
            .where_(SopAcknowledgement::STATUS.eq(status))
            .count(self.t.db())
            .await
            .unwrap()
    }

    /// The reader opens `document` for `ack` in their session.
    async fn open(&self, reader: &User, ack: &SopAcknowledgement, document: &Document) {
        self.t.app.acting_as(reader);
        self.t
            .app
            .get(&format!(
                "/documents/acknowledgements/{}/files/{}",
                ack.id, document.id
            ))
            .await
            .assert_ok();
    }

    /// The reader confirms `ack`.
    async fn confirm(&self, reader: &User, ack: &SopAcknowledgement) -> renox::testing::TestResponse {
        self.t.app.acting_as(reader);
        self.t
            .app
            .htmx()
            .post(
                &format!("/documents/acknowledgements/{}/acknowledge", ack.id),
                &[],
            )
            .await
    }

    /// The messages of `type_key`.
    fn sent(&self, type_key: &str) -> Vec<NotifyRequest> {
        self.t
            .notices
            .sent()
            .into_iter()
            .filter(|r| r.type_key == type_key)
            .collect()
    }
}

/// The users a request is for, ordered.
fn users(request: &NotifyRequest) -> Vec<i64> {
    let mut ids: Vec<i64> = request
        .recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect();
    ids.sort_unstable();
    ids
}

/// Whether `request` is about `ack`.
fn about(request: &NotifyRequest, ack: &SopAcknowledgement) -> bool {
    request.subject == Subject::new("documents.sop_acknowledgement", ack.id)
}

fn about_all<'a>(
    requests: &'a [NotifyRequest],
    ack: &'a SopAcknowledgement,
) -> Vec<&'a NotifyRequest> {
    requests.iter().filter(|r| about(r, ack)).collect()
}

/// AC1: 12 site workers and version 1 accepted: 12 pending acknowledgements,
/// 11 due in 7 days (Omar's follows his leave), each reader told on their own
/// channel.
#[renox::test]
async fn doc_rul_04_publishing_asks_every_required_reader() {
    let w = world().await;
    assert_eq!(w.count(AckStatus::Pending).await, 0, "a draft SOP asks nobody");
    let v1 = w.publish().await;
    assert_eq!(w.count(AckStatus::Pending).await, 12);
    for id in 101..=112 {
        let ack = w.ack(id, &v1).await;
        assert_eq!(ack.status, AckStatus::Pending);
        assert_eq!(ack.version_no, 1);
        assert_eq!(ack.requested_on, day(0));
        assert_eq!(ack.sop_id, w.sop.id);
        if id != OMAR {
            assert_eq!(ack.due_date, day(7), "reader {id}");
        }
    }
    // Ali has an account: told in the staff inbox. Sunil has none: told by SMS
    // where a number is on record.
    let ali = w.ack(ALI, &v1).await;
    let told = w.sent(REQUESTED);
    assert_eq!(told.len(), 2, "Ali and Bilal have accounts");
    assert!(
        told.iter()
            .any(|r| about(r, &ali) && users(r) == [w.ali.id]),
        "{told:?}"
    );
    let sunil = w.ack(SUNIL, &v1).await;
    let sms = w.sent(REQUESTED_IN_PERSON);
    assert_eq!(sms.len(), 10);
    assert!(sms.iter().any(|r| {
        about(r, &sunil)
            && r.recipients == [NotifyRecipient::Party { id: PartyId(1000 + SUNIL) }]
    }));
    // A second run of the rule on the same day asks nobody again.
    let report = run(w.t.state(), day(0)).await.unwrap();
    assert_eq!(report.requested, 0);
    assert_eq!(w.count(AckStatus::Pending).await, 12);
}

/// AC2: Ali opens version 1 and confirms: `acknowledged` with method
/// `click-to-accept`, the language and checksum of the file, and
/// `SopAcknowledged`. Confirming before opening, or for someone else, is
/// refused.
#[renox::test]
async fn doc_rul_04_the_reader_acknowledges_in_their_own_session() {
    let w = world().await;
    let v1 = w.publish().await;
    let ack = w.ack(ALI, &v1).await;

    // Not opened yet.
    w.confirm(&w.ali, &ack).await.assert_status(422);
    assert_eq!(w.reload(&ack).await.status, AckStatus::Pending);
    // Only the reader acknowledges for themselves.
    w.t.app.acting_as(&w.bilal);
    w.t.app
        .get(&format!(
            "/documents/acknowledgements/{}/files/{}",
            ack.id, v1.id
        ))
        .await
        .assert_forbidden();
    w.confirm(&w.bilal, &ack).await.assert_forbidden();

    w.open(&w.ali, &ack, &v1).await;
    w.confirm(&w.ali, &ack).await.assert_ok();
    let done = w.reload(&ack).await;
    assert_eq!(done.status, AckStatus::Acknowledged);
    assert_eq!(done.method, "click-to-accept");
    assert_eq!(done.language, "en");
    assert_eq!(done.ack_document_id, Some(v1.id));
    assert_eq!(done.acknowledged_by, Some(w.ali.id));
    assert!(done.acknowledged_at.is_some());
    let file = v1.current_file(w.t.db()).await.unwrap().unwrap();
    assert_eq!(done.checksum, file.checksum);
    assert!(!done.checksum.is_empty());
    assert_eq!(done.witness_id, None);

    let seen = w.t.acknowledged_events();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].sop, SopId(w.sop.id));
    assert_eq!(seen[0].employee, Subject::new("hr.employee", ALI));
    assert_eq!(seen[0].witness, None);
    // Confirmed once: a second confirmation finds it settled.
    w.confirm(&w.ali, &done).await.assert_status(422);
}

/// AC3: Ali still pending is reminded on the first run after the due date and
/// every 3 days; 7 days after it his manager and the owner role are told,
/// once.
#[renox::test]
async fn doc_rul_04_overdue_readers_are_reminded_and_then_escalated_once() {
    let w = world().await;
    let v1 = w.publish().await;
    let ack = w.ack(ALI, &v1).await;
    assert_eq!(ack.due_date, day(7));

    run(w.t.state(), day(7)).await.unwrap();
    assert!(w.sent(REMINDER).is_empty(), "nothing is due on the due date");

    run(w.t.state(), day(8)).await.unwrap();
    let reminders = w.sent(REMINDER);
    let mine = about_all(&reminders, &ack);
    assert_eq!(mine.len(), 1);
    assert_eq!(users(mine[0]), [w.ali.id]);
    // The same day again, and the next two days: nothing.
    for offset in [8, 9, 10] {
        run(w.t.state(), day(offset)).await.unwrap();
    }
    assert_eq!(about_all(&w.sent(REMINDER), &ack).len(), 1);

    run(w.t.state(), day(11)).await.unwrap();
    assert_eq!(about_all(&w.sent(REMINDER), &ack).len(), 2);
    assert!(w.sent(ESCALATED).is_empty());

    run(w.t.state(), day(14)).await.unwrap();
    let escalated = w.sent(ESCALATED);
    let told = about_all(&escalated, &ack);
    assert_eq!(told.len(), 1);
    let mut both = vec![w.mira.id, w.hana.id];
    both.sort_unstable();
    assert_eq!(users(told[0]), both, "the manager and the owner role");

    for offset in [15, 17, 20] {
        run(w.t.state(), day(offset)).await.unwrap();
    }
    assert_eq!(about_all(&w.sent(ESCALATED), &ack).len(), 1);
    let after = w.reload(&ack).await;
    assert_eq!(after.escalated_on, Some(day(14)));
    assert_eq!(after.last_reminded_on, Some(day(20)));

    // Once acknowledged, nothing more is sent.
    w.open(&w.ali, &after, &v1).await;
    w.confirm(&w.ali, &after).await.assert_ok();
    let reminded = about_all(&w.sent(REMINDER), &after).len();
    run(w.t.state(), day(23)).await.unwrap();
    assert_eq!(about_all(&w.sent(REMINDER), &after).len(), reminded);
    assert_eq!(w.reload(&after).await.status, AckStatus::Acknowledged);
}

/// AC4: version 2 accepted: the pending acknowledgements of version 1 are
/// `replaced`, an acknowledged one stays as history, and every reader, Ali
/// included, gets a new `pending` one.
#[renox::test]
async fn doc_rul_04_a_new_version_replaces_the_pending_and_asks_again() {
    let w = world().await;
    let v1 = w.publish().await;
    let ali_v1 = w.ack(ALI, &v1).await;
    w.open(&w.ali, &ali_v1, &v1).await;
    w.confirm(&w.ali, &ali_v1).await.assert_ok();

    let v2 = w.publish().await;
    assert_ne!(v1.id, v2.id);
    assert_eq!(w.reload(&ali_v1).await.status, AckStatus::Acknowledged);
    let ali_v2 = w.ack(ALI, &v2).await;
    assert_eq!(ali_v2.status, AckStatus::Pending);
    assert_eq!(ali_v2.version_no, 2);
    let sunil_v1 = w.ack(SUNIL, &v1).await;
    assert_eq!(w.reload(&sunil_v1).await.status, AckStatus::Replaced);
    assert!(w.reload(&sunil_v1).await.ended_reason.is_some());
    assert_eq!(w.count(AckStatus::Replaced).await, 11);
    assert_eq!(w.count(AckStatus::Pending).await, 12);
    assert_eq!(w.count(AckStatus::Acknowledged).await, 1);
}

/// AC5: Ali exits: his pending acknowledgement is `cancelled`, and no further
/// reminder follows.
#[renox::test]
async fn doc_rul_04_a_reader_who_exits_is_not_asked_any_more() {
    let w = world().await;
    let v1 = w.publish().await;
    let ack = w.ack(ALI, &v1).await;
    w.t.state()
        .emit(EmployeeExited {
            employee: EmployeeId(ALI),
            exit_date: day(1),
            route: "standard".into(),
        })
        .await
        .unwrap();
    assert_eq!(w.reload(&ack).await.status, AckStatus::Cancelled);
    assert_eq!(w.count(AckStatus::Pending).await, 11);
    w.t.staff.change(ALI, |p| p.listed = false);
    run(w.t.state(), day(20)).await.unwrap();
    assert!(about_all(&w.sent(REMINDER), &ack).is_empty());
}

/// AC6: a new site worker hired (`OnboardingStarted`) gets a pending
/// acknowledgement; so does a worker who takes the role later
/// (`EmployeeAssignmentChanged`); the daily run asks a reader it missed.
#[renox::test]
async fn doc_rul_04_a_new_reader_is_asked() {
    let w = world().await;
    let v1 = w.publish().await;
    w.t.staff.add(site_worker(113, None));
    w.t.state()
        .emit(OnboardingStarted {
            onboarding: OnboardingId(9),
            employee: EmployeeId(113),
            job_role: "site_worker".into(),
            scope: Some(Subject::new("sites", 1)),
            trigger: "hire".into(),
        })
        .await
        .unwrap();
    let new = w.ack(113, &v1).await;
    assert_eq!(new.status, AckStatus::Pending);
    assert_eq!(new.due_date, day(7));

    // A driver who becomes a site worker.
    let mut driver = site_worker(114, None);
    driver.role = "driver".into();
    w.t.staff.add(driver);
    w.t.state()
        .emit(OnboardingStarted {
            onboarding: OnboardingId(10),
            employee: EmployeeId(114),
            job_role: "driver".into(),
            scope: Some(Subject::new("sites", 1)),
            trigger: "hire".into(),
        })
        .await
        .unwrap();
    assert_eq!(w.count(AckStatus::Pending).await, 13);
    w.t.staff.change(114, |p| p.role = "site_worker".into());
    w.t.state()
        .emit(EmployeeAssignmentChanged {
            employee: EmployeeId(114),
            job_role: "site_worker".into(),
            scope: None,
            from: Some("driver".into()),
            to: Some("site_worker".into()),
        })
        .await
        .unwrap();
    assert_eq!(w.count(AckStatus::Pending).await, 14);
    // ... and who loses the role loses the request.
    w.t.staff.change(114, |p| p.role = "driver".into());
    w.t.state()
        .emit(EmployeeAssignmentChanged {
            employee: EmployeeId(114),
            job_role: "driver".into(),
            scope: None,
            from: Some("site_worker".into()),
            to: Some("driver".into()),
        })
        .await
        .unwrap();
    assert_eq!(w.count(AckStatus::Pending).await, 13);

    // A reader no event told about is asked by the daily run.
    w.t.staff.add(site_worker(115, None));
    let report = run(w.t.state(), day(1)).await.unwrap();
    assert_eq!(report.requested, 1);
    assert_eq!(w.ack(115, &v1).await.due_date, day(8));
}

/// AC7: Sunil has no account: his request is on his supervisor's *to
/// acknowledge in person* list, and the supervisor, not Sunil, gets the
/// reminders.
#[renox::test]
async fn doc_rul_04_a_reader_without_an_account_is_on_the_managers_list() {
    let w = world().await;
    let v1 = w.publish().await;
    let sunil = w.ack(SUNIL, &v1).await;

    w.t.app.acting_as(&w.mira);
    let page = w.t.app.get("/documents/acknowledgements").await;
    page.assert_ok();
    page.assert_see("SOP-HSE-01");
    page.assert_see(&format!("#{SUNIL}"));
    // Not on the list of someone who manages nobody.
    w.t.app.acting_as(&w.ali);
    w.t.app
        .get("/documents/acknowledgements")
        .await
        .assert_dont_see(&format!("#{SUNIL}"));

    run(w.t.state(), day(8)).await.unwrap();
    let reminders = w.sent(REMINDER);
    let for_sunil = about_all(&reminders, &sunil);
    assert_eq!(for_sunil.len(), 1);
    assert_eq!(users(for_sunil[0]), [w.mira.id]);
}

/// AC8: HSE officer Hana records Sunil's acknowledgement in person with his
/// drawn signature: Sunil gets an SMS naming SOP-HSE-01 and the witness.
#[renox::test]
async fn doc_rul_04_an_in_person_record_names_the_witness_and_tells_the_reader() {
    let w = world().await;
    let v1 = w.publish().await;
    let sunil = w.ack(SUNIL, &v1).await;

    // A signature is needed, and so is the right to witness.
    w.t.app.acting_as(&w.hana);
    w.t.app
        .htmx()
        .post(
            &format!("/documents/acknowledgements/{}/in-person", sunil.id),
            &[("signature", " ")],
        )
        .await
        .assert_status(422);
    w.t.app.acting_as(&w.bilal);
    w.t.app
        .htmx()
        .post(
            &format!("/documents/acknowledgements/{}/in-person", sunil.id),
            &[("signature", "data:image/png;base64,AAAA")],
        )
        .await
        .assert_forbidden();

    w.t.app.acting_as(&w.hana);
    w.t.app
        .htmx()
        .post(
            &format!("/documents/acknowledgements/{}/in-person", sunil.id),
            &[("signature", "data:image/png;base64,AAAA")],
        )
        .await
        .assert_ok();
    let done = w.reload(&sunil).await;
    assert_eq!(done.status, AckStatus::Acknowledged);
    assert_eq!(done.method, "drawn");
    assert_eq!(done.witness_id, Some(w.hana.id));
    assert_eq!(done.signature.as_deref(), Some("data:image/png;base64,AAAA"));
    assert!(done.in_person_notified_at.is_some());

    let notices = w.sent(IN_PERSON_RECORDED);
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].recipients,
        [NotifyRecipient::Party {
            id: PartyId(1000 + SUNIL)
        }]
    );
    assert_eq!(notices[0].data["sop"], "SOP-HSE-01");
    assert_eq!(notices[0].data["witness"], "Hana");
    let seen = w.t.acknowledged_events();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].witness, Some(w.hana.id));

    // The SOP page lists the record per witness.
    w.t.app.acting_as(&w.hana);
    let page = w.t.app.get(&format!("/documents/sops/{}", w.sop.id)).await;
    page.assert_ok();
    page.assert_see("Hana");
    page.assert_see("In person");
}

/// AC9: Omar is on leave until `day(30)`: his acknowledgement is due 7 days
/// after his return and nothing is sent before.
#[renox::test]
async fn doc_rul_04_a_reader_on_leave_is_asked_from_the_return_date() {
    let w = world().await;
    let v1 = w.publish().await;
    let omar = w.ack(OMAR, &v1).await;
    assert_eq!(omar.due_date, day(37));
    for offset in [8, 20, 36, 40] {
        run(w.t.state(), day(offset)).await.unwrap();
    }
    let reminders = w.sent(REMINDER);
    // He is on leave in this table for good: paused all along.
    assert!(about_all(&reminders, &omar).is_empty());
    // Back at work, he is reminded after the due date.
    w.t.staff.change(OMAR, |p| {
        p.on_leave = false;
        p.returns_on = None;
    });
    run(w.t.state(), day(38)).await.unwrap();
    assert_eq!(about_all(&w.sent(REMINDER), &omar).len(), 1);
}

/// An absconded reader (not listed by the workforce) keeps the request but is
/// not reminded.
#[renox::test]
async fn doc_rul_04_reminders_pause_for_a_reader_the_workforce_does_not_list() {
    let w = world().await;
    let v1 = w.publish().await;
    let ack = w.ack(ALI, &v1).await;
    w.t.staff.change(ALI, |p| p.listed = false);
    run(w.t.state(), day(10)).await.unwrap();
    run(w.t.state(), day(20)).await.unwrap();
    assert!(about_all(&w.sent(REMINDER), &ack).is_empty());
    assert!(about_all(&w.sent(ESCALATED), &ack).is_empty());
    assert_eq!(w.reload(&ack).await.status, AckStatus::Pending);
    // Reinstated: reminded on the first run, and the manager told.
    w.t.staff.change(ALI, |p| p.listed = true);
    run(w.t.state(), day(21)).await.unwrap();
    assert_eq!(about_all(&w.sent(REMINDER), &ack).len(), 1);
    assert_eq!(about_all(&w.sent(ESCALATED), &ack).len(), 1);
}

/// AC10: Bilal opens version 1; version 2 is accepted before he confirms: his
/// confirmation is refused and he is shown version 2.
#[renox::test]
async fn doc_rul_04_a_confirmation_on_a_superseded_version_is_refused() {
    let w = world().await;
    let v1 = w.publish().await;
    let ack = w.ack(BILAL, &v1).await;
    w.open(&w.bilal, &ack, &v1).await;
    let v2 = w.publish().await;

    let refused = w.confirm(&w.bilal, &ack).await;
    refused.assert_status(422);
    assert!(refused.text().contains("newer version"), "{}", refused.text());
    assert_eq!(w.reload(&ack).await.status, AckStatus::Replaced);

    // He is shown version 2 in his list.
    w.t.app.acting_as(&w.bilal);
    let list = w.t.app.get("/documents/acknowledgements").await;
    list.assert_ok();
    list.assert_see("v2");
    let current = w.ack(BILAL, &v2).await;
    assert_eq!(current.status, AckStatus::Pending);
    // Opening the old version is refused as well.
    w.t.app
        .get(&format!(
            "/documents/acknowledgements/{}/files/{}",
            ack.id, v1.id
        ))
        .await
        .assert_status(422);
}

/// AC11: Bilal reads the Arabic translation of version 2: the acknowledgement
/// records language `ar` and the checksum of the translation.
#[renox::test]
async fn doc_rul_04_a_translation_is_recorded_with_its_language() {
    let w = world().await;
    let _v1 = w.publish().await;
    let v2 = w.publish().await;
    w.t.app.acting_as(&w.hana);
    let translated = w
        .t
        .app
        .post(
            &format!("/test/sop-versions/{}/translations", v2.id),
            &[("language", "ar")],
        )
        .await;
    translated.assert_ok();
    let translation = Document::find_or_404(w.t.db(), translated.text().parse().unwrap())
        .await
        .unwrap();
    // A translation that is not accepted yet cannot be opened.
    let ack = w.ack(BILAL, &v2).await;
    w.t.app.acting_as(&w.bilal);
    w.t.app
        .get(&format!(
            "/documents/acknowledgements/{}/files/{}",
            ack.id, translation.id
        ))
        .await
        .assert_not_found();
    w.t.app.acting_as(&w.hana);
    w.t.app
        .post(
            &format!("/test/sop-translations/{}/accept", translation.id),
            &[],
        )
        .await
        .assert_ok();
    // It asks nobody again.
    assert_eq!(w.count(AckStatus::Pending).await, 12);

    w.open(&w.bilal, &ack, &translation).await;
    w.confirm(&w.bilal, &ack).await.assert_ok();
    let done = w.reload(&ack).await;
    assert_eq!(done.status, AckStatus::Acknowledged);
    assert_eq!(done.language, "ar");
    assert_eq!(done.ack_document_id, Some(translation.id));
    let file = translation.current_file(w.t.db()).await.unwrap().unwrap();
    assert_eq!(done.checksum, file.checksum);
}

/// Withdrawing the SOP cancels the pending acknowledgements and leaves the
/// acknowledged one as history; changing the readers asks the new ones and
/// cancels the removed.
#[renox::test]
async fn doc_rul_04_withdrawing_and_changing_readers() {
    let w = world().await;
    let v1 = w.publish().await;
    let ali = w.ack(ALI, &v1).await;
    w.open(&w.ali, &ali, &v1).await;
    w.confirm(&w.ali, &ali).await.assert_ok();

    // The readers become drivers only: the site workers are not required.
    w.t.app.acting_as(&w.hana);
    w.t.app
        .post(
            &format!("/test/sops/{}/readers", w.sop.id),
            &[("readers", "driver")],
        )
        .await
        .assert_ok();
    assert_eq!(w.count(AckStatus::Pending).await, 0);
    assert_eq!(w.count(AckStatus::Cancelled).await, 11);
    assert_eq!(w.count(AckStatus::Acknowledged).await, 1);
    // ... and back: those who were cancelled are asked again.
    w.t.app
        .post(
            &format!("/test/sops/{}/readers", w.sop.id),
            &[("readers", "site_worker@sites:1")],
        )
        .await
        .assert_ok();
    assert_eq!(w.count(AckStatus::Pending).await, 11);

    w.t.app
        .post(&format!("/test/sops/{}/withdraw", w.sop.id), &[])
        .await
        .assert_ok();
    assert_eq!(w.count(AckStatus::Pending).await, 0);
    assert_eq!(w.count(AckStatus::Acknowledged).await, 1);
    // A withdrawn SOP asks nobody.
    run(w.t.state(), day(1)).await.unwrap();
    assert_eq!(w.count(AckStatus::Pending).await, 0);
}

/// The kernel contract wl-hr's `sop_acknowledgement` step asks: the SOPs
/// whose current version the employee has not acknowledged, and recording
/// them in person.
#[renox::test]
async fn doc_rul_04_the_onboarding_contract_waits_for_the_acknowledgement() {
    let w = world().await;
    let who = |id: i64, role: &str| SopStaff {
        employee: Subject::new("hr.employee", id),
        role: role.to_owned(),
        scopes: vec![Subject::new("sites", 1)],
    };
    let contract = sop_acknowledgements(w.t.state());
    assert!(contract
        .pending_for(w.t.state(), &who(ALI, "site_worker"))
        .await
        .unwrap()
        .is_empty(), "a draft SOP is pending for nobody");

    let v1 = w.publish().await;
    assert_eq!(
        contract
            .pending_for(w.t.state(), &who(ALI, "site_worker"))
            .await
            .unwrap(),
        [SopId(w.sop.id)]
    );
    assert!(contract
        .pending_for(w.t.state(), &who(ALI, "accountant"))
        .await
        .unwrap()
        .is_empty());

    let ack = w.ack(ALI, &v1).await;
    w.open(&w.ali, &ack, &v1).await;
    w.confirm(&w.ali, &ack).await.assert_ok();
    assert!(contract
        .pending_for(w.t.state(), &who(ALI, "site_worker"))
        .await
        .unwrap()
        .is_empty());

    // Sunil is recorded in person through the contract.
    let sunil = who(SUNIL, "site_worker");
    let done = contract
        .acknowledge_in_person(w.t.state(), &sunil, "data:image/png;base64,AAAA", w.hana.id)
        .await
        .unwrap();
    assert_eq!(done, [SopId(w.sop.id)]);
    let record = w.ack(SUNIL, &v1).await;
    assert_eq!(record.status, AckStatus::Acknowledged);
    assert_eq!(record.method, "drawn");
    assert_eq!(record.witness_id, Some(w.hana.id));
    assert!(contract
        .pending_for(w.t.state(), &sunil)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(w.sent(IN_PERSON_RECORDED).len(), 1);

    // A version published later is pending again.
    let _v2 = w.publish().await;
    assert_eq!(
        contract
            .pending_for(w.t.state(), &who(ALI, "site_worker"))
            .await
            .unwrap(),
        [SopId(w.sop.id)]
    );
}
