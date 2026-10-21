//! BL-LOG-22 (#320), 03 §4.2 row *Supplier bill arrives after the job is
//! closed*, §7.1 *Freight job* rows `close` and `reopen`, §9 *VAT* and
//! *Disbursements*: a closed job is reopened with a reason, the late cost is
//! allocated with its liability, a supplementary invoice is drafted and
//! issued when it is rechargeable, and the job is closed again with the new
//! margin while the earlier margin stays in the history. The first invoice is
//! never changed. The supplementary invoice is a real wl-invoicing invoice.

mod support;

use logistics::freight_costing::{self, JobLine, line_kind};
use logistics::freight_reopen;
use logistics::{FreightJob, FreightStatus};
use renox::prelude::*;
use wl_invoicing::invoice::Invoice;
use wl_kernel::invoicing::InvoiceStatus;

use support::{World, boot, boot_with_invoicing};

/// The latest cost line of the job, as stored now.
async fn last_cost(w: &World, job: &FreightJob) -> JobLine {
    JobLine::query()
        .where_(JobLine::JOB_ID.eq(job.id))
        .where_(JobLine::KIND.eq(line_kind::COST))
        .order_by_desc(JobLine::ID)
        .first(w.db())
        .await
        .unwrap()
        .expect("a cost line")
}

/// A late cost of `amount` dirhams recorded and allocated with `liability`,
/// rechargeable when `recharge`.
async fn late_cost(w: &World, job: &FreightJob, amount: &str, liability: &str, recharge: bool) -> JobLine {
    w.post(
        &w.docs,
        &format!("/freight/{}/costs", job.id),
        &[
            ("description", "Shipping line demurrage bill"),
            ("amount", amount),
            ("source_ref", "SB-2044"),
        ],
    )
    .await
    .assert_status(303);
    let line = last_cost(w, job).await;
    w.post(
        &w.docs,
        &format!("/freight/lines/{}/allocate", line.id),
        &[
            ("liability", liability),
            ("rechargeable", if recharge { "1" } else { "0" }),
        ],
    )
    .await
    .assert_status(303);
    last_cost(w, job).await
}

/// The margins at each close, in fils.
async fn margins(w: &World, job: &FreightJob) -> Vec<i64> {
    freight_costing::margins_of(w.db(), job.id)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.margin)
        .collect()
}

/// AC1: a job closed with a margin of AED 3,000: the documentation officer
/// reopens it with the reason *demurrage*; the job is `delivered`, the
/// reopening keeps its reason and the margin it reopened, and the close is
/// still in the history.
#[renox::test]
async fn a_closed_job_is_reopened_with_a_reason_and_is_delivered_again() {
    let w = boot().await;
    let job = w.job_closed().await;
    assert_eq!(margins(&w, &job).await, [300_000]);

    // A reason is needed, a known one, and a note for *other*.
    w.job_action(&w.docs, &job, "reopen", &[("reason", " ")])
        .await
        .assert_status(422);
    w.job_action(&w.docs, &job, "reopen", &[("reason", "boredom")])
        .await
        .assert_status(422);
    w.job_action(&w.docs, &job, "reopen", &[("reason", "other"), ("note", " ")])
        .await
        .assert_status(422);
    // Only a holder of `jobs.close` reopens.
    w.job_action(&w.docs_limited, &job, "reopen", &[("reason", "demurrage")])
        .await
        .assert_forbidden();
    assert_eq!(w.job(job.id).await.status, FreightStatus::Closed);

    w.job_action(
        &w.docs,
        &job,
        "reopen",
        &[("reason", "demurrage"), ("note", "bill of 2026-11-15")],
    )
    .await
    .assert_status(303);
    assert_eq!(w.job(job.id).await.status, FreightStatus::Delivered);
    let reopenings = freight_reopen::reopenings_of(w.db(), job.id).await.unwrap();
    assert_eq!(reopenings.len(), 1);
    assert_eq!(reopenings[0].reason, "demurrage");
    assert_eq!(reopenings[0].margin_before, 300_000);
    assert_eq!(reopenings[0].reopened_by, Some(w.docs.id));
    assert_eq!(margins(&w, &job).await, [300_000]);
    // Reopened, it is not closed: it cannot be reopened again.
    w.job_action(&w.docs, &job, "reopen", &[("reason", "demurrage")])
        .await
        .assert_status(422);
}

/// A cancelled job is never reopened, nor is one that is not closed.
#[renox::test]
async fn a_cancelled_or_open_job_is_not_reopened() {
    let w = boot().await;
    let cancelled = w.open_job().await;
    w.job_action(&w.docs, &cancelled, "cancel", &[("reason", "client changed plans")])
        .await
        .assert_status(303);
    w.job_action(&w.docs, &cancelled, "reopen", &[("reason", "demurrage")])
        .await
        .assert_status(422);
    assert_eq!(w.job(cancelled.id).await.status, FreightStatus::Cancelled);
    let delivered = w.job_delivered().await;
    w.job_action(&w.docs, &delivered, "reopen", &[("reason", "demurrage")])
        .await
        .assert_status(422);
}

/// AC4: with liability *company*, no supplementary invoice and the new margin
/// is AED 2,200; the earlier margin stays in the history.
#[renox::test]
async fn a_late_cost_the_company_carries_lowers_the_margin_and_drafts_no_invoice() {
    let w = boot().await;
    let job = w.job_closed().await;
    w.job_action(&w.docs, &job, "reopen", &[("reason", "late_supplier_bill")])
        .await
        .assert_status(303);

    let line = late_cost(&w, &job, "800.00", "company", false).await;
    assert!(line.late);
    assert_eq!(line.liability.as_deref(), Some("company"));
    assert!(!line.rechargeable);
    // Nothing to recharge: no invoice is drafted.
    w.post(&w.docs, &format!("/freight/{}/supplementary", job.id), &[])
        .await
        .assert_status(422)
        .assert_see("no late cost to recharge");
    assert!(w.invoices.drafts().is_empty());

    w.job_action(&w.docs, &job, "close", &[])
        .await
        .assert_status(303);
    assert_eq!(w.job(job.id).await.status, FreightStatus::Closed);
    assert_eq!(margins(&w, &job).await, [300_000, 220_000]);
    let figures = freight_costing::figures_of(w.db(), job.id).await.unwrap();
    assert_eq!(figures.margin, 220_000);
}

/// The same guards close the job again: a late cost with no liability waits.
#[renox::test]
async fn close_again_has_the_same_guards() {
    let w = boot().await;
    let job = w.job_closed().await;
    w.job_action(&w.docs, &job, "reopen", &[("reason", "port_storage")])
        .await
        .assert_status(303);
    w.post(
        &w.docs,
        &format!("/freight/{}/costs", job.id),
        &[("description", "Port storage"), ("amount", "120.00")],
    )
    .await
    .assert_status(303);
    w.job_action(&w.docs, &job, "close", &[])
        .await
        .assert_status(422)
        .assert_see("Allocate every supplier bill");
    assert_eq!(w.job(job.id).await.status, FreightStatus::Delivered);
}

/// AC2 and AC3: not marked international: the AED 800 allocated to the client
/// and recharged is a reimbursement with the main service's 5%: a
/// supplementary invoice of AED 800 + AED 40 VAT, drafted and issued; on
/// `close` the margin is AED 3,000 (revenue +800, cost +800) and the earlier
/// margin stays in the history. The first invoice is not changed.
#[renox::test]
async fn a_rechargeable_late_cost_is_invoiced_with_five_percent_vat_and_keeps_the_margin() {
    let w = boot_with_invoicing().await;
    let client = w.client_party("Najm Client LLC").await;
    *w.job_client.lock().unwrap() = client;
    let job = w.job_closed().await;
    let first = freight_costing::lines_of(w.db(), job.id)
        .await
        .unwrap()
        .into_iter()
        .find(|line| line.kind == line_kind::REVENUE)
        .unwrap();

    w.job_action(&w.docs, &job, "reopen", &[("reason", "demurrage")])
        .await
        .assert_status(303);
    late_cost(&w, &job, "800.00", "client", true).await;
    w.post(&w.docs, &format!("/freight/{}/supplementary", job.id), &[])
        .await
        .assert_status(303);
    let lines = freight_costing::lines_of(w.db(), job.id).await.unwrap();
    let supplementary = lines
        .iter()
        .find(|line| line.kind == line_kind::REVENUE && line.late)
        .expect("a revenue line for the supplementary invoice");
    assert_eq!(supplementary.amount, 80_000);
    assert!(supplementary.issued);
    let invoice = Invoice::find_or_404(w.db(), supplementary.invoice_id.unwrap())
        .await
        .unwrap();
    assert_eq!(invoice.status, InvoiceStatus::Issued);
    assert_eq!(invoice.net_total, 80_000);
    assert_eq!(invoice.vat_total, 4_000);
    assert_eq!(invoice.gross_total, 84_000);
    // The cost is marked as recharged on it; a second call finds nothing.
    assert_eq!(last_cost(&w, &job).await.invoice_id, Some(invoice.id));
    w.post(&w.docs, &format!("/freight/{}/supplementary", job.id), &[])
        .await
        .assert_status(422);
    // The first invoice's revenue line is as it was.
    let unchanged = freight_costing::lines_of(w.db(), job.id)
        .await
        .unwrap()
        .into_iter()
        .find(|line| line.id == first.id)
        .unwrap();
    assert_eq!(unchanged.amount, first.amount);
    assert_eq!(unchanged.invoice_id, first.invoice_id);

    w.job_action(&w.docs, &job, "close", &[])
        .await
        .assert_status(303);
    assert_eq!(w.job(job.id).await.status, FreightStatus::Closed);
    // Revenue +800 and cost +800 each time it closed: 3,000 each time.
    let history = margins(&w, &job).await;
    assert_eq!(history.first(), Some(&300_000));
    assert_eq!(history.last(), Some(&300_000));
}

/// AC2 (the other half): with the three conditions of VATP013 recorded the
/// AED 800 is a disbursement, `out_of_scope`: no VAT.
#[renox::test]
async fn a_disbursement_is_out_of_scope_and_carries_no_vat() {
    let w = boot_with_invoicing().await;
    let client = w.client_party("Najm Client LLC").await;
    *w.job_client.lock().unwrap() = client;
    let job = w.job_closed().await;
    w.job_action(&w.docs, &job, "reopen", &[("reason", "late_supplier_bill")])
        .await
        .assert_status(303);
    let line = late_cost(&w, &job, "800.00", "client", true).await;
    w.post(
        &w.docs,
        &format!("/freight/lines/{}/disbursement", line.id),
        &[("conditions", "1")],
    )
    .await
    .assert_status(303);
    assert!(last_cost(&w, &job).await.disbursement);

    w.post(&w.docs, &format!("/freight/{}/supplementary", job.id), &[])
        .await
        .assert_status(303);
    let revenue = freight_costing::lines_of(w.db(), job.id)
        .await
        .unwrap()
        .into_iter()
        .find(|line| line.kind == line_kind::REVENUE && line.late)
        .unwrap();
    let invoice = Invoice::find_or_404(w.db(), revenue.invoice_id.unwrap())
        .await
        .unwrap();
    assert_eq!(invoice.net_total, 80_000);
    assert_eq!(invoice.vat_total, 0);
    assert_eq!(invoice.gross_total, 80_000);
    assert_eq!(invoice.status, InvoiceStatus::Issued);
}

/// Zero-rating only for a job marked international with its evidence
/// attached; otherwise the main service's 5%.
#[renox::test]
async fn zero_rating_needs_an_international_job_with_its_evidence() {
    let w = boot_with_invoicing().await;
    let client = w.client_party("Najm Client LLC").await;
    *w.job_client.lock().unwrap() = client;
    let job = w.job_closed().await;
    // Marked international, evidence not attached: still 5%.
    let yes = true;
    FreightJob::query()
        .where_(FreightJob::ID.eq(job.id))
        .update(w.db(), &[("international", &yes)])
        .await
        .unwrap();
    let marked = w.job(job.id).await;
    let line = JobLine {
        disbursement: false,
        ..JobLine::default()
    };
    assert_eq!(
        freight_reopen::vat_category(&marked, &line),
        wl_kernel::invoicing::VatCategory::Standard
    );
    w.post(
        &w.docs,
        &format!("/freight/{}/zero-rate-evidence", job.id),
        &[("attached", "1")],
    )
    .await
    .assert_status(303);
    let evidenced = w.job(job.id).await;
    assert_eq!(
        freight_reopen::vat_category(&evidenced, &line),
        wl_kernel::invoicing::VatCategory::ZeroRated
    );

    w.job_action(&w.docs, &job, "reopen", &[("reason", "port_storage")])
        .await
        .assert_status(303);
    late_cost(&w, &job, "800.00", "client", true).await;
    w.post(&w.docs, &format!("/freight/{}/supplementary", job.id), &[])
        .await
        .assert_status(303);
    let revenue = freight_costing::lines_of(w.db(), job.id)
        .await
        .unwrap()
        .into_iter()
        .find(|line| line.kind == line_kind::REVENUE && line.late)
        .unwrap();
    let invoice = Invoice::find_or_404(w.db(), revenue.invoice_id.unwrap())
        .await
        .unwrap();
    assert_eq!(invoice.vat_total, 0);
    assert_eq!(invoice.gross_total, 80_000);
}

/// A user who may not issue invoices leaves the draft unissued and the job
/// does not close until the invoice is issued by someone who may; the draft
/// is not made twice.
#[renox::test]
async fn an_invoice_that_is_not_issued_keeps_the_job_from_closing() {
    let w = boot_with_invoicing().await;
    let client = w.client_party("Najm Client LLC").await;
    *w.job_client.lock().unwrap() = client;
    let job = w.job_closed().await;
    w.job_action(&w.docs, &job, "reopen", &[("reason", "demurrage")])
        .await
        .assert_status(303);
    late_cost(&w, &job, "800.00", "client", true).await;

    // `docs_limited` works the job but does not hold `invoices.issue`.
    w.post(&w.docs_limited, &format!("/freight/{}/supplementary", job.id), &[])
        .await
        .assert_forbidden();
    let waiting = freight_costing::lines_of(w.db(), job.id)
        .await
        .unwrap()
        .into_iter()
        .find(|line| line.kind == line_kind::REVENUE && line.late)
        .expect("the draft stays");
    assert!(!waiting.issued);
    w.job_action(&w.docs, &job, "close", &[])
        .await
        .assert_status(422)
        .assert_see("Issue the invoice");

    // The officer who may issue finishes the same draft.
    w.post(&w.docs, &format!("/freight/{}/supplementary", job.id), &[])
        .await
        .assert_status(303);
    let drafts = Invoice::query().count(w.db()).await.unwrap();
    assert_eq!(drafts, 1);
    w.job_action(&w.docs, &job, "close", &[])
        .await
        .assert_status(303);
    assert_eq!(w.job(job.id).await.status, FreightStatus::Closed);
}
