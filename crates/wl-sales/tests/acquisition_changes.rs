//! BL-SAL-20 (#139), `SAL.ACL.09`: changing the source or campaign of an
//! acquired party or a won lead needs a reason and another holder of
//! `campaigns.manage`; the change is audited and shown on the exceptions
//! report. The pages post as users with their permissions.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::sales::{Attribute, FirstSale, campaign_attribution};
use wl_kernel::{PartyId, Subject};
use wl_sales::acquisition_change::{approve, reject, request};
use wl_sales::permissions as perm;
use wl_sales::{
    AcquisitionChange, Campaign, ChangeRefused, ChangeRequest, ChangeStatus, ChangeTarget,
    LeadSource, MarketingChannel, PartyAcquisition, Purpose,
};

use support::{Booted, boot_events};

const P: PartyId = PartyId(10);

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).unwrap()
}

async fn campaign(t: &Booted, code: &str) -> Campaign {
    Campaign {
        name: code.to_owned(),
        code: code.into(),
        purpose: Some(Purpose::Acquisition),
        marketing_channel: Some(MarketingChannel::Meta),
        owner_id: Some(1),
        start_date: Some(day(3, 1)),
        ..Campaign::default()
    }
    .store(t.db())
    .await
    .unwrap()
    .unwrap()
}

/// P, acquired through *referral* on C1 with a first sale when `sold`.
async fn acquired(t: &Booted, sold: bool) -> PartyAcquisition {
    campaign(t, "C1").await;
    campaign(t, "C2").await;
    let attribution = campaign_attribution(t.state());
    assert!(
        attribution
            .attribute(t.state(), Attribute::party(P, "referral").campaign("C1"))
            .await
            .unwrap()
    );
    if sold {
        let sale = FirstSale::new(P, Subject::new("invoicing.invoice", 1), 200_000, day(4, 2));
        attribution
            .record_first_sale(t.state(), sale)
            .await
            .unwrap();
    }
    PartyAcquisition::of_party(t.db(), P)
        .await
        .unwrap()
        .unwrap()
}

fn change(acquisition: i64, campaign: &str, by: i64) -> ChangeRequest {
    ChangeRequest {
        target: ChangeTarget::Acquisition(acquisition),
        source: "referral".into(),
        campaign: Some(campaign.into()),
        reason: "wrong campaign picked".into(),
        requested_by: by,
    }
}

/// AC9: staff A changes P's campaign from C1 to C2 after P's first sale: the
/// change waits for B; A cannot approve; after B approves,
/// `CustomerAcquisitionChanged` is emitted and the change is audited.
#[renox::test]
async fn sal20_a_change_after_the_first_sale_waits_for_another_manager() {
    let (t, events) = boot_events().await;
    let before = acquired(&t, true).await;
    let c2 = Campaign::by_code(t.db(), "C2").await.unwrap().unwrap();
    let (a, b) = (7, 8);
    let pending = request(t.state(), change(before.id, "C2", a))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.status, ChangeStatus::Pending);
    assert_eq!(
        PartyAcquisition::find(t.db(), before.id)
            .await
            .unwrap()
            .unwrap()
            .campaign_id,
        before.campaign_id,
        "nothing changes yet"
    );
    assert!(events.changed.lock().unwrap().is_empty());

    assert_eq!(
        approve(t.state(), pending.id, a)
            .await
            .unwrap()
            .unwrap_err(),
        ChangeRefused::SamePerson
    );
    let done = approve(t.state(), pending.id, b).await.unwrap().unwrap();
    assert_eq!(done.status, ChangeStatus::Applied);
    assert_eq!(done.decided_by, Some(b));
    let after = PartyAcquisition::find(t.db(), before.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.campaign_id, Some(c2.id));
    let changed = events.changed.lock().unwrap().clone();
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].party, P);
    assert_eq!(changed[0].new_campaign, Some(c2.campaign_id()));
    assert_eq!(changed[0].reason, "wrong campaign picked");
    // Decided once.
    assert_eq!(
        approve(t.state(), pending.id, b)
            .await
            .unwrap()
            .unwrap_err(),
        ChangeRefused::NotPending
    );
    // The audit record.
    let log = renox::audit::for_subject(t.db(), AcquisitionChange::TABLE, pending.id, 10)
        .await
        .unwrap();
    let entry = log
        .iter()
        .find(|l| l.action == "sales.acquisition_changed")
        .expect("the change in the activity log");
    assert_eq!(entry.user_id, Some(b));
    assert_eq!(entry.data["reason"], "wrong campaign picked");
    assert_eq!(entry.data["requested_by"], a);
}

/// A change before the first sale is applied at once, with its reason; a
/// reason is always needed; a rejected change leaves the attribution.
#[renox::test]
async fn sal20_a_change_before_the_first_sale_is_applied_and_a_rejection_changes_nothing() {
    let (t, events) = boot_events().await;
    let before = acquired(&t, false).await;
    let blank = ChangeRequest {
        reason: "  ".into(),
        ..change(before.id, "C2", 7)
    };
    assert_eq!(
        request(t.state(), blank).await.unwrap().unwrap_err(),
        ChangeRefused::ReasonRequired
    );
    assert_eq!(
        request(t.state(), change(before.id, "C1", 7))
            .await
            .unwrap()
            .unwrap_err(),
        ChangeRefused::Unchanged
    );
    assert_eq!(
        request(t.state(), change(before.id, "NOPE", 7))
            .await
            .unwrap()
            .unwrap_err(),
        ChangeRefused::UnknownCampaign
    );
    let done = request(t.state(), change(before.id, "C2", 7))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, ChangeStatus::Applied);
    assert_eq!(events.changed.lock().unwrap().len(), 1);

    // After the first sale a rejected change stays as it was.
    let sale = FirstSale::new(P, Subject::new("invoicing.invoice", 1), 200_000, day(4, 2));
    campaign_attribution(t.state())
        .record_first_sale(t.state(), sale)
        .await
        .unwrap();
    let pending = request(t.state(), change(before.id, "C1", 7))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.status, ChangeStatus::Pending);
    let rejected = reject(t.state(), pending.id, 8, "keep C2")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rejected.status, ChangeStatus::Rejected);
    assert_eq!(rejected.decision_reason.as_deref(), Some("keep C2"));
    let c2 = Campaign::by_code(t.db(), "C2").await.unwrap().unwrap();
    assert_eq!(
        PartyAcquisition::find(t.db(), before.id)
            .await
            .unwrap()
            .unwrap()
            .campaign_id,
        Some(c2.id)
    );
}

/// With the setting off a change after the first sale needs no second person.
#[renox::test]
async fn sal20_with_the_approval_setting_off_a_change_is_applied_at_once() {
    let (t, _) =
        support::boot_events_with(wl_sales::WlSales::new().acquisition_change_approval(false))
            .await;
    let before = acquired(&t, true).await;
    let done = request(t.state(), change(before.id, "C2", 7))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, ChangeStatus::Applied);
}

/// A won lead's source or campaign also needs a second person.
#[renox::test]
async fn sal20_a_won_leads_attribution_needs_a_second_person() {
    let (t, _) = boot_events().await;
    campaign(&t, "C1").await;
    campaign(&t, "C2").await;
    let lead = match wl_sales::lead::capture(
        t.state(),
        wl_sales::NewLead::staff(Some(P), "Omar", "referral"),
        day(3, 5),
    )
    .await
    .unwrap()
    .unwrap()
    {
        wl_sales::Captured::Created(lead) => lead,
        wl_sales::Captured::Noted(_) => panic!("no lead"),
    };
    // An open lead changes at once.
    let open = ChangeRequest {
        target: ChangeTarget::Lead(lead.id),
        source: "social".into(),
        campaign: None,
        reason: "asked again".into(),
        requested_by: 7,
    };
    let social = request(t.state(), open).await.unwrap().unwrap();
    assert_eq!(social.status, ChangeStatus::Applied);
    wl_sales::Lead::query()
        .where_(wl_sales::Lead::ID.eq(lead.id))
        .update(t.db(), &[("status", &"won")])
        .await
        .unwrap();
    let won = ChangeRequest {
        target: ChangeTarget::Lead(lead.id),
        source: "referral".into(),
        campaign: None,
        reason: "was a referral".into(),
        requested_by: 7,
    };
    let pending = request(t.state(), won).await.unwrap().unwrap();
    assert_eq!(pending.status, ChangeStatus::Pending);
    approve(t.state(), pending.id, 8).await.unwrap().unwrap();
    let referral = LeadSource::by_code(t.db(), "referral")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        wl_sales::Lead::find(t.db(), lead.id)
            .await
            .unwrap()
            .unwrap()
            .source_id,
        referral.id
    );
}

/// The exceptions page: the sales with no source and the changes; approving
/// needs `campaigns.manage` and another user than the requester.
#[renox::test]
async fn sal20_the_exceptions_page_lists_and_decides() {
    let (t, _) = boot_events().await;
    let acquisition = acquired(&t, true).await;
    // A sale with no source.
    let stranger = FirstSale::new(
        PartyId(12),
        Subject::new("invoicing.invoice", 2),
        120_000,
        day(4, 5),
    );
    campaign_attribution(t.state())
        .record_first_sale(t.state(), stranger)
        .await
        .unwrap();
    let a = t
        .user(
            "Amal",
            &[
                perm::CAMPAIGNS_VIEW,
                perm::CAMPAIGNS_MANAGE,
                perm::ACQUISITION_REPORTS_VIEW,
            ],
        )
        .await;
    let b = t
        .user(
            "Basel",
            &[
                perm::CAMPAIGNS_VIEW,
                perm::CAMPAIGNS_MANAGE,
                perm::ACQUISITION_REPORTS_VIEW,
            ],
        )
        .await;
    let nobody = t.user("Nobody", &[]).await;

    t.app.acting_as(&nobody);
    t.app
        .get("/sales/acquisition-exceptions")
        .await
        .assert_forbidden();

    t.app.acting_as(&a);
    t.app
        .get("/sales/acquisition-exceptions")
        .await
        .assert_ok()
        .assert_see("1,200.00");
    let response = t
        .app
        .post(
            &format!("/sales/acquisitions/{}/changes", acquisition.id),
            &[
                ("source", "referral"),
                ("campaign", "C2"),
                ("reason", "wrong campaign"),
            ],
        )
        .await;
    response.assert_ok();
    let pending = AcquisitionChange::query()
        .where_(AcquisitionChange::STATUS.eq(ChangeStatus::Pending))
        .first(t.db())
        .await
        .unwrap()
        .expect("the change waits");
    // The requester cannot approve it.
    t.app
        .post(
            &format!("/sales/acquisition-changes/{}/approve", pending.id),
            &[("x", "y")],
        )
        .await
        .assert_status(422);
    t.app.acting_as(&b);
    t.app
        .get("/sales/acquisition-exceptions")
        .await
        .assert_ok()
        .assert_see("wrong campaign");
    t.app
        .post(
            &format!("/sales/acquisition-changes/{}/approve", pending.id),
            &[("x", "y")],
        )
        .await
        .assert_ok();
    let done = AcquisitionChange::find(t.db(), pending.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, ChangeStatus::Applied);
    assert_eq!(done.decided_by, Some(b.id));
}
