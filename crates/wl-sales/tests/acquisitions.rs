//! BL-SAL-20 (#139), party acquisitions: the lead's source and campaign, the
//! first sale and its converted value, the source a new party's sale needs,
//! the nightly replacement of a first sale, the exceptions of a sale with no
//! source and the party merge.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::invoicing::{CreditNoteId, CreditNoteIssued, InvoiceId, InvoiceIssued};
use wl_kernel::sales::{Attribute, FirstSale, campaign_attribution, lead_conversion};
use wl_kernel::{PartiesMerged, PartyId, Subject};
use wl_sales::acquisition::{self, AcquisitionRefused, OTHER_SOURCE};
use wl_sales::acquisition_daily::run_daily;
use wl_sales::{
    AcquisitionSale, Campaign, Captured, LeadSource, MarketingChannel, NewLead, PartyAcquisition,
    Purpose, WlSales,
};

use support::{Booted, boot_events, boot_events_with};

const P: PartyId = PartyId(10);

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).unwrap()
}

async fn source_id(t: &Booted, code: &str) -> i64 {
    LeadSource::by_code(t.db(), code).await.unwrap().unwrap().id
}

/// An active campaign `SPRING26` for *paid ads*.
async fn c1(t: &Booted) -> Campaign {
    let campaign = Campaign {
        name: "Spring".into(),
        code: "SPRING26".into(),
        purpose: Some(Purpose::Acquisition),
        marketing_channel: Some(MarketingChannel::Meta),
        lead_source_id: Some(source_id(t, "paid_ads").await),
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
        .update(t.db(), &[("status", &"active")])
        .await
        .unwrap();
    Campaign::find(t.db(), campaign.id).await.unwrap().unwrap()
}

/// A lead of `party` from C1, qualified, won with `record`.
async fn won_lead(t: &Booted, party: PartyId, record: Subject) {
    let lead = match wl_sales::lead::capture(
        t.state(),
        NewLead::web_form(Some(party), "Omar").campaign_code("SPRING26"),
        day(3, 10),
    )
    .await
    .unwrap()
    .unwrap()
    {
        Captured::Created(lead) => lead,
        Captured::Noted(_) => panic!("no lead"),
    };
    wl_sales::Lead::query()
        .where_(wl_sales::Lead::ID.eq(lead.id))
        .update(t.db(), &[("status", &"qualified")])
        .await
        .unwrap();
    assert!(
        lead_conversion(t.state())
            .win(t.state(), lead.lead_id(), record)
            .await
            .unwrap()
    );
}

/// An invoice of `party`: AED 2,100 with VAT, 2,000 without.
fn invoice(id: i64, party: PartyId, m: u32, d: u32) -> InvoiceIssued {
    InvoiceIssued {
        invoice: InvoiceId(id),
        party,
        source: Subject::new("test.conversion", id),
        total: 210_000,
        net_total: 200_000,
        issue_date: day(m, d),
        due_date: day(m, d) + renox::chrono::Days::new(14),
    }
}

async fn acquisition_of(t: &Booted, party: PartyId) -> PartyAcquisition {
    PartyAcquisition::of_party(t.db(), party)
        .await
        .unwrap()
        .expect("the party has an acquisition")
}

/// AC2: lead L1 (C1) won by an accepted quotation: P's acquisition has the
/// source *paid ads* and the campaign C1; P's first invoice of AED 2,100
/// (AED 2,000 excluding VAT) is the first sale with value AED 2,000 and
/// `CustomerAcquired` is emitted once.
#[renox::test]
async fn sal20_the_won_lead_gives_the_acquisition_and_the_first_invoice_is_linked() {
    let (t, events) = boot_events().await;
    let campaign = c1(&t).await;
    won_lead(&t, P, Subject::new("test.quotation", 1)).await;
    let acquisition = acquisition_of(&t, P).await;
    assert_eq!(acquisition.source_id, source_id(&t, "paid_ads").await);
    assert_eq!(acquisition.campaign_id, Some(campaign.id));
    assert_eq!(acquisition.first_sale(), None);
    assert!(events.acquired.lock().unwrap().is_empty());

    t.state().emit(invoice(5, P, 4, 2)).await.unwrap();
    let acquisition = acquisition_of(&t, P).await;
    assert_eq!(
        acquisition.first_sale(),
        Some(Subject::new("invoicing.invoice", 5))
    );
    assert_eq!(acquisition.first_sale_value, 200_000);
    assert_eq!(acquisition.first_sale_date, Some(day(4, 2)));
    let acquired = events.acquired.lock().unwrap().clone();
    assert_eq!(acquired.len(), 1);
    assert_eq!(acquired[0].source, "paid_ads");
    assert_eq!(acquired[0].value, 200_000);
    assert_eq!(acquired[0].campaign, Some(campaign.campaign_id()));

    // A later invoice is a candidate, not a second customer.
    t.state().emit(invoice(6, P, 4, 20)).await.unwrap();
    assert_eq!(events.acquired.lock().unwrap().len(), 1);
    assert_eq!(
        acquisition_of(&t, P).await.first_sale(),
        Some(Subject::new("invoicing.invoice", 5))
    );
}

/// AC4: P was acquired through *referral*; a later lead L2 on C1 won for P
/// leaves P's acquisition unchanged.
#[renox::test]
async fn sal20_a_later_lead_does_not_overwrite_the_acquisition() {
    let (t, events) = boot_events().await;
    c1(&t).await;
    let attribution = campaign_attribution(t.state());
    assert!(
        attribution
            .attribute(t.state(), Attribute::party(P, "referral"))
            .await
            .unwrap()
    );
    // A second attribution is kept out too.
    assert!(
        !attribution
            .attribute(t.state(), Attribute::party(P, "social"))
            .await
            .unwrap()
    );
    won_lead(&t, P, Subject::new("test.quotation", 2)).await;
    let acquisition = acquisition_of(&t, P).await;
    assert_eq!(acquisition.source_id, source_id(&t, "referral").await);
    assert_eq!(acquisition.campaign_id, None);
    assert_eq!(events.won.lock().unwrap().len(), 1);
}

/// AC5: a counter sale for a new walk-in party cannot complete until a source
/// is picked; with 05's setting, *social* also needs a campaign.
#[renox::test]
async fn sal20_a_new_partys_sale_needs_a_source_and_where_set_a_campaign() {
    let (t, _) =
        boot_events_with(WlSales::new().campaign_required_for(&["paid_ads", "social"])).await;
    c1(&t).await;
    let check = |source: Option<&'static str>, campaign: Option<&'static str>| {
        let state = t.state().clone();
        async move {
            acquisition::check_sale_source(&state, P, source, campaign)
                .await
                .unwrap()
        }
    };
    assert_eq!(
        check(None, None).await,
        Err(AcquisitionRefused::SourceRequired)
    );
    assert_eq!(
        check(Some("nowhere"), None).await,
        Err(AcquisitionRefused::UnknownSource)
    );
    assert_eq!(check(Some("walk_in"), None).await, Ok(()));
    assert_eq!(
        check(Some("social"), None).await,
        Err(AcquisitionRefused::CampaignRequired)
    );
    assert_eq!(check(Some("social"), Some("SPRING26")).await, Ok(()));
    assert_eq!(
        check(Some("social"), Some("NOPE")).await,
        Err(AcquisitionRefused::UnknownCampaign)
    );

    // Once the source is picked and stored, the party's next sale needs none.
    let stored = acquisition::attribute_party(
        t.state(),
        acquisition::AttributeParty {
            party: P,
            source: "social".into(),
            campaign: Some("SPRING26".into()),
            set_from: "staff".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert!(stored.is_some());
    assert_eq!(check(None, None).await, Ok(()));
}

/// AC6: P's first sale SO-1 is cancelled: the nightly run links SO-2, the
/// acquired date stays; with no other sale the first sale is cleared.
#[renox::test]
async fn sal20_the_nightly_run_links_the_next_sale_or_clears_the_first() {
    let (t, events) = boot_events().await;
    let attribution = campaign_attribution(t.state());
    let so = |n| Subject::new("sales.sales_order", n);
    attribution
        .attribute(t.state(), Attribute::party(P, "walk_in"))
        .await
        .unwrap();
    let first = FirstSale::new(P, so(1), 150_000, day(4, 1));
    let second = FirstSale::new(P, so(2), 90_000, day(4, 9));
    assert!(
        attribution
            .record_first_sale(t.state(), first.clone())
            .await
            .unwrap()
    );
    // Called twice for the same sale, or for a later one: the first stays.
    assert!(
        attribution
            .record_first_sale(t.state(), first)
            .await
            .unwrap()
    );
    assert!(
        !attribution
            .record_first_sale(t.state(), second)
            .await
            .unwrap()
    );
    let before = acquisition_of(&t, P).await;
    assert_eq!(before.first_sale(), Some(so(1)));
    assert_eq!(events.acquired.lock().unwrap().len(), 1);

    attribution.sale_gone(t.state(), so(1)).await.unwrap();
    let report = run_daily(t.state()).await.unwrap();
    assert_eq!((report.replaced, report.cleared, report.failed), (1, 0, 0));
    let after = acquisition_of(&t, P).await;
    assert_eq!(after.first_sale(), Some(so(2)));
    assert_eq!(after.first_sale_value, 90_000);
    assert_eq!(after.acquired_at, before.acquired_at);
    let changed = events.changed.lock().unwrap().clone();
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].old_first_sale, Some(so(1)));
    assert_eq!(changed[0].new_first_sale, Some(so(2)));
    assert_eq!(events.acquired.lock().unwrap().len(), 1, "never twice");

    // A second run finds nothing; then the last sale goes: cleared.
    let report = run_daily(t.state()).await.unwrap();
    assert_eq!((report.replaced, report.cleared), (0, 0));
    attribution.sale_gone(t.state(), so(2)).await.unwrap();
    let report = run_daily(t.state()).await.unwrap();
    assert_eq!((report.replaced, report.cleared), (0, 1));
    let cleared = acquisition_of(&t, P).await;
    assert_eq!(cleared.first_sale(), None);
    assert_eq!(cleared.first_sale_value, 0);

    // The party buys again: the new sale is its first sale.
    let third = FirstSale::new(P, so(3), 40_000, day(5, 2));
    assert!(
        attribution
            .record_first_sale(t.state(), third)
            .await
            .unwrap()
    );
    assert_eq!(acquisition_of(&t, P).await.first_sale(), Some(so(3)));
    assert_eq!(events.acquired.lock().unwrap().len(), 1);
}

/// AC7: P's first invoice of AED 2,000 excluding VAT credited by AED 500: the
/// converted value is AED 1,500. Credited in full, the nightly run replaces
/// it.
#[renox::test]
async fn sal20_a_credit_note_lowers_the_converted_value() {
    let (t, _) = boot_events().await;
    c1(&t).await;
    won_lead(&t, P, Subject::new("test.quotation", 1)).await;
    t.state().emit(invoice(5, P, 4, 2)).await.unwrap();
    t.state()
        .emit(CreditNoteIssued {
            credit_note: CreditNoteId(1),
            invoice: InvoiceId(5),
            party: P,
            amount: 52_500,
            net_amount: 50_000,
            reason: "price".into(),
        })
        .await
        .unwrap();
    assert_eq!(acquisition_of(&t, P).await.first_sale_value, 150_000);
    t.state()
        .emit(CreditNoteIssued {
            credit_note: CreditNoteId(2),
            invoice: InvoiceId(5),
            party: P,
            amount: 157_500,
            net_amount: 150_000,
            reason: "cancelled".into(),
        })
        .await
        .unwrap();
    let report = run_daily(t.state()).await.unwrap();
    assert_eq!(report.cleared, 1);
    assert_eq!(acquisition_of(&t, P).await.first_sale(), None);
}

/// With the attribution window of 12 months the converted value is the
/// party's sales of the first 12 months, net of credit notes.
#[renox::test]
async fn sal20_the_attribution_window_sums_the_first_months_of_sales() {
    let (t, _) = boot_events_with(WlSales::new().attribution_window_months(Some(12))).await;
    let attribution = campaign_attribution(t.state());
    attribution
        .attribute(t.state(), Attribute::party(P, "walk_in"))
        .await
        .unwrap();
    for (n, m, d, value) in [(1, 4, 2, 200_000), (2, 9, 1, 300_000)] {
        let sale = FirstSale::new(P, Subject::new("invoicing.invoice", n), value, day(m, d));
        attribution
            .record_first_sale(t.state(), sale)
            .await
            .unwrap();
    }
    let late = FirstSale::new(
        P,
        Subject::new("invoicing.invoice", 3),
        900_000,
        NaiveDate::from_ymd_opt(2027, 4, 2).unwrap(),
    );
    attribution
        .record_first_sale(t.state(), late)
        .await
        .unwrap();
    acquisition::refresh_value(t.state(), P.0).await.unwrap();
    assert_eq!(acquisition_of(&t, P).await.first_sale_value, 500_000);
}

/// A sale of a party with no acquisition (an import, an API sale) sets the
/// source *other* and is listed on the exceptions report.
#[renox::test]
async fn sal20_a_sale_without_a_source_is_set_to_other_and_listed() {
    let (t, events) = boot_events().await;
    let sale = FirstSale::new(P, Subject::new("invoicing.invoice", 8), 100_000, day(4, 3));
    assert!(
        campaign_attribution(t.state())
            .record_first_sale(t.state(), sale)
            .await
            .unwrap()
    );
    let acquisition = acquisition_of(&t, P).await;
    assert!(acquisition.source_missing);
    assert_eq!(acquisition.source_id, source_id(&t, OTHER_SOURCE).await);
    assert_eq!(acquisition.set_from, "import");
    assert_eq!(events.acquired.lock().unwrap().len(), 1);
    assert_eq!(events.acquired.lock().unwrap()[0].source, "other");
}

/// AC11: merging P2 (acquired 1 May) into P (acquired 1 Mar) keeps P's
/// acquisition; P2's is superseded, so the customer is counted once.
#[renox::test]
async fn sal20_a_merge_keeps_the_acquisition_acquired_first() {
    let (t, events) = boot_events().await;
    let p2 = PartyId(11);
    let attribution = campaign_attribution(t.state());
    attribution
        .attribute(t.state(), Attribute::party(P, "referral"))
        .await
        .unwrap();
    attribution
        .attribute(t.state(), Attribute::party(p2, "social"))
        .await
        .unwrap();
    let stamp = |m, d| day(m, d).and_hms_opt(9, 0, 0).unwrap().and_utc();
    PartyAcquisition::query()
        .where_(PartyAcquisition::PARTY_ID.eq(P.0))
        .update(t.db(), &[("acquired_at", &stamp(3, 1))])
        .await
        .unwrap();
    PartyAcquisition::query()
        .where_(PartyAcquisition::PARTY_ID.eq(p2.0))
        .update(t.db(), &[("acquired_at", &stamp(5, 1))])
        .await
        .unwrap();
    let sale = FirstSale::new(p2, Subject::new("invoicing.invoice", 9), 70_000, day(5, 3));
    attribution
        .record_first_sale(t.state(), sale)
        .await
        .unwrap();
    let p2_row = acquisition_of(&t, p2).await;

    t.state()
        .emit(PartiesMerged {
            survivor: P,
            merged: p2,
        })
        .await
        .unwrap();
    let kept = acquisition_of(&t, P).await;
    assert_eq!(kept.source_id, source_id(&t, "referral").await);
    let history = PartyAcquisition::find(t.db(), p2_row.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(history.superseded_by, Some(kept.id));
    assert!(
        PartyAcquisition::of_party(t.db(), p2)
            .await
            .unwrap()
            .is_none()
    );
    let live = PartyAcquisition::query()
        .where_(PartyAcquisition::SUPERSEDED_BY.is_null())
        .count(t.db())
        .await
        .unwrap();
    assert_eq!(live, 1, "the customer is counted once");
    // The sales of the merged party moved to the survivor.
    let sales = AcquisitionSale::query()
        .where_(AcquisitionSale::PARTY_ID.eq(P.0))
        .count(t.db())
        .await
        .unwrap();
    assert_eq!(sales, 1);
    assert_eq!(events.changed.lock().unwrap()[0].reason, "merged");
}
