//! Undoing an allocation on an invoice that is not written off
//! (wl-invoicing §4 `unallocate`, #70 A2 to A5).
//!
//! The spec's one action has three targets for an open invoice, and a Renox
//! transition has one `to` (as with `allocate`, #64 A6), so there are three
//! system transitions, each from `paid`, `partially_paid` and `overdue`:
//!
//! - `unallocate` → `issued`: nothing stays allocated;
//! - `unallocate_partially` → `partially_paid`: some money stays allocated;
//! - `unallocate_overdue` → `overdue`: the due date has passed (today in
//!   Asia/Dubai), whatever stays allocated.
//!
//! [`reopen_target`] picks one and each guard re-checks that its target
//! matches. All three are system transitions, so they are never in
//! `available()` and no route runs them. Guards (all three):
//!
//! - the amount is more than 0;
//! - the amount is at most what is allocated to the invoice from the
//!   source (active rows only);
//! - no write-off is on record, so a payment from before a write-off is
//!   refused (the accountant handles it, #70 part 3).
//!
//! The effect, in the transition's transaction: the balance goes up and the
//! allocated total down, as a compare-and-set on the loaded balance (a
//! concurrent change gives 422); the source's active allocation rows are
//! marked removed, oldest first, a row of exactly the amount first, and when
//! the last row is larger it is removed and a new active row keeps the rest
//! ([`plan_removal`]); a row in the [`Unallocation`] log records it. Rows
//! are never deleted, and lines, totals, VAT, number, snapshots and
//! write-off columns are never written.
//!
//! [`run`] is the service: it looks the idempotency key up in the log (a key
//! that is there gives `skipped`), runs the transition, and logs a refusal
//! as `failed` in its own transaction. `InvoiceReopened` is emitted after
//! the commit ([`crate::events`]).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{Outcome, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, ReopenReason, Reopened, Unallocate};

use crate::allocate::amount_text;
use crate::allocation::Allocation;
use crate::invoice::Invoice;
use crate::unallocation::{Unallocation, UnallocationOutcome};

/// Reopens to `issued`.
pub const UNALLOCATE: &str = "unallocate";
/// Reopens to `partially_paid`.
pub const UNALLOCATE_PARTIALLY: &str = "unallocate_partially";
/// Reopens to `overdue`.
pub const UNALLOCATE_OVERDUE: &str = "unallocate_overdue";

/// The three transitions, with the state each one leaves the invoice in.
pub const OPEN_TARGETS: [(&str, InvoiceStatus); 3] = [
    (UNALLOCATE, InvoiceStatus::Issued),
    (UNALLOCATE_PARTIALLY, InvoiceStatus::PartiallyPaid),
    (UNALLOCATE_OVERDUE, InvoiceStatus::Overdue),
];

/// What an unallocation run carries to its guards and effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct UnallocateInput {
    /// The money's source.
    pub source: Subject,
    /// The amount to give back, in fils.
    pub amount: i64,
    /// What is allocated from `source` to the invoice and not removed.
    pub allocated_from_source: i64,
    /// The idempotency key.
    pub key: String,
    /// Why the invoice is reopened.
    pub reason: ReopenReason,
    /// The payment's reversal reason key.
    pub reversal_reason: Option<String>,
    /// The user whose action caused it; `None` for the system.
    pub ran_by: Option<i64>,
}

/// The state an invoice reopens to (#70 A2): `written_off` for a recovery;
/// else `overdue` when the due date is before `today`; else `issued` when
/// nothing stays allocated, else `partially_paid`.
pub fn reopen_target(
    due_date: Option<NaiveDate>,
    today: NaiveDate,
    remaining_allocated: i64,
    is_recovery: bool,
) -> InvoiceStatus {
    if is_recovery {
        InvoiceStatus::WrittenOff
    } else if due_date.is_some_and(|due| due < today) {
        InvoiceStatus::Overdue
    } else if remaining_allocated <= 0 {
        InvoiceStatus::Issued
    } else {
        InvoiceStatus::PartiallyPaid
    }
}

/// The transition that reopens `invoice` after giving back `amount`.
fn transition_for(invoice: &Invoice, amount: i64, on: NaiveDate) -> &'static str {
    let target = reopen_target(
        invoice.due_date,
        on,
        invoice.allocated_total - amount,
        false,
    );
    OPEN_TARGETS
        .iter()
        .find(|(_, to)| *to == target)
        .map_or(UNALLOCATE, |(name, _)| name)
}

/// The `reason` as the text stored in the allocation and the log.
pub(crate) fn reason_key(reason: ReopenReason) -> String {
    match renox::serde_json::to_value(reason) {
        Ok(Value::String(text)) => text,
        _ => String::new(),
    }
}

/// Which allocation rows an unallocation removes, and what it keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    /// The rows to mark removed, in order.
    pub remove: Vec<i64>,
    /// A row id and the amount a new active row keeps, when the last row
    /// removed was larger than what was left to remove.
    pub keep: Option<(i64, i64)>,
}

/// Plans the removal of `amount` from `rows` (id, amount), the source's
/// active rows oldest first: a row of exactly the amount first; else the
/// oldest rows whole, the last one split when it is larger. `None` when the
/// rows hold less than `amount` or `amount` is not above 0.
pub fn plan_removal(rows: &[(i64, i64)], amount: i64) -> Option<Removal> {
    if amount <= 0 {
        return None;
    }
    if let Some((id, _)) = rows.iter().find(|(_, held)| *held == amount) {
        return Some(Removal {
            remove: vec![*id],
            keep: None,
        });
    }
    let mut left = amount;
    let mut removal = Removal {
        remove: Vec::new(),
        keep: None,
    };
    for (id, held) in rows {
        if left == 0 {
            break;
        }
        removal.remove.push(*id);
        if *held <= left {
            left -= held;
        } else {
            removal.keep = Some((*id, held - left));
            left = 0;
        }
    }
    (left == 0).then_some(removal)
}

/// The three transitions, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Invoice>> {
    OPEN_TARGETS
        .iter()
        .map(|&(name, to)| rules(Transition::new(name).label(label(name)).to(to), name, to))
        .collect()
}

/// The label of transition `name`.
fn label(name: &str) -> &'static str {
    match name {
        UNALLOCATE => "Unallocate",
        UNALLOCATE_PARTIALLY => "Unallocate partially",
        _ => "Unallocate (overdue)",
    }
}

/// What the three share: system only, the states that hold money, the
/// guards and the effect.
fn rules(
    transition: Transition<Invoice>,
    name: &'static str,
    target: InvoiceStatus,
) -> Transition<Invoice> {
    transition
        .system()
        .from([
            InvoiceStatus::Paid,
            InvoiceStatus::PartiallyPaid,
            InvoiceStatus::Overdue,
        ])
        .guard("positive_amount", |_invoice: &Invoice, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.amount > 0 => Ok(()),
                    _ => Err("The amount must be more than 0.".to_owned()),
                }
            })
        })
        .guard("within_allocated", |invoice: &Invoice, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.amount <= input.allocated_from_source => Ok(()),
                    Some(input) => Err(format!(
                        "Only {} is allocated from {} to this invoice.",
                        amount_text(&invoice.currency, input.allocated_from_source),
                        input.source
                    )),
                    None => Err("The amount must be more than 0.".to_owned()),
                }
            })
        })
        .guard("no_write_off", |invoice: &Invoice, _ctx| {
            Box::pin(async move {
                if invoice.write_off_amount.is_none() {
                    Ok(())
                } else {
                    Err(
                        "The invoice was written off: the payment is handled by the \
                         accountant."
                            .to_owned(),
                    )
                }
            })
        })
        .guard("target_matches", move |invoice: &Invoice, ctx| {
            Box::pin(async move {
                let amount = input(ctx).map_or(0, |input| input.amount);
                let to = reopen_target(
                    invoice.due_date,
                    today(),
                    invoice.allocated_total - amount,
                    false,
                );
                if to == target {
                    Ok(())
                } else {
                    Err(format!("The invoice reopens as `{}`.", to.as_str()))
                }
            })
        })
        .effect(move |invoice: &Invoice, ctx, tx| {
            let (id, balance) = (invoice.id, invoice.balance);
            Box::pin(async move {
                let input = input(ctx).ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!(
                        "an unallocation run without its input"
                    ))
                })?;
                // Only from the balance this run checked, like `allocate`.
                let moved = renox::db::sql(format!(
                    "UPDATE {} SET allocated_total = allocated_total - ?, balance = balance + ?, \
                     updated_at = ? WHERE id = ? AND balance = ?",
                    Invoice::TABLE
                ))
                .bind(input.amount)
                .bind(input.amount)
                .bind(renox::db::now())
                .bind(id)
                .bind(balance)
                .execute(&mut *tx)
                .await?;
                if moved == 0 {
                    return Err(Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "The invoice's balance changed; try again.".to_owned(),
                    ));
                }
                remove_in(tx, id, &input).await?;
                let mut log = Unallocation {
                    invoice_id: id,
                    source_type: input.source.subject_type.clone(),
                    source_id: input.source.subject_id,
                    amount: input.amount,
                    key: input.key.clone(),
                    reason: input.reason,
                    reversal_reason: input.reversal_reason.clone(),
                    outcome: UnallocationOutcome::Done,
                    transition: Some(name.to_owned()),
                    balance_after: Some(balance + input.amount),
                    ran_by: input.ran_by,
                    ..Default::default()
                };
                log.insert(&mut *tx).await?;
                Ok(())
            })
        })
}

/// Marks the source's allocation rows removed for `input.amount`, and keeps
/// the rest of a split row in a new active one.
async fn remove_in(
    tx: &mut renox::db::Transaction,
    invoice_id: i64,
    input: &UnallocateInput,
) -> Result {
    let rows = active_rows(tx, invoice_id, &input.source).await?;
    let held: Vec<(i64, i64)> = rows.iter().map(|row| (row.id, row.amount)).collect();
    let plan = plan_removal(&held, input.amount).ok_or_else(|| {
        Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            "The allocation changed; try again.".to_owned(),
        )
    })?;
    let reason = reason_key(input.reason);
    for row_id in &plan.remove {
        let marked = renox::db::sql(format!(
            "UPDATE {} SET removed_at = ?, removed_reason = ?, removed_key = ?, updated_at = ? \
             WHERE id = ? AND removed_at IS NULL",
            Allocation::TABLE
        ))
        .bind(renox::db::now())
        .bind(reason.clone())
        .bind(input.key.clone())
        .bind(renox::db::now())
        .bind(*row_id)
        .execute(&mut *tx)
        .await?;
        if marked == 0 {
            return Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                "The allocation changed; try again.".to_owned(),
            ));
        }
    }
    if let Some((row_id, rest)) = plan.keep
        && let Some(split) = rows.iter().find(|row| row.id == row_id)
    {
        let mut kept = Allocation {
            invoice_id,
            source_type: split.source_type.clone(),
            source_id: split.source_id,
            amount: rest,
            allocated_at: split.allocated_at,
            created_by: split.created_by,
            ..Default::default()
        };
        kept.insert(&mut *tx).await?;
    }
    Ok(())
}

/// The active allocation rows of `invoice_id` from `source`, oldest first.
async fn active_rows(
    tx: &mut renox::db::Transaction,
    invoice_id: i64,
    source: &Subject,
) -> Result<Vec<Allocation>> {
    Allocation::query()
        .where_(Allocation::INVOICE_ID.eq(invoice_id))
        .where_(Allocation::SOURCE_TYPE.eq(source.subject_type.clone()))
        .where_(Allocation::SOURCE_ID.eq(source.subject_id))
        .where_(Allocation::REMOVED_AT.is_null())
        .order_by(Allocation::ID)
        .get(&mut *tx)
        .await
}

/// The run's input, if it is an [`UnallocateInput`].
fn input(ctx: &TransitionCtx) -> Option<UnallocateInput> {
    ctx.input_as::<UnallocateInput>().ok()
}

/// The `meta` of the history row, which the `InvoiceReopened` listener
/// reads ([`crate::events`]).
fn meta_of(input: &UnallocateInput) -> Value {
    json!({
        "source": input.source.to_string(),
        "amount": input.amount,
        "key": input.key,
        "reason": reason_key(input.reason),
        "reversal_reason": input.reversal_reason,
        "ran_by": input.ran_by,
    })
}

/// Runs the unallocation of `request` on `invoice` inside `tx`, which the
/// caller commits. Emit the returned outcome after the commit.
///
/// # Errors
///
/// `NotFrom` (422) when the invoice is not `paid`, `partially_paid` or
/// `overdue`; `Guard` (422) for an amount ≤ 0, above what is allocated from
/// the source, or an invoice with a write-off; a 422 when another change
/// moved the balance first. The caller rolls `tx` back.
pub async fn unallocate_in(
    tx: &mut renox::db::Transaction,
    invoice: &Invoice,
    request: &Unallocate,
) -> std::result::Result<Outcome<Invoice>, TransitionError> {
    let allocated_from_source: i64 = active_rows(tx, invoice.id, &request.source)
        .await?
        .iter()
        .map(|row| row.amount)
        .sum();
    let input = UnallocateInput {
        source: request.source.clone(),
        amount: request.amount,
        allocated_from_source,
        key: request.key.clone(),
        reason: request.reason,
        reversal_reason: request.reversal_reason.clone(),
        ran_by: request.ran_by,
    };
    invoice
        .transition(transition_for(invoice, request.amount, today()))
        .by_system()
        .input(&input)
        .meta(meta_of(&input))
        .run_in(tx)
        .await
}

/// A 422 with `message`.
fn refused(message: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.to_owned())
}

/// What a `skipped` call answers: the invoice as it is.
async fn skipped(state: &AppState, id: i64) -> Result<Reopened> {
    let invoice = Invoice::find_or_404(&state.db, id).await?;
    Ok(Reopened {
        new_status: invoice.status,
        balance: invoice.balance,
        skipped: true,
    })
}

/// Logs a refused `request` as `failed`, in its own transaction. A key that
/// is already in the log is left as it is.
async fn log_failed(state: &AppState, request: &Unallocate, message: String) -> Result {
    let mut log = Unallocation {
        invoice_id: request.invoice.0,
        source_type: request.source.subject_type.clone(),
        source_id: request.source.subject_id,
        amount: request.amount,
        key: request.key.clone(),
        reason: request.reason,
        reversal_reason: request.reversal_reason.clone(),
        outcome: UnallocationOutcome::Failed,
        message: Some(message),
        ran_by: request.ran_by,
        ..Default::default()
    };
    match log.insert(&state.db).await {
        Err(error) if !error.is_unique_violation() => Err(error),
        _ => Ok(()),
    }
}

/// Undoes `request.amount` fils of the allocation from `request.source` to
/// `request.invoice`, as the system, in one transaction, and reopens the
/// invoice. Idempotent per `request.key`: a key that is in the log, `done`
/// or `failed`, answers `skipped` and changes nothing. A refusal is logged
/// as `failed` (with its message) and returned as an error.
///
/// # Errors
///
/// 404 for an unknown invoice; 422 for an amount ≤ 0 or above what is
/// allocated from the source, an invoice in a state that cannot be reopened
/// here (`written_off`, `credited`, `issued`, `draft`) or with a write-off
/// on record, a `recovery_reversed` reason (not handled yet), or a balance
/// that changed under the run. Nothing is written then, apart from the
/// `failed` row of a refusal.
pub async fn run(state: &AppState, request: Unallocate) -> Result<Reopened> {
    if Unallocation::by_key(&state.db, &request.key)
        .await?
        .is_some()
    {
        return skipped(state, request.invoice.0).await;
    }
    if request.reason == ReopenReason::RecoveryReversed {
        return Err(refused(
            "Reopening a written-off invoice for a reversed recovery is not supported here.",
        ));
    }
    let Some(invoice) = Invoice::find(&state.db, request.invoice.0).await? else {
        log_failed(state, &request, "The invoice does not exist.".to_owned()).await?;
        return Err(Error::NotFound);
    };
    let mut tx = state.db.begin().await?;
    let outcome = match unallocate_in(&mut tx, &invoice, &request).await {
        Ok(outcome) => outcome,
        Err(error) => {
            drop(tx);
            return match error {
                TransitionError::Failed(error) if error.is_unique_violation() => {
                    skipped(state, invoice.id).await
                }
                TransitionError::Guard(_) | TransitionError::NotFrom(_) => {
                    log_failed(state, &request, error.to_string()).await?;
                    Err(error.into_error())
                }
                other => Err(other.into_error()),
            };
        }
    };
    tx.commit().await?;
    // After the commit: `InvoiceReopened` follows from the listener.
    outcome.emit(state).await?;
    let invoice = Invoice::find_or_404(&state.db, invoice.id).await?;
    Ok(Reopened {
        new_status: invoice.status,
        balance: invoice.balance,
        skipped: false,
    })
}

/// Removes every active allocation of invoice `id`, one source at a time
/// ([`run`] each, keyed on the source's oldest active row), and returns each
/// source with the amount removed. The invoice ends `issued`, or `overdue`
/// when its due date has passed. The caller releases its own side of the
/// money.
///
/// # Errors
///
/// 404 for an unknown invoice; the errors of [`run`]. Sources already
/// removed stay removed; calling again continues with the rest.
pub async fn unallocate_all(
    state: &AppState,
    id: InvoiceId,
    reason: ReopenReason,
) -> Result<Vec<(Subject, i64)>> {
    let invoice = Invoice::find_or_404(&state.db, id.0).await?;
    let rows = Allocation::query()
        .where_(Allocation::INVOICE_ID.eq(invoice.id))
        .where_(Allocation::REMOVED_AT.is_null())
        .order_by(Allocation::ID)
        .get(&state.db)
        .await?;
    let mut sources: Vec<(Subject, i64, i64)> = Vec::new();
    for row in &rows {
        let source = row.source();
        match sources.iter_mut().find(|(s, _, _)| *s == source) {
            Some((_, total, _)) => *total += row.amount,
            None => sources.push((source, row.amount, row.id)),
        }
    }
    let mut removed = Vec::new();
    for (source, amount, first) in sources {
        let key = format!("invoicing.unallocate_all:{}:{first}", invoice.id);
        let request = Unallocate::new(id, source.clone(), amount, key, reason);
        if !run(state, request).await?.skipped {
            removed.push((source, amount));
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 12, d).unwrap()
    }

    fn paid(due: Option<NaiveDate>, allocated: i64) -> Invoice {
        Invoice {
            id: 1,
            currency: "AED".into(),
            gross_total: 200_000,
            allocated_total: allocated,
            balance: 200_000 - allocated,
            due_date: due,
            status: InvoiceStatus::Paid,
            ..Default::default()
        }
    }

    fn run_of(amount: i64, held: i64) -> TransitionCtx {
        let input = UnallocateInput {
            source: Subject::new("payments.payment", 3),
            amount,
            allocated_from_source: held,
            key: "payments.allocation:1".into(),
            reason: ReopenReason::PaymentReversed,
            reversal_reason: Some("bank_return".into()),
            ran_by: None,
        };
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    #[test]
    fn the_target_follows_the_due_date_and_what_stays_allocated() {
        use InvoiceStatus::{Issued, Overdue, PartiallyPaid, WrittenOff};
        // Due on or after today: not overdue.
        assert_eq!(reopen_target(Some(day(10)), day(10), 0, false), Issued);
        assert_eq!(reopen_target(Some(day(31)), day(10), 0, false), Issued);
        assert_eq!(
            reopen_target(Some(day(31)), day(10), 120_000, false),
            PartiallyPaid
        );
        // Due before today: overdue, whatever stays allocated.
        assert_eq!(reopen_target(Some(day(9)), day(10), 0, false), Overdue);
        assert_eq!(
            reopen_target(Some(day(9)), day(10), 120_000, false),
            Overdue
        );
        // No due date: never overdue.
        assert_eq!(reopen_target(None, day(10), 0, false), Issued);
        // A recovery goes back to the write-off.
        assert_eq!(reopen_target(Some(day(9)), day(10), 0, true), WrittenOff);
        assert_eq!(reopen_target(None, day(10), 5, true), WrittenOff);
    }

    #[test]
    fn the_invoice_picks_its_transition() {
        let on = day(10);
        // A payment of 2,000.00 reversed, nothing else allocated.
        assert_eq!(
            transition_for(&paid(Some(day(31)), 200_000), 200_000, on),
            UNALLOCATE
        );
        // Only 800.00 of 2,000.00 reversed.
        assert_eq!(
            transition_for(&paid(Some(day(31)), 200_000), 80_000, on),
            UNALLOCATE_PARTIALLY
        );
        assert_eq!(
            transition_for(&paid(Some(day(5)), 200_000), 80_000, on),
            UNALLOCATE_OVERDUE
        );
    }

    #[test]
    fn they_go_from_the_states_that_hold_money_and_are_system_only() {
        let machine = Invoice::machine();
        for (name, to) in OPEN_TARGETS {
            let transition = machine.find(name).expect("transition");
            assert_eq!(
                transition.from_states(),
                ["paid", "partially_paid", "overdue"]
            );
            assert_eq!(transition.to_state(), Some(to.as_str()));
            assert!(transition.is_system());
            for status in ["draft", "issued", "written_off", "credited"] {
                assert!(transition.check_from(status).is_err());
            }
        }
    }

    #[renox::test]
    async fn the_guards_refuse_nothing_too_much_or_a_write_off() {
        let machine = Invoice::machine();
        let transition = machine.find(UNALLOCATE).expect("unallocate");
        let invoice = paid(Some(day(31)), 200_000);
        for amount in [0, -100] {
            assert!(matches!(
                transition
                    .check_guards(&invoice, &run_of(amount, 200_000))
                    .await,
                Err(TransitionError::Guard(_))
            ));
        }
        match transition
            .check_guards(&invoice, &run_of(90_000, 80_000))
            .await
        {
            Err(TransitionError::Guard(message)) => assert!(
                message.starts_with("Only AED 800.00 is allocated from payments.payment:3"),
                "{message}"
            ),
            other => panic!("expected a guard refusal, got {other:?}"),
        }
        let written_off = Invoice {
            write_off_amount: Some(105_000),
            ..invoice.clone()
        };
        assert!(matches!(
            transition
                .check_guards(&written_off, &run_of(200_000, 200_000))
                .await,
            Err(TransitionError::Guard(_))
        ));
        assert!(matches!(
            transition
                .check_guards(&invoice, &TransitionCtx::new(None))
                .await,
            Err(TransitionError::Guard(_))
        ));
    }

    #[renox::test]
    async fn each_guard_accepts_only_its_own_target() {
        let machine = Invoice::machine();
        let invoice = paid(Some(day(31)), 200_000);
        let ctx = run_of(80_000, 200_000);
        let on = today();
        let target = reopen_target(invoice.due_date, on, 120_000, false);
        for (name, to) in OPEN_TARGETS {
            let ok = machine
                .find(name)
                .expect("transition")
                .check_guards(&invoice, &ctx)
                .await
                .is_ok();
            assert_eq!(ok, to == target, "{name}");
        }
    }

    #[test]
    fn the_removal_takes_an_exact_row_first() {
        let rows = [(1, 120_000), (2, 80_000), (3, 80_000)];
        assert_eq!(
            plan_removal(&rows, 80_000),
            Some(Removal {
                remove: vec![2],
                keep: None
            })
        );
    }

    #[test]
    fn the_removal_takes_the_oldest_rows_and_splits_a_larger_one() {
        let rows = [(1, 50_000), (2, 100_000)];
        assert_eq!(
            plan_removal(&rows, 70_000),
            Some(Removal {
                remove: vec![1, 2],
                keep: Some((2, 80_000))
            })
        );
        assert_eq!(
            plan_removal(&rows, 150_000),
            Some(Removal {
                remove: vec![1, 2],
                keep: None
            })
        );
        // One larger row.
        assert_eq!(
            plan_removal(&[(4, 200_000)], 30_000),
            Some(Removal {
                remove: vec![4],
                keep: Some((4, 170_000))
            })
        );
    }

    #[test]
    fn the_removal_refuses_more_than_is_held() {
        assert_eq!(plan_removal(&[(1, 100)], 101), None);
        assert_eq!(plan_removal(&[], 1), None);
        assert_eq!(plan_removal(&[(1, 100)], 0), None);
    }

    #[test]
    fn the_reason_is_stored_in_snake_case() {
        assert_eq!(
            reason_key(ReopenReason::PaymentReversed),
            "payment_reversed"
        );
        assert_eq!(
            reason_key(ReopenReason::AllocationMoved),
            "allocation_moved"
        );
    }
}
