//! Terminating a lease and settling the money after move-out (`terminate`,
//! `PRP.RUL.13`, `POR.ACL.01`, 01 Property §4.3 step 4 and row "Early
//! termination by the tenant", §7.1 *Lease*, *Unit* and *Cheque*; BL-PRP-11).
//!
//! When a tenant has moved out the property manager records the move-out
//! inspection and the keys ([`record_inspection`]) and runs `terminate`
//! ([`terminate`]) with the move-out date, the optional date until which the
//! contract keeps rent due (`rent_due_until`) and the choice for the credit
//! that is left. The transition needs the inspection completed and all the
//! keys returned.
//!
//! **In the transition's transaction** ([`terminate_effects`]): the move-out
//! date and the day are stored, the unit is vacated (`vacate_to_maintenance`
//! when the inspection found defects, else `vacate`), and the scheduled
//! charges after the move-out date, or after `rent_due_until` when set, are
//! cancelled. Rent and recurring charges only: a one-off charge (the early
//! termination penalty, a fine) stays. A charge is recurring when it is rent
//! or its type has more than one charge in the plan.
//!
//! **After the commit** ([`settle`]), with the contracts of the kernel, which
//! cannot join the transition's transaction:
//!
//! - the invoices issued for periods after the cutoff are credited pro rata by
//!   a partial credit note: amount × days after the cutoff ÷ days in the
//!   invoice's period, rounded half up to a fils ([`credit_for`]). On an
//!   invoice that is paid (or paid in part) the part of the credit note that
//!   no balance of the invoice uses stays on the credit note as the tenant's
//!   credit. A draft invoice for a period wholly after the cutoff is
//!   discarded;
//! - the cheques `received` or `due_for_deposit` and dated after the cutoff
//!   are returned against one receipt; those for kept periods stay;
//! - the settlement of the held deposit starts (nothing is published yet);
//! - the credit that is left on those credit notes (the termination credit)
//!   is **held for the deposit settlement** when a deposit is held
//!   ([`Lease::credit_held`], BL-PRP-32): the deductions are itemised only
//!   after the move-out inspection is settled (BL-PRP-17), and the credit
//!   covers them first when the deposit is refunded or forfeited
//!   ([`crate::termination_credit`]). With no deposit held (or one settled
//!   already) it follows the property manager's [`CreditChoice`] at once
//!   ([`settle_credit`]): `refund` asks it back through the Refund flow (the
//!   credit note is the refund's source, taken first through the invoicer,
//!   so it cannot be refunded twice; the refund waits for another user's
//!   approval, `FIN.ACL.04`), `keep` leaves it on the credit notes for the
//!   tenant to use;
//! - `LeaseTerminated` is emitted, which turns the tenant's portal read-only.
//!
//! **Before the transition** ([`money_problem`], BL-PRP-33): the spec runs the
//! money effects "all in one transaction" and rolls the whole `terminate` back
//! when crediting one invoice fails. The kernel's money contracts cannot join
//! the transition's transaction yet (their transaction-aware variants are
//! BL-KRN-11), so [`terminate`] first checks every step that can be known to
//! fail: an invoice to credit that cannot be found, a draft that straddles the
//! cutoff, a rent line without the credit left. Such a `terminate` is refused
//! with the reason and nothing changes: the lease stays `notice_given`, no
//! credit note, return or deposit settlement is made, no task is opened and no
//! `LeaseTerminated` is emitted; the user fixes the cause and runs it again.
//!
//! A step that still fails after the commit (a refusal the check cannot see)
//! does not stop the others: the accountant gets one task that lists what to
//! finish by hand, as for a cancellation. It goes away with BL-KRN-11, when
//! the effects run inside the transition's transaction.
//!
//! **`PRP.RUL.13`** ([`chase_ejari`], daily 02:00 Asia/Dubai): 7 days after the
//! lease became `terminated`, when an Ejari or Tawtheeq number is recorded and
//! no cancellation date, the property manager is notified and gets a task,
//! once. Recording the cancellation date ([`record_ejari_cancellation`])
//! closes the task.
//!
//! Decided where the spec is silent:
//!
//! - The move-out date is not checked against today: the keys may be
//!   returned on a past date.
//! - A cutoff earlier than the move-out date is raised to it.
//! - The period of an invoice is from its charge's due date to the day before
//!   the next charge of the same type, the last one to the end date of the
//!   lease ([`crate::renew::periods`]).

use std::collections::BTreeSet;

use renox::chrono::{Duration, NaiveDate};
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::cheques::{ChequeId, ChequeStatus, ReturnCheques, cheques};
use wl_kernel::deposits::deposits;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{
    CreditLine, CreditNoteId, CreditNoteStatus, InvoiceId, InvoiceStatus, invoicer,
};
use wl_kernel::payments::{RefundRequest, RefundSource, payment_recorder};
use wl_kernel::role_tasks::RoleTask;
use wl_kernel::tickets::task_creator;

use crate::building::refused;
use crate::charge::{ChargeStatus, LeaseCharge, RENT};
use crate::charge_type::EARLY_TERMINATION_FEE;
use crate::cheques::of_version;
use crate::db::set_lease;
use crate::events::LeaseTerminated;
use crate::lease::{CreditChoice, Lease, LeaseStatus};
use crate::lease_machine::TERMINATE;
use crate::notice::move_out_key;
use crate::notices;
use crate::permissions::{self, ACCOUNTANT_ROLE, PROPERTY_MANAGER_ROLE};
use crate::renew::periods;
use crate::texts::english;
use crate::unit::Unit;
use crate::unit_machine::{VACATE, VACATE_TO_MAINTENANCE};

/// The scheduled task and the command of `PRP.RUL.13`.
pub const TASK: &str = "property:ejari-cancellations";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "02:00";

/// The rule.
pub const RULE: &str = "PRP.RUL.13";

/// N of `PRP.RUL.13`: the days after the lease became `terminated`.
pub const CANCEL_DAYS: i64 = 7;

/// The key the Ejari cancellation task of `lease` is found by while open.
pub fn cancel_task_key(lease: &Lease) -> String {
    format!("property.ejari_cancel:{}", lease.id)
}

/// What a `terminate` run carries to its guards and effect.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminateInput {
    /// The day the keys were returned.
    #[serde(default)]
    pub move_out: Option<NaiveDate>,
    /// Rent stays due to this day, when the contract says so.
    #[serde(default)]
    pub rent_due_until: Option<NaiveDate>,
    /// Keep the credit that is left as credit instead of refunding it.
    #[serde(default)]
    pub keep_credit: bool,
    /// The day the run is for; today when empty.
    #[serde(default)]
    pub day: Option<NaiveDate>,
}

/// The input of a `terminate` run.
pub fn terminate_input(ctx: &TransitionCtx) -> TerminateInput {
    ctx.input_as::<TerminateInput>().unwrap_or_default()
}

/// Why `lease` cannot be terminated with `input`, if it cannot: the move-out
/// inspection is completed and all the keys are back, the move-out date is
/// given and not before the lease started, and rent is not due until a day
/// before it.
pub fn problem(lease: &Lease, input: &TerminateInput) -> Option<String> {
    if lease.inspection_done_on.is_none() {
        return Some(english("property.terminate.inspection_missing", &[]));
    }
    if lease.keys_returned_on.is_none() {
        return Some(english("property.terminate.keys_missing", &[]));
    }
    let Some(move_out) = input.move_out else {
        return Some(english("property.terminate.move_out_required", &[]));
    };
    if move_out < lease.start_date {
        return Some(english("property.terminate.move_out_early", &[]));
    }
    if input.rent_due_until.is_some_and(|until| until < move_out) {
        return Some(english("property.terminate.due_until_early", &[]));
    }
    None
}

/// The last day whose rent and cheques stay: the day rent is due until, when
/// the contract says so, else the move-out date.
pub fn cutoff(move_out: NaiveDate, rent_due_until: Option<NaiveDate>) -> NaiveDate {
    rent_due_until.map_or(move_out, |until| until.max(move_out))
}

/// `amount × days ÷ of`, rounded half up to a fils.
pub fn prorated(amount: i64, days: i64, of: i64) -> i64 {
    if of <= 0 || days <= 0 {
        return 0;
    }
    let numerator = i128::from(amount) * i128::from(days) * 2 + i128::from(of);
    i64::try_from(numerator / (2 * i128::from(of))).unwrap_or(i64::MAX)
}

/// The net amount to credit for an invoice of `amount` fils whose period runs
/// from `period.0` to `period.1`, when the lease ends on `cutoff`: the days
/// after it, daily, rounded half up. 0 when the period ends on or before it.
pub fn credit_for(amount: i64, period: (NaiveDate, NaiveDate), cutoff: NaiveDate) -> i64 {
    let (start, end) = period;
    if end <= cutoff {
        return 0;
    }
    let total = (end - start).num_days() + 1;
    let after = if start > cutoff {
        total
    } else {
        (end - cutoff).num_days()
    };
    prorated(amount, after, total)
}

/// Whether `charge` is rent or recurring and so cancelled or credited after
/// the cutoff: a one-off charge (the early termination penalty, a fine) stays.
/// `plan` is the live charges of the lease.
pub fn is_recurring(charge: &LeaseCharge, plan: &[&LeaseCharge]) -> bool {
    if charge.charge_type == EARLY_TERMINATION_FEE {
        return false;
    }
    charge.charge_type == RENT
        || plan
            .iter()
            .filter(|c| c.charge_type == charge.charge_type)
            .count()
            > 1
}

/// The effect of `terminate` inside its transaction: the move-out is stored,
/// the unit is vacated and the recurring charges after the cutoff that are
/// not invoiced are cancelled.
pub(crate) async fn terminate_effects(
    tx: &mut Transaction,
    lease: &Lease,
    input: &TerminateInput,
) -> Result {
    let Some(move_out) = input.move_out else {
        return Err(refused(english(
            "property.terminate.move_out_required",
            &[],
        )));
    };
    let day = input.day.unwrap_or_else(today);
    let due_until = input.rent_due_until.or(lease.rent_due_until);
    let choice = if input.keep_credit {
        CreditChoice::Keep
    } else {
        CreditChoice::Refund
    };
    set_lease(
        &mut *tx,
        lease.id,
        &[
            ("actual_move_out", &Some(move_out)),
            ("terminated_on", &Some(day)),
            ("rent_due_until", &due_until),
            ("credit_choice", &choice),
        ],
    )
    .await?;

    let cut = cutoff(move_out, due_until);
    let charges = LeaseCharge::query()
        .where_(LeaseCharge::LEASE_ID.eq(lease.id))
        .get(&mut *tx)
        .await?;
    let plan: Vec<&LeaseCharge> = charges
        .iter()
        .filter(|c| c.status != ChargeStatus::Cancelled)
        .collect();
    let ids: Vec<i64> = charges
        .iter()
        .filter(|c| {
            matches!(c.status, ChargeStatus::Planned | ChargeStatus::Scheduled)
                && c.due_date > cut
                && is_recurring(c, &plan)
        })
        .map(|c| c.id)
        .collect();
    if !ids.is_empty() {
        LeaseCharge::query()
            .where_(LeaseCharge::ID.is_in(ids))
            .update(
                &mut *tx,
                &[
                    ("status", &ChargeStatus::Cancelled),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
    }

    let unit = Unit::find(&mut *tx, lease.unit_id)
        .await?
        .ok_or(Error::NotFound)?;
    let name = if lease.inspection_defects {
        VACATE_TO_MAINTENANCE
    } else {
        VACATE
    };
    let outcome = unit
        .transition(name)
        .by_system()
        .run_in(&mut *tx)
        .await
        .map_err(TransitionError::into_error)?;
    drop(outcome);
    Ok(())
}

/// Why the money effects of terminating `lease` with `input` would fail, if
/// they would (BL-PRP-33): every invoice to credit must be found, a draft must
/// not straddle the cutoff (it can be neither discarded nor credited), and the
/// rent line of an issued invoice must have the credit left. Asked before the
/// transition, so a refused `terminate` changes nothing, creates no task and
/// emits no event.
///
/// # Errors
///
/// The database's errors.
pub async fn money_problem(
    state: &AppState,
    lease: &Lease,
    input: &TerminateInput,
) -> Result<Option<String>> {
    let Some(move_out) = input.move_out else {
        return Ok(None);
    };
    let cut = cutoff(move_out, input.rent_due_until.or(lease.rent_due_until));
    let charges = lease.charges(&state.db).await?;
    let plan: Vec<&LeaseCharge> = charges
        .iter()
        .filter(|c| c.status != ChargeStatus::Cancelled || c.due_date > cut)
        .collect();
    let ranges = periods(plan.iter().copied(), lease.end_date);
    let contract = invoicer(state);
    for charge in charges
        .iter()
        .filter(|c| c.status == ChargeStatus::Invoiced && is_recurring(c, &plan))
    {
        let (Some(invoice_id), Some(period)) = (charge.invoice_id, ranges.get(&charge.id)) else {
            continue;
        };
        let net = credit_for(charge.amount, *period, cut);
        if net == 0 {
            continue;
        }
        let invoice = match contract.find(&state.db, InvoiceId(invoice_id)).await {
            Ok(Some(invoice)) => invoice,
            Ok(None) => {
                return Ok(Some(english(
                    "property.terminate.invoice_missing",
                    &[("invoice", &invoice_id)],
                )));
            }
            Err(error) => return Ok(Some(problem_line(&format!("invoice {invoice_id}"), &error))),
        };
        match invoice.status {
            InvoiceStatus::Draft if period.0 > cut => {}
            InvoiceStatus::Draft => {
                return Ok(Some(english(
                    "property.terminate.draft_blocks",
                    &[("invoice", &invoice_id)],
                )));
            }
            InvoiceStatus::Issued
            | InvoiceStatus::PartiallyPaid
            | InvoiceStatus::Overdue
            | InvoiceStatus::Paid => {
                let lines = contract.lines_of(&state.db, InvoiceId(invoice_id)).await?;
                let Some(line) = lines.iter().find(|l| l.position == 1) else {
                    continue;
                };
                let credited: i64 = contract
                    .credit_notes_for(&state.db, InvoiceId(invoice_id))
                    .await?
                    .iter()
                    .filter(|note| note.status != CreditNoteStatus::Draft)
                    .map(|note| note.gross_total)
                    .sum();
                let left = line.gross - credited;
                if net > left {
                    let number = invoice
                        .number
                        .clone()
                        .unwrap_or_else(|| format!("#{invoice_id}"));
                    return Ok(Some(english(
                        "property.terminate.credit_too_large",
                        &[
                            ("invoice", &number),
                            ("needed", &crate::money_text(net)),
                            ("left", &crate::money_text(left.max(0))),
                        ],
                    )));
                }
            }
            InvoiceStatus::WrittenOff | InvoiceStatus::Credited => {}
        }
    }
    Ok(None)
}

/// The lease `id`, or a 404.
async fn load(state: &AppState, id: i64) -> std::result::Result<Lease, TransitionError> {
    Lease::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// What a user enters to terminate a lease.
#[derive(Debug, Clone)]
pub struct Termination {
    /// The day the keys were returned.
    pub move_out: NaiveDate,
    /// Rent stays due to this day, when the contract says so.
    pub rent_due_until: Option<NaiveDate>,
    /// Keep the credit that is left as credit instead of refunding it.
    pub keep_credit: bool,
}

impl Termination {
    /// A termination with the move-out on `move_out`.
    pub fn on(move_out: NaiveDate) -> Self {
        Termination {
            move_out,
            rent_due_until: None,
            keep_credit: false,
        }
    }
}

/// `user` terminates the lease `id`.
///
/// # Errors
///
/// `Forbidden` without `leases.manage` in the lease's building, `NotFrom`
/// unless `notice_given`, `Guard` (422) unless the move-out inspection is
/// completed and all the keys are returned, or for a move-out date the lease
/// cannot have.
pub async fn terminate(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    termination: Termination,
) -> std::result::Result<Lease, TransitionError> {
    let lease = load(state, id).await?;
    let input = TerminateInput {
        move_out: Some(termination.move_out),
        rent_due_until: termination.rent_due_until,
        keep_credit: termination.keep_credit,
        day: Some(today()),
    };
    // The money effects are checked before anything changes (BL-PRP-33): a
    // step that would fail refuses `terminate` with its reason.
    if lease.status == LeaseStatus::NoticeGiven
        && user.has_permission_in(permissions::LEASES_MANAGE, &lease.scope())
        && problem(&lease, &input).is_none()
        && let Some(reason) = money_problem(state, &lease, &input).await?
    {
        return Err(TransitionError::Guard(reason));
    }
    let reason = english("property.terminate.history", &[]);
    let _outcome = lease
        .transition(TERMINATE)
        .by(user)
        .reason(reason)
        .input(&input)
        .run(state)
        .await?;
    load(state, id).await
}

/// What [`settle`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settlement {
    /// The invoices credited, with the net amount of the credit note.
    pub credited: Vec<(i64, i64)>,
    /// The draft invoices discarded.
    pub discarded: Vec<i64>,
    /// The cheques handed back.
    pub returned: Vec<ChequeId>,
    /// The handover receipt of the cheques handed back.
    pub receipt: Option<String>,
    /// The deposit whose settlement started.
    pub deposit: Option<i64>,
    /// The net amount credited in all, in fils.
    pub credit: i64,
    /// The credit that stayed on the credit notes (the invoices did not use
    /// it) and was asked back, in fils: the refunds of [`Settlement::refunds`].
    pub refunded: i64,
    /// The refunds asked for that credit, oldest first.
    pub refunds: Vec<i64>,
    /// The credit that stayed with the tenant because the choice is to keep
    /// it, in fils.
    pub kept: i64,
    /// The credit held for the deposit settlement, in fils: a deposit is held,
    /// so the credit goes to the deductions first (BL-PRP-32).
    pub held: i64,
    /// The credit notes the held credit sits on.
    pub held_notes: Vec<CreditNoteId>,
    /// What could not be done and was left to the accountant.
    pub problems: Vec<String>,
    /// Whether this run did the work (`false` for a repeat).
    pub ran: bool,
}

/// A line for the accountant: what failed and why.
fn problem_line(what: &str, error: &Error) -> String {
    format!("{what}: {error}")
}

/// Runs the money effects of the terminated lease `id`, on behalf of `by`.
/// Idempotent: a lease whose effects ran is left alone.
///
/// # Errors
///
/// The database's errors; a failing contract call is a problem in the result
/// and a task for the accountant, not an error.
pub async fn settle(state: &AppState, id: i64, by: Option<i64>) -> Result<Settlement> {
    settle_keeping(state, id, by, &BTreeSet::new()).await
}

/// Like [`settle`], but the cheques whose ids are in `keep` are not returned
/// (a forced termination holds them as evidence, [`crate::force`]).
///
/// # Errors
///
/// The database's errors; a failing contract call is a problem in the result
/// and a task for the accountant, not an error.
pub async fn settle_keeping(
    state: &AppState,
    id: i64,
    by: Option<i64>,
    keep: &BTreeSet<i64>,
) -> Result<Settlement> {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    let Some(move_out) = lease.actual_move_out else {
        return Ok(Settlement::default());
    };
    if lease.status != LeaseStatus::Terminated || lease.money_settled_at.is_some() {
        return Ok(Settlement::default());
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
        return Ok(Settlement::default());
    }
    let mut done = Settlement {
        ran: true,
        ..Settlement::default()
    };
    let mut notes: Vec<CreditNoteId> = Vec::new();
    let cut = cutoff(move_out, lease.rent_due_until);
    let reason = english(
        "property.terminate.reason",
        &[("lease", &lease.number_with_version())],
    );

    // 1. The invoices of periods after the cutoff.
    let charges = lease.charges(&state.db).await?;
    let plan: Vec<&LeaseCharge> = charges
        .iter()
        .filter(|c| c.status != ChargeStatus::Cancelled || c.due_date > cut)
        .collect();
    let ranges = periods(plan.iter().copied(), lease.end_date);
    for charge in charges
        .iter()
        .filter(|c| c.status == ChargeStatus::Invoiced && is_recurring(c, &plan))
    {
        let (Some(invoice_id), Some(period)) = (charge.invoice_id, ranges.get(&charge.id)) else {
            continue;
        };
        let net = credit_for(charge.amount, *period, cut);
        if net == 0 {
            continue;
        }
        let contract = invoicer(state);
        let found = match contract.find(&state.db, InvoiceId(invoice_id)).await {
            Ok(found) => found,
            Err(error) => {
                done.problems
                    .push(problem_line(&format!("invoice {invoice_id}"), &error));
                continue;
            }
        };
        let Some(invoice) = found else {
            continue;
        };
        match invoice.status {
            InvoiceStatus::Draft if period.0 > cut => {
                match contract
                    .discard_draft(&state.db, InvoiceId(invoice_id))
                    .await
                {
                    Ok(_) => {
                        done.discarded.push(invoice_id);
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
                    Err(error) => done
                        .problems
                        .push(problem_line(&format!("invoice {invoice_id}"), &error)),
                }
            }
            InvoiceStatus::Draft => done.problems.push(english(
                "property.terminate.draft_straddles",
                &[("invoice", &invoice_id)],
            )),
            InvoiceStatus::Issued
            | InvoiceStatus::PartiallyPaid
            | InvoiceStatus::Overdue
            | InvoiceStatus::Paid => {
                match contract
                    .credit_partly(
                        state,
                        InvoiceId(invoice_id),
                        vec![CreditLine::new(1, net)],
                        reason.clone(),
                        by,
                    )
                    .await
                {
                    Ok(note) => {
                        done.credited.push((invoice_id, net));
                        done.credit += net;
                        notes.push(note);
                    }
                    Err(error) => done
                        .problems
                        .push(problem_line(&format!("invoice {invoice_id}"), &error)),
                }
            }
            InvoiceStatus::WrittenOff | InvoiceStatus::Credited => {}
        }
    }

    // 2. The cheques dated after the cutoff go back against one receipt.
    match of_version(state, &lease).await {
        Ok(held) => {
            let later: Vec<ChequeId> = held
                .iter()
                .filter(|c| {
                    matches!(
                        c.status,
                        ChequeStatus::Received | ChequeStatus::DueForDeposit
                    ) && c.date > cut
                        && !keep.contains(&c.id.0)
                })
                .map(|c| c.id)
                .collect();
            if !later.is_empty() {
                match cheques(state)
                    .return_undeposited(state, ReturnCheques::new(later, &reason), by)
                    .await
                {
                    Ok(Some(back)) => {
                        done.returned = back.returned;
                        done.receipt = Some(back.receipt);
                    }
                    Ok(None) => {}
                    Err(error) => done.problems.push(problem_line("cheques", &error)),
                }
            }
        }
        Err(error) => done.problems.push(problem_line("cheques", &error)),
    }

    // 3. The settlement of the deposit starts.
    match deposits(state)
        .start_settlement(state, &lease.agreement())
        .await
    {
        Ok(started) => done.deposit = started.map(|s| s.deposit.0),
        Err(error) => done.problems.push(problem_line("deposit", &error)),
    }

    // 3b. The credit that the invoices did not use: held for the deductions
    // of the deposit being settled, else as the property manager chose.
    if done.deposit.is_some() {
        hold_credit(state, &notes, &mut done).await;
    } else {
        settle_credit(state, &lease, &notes, by, &mut done).await;
    }

    // 4. What was credited, what is left by hand, the event.
    Lease::query()
        .where_(Lease::ID.eq(id))
        .update(
            &state.db,
            &[
                ("termination_credit", &done.credit),
                ("credit_held", &done.held),
                ("credit_notes_held", &held_notes_text(&done.held_notes)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    if !done.problems.is_empty() {
        let title = english(
            "property.terminate.task_problems",
            &[("lease", &lease.number_with_version())],
        );
        let task = RoleTask::new(
            lease.subject(),
            ACCOUNTANT_ROLE,
            &title,
            &format!("property.terminate_problems:{}", lease.id),
        )
        .description(&done.problems.join("\n"));
        crate::tasks::open(state, &lease.scope(), task).await?;
    }
    state
        .emit(LeaseTerminated {
            lease: lease.id,
            root: lease.root_id,
            unit: lease.unit_id,
            tenant: lease.tenant(),
            move_out,
            rent_due_until: lease.rent_due_until,
            credited: done.credit,
        })
        .await?;
    Ok(done)
}

/// The credit notes of `notes`, as the lease stores them: comma separated,
/// `None` for none.
pub fn held_notes_text(notes: &[CreditNoteId]) -> Option<String> {
    if notes.is_empty() {
        return None;
    }
    Some(
        notes
            .iter()
            .map(|note| note.0.to_string())
            .collect::<Vec<_>>()
            .join(","),
    )
}

/// The credit notes a lease stores in `text` ([`held_notes_text`]).
pub fn held_notes_of(text: Option<&str>) -> Vec<CreditNoteId> {
    text.unwrap_or_default()
        .split(',')
        .filter_map(|part| part.trim().parse::<i64>().ok())
        .map(CreditNoteId)
        .collect()
}

/// A deposit is held: the credit left on the credit notes of the termination
/// waits for the deposit settlement, where it covers the deductions first
/// ([`crate::termination_credit`]). Nothing is refunded or released now.
async fn hold_credit(state: &AppState, notes: &[CreditNoteId], done: &mut Settlement) {
    let contract = invoicer(state);
    for note in notes {
        match contract.find_credit_note(&state.db, *note).await {
            Ok(Some(summary))
                if summary.status == CreditNoteStatus::Issued && summary.remaining > 0 =>
            {
                done.held += summary.remaining;
                done.held_notes.push(*note);
            }
            Ok(_) => {}
            Err(error) => done
                .problems
                .push(problem_line(&format!("credit note {note}"), &error)),
        }
    }
}

/// The credit left on the credit notes of the termination, as the property
/// manager chose (`lease.credit_choice`): asked back through the Refund flow,
/// or kept for the tenant. A failing step is a problem for the accountant, not
/// an error.
pub(crate) async fn settle_credit(
    state: &AppState,
    lease: &Lease,
    notes: &[CreditNoteId],
    by: Option<i64>,
    done: &mut Settlement,
) {
    let contract = invoicer(state);
    for note in notes {
        let summary = match contract.find_credit_note(&state.db, *note).await {
            Ok(Some(summary)) => summary,
            Ok(None) => continue,
            Err(error) => {
                done.problems
                    .push(problem_line(&format!("credit note {note}"), &error));
                continue;
            }
        };
        if summary.status != CreditNoteStatus::Issued || summary.remaining <= 0 {
            continue;
        }
        if lease.credit_choice == CreditChoice::Keep {
            done.kept += summary.remaining;
            continue;
        }
        let Some(entity) = summary.legal_entity_id else {
            done.problems.push(english(
                "property.terminate.credit_no_entity",
                &[("note", &note.0)],
            ));
            continue;
        };
        // Built before the await: the argument list of `english` is not `Send`.
        let reason = english(
            "property.terminate.refund_reason",
            &[("lease", &lease.number_with_version())],
        );
        let asked = payment_recorder(state)
            .request_refund(
                state,
                RefundRequest {
                    source: RefundSource::CreditNote(note.subject()),
                    party: lease.tenant(),
                    legal_entity_id: entity,
                    amount: summary.remaining,
                    currency: summary.currency.clone(),
                    method: None,
                    gateway_txn: None,
                    reason,
                },
                by,
            )
            .await;
        let refund = match asked {
            Ok(refund) => refund,
            Err(error) => {
                done.problems.push(problem_line(
                    &format!("refund of credit note {note}"),
                    &error,
                ));
                continue;
            }
        };
        // The refund exists: the credit note gives the amount to it, so the
        // same credit is not refunded twice.
        match contract
            .take_credit_for_refund(state, *note, refund.subject(), 1, summary.remaining)
            .await
        {
            Ok(()) => {
                done.refunded += summary.remaining;
                done.refunds.push(refund.0);
            }
            Err(error) => {
                let line = english(
                    "property.terminate.credit_not_taken",
                    &[("refund", &refund.0), ("note", &note.0)],
                );
                done.problems.push(format!("{line} ({error})"));
            }
        }
    }
}

/// What a user records of the move-out inspection.
#[derive(Debug, Clone, Default)]
pub struct Inspection {
    /// The day the inspection was completed.
    pub done_on: Option<NaiveDate>,
    /// The day all the keys were returned.
    pub keys_returned_on: Option<NaiveDate>,
    /// The inspection found defects that need work: the unit goes to
    /// maintenance when the tenant has left.
    pub defects: bool,
}

/// `user` records the move-out inspection of lease `id`: when it was
/// completed (which closes its ticket), when the keys were returned, and
/// whether it found defects. A date left out keeps what was recorded.
///
/// # Errors
///
/// 403 without `leases.manage` in the lease's building; 422 unless the lease
/// has a notice, or when nothing is given.
pub async fn record_inspection(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    inspection: Inspection,
) -> Result<Lease> {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    if !user.has_permission_in(permissions::LEASES_MANAGE, &lease.scope()) {
        return Err(Error::Forbidden);
    }
    if lease.status != LeaseStatus::NoticeGiven {
        return Err(refused(english("property.inspection.no_notice", &[])));
    }
    if inspection.done_on.is_none() && inspection.keys_returned_on.is_none() {
        return Err(refused(english("property.inspection.nothing", &[])));
    }
    if let Some(day) = inspection.done_on {
        Lease::query()
            .where_(Lease::ID.eq(id))
            .update(
                &state.db,
                &[
                    ("inspection_done_on", &Some(day)),
                    ("inspection_defects", &inspection.defects),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
        let creator = task_creator(state);
        if let Some(ticket) = creator.find_open(state, &move_out_key(&lease)).await? {
            let note = english("property.inspection.ticket_note", &[]);
            creator.complete(state, ticket.id, &note).await?;
        }
    }
    if let Some(day) = inspection.keys_returned_on {
        Lease::query()
            .where_(Lease::ID.eq(id))
            .update(
                &state.db,
                &[
                    ("keys_returned_on", &Some(day)),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
    }
    Lease::find(&state.db, id).await?.ok_or(Error::NotFound)
}

/// `user` records the day the Ejari or Tawtheeq registration of the
/// terminated lease `id` was cancelled, which closes the task of
/// `PRP.RUL.13`.
///
/// # Errors
///
/// 403 without `leases.manage` in the lease's building; 422 unless the lease
/// is terminated and has a registration number.
pub async fn record_ejari_cancellation(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    cancelled_on: NaiveDate,
) -> Result<Lease> {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    if !user.has_permission_in(permissions::LEASES_MANAGE, &lease.scope()) {
        return Err(Error::Forbidden);
    }
    if lease.status != LeaseStatus::Terminated || lease.ejari_number.is_none() {
        return Err(refused(english("property.ejari.cancel_refused", &[])));
    }
    Lease::query()
        .where_(Lease::ID.eq(id))
        .update(
            &state.db,
            &[
                ("ejari_cancelled_on", &Some(cancelled_on)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    let creator = task_creator(state);
    if let Some(task) = creator.find_open(state, &cancel_task_key(&lease)).await? {
        let note = english("property.ejari.cancelled", &[]);
        creator.complete(state, task.id, &note).await?;
    }
    Lease::find(&state.db, id).await?.ok_or(Error::NotFound)
}

/// `PRP.RUL.13` for `day`: the terminated leases with a registration number
/// and no cancellation date, 7 days after they were terminated, tell the
/// property managers and open the task, once. Returns how many leases were
/// handled. Every lease is tried; the first failure is returned after the
/// others.
///
/// # Errors
///
/// The database's errors, or the first lease that could not be handled.
pub async fn chase_ejari(state: &AppState, day: NaiveDate) -> Result<usize> {
    let due = Lease::query()
        .where_(Lease::STATUS.eq(LeaseStatus::Terminated))
        .where_(Lease::EJARI_NUMBER.is_not_null())
        .where_(Lease::EJARI_CANCELLED_ON.is_null())
        .where_(Lease::EJARI_CHASED_ON.is_null())
        .where_(Lease::TERMINATED_ON.lte(day - Duration::days(CANCEL_DAYS)))
        .order_by(Lease::ID)
        .get(&state.db)
        .await?;
    let mut handled = 0;
    let mut failure = None;
    for lease in &due {
        match chase_one(state, lease, day).await {
            Ok(true) => handled += 1,
            Ok(false) => {}
            Err(error) => {
                eprintln!(
                    "property:ejari-cancellations: lease {}: {error:?}",
                    lease.id
                );
                failure.get_or_insert(error);
            }
        }
    }
    failure.map_or(Ok(handled), Err)
}

/// Chases one lease; `false` when another run did it first.
async fn chase_one(state: &AppState, lease: &Lease, day: NaiveDate) -> Result<bool> {
    // Stamped first: of two runs, one chases.
    let claimed = Lease::query()
        .where_(Lease::ID.eq(lease.id))
        .where_(Lease::EJARI_CHASED_ON.is_null())
        .where_(Lease::EJARI_CANCELLED_ON.is_null())
        .update(
            &state.db,
            &[
                ("ejari_chased_on", &Some(day)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    if claimed == 0 {
        return Ok(false);
    }
    notices::ejari_cancellation(state, lease).await?;
    let title = english(
        "property.ejari.cancel_task",
        &[("lease", &lease.number_with_version())],
    );
    let description = english(
        "property.ejari.cancel_task_description",
        &[
            ("number", &lease.ejari_number.clone().unwrap_or_default()),
            ("days", &CANCEL_DAYS),
        ],
    );
    let task = RoleTask::new(
        lease.subject(),
        PROPERTY_MANAGER_ROLE,
        &title,
        &cancel_task_key(lease),
    )
    .description(&description);
    crate::tasks::open(state, &lease.scope(), task).await?;
    Ok(true)
}

/// After the commit of `terminate` on lease `id`: the money effects.
pub(crate) async fn after_terminate(state: &AppState, id: i64, by: Option<i64>) -> Result {
    settle(state, id, by).await.map(drop)
}

/// Registers the daily task of `PRP.RUL.13` and its command.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            chase_ejari(&state, today()).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
    app.command(
        TASK,
        "Chase the cancellation of the Ejari or Tawtheeq registration of terminated leases (--date YYYY-MM-DD checks another day)",
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
            let handled = chase_ejari(&state, on).await?;
            println!("{handled} leases chased for the Ejari cancellation.");
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2027, m, d).unwrap()
    }

    #[test]
    fn a_pro_rata_amount_is_daily_and_rounds_half_up() {
        // AED 9,000 for July, 11 of 31 days after the 20th: AED 3,193.55.
        assert_eq!(prorated(900_000, 11, 31), 319_355);
        assert_eq!(prorated(900_000, 31, 31), 900_000);
        assert_eq!(prorated(900_000, 0, 31), 0);
        // Half a fils rounds up: 1 fils × 1 / 2.
        assert_eq!(prorated(1, 1, 2), 1);
        assert_eq!(prorated(3, 1, 3), 1);
        assert_eq!(prorated(100, 1, 3), 33);
        assert_eq!(prorated(100, 2, 3), 67);
    }

    #[test]
    fn an_invoice_is_credited_for_the_days_after_the_cutoff() {
        let july = (day(7, 1), day(7, 31));
        assert_eq!(credit_for(900_000, july, day(7, 20)), 319_355);
        // Moved out on the last day: nothing after it.
        assert_eq!(credit_for(900_000, july, day(7, 31)), 0);
        assert_eq!(credit_for(900_000, july, day(8, 15)), 0);
        // A period wholly after the cutoff is credited in full.
        let august = (day(8, 1), day(8, 31));
        assert_eq!(credit_for(900_000, august, day(7, 20)), 900_000);
        // Moved out the day before the period starts.
        assert_eq!(credit_for(900_000, august, day(7, 31)), 900_000);
        // Moved out on the first day: 30 of 31 days after it.
        assert_eq!(
            credit_for(900_000, july, day(7, 1)),
            prorated(900_000, 30, 31)
        );
    }

    #[test]
    fn the_cutoff_is_the_move_out_or_the_day_rent_is_due_until() {
        assert_eq!(cutoff(day(7, 20), None), day(7, 20));
        assert_eq!(cutoff(day(7, 20), Some(day(9, 30))), day(9, 30));
        assert_eq!(cutoff(day(7, 20), Some(day(7, 1))), day(7, 20));
    }

    #[test]
    fn rent_and_repeated_charges_are_recurring_and_one_off_charges_are_not() {
        let charge = |kind: &str| LeaseCharge {
            charge_type: kind.to_owned(),
            status: ChargeStatus::Scheduled,
            ..Default::default()
        };
        let rent = charge(RENT);
        let fee = charge(EARLY_TERMINATION_FEE);
        let parking_a = charge("parking");
        let parking_b = charge("parking");
        let fine = charge("fine");
        let plan = [&rent, &fee, &parking_a, &parking_b, &fine];
        assert!(is_recurring(&rent, &plan));
        assert!(!is_recurring(&fee, &plan));
        assert!(is_recurring(&parking_a, &plan));
        assert!(!is_recurring(&fine, &plan));
    }

    #[test]
    fn terminate_needs_the_inspection_the_keys_and_a_move_out_date() {
        let mut lease = Lease {
            start_date: day(1, 1),
            ..Default::default()
        };
        let input = TerminateInput {
            move_out: Some(day(7, 20)),
            ..Default::default()
        };
        assert!(problem(&lease, &input).unwrap().contains("inspection"));
        lease.inspection_done_on = Some(day(7, 19));
        assert!(problem(&lease, &input).unwrap().contains("keys"));
        lease.keys_returned_on = Some(day(7, 20));
        assert_eq!(problem(&lease, &input), None);
        let none = TerminateInput::default();
        assert!(problem(&lease, &none).unwrap().contains("move-out date"));
        let early = TerminateInput {
            move_out: Some(day(1, 1) - Duration::days(1)),
            ..Default::default()
        };
        assert!(problem(&lease, &early).is_some());
        let before = TerminateInput {
            move_out: Some(day(7, 20)),
            rent_due_until: Some(day(7, 1)),
            ..Default::default()
        };
        assert!(problem(&lease, &before).is_some());
    }

    #[test]
    fn the_ejari_task_is_found_by_the_lease() {
        let lease = Lease {
            id: 8,
            ..Default::default()
        };
        assert_eq!(cancel_task_key(&lease), "property.ejari_cancel:8");
        assert_eq!(CANCEL_DAYS, 7);
    }
}
