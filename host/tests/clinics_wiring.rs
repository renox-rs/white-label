//! BL-CLN-12 (#360) in the real host: the Clinics client boots with its
//! service types, which need the practitioner licence and the branch
//! privileges, and with the two document kinds that block booking and
//! check-in with override `never` (`SCH.ACL.04`, 05 §4.5).

mod support;

use clinics::seed::REQUIRED_QUALIFICATIONS;
use host::roles::RECEPTIONIST;
use renox::prelude::*;
use support::{clinics_booted, person};
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

/// The licence and the privileges block `booking.book` and
/// `booking.check_in`, never overridden.
#[renox::test]
async fn the_practitioner_documents_block_booking_and_check_in_without_override() {
    let app = clinics_booted().await;
    let documents = guard(app.state());

    for guarded in ["booking.book", "booking.check_in"] {
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
