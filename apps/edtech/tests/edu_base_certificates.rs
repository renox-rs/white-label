//! The base of the 10 EdTech certificate (base for #442, #451): a draft when a
//! course enrolment completes, `issue` with a gapless number, the public
//! verification page that needs no login, `revoke`, and `reissue` (the seam of
//! BL-EDU-16) that supersedes the certificate and links a corrected draft.

mod support;

use edtech::certificate::{Certificate, CertificateStatus};
use edtech::certificates::{self, Corrections};
use edtech::enrolments;
use edtech::{Batch, EnrolmentStatus};
use renox::prelude::*;
use support::{Centre, boot, day, next_weekday};

/// An open batch from next Monday.
async fn open_batch(t: &Centre) -> Batch {
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    t.open_batch(&tutor, &room, next_weekday(day(7), 1)).await
}

/// A draft certificate of a student who completed the course.
async fn draft_for(t: &Centre, batch: &Batch, id: i64, name: &str) -> Certificate {
    let student = t.person(id, name);
    let desk = t.desk().await;
    let e = t.enrol(student, batch).await;
    enrolments::activate(t.state(), Some(&desk), e.id)
        .await
        .unwrap();
    enrolments::complete(t.state(), e.id).await.unwrap();
    Certificate::of_enrolment(t.db(), e.id)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("a draft certificate")
}

/// Completing a course enrolment gives the seat back and drafts a certificate
/// with the name, the course and the completion date; a package or a
/// membership has none.
#[renox::test]
async fn edu_base_completing_an_enrolment_drafts_its_certificate() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let draft = draft_for(&t, &batch, 1, "Sara Ali").await;

    assert_eq!(draft.status, CertificateStatus::Draft);
    assert_eq!(draft.student_name, "Sara Ali");
    assert_eq!(draft.course_title, "IELTS preparation");
    assert_eq!(draft.completion_date, day(0));
    assert!(draft.number.is_none());
    assert_eq!(draft.verification_code.len(), 10);
    let e = edtech::Enrolment::find_or_404(t.db(), draft.enrolment_id)
        .await
        .unwrap();
    assert_eq!(e.status, EnrolmentStatus::Completed);
    assert_eq!(e.completed_on, Some(day(0)));
    assert!(!e.seat_held);
    assert_eq!(
        Batch::find_or_404(t.db(), batch.id)
            .await
            .unwrap()
            .seats_taken,
        0
    );
    // Evaluating again makes no second draft.
    assert!(
        certificates::evaluate(t.state(), &e)
            .await
            .unwrap()
            .is_none()
    );
}

/// `issue` numbers the certificate from a gapless series; the public page
/// verifies it without a login, and knows nothing of a draft.
#[renox::test]
async fn edu_base_an_issued_certificate_is_verified_on_the_public_page() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let first = draft_for(&t, &batch, 1, "Sara Ali").await;
    let second = draft_for(&t, &batch, 2, "Omar Khan").await;
    t.app.logout();
    t.app
        .get(&first.verification_path())
        .await
        .assert_ok()
        .assert_see("No published certificate");

    let registrar = t.registrar().await;
    let issued = certificates::issue(t.state(), &registrar, first.id)
        .await
        .unwrap();
    assert_eq!(issued.status, CertificateStatus::Issued);
    assert_eq!(issued.number.as_deref(), Some("CRT-0001"));
    assert_eq!(issued.issued_on, Some(day(0)));
    let next = certificates::issue(t.state(), &registrar, second.id)
        .await
        .unwrap();
    assert_eq!(next.number.as_deref(), Some("CRT-0002"));

    t.app.logout();
    t.app
        .get(&issued.verification_path())
        .await
        .assert_ok()
        .assert_see("CRT-0001")
        .assert_see("Sara Ali")
        .assert_see("IELTS preparation")
        .assert_see("Valid");
    // The code is read in any case.
    t.app
        .get(&format!(
            "/verify/certificate/{}",
            issued.verification_code.to_lowercase()
        ))
        .await
        .assert_ok()
        .assert_see("CRT-0001");
    t.app
        .get("/verify/certificate/NOSUCHCODE")
        .await
        .assert_ok()
        .assert_see("No published certificate");
}

/// A certificate is issued only by `certificates.issue`, and only when the
/// completion criteria are met.
#[renox::test]
async fn edu_base_only_a_completed_enrolment_gets_its_certificate_issued() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let student = t.person(1, "Sara Ali");
    let active = t.enrol(student, &batch).await;
    let desk = t.desk().await;
    enrolments::activate(t.state(), Some(&desk), active.id)
        .await
        .unwrap();
    let mut draft = Certificate {
        enrolment_id: active.id,
        student_party_id: student,
        student_name: "Sara Ali".to_owned(),
        course_title: "IELTS preparation".to_owned(),
        completion_date: day(0),
        verification_code: edtech::certificate::new_code(),
        ..Certificate::default()
    };
    draft.save(t.db()).await.unwrap();

    let registrar = t.registrar().await;
    let refused = certificates::issue(t.state(), &registrar, draft.id)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("criteria"));

    let finished = draft_for(&t, &batch, 2, "Omar Khan").await;
    assert!(
        certificates::issue(t.state(), &desk, finished.id)
            .await
            .is_err()
    );
    assert!(
        certificates::issue(t.state(), &registrar, finished.id)
            .await
            .is_ok()
    );
    // A certificate that was issued is not issued again.
    assert!(
        certificates::issue(t.state(), &registrar, finished.id)
            .await
            .is_err()
    );
}

/// `revoke` is the company admin's, needs a reason, and the public page then
/// says so.
#[renox::test]
async fn edu_base_a_revoked_certificate_says_so_on_the_public_page() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let draft = draft_for(&t, &batch, 1, "Sara Ali").await;
    let registrar = t.registrar().await;
    let admin = t.company_admin().await;
    let issued = certificates::issue(t.state(), &registrar, draft.id)
        .await
        .unwrap();

    assert!(
        certificates::revoke(t.state(), &registrar, issued.id, "error")
            .await
            .is_err()
    );
    assert!(
        certificates::revoke(t.state(), &admin, issued.id, " ")
            .await
            .is_err()
    );
    let revoked = certificates::revoke(t.state(), &admin, issued.id, "issued in error")
        .await
        .unwrap();
    assert_eq!(revoked.status, CertificateStatus::Revoked);
    assert_eq!(revoked.revoke_reason.as_deref(), Some("issued in error"));
    assert_eq!(revoked.revoked_on, Some(day(0)));

    t.app.logout();
    t.app
        .get(&issued.verification_path())
        .await
        .assert_ok()
        .assert_see("Revoked")
        .assert_see("CRT-0001");
    // A revoked certificate is not reissued.
    assert!(
        certificates::reissue(
            t.state(),
            &registrar,
            issued.id,
            "name",
            Corrections::default()
        )
        .await
        .is_err()
    );
}

/// `reissue` supersedes the certificate and creates a linked draft with the
/// corrected data; until the new one is issued the old page says "replacement
/// pending", after it the old page points to the new number. The old number is
/// never reused and a recorded attestation is copied only when confirmed.
#[renox::test]
async fn edu_base_a_reissue_supersedes_the_certificate_and_links_a_corrected_draft() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let draft = draft_for(&t, &batch, 1, "Sara Ali").await;
    let registrar = t.registrar().await;
    let issued = certificates::issue(t.state(), &registrar, draft.id)
        .await
        .unwrap();
    Certificate::query()
        .where_(Certificate::ID.eq(issued.id))
        .update(t.db(), &[("attestation_ref", &Some("KHDA-77".to_owned()))])
        .await
        .unwrap();

    let blank = certificates::reissue(
        t.state(),
        &registrar,
        issued.id,
        " ",
        Corrections::default(),
    )
    .await;
    assert!(blank.is_err());

    let replacement = certificates::reissue(
        t.state(),
        &registrar,
        issued.id,
        "name correction",
        Corrections {
            student_name: Some("Sarah Ali".to_owned()),
            completion_date: None,
            copy_attestation: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(replacement.status, CertificateStatus::Draft);
    assert_eq!(replacement.supersedes_id, Some(issued.id));
    assert_eq!(replacement.student_name, "Sarah Ali");
    assert!(replacement.number.is_none());
    assert!(replacement.attestation_ref.is_none());
    assert_ne!(replacement.verification_code, issued.verification_code);
    let old = Certificate::find_or_404(t.db(), issued.id).await.unwrap();
    assert_eq!(old.status, CertificateStatus::Superseded);
    assert_eq!(old.reissue_reason.as_deref(), Some("name correction"));
    assert!(old.superseded_by_id.is_none());

    t.app.logout();
    t.app
        .get(&old.verification_path())
        .await
        .assert_ok()
        .assert_see("replacement pending");

    let numbered = certificates::issue(t.state(), &registrar, replacement.id)
        .await
        .unwrap();
    assert_eq!(numbered.number.as_deref(), Some("CRT-0002"));
    let old = Certificate::find_or_404(t.db(), issued.id).await.unwrap();
    assert_eq!(old.superseded_by_id, Some(numbered.id));
    t.app
        .get(&old.verification_path())
        .await
        .assert_ok()
        .assert_see("Superseded")
        .assert_see("CRT-0002")
        .assert_see(&numbered.verification_path());

    // A superseded certificate is not reissued again.
    assert!(
        certificates::reissue(
            t.state(),
            &registrar,
            issued.id,
            "again",
            Corrections::default()
        )
        .await
        .is_err()
    );
}

/// The attestation of the old certificate is copied only on confirmation.
#[renox::test]
async fn edu_base_a_recorded_attestation_is_copied_only_when_confirmed() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let draft = draft_for(&t, &batch, 1, "Sara Ali").await;
    let registrar = t.registrar().await;
    let issued = certificates::issue(t.state(), &registrar, draft.id)
        .await
        .unwrap();
    Certificate::query()
        .where_(Certificate::ID.eq(issued.id))
        .update(t.db(), &[("attestation_ref", &Some("KHDA-77".to_owned()))])
        .await
        .unwrap();
    let replacement = certificates::reissue(
        t.state(),
        &registrar,
        issued.id,
        "date correction",
        Corrections {
            student_name: None,
            completion_date: Some(day(-2)),
            copy_attestation: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(replacement.attestation_ref.as_deref(), Some("KHDA-77"));
    assert_eq!(replacement.completion_date, day(-2));
    assert_eq!(replacement.student_name, "Sara Ali");
}

/// The certificate pages: the list, the page with `issue`, and `revoke`.
#[renox::test]
async fn edu_base_the_certificate_pages_show_issue_and_revoke() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let draft = draft_for(&t, &batch, 1, "Sara Ali").await;
    let registrar = t.registrar_user().await;
    t.app.acting_as(&registrar);

    t.app
        .get("/edtech/certificates")
        .await
        .assert_ok()
        .assert_see("Sara Ali")
        .assert_see("Draft");
    t.app
        .get(&format!("/edtech/certificates/{}", draft.id))
        .await
        .assert_ok()
        .assert_see("Issue")
        .assert_see(&draft.verification_code);
    t.app
        .htmx()
        .post(
            &format!("/edtech/certificates/{}/issue", draft.id),
            &[("x", "y")],
        )
        .await
        .assert_ok();
    let issued = Certificate::find_or_404(t.db(), draft.id).await.unwrap();
    assert_eq!(issued.status, CertificateStatus::Issued);
    t.app
        .htmx()
        .post(
            &format!("/edtech/certificates/{}/revoke", draft.id),
            &[("reason", "error")],
        )
        .await
        .assert_forbidden();

    let admin = t.company_admin_user().await;
    t.app.acting_as(&admin);
    t.app
        .get(&format!("/edtech/certificates/{}", draft.id))
        .await
        .assert_ok()
        .assert_see("CRT-0001")
        .assert_see("Revoke");
    t.app
        .htmx()
        .post(
            &format!("/edtech/certificates/{}/revoke", draft.id),
            &[("reason", "issued in error")],
        )
        .await
        .assert_ok();
    let revoked = Certificate::find_or_404(t.db(), draft.id).await.unwrap();
    assert_eq!(revoked.status, CertificateStatus::Revoked);
}
