//! One tax invoice per instalment, on its due date (01 Property §4.1 step 10,
//! §13 question 4, `FIN.BIL.01`, `FIN.INV.01`).
//!
//! The daily run ([`run`], 02:00 Asia/Dubai, also the command
//! [`TASK`]) turns every *scheduled* charge whose due date has come into a
//! draft invoice through the kernel [`Invoicer`](wl_kernel::invoicing::Invoicer):
//! the tenant is billed, the lease is the source, the building is the scope
//! that picks the issuing legal entity (the owner or landlord of record,
//! decision 0003), the due date is also the date of supply (the tax point),
//! and the line carries the charge's VAT category (exempt for a residential
//! lease, standard for a commercial one). The charge is marked *invoiced* with
//! the invoice's id. Whether the draft is issued at once is the setting
//! *drafts issued automatically* of wl-invoicing.
//!
//! Only a lease in force is billed: a lease being prepared, cancelled or ended
//! has no charge to invoice. Without `wl-invoicing` nothing is drafted and the
//! charges wait.

use renox::chrono::{Duration, NaiveDate};
use renox::prelude::*;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, InvoiceId, invoicer};

use crate::building::BUILDING_SUBJECT_TYPE;
use crate::charge::{ChargeStatus, LeaseCharge};
use crate::lease::{Lease, LeaseStatus};

/// The scheduled task and the command.
pub const TASK: &str = "property:invoice-charges";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "02:00";

/// The days before the due date a draft is made (the issue lead time of
/// §13 question 4, 0 by default).
pub const LEAD_DAYS: i64 = 0;

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Invoices drafted.
    pub invoiced: usize,
    /// Charges left for later: no invoicing, or the lease is not in force.
    pub waiting: usize,
    /// Charges that failed; the run moves on.
    pub failed: usize,
}

/// The invoice draft of `charge` of `lease`.
pub fn draft_of(lease: &Lease, charge: &LeaseCharge) -> InvoiceDraft {
    let text = format!("{} · {}", charge.description, lease.number());
    InvoiceDraft::new(lease.tenant(), lease.subject())
        .scope(wl_kernel::Subject::new(BUILDING_SUBJECT_TYPE, lease.building_id))
        .due_date(charge.due_date)
        .date_of_supply(charge.due_date)
        .line(
            DraftLine::new(&text, charge.amount, charge.vat_category)
                .charge_type(&charge.charge_type)
                .origin(charge.subject()),
        )
}

/// Drafts the invoices of the scheduled charges due by `day` (plus
/// [`LEAD_DAYS`]).
pub async fn run(state: &AppState, day: NaiveDate) -> Result<Report> {
    let mut report = Report::default();
    let due = LeaseCharge::query()
        .where_(LeaseCharge::STATUS.eq(ChargeStatus::Scheduled))
        .where_(LeaseCharge::DUE_DATE.lte(day + Duration::days(LEAD_DAYS)))
        .order_by(LeaseCharge::DUE_DATE)
        .order_by(LeaseCharge::ID)
        .get(&state.db)
        .await?;
    for charge in due {
        match invoice_one(state, &charge).await {
            Ok(true) => report.invoiced += 1,
            Ok(false) => report.waiting += 1,
            Err(error) => {
                report.failed += 1;
                eprintln!("{TASK}: charge {}: {error:?}", charge.id);
            }
        }
    }
    Ok(report)
}

/// Drafts the invoice of one charge; `false` when it has to wait.
async fn invoice_one(state: &AppState, charge: &LeaseCharge) -> Result<bool> {
    let Some(lease) = Lease::find(&state.db, charge.lease_id).await? else {
        return Ok(false);
    };
    // A version replaced by an amendment still bills what it kept before the
    // effective date (the charges after it were cancelled).
    if !LeaseStatus::RUNNING.contains(&lease.status) && lease.status != LeaseStatus::Amended {
        return Ok(false);
    }
    Ok(invoice_charge(state, &lease, charge).await?.is_some())
}

/// Drafts the invoice of the scheduled `charge` of `lease` and marks the
/// charge invoiced; `None` when invoicing is off or another run invoiced the
/// charge first. The charge is invoiced once, even when two runs overlap.
pub(crate) async fn invoice_charge(
    state: &AppState,
    lease: &Lease,
    charge: &LeaseCharge,
) -> Result<Option<InvoiceId>> {
    let Some(id) = invoicer(state)
        .draft(&state.db, draft_of(lease, charge))
        .await?
    else {
        return Ok(None);
    };
    let moved = LeaseCharge::query()
        .where_(LeaseCharge::ID.eq(charge.id))
        .where_(LeaseCharge::STATUS.eq(ChargeStatus::Scheduled))
        .update(
            &state.db,
            &[
                ("status", &ChargeStatus::Invoiced),
                ("invoice_id", &Some(id.0)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    if moved == 0 {
        // Another run got there first: drop our draft.
        invoicer(state).discard_draft(&state.db, id).await?;
        return Ok(None);
    }
    Ok(Some(id))
}

/// Registers the daily task and its command.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state, today()).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
    app.command(
        TASK,
        "Draft the invoices of the lease charges that are due (--date YYYY-MM-DD checks another day)",
        |args, state: AppState| async move {
            let on = match args.value("--date") {
                None => today(),
                Some(text) => text.parse().map_err(|_| {
                    Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        format!("--date must be YYYY-MM-DD, got `{text}`"),
                    )
                })?,
            };
            let report = run(&state, on).await?;
            println!(
                "{} invoices drafted, {} waiting, {} failed.",
                report.invoiced, report.waiting, report.failed
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::VatCategory;

    #[test]
    fn the_draft_bills_the_tenant_for_the_lease_with_the_charges_vat() {
        let lease = Lease {
            id: 5,
            root_id: 5,
            building_id: 2,
            tenant_party_id: 7,
            ..Default::default()
        };
        let charge = LeaseCharge {
            id: 11,
            description: "Rent 3/12".into(),
            charge_type: "rent".into(),
            amount: 2_500_000,
            vat_category: VatCategory::Exempt,
            due_date: NaiveDate::from_ymd_opt(2027, 3, 1).unwrap(),
            ..Default::default()
        };
        let draft = draft_of(&lease, &charge);
        assert_eq!(draft.party.0, 7);
        assert_eq!(draft.source.to_string(), "property.lease:5");
        assert_eq!(draft.scope.unwrap().to_string(), "property.building:2");
        assert_eq!(draft.due_date, Some(charge.due_date));
        assert_eq!(draft.date_of_supply, Some(charge.due_date));
        assert_eq!(draft.lines.len(), 1);
        assert_eq!(draft.lines[0].description, "Rent 3/12 · L-000005");
        assert_eq!(draft.lines[0].unit_price, 2_500_000);
        assert_eq!(draft.lines[0].vat_category, VatCategory::Exempt);
        assert_eq!(
            draft.lines[0].origin.as_ref().map(ToString::to_string).as_deref(),
            Some("property.lease_charge:11")
        );
    }
}
