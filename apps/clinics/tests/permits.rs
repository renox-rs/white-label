//! The advertising permit of a campaign (05 section 9, `CLN.ACL.13`;
//! BL-CLN-15): the medical director records the number and the validity, a
//! permit is valid through its last day, a campaign that promotes no service
//! needs none, and only the medical director records. The guard that refuses
//! an activation is wl-sales' `CampaignGuard`, proved through the real host.

mod support;

use clinics::permits::{self, AdvertisingPermit, NewPermit, Problem};
use renox::prelude::*;
use support::{boot, day};

#[renox::test]
async fn the_medical_director_records_a_permit_and_it_is_valid_through_its_last_day() {
    let t = boot().await;
    let owner = t
        .user("Olga", &[clinics::permissions::PERMITS_MANAGE])
        .await;
    let owner = t.auth(&owner).await;
    let marketing = t.user("Mona", &[clinics::permissions::PERMITS_VIEW]).await;
    let marketing = t.auth(&marketing).await;

    // Nothing recorded: a campaign is taken to promote services.
    assert_eq!(
        permits::problem(t.db(), 7, day(0)).await.unwrap(),
        Some(Problem::Missing)
    );

    // Only the holder of `advertising_permits.manage` records.
    let refused = permits::record(t.state(), &marketing, NewPermit::new(7, "AD-77", day(30))).await;
    assert!(matches!(refused, Err(Error::Forbidden)));

    let permit = permits::record(t.state(), &owner, NewPermit::new(7, " AD-77 ", day(30)))
        .await
        .unwrap();
    assert_eq!(permit.permit_number, "AD-77");
    assert_eq!(permit.approved_by, Some(owner.id));
    assert_eq!(permits::problem(t.db(), 7, day(30)).await.unwrap(), None);
    assert_eq!(
        permits::problem(t.db(), 7, day(31)).await.unwrap(),
        Some(Problem::Lapsed)
    );
    assert_eq!(permits::last_day(t.db(), 7).await.unwrap(), Some(day(30)));

    // A renewal changes the same row.
    permits::record(t.state(), &owner, NewPermit::new(7, "AD-78", day(60)))
        .await
        .unwrap();
    let all = AdvertisingPermit::listed(t.db()).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].permit_number, "AD-78");

    // A permit needs its number and a last day that has not passed.
    for bad in [
        NewPermit::new(8, " ", day(30)),
        NewPermit {
            valid_to: None,
            ..NewPermit::new(8, "AD-1", day(30))
        },
        NewPermit::new(8, "AD-1", day(-1)),
        NewPermit::new(0, "AD-1", day(30)),
    ] {
        let refused = permits::record(t.state(), &owner, bad).await;
        assert!(
            matches!(
                refused,
                Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
            ),
            "{refused:?}"
        );
    }

    // A campaign that promotes no service needs none, and has no last day.
    permits::record(t.state(), &owner, NewPermit::no_services(9))
        .await
        .unwrap();
    assert_eq!(permits::problem(t.db(), 9, day(500)).await.unwrap(), None);
    assert_eq!(permits::last_day(t.db(), 9).await.unwrap(), None);
}

#[renox::test]
async fn the_permit_page_is_the_medical_directors_to_change_and_the_marketing_to_read() {
    let t = boot().await;
    let owner = t
        .user("Olga", &[clinics::permissions::PERMITS_MANAGE])
        .await;
    let marketing = t.user("Mona", &[clinics::permissions::PERMITS_VIEW]).await;
    let nobody = t.user("Nora", &[]).await;
    let valid_to = day(40).to_string();

    t.app.acting_as(&owner);
    t.app
        .htmx()
        .post(
            "/clinics/campaign-permits",
            &[
                ("campaign", "12"),
                ("promotes", "yes"),
                ("permit_number", "AD-12"),
                ("valid_to", valid_to.as_str()),
            ],
        )
        .await
        .assert_ok();
    t.app
        .get("/clinics/campaign-permits")
        .await
        .assert_ok()
        .assert_see("AD-12");

    t.app.acting_as(&marketing);
    t.app
        .get("/clinics/campaign-permits")
        .await
        .assert_ok()
        .assert_see("AD-12");
    t.app
        .htmx()
        .post(
            "/clinics/campaign-permits",
            &[("campaign", "13"), ("promotes", "no")],
        )
        .await
        .assert_forbidden();

    t.app.acting_as(&nobody);
    t.app
        .get("/clinics/campaign-permits")
        .await
        .assert_forbidden();
}
