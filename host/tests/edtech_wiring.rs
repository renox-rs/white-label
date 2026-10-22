//! 10 EdTech in the real host (base for #442, #443, #452): the EdTech client
//! boots with its service types, ticket categories and document kinds, and
//! the roles of 10 §3.1 hold the permissions of the app.

mod support;

use edtech::permissions as edu;
use host::roles;
use renox::prelude::*;
use support::{edtech_booted, person};
use wl_kernel::documents::{BlockMode, OverrideMode, guard};
use wl_scheduling::ServiceType;
use wl_ticketing::Category;

/// The grants of `role` in the host.
fn grants(role: &str) -> &'static [&'static str] {
    roles::ROLES
        .iter()
        .find(|(name, _)| *name == role)
        .map(|(_, grants)| *grants)
        .unwrap_or_else(|| panic!("no role {role}"))
}

/// `[client] app = "edtech"` is a built vertical.
#[test]
fn edu_base_edtech_is_a_built_vertical() {
    assert!(host::VERTICALS.contains(&"edtech"));
}

/// The seed stores the service types (no check-in, no deposit) and the ticket
/// categories; a second install changes nothing.
#[renox::test]
async fn edu_base_the_service_types_and_categories_are_installed_once() {
    let app = edtech_booted().await;

    let types = ServiceType::query()
        .order_by(ServiceType::CODE)
        .get(app.db())
        .await
        .unwrap();
    let codes: Vec<&str> = types.iter().map(|s| s.code.as_str()).collect();
    assert_eq!(codes, ["one_to_one", "placement_test", "trial_class"]);
    for service in &types {
        assert!(!service.check_in_required);
        assert_eq!(service.deposit_amount, 0);
    }
    for spec in edtech::seed::CATEGORIES {
        assert!(
            Category::by_key(app.db(), spec.key)
                .await
                .unwrap()
                .is_some(),
            "{}",
            spec.key
        );
    }
    let again = host::edtech_seed::install(app.state()).await.unwrap();
    assert_eq!(again.total(), 0);
}

/// The centre permit blocks `batch.open` and `certificate.issue` and the
/// police clearance blocks the tutor assignments and flags the attendance;
/// the override of the centre permit is never.
#[renox::test]
async fn edu_base_the_document_kinds_carry_their_blocks() {
    let app = edtech_booted().await;
    let documents = guard(app.state());

    let open = documents
        .blocks_for(app.db(), "batch.open")
        .await
        .unwrap();
    let mut kinds: Vec<&str> = open.iter().map(|e| e.kind.as_str()).collect();
    kinds.sort_unstable();
    assert_eq!(
        kinds,
        ["centre_permit", "course_approval", "police_clearance", "tutor_permit"]
    );
    assert!(open.iter().all(|e| e.mode == BlockMode::Block));
    assert!(open.iter().all(|e| e.override_mode == OverrideMode::Never));

    let attendance = documents
        .blocks_for(app.db(), "session.submit_attendance")
        .await
        .unwrap();
    assert!(attendance.iter().all(|e| e.mode == BlockMode::Flag));
    assert_eq!(attendance.len(), 2);
}

/// The roles of 10 §3.1 exist and hold the app's permissions: the tutor
/// teaches but plans nothing, the coordinator plans, the centre manager
/// approves and exceeds capacity, the counsellor only looks.
#[test]
fn edu_base_the_roles_hold_the_permissions_of_the_spec() {
    assert!(grants(roles::TUTOR).contains(&edu::SESSIONS_TEACH));
    assert!(!grants(roles::TUTOR).contains(&edu::BATCHES_MANAGE));
    assert!(grants(roles::ACADEMIC_COORDINATOR).contains(&edu::BATCHES_MANAGE));
    assert!(!grants(roles::ACADEMIC_COORDINATOR).contains(&edu::BATCHES_APPROVE));
    assert!(grants(roles::CENTRE_MANAGER).contains(&edu::BATCHES_APPROVE));
    assert!(grants(roles::CENTRE_MANAGER).contains(&edu::SEATS_EXCEED));
    assert!(!grants(roles::COUNSELLOR).contains(&edu::SEATS_EXCEED));
    assert!(grants(roles::COUNSELLOR).contains(&edu::BATCHES_VIEW));
    assert!(grants(roles::ADMIN).contains(&edu::COURSES_MANAGE));
}

/// A tutor signs in to the EdTech host and meets the dashboard, not an error.
#[renox::test]
async fn edu_base_a_tutor_signs_in() {
    let app = edtech_booted().await;
    let tutor = person(&app, "Tariq", roles::TUTOR).await;
    app.acting_as(&tutor);
    app.get("/dashboard").await.assert_ok();
}

/// A withdrawal that gives a refund is decided by a holder of
/// `enrolments.approve_refund` (the centre manager, the accountant), never by
/// the counsellor who asked (BL-EDU-03, `FIN.ACL.04`): the client stores the
/// policy, shows it in words on the dashboard and links the enrolment page.
#[renox::test]
async fn edu_03_the_withdrawal_policy_is_stored_in_words_and_decided_by_the_centre() {
    use wl_kernel::approvals::{ApprovalPolicy, Step, action_is_labelled, subject_url};

    let app = edtech_booted().await;
    let policies = ApprovalPolicy::query()
        .where_(ApprovalPolicy::ACTION.eq("edtech.withdraw"))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(policies.len(), 1);
    assert!(policies[0].sod, "the requester never approves");
    assert_eq!(
        policies[0].steps.0,
        [Step::holders_of(edu::ENROLMENTS_REFUND_APPROVE)]
    );
    assert!(action_is_labelled("edtech.withdraw"));
    assert_eq!(
        subject_url("edtech.enrolment", 4).as_deref(),
        Some("/edtech/enrolments/4")
    );
    assert!(grants(roles::CENTRE_MANAGER).contains(&edu::ENROLMENTS_REFUND_APPROVE));
    assert!(grants(roles::ACCOUNTANT).contains(&edu::ENROLMENTS_REFUND_APPROVE));
    assert!(!grants(roles::COUNSELLOR).contains(&edu::ENROLMENTS_REFUND_APPROVE));
}

/// Terminating a running batch is requested by the coordinator and approved by
/// the centre manager, never the requester (BL-EDU-12, `EDU.ACL.15`): the client
/// stores the policy, shows it in words and links the batch page.
#[renox::test]
async fn edu_12_the_termination_policy_is_the_centre_managers_and_in_words() {
    use wl_kernel::approvals::{ApprovalPolicy, Step, action_is_labelled, subject_url};

    let app = edtech_booted().await;
    let policies = ApprovalPolicy::query()
        .where_(ApprovalPolicy::ACTION.eq("edtech.terminate"))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(policies.len(), 1);
    assert!(policies[0].sod, "the requester never approves");
    assert_eq!(policies[0].steps.0, [Step::holders_of(edu::BATCHES_APPROVE)]);
    assert!(action_is_labelled("edtech.terminate"));
    assert_eq!(
        subject_url("edtech.batch", 9).as_deref(),
        Some("/edtech/batches/9")
    );
    assert!(grants(roles::ACADEMIC_COORDINATOR).contains(&edu::BATCHES_MANAGE));
    assert!(!grants(roles::ACADEMIC_COORDINATOR).contains(&edu::BATCHES_APPROVE));
    assert!(grants(roles::CENTRE_MANAGER).contains(&edu::BATCHES_APPROVE));
}

/// A blocked payer or student pauses their enrolments (BL-EDU-04,
/// wl-parties §5.6): the kind `enrolment` has the block effect *pause*, decided
/// by the centre manager, and only the centre manager withdraws an enrolment
/// while it is blocked.
#[renox::test]
async fn edu_04_the_enrolment_pauses_on_a_block_and_the_centre_manager_decides() {
    use renox::auth::permissions::Scope;
    use wl_kernel::block_effects::{BlockEffect, effect_for};

    let _app = edtech_booted().await;
    let rule = effect_for("enrolment", &Scope::global());
    assert_eq!(rule.effect, BlockEffect::Pause);
    assert_eq!(rule.decider_role.as_deref(), Some(edu::CENTRE_MANAGER_ROLE));
    assert!(grants(roles::CENTRE_MANAGER).contains(&edu::ENROLMENTS_APPROVE));
    assert!(!grants(roles::COUNSELLOR).contains(&edu::ENROLMENTS_APPROVE));
}
