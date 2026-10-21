//! BL-LOG-23 (#321), 03 §4.2 row *Client cancels before departure*, §7.1
//! *Freight job* row `cancel`, §9 *VAT*, `FIN.QUO.01` (the quotation is a
//! seam: the terms are entered on the job, or a provider answers them):
//! cancelling a job before departure drafts one invoice with the cancellation
//! charge and the rechargeable incurred costs, or no charge and a task for the
//! documentation officer when the quotation has no cancellation terms; a cost
//! billed after the cancel is decided by the accountant.

mod support;

use logistics::freight_cancel::{self, FreightCancellation};
use logistics::freight_costing::{self, JobLine, line_kind};
use logistics::notices::STAFF_NOTICE;
use logistics::permissions as perm;
use logistics::{FreightJob, FreightStatus};
use renox::prelude::*;
use wl_invoicing::invoice::Invoice;
use wl_invoicing::line::InvoiceLine;
use wl_kernel::invoicing::{InvoiceStatus, VatCategory};
use wl_kernel::quotations::QuotationTerms;

use support::{World, boot, boot_with_invoicing};

/// A world with the real invoicing and a real client party for the jobs.
async fn invoicing_world() -> World {
    let w = boot_with_invoicing().await;
    let client = w.client_party("Najm Client LLC").await;
    *w.job_client.lock().unwrap() = client;
    w
}

/// The latest cost line of the job.
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

/// A cost of `amount` dirhams recorded on the job and allocated with
/// `liability`, rechargeable when `recharge`.
async fn cost(w: &World, job: &FreightJob, amount: &str, liability: &str, recharge: bool) -> JobLine {
    w.post(
        &w.docs,
        &format!("/freight/{}/costs", job.id),
        &[
            ("description", "Booking fee"),
            ("amount", amount),
            ("source_ref", "SB-3001"),
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

/// The documentation officer enters the terms of the quotation of the job.
async fn quote(w: &World, job: &FreightJob, fields: &[(&str, &str)]) {
    w.post(&w.docs, &format!("/freight/{}/quotation", job.id), fields)
        .await
        .assert_status(303);
}

/// The invoices of the real invoicing, oldest first.
async fn invoices(w: &World) -> Vec<Invoice> {
    Invoice::query()
        .order_by(Invoice::ID)
        .get(w.db())
        .await
        .unwrap()
}

/// A `documents_pending` job.
async fn pending_job(w: &World) -> FreightJob {
    let job = w.open_job().await;
    w.job_action(&w.docs, &job, "request-documents", &[("missing", "Packing list")])
        .await
        .assert_status(303);
    let after = w.job(job.id).await;
    assert_eq!(after.status, FreightStatus::DocumentsPending);
    after
}

/// The cancellation record of the job.
async fn cancellation(w: &World, job: &FreightJob) -> FreightCancellation {
    freight_cancel::cancellation_of(w.db(), job.id)
        .await
        .unwrap()
        .expect("the cancellation is recorded")
}

/// AC1: a `documents_pending` job whose quotation has a cancellation charge of
/// AED 500 and an allocated booking fee of AED 200 (rechargeable): `cancel`
/// with the reason makes it `cancelled` and drafts one invoice of AED 500 +
/// AED 200 + AED 35 VAT (5% on both) = AED 735.
#[renox::test]
async fn cancel_drafts_one_invoice_with_the_charge_and_the_booking_fee_at_five_percent() {
    let w = invoicing_world().await;
    let job = pending_job(&w).await;
    quote(
        &w,
        &job,
        &[
            ("reference", "QUO-2026-044"),
            ("fixed_charge", "500.00"),
            ("recharge_incurred", "1"),
        ],
    )
    .await;
    cost(&w, &job, "200.00", "client", true).await;

    // A reason is needed; only a holder of `jobs.manage` cancels.
    w.job_action(&w.docs, &job, "cancel", &[("reason", " ")])
        .await
        .assert_status(422);
    w.job_action(&w.hub, &job, "cancel", &[("reason", "client cancelled order")])
        .await
        .assert_forbidden();
    assert!(invoices(&w).await.is_empty());

    w.job_action(&w.docs, &job, "cancel", &[("reason", "client cancelled order")])
        .await
        .assert_status(303);
    let cancelled = w.job(job.id).await;
    assert_eq!(cancelled.status, FreightStatus::Cancelled);
    assert_eq!(cancelled.cancel_reason.as_deref(), Some("client cancelled order"));

    let drafted = invoices(&w).await;
    assert_eq!(drafted.len(), 1, "one invoice");
    let invoice = &drafted[0];
    assert_eq!(invoice.status, InvoiceStatus::Draft);
    assert_eq!(invoice.net_total, 70_000);
    assert_eq!(invoice.vat_total, 3_500);
    assert_eq!(invoice.gross_total, 73_500);
    let lines = InvoiceLine::query()
        .where_(InvoiceLine::INVOICE_ID.eq(invoice.id))
        .order_by(InvoiceLine::POSITION)
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].net_amount, 50_000);
    assert_eq!(lines[0].vat_category, VatCategory::Standard);
    assert_eq!(lines[1].net_amount, 20_000);
    assert_eq!(lines[1].vat_category, VatCategory::Standard);
    assert_eq!(lines[1].charge_type.as_deref(), Some("reimbursement"));

    let record = cancellation(&w, &job).await;
    assert_eq!(record.stage, "documents_pending");
    assert_eq!(record.charge, 50_000);
    assert_eq!(record.recharged, 20_000);
    assert_eq!(record.absorbed, 0);
    assert_eq!(record.invoice_id, Some(invoice.id));
    assert_eq!(record.quotation_ref, "QUO-2026-044");
    // The cost is marked as recharged on the invoice and the revenue is the
    // job's.
    assert_eq!(last_cost(&w, &job).await.invoice_id, Some(invoice.id));
    let figures = freight_costing::figures_of(w.db(), job.id).await.unwrap();
    assert_eq!((figures.revenue, figures.cost), (70_000, 20_000));
    assert!(w.tickets.opened().is_empty(), "nothing for the officer to decide");
}

/// A booking fee with the three conditions of VATP013 recorded is recharged
/// as a disbursement, `out_of_scope`; without them as a reimbursement at 5%.
#[renox::test]
async fn an_incurred_disbursement_is_recharged_out_of_scope_and_a_reimbursement_at_five_percent() {
    let w = invoicing_world().await;
    let job = w.open_job().await;
    quote(&w, &job, &[("fixed_charge", "500.00"), ("recharge_incurred", "1")]).await;
    let fee = cost(&w, &job, "200.00", "client", true).await;
    w.post(
        &w.docs,
        &format!("/freight/lines/{}/disbursement", fee.id),
        &[("conditions", "1")],
    )
    .await
    .assert_status(303);
    cost(&w, &job, "100.00", "client", true).await;
    w.job_action(&w.docs, &job, "cancel", &[("reason", "client cancelled order")])
        .await
        .assert_status(303);
    let invoice = invoices(&w).await.remove(0);
    // 500 + 200 (no VAT) + 100 (VAT 5.00 on the charge and the fee: 25 + 5).
    assert_eq!(invoice.net_total, 80_000);
    assert_eq!(invoice.vat_total, 3_000);
    let lines = InvoiceLine::query()
        .where_(InvoiceLine::INVOICE_ID.eq(invoice.id))
        .order_by(InvoiceLine::POSITION)
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(lines[1].vat_category, VatCategory::OutOfScope);
    assert_eq!(lines[1].charge_type.as_deref(), Some("disbursement"));
    assert_eq!(lines[2].vat_category, VatCategory::Standard);
}

/// AC2: a quotation with per-stage charges (`booked` AED 300,
/// `documents_pending` AED 500) and a `booked` job: `cancel` drafts AED 300
/// + 5% VAT.
#[renox::test]
async fn a_charge_per_stage_uses_the_stage_of_the_job_at_cancel() {
    let w = invoicing_world().await;
    let job = w.open_job().await;
    quote(
        &w,
        &job,
        &[("booked_charge", "300.00"), ("documents_pending_charge", "500.00")],
    )
    .await;
    w.job_action(&w.docs, &job, "cancel", &[("reason", "client cancelled order")])
        .await
        .assert_status(303);
    let invoice = invoices(&w).await.remove(0);
    assert_eq!(invoice.net_total, 30_000);
    assert_eq!(invoice.vat_total, 1_500);
    assert_eq!(invoice.gross_total, 31_500);
    assert_eq!(cancellation(&w, &job).await.stage, "booked");

    // The same quotation at `documents_pending` charges AED 500.
    let other = pending_job(&w).await;
    quote(
        &w,
        &other,
        &[("booked_charge", "300.00"), ("documents_pending_charge", "500.00")],
    )
    .await;
    w.job_action(&w.docs, &other, "cancel", &[("reason", "client cancelled order")])
        .await
        .assert_status(303);
    assert_eq!(invoices(&w).await[1].net_total, 50_000);
}

/// AC3: an `in_transit` job is not cancelled and its page does not offer
/// `cancel`; a booked job's page does.
#[renox::test]
async fn cancel_is_not_offered_after_departure() {
    let w = boot().await;
    let departed = w.in_transit().await;
    w.job_action(&w.docs, &departed, "cancel", &[("reason", "client cancelled order")])
        .await
        .assert_status(422);
    assert_eq!(w.job(departed.id).await.status, FreightStatus::InTransit);
    assert!(cancellation_missing(&w, &departed).await);
    w.get(&w.docs, &format!("/freight/{}", departed.id))
        .await
        .assert_ok()
        .assert_dont_see(&format!("/freight/{}/cancel", departed.id));
    // The quotation terms are not entered after departure either.
    w.post(
        &w.docs,
        &format!("/freight/{}/quotation", departed.id),
        &[("fixed_charge", "500.00")],
    )
    .await
    .assert_status(422);

    let booked = w.open_job().await;
    w.get(&w.docs, &format!("/freight/{}", booked.id))
        .await
        .assert_ok()
        .assert_see(&format!("/freight/{}/cancel", booked.id));
}

async fn cancellation_missing(w: &World, job: &FreightJob) -> bool {
    freight_cancel::cancellation_of(w.db(), job.id)
        .await
        .unwrap()
        .is_none()
}

/// AC4: no cancellation terms: `cancel` drafts no charge and the
/// documentation officer gets a task; so does a quotation whose per-stage
/// charges have no amount for the job's stage.
#[renox::test]
async fn without_cancellation_terms_no_charge_is_drafted_and_the_officer_gets_a_task() {
    let w = boot().await;
    let job = w.open_job().await;
    cost(&w, &job, "200.00", "client", true).await;
    w.job_action(&w.docs, &job, "cancel", &[("reason", "client cancelled order")])
        .await
        .assert_status(303);
    assert_eq!(w.job(job.id).await.status, FreightStatus::Cancelled);
    assert!(w.invoices.drafts().is_empty(), "no charge, no invoice");
    let tasks = w.tickets.opened();
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].title.contains("FRJ-0001"));
    assert!(tasks[0].description.contains("AED 200.00"));
    let record = cancellation(&w, &job).await;
    assert!(!record.terms_found);
    assert_eq!(record.invoice_id, None);
    assert!(record.ticket_id.is_some());
    // The incurred cost is not dropped: it stays the job's cost, its loss.
    assert_eq!(record.absorbed, 20_000);
    let figures = freight_costing::figures_of(w.db(), job.id).await.unwrap();
    assert_eq!(figures.margin, -20_000);

    // Per-stage charges with none for this stage are no terms at all.
    let pending = pending_job(&w).await;
    quote(&w, &pending, &[("booked_charge", "300.00")]).await;
    w.job_action(&w.docs, &pending, "cancel", &[("reason", "client cancelled order")])
        .await
        .assert_status(303);
    assert!(w.invoices.drafts().is_empty());
    assert_eq!(w.tickets.opened().len(), 2);
}

/// A quotation that does not recharge the incurred costs: only the charge is
/// invoiced and the costs are shown as the job's loss.
#[renox::test]
async fn incurred_costs_the_quotation_does_not_recharge_are_the_jobs_loss() {
    let w = boot().await;
    let job = w.open_job().await;
    quote(&w, &job, &[("fixed_charge", "500.00"), ("recharge_incurred", "0")]).await;
    let fee = cost(&w, &job, "200.00", "client", true).await;
    w.job_action(&w.docs, &job, "cancel", &[("reason", "client cancelled order")])
        .await
        .assert_status(303);
    let drafts = w.invoices.drafts();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].lines.len(), 1);
    assert_eq!(drafts[0].lines[0].unit_price, 50_000);
    assert_eq!(drafts[0].lines[0].vat_category, VatCategory::Standard);
    let record = cancellation(&w, &job).await;
    assert_eq!((record.charge, record.recharged, record.absorbed), (50_000, 0, 20_000));
    assert_eq!(last_cost(&w, &job).await.invoice_id, None);
    assert_eq!(fee.id, last_cost(&w, &job).await.id);
}

/// The quotation seam: a provider of the app answers the terms of the job
/// and wins over what was typed.
#[renox::test]
async fn the_quotations_provider_answers_the_cancellation_terms() {
    let w = boot().await;
    let job = w.open_job().await;
    let mut terms = QuotationTerms::new("QUO-9");
    terms.cancellation_fixed = Some(40_000);
    w.quotes.set(job.id, terms);
    w.job_action(&w.docs, &job, "cancel", &[("reason", "client cancelled order")])
        .await
        .assert_status(303);
    let drafts = w.invoices.drafts();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].lines[0].unit_price, 40_000);
    assert_eq!(cancellation(&w, &job).await.quotation_ref, "QUO-9");
}

/// Entering the terms needs `jobs.manage` and sane amounts.
#[renox::test]
async fn the_quotation_terms_need_the_permission_and_a_valid_amount() {
    let w = boot().await;
    let job = w.open_job().await;
    w.post(&w.hub, &format!("/freight/{}/quotation", job.id), &[("fixed_charge", "500.00")])
        .await
        .assert_forbidden();
    w.post(&w.docs, &format!("/freight/{}/quotation", job.id), &[("fixed_charge", "abc")])
        .await
        .assert_status(422);
    w.post(&w.docs, &format!("/freight/{}/quotation", job.id), &[("fixed_charge", "-5.00")])
        .await
        .assert_status(422);
    quote(&w, &job, &[("fixed_charge", "500.00")]).await;
    // Entered again, the terms are replaced, not added.
    quote(&w, &job, &[("fixed_charge", "250.00")]).await;
    let terms = freight_cancel::terms_of(w.state(), &job).await.unwrap().unwrap();
    assert_eq!(terms.cancellation_fixed, Some(25_000));
}

/// A cost billed after `cancel` is allocated to the cancelled job, which
/// stays `cancelled`; the accountant is told and decides whether to invoice
/// it; a cost the company is liable for is never invoiced.
#[renox::test]
async fn a_cost_billed_after_cancel_is_decided_by_the_accountant() {
    let w = boot().await;
    let amina = w
        .user_with("Amina", &[perm::JOBS_VIEW, perm::JOBS_BILL_CANCELLED])
        .await;
    let job = w.open_job().await;
    w.job_action(&w.docs, &job, "cancel", &[("reason", "client cancelled order")])
        .await
        .assert_status(303);
    let before = w.notices.of_type(STAFF_NOTICE).len();

    // The bill arrives after the cancel: allocated to the job, which stays.
    let late = cost(&w, &job, "350.00", "client", true).await;
    assert_eq!(w.job(job.id).await.status, FreightStatus::Cancelled);
    assert!(w.notices.of_type(STAFF_NOTICE).len() > before);
    assert!(w.notices.users_of_type(STAFF_NOTICE).contains(&amina.id));
    assert_eq!(late.decision, None);

    // The officer does not decide; the accountant does.
    w.post(&w.docs, &format!("/freight/lines/{}/decide", late.id), &[("invoice", "1")])
        .await
        .assert_forbidden();
    w.post(&w.hub, &format!("/freight/lines/{}/decide", late.id), &[("invoice", "1")])
        .await
        .assert_forbidden();
    w.post(&amina, &format!("/freight/lines/{}/decide", late.id), &[("invoice", "1")])
        .await
        .assert_status(303);
    let decided = last_cost(&w, &job).await;
    assert_eq!(decided.decision.as_deref(), Some("invoiced"));
    assert!(decided.invoice_id.is_some());
    assert_eq!(w.invoices.drafts().len(), 1);
    assert_eq!(w.invoices.drafts()[0].lines[0].unit_price, 35_000);
    assert_eq!(w.job(job.id).await.status, FreightStatus::Cancelled);
    // Decided once.
    w.post(&amina, &format!("/freight/lines/{}/decide", late.id), &[("invoice", "0")])
        .await
        .assert_status(422);

    // A cost the company is liable for is not invoiced, only absorbed.
    let ours = cost(&w, &job, "90.00", "company", false).await;
    w.post(&amina, &format!("/freight/lines/{}/decide", ours.id), &[("invoice", "1")])
        .await
        .assert_status(422);
    w.post(&amina, &format!("/freight/lines/{}/decide", ours.id), &[("invoice", "0")])
        .await
        .assert_status(303);
    assert_eq!(last_cost(&w, &job).await.decision.as_deref(), Some("absorbed"));
    assert_eq!(w.invoices.drafts().len(), 1);
}

/// A decision is only for a cost of a cancelled job that has its liability.
#[renox::test]
async fn only_an_allocated_cost_of_a_cancelled_job_is_decided() {
    let w = boot().await;
    let amina = w
        .user_with("Amina", &[perm::JOBS_VIEW, perm::JOBS_BILL_CANCELLED])
        .await;
    let open = w.open_job().await;
    let line = cost(&w, &open, "50.00", "client", true).await;
    w.post(&amina, &format!("/freight/lines/{}/decide", line.id), &[("invoice", "1")])
        .await
        .assert_status(422);

    let job = w.open_job().await;
    w.job_action(&w.docs, &job, "cancel", &[("reason", "client cancelled order")])
        .await
        .assert_status(303);
    w.post(
        &w.docs,
        &format!("/freight/{}/costs", job.id),
        &[("description", "Storage"), ("amount", "60.00")],
    )
    .await
    .assert_status(303);
    let unallocated = last_cost(&w, &job).await;
    w.post(
        &amina,
        &format!("/freight/lines/{}/decide", unallocated.id),
        &[("invoice", "1")],
    )
    .await
    .assert_status(422);
}
