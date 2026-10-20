//! BL-PRP-25 (#275): compliance flags of buildings and expired compliance
//! certificates show on the "Buildings non-compliant" tile; leasing is not
//! blocked.

mod support;

use property::Building;
use property::compliance::{BuildingFlag, CERTIFICATE_EXPIRED, non_compliant};
use property::notices::CERTIFICATE_EXPIRED as CERTIFICATE_NOTICE;
use renox::chrono::Duration;
use wl_kernel::Subject;
use wl_kernel::documents::{DocumentAccepted, DocumentExpired, DocumentId};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::tickets::{ComplianceFlagCleared, ComplianceFlagRaised};

use support::{ACCOUNTANT, AGENT, MANAGER, aed, boot, day};

fn subject(building: &Building) -> Subject {
    Subject::new("property.building", building.id)
}

fn expired(building: &Building, days_ago: i64) -> DocumentExpired {
    DocumentExpired {
        document: DocumentId(1),
        subject: subject(building),
        kind: "building_safety_certificate".to_owned(),
        expires_on: day(-days_ago),
        blocks: Vec::new(),
        blocked: Vec::new(),
    }
}

/// AC2: the day after a certificate expires the building shows on the tile
/// with the cause `certificate_expired`, the company admin is notified, and a
/// new lease on the building is still activated; a renewed certificate clears
/// the flag.
#[renox::test]
async fn bl_prp_25_an_expired_certificate_flags_the_building_but_not_the_leases() {
    let t = boot().await;
    let tower_a = t.building_coded("TWR-A").await;
    let admin = t.user_with("Ada", wl_kernel::COMPANY_ADMIN_ROLE).await;
    let agent = t.user_in("Ali", AGENT, tower_a.id).await;
    let manager = t.user_in("Sami", MANAGER, tower_a.id).await;

    t.state().emit(expired(&tower_a, 1)).await.unwrap();
    let open = BuildingFlag::open(t.db()).await.unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].cause, CERTIFICATE_EXPIRED);
    assert_eq!(open[0].category, "building_safety_certificate");
    assert_eq!(open[0].building_id, tower_a.id);
    assert_eq!(non_compliant(t.db()).await.unwrap(), vec![tower_a.id]);

    let told = t.notices.of_type(CERTIFICATE_NOTICE);
    assert_eq!(told.len(), 1);
    assert!(
        told[0]
            .recipients
            .iter()
            .any(|r| matches!(r, NotifyRecipient::User { id } if *id == admin.id))
    );

    // The same event twice raises one flag.
    t.state().emit(expired(&tower_a, 1)).await.unwrap();
    assert_eq!(BuildingFlag::open(t.db()).await.unwrap().len(), 1);

    // Leasing is not blocked.
    let unit = t.unit(&tower_a, "501").await;
    let lease = t.active_lease(&unit, &agent, &manager).await;
    assert_eq!(t.status(&lease).await, property::LeaseStatus::Active);

    // The tile and the list.
    t.app.acting_as(&manager);
    t.app
        .get("/property/compliance")
        .await
        .assert_ok()
        .assert_see("Tower TWR-A")
        .assert_see("Certificate expired");

    // A renewed certificate that has not expired clears the flag; one that
    // is already expired does not.
    let renewed = |expires_on| DocumentAccepted {
        document: DocumentId(2),
        subject: subject(&tower_a),
        kind: "building_safety_certificate".to_owned(),
        expires_on,
    };
    t.state()
        .emit(renewed(Some(day(0) - Duration::days(2))))
        .await
        .unwrap();
    assert_eq!(BuildingFlag::open(t.db()).await.unwrap().len(), 1);
    t.state().emit(renewed(Some(day(365)))).await.unwrap();
    assert!(BuildingFlag::open(t.db()).await.unwrap().is_empty());
    let _ = aed(1);
}

/// AC3: a `visit_missed` flag on Tower B shows on the tile and disappears
/// when the flag clears. A flag on something that is not a building is not
/// shown here.
#[renox::test]
async fn bl_prp_25_a_ticketing_flag_shows_until_it_clears() {
    let t = boot().await;
    let tower_b = t.building_coded("TWR-B").await;
    let manager = t.user_in("Sami", MANAGER, tower_b.id).await;
    let raised = ComplianceFlagRaised {
        subject: subject(&tower_b),
        category: "fire_safety".to_owned(),
        reason: "visit_missed".to_owned(),
        contract: None,
    };
    t.state().emit(raised.clone()).await.unwrap();
    t.state()
        .emit(ComplianceFlagRaised {
            subject: Subject::new("property.building_system", 4),
            ..raised.clone()
        })
        .await
        .unwrap();
    assert_eq!(non_compliant(t.db()).await.unwrap(), vec![tower_b.id]);

    t.app.acting_as(&manager);
    t.app
        .get("/property")
        .await
        .assert_ok()
        .assert_see("Buildings non-compliant (1)");
    t.app
        .get("/property/compliance")
        .await
        .assert_ok()
        .assert_see("Planned visit missed");

    t.state()
        .emit(ComplianceFlagCleared {
            subject: subject(&tower_b),
            category: raised.category,
            reason: raised.reason,
            contract: None,
        })
        .await
        .unwrap();
    assert!(non_compliant(t.db()).await.unwrap().is_empty());
    t.app
        .get("/property/compliance")
        .await
        .assert_ok()
        .assert_see("No building is flagged");
}

/// Only holders of the permission see the page, and a property manager sees
/// the buildings they hold it in.
#[renox::test]
async fn bl_prp_25_the_tile_is_for_the_managers_of_the_buildings() {
    let t = boot().await;
    let a = t.building_coded("TWR-A").await;
    let b = t.building_coded("TWR-B").await;
    let manager_a = t.user_in("Sami", MANAGER, a.id).await;
    let accountant = t.user_with("Amal", ACCOUNTANT).await;
    for building in [&a, &b] {
        t.state()
            .emit(ComplianceFlagRaised {
                subject: subject(building),
                category: "lifts".to_owned(),
                reason: "no_contract".to_owned(),
                contract: None,
            })
            .await
            .unwrap();
    }
    t.app.acting_as(&manager_a);
    t.app
        .get("/property/compliance")
        .await
        .assert_ok()
        .assert_see("Tower TWR-A")
        .assert_dont_see("Tower TWR-B");
    t.app.acting_as(&accountant);
    t.app.get("/property/compliance").await.assert_forbidden();
}
