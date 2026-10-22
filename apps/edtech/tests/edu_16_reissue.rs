//! BL-EDU-16 (#451): a certificate with a wrong name or date is never edited.
//! It is reissued: the old one becomes `superseded`, a linked draft with the
//! corrections is issued under a new number, the old public page points to it
//! and the student is told; a lost copy is printed again without any change.
//! The QR image and the PDF are wl-documents' and are deferred; the page of the
//! copy prints the wording, the number and the code.

mod support;

use edtech::certificate::{Certificate, CertificateStatus};
use edtech::certificates::{self, Corrections};
use edtech::courses::{self, NewCourse};
use edtech::enrolments;
use edtech::notices::FAMILY_NOTICE;
use edtech::{Batch, Course, Enrolment};
use renox::prelude::*;
use renox::state_machine::history;
use support::{Centre, Messages, boot, day, next_weekday};
use wl_kernel::Subject;

/// An open batch from next Monday.
async fn open_batch(t: &Centre) -> Batch {
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    t.open_batch(&tutor, &room, next_weekday(day(7), 1)).await
}

/// A draft certificate of party `id` named `name`, who completed the course.
async fn draft_for(t: &Centre, batch: &Batch, id: i64, name: &str) -> (Certificate, Enrolment) {
    let student = t.person(id, name);
    let desk = t.desk().await;
    let e = t.enrol(student, batch).await;
    enrolments::activate(t.state(), Some(&desk), e.id).await.unwrap();
    enrolments::complete(t.state(), e.id).await.unwrap();
    let draft = Certificate::of_enrolment(t.db(), e.id)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("a draft certificate");
    (draft, e)
}

/// An issued certificate of Ahmad, with the number the series gave it.
async fn issued_for_ahmad(t: &Centre, batch: &Batch) -> (Certificate, Enrolment) {
    let (draft, e) = draft_for(t, batch, 1, "Ahmad Yusuf").await;
    let registrar = t.registrar().await;
    let issued = certificates::issue(t.state(), &registrar, draft.id)
        .await
        .unwrap();
    assert_eq!(issued.status, CertificateStatus::Issued);
    (issued, e)
}

fn corrected_name() -> Corrections {
    Corrections {
        student_name: Some("Ahmed Yusuf".to_owned()),
        ..Corrections::default()
    }
}

/// Given issued certificate CRT-0001 with the name "Ahmad" instead of "Ahmed",
/// when the coordinator reissues it with the reason "name correction" and the
/// corrected name, it is `superseded` and a linked draft exists; the issued
/// certificate itself was not edited.
#[renox::test]
async fn edu_16_a_reissue_supersedes_the_certificate_and_links_a_corrected_draft() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let (issued, e) = issued_for_ahmad(&t, &batch).await;
    assert_eq!(issued.number.as_deref(), Some("CRT-0001"));
    let registrar = t.registrar().await;

    let draft = certificates::reissue(
        t.state(),
        &registrar,
        issued.id,
        "name correction",
        corrected_name(),
    )
    .await
    .unwrap();

    let old = Certificate::find_or_404(t.db(), issued.id).await.unwrap();
    assert_eq!(old.status, CertificateStatus::Superseded);
    assert_eq!(old.reissue_reason.as_deref(), Some("name correction"));
    assert_eq!(old.student_name, "Ahmad Yusuf", "the issued certificate is never edited");
    assert_eq!(old.number.as_deref(), Some("CRT-0001"));
    assert_eq!(draft.status, CertificateStatus::Draft);
    assert_eq!(draft.supersedes_id, Some(old.id));
    assert_eq!(draft.student_name, "Ahmed Yusuf");
    assert_eq!(draft.enrolment_id, e.id);
    assert!(draft.number.is_none(), "numbered when it is issued");
    assert_ne!(draft.verification_code, old.verification_code);
    // The history of the old one says what happened.
    let rows = history(t.db(), "edtech_certificates", old.id).await.unwrap();
    assert_eq!(rows.last().map(|r| r.transition.as_str()), Some("reissue"));
}

/// When the draft is issued as CRT-0002, the public page of CRT-0001 points to
/// it and the student is told; until then the old page says "replacement
/// pending". The old number is not reused.
#[renox::test]
async fn edu_16_the_old_page_points_to_the_new_number_and_the_student_is_told() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let (issued, e) = issued_for_ahmad(&t, &batch).await;
    let registrar = t.registrar().await;
    let draft = certificates::reissue(
        t.state(),
        &registrar,
        issued.id,
        "name correction",
        corrected_name(),
    )
    .await
    .unwrap();
    let told = t.messages.of_type(FAMILY_NOTICE).len();

    t.app.logout();
    t.app
        .get(&issued.verification_path())
        .await
        .assert_ok()
        .assert_see("Superseded")
        .assert_see("replacement pending");
    assert_eq!(t.messages.of_type(FAMILY_NOTICE).len(), told, "nobody is told of a draft");

    let numbered = certificates::issue(t.state(), &registrar, draft.id)
        .await
        .unwrap();

    assert_eq!(numbered.number.as_deref(), Some("CRT-0002"));
    assert_ne!(numbered.number, issued.number, "the old number is not reused");
    let old = Certificate::find_or_404(t.db(), issued.id).await.unwrap();
    assert_eq!(old.superseded_by_id, Some(numbered.id));
    t.app
        .get(&old.verification_path())
        .await
        .assert_ok()
        .assert_see("CRT-0002")
        .assert_see(&numbered.verification_path());
    t.app
        .get(&numbered.verification_path())
        .await
        .assert_ok()
        .assert_see("Ahmed Yusuf")
        .assert_see("Valid");
    let notices = t.messages.of_type(FAMILY_NOTICE);
    assert_eq!(notices.len(), told + 1, "the student is told once");
    let notice = notices.last().unwrap();
    assert!(Messages::told_party(notice, e.student_party_id));
    let text = Messages::text(notice);
    assert!(text.contains("CRT-0002"), "{text}");
    assert!(text.contains(&numbered.verification_path()), "{text}");
}

/// Reissue without a reason is refused; from `revoked` or `superseded` it is
/// not offered; nothing changed.
#[renox::test]
async fn edu_16_a_reissue_needs_a_reason_and_an_issued_certificate() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let (issued, e) = issued_for_ahmad(&t, &batch).await;
    let registrar = t.registrar().await;

    for reason in ["", "   "] {
        let refused = certificates::reissue(
            t.state(),
            &registrar,
            issued.id,
            reason,
            corrected_name(),
        )
        .await;
        assert!(refused.is_err(), "no reason: {reason:?}");
    }
    let still = Certificate::find_or_404(t.db(), issued.id).await.unwrap();
    assert_eq!(still.status, CertificateStatus::Issued);
    assert_eq!(Certificate::of_enrolment(t.db(), e.id).await.unwrap().len(), 1);

    // The page offers it for an issued certificate to the one who may issue.
    let registrar_user = t.registrar_user().await;
    t.app.acting_as(&registrar_user);
    let path = format!("/edtech/certificates/{}", issued.id);
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("A certificate is never edited.")
        .assert_see("Print a copy");

    // Superseded: no longer offered, and refused.
    certificates::reissue(t.state(), &registrar, issued.id, "name", corrected_name())
        .await
        .unwrap();
    let page = t.app.get(&path).await;
    page.assert_ok();
    assert!(!page.text().contains("A certificate is never edited."));
    assert!(!page.text().contains("Print a copy"));
    assert!(
        certificates::reissue(t.state(), &registrar, issued.id, "again", corrected_name())
            .await
            .is_err()
    );

    // Revoked: the same, for another certificate.
    let (second, _) = draft_for(&t, &batch, 2, "Omar Khan").await;
    let second = certificates::issue(t.state(), &registrar, second.id).await.unwrap();
    let admin = t.company_admin().await;
    certificates::revoke(t.state(), &admin, second.id, "given in error")
        .await
        .unwrap();
    assert!(
        certificates::reissue(t.state(), &registrar, second.id, "name", corrected_name())
            .await
            .is_err()
    );
    let revoked = t.app.get(&format!("/edtech/certificates/{}", second.id)).await;
    revoked.assert_ok();
    assert!(!revoked.text().contains("A certificate is never edited."));
}

/// Generating a copy of CRT-0002 leaves its state and number unchanged, logs
/// the copy in its history, and the printable page reads the same each time.
#[renox::test]
async fn edu_16_a_copy_changes_nothing_and_is_logged_in_the_history() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let (issued, _) = issued_for_ahmad(&t, &batch).await;
    let registrar = t.registrar().await;

    let first = certificates::print_copy(t.state(), &registrar, issued.id)
        .await
        .unwrap();
    let second = certificates::print_copy(t.state(), &registrar, issued.id)
        .await
        .unwrap();

    for copy in [&first, &second] {
        assert_eq!(copy.status, CertificateStatus::Issued);
        assert_eq!(copy.number, issued.number);
        assert_eq!(copy.verification_code, issued.verification_code);
        assert_eq!(copy.student_name, issued.student_name);
    }
    assert_eq!(first.copies_printed, 1);
    assert_eq!(second.copies_printed, 2);
    assert_eq!(second.last_copy_on, Some(day(0)));
    let rows = history(t.db(), "edtech_certificates", issued.id).await.unwrap();
    let copies = rows.iter().filter(|r| r.transition == "print_copy").count();
    assert_eq!(copies, 2, "each copy is in the history");

    // The page: the action prints, the printable page reads the same.
    let registrar_user = t.registrar_user().await;
    t.app.acting_as(&registrar_user);
    t.app
        .htmx()
        .post(&format!("/edtech/certificates/{}/copy", issued.id), &[("x", "y")])
        .await
        .assert_ok();
    t.app
        .get(&format!("/edtech/certificates/{}/print", issued.id))
        .await
        .assert_ok()
        .assert_see("CRT-0001")
        .assert_see("Ahmad Yusuf")
        .assert_see(&issued.verification_code)
        .assert_see("IELTS preparation");
    assert_eq!(
        Certificate::find_or_404(t.db(), issued.id).await.unwrap().copies_printed,
        3
    );
}

/// A draft, a superseded and a revoked certificate have no copy.
#[renox::test]
async fn edu_16_only_an_issued_certificate_has_a_copy() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let (draft, _) = draft_for(&t, &batch, 1, "Ahmad Yusuf").await;
    let registrar = t.registrar().await;

    assert!(certificates::print_copy(t.state(), &registrar, draft.id).await.is_err());
    let registrar_user = t.registrar_user().await;
    t.app.acting_as(&registrar_user);
    t.app
        .get(&format!("/edtech/certificates/{}/print", draft.id))
        .await
        .assert_status(422);

    let issued = certificates::issue(t.state(), &registrar, draft.id).await.unwrap();
    certificates::reissue(t.state(), &registrar, issued.id, "name", corrected_name())
        .await
        .unwrap();
    assert!(certificates::print_copy(t.state(), &registrar, issued.id).await.is_err());
    // A user without `certificates.issue` cannot print one.
    let desk = t.desk().await;
    let (other, _) = draft_for(&t, &batch, 2, "Omar Khan").await;
    let other = certificates::issue(t.state(), &registrar, other.id).await.unwrap();
    assert!(certificates::print_copy(t.state(), &desk, other.id).await.is_err());
}

/// The reissued draft is judged on the enrolment's completion date: a correct
/// certificate earned under a valid approval is reissued and issued after the
/// approval expired, while a draft completed today is refused.
#[renox::test]
async fn edu_16_the_reissued_draft_is_judged_on_the_completion_date() {
    let t = boot().await;
    t.documents.edtech_blocks();
    let amal = t.coordinator().await;
    let mut new = NewCourse::new("IELTS-B2", "IELTS preparation", "English language", 40).level("B2");
    new.approval_required = true;
    new.approval_no = Some("KHDA-2026-77".to_owned());
    let course: Course = courses::create(t.state(), &amal, new).await.unwrap();
    let entity = wl_kernel::LegalEntity::default_entity(t.db()).await.unwrap();
    t.documents.set(
        &Subject::new("kernel.legal_entity", entity.id),
        "centre_permit",
        Some(day(400)),
    );
    t.documents.set(&course.subject(), "course_approval", Some(day(400)));
    let batch = open_batch(&t).await;
    let (issued, e) = issued_for_ahmad(&t, &batch).await;
    assert!(issued.states_approval);
    let registrar = t.registrar().await;

    // The approval expired yesterday; the course was completed a month ago.
    t.documents.set(&course.subject(), "course_approval", Some(day(-1)));
    Enrolment::query()
        .where_(Enrolment::ID.eq(e.id))
        .update(t.db(), &[("completed_on", &Some(day(-30)))])
        .await
        .unwrap();
    let draft = certificates::reissue(t.state(), &registrar, issued.id, "name correction", corrected_name())
        .await
        .unwrap();
    let numbered = certificates::issue(t.state(), &registrar, draft.id)
        .await
        .expect("judged on the completion date, the approval was valid");
    assert_eq!(numbered.status, CertificateStatus::Issued);

    // A draft completed today is judged today: refused.
    let (fresh, _) = draft_for(&t, &batch, 2, "Omar Khan").await;
    let error = certificates::issue(t.state(), &registrar, fresh.id)
        .await
        .unwrap_err();
    assert!(matches!(&error, Error::Status(StatusCode::UNPROCESSABLE_ENTITY, m)
        if m.contains("Course approval")), "{error:?}");
}

/// The reissue form on the page: it needs a reason, takes the corrected name and
/// copies the recorded attestation only when ticked, and opens the new draft.
#[renox::test]
async fn edu_16_the_page_reissues_with_the_corrected_data() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let (issued, _) = issued_for_ahmad(&t, &batch).await;
    Certificate::query()
        .where_(Certificate::ID.eq(issued.id))
        .update(t.db(), &[("attestation_ref", &Some("KHDA-77".to_owned()))])
        .await
        .unwrap();
    let registrar_user = t.registrar_user().await;
    t.app.acting_as(&registrar_user);
    let path = format!("/edtech/certificates/{}/reissue", issued.id);

    t.app
        .htmx()
        .post(
            &path,
            &[("reason", ""), ("student_name", "Ahmed Yusuf"), ("completion_date", "")],
        )
        .await
        .assert_status(422);
    assert_eq!(
        Certificate::find_or_404(t.db(), issued.id).await.unwrap().status,
        CertificateStatus::Issued
    );

    t.app
        .htmx()
        .post(
            &path,
            &[
                ("reason", "name correction"),
                ("student_name", "Ahmed Yusuf"),
                ("completion_date", ""),
                ("copy_attestation", "on"),
            ],
        )
        .await
        .assert_ok();

    let draft = Certificate::query()
        .where_(Certificate::SUPERSEDES_ID.eq(issued.id))
        .first(t.db())
        .await
        .unwrap()
        .expect("the linked draft");
    assert_eq!(draft.student_name, "Ahmed Yusuf");
    assert_eq!(draft.completion_date, issued.completion_date);
    assert_eq!(draft.attestation_ref.as_deref(), Some("KHDA-77"));
}
