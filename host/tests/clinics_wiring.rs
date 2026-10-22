//! BL-CLN-12 (#360) in the real host: the Clinics client boots with its
//! service types, which need the practitioner licence and the branch
//! privileges, and with the two document kinds that block booking and
//! check-in with override `never` (`SCH.ACL.04`, 05 §4.5).

mod support;

use clinics::seed::REQUIRED_QUALIFICATIONS;
use host::roles::RECEPTIONIST;
use renox::prelude::*;
use support::{clinics_booted, person};
use wl_kernel::approvals::{ApprovalPolicy, Step, action_is_labelled};
use wl_kernel::documents::{BlockMode, OverrideMode, guard};
use wl_scheduling::ServiceType;

/// The seed stores the service types, each checking the patient in and
/// needing both qualifications; a second install changes nothing.
#[renox::test]
async fn the_service_types_need_the_licence_and_the_privileges() {
    let app = clinics_booted().await;

    let types = ServiceType::query()
        .order_by(ServiceType::CODE)
        .get(app.db())
        .await
        .unwrap();

    let codes: Vec<&str> = types.iter().map(|s| s.code.as_str()).collect();
    assert_eq!(codes, ["consultation", "follow_up", "procedure"]);
    for service in &types {
        assert_eq!(service.required_qualifications, REQUIRED_QUALIFICATIONS);
        assert!(service.check_in_required && service.track_in_progress);
    }
    let again = host::clinics_seed::install(app.state()).await.unwrap();
    assert_eq!(again.total(), 0);
}

/// Every approval the Clinics client can ask for shows in words on the
/// dashboard ("Decide: Record a payer's remittance"), never as its key
/// (`invoicing.payer_money`).
#[renox::test]
async fn every_approval_action_has_words() {
    let app = clinics_booted().await;
    let policies = ApprovalPolicy::query().get(app.db()).await.unwrap();
    assert!(
        policies.len() > 20,
        "the client stores its approval policies"
    );
    let raw: Vec<&str> = policies
        .iter()
        .map(|policy| policy.action.as_str())
        .filter(|action| !action_is_labelled(action))
        .collect();
    assert!(raw.is_empty(), "approvals shown as a key: {raw:?}");
}

/// A payer bill's write-off above the limit is the owner's to approve
/// (`FIN.ACL.14`): the Clinics client builds wl-invoicing with the owner and the
/// company administrators as approvers, so a new database stores that. A
/// database that was made with wl-invoicing's own default keeps its row
/// through `wl:install`; the clinics install brings that row to the owner and
/// leaves a row an admin changed as it is.
#[renox::test]
async fn the_write_off_of_a_payer_bill_is_the_owners_to_approve() {
    let app = clinics_booted().await;
    let owner = Step::one_of(["owner", "admin"]);
    let stored = || async {
        ApprovalPolicy::query()
            .where_(ApprovalPolicy::ACTION.eq("invoicing.payer_write_off"))
            .get(app.db())
            .await
            .unwrap()
    };
    let policies = stored().await;
    assert_eq!(policies.len(), 1);
    assert_eq!(policies[0].steps.0, std::slice::from_ref(&owner));

    // Made by an earlier build: the module's default, with the manager's permission.
    let mut policy = policies[0].clone();
    policy.steps = renox::db::Json(vec![
        Step::one_of(["admin"]).or_permission("payer_bills.write_off"),
    ]);
    policy.save(app.db()).await.unwrap();
    host::clinics_seed::install(app.state()).await.unwrap();
    assert_eq!(stored().await[0].steps.0, std::slice::from_ref(&owner));

    // An admin's own choice is kept.
    let mut policy = stored().await.remove(0);
    policy.steps = renox::db::Json(vec![Step::one_of(["accountant"])]);
    policy.save(app.db()).await.unwrap();
    host::clinics_seed::install(app.state()).await.unwrap();
    assert_eq!(stored().await[0].steps.0, [Step::one_of(["accountant"])]);
}

/// The licence and the privileges block `booking.book`, `booking.check_in`
/// and `booking.start` (05 §4.5), never overridden.
#[renox::test]
async fn the_practitioner_documents_block_booking_and_check_in_without_override() {
    let app = clinics_booted().await;
    let documents = guard(app.state());

    for guarded in ["booking.book", "booking.check_in", "booking.start"] {
        let entries = documents.blocks_for(app.db(), guarded).await.unwrap();
        let mut kinds: Vec<&str> = entries.iter().map(|e| e.kind.as_str()).collect();
        kinds.sort_unstable();
        assert_eq!(kinds, ["branch_privileges", "practitioner_licence"]);
        assert!(entries.iter().all(|e| e.mode == BlockMode::Block));
        assert!(
            entries
                .iter()
                .all(|e| e.override_mode == OverrideMode::Never)
        );
    }
}

/// The receptionist reaches the insurance policies; someone with no role
/// does not.
#[renox::test]
async fn the_receptionist_reaches_the_insurance_policies() {
    let app = clinics_booted().await;
    let rita = person(&app, "Rita", RECEPTIONIST).await;
    let nobody = person(&app, "Nora", host::roles::MEMBER).await;

    app.acting_as(&rita);
    app.get("/clinics/policies").await.assert_ok();
    app.acting_as(&nobody);
    app.get("/clinics/policies").await.assert_forbidden();
}

/// 05 §4.5 and `DOC.RUL.03` through the real wl-documents: an expired
/// practitioner licence tells the holders of the responsible role (the clinic
/// manager) with the documents' renewal notice and the owner nothing, because
/// the kinds are not compliance-critical; the owner is told by the clinic's own
/// `CLN.RUL.01` message (proved in `apps/clinics/tests/licences.rs`).
///
/// Known limit, kept visible here: the clinic manager gets two messages for one
/// expiry, `documents.expired` from `DOC.RUL.03` (the renewal notice) and
/// `clinics.licence_expired` from `CLN.RUL.01` (the appointments affected).
/// wl-documents has no seam to suppress the first.
#[renox::test]
async fn doc_rul_03_tells_the_manager_and_not_the_owner() {
    use host::roles::{CLINIC_MANAGER, OWNER};
    use renox::chrono::Days;
    use wl_documents::{Document, DocumentKind, DocumentStatus, expiry};
    use wl_kernel::documents::today;
    use wl_notify::Message;

    let app = clinics_booted().await;
    let manager = person(&app, "Mona", CLINIC_MANAGER).await;
    let owner = person(&app, "Olga", OWNER).await;
    let kind = DocumentKind::by_key(app.db(), "practitioner_licence")
        .await
        .unwrap()
        .expect("the kind is seeded");
    assert!(!kind.compliance_critical);
    let expired_on = today().checked_sub_days(Days::new(1)).unwrap();
    Document::create(
        app.db(),
        Document {
            subject_type: "hr.employee".into(),
            subject_id: 1,
            kind_id: kind.id,
            title: "Dr K licence".into(),
            expiry_date: Some(expired_on),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let report = expiry::run(app.state(), today()).await.unwrap();
    assert_eq!((report.expired_events, report.failed), (1, 0));

    let notices = Message::query()
        .where_(Message::TYPE_KEY.eq("documents.expired"))
        .order_by(Message::ID)
        .get(app.db())
        .await
        .unwrap();
    let told: Vec<i64> = notices.iter().filter_map(|m| m.user_id).collect();
    assert_eq!(
        told,
        [manager.id],
        "the manager is told once, the owner never"
    );
    assert!(!told.contains(&owner.id));
}
