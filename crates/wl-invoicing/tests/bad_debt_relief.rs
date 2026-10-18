//! Proposing bad-debt VAT relief (wl-invoicing §5.9, `FIN.RUL.14`): the
//! acceptance checks AC1–AC8 of #65, and the summary page and the invoice
//! page that show the proposal.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::bad_debt_relief::{self, ReliefRun};
use wl_invoicing::vat_summary::{self, VatSummary, VatSummaryStatus};
use wl_invoicing::{
    AdjustmentStatus, Invoice, InvoiceLine, ReliefMonths, VatAdjustment, WlInvoicing,
    permissions as perm,
};
use wl_kernel::LegalEntity;
use wl_kernel::documents::today;
use wl_kernel::notify::NotifyRecipient;

use support::{Booted, boot, boot_with};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("a date")
}

/// The date of supply of the invoices below, in Q1 2026.
fn supply() -> NaiveDate {
    day(2026, 3, 15)
}

/// The day the rule first proposes for `supply()`.
fn first_day() -> NaiveDate {
    day(2026, 9, 16)
}

/// Entity PGP is quarterly; its Q1 2026 is `q1`, and its Q3 2026 is `open`.
/// The invoice is written off with notice date 2026-07-01.
struct Case {
    t: Booted,
    pgp: LegalEntity,
    invoice: Invoice,
    relief: VatAdjustment,
    q3: VatSummary,
}

async fn case(q1: Option<VatSummaryStatus>) -> Case {
    let t = boot().await;
    let pgp = t.entity("PGP").await;
    if let Some(status) = q1 {
        t.summary(&pgp, supply(), status).await;
    }
    let q3 = t.summary(&pgp, first_day(), VatSummaryStatus::Open).await;
    let (invoice, relief) = t.written_off(None, supply(), Some(day(2026, 7, 1))).await;
    assert_eq!(invoice.legal_entity_id, Some(pgp.id));
    Case {
        t,
        pgp,
        invoice,
        relief,
        q3,
    }
}

impl Case {
    async fn run(&self, on: NaiveDate) -> ReliefRun {
        bad_debt_relief::run(self.t.state(), on)
            .await
            .expect("the rule runs")
    }

    async fn relief(&self) -> VatAdjustment {
        self.t.adjustment(self.relief.id).await
    }

    fn notices(&self) -> Vec<wl_kernel::notify::NotifyRequest> {
        self.t.notices.of_type(bad_debt_relief::PROPOSED)
    }
}

/// The values of the invoice the rule must never change.
fn frozen(invoice: &Invoice) -> impl PartialEq + std::fmt::Debug {
    (
        invoice.status,
        invoice.net_total,
        invoice.vat_total,
        invoice.gross_total,
        invoice.balance,
        invoice.write_off_amount,
        invoice.write_off_date,
        invoice.write_off_notice_on,
        invoice.write_off_reason_code.clone(),
        invoice.write_off_approval_id,
        invoice.write_off_recovered_amount,
    )
}

/// AC1: Q1 locked, Q3 open; the invoice (supply 2026-03-15, notice
/// 2026-07-01) is proposed on Q3 on 2026-09-16 and the accountant gets one
/// notice; the invoice is untouched and there is no credit note.
#[renox::test]
async fn relief_is_proposed_on_the_open_summary_and_the_accountant_told() {
    let c = case(Some(VatSummaryStatus::Locked)).await;
    let accountant = c.t.accountant("Zaid").await;
    let lines_before = wl_invoicing::invoicer::lines_of(c.t.db(), c.invoice.id)
        .await
        .unwrap();

    let ran = c.run(first_day()).await;
    assert_eq!(
        ran,
        ReliefRun {
            proposed: 1,
            waiting_for_open_summary: 0,
            ..Default::default()
        }
    );

    let relief = c.relief().await;
    assert_eq!(relief.status, AdjustmentStatus::Proposed);
    assert_eq!(relief.summary_id, Some(c.q3.id));
    assert!(relief.proposed_at.is_some());
    assert_eq!(relief.amount, 5_000);
    // The rule never includes anything.
    assert_eq!(relief.decided_by, None);
    assert_eq!(relief.decision_reason, None);

    let notices = c.notices();
    assert_eq!(notices.len(), 1);
    assert!(matches!(
        notices[0].recipients.as_slice(),
        [NotifyRecipient::User { id }] if *id == accountant.id
    ));
    let number = c.invoice.number.clone().unwrap();
    assert_eq!(notices[0].data["number"], number.as_str());
    assert_eq!(notices[0].data["amount"], "AED 50.00");
    assert_eq!(notices[0].data["party_name"], "Blue Dunes LLC");
    assert_eq!(notices[0].data["period"], "1 Jul 2026 to 30 Sep 2026");
    assert_eq!(
        notices[0].data["link"],
        format!("/vat-summaries/{}", c.q3.id).as_str()
    );

    let invoice = c.t.reload(&c.invoice).await;
    assert_eq!(frozen(&invoice), frozen(&c.invoice));
    let lines_after = wl_invoicing::invoicer::lines_of(c.t.db(), c.invoice.id)
        .await
        .unwrap();
    assert_eq!(lines_after.len(), lines_before.len());
    for (before, after) in lines_before.iter().zip(&lines_after) {
        assert_eq!(
            (before.net_amount, before.vat_amount, before.gross_amount),
            (after.net_amount, after.vat_amount, after.gross_amount)
        );
    }
    assert_eq!(
        InvoiceLine::query()
            .where_(InvoiceLine::INVOICE_ID.eq(c.invoice.id))
            .count(c.t.db())
            .await
            .unwrap(),
        1
    );
    // No credit note: the invoice is the only document.
    assert_eq!(Invoice::query().count(c.t.db()).await.unwrap(), 1);
}

/// AC2: the day before, the adjustment stays `pending` and nobody is told.
#[renox::test]
async fn relief_waits_for_the_day_after_the_months() {
    let c = case(Some(VatSummaryStatus::Locked)).await;
    c.t.accountant("Zaid").await;

    let ran = c.run(day(2026, 9, 15)).await;
    assert_eq!(ran, ReliefRun::default());
    assert_eq!(c.relief().await.status, AdjustmentStatus::Pending);
    assert_eq!(c.relief().await.summary_id, None);
    assert!(c.notices().is_empty());

    assert_eq!(c.run(first_day()).await.proposed, 1);
}

/// AC3: no notice date, Q1 only `reviewed`, or no Q1 summary at all: the
/// adjustment stays `pending`; once the condition holds, the next run
/// proposes it.
#[renox::test]
async fn relief_needs_the_notice_and_the_locked_supply_period() {
    // Without a notice date.
    let c = case(Some(VatSummaryStatus::Locked)).await;
    c.t.accountant("Zaid").await;
    Invoice::query()
        .where_(Invoice::ID.eq(c.invoice.id))
        .update(c.t.db(), &[("write_off_notice_on", &None::<NaiveDate>)])
        .await
        .unwrap();
    assert_eq!(c.run(first_day()).await, ReliefRun::default());
    assert_eq!(c.relief().await.status, AdjustmentStatus::Pending);
    assert!(c.notices().is_empty());
    Invoice::query()
        .where_(Invoice::ID.eq(c.invoice.id))
        .update(c.t.db(), &[("write_off_notice_on", &Some(day(2026, 7, 1)))])
        .await
        .unwrap();
    assert_eq!(c.run(first_day()).await.proposed, 1);

    // Q1 reviewed, then locked.
    let c = case(Some(VatSummaryStatus::Reviewed)).await;
    c.t.accountant("Zaid").await;
    assert_eq!(c.run(first_day()).await, ReliefRun::default());
    assert_eq!(c.relief().await.status, AdjustmentStatus::Pending);
    assert!(c.notices().is_empty());
    let q1 = VatSummary::containing(c.t.db(), &c.t.config(), &c.pgp, supply())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(q1.status, VatSummaryStatus::Reviewed);
    let reviewer = c.t.helper("Rev", &[]).await;
    c.t.post_transition(&q1, "lock", &reviewer, "")
        .await
        .assert_status(303);
    assert_eq!(c.run(first_day()).await.proposed, 1);
    assert_eq!(c.relief().await.status, AdjustmentStatus::Proposed);

    // No Q1 summary: it is created, prepared, reviewed and locked later.
    let c = case(None).await;
    c.t.accountant("Zaid").await;
    assert_eq!(c.run(first_day()).await, ReliefRun::default());
    assert_eq!(c.relief().await.status, AdjustmentStatus::Pending);
    assert!(c.notices().is_empty());
    c.t.summary(&c.pgp, supply(), VatSummaryStatus::Locked)
        .await;
    assert_eq!(c.run(first_day()).await.proposed, 1);
}

/// AC4: a second run the same day does nothing; a run skipped for days
/// proposes on the next run.
#[renox::test]
async fn relief_is_proposed_once_and_catches_up() {
    let c = case(Some(VatSummaryStatus::Locked)).await;
    c.t.accountant("Zaid").await;
    assert_eq!(c.run(first_day()).await.proposed, 1);
    assert_eq!(c.run(first_day()).await, ReliefRun::default());
    assert_eq!(c.notices().len(), 1);

    let c = case(Some(VatSummaryStatus::Locked)).await;
    c.t.accountant("Zaid").await;
    // Nobody ran on the 16th or the 17th.
    assert_eq!(c.run(day(2026, 9, 18)).await.proposed, 1);
    assert_eq!(c.relief().await.summary_id, Some(c.q3.id));
    assert_eq!(c.notices().len(), 1);
}

/// AC5: no `open` summary (Q3 already `prepared`): nothing is proposed and
/// nobody is told; after an `open` summary exists the next run proposes.
#[renox::test]
async fn relief_waits_for_an_open_summary() {
    let c = case(Some(VatSummaryStatus::Locked)).await;
    c.t.accountant("Zaid").await;
    c.t.summary(&c.pgp, first_day(), VatSummaryStatus::Prepared)
        .await;
    assert_eq!(
        c.t.summary_now(c.q3.id).await.status,
        VatSummaryStatus::Prepared
    );

    let ran = c.run(first_day()).await;
    assert_eq!(
        ran,
        ReliefRun {
            proposed: 0,
            waiting_for_open_summary: 1,
            ..Default::default()
        }
    );
    assert_eq!(c.relief().await.status, AdjustmentStatus::Pending);
    assert!(c.notices().is_empty());

    // The next period has its open summary.
    let q4_day = day(2026, 10, 2);
    let config = c.t.config();
    let q4 = vat_summary::ensure_open(c.t.db(), &config, &c.pgp, q4_day)
        .await
        .unwrap()
        .expect("Q4 is new");
    assert_eq!(c.run(q4_day).await.proposed, 1);
    assert_eq!(c.relief().await.summary_id, Some(q4.id));
    assert_eq!(c.notices().len(), 1);
}

/// AC6: the months setting is never below 6, the defaults, and the
/// scheduled task runs the rule.
#[renox::test]
async fn the_settings_and_the_scheduled_task() {
    let refused = ReliefMonths::new(5).unwrap_err();
    assert_eq!(
        refused.to_string(),
        "The months after the date of supply cannot be below 6"
    );
    assert!(ReliefMonths::new(6).is_ok());

    let t = boot().await;
    let config = t.config();
    assert!(config.bad_debt_relief);
    assert_eq!(config.bad_debt_relief_run_at, "02:00");
    assert_eq!(config.bad_debt_relief_months.get(), 6);

    // A supply long ago, its period locked, and the running period open:
    // the task (which runs for today) proposes.
    let pgp = t.entity("PGP").await;
    t.summary(&pgp, day(2020, 3, 15), VatSummaryStatus::Locked)
        .await;
    let running = t.summary(&pgp, today(), VatSummaryStatus::Open).await;
    let (_, relief) = t
        .written_off(None, day(2020, 3, 15), Some(day(2020, 7, 1)))
        .await;
    t.app
        .kernel()
        .run_scheduled(bad_debt_relief::TASK)
        .await
        .expect("the task runs");
    let relief = t.adjustment(relief.id).await;
    assert_eq!(relief.status, AdjustmentStatus::Proposed);
    assert_eq!(relief.summary_id, Some(running.id));

    // Switched off, the task is not scheduled.
    let off = boot_with(WlInvoicing::new().bad_debt_relief(false)).await;
    assert!(
        off.app
            .kernel()
            .run_scheduled(bad_debt_relief::TASK)
            .await
            .is_err()
    );
}

/// A longer wait than 6 months holds the proposal back.
#[renox::test]
async fn the_months_setting_is_used() {
    let t =
        boot_with(WlInvoicing::new().bad_debt_relief_months(ReliefMonths::new(9).unwrap())).await;
    let pgp = t.entity("PGP").await;
    t.summary(&pgp, supply(), VatSummaryStatus::Locked).await;
    t.summary(&pgp, first_day(), VatSummaryStatus::Open).await;
    let (_, relief) = t.written_off(None, supply(), Some(day(2026, 7, 1))).await;
    t.accountant("Zaid").await;
    assert_eq!(
        bad_debt_relief::run(t.state(), first_day())
            .await
            .unwrap()
            .proposed,
        0
    );
    assert_eq!(
        t.adjustment(relief.id).await.status,
        AdjustmentStatus::Pending
    );
    // 2026-03-15 + 9 months: first on 2026-12-16, which needs an open
    // summary of its own period.
    let q4 = day(2026, 12, 16);
    t.summary(&pgp, q4, VatSummaryStatus::Open).await;
    assert_eq!(
        bad_debt_relief::run(t.state(), q4).await.unwrap().proposed,
        1
    );
}

/// AC7: on the summary page an accountant excludes the proposal with a
/// reason; it is never proposed again. A missing reason is refused (422),
/// a user without `vat.prepare` gets 403, and the rule never includes.
#[renox::test]
async fn an_accountant_excludes_the_proposal_with_a_reason() {
    let c = case(Some(VatSummaryStatus::Locked)).await;
    let accountant = c.t.accountant("Zaid").await;
    c.run(first_day()).await;
    let relief = c.relief().await;
    assert_eq!(relief.status, AdjustmentStatus::Proposed);

    // The page offers both sheets.
    c.t.app.acting_as(&accountant);
    let page = c.t.app.get(&format!("/vat-summaries/{}", c.q3.id)).await;
    page.assert_ok()
        .assert_see("Bad-debt relief")
        .assert_see(&c.invoice.number.clone().unwrap())
        .assert_see("AED 50.00")
        .assert_see("Include")
        .assert_see("Exclude");

    // No reason: refused, nothing changes.
    for action in ["exclude", "include"] {
        for reason in ["", "   "] {
            c.t.post_decision(&c.q3, &relief, action, &accountant, reason)
                .await
                .assert_status(422);
        }
    }
    assert_eq!(c.relief().await.status, AdjustmentStatus::Proposed);

    // Without `vat.prepare`.
    let reader = c.t.helper("Rita", &[perm::VAT_VIEW]).await;
    c.t.post_decision(&c.q3, &relief, "exclude", &reader, "Not mine")
        .await
        .assert_forbidden();
    let nobody = c.t.viewer().await;
    c.t.post_decision(&c.q3, &relief, "include", &nobody, "Not mine")
        .await
        .assert_forbidden();
    assert_eq!(c.relief().await.status, AdjustmentStatus::Proposed);

    // Excluded with a reason.
    c.t.post_decision(&c.q3, &relief, "exclude", &accountant, "Customer paid")
        .await
        .assert_ok();
    let excluded = c.relief().await;
    assert_eq!(excluded.status, AdjustmentStatus::Excluded);
    assert_eq!(excluded.decided_by, Some(accountant.id));
    assert!(excluded.decided_at.is_some());
    assert_eq!(excluded.decision_reason.as_deref(), Some("Customer paid"));
    // And once decided, the other choice is gone.
    c.t.post_decision(&c.q3, &relief, "include", &accountant, "Changed my mind")
        .await
        .assert_status(422);

    // Later runs never propose it again.
    assert_eq!(c.run(first_day()).await, ReliefRun::default());
    assert_eq!(c.run(day(2026, 12, 1)).await, ReliefRun::default());
    assert_eq!(c.relief().await.status, AdjustmentStatus::Excluded);
    assert_eq!(c.notices().len(), 1);

    // The pages show the decision.
    c.t.app.acting_as(&accountant);
    c.t.app
        .get(&format!("/vat-summaries/{}", c.q3.id))
        .await
        .assert_ok()
        .assert_see("Customer paid")
        .assert_see("Excluded")
        .assert_dont_see("Exclude</");
    c.t.app
        .get(&format!("/invoices/{}", c.invoice.id))
        .await
        .assert_ok()
        .assert_see("VAT bad-debt relief")
        .assert_see("Excluded")
        .assert_see("On the VAT summary for 2026-07-01 to 2026-09-30")
        .assert_see("Customer paid");
}

/// AC7: include with a reason gives `included`; the rule did not.
#[renox::test]
async fn an_accountant_includes_the_proposal_with_a_reason() {
    let c = case(Some(VatSummaryStatus::Locked)).await;
    let accountant = c.t.accountant("Zaid").await;
    c.run(first_day()).await;
    let relief = c.relief().await;
    assert_ne!(relief.status, AdjustmentStatus::Included);

    c.t.post_decision(&c.q3, &relief, "include", &accountant, "Meets the rules")
        .await
        .assert_ok();
    let included = c.relief().await;
    assert_eq!(included.status, AdjustmentStatus::Included);
    assert_eq!(included.decided_by, Some(accountant.id));
    assert_eq!(included.decision_reason.as_deref(), Some("Meets the rules"));
    assert_eq!(c.run(first_day()).await, ReliefRun::default());

    c.t.app.acting_as(&accountant);
    c.t.app
        .get(&format!("/invoices/{}", c.invoice.id))
        .await
        .assert_ok()
        .assert_see("Included")
        .assert_see("Meets the rules");
}

/// A decision is refused once the summary is `reviewed`: the preparer has
/// handed it over.
#[renox::test]
async fn no_decision_on_a_reviewed_summary() {
    let c = case(Some(VatSummaryStatus::Locked)).await;
    let accountant = c.t.accountant("Zaid").await;
    c.run(first_day()).await;
    let relief = c.relief().await;
    let q3 =
        c.t.summary(&c.pgp, first_day(), VatSummaryStatus::Reviewed)
            .await;
    assert_eq!(q3.id, c.q3.id);
    assert_eq!(q3.status, VatSummaryStatus::Reviewed);
    c.t.post_decision(&q3, &relief, "exclude", &accountant, "Too late")
        .await
        .assert_status(422);
    assert_eq!(c.relief().await.status, AdjustmentStatus::Proposed);
    // The page offers no sheet then.
    c.t.app.acting_as(&accountant);
    c.t.app
        .get(&format!("/vat-summaries/{}", q3.id))
        .await
        .assert_ok()
        .assert_dont_see("exclude-adjustment-");
}

/// The invoice page's write-off section follows the relief: pending, then
/// proposed on the period.
#[renox::test]
async fn the_invoice_page_shows_where_the_relief_stands() {
    let c = case(Some(VatSummaryStatus::Locked)).await;
    let accountant = c.t.accountant("Zaid").await;
    c.t.app.acting_as(&accountant);
    let url = format!("/invoices/{}", c.invoice.id);
    c.t.app
        .get(&url)
        .await
        .assert_ok()
        .assert_see("VAT bad-debt relief")
        .assert_see("Pending")
        .assert_dont_see("On the VAT summary for");
    c.run(first_day()).await;
    c.t.app
        .get(&url)
        .await
        .assert_ok()
        .assert_see("Proposed")
        .assert_see("On the VAT summary for 2026-07-01 to 2026-09-30");
}

/// AC8: an invoice of PGR goes onto PGR's open summary, not PGP's.
#[renox::test]
async fn relief_goes_to_the_issuing_entitys_summary() {
    let t = boot().await;
    let pgp = t.entity("PGP").await;
    let pgr = t.new_entity("PGR").await;
    t.summary(&pgp, supply(), VatSummaryStatus::Locked).await;
    t.summary(&pgr, supply(), VatSummaryStatus::Locked).await;
    let pgp_q3 = t.summary(&pgp, first_day(), VatSummaryStatus::Open).await;
    let pgr_q3 = t.summary(&pgr, first_day(), VatSummaryStatus::Open).await;
    let (invoice, relief) = t
        .written_off(Some(&pgr), supply(), Some(day(2026, 7, 1)))
        .await;
    assert_eq!(invoice.legal_entity_id, Some(pgr.id));
    assert_eq!(relief.legal_entity_id, Some(pgr.id));
    t.accountant("Zaid").await;

    let ran = bad_debt_relief::run(t.state(), first_day()).await.unwrap();
    assert_eq!(ran.proposed, 1);
    let relief = t.adjustment(relief.id).await;
    assert_eq!(relief.status, AdjustmentStatus::Proposed);
    assert_eq!(relief.summary_id, Some(pgr_q3.id));
    assert_ne!(relief.summary_id, Some(pgp_q3.id));
}

/// AC8: a PGR invoice whose supply period is locked only for PGP is not
/// proposed.
#[renox::test]
async fn another_entitys_locked_period_does_not_count() {
    let t = boot().await;
    let pgp = t.entity("PGP").await;
    let pgr = t.new_entity("PGR").await;
    t.summary(&pgp, supply(), VatSummaryStatus::Locked).await;
    // PGR's Q1 is open (not locked); both have an open Q3.
    t.summary(&pgr, supply(), VatSummaryStatus::Open).await;
    t.summary(&pgp, first_day(), VatSummaryStatus::Open).await;
    t.summary(&pgr, first_day(), VatSummaryStatus::Open).await;
    let (_, relief) = t
        .written_off(Some(&pgr), supply(), Some(day(2026, 7, 1)))
        .await;
    t.accountant("Zaid").await;

    let ran = bad_debt_relief::run(t.state(), first_day()).await.unwrap();
    assert_eq!(ran, ReliefRun::default());
    assert_eq!(
        t.adjustment(relief.id).await.status,
        AdjustmentStatus::Pending
    );
    assert!(t.notices.of_type(bad_debt_relief::PROPOSED).is_empty());
}
