//! BL-LOG-24 (#322), 03 §4.2 row *Container free time*, §7.2 `LOG.RUL.14`,
//! §9 *VAT* and *Disbursements*, `FIN.PAB.01`: a container's free-time end
//! dates alert the documentation officer and the client 2 days before and on
//! the day, then only add a daily line to the job's log; a supplier bill charge
//! is allocated to the job and the container with a liability that is required
//! first, recharged to the client at 5% or as a disbursement `out_of_scope`,
//! or absorbed in the margin when the company is liable.

mod support;

use logistics::freight_costing::{self, JobLine, line_kind};
use logistics::freight_free_time::{self, FREE_TIME_TASK, FREE_TIME_TASK_AT, FreeTimeNotice};
use logistics::notices::{CLIENT_NOTICE, STAFF_NOTICE};
use logistics::{FreightContainer, FreightJob};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::invoice::Invoice;
use wl_invoicing::line::InvoiceLine;
use wl_invoicing::BillLine;
use wl_kernel::invoicing::{BillRequest, DraftLine, VatCategory, supplier_bills};

use support::{World, boot, boot_with_invoicing};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// A world with the real invoicing and a real client party for the jobs.
async fn invoicing_world() -> World {
    let w = boot_with_invoicing().await;
    let client = w.client_party("Najm Client LLC").await;
    *w.job_client.lock().unwrap() = client;
    w
}

/// Adds container MSKU123 to `job` with the free-time `fields`.
async fn add_container(w: &World, job: &FreightJob, fields: &[(&str, &str)]) -> FreightContainer {
    let mut all = vec![("number", "MSKU123"), ("size", "40HC")];
    all.extend_from_slice(fields);
    w.post(&w.docs, &format!("/freight/{}/containers", job.id), &all)
        .await
        .assert_status(303);
    FreightContainer::query()
        .where_(FreightContainer::JOB_ID.eq(job.id))
        .first(w.db())
        .await
        .unwrap()
        .expect("the container is added")
}

/// An in-transit job with MSKU123 free of demurrage until 2026-11-10.
async fn demurrage_job(w: &World) -> (FreightJob, FreightContainer) {
    let job = w.in_transit().await;
    let container = add_container(w, &job, &[("demurrage_end", "2026-11-10")]).await;
    (job, container)
}

async fn notices_to_client(w: &World) -> usize {
    w.notices.of_type(CLIENT_NOTICE).len()
}

async fn log_days(w: &World, job: &FreightJob) -> Vec<NaiveDate> {
    freight_free_time::log_of(w.db(), job.id)
        .await
        .unwrap()
        .into_iter()
        .map(|line| line.log_date)
        .collect()
}

/// AC1: MSKU123 with demurrage free time ending 2026-11-10: the documentation
/// officer and the client are told on 2026-11-08 and on 2026-11-10, once each,
/// and not on the days between; the rule is scheduled at 06:00.
#[renox::test]
async fn the_officer_and_the_client_are_told_two_days_before_and_on_the_end_date() {
    let w = boot().await;
    let (job, container) = demurrage_job(&w).await;
    let staff_before = w.notices.of_type(STAFF_NOTICE).len();
    let client_before = notices_to_client(&w).await;

    let early = freight_free_time::run_free_time_alerts(w.state(), day(2026, 11, 7))
        .await
        .unwrap();
    assert_eq!((early.notices, early.lines), (0, 0));
    assert_eq!(notices_to_client(&w).await, client_before);

    let two_before = freight_free_time::run_free_time_alerts(w.state(), day(2026, 11, 8))
        .await
        .unwrap();
    assert_eq!(two_before.notices, 1);
    assert_eq!(notices_to_client(&w).await, client_before + 1);
    assert!(w.notices.users_of_type(STAFF_NOTICE).contains(&w.docs.id));
    let staff_after = w.notices.of_type(STAFF_NOTICE).len();
    assert!(staff_after > staff_before, "the officers are told");
    let told = w.notices.of_type(CLIENT_NOTICE);
    assert!(told.last().unwrap().data.to_string().contains("MSKU123"));
    assert!(told.last().unwrap().data.to_string().contains("2026-11-10"));

    // Once: the same day again sends nothing, nor does the day between.
    freight_free_time::run_free_time_alerts(w.state(), day(2026, 11, 8))
        .await
        .unwrap();
    freight_free_time::run_free_time_alerts(w.state(), day(2026, 11, 9))
        .await
        .unwrap();
    assert_eq!(notices_to_client(&w).await, client_before + 1);

    let on_the_day = freight_free_time::run_free_time_alerts(w.state(), day(2026, 11, 10))
        .await
        .unwrap();
    assert_eq!(on_the_day.notices, 1);
    assert_eq!(on_the_day.lines, 0, "the end date itself is still free");
    assert_eq!(notices_to_client(&w).await, client_before + 2);
    let sent = FreeTimeNotice::query()
        .where_(FreeTimeNotice::CONTAINER_ID.eq(container.id))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(sent.len(), 2);

    // The rule is on the scheduler at 06:00 and runs.
    assert_eq!(FREE_TIME_TASK_AT, "06:00");
    w.app
        .kernel()
        .run_scheduled(FREE_TIME_TASK)
        .await
        .expect("the task is registered and runs");
    assert_eq!(job.id, w.job(job.id).await.id);
}

/// AC2: on 2026-11-11, 12 and 13 a daily line is added and nothing more is
/// sent; running a day twice adds no second line.
#[renox::test]
async fn each_day_past_the_end_date_adds_a_log_line_and_sends_nothing() {
    let w = boot().await;
    let (job, _container) = demurrage_job(&w).await;
    for date in [day(2026, 11, 8), day(2026, 11, 10)] {
        freight_free_time::run_free_time_alerts(w.state(), date)
            .await
            .unwrap();
    }
    let told = notices_to_client(&w).await;
    let staff = w.notices.of_type(STAFF_NOTICE).len();
    for date in [day(2026, 11, 11), day(2026, 11, 12), day(2026, 11, 12), day(2026, 11, 13)] {
        freight_free_time::run_free_time_alerts(w.state(), date)
            .await
            .unwrap();
    }
    assert_eq!(
        log_days(&w, &job).await,
        [day(2026, 11, 11), day(2026, 11, 12), day(2026, 11, 13)]
    );
    assert_eq!(notices_to_client(&w).await, told);
    assert_eq!(w.notices.of_type(STAFF_NOTICE).len(), staff);
}

/// The charge of a supplier bill for container MSKU123, `days` days of
/// demurrage at AED 350 a day, recorded as a bill from a shipping line for the
/// job; the id of its line.
async fn demurrage_bill(w: &World, job: &FreightJob, number: &str, days: i64) -> i64 {
    let line_company = w.client_party("Maersk Line").await;
    let request = BillRequest::new(line_company, number, day(2026, 11, 14), job.subject()).line(
        DraftLine::new("Demurrage MSKU123", 35_000 * days, VatCategory::Standard),
    );
    let made = supplier_bills(w.state())
        .create_submitted(w.state(), request)
        .await
        .unwrap()
        .expect("invoicing is on");
    BillLine::query()
        .where_(BillLine::BILL_ID.eq(made.0))
        .first(w.db())
        .await
        .unwrap()
        .expect("the bill line")
        .id
}

/// Allocates the bill line to the job with the fields.
async fn allocate_bill(w: &World, job: &FreightJob, bill_line: i64, rest: &[(&str, &str)]) {
    let line = bill_line.to_string();
    let mut fields = vec![
        ("bill_line_id", line.as_str()),
        ("container", "MSKU123"),
        ("charge_type", "demurrage"),
        ("billed_days", "3"),
    ];
    fields.extend_from_slice(rest);
    w.post(&w.docs, &format!("/freight/{}/costs/bill", job.id), &fields)
        .await
        .assert_status(303);
}

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

/// The invoice lines of the latest invoice.
async fn invoice_lines(w: &World) -> (Invoice, Vec<InvoiceLine>) {
    let invoice = Invoice::query()
        .order_by_desc(Invoice::ID)
        .first(w.db())
        .await
        .unwrap()
        .expect("an invoice");
    let lines = InvoiceLine::query()
        .where_(InvoiceLine::INVOICE_ID.eq(invoice.id))
        .order_by(InvoiceLine::POSITION)
        .get(w.db())
        .await
        .unwrap();
    (invoice, lines)
}

/// AC3: the line bills 3 days (AED 1,050) with liability *client* and the
/// conditions not met, on a job whose main service is at 5%: the invoice line
/// is AED 1,050 + AED 52.50 VAT; the bill shows the log's 3 days next to the
/// 3 billed.
#[renox::test]
async fn a_client_liable_charge_is_recharged_at_five_percent_and_matches_the_log() {
    let w = invoicing_world().await;
    let (job, container) = demurrage_job(&w).await;
    for date in [day(2026, 11, 11), day(2026, 11, 12), day(2026, 11, 13)] {
        freight_free_time::run_free_time_alerts(w.state(), date)
            .await
            .unwrap();
    }
    let bill_line = demurrage_bill(&w, &job, "MSC-77001", 3).await;
    allocate_bill(
        &w,
        &job,
        bill_line,
        &[
            ("liability", "client"),
            ("reason", "late_collection_by_client"),
            ("rechargeable", "1"),
        ],
    )
    .await;
    let cost = last_cost(&w, &job).await;
    assert_eq!(cost.amount, 105_000);
    assert_eq!(cost.liability.as_deref(), Some("client"));
    assert_eq!(cost.liability_reason.as_deref(), Some("late_collection_by_client"));
    assert_eq!(cost.container_id, Some(container.id));
    assert_eq!(cost.charge_type.as_deref(), Some("demurrage"));
    assert!(cost.allocated && cost.rechargeable);
    assert_eq!(
        freight_free_time::days_compared(w.db(), &cost).await.unwrap(),
        Some((3, 3))
    );
    w.get(&w.docs, &format!("/freight/{}", job.id))
        .await
        .assert_ok()
        .assert_see("3 / 3");

    w.post(&w.docs, &format!("/freight/{}/recharge", job.id), &[])
        .await
        .assert_status(303);
    let (invoice, lines) = invoice_lines(&w).await;
    assert_eq!(invoice.net_total, 105_000);
    assert_eq!(invoice.vat_total, 5_250);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].vat_category, VatCategory::Standard);
    assert_eq!(lines[0].charge_type.as_deref(), Some("reimbursement"));
    assert_eq!(last_cost(&w, &job).await.invoice_id, Some(invoice.id));
    // Recharged once.
    w.post(&w.docs, &format!("/freight/{}/recharge", job.id), &[])
        .await
        .assert_status(422);
}

/// AC3 (the other half): with the three conditions of VATP013 recorded the
/// AED 1,050 is a disbursement, `out_of_scope`, with no VAT.
#[renox::test]
async fn a_disbursement_with_the_conditions_is_billed_out_of_scope() {
    let w = invoicing_world().await;
    let (job, _container) = demurrage_job(&w).await;
    let bill_line = demurrage_bill(&w, &job, "MSC-77002", 3).await;
    allocate_bill(
        &w,
        &job,
        bill_line,
        &[
            ("liability", "client"),
            ("reason", "late_documents"),
            ("rechargeable", "1"),
        ],
    )
    .await;
    let cost = last_cost(&w, &job).await;
    w.post(
        &w.docs,
        &format!("/freight/lines/{}/disbursement", cost.id),
        &[("conditions", "1")],
    )
    .await
    .assert_status(303);
    w.post(&w.docs, &format!("/freight/{}/recharge", job.id), &[])
        .await
        .assert_status(303);
    let (invoice, lines) = invoice_lines(&w).await;
    assert_eq!(invoice.net_total, 105_000);
    assert_eq!(invoice.vat_total, 0);
    assert_eq!(lines[0].vat_category, VatCategory::OutOfScope);
    assert_eq!(lines[0].charge_type.as_deref(), Some("disbursement"));
}

/// AC4: with liability *company* nothing is recharged and the margin drops by
/// AED 1,050.
#[renox::test]
async fn a_company_liable_charge_is_not_recharged_and_lowers_the_margin() {
    let w = invoicing_world().await;
    let job = w.job_delivered().await;
    add_container(&w, &job, &[("demurrage_end", "2026-11-10")]).await;
    w.job_revenue(&job, "10000.00").await;
    let bill_line = demurrage_bill(&w, &job, "MSC-77003", 3).await;
    // Even asked to be rechargeable, a company liability is not.
    allocate_bill(
        &w,
        &job,
        bill_line,
        &[
            ("liability", "company"),
            ("reason", "company_delay"),
            ("rechargeable", "1"),
        ],
    )
    .await;
    let cost = last_cost(&w, &job).await;
    assert_eq!(cost.liability.as_deref(), Some("company"));
    assert!(!cost.rechargeable);
    w.post(&w.docs, &format!("/freight/{}/recharge", job.id), &[])
        .await
        .assert_status(422);
    let figures = freight_costing::figures_of(w.db(), job.id).await.unwrap();
    assert_eq!(figures.margin, 1_000_000 - 105_000);
    w.job_action(&w.docs, &job, "close", &[])
        .await
        .assert_status(303);
    let margins = freight_costing::margins_of(w.db(), job.id).await.unwrap();
    assert_eq!(margins[0].margin, 895_000);
    assert_eq!(Invoice::query().count(w.db()).await.unwrap(), 0);
}

/// The liability, with a reason that fits it, is required before the bill is
/// approved for the job; the bill line must exist and is allocated once.
#[renox::test]
async fn the_liability_and_its_reason_are_required_before_the_bill_is_allocated() {
    let w = invoicing_world().await;
    let (job, _container) = demurrage_job(&w).await;
    let bill_line = demurrage_bill(&w, &job, "MSC-77004", 3).await;
    let line = bill_line.to_string();
    let base = [
        ("bill_line_id", line.as_str()),
        ("container", "MSKU123"),
        ("charge_type", "demurrage"),
        ("billed_days", "3"),
    ];
    let path = format!("/freight/{}/costs/bill", job.id);
    let with = |extra: &[(&'static str, &'static str)]| {
        let mut all: Vec<(&str, &str)> = base.to_vec();
        all.extend_from_slice(extra);
        all
    };
    // No liability: refused, nothing allocated.
    w.post(&w.docs, &path, &with(&[("reason", "late_documents")]))
        .await
        .assert_status(422)
        .assert_see("liability");
    // A reason of the other liability.
    w.post(
        &w.docs,
        &path,
        &with(&[("liability", "client"), ("reason", "company_delay")]),
    )
    .await
    .assert_status(422);
    w.post(
        &w.docs,
        &path,
        &with(&[("liability", "company"), ("reason", "late_documents")]),
    )
    .await
    .assert_status(422);
    // Not the officer's permission.
    w.post(
        &w.hub,
        &path,
        &with(&[("liability", "company"), ("reason", "other")]),
    )
    .await
    .assert_forbidden();
    // A container the job does not have, a bill line there is not.
    let mut other = with(&[("liability", "company"), ("reason", "other")]);
    other[1] = ("container", "TCLU999");
    w.post(&w.docs, &path, &other).await.assert_status(422);
    let mut missing = with(&[("liability", "company"), ("reason", "other")]);
    missing[0] = ("bill_line_id", "99999");
    w.post(&w.docs, &path, &missing).await.assert_status(422);
    assert_eq!(
        JobLine::query()
            .where_(JobLine::JOB_ID.eq(job.id))
            .count(w.db())
            .await
            .unwrap(),
        0
    );
    // Allocated once.
    allocate_bill(&w, &job, bill_line, &[("liability", "company"), ("reason", "other")]).await;
    w.post(
        &w.docs,
        &path,
        &with(&[("liability", "company"), ("reason", "other")]),
    )
    .await
    .assert_status(422);
}

/// A bill that charges other days than the log is shown, not blocked.
#[renox::test]
async fn a_bill_that_differs_from_the_log_is_shown_and_not_blocked() {
    let w = invoicing_world().await;
    let (job, _container) = demurrage_job(&w).await;
    freight_free_time::run_free_time_alerts(w.state(), day(2026, 11, 11))
        .await
        .unwrap();
    let bill_line = demurrage_bill(&w, &job, "MSC-77005", 3).await;
    allocate_bill(&w, &job, bill_line, &[("liability", "company"), ("reason", "other")]).await;
    let cost = last_cost(&w, &job).await;
    assert_eq!(
        freight_free_time::days_compared(w.db(), &cost).await.unwrap(),
        Some((1, 3))
    );
    w.get(&w.docs, &format!("/freight/{}", job.id))
        .await
        .assert_ok()
        .assert_see("1 / 3");
}

/// AC5: "collected 2026-11-13" recorded at 15:00 on 2026-11-13 keeps the
/// 13 November line (written at 06:00) and no line is written on 2026-11-14:
/// the log has 3 lines, matching the 3 billed days.
#[renox::test]
async fn recording_the_collection_keeps_that_days_line_and_writes_none_after() {
    let w = boot().await;
    let (job, container) = demurrage_job(&w).await;
    for date in [day(2026, 11, 11), day(2026, 11, 12), day(2026, 11, 13)] {
        freight_free_time::run_free_time_alerts(w.state(), date)
            .await
            .unwrap();
    }
    // Only a holder of `jobs.manage` records it.
    w.post(
        &w.hub,
        &format!("/freight/containers/{}/collected", container.id),
        &[("date", "2026-11-13")],
    )
    .await
    .assert_forbidden();
    w.post(
        &w.docs,
        &format!("/freight/containers/{}/collected", container.id),
        &[("date", "next week")],
    )
    .await
    .assert_status(422);
    w.post(
        &w.docs,
        &format!("/freight/containers/{}/collected", container.id),
        &[("date", "2026-11-13")],
    )
    .await
    .assert_status(303);
    freight_free_time::run_free_time_alerts(w.state(), day(2026, 11, 13))
        .await
        .unwrap();
    freight_free_time::run_free_time_alerts(w.state(), day(2026, 11, 14))
        .await
        .unwrap();
    assert_eq!(
        log_days(&w, &job).await,
        [day(2026, 11, 11), day(2026, 11, 12), day(2026, 11, 13)]
    );
    assert_eq!(
        freight_free_time::logged_days(w.db(), container.id, "demurrage")
            .await
            .unwrap(),
        3
    );
    let stored = FreightContainer::find(w.db(), container.id).await.unwrap().unwrap();
    assert_eq!(stored.collected_on, Some(day(2026, 11, 13)));
}

/// A collection recorded for a day before the run writes no line for it, and
/// returning the container empty stops detention but not demurrage.
#[renox::test]
async fn the_log_stops_by_charge_type_and_not_while_the_container_is_still_out() {
    let w = boot().await;
    let job = w.in_transit().await;
    let container = add_container(
        &w,
        &job,
        &[("demurrage_end", "2026-11-10"), ("detention_end", "2026-11-10")],
    )
    .await;
    freight_free_time::run_free_time_alerts(w.state(), day(2026, 11, 11))
        .await
        .unwrap();
    assert_eq!(log_days(&w, &job).await.len(), 2, "demurrage and detention");
    w.post(
        &w.docs,
        &format!("/freight/containers/{}/returned-empty", container.id),
        &[("date", "2026-11-11")],
    )
    .await
    .assert_status(303);
    freight_free_time::run_free_time_alerts(w.state(), day(2026, 11, 12))
        .await
        .unwrap();
    let lines = freight_free_time::log_of(w.db(), job.id).await.unwrap();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines.last().unwrap().charge_type, "demurrage");
    assert_eq!(lines.last().unwrap().log_date, day(2026, 11, 12));
    // Collected on the 11th: a run on the 12th writes nothing more.
    w.post(
        &w.docs,
        &format!("/freight/containers/{}/collected", container.id),
        &[("date", "2026-11-11")],
    )
    .await
    .assert_status(303);
    freight_free_time::run_free_time_alerts(w.state(), day(2026, 11, 13))
        .await
        .unwrap();
    assert_eq!(freight_free_time::log_of(w.db(), job.id).await.unwrap().len(), 3);
}

/// AC6: the end date extended on 2026-11-09 from 2026-11-10 to 2026-11-15 (the
/// 2026-11-08 notice already sent): notices go out on 2026-11-13 and
/// 2026-11-15, none on 2026-11-10, and the first log line is for 2026-11-16.
#[renox::test]
async fn an_extended_end_date_has_its_own_notices_and_log() {
    let w = boot().await;
    let (job, container) = demurrage_job(&w).await;
    freight_free_time::run_free_time_alerts(w.state(), day(2026, 11, 8))
        .await
        .unwrap();
    let after_first = notices_to_client(&w).await;

    // The extension needs the officer's permission and is audited.
    w.post(
        &w.hub,
        &format!("/freight/containers/{}/dates", container.id),
        &[("demurrage_end", "2026-11-15")],
    )
    .await
    .assert_forbidden();
    w.post(
        &w.docs,
        &format!("/freight/containers/{}/dates", container.id),
        &[("demurrage_end", "2026-11-15")],
    )
    .await
    .assert_status(303);
    let events = job.events(w.db()).await.unwrap();
    let changed = events
        .iter()
        .find(|event| event.kind == "free_time_changed")
        .expect("the change is audited");
    assert!(changed.note.contains("2026-11-10 -> 2026-11-15"));
    assert_eq!(changed.actor_user_id, Some(w.docs.id));

    for (date, expected) in [
        (day(2026, 11, 10), 0),
        (day(2026, 11, 11), 0),
        (day(2026, 11, 12), 0),
        (day(2026, 11, 13), 1),
        (day(2026, 11, 14), 0),
        (day(2026, 11, 15), 1),
    ] {
        let before = notices_to_client(&w).await;
        freight_free_time::run_free_time_alerts(w.state(), date)
            .await
            .unwrap();
        assert_eq!(notices_to_client(&w).await - before, expected, "{date}");
    }
    assert_eq!(notices_to_client(&w).await, after_first + 2);
    assert!(log_days(&w, &job).await.is_empty(), "nothing before the new date");
    freight_free_time::run_free_time_alerts(w.state(), day(2026, 11, 16))
        .await
        .unwrap();
    assert_eq!(log_days(&w, &job).await, [day(2026, 11, 16)]);
    let new_date = FreeTimeNotice::query()
        .where_(FreeTimeNotice::END_DATE.eq(day(2026, 11, 15)))
        .count(w.db())
        .await
        .unwrap();
    assert_eq!(new_date, 2);
}

/// An extension made before the first notice drops the old date's unsent
/// notice, and log lines already written stay with a note.
#[renox::test]
async fn an_extension_drops_the_unsent_notice_and_notes_the_lines_written() {
    let w = boot().await;
    let (job, container) = demurrage_job(&w).await;
    w.post(
        &w.docs,
        &format!("/freight/containers/{}/dates", container.id),
        &[("demurrage_end", "2026-11-20")],
    )
    .await
    .assert_status(303);
    let before = notices_to_client(&w).await;
    freight_free_time::run_free_time_alerts(w.state(), day(2026, 11, 8))
        .await
        .unwrap();
    freight_free_time::run_free_time_alerts(w.state(), day(2026, 11, 10))
        .await
        .unwrap();
    assert_eq!(notices_to_client(&w).await, before, "the old date sends nothing");

    // Lines written for the old date stay, with a note, when it is extended.
    let other = w.in_transit().await;
    let kept = add_container(&w, &other, &[("demurrage_end", "2026-11-10")]).await;
    freight_free_time::run_free_time_alerts(w.state(), day(2026, 11, 11))
        .await
        .unwrap();
    w.post(
        &w.docs,
        &format!("/freight/containers/{}/dates", kept.id),
        &[("demurrage_end", "2026-11-15")],
    )
    .await
    .assert_status(303);
    let lines = freight_free_time::log_of(w.db(), other.id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].note.contains("2026-11-15"));
    assert_eq!(job.id, w.job(job.id).await.id);
}

/// The rule covers a job in any state but `closed` and `cancelled`: detention
/// after `delivered` still alerts; a cancelled job does not.
#[renox::test]
async fn a_delivered_job_is_still_watched_and_a_cancelled_one_is_not() {
    let w = boot().await;
    let delivered = w.job_delivered().await;
    add_container(&w, &delivered, &[("detention_end", "2026-11-10")]).await;
    let booked = w.open_job().await;
    add_container(&w, &booked, &[("detention_end", "2026-11-10")]).await;
    w.job_action(&w.docs, &booked, "cancel", &[("reason", "client cancelled order")])
        .await
        .assert_status(303);

    let run = freight_free_time::run_free_time_alerts(w.state(), day(2026, 11, 10))
        .await
        .unwrap();
    assert_eq!(run.notices, 1);
    let next = freight_free_time::run_free_time_alerts(w.state(), day(2026, 11, 11))
        .await
        .unwrap();
    assert_eq!(next.lines, 1);
    assert_eq!(log_days(&w, &delivered).await, [day(2026, 11, 11)]);
    assert!(log_days(&w, &booked).await.is_empty());
}
