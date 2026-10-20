//! #338 (BL-ACC-14, `ACC.ACL.13`): a change to the calendar-driving fields of
//! a tax profile takes effect only after a manager other than the requester
//! approves it, with an effective date; one pending change per profile; every
//! approval stores a new version; a contact e-mail needs no approval.

mod support;

use accounting::regeneration::RULE;
use accounting::tax_profile_change::{
    ChangeStatus, REQUEST, TAX_PROFILE_CHANGE_SUBJECT_TYPE, TaxProfileChange,
};
use accounting::{TaxProfile, tax_profile};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::TestResponse;
use wl_kernel::approvals::{self, ApprovalRequest, ApprovalStatus};
use wl_kernel::automation::{RuleRun, RuleRunStatus};

use support::{Booted, Client, boot};

fn day(text: &str) -> NaiveDate {
    text.parse().unwrap()
}

/// The profile of `client` with the quarters ending in January.
async fn january(t: &Booted, client: &Client) -> TaxProfile {
    t.tax_profile_staggered(client, day("2026-01-31")).await
}

/// `user` asks to change `client`'s profile to the quarters ending
/// `first_end` (and QFZP `qfzp`), effective `effective`; everything else as
/// the fixture.
async fn ask(
    t: &Booted,
    user: &User,
    client: &Client,
    first_end: &str,
    qfzp: &str,
    effective: &str,
) -> TestResponse {
    t.post(
        user,
        &format!(
            "/accounting/clients/{}/tax-profile/change",
            client.company.0
        ),
        &[
            ("trn", "100000000000003"),
            ("vat_registered", "1"),
            ("vat_period", "quarterly"),
            ("vat_first_period_end", first_end),
            ("financial_year_end", "2026-12-31"),
            ("ct_registration_no", "CT-100200300"),
            ("qfzp", qfzp),
            ("effective_date", effective),
            ("note", "The FTA moved the stagger"),
        ],
    )
    .await
}

/// The change of `client` waiting for approval.
async fn pending(t: &Booted, client: &Client) -> TaxProfileChange {
    TaxProfileChange::pending(t.db(), client.company)
        .await
        .unwrap()
        .expect("a change is pending approval")
}

/// The approval request pending about `change`.
async fn pending_request(t: &Booted, change: &TaxProfileChange) -> ApprovalRequest {
    approvals::pending_for(t.db(), &change.subject())
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("an approval request is pending")
}

/// The run of `ACC.RUL.10` for `change`, if it started.
async fn rule_run(t: &Booted, change: &TaxProfileChange) -> Option<RuleRun> {
    RuleRun::find_occurrence(
        t.db(),
        RULE,
        &change.subject(),
        &format!("{RULE}:{}", change.subject()),
    )
    .await
    .unwrap()
}

/// Accountant A changes the VAT stagger from the quarter ending January to
/// the quarter ending February, effective 2027-02-01: the profile shows the
/// change pending and the version in force is the old one.
#[renox::test]
async fn a_requested_change_is_pending_and_the_profile_in_force_is_unchanged() {
    let t = boot().await;
    let accountant = t.accountant().await;
    let client = t.client().await;
    let base = january(&t, &client).await;

    ask(&t, &accountant, &client, "2026-02-28", "", "2027-02-01")
        .await
        .assert_ok();
    let change = pending(&t, &client).await;
    assert_eq!(change.status, ChangeStatus::PendingApproval);
    assert_eq!(change.fields(), ["vat_first_period_end"]);
    assert_eq!(change.requested_by, Some(accountant.id));
    assert_eq!(change.base_profile_id, Some(base.id));
    assert_eq!(change.effective_date, day("2027-02-01"));

    // The profile page shows it as pending approval.
    t.get(
        &accountant,
        &format!("/accounting/clients/{}/tax-profile", client.company.0),
    )
    .await
    .assert_ok()
    .assert_see("Pending approval")
    .assert_see("2027-02-01");

    // The calendar keeps the current version.
    let versions = TaxProfile::versions(t.db(), client.company).await.unwrap();
    assert_eq!(versions.len(), 1);
    for on in [day("2026-11-01"), day("2027-02-01"), day("2027-06-01")] {
        let current = TaxProfile::current(t.db(), client.company, on)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.vat_first_period_end, Some(day("2026-01-31")));
    }

    // The request waits for a holder of the approve permission.
    let request = pending_request(&t, &change).await;
    assert_eq!(request.requester_id, accountant.id);
    assert_eq!(request.transition, REQUEST);
    assert_eq!(request.subject_type, TAX_PROFILE_CHANGE_SUBJECT_TYPE);
    assert_eq!(request.payload.0["effective_date"], "2027-02-01");
    assert_eq!(request.payload.0["past"], false);
    assert!(
        rule_run(&t, &change).await.is_none(),
        "ACC.RUL.10 does not run before the approval"
    );
}

/// Manager B approves: the profile has a new version effective 2027-02-01
/// approved by B (the old one is kept), and `ACC.RUL.10` runs.
#[renox::test]
async fn the_approval_stores_a_new_version_and_starts_the_calendar_rule() {
    let t = boot().await;
    let accountant = t.accountant().await;
    let manager = t.manager().await;
    let client = t.client().await;
    let base = january(&t, &client).await;
    ask(&t, &accountant, &client, "2026-02-28", "", "2027-02-01")
        .await
        .assert_ok();
    let change = pending(&t, &client).await;
    let request = pending_request(&t, &change).await;

    t.run(approvals::approve(
        t.state(),
        request.id,
        &manager,
        "Per the FTA notice",
    ))
    .await
    .unwrap();

    let versions = TaxProfile::versions(t.db(), client.company).await.unwrap();
    assert_eq!(versions.len(), 2, "each approval is a new version");
    assert_eq!(versions[0].id, base.id);
    assert_eq!(versions[0].vat_first_period_end, Some(day("2026-01-31")));
    let new = &versions[1];
    assert_eq!(new.effective_date, day("2027-02-01"));
    assert_eq!(new.vat_first_period_end, Some(day("2026-02-28")));
    assert_eq!(new.approved_by, Some(manager.id));
    assert_eq!(new.created_by, Some(accountant.id));
    // In force from its effective date, not before.
    let before = TaxProfile::current(t.db(), client.company, day("2027-01-31"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.id, base.id);
    let after = TaxProfile::current(t.db(), client.company, day("2027-02-01"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.id, new.id);

    let done = TaxProfileChange::find_or_404(t.db(), change.id)
        .await
        .unwrap();
    assert_eq!(done.status, ChangeStatus::Approved);
    assert_eq!(done.profile_id, Some(new.id));
    assert_eq!(done.decided_by, Some(manager.id));
    assert!(
        TaxProfileChange::pending(t.db(), client.company)
            .await
            .unwrap()
            .is_none()
    );
    let run = rule_run(&t, &change).await.expect("ACC.RUL.10 ran");
    assert_eq!(run.status, RuleRunStatus::Succeeded);
}

/// Manager A cannot approve their own change; another manager can.
#[renox::test]
async fn a_manager_cannot_approve_their_own_change() {
    let t = boot().await;
    let amal = t.manager().await;
    let bashir = t.manager_named("Bashir").await;
    let client = t.client().await;
    january(&t, &client).await;
    ask(&t, &amal, &client, "2026-02-28", "", "2027-02-01")
        .await
        .assert_ok();
    let change = pending(&t, &client).await;
    let request = pending_request(&t, &change).await;

    let own = t
        .run(approvals::approve(t.state(), request.id, &amal, "mine"))
        .await;
    assert!(own.is_err(), "the requester may not approve");
    assert_eq!(
        TaxProfile::versions(t.db(), client.company)
            .await
            .unwrap()
            .len(),
        1
    );
    let still = pending_request(&t, &change).await;
    assert_eq!(still.status, ApprovalStatus::Pending);

    t.run(approvals::approve(t.state(), request.id, &bashir, "ok"))
        .await
        .unwrap();
    assert_eq!(
        TaxProfile::versions(t.db(), client.company)
            .await
            .unwrap()
            .len(),
        2
    );
}

/// One pending change per profile: a second request is refused until the
/// first is decided; a rejection keeps the current version and frees the
/// profile for a new request.
#[renox::test]
async fn a_second_request_waits_for_the_first_and_a_rejection_keeps_the_version() {
    let t = boot().await;
    let accountant = t.accountant().await;
    let manager = t.manager().await;
    let client = t.client().await;
    january(&t, &client).await;
    ask(&t, &accountant, &client, "2026-02-28", "", "2027-02-01")
        .await
        .assert_ok();
    let first = pending(&t, &client).await;

    ask(&t, &accountant, &client, "2026-02-28", "1", "2027-03-01")
        .await
        .assert_status(422);
    assert_eq!(
        TaxProfileChange::of_client(t.db(), client.company)
            .await
            .unwrap()
            .len(),
        1
    );

    let request = pending_request(&t, &first).await;
    t.run(approvals::reject(
        t.state(),
        request.id,
        &manager,
        "Wrong quarter",
    ))
    .await
    .unwrap();
    let declined = TaxProfileChange::find_or_404(t.db(), first.id)
        .await
        .unwrap();
    assert_eq!(declined.status, ChangeStatus::Declined);
    assert_eq!(declined.decision_note.as_deref(), Some("Wrong quarter"));
    assert_eq!(
        TaxProfile::versions(t.db(), client.company)
            .await
            .unwrap()
            .len(),
        1,
        "the current version stays"
    );
    assert!(rule_run(&t, &first).await.is_none());

    ask(&t, &accountant, &client, "2026-02-28", "1", "2027-03-01")
        .await
        .assert_ok();
    assert_eq!(
        pending(&t, &client).await.status,
        ChangeStatus::PendingApproval
    );
}

/// Changing a contact e-mail needs no approval: no change, no request, the
/// version is updated in place; a form that differs only there is not a
/// change.
#[renox::test]
async fn a_contact_email_needs_no_approval() {
    let t = boot().await;
    let accountant = t.accountant().await;
    let client = t.client().await;
    let base = january(&t, &client).await;

    t.post(
        &accountant,
        &format!(
            "/accounting/clients/{}/tax-profile/contact",
            client.company.0
        ),
        &[
            ("contact_email", "tax@falcon-finance.example"),
            ("notes", "Prefers e-mail"),
        ],
    )
    .await
    .assert_ok();
    let versions = TaxProfile::versions(t.db(), client.company).await.unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].id, base.id);
    assert_eq!(
        versions[0].contact_email.as_deref(),
        Some("tax@falcon-finance.example")
    );
    assert_eq!(versions[0].notes.as_deref(), Some("Prefers e-mail"));
    assert!(
        TaxProfileChange::of_client(t.db(), client.company)
            .await
            .unwrap()
            .is_empty()
    );

    // A proposal that differs only in the contact is no change.
    let other = TaxProfile {
        contact_email: Some("other@falcon-finance.example".into()),
        ..versions[0].clone()
    };
    assert!(tax_profile::changed_fields(&versions[0], &other).is_empty());
    ask(&t, &accountant, &client, "2026-01-31", "", "2027-02-01")
        .await
        .assert_status(422);
    assert!(
        TaxProfileChange::of_client(t.db(), client.company)
            .await
            .unwrap()
            .is_empty()
    );
}

/// A date in the past is allowed (the FTA confirms late); the approval request
/// carries it so the approver sees it highlighted, and the new version applies
/// from that day. The first version of a client is recorded once; a second
/// `record` is refused.
#[renox::test]
async fn a_past_effective_date_is_flagged_and_the_first_version_is_recorded_once() {
    let t = boot().await;
    let accountant = t.accountant().await;
    let manager = t.manager().await;
    let client = t.client().await;
    let base = january(&t, &client).await;
    let recorded_again = tax_profile::record(t.db(), None, base.clone()).await;
    assert!(recorded_again.is_err());

    t.set_clock(2, 10, 0);
    ask(&t, &accountant, &client, "2026-02-28", "", "2026-10-01")
        .await
        .assert_ok();
    let change = pending(&t, &client).await;
    assert!(change.is_in_the_past(day("2026-11-02")));
    let request = pending_request(&t, &change).await;
    assert_eq!(request.payload.0["past"], true);
    t.get(
        &manager,
        &format!("/accounting/clients/{}/tax-profile", client.company.0),
    )
    .await
    .assert_ok()
    .assert_see("effective date is in the past");
    t.run(approvals::approve(t.state(), request.id, &manager, "ok"))
        .await
        .unwrap();
    let current = TaxProfile::current(t.db(), client.company, day("2026-10-02"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.effective_date, day("2026-10-01"));
}

/// A deregistration from VAT and a change of the corporate tax registration
/// are calendar-driving: each waits for a manager's approval, names its
/// fields, and leaves the profile in force unchanged.
#[renox::test]
async fn a_deregistration_and_a_ct_registration_change_need_the_approval() {
    let t = boot().await;
    let accountant = t.accountant().await;
    let manager = t.manager().await;
    let client = t.client().await;
    january(&t, &client).await;
    let url = format!(
        "/accounting/clients/{}/tax-profile/change",
        client.company.0
    );

    // Corporate tax registration only.
    t.post(
        &accountant,
        &url,
        &[
            ("trn", "100000000000003"),
            ("vat_registered", "1"),
            ("vat_period", "quarterly"),
            ("vat_first_period_end", "2026-01-31"),
            ("financial_year_end", "2026-12-31"),
            ("ct_registration_no", "CT-999888777"),
            ("effective_date", "2027-02-01"),
        ],
    )
    .await
    .assert_ok();
    let ct = pending(&t, &client).await;
    assert_eq!(ct.fields(), ["ct_registration_no"]);
    let request = pending_request(&t, &ct).await;
    t.run(approvals::reject(
        t.state(),
        request.id,
        &manager,
        "Not yet",
    ))
    .await
    .unwrap();

    // Deregistration: the registration, the period and the stagger change.
    t.post(
        &accountant,
        &url,
        &[
            ("trn", "100000000000003"),
            ("vat_period", "not_registered"),
            ("financial_year_end", "2026-12-31"),
            ("ct_registration_no", "CT-100200300"),
            ("effective_date", "2027-02-01"),
        ],
    )
    .await
    .assert_ok();
    let dereg = pending(&t, &client).await;
    assert_eq!(
        dereg.fields(),
        ["vat_registered", "vat_period", "vat_first_period_end"]
    );
    let current = TaxProfile::current(t.db(), client.company, day("2027-03-01"))
        .await
        .unwrap()
        .unwrap();
    assert!(current.vat_registered, "in force until it is approved");
    let request = pending_request(&t, &dereg).await;
    t.run(approvals::approve(t.state(), request.id, &manager, "ok"))
        .await
        .unwrap();
    let after = TaxProfile::current(t.db(), client.company, day("2027-03-01"))
        .await
        .unwrap()
        .unwrap();
    assert!(!after.vat_registered);
}
