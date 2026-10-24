//! The termination credit applied to the deposit deductions first (01 Property
//! §4.3 rows "Early termination by the tenant" and "Deductions exceed the
//! deposit", §7.1 *Lease* `terminate` and *Deposit*; BL-PRP-32).
//!
//! When a lease ends early, the paid invoices for periods after the move-out
//! are credited pro rata, and what those credit notes leave is the tenant's
//! credit ([`crate::termination`]). When a deposit is held, `terminate` does
//! not refund or release that credit: it is held for the settlement
//! ([`Lease::credit_held`], with the credit notes it sits on), because the
//! deductions are itemised only after the move-out inspection (BL-PRP-17).
//!
//! When the deposit is settled the credit covers the deductions **before the
//! deposit**, then the rest follows the property manager's choice recorded at
//! `terminate` (`refund`: the Refund flow with the credit note as source,
//! `FIN.ACL.04`; `keep`: it stays on the tenant's account):
//!
//! - **Forfeiture applied to the deductions** ([`crate::deposit_forfeit::apply`]):
//!   the credit pays the unpaid rent invoices first, oldest first
//!   ([`use_on_rent`]); the deposit then covers the rest of the rent and the
//!   damage lines; of the damages the deposit did not cover, the damages
//!   invoice is raised and the credit that is left pays it ([`finish`]), so
//!   the tenant owes only what neither covered.
//! - **Refund or a plain forfeit** (wl-payments, `DepositSettled`,
//!   [`on_deposit_settled`]): the credit pays the unpaid rent invoices, oldest
//!   first, and the rest follows the choice. wl-payments' `refund` takes the
//!   published damage lines from the deposit itself; the kernel offers no way
//!   to tell it that the credit covered them, so on that path the deposit
//!   covers the damage lines and the credit what is left (the tenant gets the
//!   same total back). Decided where the kernel is silent; recorded in the
//!   issue.
//!
//! A credit note pays an invoice in two steps through the kernel's invoicer:
//! its remaining balance is taken for the use (`take_credit_for_refund`, keyed
//! by [`CREDIT_USE_SUBJECT`] and the invoice, idempotent, so the same credit is
//! never used twice) and the amount is allocated to the invoice with the credit
//! note as source. The kernel has no "use a credit note on an invoice" call
//! (wl-invoicing keeps it internal).
//!
//! **Disputed deductions:** nothing is applied while a line of the published
//! deductions is disputed; the settlement waits.
//!
//! The work runs once per lease: [`Lease::credit_settled_on`] is claimed
//! first.

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::deposits::{DepositSettled, deposit_deductions};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{CreditNoteId, CreditNoteStatus, InvoiceId, invoicer};
use wl_kernel::role_tasks::RoleTask;

use crate::deposit_forfeit::unpaid_invoices;
use crate::lease::{LEASE_SUBJECT_TYPE, Lease};
use crate::permissions::ACCOUNTANT_ROLE;
use crate::termination::{Settlement, held_notes_of, settle_credit};
use crate::texts::english;

/// The subject a use of the termination credit on an invoice is recorded
/// under, with the invoice's id: `property.credit_use:12`.
pub const CREDIT_USE_SUBJECT: &str = "property.credit_use";

/// How `credit` fils cover `targets` (each `(invoice, open)`), in order: what
/// goes to each and what is left.
pub fn spread(credit: i64, targets: &[(i64, i64)]) -> (Vec<(i64, i64)>, i64) {
    let mut left = credit.max(0);
    let mut shares = Vec::new();
    for (invoice, open) in targets {
        if left == 0 {
            break;
        }
        let share = left.min((*open).max(0));
        if share > 0 {
            shares.push((*invoice, share));
            left -= share;
        }
    }
    (shares, left)
}

/// What the held credit did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreditUse {
    /// What went to each invoice, in fils.
    pub applied: Vec<(i64, i64)>,
    /// What went to the deductions in all.
    pub to_deductions: i64,
    /// What was asked back through the Refund flow.
    pub refunded: i64,
    /// What stayed on the tenant's account.
    pub kept: i64,
    /// What could not be done, for the accountant.
    pub problems: Vec<String>,
}

/// Whether a line of the published deductions of `lease` is disputed.
///
/// # Errors
///
/// The deposit book's errors.
pub async fn deductions_disputed(state: &AppState, lease: &Lease) -> Result<bool> {
    Ok(deposit_deductions(state)
        .published_for(state, lease.tenant())
        .await?
        .into_iter()
        .any(|shown| shown.source == lease.agreement() && !shown.disputed_lines.is_empty()))
}

/// Claims the held credit of `lease` for this run: `false` when there is none,
/// or another run settled it.
///
/// # Errors
///
/// The database's errors.
pub async fn claim(db: &Db, lease: &Lease) -> Result<bool> {
    if lease.credit_held <= 0 || lease.credit_settled_on.is_some() {
        return Ok(false);
    }
    let claimed = Lease::query()
        .where_(Lease::ID.eq(lease.id))
        .where_(Lease::CREDIT_SETTLED_ON.is_null())
        .update(
            db,
            &[
                ("credit_settled_on", &Some(today())),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(claimed > 0)
}

/// The credit notes of `lease`'s held credit with what each still has.
async fn notes_with_remaining(
    state: &AppState,
    lease: &Lease,
    done: &mut CreditUse,
) -> Vec<(CreditNoteId, i64)> {
    let mut found = Vec::new();
    for note in held_notes_of(lease.credit_notes_held.as_deref()) {
        match invoicer(state).find_credit_note(&state.db, note).await {
            Ok(Some(summary))
                if summary.status == CreditNoteStatus::Issued && summary.remaining > 0 =>
            {
                found.push((note, summary.remaining));
            }
            Ok(_) => {}
            Err(error) => done.problems.push(format!("credit note {note}: {error}")),
        }
    }
    found
}

/// Uses the held credit of `lease` on `targets` (`(invoice, open)`), in order.
async fn use_on(state: &AppState, lease: &Lease, targets: &[(i64, i64)], done: &mut CreditUse) {
    let mut open: Vec<(i64, i64)> = targets.to_vec();
    for (note, remaining) in notes_with_remaining(state, lease, done).await {
        let (shares, _) = spread(remaining, &open);
        for (invoice, amount) in shares {
            let contract = invoicer(state);
            // Taken from the credit note first, so it is never used twice.
            let taken = contract
                .take_credit_for_refund(
                    state,
                    note,
                    Subject::new(CREDIT_USE_SUBJECT, invoice),
                    1,
                    amount,
                )
                .await;
            if let Err(error) = taken {
                done.problems
                    .push(format!("credit note {note} on invoice {invoice}: {error}"));
                continue;
            }
            match contract
                .allocate(state, InvoiceId(invoice), note.subject(), amount)
                .await
            {
                Ok(()) => {
                    done.applied.push((invoice, amount));
                    done.to_deductions += amount;
                    if let Some(target) = open.iter_mut().find(|(id, _)| *id == invoice) {
                        target.1 -= amount;
                    }
                }
                Err(error) => done
                    .problems
                    .push(format!("credit note {note} on invoice {invoice}: {error}")),
            }
        }
    }
}

/// Uses the held credit of `lease` on its unpaid rent invoices, oldest first
/// (the run must have [`claim`]ed it). Returns what was applied.
///
/// # Errors
///
/// The database's and the invoicer's errors.
pub async fn use_on_rent(state: &AppState, lease: &Lease) -> Result<CreditUse> {
    let mut done = CreditUse::default();
    let unpaid: Vec<(i64, i64)> = unpaid_invoices(state, lease.root_id)
        .await?
        .iter()
        .map(|u| (u.id, u.open))
        .collect();
    use_on(state, lease, &unpaid, &mut done).await;
    Ok(done)
}

/// Ends the settlement of the held credit of `lease` (claimed by this run):
/// the credit left pays `damages` (the damages invoice, when one was raised),
/// then the rest is refunded or kept as the property manager chose. `before`
/// is what [`use_on_rent`] did. The figures are stored on the lease; what
/// failed is a task for the accountant.
///
/// # Errors
///
/// The database's errors.
pub async fn finish(
    state: &AppState,
    lease: &Lease,
    before: CreditUse,
    damages: Option<InvoiceId>,
    by: Option<i64>,
) -> Result<CreditUse> {
    let mut done = before;
    let mut waits = false;
    if let Some(invoice) = damages
        && let Some(summary) = invoicer(state).find(&state.db, invoice).await?
        && summary.balance > 0
    {
        if summary.status.is_open() {
            use_on(state, lease, &[(invoice.0, summary.balance)], &mut done).await;
        } else {
            // A damages invoice still in draft: the credit waits for it, it is
            // not released before the deductions are settled.
            waits = true;
            done.problems.push(english(
                "property.credit.damages_waiting",
                &[("invoice", &invoice.0)],
            ));
        }
    }
    // The rest follows the choice recorded at `terminate`.
    if !waits {
        let notes = held_notes_of(lease.credit_notes_held.as_deref());
        let mut rest = Settlement::default();
        settle_credit(state, lease, &notes, by, &mut rest).await;
        done.refunded = rest.refunded;
        done.kept = rest.kept;
        done.problems.extend(rest.problems);
    }
    Lease::query()
        .where_(Lease::ID.eq(lease.id))
        .update(
            &state.db,
            &[
                ("credit_to_deductions", &done.to_deductions),
                ("credit_refunded", &done.refunded),
                ("credit_kept", &done.kept),
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
            &format!("property.credit_problems:{}", lease.id),
        )
        .description(&done.problems.join("\n"));
        crate::tasks::open(state, &lease.scope(), task).await?;
    }
    Ok(done)
}

/// The version of the lease `root_id` that holds a termination credit.
///
/// # Errors
///
/// The database's errors.
pub async fn holder(db: &Db, root_id: i64) -> Result<Option<Lease>> {
    Ok(Lease::versions_of(db, root_id)
        .await?
        .into_iter()
        .find(|lease| lease.credit_held > 0))
}

/// Settles the held credit of the lease `root_id` once its deposit is settled
/// (refunded or forfeited by wl-payments): the credit pays the unpaid rent,
/// then the rest is refunded or kept. Nothing happens while a deduction line
/// is disputed, or when the credit was settled already (a forfeiture applied
/// to the deductions settles it itself). `None` when nothing was done.
///
/// # Errors
///
/// The database's errors.
pub async fn settle_held(
    state: &AppState,
    root_id: i64,
    by: Option<i64>,
) -> Result<Option<CreditUse>> {
    let Some(lease) = holder(&state.db, root_id).await? else {
        return Ok(None);
    };
    if lease.credit_settled_on.is_some() || deductions_disputed(state, &lease).await? {
        return Ok(None);
    }
    if !claim(&state.db, &lease).await? {
        return Ok(None);
    }
    let before = use_on_rent(state, &lease).await?;
    finish(state, &lease, before, None, by).await.map(Some)
}

/// `DepositSettled`: a refunded or forfeited deposit of a lease settles its
/// held credit ([`settle_held`]).
async fn on_deposit_settled(state: &AppState, event: &DepositSettled) -> Result {
    if event.source.subject_type != LEASE_SUBJECT_TYPE {
        return Ok(());
    }
    settle_held(state, event.source.subject_id, None)
        .await
        .map(drop)
}

/// Registers the listener.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: DepositSettled, state: AppState| async move {
        on_deposit_settled(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_credit_covers_the_invoices_in_order_and_says_what_is_left() {
        // AED 3,193.55 over unpaid rent of AED 2,000.
        assert_eq!(
            spread(319_355, &[(4, 200_000)]),
            (vec![(4, 200_000)], 119_355)
        );
        // AED 3,193.55 over rent of AED 4,500: all of it goes to the rent.
        assert_eq!(spread(319_355, &[(4, 450_000)]), (vec![(4, 319_355)], 0));
        // The oldest invoice first, then the next.
        assert_eq!(
            spread(300_000, &[(1, 100_000), (2, 400_000)]),
            (vec![(1, 100_000), (2, 200_000)], 0)
        );
        // Nothing to cover: everything is left.
        assert_eq!(spread(319_355, &[]), (vec![], 319_355));
        assert_eq!(spread(0, &[(1, 100)]), (vec![], 0));
    }
}
