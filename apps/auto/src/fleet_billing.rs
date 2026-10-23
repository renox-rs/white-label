//! The job cards of a fleet and the consolidated monthly invoice (08 §4.5
//! steps 3 and 4, `FIN.USG.01`, `FIN.BIL.01`, `FIN.INV.01`, `AUT.RUL.05`).
//!
//! A job card opened for a vehicle under a contract in force is a **fleet
//! job** ([`attach_job`]): it carries the fleet's PO reference and is not
//! billed on delivery ([`crate::billing::bill`] answers `None`). Once a month
//! the contract is billed in one invoice with a line per approved line of
//! every delivered job, each naming its vehicle, job card and PO reference
//! ([`invoice_period`]). The periods billed are kept ([`FleetInvoice`]) and the
//! contract knows the day it was invoiced through, so a period is never billed
//! twice, none is skipped, and a termination cannot take effect inside an
//! invoiced period.
//!
//! Decisions:
//!
//! - The usage recorder of the kernel (`UsageRecorder`) prices timesheet
//!   hours (09); the fleet's job card lines are priced already, so the
//!   invoice is drafted through the kernel's `Invoicer` (see `DEFERRED.md`).
//! - The invoice is **drafted** by the daily run (`AUT.RUL.05`: "draft the
//!   consolidated fleet invoice"); a user holding `invoices.issue` who bills a
//!   period issues it too.
//! - A period with nothing to bill is recorded without an invoice.
//! - A job delivered after the end of a period that is already invoiced goes
//!   into the next one: the jobs billed are those delivered up to the period's
//!   last day and not billed yet.

use renox::chrono::{Datelike, NaiveDate};
use renox::db::now;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::invoicing::{InvoiceDraft, InvoiceId, invoicer};

use crate::billing::{labour_draft_line, part_draft_line};
use crate::db::refused;
use crate::estimate::Estimate;
use crate::fleet_contract::{FleetContract, FleetStatus};
use crate::fleet_invoice::{FleetInvoice, KIND_MONTHLY};
use crate::fleet_job::FleetJob;
use crate::job_card::{JobCard, JobCardStatus};
use crate::labour_line::LineStatus;
use crate::permissions;
use crate::texts::english;
use crate::vehicle::Vehicle;

/// Makes a fleet job of `card` when its vehicle is under a contract in force.
/// A job card that is a fleet job already answers its job.
///
/// # Errors
///
/// The database's errors.
pub async fn attach_job(state: &AppState, card: &JobCard) -> Result<Option<FleetJob>> {
    if let Some(found) = FleetJob::of_job_card(&state.db, card.id).await? {
        return Ok(Some(found));
    }
    let Some(vehicle) = Vehicle::find(&state.db, card.vehicle_id).await? else {
        return Ok(None);
    };
    let Some(contract_id) = vehicle.fleet_contract_id else {
        return Ok(None);
    };
    let in_force = FleetContract::find(&state.db, contract_id)
        .await?
        .is_some_and(|contract| contract.status.is_in_force());
    if !in_force {
        return Ok(None);
    }
    let mut job = FleetJob {
        contract_id,
        job_card_id: card.id,
        vehicle_id: vehicle.id,
        ..FleetJob::default()
    };
    job.save(&state.db).await?;
    Ok(Some(job))
}

/// Whether job card `job_card_id` is billed in a contract's monthly invoice
/// and not on delivery.
///
/// # Errors
///
/// The database's errors.
pub async fn bills_monthly(db: &Db, job_card_id: i64) -> Result<bool> {
    Ok(FleetJob::of_job_card(db, job_card_id).await?.is_some())
}

/// Records the fleet's PO reference of the job of job card `job_card_id`.
///
/// # Errors
///
/// A 403 without `fleet_contracts.manage` or `job_cards.advise`; a 404 for a
/// job card that is not under a fleet contract; a 422 for a job already
/// billed.
pub async fn set_po(
    state: &AppState,
    by: &AuthUser,
    job_card_id: i64,
    po_reference: &str,
) -> Result<FleetJob> {
    if !(by.has_permission(permissions::FLEET_CONTRACTS_MANAGE)
        || by.has_permission(permissions::JOB_CARDS_ADVISE))
    {
        return Err(Error::Forbidden);
    }
    let mut job = FleetJob::of_job_card(&state.db, job_card_id)
        .await?
        .ok_or(Error::NotFound)?;
    if job.invoice_id.is_some() {
        return Err(refused(english("auto.fleet.problem.job_billed", &[])));
    }
    let po = po_reference.trim();
    job.po_reference = (!po.is_empty()).then(|| po.to_owned());
    job.save(&state.db).await?;
    FleetJob::find_or_404(&state.db, job.id).await
}

/// The first and last day of the month before `day`.
pub fn previous_month(day: NaiveDate) -> (NaiveDate, NaiveDate) {
    let first_of_this = day.with_day(1).unwrap_or(day);
    let last = first_of_this.pred_opt().unwrap_or(first_of_this);
    (last.with_day(1).unwrap_or(last), last)
}

/// An invoice line of a fleet job: `Dubai A 12345 · JOB-00007 · PO 4471 ·
/// Brake pad set (new)`.
pub fn line_text(plate: &str, number: &str, po: Option<&str>, line: &str) -> String {
    match po {
        Some(po) if !po.trim().is_empty() => {
            format!("{plate} · {number} · PO {} · {line}", po.trim())
        }
        _ => format!("{plate} · {number} · {line}"),
    }
}

/// The delivered jobs of `contract_id` not billed yet whose job card was
/// delivered up to `to`, oldest first, with their job cards.
///
/// # Errors
///
/// The database's errors.
pub async fn unbilled_jobs(
    db: &Db,
    contract_id: i64,
    to: NaiveDate,
) -> Result<Vec<(FleetJob, JobCard)>> {
    let mut found = Vec::new();
    let mut jobs = FleetJob::of_contract(db, contract_id).await?;
    jobs.sort_by_key(|job| job.id);
    for job in jobs.into_iter().filter(|job| job.invoice_id.is_none()) {
        let Some(card) = JobCard::find(db, job.job_card_id).await? else {
            continue;
        };
        let delivered = card
            .delivered_at
            .map(dubai_date)
            .is_some_and(|day| day <= to);
        if card.status == JobCardStatus::Delivered && delivered {
            found.push((job, card));
        }
    }
    Ok(found)
}

/// Bills the period `from` .. `to` of contract `contract_id` in one invoice
/// (`FIN.USG.01`): a line per approved line of every delivered job not billed
/// yet, naming the vehicle, the job card and the PO reference. `by` is the
/// user who bills it (`None` for the daily run, which drafts only); a user
/// holding `invoices.issue` issues it.
///
/// # Errors
///
/// A 403 for a user without `fleet_contracts.manage` or `invoices.issue`; a
/// 422 for a contract that is still a draft, a period that is not over, that
/// overlaps what was invoiced or leaves a gap after it, or when invoicing is
/// off.
pub async fn invoice_period(
    state: &AppState,
    by: Option<&AuthUser>,
    contract_id: i64,
    from: NaiveDate,
    to: NaiveDate,
    kind: &str,
) -> Result<FleetInvoice> {
    if let Some(user) = by
        && !(user.has_permission(permissions::FLEET_CONTRACTS_MANAGE)
            || user.has_permission(permissions::INVOICES_ISSUE))
    {
        return Err(Error::Forbidden);
    }
    let contract = FleetContract::find_or_404(&state.db, contract_id).await?;
    if contract.status == FleetStatus::Draft {
        return Err(refused(english("auto.fleet.problem.not_in_force", &[])));
    }
    if from > to || to >= today() {
        return Err(refused(english("auto.fleet.problem.period", &[])));
    }
    if let Some(through) = contract.invoiced_through_date {
        if from <= through {
            return Err(refused(english("auto.fleet.problem.period_billed", &[])));
        }
        if through.succ_opt() != Some(from) {
            return Err(refused(english("auto.fleet.problem.period_gap", &[])));
        }
    }
    let jobs = unbilled_jobs(&state.db, contract.id, to).await?;
    let mut draft =
        InvoiceDraft::new(PartyId(contract.fleet_party_id), contract.subject()).date_of_supply(to);
    let mut lines = 0_i64;
    let mut net = 0_i64;
    for (job, card) in &jobs {
        let vehicle = Vehicle::find_or_404(&state.db, card.vehicle_id).await?;
        let estimate = Estimate::load(&state.db, card.id).await?;
        let plate = vehicle.plate();
        let number = card.number();
        let po = job.po_reference.clone();
        for line in estimate
            .labour
            .iter()
            .filter(|l| l.approval == LineStatus::Approved)
        {
            let mut billed = labour_draft_line(line, card.subject());
            billed.description = line_text(&plate, &number, po.as_deref(), &billed.description);
            net += billed.net();
            draft = draft.line(billed);
            lines += 1;
        }
        for line in estimate
            .parts
            .iter()
            .filter(|l| l.approval == LineStatus::Approved)
        {
            let mut billed = part_draft_line(line, card.subject());
            billed.description = line_text(&plate, &number, po.as_deref(), &billed.description);
            net += billed.net();
            draft = draft.line(billed);
            lines += 1;
        }
    }
    let mut invoice_id: Option<i64> = None;
    if lines > 0 {
        let books = invoicer(state);
        let made = books
            .draft(&state.db, draft)
            .await?
            .ok_or_else(|| refused(english("auto.fleet.problem.invoicing_off", &[])))?;
        invoice_id = Some(made.0);
        if let Some(user) = by
            && user.has_permission(permissions::INVOICES_ISSUE)
        {
            let _issued = books.issue(state, InvoiceId(made.0), Some(user)).await;
        }
    }
    let mut record = FleetInvoice {
        contract_id: contract.id,
        period_start_date: from,
        period_end_date: to,
        kind: kind.to_owned(),
        invoice_id,
        jobs: i64::try_from(jobs.len()).unwrap_or(i64::MAX),
        net_total: net,
        created_by: by.map(|user| user.id),
        ..FleetInvoice::default()
    };
    record.save(&state.db).await?;
    if let Some(id) = invoice_id {
        for (job, _) in &jobs {
            FleetJob::query()
                .where_(FleetJob::ID.eq(job.id))
                .update(
                    &state.db,
                    &[
                        ("invoice_id", &Some(id)),
                        ("invoiced_at", &Some(now())),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
        }
    }
    FleetContract::query()
        .where_(FleetContract::ID.eq(contract.id))
        .update(
            &state.db,
            &[("invoiced_through_date", &Some(to)), ("updated_at", &now())],
        )
        .await?;
    FleetInvoice::find_or_404(&state.db, record.id).await
}

/// The monthly run of one contract (`AUT.RUL.05`, D = 1): drafts the invoice of
/// the month before `day`, from the day after the last period invoiced (or the
/// contract's first day). `None` when the contract is not in force, started
/// after that month, or is invoiced through it.
///
/// # Errors
///
/// As for [`invoice_period`].
pub async fn invoice_previous_month(
    state: &AppState,
    contract: &FleetContract,
    day: NaiveDate,
) -> Result<Option<FleetInvoice>> {
    if !contract.status.is_in_force() {
        return Ok(None);
    }
    let (first, last) = previous_month(day);
    let from = match contract.invoiced_through_date {
        Some(through) => through.succ_opt().unwrap_or(first),
        None => first.max(contract.start_date),
    };
    if from > last || contract.start_date > last {
        return Ok(None);
    }
    Ok(Some(
        invoice_period(state, None, contract.id, from, last, KIND_MONTHLY).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(month: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, d).unwrap()
    }

    #[test]
    fn the_month_before_a_day_is_whole() {
        assert_eq!(previous_month(day(11, 1)), (day(10, 1), day(10, 31)));
        assert_eq!(previous_month(day(11, 17)), (day(10, 1), day(10, 31)));
        assert_eq!(
            previous_month(day(1, 1)),
            (
                NaiveDate::from_ymd_opt(2025, 12, 1).unwrap(),
                NaiveDate::from_ymd_opt(2025, 12, 31).unwrap()
            )
        );
        assert_eq!(previous_month(day(3, 1)), (day(2, 1), day(2, 28)));
    }

    #[test]
    fn an_invoice_line_names_the_vehicle_the_job_and_the_po() {
        assert_eq!(
            line_text(
                "Dubai A 12345",
                "JOB-00007",
                Some(" 4471 "),
                "Brake pad set (new)"
            ),
            "Dubai A 12345 · JOB-00007 · PO 4471 · Brake pad set (new)"
        );
        assert_eq!(
            line_text("Dubai A 12345", "JOB-00007", None, "Oil change"),
            "Dubai A 12345 · JOB-00007 · Oil change"
        );
        assert_eq!(
            line_text("Dubai A 12345", "JOB-00007", Some("  "), "Oil change"),
            "Dubai A 12345 · JOB-00007 · Oil change"
        );
    }
}
