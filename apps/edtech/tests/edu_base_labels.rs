//! The labels of the 10 EdTech records (base for #442, #443): a document or a
//! bill of another module that names a batch, a session, an enrolment, a
//! certificate or a course shows its number, so a document kind's `blocks`
//! entry on `batch.open` or `certificate.issue` can name them.

mod support;

use edtech::enrolments;
use edtech::{Certificate, Enrolment};
use support::{boot, day, next_weekday};
use wl_kernel::Subject;
use wl_kernel::documents::{labelled_subject_types, subject_label};

/// The label of `subject`, if its type has one and the record is there.
async fn label(t: &support::Centre, subject: Subject) -> Option<String> {
    subject_label(t.db(), &subject).await.unwrap()
}

#[renox::test]
async fn edu_base_the_records_are_named_for_documents_and_bills() {
    let t = boot().await;
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let batch = t.open_batch(&tutor, &room, next_weekday(day(7), 1)).await;
    let student = t.person(1, "Sara Ali");
    let desk = t.desk().await;
    let e: Enrolment = t.enrol(student, &batch).await;
    enrolments::activate(t.state(), Some(&desk), e.id).await.unwrap();
    enrolments::complete(t.state(), e.id).await.unwrap();
    let certificate: Certificate = Certificate::of_enrolment(t.db(), e.id)
        .await
        .unwrap()
        .remove(0);
    let course = t.course().await;
    let session = t.sessions_of(&batch).await.remove(0);

    assert_eq!(label(&t, batch.subject()).await, Some(batch.number()));
    assert_eq!(label(&t, e.subject()).await, Some(e.number()));
    assert_eq!(label(&t, certificate.subject()).await, Some(certificate.label()));
    assert_eq!(label(&t, course.subject()).await, Some(course.title()));
    assert_eq!(
        label(&t, session.subject()).await,
        Some(format!("{} · 1", batch.number()))
    );
    assert_eq!(label(&t, Subject::new("edtech.enrolment", 9999)).await, None);

    let types = labelled_subject_types();
    for kind in [
        "edtech.batch",
        "edtech.session",
        "edtech.enrolment",
        "edtech.certificate",
        "edtech.course",
    ] {
        assert!(types.iter().any(|t| t == kind), "{kind}");
    }
}
