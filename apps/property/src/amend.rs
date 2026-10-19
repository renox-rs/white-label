//! Amending an active lease in the middle of its term (`amend`, `PRP.ACL.04`,
//! `FIN.ACL.01`, 01 Property §4.3 row "Mid-term change", §7.1 *Lease*;
//! BL-PRP-08).
//!
//! An amendment is a lease version of kind `amendment` with an effective date
//! ([`crate::leases::create_version`]). `submit` checks the date (within the
//! previous version's term and not before the first day of the current month,
//! so closed VAT periods are not touched) and the legal case flag; the
//! activation of the amendment runs `amend` on the previous version.
//!
//! **In the transaction of the activation** ([`amend_effects`]): the previous
//! version points to the amendment, and its scheduled charges due on or after
//! the effective date are cancelled; the amendment's own charges, scheduled by
//! the activation, take their place.
//!
//! **After the commit** ([`after_amend`]), with the contracts of the kernel,
//! which cannot join the transaction:
//!
//! - **The invoices of the old version for periods from the effective date
//!   on, and the one that straddles it,** are not edited (`FIN.ACL.01`):
//!   a paid invoice is first unallocated (the payment goes back to the
//!   tenant's credit), the invoice is credited in full, and it is reissued.
//!   For a period wholly after the effective date the reissue is the invoice
//!   of the amendment's charge for that period, when it is due; for the
//!   period that straddles the effective date it is one invoice of two lines,
//!   the old terms up to the day before the effective date and the new terms
//!   from it, each daily pro rata (amount × days ÷ days in the period, rounded
//!   half up to a fils). The payments taken off are allocated to the reissued
//!   invoices, what exceeds them stays as the tenant's credit. A draft invoice
//!   is discarded and replaced the same way.
//! - **The cheques** of the old version that are `received` or
//!   `due_for_deposit` and pay a charge from the effective date on are
//!   relinked to a charge of the amendment with the same amount; the others are
//!   handed back against one receipt, and the accountant gets a task to collect
//!   replacements. A deposited or cleared cheque is never touched.
//! - **The deposit** is carried over: a changed amount asks the deposit to
//!   `adjust` ([`crate::lease_events`]); an increase becomes a top-up charge
//!   on the effective date ([`LeaseTopUps`]), a decrease a part refund through
//!   the Refund flow, which wl-payments asks for.
//! - `LeaseAmended` is emitted. The Ejari task (`PRP.RUL.02`) is opened by the
//!   activation like for any version.
//!
//! A step that fails does not stop the others: the accountant gets one task
//! that lists what to finish by hand, as for a termination.
//!
//! Decided where the spec is silent:
//!
//! - The rate of the new terms for the straddling period is the amount of the
//!   amendment's first charge of the same type that is due after the period;
//!   without one (the charge type is gone) only the old terms are billed.
//! - A scheduled charge of the old version due before the effective date that
//!   is not invoiced yet stays and is invoiced by the daily run as before.
//! - A one-off charge (the early termination penalty, a fine) is not credited.
//! - Allocating a payment to a reissued invoice needs the invoice to be
//!   issued; where drafts are not issued automatically the allocation is
//!   refused, the payment stays as credit and the accountant's task says so.

use std::collections::BTreeSet;

use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use wl_kernel::cheques::{ChequeId, ChequeStatus, ReturnCheques, cheques};
use wl_kernel::deposits::{DepositFuture, DepositTopUps, TopUpCharge};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{
    DraftLine, InvoiceDraft, InvoiceId, InvoiceStatus, ReopenReason, invoicer,
};
use wl_kernel::role_tasks::RoleTask;
use wl_kernel::{Subject, VatCategory};

use crate::building::{BUILDING_SUBJECT_TYPE, refused};
use crate::charge::{CHARGE_SUBJECT_TYPE, ChargeStatus, LeaseCharge};
use crate::cheques::of_version;
use crate::db::set_lease;
use crate::events::LeaseAmended;
use crate::lease::{Lease, LeaseStatus, VersionKind};
use crate::permissions::ACCOUNTANT_ROLE;
use crate::renew::periods;
use crate::termination::{is_recurring, prorated};
use crate::texts::english;

/// The charge type of the top-up of a deposit that grew.
pub const DEPOSIT_TOP_UP: &str = "deposit_top_up";

/// The effect of `amend` inside its transaction. `next` is the amendment
/// version that was activated; without it the `active` amendment of `old` is
/// looked for.
///
/// # Errors
///
/// 422 when no amendment version of `old` is `active` or it has no effective
/// date; the database's errors.
pub(crate) async fn amend_effects(
    tx: &mut Transaction,
    old: &Lease,
    next: Option<i64>,
) -> Result {
    let found = match next {
        Some(id) => Lease::find(&mut *tx, id).await?,
        None => {
            Lease::query()
                .where_(Lease::PREVIOUS_ID.eq(old.id))
                .where_(Lease::VERSION_KIND.eq(VersionKind::Amendment))
                .where_(Lease::STATUS.eq(LeaseStatus::Active))
                .first(&mut *tx)
                .await?
        }
    };
    let new = found
        .filter(|n| {
            n.previous_id == Some(old.id)
                && n.version_kind == VersionKind::Amendment
                && n.status == LeaseStatus::Active
        })
        .ok_or_else(|| refused(english("property.amend.no_amendment", &[])))?;
    let Some(effective) = new.effective_date else {
        return Err(refused(english("property.leases.effective_required", &[])));
    };

    set_lease(&mut *tx, old.id, &[("next_id", &Some(new.id))]).await?;
    LeaseCharge::query()
        .where_(LeaseCharge::LEASE_ID.eq(old.id))
        .where_(LeaseCharge::STATUS.is_in([ChargeStatus::Planned, ChargeStatus::Scheduled]))
        .where_(LeaseCharge::DUE_DATE.gte(effective))
        .update(
            &mut *tx,
            &[
                ("status", &ChargeStatus::Cancelled),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// Whether the period `period` (first and last day) is wholly on or after
/// `effective`.
pub fn is_after(period: (NaiveDate, NaiveDate), effective: NaiveDate) -> bool {
    period.0 >= effective
}

/// Whether the period `period` starts before `effective` and ends on or after
/// it.
pub fn straddles(period: (NaiveDate, NaiveDate), effective: NaiveDate) -> bool {
    period.0 < effective && period.1 >= effective
}

/// The two lines of an invoice that straddles `effective`: the old amount for
/// the days up to the day before it and the new amount for the days from it,
/// each daily pro rata of the whole period, rounded half up to a fils.
pub fn split(
    old_amount: i64,
    new_amount: i64,
    period: (NaiveDate, NaiveDate),
    effective: NaiveDate,
) -> (i64, i64) {
    let total = (period.1 - period.0).num_days() + 1;
    let before = (effective - period.0).num_days().clamp(0, total);
    (
        prorated(old_amount, before, total),
        prorated(new_amount, total - before, total),
    )
}

/// The charge whose amount is the new rate for a period that ends on `end`:
/// the first of the new charges of `charge_type` due after it, else the first
/// of that type.
fn new_rate<'a>(
    new_charges: &'a [LeaseCharge],
    charge_type: &str,
    end: NaiveDate,
) -> Option<&'a LeaseCharge> {
    let of_type = || new_charges.iter().filter(|c| c.charge_type == charge_type);
    of_type()
        .filter(|c| c.due_date > end)
        .min_by_key(|c| (c.due_date, c.id))
        .or_else(|| of_type().min_by_key(|c| (c.due_date, c.id)))
}

/// What [`settle`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Amendment {
    /// The invoices credited in full.
    pub credited: Vec<i64>,
    /// The draft invoices discarded.
    pub discarded: Vec<i64>,
    /// The invoices made again.
    pub reissued: Vec<i64>,
    /// The fils of payments taken off the credited invoices.
    pub unallocated: i64,
    /// The fils of those payments that stay as the tenant's credit.
    pub credit_left: i64,
    /// The cheques moved to a charge of the amendment.
    pub relinked: Vec<ChequeId>,
    /// The cheques handed back.
    pub returned: Vec<ChequeId>,
    /// The handover receipt of the cheques handed back.
    pub receipt: Option<String>,
    /// What could not be done and was left to the accountant.
    pub problems: Vec<String>,
    /// Whether this run did the work (`false` for a repeat).
    pub ran: bool,
}

/// A line for the accountant: what failed and why.
fn problem_line(what: &str, error: &Error) -> String {
    format!("{what}: {error}")
}

/// What the steps of [`settle`] share.
struct Run<'a> {
    /// The amendment version.
    new: &'a Lease,
    /// Its effective date.
    effective: NaiveDate,
    /// Its live charges.
    new_charges: &'a [LeaseCharge],
    /// The user the work is done for.
    by: Option<i64>,
}

/// Makes the invoices that replace the credited or discarded invoice of
/// `charge`, whose period is `period`.
async fn reissue(
    state: &AppState,
    run: &Run<'_>,
    charge: &LeaseCharge,
    period: (NaiveDate, NaiveDate),
) -> Result<Vec<InvoiceId>> {
    let Run {
        new,
        effective,
        new_charges,
        ..
    } = *run;
    if is_after(period, effective) {
        // The amendment's charges of the period that are due are invoiced now;
        // the others wait for the daily run.
        let mut made = Vec::new();
        for next in new_charges.iter().filter(|c| {
            c.charge_type == charge.charge_type
                && c.status == ChargeStatus::Scheduled
                && c.due_date >= period.0
                && c.due_date <= period.1
                && c.due_date <= today()
        }) {
            if let Some(id) = crate::invoicing::invoice_charge(state, new, next).await? {
                made.push(id);
            }
        }
        return Ok(made);
    }
    let rate = new_rate(new_charges, &charge.charge_type, period.1);
    let (before, after) = split(
        charge.amount,
        rate.map_or(0, |c| c.amount),
        period,
        effective,
    );
    let last_old_day = effective - renox::chrono::Duration::days(1);
    let old_text = english(
        "property.amend.line_old",
        &[
            ("lease", &new.number()),
            ("from", &period.0),
            ("to", &last_old_day),
        ],
    );
    let mut draft = InvoiceDraft::new(new.tenant(), new.subject())
        .scope(Subject::new(BUILDING_SUBJECT_TYPE, new.building_id))
        .due_date(charge.due_date)
        .date_of_supply(charge.due_date)
        .line(
            DraftLine::new(&old_text, before, charge.vat_category)
                .charge_type(&charge.charge_type)
                .origin(charge.subject()),
        );
    if let Some(rate) = rate {
        let new_text = english(
            "property.amend.line_new",
            &[
                ("lease", &new.number()),
                ("from", &effective),
                ("to", &period.1),
            ],
        );
        draft = draft.line(
            DraftLine::new(&new_text, after, rate.vat_category)
                .charge_type(&rate.charge_type)
                .origin(rate.subject()),
        );
    }
    let Some(id) = invoicer(state).draft(&state.db, draft).await? else {
        return Ok(Vec::new());
    };
    // The charge now stands for the reissued invoice.
    LeaseCharge::query()
        .where_(LeaseCharge::ID.eq(charge.id))
        .update(
            &state.db,
            &[
                ("invoice_id", &Some(id.0)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(vec![id])
}

/// Credits or discards the invoice of `charge` and makes it again, and puts
/// the payments taken off on the invoices made.
async fn replace_invoice(
    state: &AppState,
    run: &Run<'_>,
    charge: &LeaseCharge,
    period: (NaiveDate, NaiveDate),
    done: &mut Amendment,
) -> Result {
    let Run {
        new,
        effective,
        by,
        ..
    } = *run;
    let Some(invoice_id) = charge.invoice_id else {
        return Ok(());
    };
    let contract = invoicer(state);
    let Some(invoice) = contract.find(&state.db, InvoiceId(invoice_id)).await? else {
        return Ok(());
    };
    let mut freed = Vec::new();
    match invoice.status {
        InvoiceStatus::Draft => {
            if !contract
                .discard_draft(&state.db, InvoiceId(invoice_id))
                .await?
            {
                return Ok(());
            }
            done.discarded.push(invoice_id);
            if is_after(period, effective) {
                // The amendment's charge is invoiced on its due date.
                LeaseCharge::query()
                    .where_(LeaseCharge::ID.eq(charge.id))
                    .update(
                        &state.db,
                        &[
                            ("status", &ChargeStatus::Cancelled),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
            }
        }
        InvoiceStatus::Issued
        | InvoiceStatus::PartiallyPaid
        | InvoiceStatus::Overdue
        | InvoiceStatus::Paid => {
            if invoice.gross_total > invoice.balance {
                freed = contract
                    .unallocate_all(state, InvoiceId(invoice_id), ReopenReason::AllocationMoved)
                    .await?;
            }
            let note = english(
                "property.amend.credit_reason",
                &[("lease", &new.number_with_version())],
            );
            contract
                .credit_in_full(state, InvoiceId(invoice_id), note, by)
                .await?;
            done.credited.push(invoice_id);
        }
        InvoiceStatus::WrittenOff | InvoiceStatus::Credited => return Ok(()),
    }
    let made = reissue(state, run, charge, period).await?;
    done.reissued.extend(made.iter().map(|id| id.0));

    // The payments taken off go to the invoices made again.
    let mut room = Vec::new();
    for id in &made {
        if let Some(summary) = contract.find(&state.db, *id).await? {
            room.push((*id, summary.gross_total));
        }
    }
    for (source, amount) in freed {
        done.unallocated += amount;
        let mut left = amount;
        for (id, capacity) in &mut room {
            let take = left.min(*capacity);
            if take <= 0 {
                continue;
            }
            match contract.allocate(state, *id, source.clone(), take).await {
                Ok(()) => {
                    *capacity -= take;
                    left -= take;
                }
                Err(error) => {
                    done.problems
                        .push(problem_line(&format!("invoice {id} allocation"), &error));
                    break;
                }
            }
        }
        done.credit_left += left;
    }
    Ok(())
}

/// Relinks or hands back the cheques of `old` that pay the charges in
/// `affected`.
async fn settle_cheques(
    state: &AppState,
    old: &Lease,
    old_charges: &[LeaseCharge],
    new_charges: &[LeaseCharge],
    affected: &BTreeSet<i64>,
    by: Option<i64>,
    done: &mut Amendment,
) -> Result {
    let mut held = of_version(state, old).await?;
    held.sort_by_key(|c| (c.date, c.id.0));
    let mut taken: BTreeSet<i64> = BTreeSet::new();
    let mut hand_back = Vec::new();
    for cheque in held.iter().filter(|c| {
        matches!(c.status, ChequeStatus::Received | ChequeStatus::DueForDeposit)
    }) {
        let Some(paid) = cheque
            .linked
            .iter()
            .filter(|link| link.subject_type == CHARGE_SUBJECT_TYPE)
            .find_map(|link| {
                old_charges
                    .iter()
                    .find(|c| c.id == link.subject_id && affected.contains(&c.id))
            })
        else {
            continue;
        };
        let target = new_charges
            .iter()
            .filter(|n| {
                n.charge_type == paid.charge_type
                    && n.amount == cheque.amount
                    && !taken.contains(&n.id)
            })
            .min_by_key(|n| ((n.due_date - paid.due_date).num_days().abs(), n.id));
        match target {
            Some(next) => {
                match cheques(state)
                    .relink(state, cheque.id, paid.subject(), next.subject())
                    .await
                {
                    Ok(()) => {
                        taken.insert(next.id);
                        done.relinked.push(cheque.id);
                    }
                    Err(error) => done
                        .problems
                        .push(problem_line(&format!("cheque {}", cheque.number), &error)),
                }
            }
            None => hand_back.push(cheque.id),
        }
    }
    if hand_back.is_empty() {
        return Ok(());
    }
    let reason = english(
        "property.amend.cheque_reason",
        &[("lease", &old.number_with_version())],
    );
    match cheques(state)
        .return_undeposited(state, ReturnCheques::new(hand_back, &reason), by)
        .await
    {
        Ok(Some(back)) => {
            done.returned = back.returned;
            done.receipt = Some(back.receipt);
        }
        Ok(None) => {}
        Err(error) => done.problems.push(problem_line("cheques", &error)),
    }
    Ok(())
}

/// Runs the money effects of the activated amendment `id`, on behalf of `by`.
/// Idempotent: an amendment whose effects ran is left alone.
///
/// # Errors
///
/// The database's errors; a failing contract call is a problem in the result
/// and a task for the accountant, not an error.
pub async fn settle(state: &AppState, id: i64, by: Option<i64>) -> Result<Amendment> {
    let new = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    let (Some(effective), Some(old)) = (new.effective_date, new.previous(&state.db).await?)
    else {
        return Ok(Amendment::default());
    };
    if new.version_kind != VersionKind::Amendment || new.status != LeaseStatus::Active {
        return Ok(Amendment::default());
    }
    // Claimed first: of two runs holding the same row, one settles.
    let claimed = Lease::query()
        .where_(Lease::ID.eq(id))
        .where_(Lease::MONEY_SETTLED_AT.is_null())
        .update(
            &state.db,
            &[
                ("money_settled_at", &Some(renox::db::now())),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    if claimed == 0 {
        return Ok(Amendment::default());
    }
    let mut done = Amendment {
        ran: true,
        ..Amendment::default()
    };

    let old_charges = old.charges(&state.db).await?;
    let new_charges: Vec<LeaseCharge> = new
        .charges(&state.db)
        .await?
        .into_iter()
        .filter(|c| c.status != ChargeStatus::Cancelled)
        .collect();
    // The charges `amend` cancelled still stand for periods.
    let plan: Vec<&LeaseCharge> = old_charges
        .iter()
        .filter(|c| c.status != ChargeStatus::Cancelled || c.due_date >= effective)
        .collect();
    let ranges = periods(plan.iter().copied(), old.end_date);
    let run = Run {
        new: &new,
        effective,
        new_charges: &new_charges,
        by,
    };
    let mut affected: BTreeSet<i64> = old_charges
        .iter()
        .filter(|c| c.due_date >= effective)
        .map(|c| c.id)
        .collect();

    // 1. The invoices of the periods from the effective date on.
    for charge in old_charges
        .iter()
        .filter(|c| c.status == ChargeStatus::Invoiced && is_recurring(c, &plan))
    {
        let Some(period) = ranges.get(&charge.id).copied() else {
            continue;
        };
        if !is_after(period, effective) && !straddles(period, effective) {
            continue;
        }
        affected.insert(charge.id);
        let invoice_id = charge.invoice_id.unwrap_or_default();
        if let Err(error) = replace_invoice(state, &run, charge, period, &mut done).await {
            done.problems
                .push(problem_line(&format!("invoice {invoice_id}"), &error));
        }
    }

    // 2. The cheques.
    if let Err(error) = settle_cheques(
        state,
        &old,
        &old_charges,
        &new_charges,
        &affected,
        by,
        &mut done,
    )
    .await
    {
        done.problems.push(problem_line("cheques", &error));
    }

    // 3. What the accountant finishes by hand.
    if !done.returned.is_empty() {
        let title = english(
            "property.amend.task_cheques",
            &[("lease", &new.number_with_version())],
        );
        let description = english(
            "property.amend.task_cheques_description",
            &[
                ("count", &done.returned.len()),
                ("receipt", &done.receipt.clone().unwrap_or_default()),
            ],
        );
        let task = RoleTask::new(new.subject(), ACCOUNTANT_ROLE, &title, &cheques_key(&new))
            .description(&description);
        crate::tasks::open(state, &new.scope(), task).await?;
    }
    if done.credit_left > 0 {
        let note = english(
            "property.amend.credit_left",
            &[("amount", &crate::money_text(done.credit_left))],
        );
        done.problems.push(note);
    }
    if !done.problems.is_empty() {
        let title = english(
            "property.amend.task_problems",
            &[("lease", &new.number_with_version())],
        );
        let task = RoleTask::new(
            new.subject(),
            ACCOUNTANT_ROLE,
            &title,
            &problems_key(&new),
        )
        .description(&done.problems.join("\n"));
        crate::tasks::open(state, &new.scope(), task).await?;
    }
    state
        .emit(LeaseAmended {
            lease: new.id,
            previous: old.id,
            root: new.root_id,
            unit: new.unit_id,
            tenant: new.tenant(),
            effective_date: effective,
            annual_rent: new.annual_rent,
            previous_annual_rent: old.annual_rent,
            credited_invoices: done.credited.len(),
            reissued_invoices: done.reissued.len(),
            returned_cheques: done.returned.len(),
        })
        .await?;
    Ok(done)
}

/// The key the task to collect replacement cheques of the amendment `lease`
/// is found by.
pub fn cheques_key(lease: &Lease) -> String {
    format!("property.amend_cheques:{}", lease.id)
}

/// The key the task of what the accountant finishes by hand after the
/// amendment `lease` is found by.
pub fn problems_key(lease: &Lease) -> String {
    format!("property.amend_problems:{}", lease.id)
}

/// After the commit of the activation of the version `lease`: the money
/// effects of an amendment. Nothing for another kind of version.
pub(crate) async fn after_amend(state: &AppState, lease: &Lease, by: Option<i64>) -> Result {
    if lease.version_kind != VersionKind::Amendment {
        return Ok(());
    }
    settle(state, lease.id, by).await.map(drop)
}

/// Raises the top-up charge of a deposit that grew as a charge of the lease
/// version that raised it, due on the effective date (the daily run invoices
/// it). The charge catalogue of wl-invoicing is not built yet, so the lease
/// keeps the charge, like its rent.
pub struct LeaseTopUps;

impl DepositTopUps for LeaseTopUps {
    fn request<'a>(
        &'a self,
        state: &'a AppState,
        charge: TopUpCharge,
    ) -> DepositFuture<'a, Option<Subject>> {
        Box::pin(async move {
            if charge.amount <= 0 {
                return Ok(None);
            }
            let versions = Lease::versions_of(&state.db, charge.source.subject_id).await?;
            let Some(lease) = versions
                .iter()
                .find(|l| l.version == charge.source_version && l.status == LeaseStatus::Active)
                .or_else(|| {
                    versions
                        .iter()
                        .rev()
                        .find(|l| LeaseStatus::RUNNING.contains(&l.status))
                })
            else {
                return Ok(None);
            };
            let position = lease
                .charges(&state.db)
                .await?
                .iter()
                .map(|c| c.position)
                .max()
                .unwrap_or(0)
                + 1;
            let description = english(
                "property.amend.top_up_line",
                &[("lease", &lease.number_with_version())],
            );
            let made = LeaseCharge::create(
                &state.db,
                LeaseCharge {
                    lease_id: lease.id,
                    position,
                    charge_type: DEPOSIT_TOP_UP.to_owned(),
                    description,
                    due_date: charge.due,
                    amount: charge.amount,
                    vat_category: VatCategory::OutOfScope,
                    status: ChargeStatus::Scheduled,
                    ..Default::default()
                },
            )
            .await?;
            Ok(Some(made.subject()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2027, m, d).unwrap()
    }

    #[test]
    fn a_period_after_the_effective_date_is_reissued_whole() {
        let may = (day(5, 1), day(5, 31));
        assert!(is_after(may, day(5, 1)));
        assert!(is_after(may, day(4, 20)));
        assert!(!straddles(may, day(5, 1)));
        assert!(!is_after(may, day(5, 2)));
    }

    #[test]
    fn a_period_that_contains_the_effective_date_straddles_it() {
        let april = (day(4, 1), day(4, 30));
        assert!(straddles(april, day(4, 15)));
        assert!(straddles(april, day(4, 30)));
        assert!(!straddles(april, day(5, 1)));
        assert!(!straddles(april, day(4, 1)));
    }

    #[test]
    fn a_straddling_invoice_splits_by_days_of_the_period() {
        // AED 10,000 to the 14th, AED 11,000 from the 15th: 14 and 16 of 30
        // days.
        let april = (day(4, 1), day(4, 30));
        assert_eq!(
            split(1_000_000, 1_100_000, april, day(4, 15)),
            (466_667, 586_667)
        );
        // Effective on the second day: one day at the old rate.
        assert_eq!(
            split(1_000_000, 1_100_000, april, day(4, 2)),
            (33_333, 1_100_000 - 36_667)
        );
    }

    #[test]
    fn the_new_rate_is_the_first_charge_of_the_type_after_the_period() {
        let charge = |id, kind: &str, month, amount| LeaseCharge {
            id,
            charge_type: kind.to_owned(),
            due_date: day(month, 1),
            amount,
            ..Default::default()
        };
        let charges = [
            charge(1, "rent", 5, 1_100_000),
            charge(2, "rent", 6, 1_100_000),
            charge(3, "parking", 5, 50_000),
        ];
        let rent = new_rate(&charges, "rent", day(4, 30)).unwrap();
        assert_eq!(rent.id, 1);
        // No charge after the period: the first one.
        assert_eq!(new_rate(&charges, "rent", day(9, 30)).unwrap().id, 1);
        assert!(new_rate(&charges, "garden", day(4, 30)).is_none());
    }

    #[test]
    fn the_tasks_are_found_by_the_amendment() {
        let lease = Lease {
            id: 12,
            ..Default::default()
        };
        assert_eq!(cheques_key(&lease), "property.amend_cheques:12");
        assert_eq!(problems_key(&lease), "property.amend_problems:12");
    }
}
