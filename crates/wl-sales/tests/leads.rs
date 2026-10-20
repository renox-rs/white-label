//! BL-SAL-20 (#139), leads: the source and campaign from a web form code,
//! the late-response days, the note for a second form submission, the
//! required campaign, `win` and `revert_win`. A test cannot build an
//! `AuthUser`, so the user transitions are checked through their guards.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionCtx};
use wl_kernel::sales::{LeadId, lead_conversion};
use wl_kernel::voids::RecordVoided;
use wl_kernel::{PartyId, Subject};
use wl_sales::lead_machine::{CONTACT, LOSE};
use wl_sales::{
    Campaign, Captured, Lead, LeadRefused, LeadSource, LeadStatus, MarketingChannel, NewLead,
    PartyAcquisition, Purpose, WlSales,
};

use support::{Booted, boot_events, boot_events_with};

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).unwrap()
}

/// C1: `SPRING26`, Meta, 1-31 Mar 2026, source *paid ads*, in `status`.
async fn c1(t: &Booted, status: &str) -> Campaign {
    let paid_ads = LeadSource::by_code(t.db(), "paid_ads")
        .await
        .unwrap()
        .unwrap();
    let campaign = Campaign {
        name: "Spring".into(),
        code: "SPRING26".into(),
        purpose: Some(Purpose::Acquisition),
        marketing_channel: Some(MarketingChannel::Meta),
        lead_source_id: Some(paid_ads.id),
        owner_id: Some(1),
        start_date: Some(day(3, 1)),
        end_date: Some(day(3, 31)),
        ..Campaign::default()
    }
    .store(t.db())
    .await
    .unwrap()
    .unwrap();
    Campaign::query()
        .where_(Campaign::ID.eq(campaign.id))
        .update(t.db(), &[("status", &status)])
        .await
        .unwrap();
    Campaign::find(t.db(), campaign.id).await.unwrap().unwrap()
}

async fn source_code(t: &Booted, lead: &Lead) -> String {
    LeadSource::find(t.db(), lead.source_id)
        .await
        .unwrap()
        .unwrap()
        .code
}

fn created(captured: std::result::Result<Captured, LeadRefused>) -> Lead {
    match captured.expect("the lead is captured") {
        Captured::Created(lead) => lead,
        Captured::Noted(id) => panic!("expected a new lead, got a note on {id}"),
    }
}

/// Sets a lead's status outside the machine, for a fixture.
async fn set_status(t: &Booted, lead: &Lead, status: &str) {
    Lead::query()
        .where_(Lead::ID.eq(lead.id))
        .update(t.db(), &[("status", &status)])
        .await
        .unwrap();
}

/// AC1: a web form link `?c=SPRING26` while C1 is active gives the lead the
/// source *paid ads* and the campaign C1.
#[renox::test]
async fn sal20_a_web_form_code_sets_the_campaign_and_the_source_it_implies() {
    let (t, events) = boot_events().await;
    let campaign = c1(&t, "active").await;
    let lead = created(
        wl_sales::lead::capture(
            t.state(),
            NewLead::web_form(None, "Omar").campaign_code("spring26"),
            day(3, 10),
        )
        .await
        .unwrap(),
    );
    assert_eq!(lead.campaign_id, Some(campaign.id));
    assert_eq!(source_code(&t, &lead).await, "paid_ads");
    assert_eq!(lead.status, LeadStatus::New);
    let seen = events.created.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].source, "paid_ads");
    assert_eq!(seen[0].campaign, Some(campaign.campaign_id()));
}

/// AC3: C1 ended on 31 Mar: a lead with the code on 20 Apr is attributed, one
/// on 5 May gets the source only. The code of a draft campaign is ignored.
#[renox::test]
async fn sal20_a_lead_after_the_campaign_end_is_attributed_for_the_late_response_days() {
    let (t, _) = boot_events().await;
    let campaign = c1(&t, "ended").await;
    let april = created(
        wl_sales::lead::capture(
            t.state(),
            NewLead::web_form(None, "Late").campaign_code("SPRING26"),
            day(4, 20),
        )
        .await
        .unwrap(),
    );
    assert_eq!(april.campaign_id, Some(campaign.id));
    let may = created(
        wl_sales::lead::capture(
            t.state(),
            NewLead::web_form(None, "Later").campaign_code("SPRING26"),
            day(5, 5),
        )
        .await
        .unwrap(),
    );
    assert_eq!(may.campaign_id, None);
    assert_eq!(source_code(&t, &may).await, "website");

    Campaign::query()
        .where_(Campaign::ID.eq(campaign.id))
        .update(t.db(), &[("status", &"draft")])
        .await
        .unwrap();
    let draft = created(
        wl_sales::lead::capture(
            t.state(),
            NewLead::web_form(None, "Early").campaign_code("SPRING26"),
            day(3, 1),
        )
        .await
        .unwrap(),
    );
    assert_eq!(draft.campaign_id, None);
}

/// AC10: a second web form submission for the party on C1 while L1 is open
/// adds a note to L1 and creates no lead.
#[renox::test]
async fn sal20_a_second_form_submission_is_a_note_on_the_open_lead() {
    let (t, events) = boot_events().await;
    let campaign = c1(&t, "active").await;
    let form = || NewLead::web_form(Some(PartyId(10)), "Omar").campaign_code("SPRING26");
    let first = created(
        wl_sales::lead::capture(t.state(), form(), day(3, 10))
            .await
            .unwrap(),
    );
    match wl_sales::lead::capture(t.state(), form(), day(3, 11))
        .await
        .unwrap()
        .unwrap()
    {
        Captured::Noted(id) => assert_eq!(id, first.id),
        Captured::Created(_) => panic!("a second lead was created"),
    }
    let leads = Lead::query()
        .where_(Lead::CAMPAIGN_ID.eq(campaign.id))
        .count(t.db())
        .await
        .unwrap();
    assert_eq!(leads, 1);
    let notes = first.notes(t.db()).await.unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].kind, "web_form");
    assert_eq!(events.created.lock().unwrap().len(), 1);

    // Once the first lead is closed, the next enquiry is a lead again.
    set_status(&t, &first, "lost").await;
    let again = created(
        wl_sales::lead::capture(t.state(), form(), day(3, 12))
            .await
            .unwrap(),
    );
    assert_ne!(again.id, first.id);
}

/// A staff lead needs a source; with the setting (05), *paid ads* and
/// *social* need a campaign too.
#[renox::test]
async fn sal20_a_lead_needs_a_source_and_a_campaign_where_the_setting_says() {
    let (t, _) =
        boot_events_with(WlSales::new().campaign_required_for(&["paid_ads", "social"])).await;
    let no_source = NewLead {
        contact_name: "Omar".into(),
        ..NewLead::default()
    };
    assert_eq!(
        wl_sales::lead::capture(t.state(), no_source, day(3, 1))
            .await
            .unwrap()
            .unwrap_err(),
        LeadRefused::SourceRequired
    );
    assert_eq!(
        wl_sales::lead::capture(
            t.state(),
            NewLead::staff(None, "Omar", "nowhere"),
            day(3, 1)
        )
        .await
        .unwrap()
        .unwrap_err(),
        LeadRefused::UnknownSource
    );
    assert_eq!(
        wl_sales::lead::capture(t.state(), NewLead::staff(None, "Omar", "social"), day(3, 1))
            .await
            .unwrap()
            .unwrap_err(),
        LeadRefused::CampaignRequired
    );
    c1(&t, "active").await;
    let ok = created(
        wl_sales::lead::capture(
            t.state(),
            NewLead::staff(None, "Omar", "social").campaign_code("SPRING26"),
            day(3, 5),
        )
        .await
        .unwrap(),
    );
    assert_eq!(source_code(&t, &ok).await, "social");
    let referral = created(
        wl_sales::lead::capture(
            t.state(),
            NewLead::staff(None, "Sara", "referral"),
            day(3, 5),
        )
        .await
        .unwrap(),
    );
    assert_eq!(referral.campaign_id, None);
}

/// The guards of the user transitions: `contact` needs the note, `lose` the
/// reason.
#[renox::test]
async fn sal20_contact_needs_a_note_and_lose_a_reason() {
    let (t, _) = boot_events().await;
    let lead = created(
        wl_sales::lead::capture(
            t.state(),
            NewLead::staff(None, "Omar", "referral"),
            day(3, 5),
        )
        .await
        .unwrap(),
    );
    let machine = Lead::machine();
    let blank = TransitionCtx::new(None).with_reason("  ");
    let noted = TransitionCtx::new(None).with_reason("Called, wants a viewing");
    let contact = machine.find(CONTACT).unwrap();
    assert!(contact.check_guards(&lead, &blank).await.is_err());
    assert!(contact.check_guards(&lead, &noted).await.is_ok());
    let lose = machine.find(LOSE).unwrap();
    assert!(lose.check_guards(&lead, &blank).await.is_err());
    assert!(
        lose.check_guards(&lead, &TransitionCtx::new(None).with_reason("duplicate"))
            .await
            .is_ok()
    );
}

/// A lead closed as duplicate or spam is left out of the counts.
#[renox::test]
async fn sal20_duplicate_and_spam_leads_are_left_out_of_the_counts() {
    let (t, _) = boot_events().await;
    let lead = created(
        wl_sales::lead::capture(
            t.state(),
            NewLead::staff(None, "Omar", "referral"),
            day(3, 5),
        )
        .await
        .unwrap(),
    );
    Lead::query()
        .where_(Lead::ID.eq(lead.id))
        .update(
            t.db(),
            &[
                ("status", &"lost"),
                ("lost_reason", &Some("spam_or_invalid")),
            ],
        )
        .await
        .unwrap();
    let lead = Lead::find(t.db(), lead.id).await.unwrap().unwrap();
    assert!(lead.left_out_of_counts());
}

/// AC8: a test conversion record X-1 wins lead L3; voided as entered in
/// error, L3 is `qualified` again and `LeadWinReverted` is emitted. A
/// conversion record that is only cancelled leaves the lead `won`.
#[renox::test]
async fn sal20_voiding_the_conversion_record_reverts_the_win() {
    let (t, events) = boot_events().await;
    let lead = created(
        wl_sales::lead::capture(
            t.state(),
            NewLead::staff(Some(PartyId(10)), "Omar", "referral").owner(1),
            day(3, 5),
        )
        .await
        .unwrap(),
    );
    // A new lead is not won: only a contacted or qualified one.
    let record = Subject::new("test.conversion", 1);
    let conversion = lead_conversion(t.state());
    assert!(
        !conversion
            .win(t.state(), LeadId(lead.id), record.clone())
            .await
            .unwrap()
    );
    set_status(&t, &lead, "qualified").await;
    assert!(
        conversion
            .win(t.state(), LeadId(lead.id), record.clone())
            .await
            .unwrap()
    );
    let won = Lead::find(t.db(), lead.id).await.unwrap().unwrap();
    assert_eq!(won.status, LeadStatus::Won);
    assert_eq!(won.converted(), Some(record.clone()));
    assert_eq!(events.won.lock().unwrap().len(), 1);
    assert_eq!(events.won.lock().unwrap()[0].converted, record);
    let acquisition = PartyAcquisition::of_party(t.db(), PartyId(10))
        .await
        .unwrap()
        .expect("the party takes the lead's source");
    assert_eq!(acquisition.lead_id, Some(lead.id));
    assert_eq!(acquisition.set_from, "lead");

    // A void of another record changes nothing.
    t.state()
        .emit(RecordVoided {
            subject: Subject::new("test.conversion", 2),
            state_before: "active".into(),
            reason: "error".into(),
            user_id: 1,
            approval_request_id: None,
        })
        .await
        .unwrap();
    assert_eq!(
        Lead::find(t.db(), lead.id).await.unwrap().unwrap().status,
        LeadStatus::Won
    );

    t.state()
        .emit(RecordVoided {
            subject: record.clone(),
            state_before: "active".into(),
            reason: "entered in error".into(),
            user_id: 1,
            approval_request_id: None,
        })
        .await
        .unwrap();
    let back = Lead::find(t.db(), lead.id).await.unwrap().unwrap();
    assert_eq!(back.status, LeadStatus::Qualified);
    assert_eq!(back.converted(), None);
    let reverted = events.reverted.lock().unwrap().clone();
    assert_eq!(reverted.len(), 1);
    assert_eq!(reverted[0].voided, record);
    // The acquisition set from the lead, with no other first sale, is cleared.
    assert!(
        PartyAcquisition::of_party(t.db(), PartyId(10))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(events.changed.lock().unwrap().len(), 1);
    assert_eq!(
        events.changed.lock().unwrap()[0].reason,
        "lead_win_reverted"
    );
}

/// AC1 through the web form route: the link `/enquiries?c=SPRING26` posts the
/// form, needs no login, and the lead has the source *paid ads* and C1.
#[renox::test]
async fn sal20_the_web_form_route_reads_the_campaign_code_of_the_link() {
    let (t, events) = boot_events().await;
    let campaign = c1(&t, "active").await;
    t.app
        .post(
            "/enquiries?c=spring26",
            &[("name", "Omar"), ("interest", "Villa")],
        )
        .await
        .assert_status(303);
    let leads = Lead::query().get(t.db()).await.unwrap();
    assert_eq!(leads.len(), 1);
    assert_eq!(leads[0].campaign_id, Some(campaign.id));
    assert_eq!(source_code(&t, &leads[0]).await, "paid_ads");
    assert_eq!(leads[0].status, LeadStatus::New);
    assert_eq!(events.created.lock().unwrap().len(), 1);

    // No code: the lead has the source *website* and no campaign.
    t.app
        .post("/enquiries", &[("name", "Sara")])
        .await
        .assert_status(303);
    let plain = Lead::query()
        .where_(Lead::CAMPAIGN_ID.is_null())
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(plain.len(), 1);
    assert_eq!(source_code(&t, &plain[0]).await, "website");

    // A form without a name is refused and creates nothing.
    t.app
        .post("/enquiries?c=SPRING26", &[("name", " ")])
        .await
        .assert_status(422);
    assert_eq!(Lead::query().count(t.db()).await.unwrap(), 2);
}

/// Edge case of AC8: a conversion record cancelled for a business reason
/// (not voided) leaves the lead `won` and its acquisition: it did convert.
/// The record's own cancellation emits no `RecordVoided`; the nightly run
/// changes nothing either.
#[renox::test]
async fn sal20_a_business_cancellation_of_the_conversion_record_leaves_the_lead_won() {
    let (t, events) = boot_events().await;
    let lead = created(
        wl_sales::lead::capture(
            t.state(),
            NewLead::staff(Some(PartyId(10)), "Omar", "referral").owner(1),
            day(3, 5),
        )
        .await
        .unwrap(),
    );
    set_status(&t, &lead, "qualified").await;
    let record = Subject::new("test.conversion", 7);
    assert!(
        lead_conversion(t.state())
            .win(t.state(), LeadId(lead.id), record.clone())
            .await
            .unwrap()
    );

    // The record is cancelled by the business (no void): its owner only
    // changes its own status. The sales module hears nothing, and the
    // nightly run finds nothing to revert.
    wl_sales::acquisition_daily::run_daily(t.state())
        .await
        .unwrap();
    let still = Lead::find(t.db(), lead.id).await.unwrap().unwrap();
    assert_eq!(still.status, LeadStatus::Won);
    assert_eq!(still.converted(), Some(record));
    assert!(events.reverted.lock().unwrap().is_empty());
    assert!(
        PartyAcquisition::of_party(t.db(), PartyId(10))
            .await
            .unwrap()
            .is_some()
    );
}
