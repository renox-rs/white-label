//! PRP.PRT.01 and PRP.PRT.02: the portfolio, the units and their status
//! machine; the pages that show and change them.

mod support;

use property::unit_machine::{
    CANCEL_NOTICE, MARK_NOTICE, MARK_READY, OCCUPY, RELEASE_BY_SYSTEM, RELIST, RESERVE, VACATE,
    VACATE_TO_MAINTENANCE, WITHDRAW,
};
use property::{
    Building, NewBuilding, NewUnit, Unit, UnitStatus, UnitType, UnitUse, create_building,
    create_unit, ensure_floor,
};
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};

use support::{MANAGER, boot};

/// A building takes an upper-case code and a default emirate; a second one
/// with the same code is refused.
#[renox::test]
async fn prp_prt_01_a_building_has_a_unique_code() {
    let t = boot().await;
    let building = create_building(
        t.db(),
        NewBuilding {
            code: " twr-a ".into(),
            name_en: "Tower A".into(),
            name_ar: Some("برج أ".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(building.code, "TWR-A");
    assert_eq!(building.emirate, "dubai");
    assert_eq!(building.display_name("ar"), "برج أ");
    let again = create_building(
        t.db(),
        NewBuilding {
            code: "TWR-A".into(),
            name_en: "Another".into(),
            ..Default::default()
        },
    )
    .await;
    assert!(matches!(
        again,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
    let nameless = create_building(
        t.db(),
        NewBuilding {
            code: "B".into(),
            ..Default::default()
        },
    )
    .await;
    assert!(nameless.is_err());
}

/// A unit has its attributes, starts available, and its number is unique in
/// its building; a floor is added once.
#[renox::test]
async fn prp_prt_01_a_unit_has_attributes_and_starts_available() {
    let t = boot().await;
    let building = t.building().await;
    let floor = ensure_floor(t.db(), building.id, 12, "Level 12")
        .await
        .unwrap();
    let again = ensure_floor(t.db(), building.id, 12, "ignored")
        .await
        .unwrap();
    assert_eq!(floor.id, again.id);
    let unit = create_unit(
        t.db(),
        NewUnit {
            building_id: building.id,
            floor_id: Some(floor.id),
            number: "1204".into(),
            unit_type: UnitType::Apartment,
            usage: UnitUse::Residential,
            area_sqft_milli: Some(1_250_500),
            bedrooms: Some(2),
            asking_rent: Some(9_000_000),
            notes: Some("  corner unit ".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(unit.status, UnitStatus::Available);
    assert_eq!(unit.notes.as_deref(), Some("corner unit"));
    let duplicate = create_unit(
        t.db(),
        NewUnit {
            building_id: building.id,
            number: "1204".into(),
            ..Default::default()
        },
    )
    .await;
    assert!(duplicate.is_err());
    let no_building = create_unit(
        t.db(),
        NewUnit {
            building_id: 999,
            number: "1".into(),
            ..Default::default()
        },
    )
    .await;
    assert!(no_building.is_err());
}

/// PRP.PRT.02: the system moves a unit through occupy, notice and vacate.
#[renox::test]
async fn prp_prt_02_the_system_drives_the_unit() {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;

    let run = |unit: Unit, name: &'static str| {
        let state = t.state().clone();
        async move { unit.transition(name).by_system().run(&state).await }
    };
    let _outcome = run(unit.clone(), OCCUPY).await.unwrap();
    assert_eq!(t.unit_now(&unit).await.status, UnitStatus::Occupied);
    let _outcome = run(t.unit_now(&unit).await, MARK_NOTICE).await.unwrap();
    assert_eq!(t.unit_now(&unit).await.status, UnitStatus::NoticeGiven);
    let _outcome = run(t.unit_now(&unit).await, CANCEL_NOTICE).await.unwrap();
    assert_eq!(t.unit_now(&unit).await.status, UnitStatus::Occupied);
    let _outcome = run(t.unit_now(&unit).await, MARK_NOTICE).await.unwrap();
    let _outcome = run(t.unit_now(&unit).await, VACATE).await.unwrap();
    assert_eq!(t.unit_now(&unit).await.status, UnitStatus::Available);

    let _outcome = run(t.unit_now(&unit).await, OCCUPY).await.unwrap();
    let _outcome = run(t.unit_now(&unit).await, VACATE_TO_MAINTENANCE)
        .await
        .unwrap();
    assert_eq!(t.unit_now(&unit).await.status, UnitStatus::UnderMaintenance);
    // A unit cannot be occupied twice, nor released unless reserved.
    let refused = run(t.unit_now(&unit).await, OCCUPY).await;
    assert!(matches!(refused, Err(TransitionError::NotFrom(_))));
    let refused = run(t.unit_now(&unit).await, RELEASE_BY_SYSTEM).await;
    assert!(matches!(refused, Err(TransitionError::NotFrom(_))));
}

/// A user may not run a system transition; `reserve` waits for the booking
/// deposit of reservations.
#[renox::test]
async fn prp_prt_02_a_user_cannot_run_a_system_transition_and_reserve_is_later() {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    t.post_plain(
        &manager,
        &format!("/property/units/{}/transitions/{OCCUPY}", unit.id),
        &[],
    )
    .await
    .assert_forbidden();
    let reserve = t
        .post_plain(
            &manager,
            &format!("/property/units/{}/transitions/{RESERVE}", unit.id),
            &[],
        )
        .await;
    reserve.assert_status(422);
    assert_eq!(t.unit_now(&unit).await.status, UnitStatus::Available);
}

/// The property manager of a building takes a unit off the market and puts it
/// back; the page offers exactly those transitions, in their building.
#[renox::test]
async fn prp_prt_02_a_manager_withdraws_and_relists_a_unit_in_their_building() {
    let t = boot().await;
    let mine = t.building().await;
    let other = t.building_coded("TWR-B").await;
    let unit = t.unit(&mine, "1204").await;
    let foreign = t.unit(&other, "301").await;
    let manager = t.user_in("Mona", MANAGER, mine.id).await;

    t.post_plain(
        &manager,
        &format!("/property/units/{}/transitions/{WITHDRAW}", unit.id),
        &[],
    )
    .await
    .assert_status(303);
    assert_eq!(t.unit_now(&unit).await.status, UnitStatus::OffMarket);
    t.post_plain(
        &manager,
        &format!("/property/units/{}/transitions/{RELIST}", unit.id),
        &[],
    )
    .await
    .assert_status(303);
    assert_eq!(t.unit_now(&unit).await.status, UnitStatus::Available);

    // Not in the other building.
    t.post_plain(
        &manager,
        &format!("/property/units/{}/transitions/{WITHDRAW}", foreign.id),
        &[],
    )
    .await
    .assert_forbidden();
    assert_eq!(t.unit_now(&foreign).await.status, UnitStatus::Available);
    // `mark_ready` needs a unit under maintenance.
    t.post_plain(
        &manager,
        &format!("/property/units/{}/transitions/{MARK_READY}", unit.id),
        &[],
    )
    .await
    .assert_status(422);
}

/// The portfolio page lists the buildings the user may see with their
/// occupancy; the building page lists the units.
#[renox::test]
async fn prp_prt_01_the_pages_show_the_buildings_and_their_units() {
    let t = boot().await;
    let mine = t.building().await;
    let other = t.building_coded("TWR-B").await;
    let one = t.unit(&mine, "1204").await;
    t.unit(&mine, "1205").await;
    t.unit(&other, "301").await;
    let _outcome = one
        .transition(OCCUPY)
        .by_system()
        .run(t.state())
        .await
        .unwrap();
    let manager = t.user_in("Mona", MANAGER, mine.id).await;
    t.app.acting_as(&manager);

    t.app
        .get("/property")
        .await
        .assert_ok()
        .assert_view("property/index.html")
        .assert_see("Tower TWR-A")
        .assert_see("Occupancy (%)")
        .assert_dont_see("Tower TWR-B");
    t.app
        .get(&format!("/property/buildings/{}", mine.id))
        .await
        .assert_ok()
        .assert_view("property/building.html")
        .assert_see("1204")
        .assert_see("1205");
    t.app
        .get(&format!("/property/buildings/{}", other.id))
        .await
        .assert_forbidden();
}

/// The forms add a building and a unit; a user without `units.manage` is
/// refused.
#[renox::test]
async fn prp_prt_01_the_forms_add_a_building_and_a_unit() {
    let t = boot().await;
    let admin = t.user_with("Ada", wl_kernel::COMPANY_ADMIN_ROLE).await;
    let created = t
        .post(
            &admin,
            "/property/buildings",
            &[
                ("code", "tw-c"),
                ("name_en", "Tower C"),
                ("emirate", "dubai"),
            ],
        )
        .await;
    created.assert_ok();
    let building = Building::query()
        .where_(Building::CODE.eq("TW-C"))
        .first(t.db())
        .await
        .unwrap()
        .expect("the building was added");
    t.post(
        &admin,
        &format!("/property/buildings/{}/units", building.id),
        &[
            ("number", "7"),
            ("unit_type", "shop"),
            ("usage", "commercial"),
            ("area", "900.5"),
            ("asking_rent", "250,000"),
        ],
    )
    .await
    .assert_ok();
    let unit = Unit::of_building(t.db(), building.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(unit.unit_type, UnitType::Shop);
    assert_eq!(unit.usage, UnitUse::Commercial);
    assert_eq!(unit.area_sqft_milli, Some(900_500));
    assert_eq!(unit.asking_rent, Some(25_000_000));

    let agent = t.user_in("Ali", support::AGENT, building.id).await;
    t.post(
        &agent,
        &format!("/property/buildings/{}/units", building.id),
        &[("number", "8")],
    )
    .await
    .assert_forbidden();
}
