//! Refusing a guarded transition while a required document is expired or
//! missing (wl-documents §5.6): the acceptance checks AC1–AC6, AC8 and the
//! no-op half of AC10 of #45.
//!
//! No module guards a transition yet, so the checks use the test-registered
//! `BlockSubjects` providers of `support` and call the kernel guard the way
//! a guarded module will, before its `record.transition(..)`.

mod support;

use renox::prelude::*;
use renox::testing::TestApp;
use wl_documents::DocumentStatus;
use wl_kernel::documents::{self, InvalidReason, Validity};

use support::{
    Booted, FIRE_TICKET, PLUMBING_TICKET, RAVI, boot, civil_defence, day, driving_licence, ravi,
    residence_visa, safefire, truck, vehicle_insurance,
};

const ACTIVATE: &str = "deployment.activate";

/// The refusal message of `check`, or a panic when it is allowed.
async fn refusal(
    t: &Booted,
    guarded: &str,
    record: i64,
    on_date: renox::chrono::NaiveDate,
) -> documents::BlockRefusal {
    documents::guard(t.state())
        .check(t.state(), guarded, record, on_date)
        .await
        .expect("the check runs")
        .expect_err("the transition is refused")
}

/// Whether `check` allows the transition.
async fn allowed(
    t: &Booted,
    guarded: &str,
    record: i64,
    on_date: renox::chrono::NaiveDate,
) -> bool {
    documents::guard(t.state())
        .check(t.state(), guarded, record, on_date)
        .await
        .expect("the check runs")
        .is_ok()
}

/// AC1: Ravi's residence visa expired; activating his deployment is refused
/// with the document, the date, who renews and that no override is allowed.
#[renox::test]
async fn an_expired_visa_refuses_the_deployment() {
    let t = boot().await;
    t.install(&[residence_visa()]).await;
    let rita = t.reviewer().await;
    let expiry = day(20);
    t.accepted(&rita, ravi(), "residence_visa", Some(expiry))
        .await;

    let refused = refusal(&t, ACTIVATE, RAVI, day(21)).await;
    let expected = format!("Residence visa expired on {expiry}; renew: PRO; override: not allowed");
    assert_eq!(refused.message(), expected);
    assert_eq!(refused.guarded, ACTIVATE);
    assert_eq!(refused.record_id, RAVI);
    assert_eq!(refused.failures.len(), 1);
    assert_eq!(refused.failures[0].subject, ravi());
    assert!(!refused.failures[0].validity.is_valid());

    match refused.into_error() {
        Error::Status(code, message) => {
            assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(message, expected);
        }
        other => panic!("unexpected error {other:?}"),
    }
}

/// AC2: the visa is valid through its expiry date.
#[renox::test]
async fn the_visa_is_valid_through_its_expiry_date() {
    let t = boot().await;
    t.install(&[residence_visa()]).await;
    let rita = t.reviewer().await;
    let expiry = day(20);
    t.accepted(&rita, ravi(), "residence_visa", Some(expiry))
        .await;

    assert!(allowed(&t, ACTIVATE, RAVI, expiry).await);
    assert!(allowed(&t, ACTIVATE, RAVI, day(1)).await);
    assert!(!allowed(&t, ACTIVATE, RAVI, day(21)).await);
}

/// AC3: a renewal waiting for review is named in the refusal, and the
/// review queue lists it ahead of older pending documents.
#[renox::test]
async fn a_pending_renewal_is_named_and_comes_first_in_the_queue() {
    let t = boot().await;
    t.install(&[residence_visa(), driving_licence()]).await;
    let rita = t.reviewer().await;
    let expiry = day(20);
    t.accepted(&rita, ravi(), "residence_visa", Some(expiry))
        .await;

    // An older document waits in the queue; then the renewal is uploaded.
    let older = t
        .attach(
            wl_kernel::Subject::new("hr.employee", 8),
            "driving_licence",
            Some(day(300)),
        )
        .await;
    let renewal = t.attach(ravi(), "residence_visa", Some(day(400))).await;
    assert_eq!(renewal.status, DocumentStatus::PendingReview);
    assert!(renewal.review_priority_at.is_none());

    let queue = |body: &str| {
        let visa = body.find("Residence visa").expect("the renewal is listed");
        let licence = body
            .find("Driving licence")
            .expect("the older one is listed");
        (visa, licence)
    };
    t.app.acting_as(&rita);
    let page = t.app.get("/documents/review").await;
    page.assert_ok();
    let (visa, licence) = queue(&page.text());
    assert!(licence < visa, "oldest first until a transition waits");

    let refused = refusal(&t, ACTIVATE, RAVI, day(21)).await;
    assert_eq!(
        refused.message(),
        format!(
            "Residence visa expired on {expiry}; renew: PRO; override: not allowed; \
             renewal pending review since {}",
            wl_kernel::documents::today()
        )
    );
    let Validity::Invalid { pending, .. } = &refused.failures[0].validity else {
        panic!("the visa is invalid");
    };
    assert_eq!(pending.as_ref().unwrap().document, renewal.document_id());

    assert!(t.reload(&renewal).await.review_priority_at.is_some());
    assert!(t.reload(&older).await.review_priority_at.is_none());
    let page = t.app.get("/documents/review").await;
    page.assert_ok();
    let (visa, licence) = queue(&page.text());
    assert!(visa < licence, "the renewal a refusal waits for is first");
}

/// AC4: a valid driver licence and expired vehicle insurance refuse
/// `run.dispatch`, and the message names only the insurance.
#[renox::test]
async fn the_message_names_only_the_document_that_fails() {
    let t = boot().await;
    t.install(&[driving_licence(), vehicle_insurance()]).await;
    let rita = t.reviewer().await;
    t.accepted(&rita, ravi(), "driving_licence", Some(day(200)))
        .await;
    let expired = day(-3);
    t.accepted(&rita, truck(), "vehicle_insurance", Some(expired))
        .await;

    let refused = refusal(&t, "run.dispatch", RAVI, documents::today()).await;
    assert_eq!(refused.failures.len(), 1);
    assert_eq!(refused.failures[0].subject, truck());
    assert_eq!(
        refused.message(),
        format!(
            "Vehicle insurance expired on {expired}; renew: fleet_manager; override: not allowed"
        )
    );
    assert!(!refused.message().contains("Driving licence"));
}

/// AC5: once the renewal is accepted the old visa is superseded, the
/// guard answers yes for a date inside the old visa's term, and a check
/// right after the acceptance is allowed (nothing is cached).
#[renox::test]
async fn an_accepted_renewal_makes_the_next_check_allowed() {
    let t = boot().await;
    t.install(&[residence_visa()]).await;
    let rita = t.reviewer().await;
    let old = t
        .accepted(&rita, ravi(), "residence_visa", Some(day(20)))
        .await;
    let renewal = t.attach(ravi(), "residence_visa", Some(day(400))).await;
    assert!(!allowed(&t, ACTIVATE, RAVI, day(21)).await);

    let renewal = t.accept(&rita, &renewal).await;
    assert_eq!(renewal.status, DocumentStatus::Accepted);
    assert_eq!(t.reload(&old).await.status, DocumentStatus::Superseded);

    let guard = documents::guard(t.state());
    let validity = guard
        .is_valid(t.db(), &ravi(), "residence_visa", day(10))
        .await
        .unwrap();
    assert!(validity.is_valid(), "{validity:?}");
    assert!(allowed(&t, ACTIVATE, RAVI, day(21)).await);
}

/// AC6: no document, or only a rejected or pending one, answers no.
#[renox::test]
async fn a_missing_rejected_or_pending_document_answers_no() {
    let t = boot().await;
    t.install(&[residence_visa()]).await;
    let rita = t.reviewer().await;
    let guard = documents::guard(t.state());
    let on = day(1);

    // No document at all.
    let nobody = 11;
    let validity = guard
        .is_valid(
            t.db(),
            &wl_kernel::Subject::new("hr.employee", nobody),
            "residence_visa",
            on,
        )
        .await
        .unwrap();
    assert!(matches!(
        validity,
        Validity::Invalid {
            reason: InvalidReason::Missing,
            document: None,
            ..
        }
    ));
    assert_eq!(
        refusal(&t, ACTIVATE, nobody, on).await.message(),
        "Residence visa missing; renew: PRO; override: not allowed"
    );

    // Only a pending one.
    let waiting = 12;
    let pending = t
        .attach(
            wl_kernel::Subject::new("hr.employee", waiting),
            "residence_visa",
            Some(day(400)),
        )
        .await;
    let validity = guard
        .is_valid(t.db(), &pending.subject(), "residence_visa", on)
        .await
        .unwrap();
    assert!(matches!(
        validity,
        Validity::Invalid {
            reason: InvalidReason::PendingReview,
            ..
        }
    ));
    assert!(!allowed(&t, ACTIVATE, waiting, on).await);

    // Only a rejected one.
    let turned_down = 13;
    let rejected = t
        .attach(
            wl_kernel::Subject::new("hr.employee", turned_down),
            "residence_visa",
            Some(day(400)),
        )
        .await;
    t.app.acting_as(&rita);
    t.app
        .htmx()
        .post(
            &format!("/documents/{}/reject", rejected.id),
            &[("reason", "Unreadable scan")],
        )
        .await
        .assert_ok();
    assert_eq!(t.reload(&rejected).await.status, DocumentStatus::Rejected);
    let validity = guard
        .is_valid(t.db(), &rejected.subject(), "residence_visa", on)
        .await
        .unwrap();
    assert!(matches!(
        validity,
        Validity::Invalid {
            reason: InvalidReason::Rejected,
            ..
        }
    ));
    assert!(!allowed(&t, ACTIVATE, turned_down, on).await);
}

/// AC8: SafeFire's expired Civil Defence approval refuses its fire alarm
/// ticket and not its plumbing ticket (the condition does not match).
#[renox::test]
async fn the_condition_decides_which_tickets_are_refused() {
    let t = boot().await;
    t.install(&[civil_defence()]).await;
    let rita = t.reviewer().await;
    let expired = day(-10);
    t.accepted(&rita, safefire(), "civil_defence_approval", Some(expired))
        .await;
    let today = documents::today();

    let refused = refusal(&t, "ticket.resolve", FIRE_TICKET, today).await;
    assert_eq!(
        refused.message(),
        format!(
            "Civil Defence approval expired on {expired}; renew: vendor_manager; \
             override: not allowed"
        )
    );
    assert!(allowed(&t, "ticket.resolve", PLUMBING_TICKET, today).await);
}

/// An entry whose transition has no provider is an error, never a yes.
#[renox::test]
async fn a_guarded_transition_without_a_provider_is_never_allowed() {
    let t = boot().await;
    t.install(&[wl_documents::KindSeed::new("orphan_doc", "Orphan")
        .subject_type("hr.employee")
        .blocks("orphan_unit.go", wl_kernel::documents::BlockMode::Block)])
        .await;
    let result = documents::guard(t.state())
        .check(t.state(), "orphan_unit.go", 1, documents::today())
        .await;
    assert!(result.is_err());
}

/// AC10: without wl-documents the guard is the no-op: no blocks, always
/// valid, every check allowed.
#[renox::test]
async fn without_the_module_the_guard_is_the_no_op() {
    let app = TestApp::new(App::new()).await;
    let state = app.state();
    let guard = documents::guard(state);
    assert!(
        guard
            .blocks_for(app.db(), ACTIVATE)
            .await
            .unwrap()
            .is_empty()
    );
    let validity = guard
        .is_valid(app.db(), &ravi(), "residence_visa", documents::today())
        .await
        .unwrap();
    assert!(validity.is_valid());
    let outcome = guard
        .check(state, ACTIVATE, RAVI, documents::today())
        .await
        .unwrap();
    assert!(outcome.is_ok());
}
