//! The Supplier debit note state machine (wl-invoicing §4).
//!
//! ```text
//! [*] --> draft --issue--> issued --settle--> settled
//!                          issued --cancel--> cancelled
//! ```
//!
//! All four states are declared here with their tones (`settled` and
//! `cancelled` are terminal), and a new debit note is inserted only as
//! `draft`. `SupplierDebitNote` is `#[model(state_machine)]`, so `save`
//! refuses a changed status. `draft` is a service ([`crate::debit_notes`]),
//! not a transition.
//!
//! `issue` (draft to issued, `supplier_bills.approve`) has a system twin,
//! `issue_system`, for callers with no signed-in user; they name the acting
//! user in the run's input. The machine has no app state, so
//! [`crate::debit_notes`] gathers what the guards need into an
//! [`IssueInput`]. The guards (`FIN.ACL.09`, `FIN.ACL.17`):
//!
//! - the note is linked to an `approved`, `partially_paid` or `paid` bill;
//! - it has lines, none above what is left on its bill line after the
//!   `issued` and `settled` notes;
//! - today is not in a locked VAT period of the receiving entity;
//! - a manual note (no dispatched return) is issued by another person than
//!   its creator.
//!
//! The effect, in the transaction, takes the number from the
//! `supplier_debit_note` series (a rolled-back issue leaves no gap), sets the
//! issue date and the issuer, and splits the gross: what the bill's open
//! balance can take (`applied_to_bill`, applied by the service through the
//! bill's `pay`) and the rest, the supplier's `debit_balance`. The cost
//! reversal, the supplier notice and `SupplierDebitNoteIssued` follow the
//! commit ([`crate::debit_note_events`]).
//!
//! `settle` (issued to settled, `supplier_bills.approve`, system twin
//! `settle_system`) needs the supplier's tax credit note recorded, unless the
//! client's setting accepts the debit note alone, and the whole amount
//! applied: nothing left of the debit balance, or the part the supplier paid
//! short accepted with a reason (the difference). `cancel` (draft or issued to
//! cancelled, `supplier_bills.approve`, system twin `cancel_system`) needs a
//! reason and is refused once the supplier has paid something back. Both
//! need what [`crate::debit_note_settle`] gathers, so only its services run
//! them; the bills it reduced reopen there, in the same transaction.

use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::LegalEntity;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{SupplierDebitNoteStatus, supplier_debit_note_series};
use wl_kernel::machines::SubjectMachine;

use crate::permissions;
use crate::supplier_bill::{SupplierBill, SupplierBillStatus};
use crate::supplier_debit_note::{DebitNoteOrigin, SUBJECT_TYPE, SupplierDebitNote};
use crate::texts::english;

/// The user transition that issues a draft.
pub const ISSUE: &str = "issue";
/// The system twin of [`ISSUE`], for callers with no signed-in user.
pub const ISSUE_SYSTEM: &str = "issue_system";
/// The transition that settles an issued note.
pub const SETTLE: &str = "settle";
/// The system twin of [`SETTLE`].
pub const SETTLE_SYSTEM: &str = "settle_system";
/// The transition that cancels a draft or issued note.
pub const CANCEL: &str = "cancel";
/// The system twin of [`CANCEL`].
pub const CANCEL_SYSTEM: &str = "cancel_system";

/// What an issue run carries to its guards, gathered by
/// [`crate::debit_notes`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct IssueInput {
    /// Whether the linked bill is `approved`, `partially_paid` or `paid`.
    pub bill_debitable: bool,
    /// How many lines the note has.
    pub lines: usize,
    /// Why a line takes more than is left on its bill line.
    pub caps_problem: Option<String>,
    /// Whether today is in a locked VAT period of the receiving entity.
    pub period_locked: bool,
    /// The user a system run acts for.
    pub by: Option<i64>,
}

/// What a settle run carries to its guards, gathered by
/// [`crate::debit_note_settle`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SettleInput {
    /// Whether the supplier's tax credit note must be recorded first.
    pub tax_credit_note_required: bool,
    /// The part of the debit balance the supplier paid short, accepted.
    pub difference: i64,
    /// Why the difference is accepted.
    pub difference_reason: Option<String>,
    /// The user a system run acts for.
    pub by: Option<i64>,
}

/// What a cancel run carries to its guards.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct CancelInput {
    /// What the supplier already paid back, in fils.
    pub refunded: i64,
    /// The user a system run acts for.
    pub by: Option<i64>,
}

/// The run's input, if it is an [`IssueInput`].
fn input(ctx: &TransitionCtx) -> Option<IssueInput> {
    ctx.input_as::<IssueInput>().ok()
}

/// The user running the transition: the signed-in user, else the user a
/// system run acts for.
fn actor_id(ctx: &TransitionCtx) -> Option<i64> {
    ctx.actor()
        .map(|user| user.id)
        .or_else(|| input(ctx).and_then(|i| i.by))
}

/// The user a settle or cancel run acts for.
fn settle_actor(ctx: &TransitionCtx) -> Option<i64> {
    ctx.actor()
        .map(|user| user.id)
        .or_else(|| ctx.input_as::<SettleInput>().ok().and_then(|i| i.by))
}

/// The `settle` and `settle_system` transitions, for the machine.
pub(crate) fn settle_transitions() -> [Transition<SupplierDebitNote>; 2] {
    [
        settle_rules(
            Transition::new(SETTLE)
                .label("Settle")
                .tone("success")
                .permission(permissions::SUPPLIER_BILLS_APPROVE),
        ),
        settle_rules(
            Transition::new(SETTLE_SYSTEM)
                .label("Settle (system)")
                .system(),
        ),
    ]
}

/// What both settle transitions share.
fn settle_rules(transition: Transition<SupplierDebitNote>) -> Transition<SupplierDebitNote> {
    transition
        .from([SupplierDebitNoteStatus::Issued])
        .to(SupplierDebitNoteStatus::Settled)
        .guard("tax_credit_note_recorded", |note: &SupplierDebitNote, ctx| {
            let recorded = note.has_tax_credit_note();
            let input = ctx.input_as::<SettleInput>().ok();
            Box::pin(async move {
                match input {
                    Some(input) if !input.tax_credit_note_required || recorded => Ok(()),
                    Some(_) => {
                        Err(english("wl_invoicing.debit_note.errors.no_tax_credit_note").to_owned())
                    }
                    None => Err(english("wl_invoicing.debit_note.errors.not_checked").to_owned()),
                }
            })
        })
        .guard("fully_applied", |note: &SupplierDebitNote, ctx| {
            let open = note.debit_balance;
            let input = ctx.input_as::<SettleInput>().ok();
            Box::pin(async move {
                let Some(input) = input else {
                    return Err(english("wl_invoicing.debit_note.errors.not_checked").to_owned());
                };
                let reason_given = input
                    .difference_reason
                    .as_deref()
                    .is_some_and(|r| !r.trim().is_empty());
                match (open, input.difference) {
                    (0, 0) => Ok(()),
                    (open, difference) if difference > 0 && difference == open => {
                        if reason_given {
                            Ok(())
                        } else {
                            Err(english("wl_invoicing.debit_note.errors.difference_reason").to_owned())
                        }
                    }
                    _ => Err(english("wl_invoicing.debit_note.errors.not_fully_applied").to_owned()),
                }
            })
        })
        .effect(|note: &SupplierDebitNote, ctx, tx| {
            let id = note.id;
            let by = settle_actor(ctx);
            let input = ctx.input_as::<SettleInput>().ok();
            Box::pin(async move {
                let difference = input.as_ref().map_or(0, |i| i.difference);
                let reason = input
                    .and_then(|i| i.difference_reason)
                    .map(|r| r.trim().to_owned())
                    .filter(|r| !r.is_empty());
                if difference > 0 {
                    crate::debit_note_movement::DebitNoteMovement::new(
                        id,
                        crate::debit_note_movement::DebitMovementKind::Difference,
                        None,
                        difference,
                        by,
                    )
                    .insert(&mut *tx)
                    .await?;
                }
                // Not the status column, so outside the machine's guard.
                SupplierDebitNote::query()
                    .where_(SupplierDebitNote::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("settled_at", &Some(renox::db::now())),
                            ("settled_by", &by),
                            ("settle_difference", &difference),
                            ("settle_difference_reason", &reason),
                            ("debit_balance", &0_i64),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// The `cancel` and `cancel_system` transitions, for the machine.
pub(crate) fn cancel_transitions() -> [Transition<SupplierDebitNote>; 2] {
    [
        cancel_rules(
            Transition::new(CANCEL)
                .label("Cancel")
                .tone("danger")
                .permission(permissions::SUPPLIER_BILLS_APPROVE),
        ),
        cancel_rules(
            Transition::new(CANCEL_SYSTEM)
                .label("Cancel (system)")
                .system(),
        ),
    ]
}

/// What both cancel transitions share.
fn cancel_rules(transition: Transition<SupplierDebitNote>) -> Transition<SupplierDebitNote> {
    transition
        .from([SupplierDebitNoteStatus::Draft, SupplierDebitNoteStatus::Issued])
        .to(SupplierDebitNoteStatus::Cancelled)
        .reason_required()
        .guard("nothing_refunded", |_note: &SupplierDebitNote, ctx| {
            let input = ctx.input_as::<CancelInput>().ok();
            Box::pin(async move {
                match input {
                    Some(input) if input.refunded == 0 => Ok(()),
                    Some(_) => Err(english("wl_invoicing.debit_note.errors.refunded").to_owned()),
                    None => Err(english("wl_invoicing.debit_note.errors.not_checked").to_owned()),
                }
            })
        })
        .effect(|note: &SupplierDebitNote, ctx, tx| {
            let id = note.id;
            let by = ctx
                .actor()
                .map(|user| user.id)
                .or_else(|| ctx.input_as::<CancelInput>().ok().and_then(|i| i.by));
            let reason = ctx
                .reason()
                .map(|r| r.trim().to_owned())
                .filter(|r| !r.is_empty());
            Box::pin(async move {
                SupplierDebitNote::query()
                    .where_(SupplierDebitNote::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("cancelled_at", &Some(renox::db::now())),
                            ("cancelled_by", &by),
                            ("cancel_reason", &reason),
                            ("debit_balance", &0_i64),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// The `issue` and `issue_system` transitions, for the machine.
pub(crate) fn issue_transitions() -> [Transition<SupplierDebitNote>; 2] {
    [
        issue_rules(
            Transition::new(ISSUE)
                .label("Issue")
                .tone("primary")
                .permission(permissions::SUPPLIER_BILLS_APPROVE),
        ),
        issue_rules(
            Transition::new(ISSUE_SYSTEM)
                .label("Issue (system)")
                .system(),
        ),
    ]
}

/// What both issue transitions share: the guards and the effect.
fn issue_rules(transition: Transition<SupplierDebitNote>) -> Transition<SupplierDebitNote> {
    transition
        .from([SupplierDebitNoteStatus::Draft])
        .to(SupplierDebitNoteStatus::Issued)
        .guard("linked_to_bill", |note: &SupplierDebitNote, _ctx| {
            let linked = note.bill_id.is_some();
            Box::pin(async move {
                if linked {
                    Ok(())
                } else {
                    Err("The debit note is waiting for the supplier bill.".to_owned())
                }
            })
        })
        .guard("bill_debitable", |_note: &SupplierDebitNote, ctx| {
            let ok = input(ctx).is_some_and(|i| i.bill_debitable);
            Box::pin(async move {
                if ok {
                    Ok(())
                } else {
                    Err(english("wl_invoicing.debit_note.errors.bill_state").to_owned())
                }
            })
        })
        .guard("has_lines", |_note: &SupplierDebitNote, ctx| {
            let lines = input(ctx).map_or(0, |i| i.lines);
            Box::pin(async move {
                if lines > 0 {
                    Ok(())
                } else {
                    Err(english("wl_invoicing.debit_note.errors.lines").to_owned())
                }
            })
        })
        .guard("within_bill_lines", |_note: &SupplierDebitNote, ctx| {
            let problem = input(ctx).and_then(|i| i.caps_problem);
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .guard("vat_period_open", |_note: &SupplierDebitNote, ctx| {
            let locked = input(ctx).is_none_or(|i| i.period_locked);
            Box::pin(async move {
                if locked {
                    Err(english("wl_invoicing.debit_note.errors.period_locked").to_owned())
                } else {
                    Ok(())
                }
            })
        })
        .guard("second_person", |note: &SupplierDebitNote, ctx| {
            // `FIN.ACL.17`: a manual note is never issued by its creator.
            let creator = note.created_by.filter(|_| note.origin == DebitNoteOrigin::Manual);
            let actor = actor_id(ctx);
            Box::pin(async move {
                match (creator, actor) {
                    (Some(creator), Some(actor)) if creator == actor => {
                        Err(english("wl_invoicing.debit_note.errors.creator").to_owned())
                    }
                    _ => Ok(()),
                }
            })
        })
        .effect(|note: &SupplierDebitNote, ctx, tx| {
            let note = note.clone();
            let by = actor_id(ctx);
            Box::pin(async move { write_issue_in(tx, &note, by).await })
        })
}

/// What `issue` writes besides the status, in the run's transaction: the
/// number from the `supplier_debit_note` series, the issue date and issuer,
/// and the split of the gross between the bill's open balance and the
/// supplier's debit balance.
pub(crate) async fn write_issue_in(
    tx: &mut renox::db::Transaction,
    note: &SupplierDebitNote,
    by: Option<i64>,
) -> Result {
    let bill_id = note.bill_id.ok_or_else(|| {
        Error::from(renox::anyhow::anyhow!(
            "supplier debit note {} has no bill",
            note.id
        ))
    })?;
    let bill = SupplierBill::find_or_404(&mut *tx, bill_id).await?;
    let entity = LegalEntity::find_or_404(&mut *tx, note.legal_entity_id).await?;
    let active = LegalEntity::query()
        .where_(LegalEntity::STATUS.eq("active"))
        .count(&mut *tx)
        .await?;
    let issue_date = today();
    // Inside the issue transaction only: a rollback gives the number back,
    // so the series stays gapless.
    let number = supplier_debit_note_series(usize::try_from(active).unwrap_or(usize::MAX))
        .next(&mut *tx, None, Some(&entity), issue_date)
        .await?;
    // A `paid` bill is left as it is: the whole amount is the supplier's
    // debit balance. An open one takes what its balance can; the rest is the
    // debit balance (§5.13 step 3).
    let open = matches!(
        bill.status,
        SupplierBillStatus::Approved | SupplierBillStatus::PartiallyPaid
    );
    let applied = if open {
        note.gross_total.min(bill.balance.max(0))
    } else {
        0
    };
    let debit_balance = note.gross_total - applied;
    // Not the status column, so outside the machine's guard.
    SupplierDebitNote::query()
        .where_(SupplierDebitNote::ID.eq(note.id))
        .update(
            &mut *tx,
            &[
                ("number", &Some(number)),
                ("issue_date", &Some(issue_date)),
                ("issued_by", &by),
                ("applied_to_bill", &applied),
                ("debit_balance", &debit_balance),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

impl StateMachine for SupplierDebitNote {
    type State = SupplierDebitNoteStatus;
    const COLUMN: renox::db::Col<SupplierDebitNote, SupplierDebitNoteStatus> =
        SupplierDebitNote::STATUS;

    fn machine() -> Machine<Self> {
        use SupplierDebitNoteStatus::{Cancelled, Draft, Issued, Settled};
        let machine = Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Issued)
            .tone("info")
            .state(Settled)
            .tone("success")
            .terminal()
            .state(Cancelled)
            .tone("danger")
            .terminal();
        issue_transitions()
            .into_iter()
            .chain(settle_transitions())
            .chain(cancel_transitions())
            .fold(machine, Machine::transition)
    }
}

/// Registers the machine as the subject `invoicing.supplier_debit_note`
/// and for `rnx state:diagram SupplierDebitNote`.
pub(crate) fn register() {
    SubjectMachine::of::<SupplierDebitNote>(SUBJECT_TYPE).register();
    renox::state_machine::register::<SupplierDebitNote>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::state_machine::TransitionCtx;

    #[test]
    fn the_four_states_and_their_tones() {
        let machine = SupplierDebitNote::machine();
        let states: Vec<_> = machine.states().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(states, ["draft", "issued", "settled", "cancelled"]);
        assert_eq!(machine.initial_states(), ["draft"]);
        let terminal: Vec<_> = machine
            .states()
            .iter()
            .filter(|s| s.terminal)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(terminal, ["settled", "cancelled"]);
        assert_eq!(machine.find(ISSUE).unwrap().from_states().len(), 1);
    }

    #[test]
    fn the_machine_is_registered_without_a_waiting_state() {
        let machine = SubjectMachine::of::<SupplierDebitNote>(SUBJECT_TYPE).build();
        assert_eq!(machine.subject_type(), "invoicing.supplier_debit_note");
        assert_eq!(machine.waiting_state(), None);
        assert!(machine.transition(ISSUE).is_some());
    }

    fn ctx(input: &IssueInput) -> TransitionCtx {
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    fn ready() -> IssueInput {
        IssueInput {
            bill_debitable: true,
            lines: 1,
            by: Some(8),
            ..Default::default()
        }
    }

    async fn refusal(note: &SupplierDebitNote, ctx: &TransitionCtx) -> Option<String> {
        let machine = SupplierDebitNote::machine();
        let issue = machine.find(ISSUE).unwrap();
        issue.check_guards(note, ctx).await.err().map(|e| e.to_string())
    }

    #[renox::test]
    async fn a_note_waiting_for_its_bill_cannot_be_issued() {
        let waiting = SupplierDebitNote::default();
        let refused = refusal(&waiting, &ctx(&ready())).await.unwrap();
        assert!(refused.contains("waiting"));
        let linked = SupplierDebitNote {
            bill_id: Some(3),
            created_by: Some(7),
            ..Default::default()
        };
        assert_eq!(refusal(&linked, &ctx(&ready())).await, None);
    }

    #[renox::test]
    async fn the_guards_read_what_the_service_gathered() {
        let linked = SupplierDebitNote {
            bill_id: Some(3),
            created_by: Some(7),
            ..Default::default()
        };
        let wrong_bill = IssueInput {
            bill_debitable: false,
            ..ready()
        };
        assert!(refusal(&linked, &ctx(&wrong_bill)).await.unwrap().contains("bill"));
        let no_lines = IssueInput { lines: 0, ..ready() };
        assert!(refusal(&linked, &ctx(&no_lines)).await.unwrap().contains("line"));
        let over = IssueInput {
            caps_problem: Some("too much".into()),
            ..ready()
        };
        assert_eq!(refusal(&linked, &ctx(&over)).await.as_deref(), Some("too much"));
        let locked = IssueInput {
            period_locked: true,
            ..ready()
        };
        assert!(refusal(&linked, &ctx(&locked)).await.unwrap().contains("VAT period"));
        // Without the gathered input nothing is issued.
        assert!(refusal(&linked, &TransitionCtx::new(None)).await.is_some());
    }

    #[renox::test]
    async fn the_creator_of_a_manual_note_cannot_issue_it_but_a_return_note_may_be() {
        let manual = SupplierDebitNote {
            bill_id: Some(3),
            created_by: Some(7),
            origin: DebitNoteOrigin::Manual,
            ..Default::default()
        };
        let by_creator = IssueInput {
            by: Some(7),
            ..ready()
        };
        let refused = refusal(&manual, &ctx(&by_creator)).await.unwrap();
        assert!(refused.contains("creator"));
        assert_eq!(refusal(&manual, &ctx(&ready())).await, None);
        let from_return = SupplierDebitNote {
            origin: DebitNoteOrigin::Return,
            ..manual
        };
        assert_eq!(refusal(&from_return, &ctx(&by_creator)).await, None);
    }
}
