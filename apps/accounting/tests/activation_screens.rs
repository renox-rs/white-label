//! #489 and #490: the two onboarding records `activate` waits for can be made
//! from the screens: the engagement page opens the client's CDD file, and the
//! tax profile page records the client's first profile (04 §4.1 steps 4, 5
//! and 7).

mod support;

use accounting::cdd::CddFile;
use accounting::permissions;
use accounting::{TaxProfile, VatPeriod};

use support::boot;

/// An onboarding engagement offers *Open the CDD file* to a user who prepares
/// CDD files; running it opens one file, a second run opens no second file,
/// and the page then links to the file instead of offering it again.
#[renox::test]
async fn the_engagement_page_opens_the_cdd_file_once() {
    let t = boot().await;
    let accountant = t.accountant().await;
    let client = t.client().await;
    let engagement = t.engagement(&client).await;
    let page = format!("/accounting/engagements/{}", engagement.id);

    assert!(
        CddFile::current(t.db(), client.company)
            .await
            .unwrap()
            .is_none()
    );
    t.get(&accountant, &page)
        .await
        .assert_ok()
        .assert_see("Open the CDD file")
        .assert_see(&format!("engagement-cdd-{}", engagement.id));

    t.post(&accountant, &format!("{page}/cdd"), &[])
        .await
        .assert_ok();
    let file = CddFile::current(t.db(), client.company)
        .await
        .unwrap()
        .expect("the file is open");
    t.post(&accountant, &format!("{page}/cdd"), &[])
        .await
        .assert_ok();
    let again = CddFile::current(t.db(), client.company)
        .await
        .unwrap()
        .expect("the file is still there");
    assert_eq!(again.id, file.id, "no second file is opened");

    t.get(&accountant, &page)
        .await
        .assert_ok()
        .assert_see(&format!("/accounting/cdd/{}", file.id))
        .assert_dont_see(&format!("engagement-cdd-{}", engagement.id));
}

/// Without `cdd.prepare` the action is not offered and the route refuses.
#[renox::test]
async fn opening_the_cdd_file_needs_cdd_prepare() {
    let t = boot().await;
    let viewer = t
        .user_with_grants("Vic", &[permissions::ENGAGEMENTS_VIEW])
        .await;
    let client = t.client().await;
    let engagement = t.engagement(&client).await;
    let page = format!("/accounting/engagements/{}", engagement.id);

    t.get(&viewer, &page)
        .await
        .assert_ok()
        .assert_dont_see(&format!("engagement-cdd-{}", engagement.id));
    t.post(&viewer, &format!("{page}/cdd"), &[])
        .await
        .assert_forbidden();
    assert!(
        CddFile::current(t.db(), client.company)
            .await
            .unwrap()
            .is_none()
    );
}

/// A client without a profile gets the *Record the tax profile* form; the
/// record counts as approved by the user who made it, is complete, and the
/// form gives way to the change request. A second record is refused.
#[renox::test]
async fn the_tax_profile_page_records_the_first_version_once() {
    let t = boot().await;
    let accountant = t.accountant().await;
    let client = t.client().await;
    let page = format!("/accounting/clients/{}/tax-profile", client.company.0);
    let form = [
        ("trn", "100000000000003"),
        ("vat_registered", "1"),
        ("vat_period", "quarterly"),
        ("vat_first_period_end", "2026-03-31"),
        ("financial_year_end", "2026-12-31"),
        ("ct_registration_no", "CT-100200300"),
        ("effective_date", "2026-01-01"),
        ("note", "Recorded at onboarding"),
    ];

    t.get(&accountant, &page)
        .await
        .assert_ok()
        .assert_see("Record the tax profile")
        .assert_dont_see("Change the profile");
    t.post(&accountant, &page, &form).await.assert_ok();

    let profile = TaxProfile::latest(t.db(), client.company)
        .await
        .unwrap()
        .expect("the profile is recorded");
    assert_eq!(profile.trn.as_deref(), Some("100000000000003"));
    assert_eq!(profile.vat_period, VatPeriod::Quarterly);
    assert_eq!(profile.approved_by, Some(accountant.id));
    assert_eq!(profile.notes.as_deref(), Some("Recorded at onboarding"));
    assert!(profile.is_complete());

    t.get(&accountant, &page)
        .await
        .assert_ok()
        .assert_see("Change the profile")
        .assert_dont_see("Record the tax profile");
    t.post(&accountant, &page, &form).await.assert_status(422);
    assert_eq!(
        TaxProfile::versions(t.db(), client.company)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// The date is required and the form is for a user who manages profiles.
#[renox::test]
async fn recording_the_first_profile_needs_a_date_and_the_permission() {
    let t = boot().await;
    let accountant = t.accountant().await;
    let viewer = t
        .user_with_grants("Vic", &[permissions::TAX_PROFILES_VIEW])
        .await;
    let client = t.client().await;
    let page = format!("/accounting/clients/{}/tax-profile", client.company.0);

    t.get(&viewer, &page)
        .await
        .assert_ok()
        .assert_dont_see("Record the tax profile");
    t.post(&viewer, &page, &[("effective_date", "2026-01-01")])
        .await
        .assert_forbidden();
    t.post(&accountant, &page, &[("trn", "100000000000003")])
        .await
        .assert_status(422);
    assert!(
        TaxProfile::latest(t.db(), client.company)
            .await
            .unwrap()
            .is_none()
    );
}
