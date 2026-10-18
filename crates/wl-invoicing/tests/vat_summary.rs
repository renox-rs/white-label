//! The VAT summary (wl-invoicing §4, `FIN.ACL.10`): the lifecycle, the
//! running period's summary, and its pages; AC9 of #65.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::vat_summary::{self, VatSummary, VatSummaryStatus};
use wl_kernel::documents::today;

use support::{Booted, boot};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("a date")
}

/// Q1 2026: over.
fn over() -> NaiveDate {
    day(2026, 2, 10)
}

async fn summaries_of(t: &Booted, entity: i64) -> Vec<VatSummary> {
    VatSummary::query()
        .where_(VatSummary::LEGAL_ENTITY_ID.eq(entity))
        .order_by(VatSummary::PERIOD_START)
        .get(t.db())
        .await
        .unwrap()
}

/// AC9: `prepare` before the end of the period is refused; after it, it
/// records the preparer.
#[renox::test]
async fn prepare_waits_for_the_end_of_the_period() {
    let t = boot().await;
    let pgp = t.entity("PGP").await;
    let ann = t.accountant("Ann").await;
    let running = t.summary(&pgp, today(), VatSummaryStatus::Open).await;
    assert!(running.period_end >= today());
    t.post_transition(&running, "prepare", &ann, "")
        .await
        .assert_status(422);
    assert_eq!(
        t.summary_now(running.id).await.status,
        VatSummaryStatus::Open
    );

    let q1 = t.summary(&pgp, over(), VatSummaryStatus::Open).await;
    t.post_transition(&q1, "prepare", &ann, "")
        .await
        .assert_status(303);
    let prepared = t.summary_now(q1.id).await;
    assert_eq!(prepared.status, VatSummaryStatus::Prepared);
    assert_eq!(prepared.prepared_by, Some(ann.id));
    assert!(prepared.prepared_at.is_some());
}

/// AC9: `review` by the preparer is refused (`FIN.ACL.10`); by a second
/// accountant it succeeds.
#[renox::test]
async fn the_preparer_does_not_review() {
    let t = boot().await;
    let pgp = t.entity("PGP").await;
    let ann = t.accountant("Ann").await;
    let ben = t.accountant("Ben").await;
    let own = t
        .summary(&pgp, day(2025, 11, 5), VatSummaryStatus::Open)
        .await;
    t.post_transition(&own, "prepare", &ann, "")
        .await
        .assert_status(303);
    t.post_transition(&own, "review", &ann, "")
        .await
        .assert_status(422);
    assert_eq!(
        t.summary_now(own.id).await.status,
        VatSummaryStatus::Prepared
    );
    t.post_transition(&own, "review", &ben, "")
        .await
        .assert_status(303);
    let reviewed = t.summary_now(own.id).await;
    assert_eq!(reviewed.status, VatSummaryStatus::Reviewed);
    assert_eq!(reviewed.reviewed_by, Some(ben.id));
    assert_eq!(reviewed.prepared_by, Some(ann.id));
}

/// AC9: `send_back` needs a reason; `lock` gives `locked`; a locked
/// summary goes no further.
#[renox::test]
async fn send_back_needs_a_reason_and_lock_closes_the_period() {
    let t = boot().await;
    let pgp = t.entity("PGP").await;
    let ann = t.accountant("Ann").await;
    let ben = t.accountant("Ben").await;
    let q1 = t.summary(&pgp, over(), VatSummaryStatus::Open).await;
    t.post_transition(&q1, "prepare", &ann, "")
        .await
        .assert_status(303);
    t.post_transition(&q1, "review", &ben, "")
        .await
        .assert_status(303);

    t.post_transition(&q1, "send_back", &ben, "")
        .await
        .assert_status(422);
    assert_eq!(
        t.summary_now(q1.id).await.status,
        VatSummaryStatus::Reviewed
    );
    t.post_transition(&q1, "send_back", &ben, "Wrong figure")
        .await
        .assert_status(303);
    let back = t.summary_now(q1.id).await;
    assert_eq!(back.status, VatSummaryStatus::Prepared);
    assert_eq!(back.send_back_reason.as_deref(), Some("Wrong figure"));
    assert_eq!(back.reviewed_by, None);

    t.post_transition(&q1, "review", &ben, "")
        .await
        .assert_status(303);
    t.post_transition(&q1, "lock", &ben, "")
        .await
        .assert_status(303);
    let locked = t.summary_now(q1.id).await;
    assert_eq!(locked.status, VatSummaryStatus::Locked);
    assert_eq!(locked.locked_by, Some(ben.id));
    assert!(locked.locked_at.is_some());

    for name in ["prepare", "review", "send_back", "lock"] {
        t.post_transition(&locked, name, &ann, "Because")
            .await
            .assert_status(422);
    }
    assert_eq!(t.summary_now(q1.id).await.status, VatSummaryStatus::Locked);
    let history: Vec<String> = renox::state_machine::history(t.db(), VatSummary::TABLE, q1.id)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.transition)
        .collect();
    assert_eq!(
        history,
        ["prepare", "review", "send_back", "review", "lock"]
    );
}

/// AC9: the transitions need their permission.
#[renox::test]
async fn each_transition_needs_its_permission() {
    let t = boot().await;
    let pgp = t.entity("PGP").await;
    let reader = t
        .helper("Rita", &[wl_invoicing::permissions::VAT_VIEW])
        .await;
    let q1 = t.summary(&pgp, over(), VatSummaryStatus::Open).await;
    t.post_transition(&q1, "prepare", &reader, "")
        .await
        .assert_forbidden();
    assert_eq!(t.summary_now(q1.id).await.status, VatSummaryStatus::Open);
}

/// AC9: saving a changed status outside a transition is refused.
#[renox::test]
async fn the_status_changes_only_through_a_transition() {
    let t = boot().await;
    let pgp = t.entity("PGP").await;
    let q1 = t.summary(&pgp, over(), VatSummaryStatus::Open).await;
    let mut changed = q1.clone();
    changed.status = VatSummaryStatus::Locked;
    assert!(changed.save(t.db()).await.is_err());
    assert_eq!(t.summary_now(q1.id).await.status, VatSummaryStatus::Open);
}

/// AC9: `ensure_open` creates one summary per running period and never a
/// second; the daily task does the same for every VAT-registered entity.
#[renox::test]
async fn ensure_open_creates_one_summary_per_period() {
    let t = boot().await;
    let pgp = t.entity("PGP").await;
    let config = t.config();
    let first = vat_summary::ensure_open(t.db(), &config, &pgp, today())
        .await
        .unwrap()
        .expect("the first call creates it");
    assert_eq!(first.status, VatSummaryStatus::Open);
    assert!(first.period_start <= today() && today() <= first.period_end);
    assert!(
        vat_summary::ensure_open(t.db(), &config, &pgp, today())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(summaries_of(&t, pgp.id).await.len(), 1);

    // The task: nothing new for PGP, nothing for PGFZ (not VAT registered).
    t.app
        .kernel()
        .run_scheduled(vat_summary::TASK)
        .await
        .expect("the task runs");
    assert_eq!(summaries_of(&t, pgp.id).await.len(), 1);
    let pgfz = t.entity("PGFZ").await;
    assert!(summaries_of(&t, pgfz.id).await.is_empty());

    // A period already prepared early is not given a second summary.
    let early = t.summary(&pgp, over(), VatSummaryStatus::Prepared).await;
    assert!(
        vat_summary::ensure_open(t.db(), &config, &pgp, over())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        t.summary_now(early.id).await.status,
        VatSummaryStatus::Prepared
    );
    assert_eq!(summaries_of(&t, pgp.id).await.len(), 2);
}

/// AC9: `/vat-summaries` answers 200 with `vat.view` and 403 without it;
/// the grid lists the summaries.
#[renox::test]
async fn the_list_needs_vat_view_and_shows_the_summaries() {
    let t = boot().await;
    let pgp = t.entity("PGP").await;
    let q1 = t.summary(&pgp, over(), VatSummaryStatus::Locked).await;
    let running = t.summary(&pgp, today(), VatSummaryStatus::Open).await;
    let accountant = t.accountant("Ann").await;
    let viewer = t.viewer().await;

    t.app.acting_as(&accountant);
    t.app
        .get("/vat-summaries")
        .await
        .assert_ok()
        .assert_see("VAT summaries")
        .assert_see("PGP")
        .assert_see("2026-01-01")
        .assert_see("Locked")
        .assert_see(&format!("/vat-summaries/{}", q1.id))
        .assert_see(&format!("/vat-summaries/{}", running.id));

    t.app.acting_as(&viewer);
    t.app.get("/vat-summaries").await.assert_forbidden();
    t.app
        .get(&format!("/vat-summaries/{}", q1.id))
        .await
        .assert_forbidden();
}

/// The page of a summary: period, who did what, the actions of its state
/// and the history.
#[renox::test]
async fn the_summary_page_shows_who_did_what_and_the_actions() {
    let t = boot().await;
    let pgp = t.entity("PGP").await;
    let ann = t.accountant("Ann").await;
    let ben = t.accountant("Ben").await;
    let q1 = t.summary(&pgp, over(), VatSummaryStatus::Open).await;

    t.app.acting_as(&ann);
    let url = format!("/vat-summaries/{}", q1.id);
    t.app
        .get(&url)
        .await
        .assert_ok()
        .assert_see("PGP 2026-01-01 to 2026-03-31")
        .assert_see("Prepare")
        .assert_see("No adjustments");

    t.post_transition(&q1, "prepare", &ann, "")
        .await
        .assert_status(303);
    t.post_transition(&q1, "review", &ben, "")
        .await
        .assert_status(303);
    t.app.acting_as(&ben);
    t.app
        .get(&url)
        .await
        .assert_ok()
        .assert_see("Prepared by")
        .assert_see("Ann")
        .assert_see("Ben")
        .assert_see("Send back")
        .assert_see("Lock");
}
