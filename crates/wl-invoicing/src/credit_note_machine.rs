//! The Credit note state machine (wl-invoicing §4).
//!
//! ```text
//! [*] --> draft --issue--> issued --apply--> applied
//!                          applied --reopen--> issued
//! ```
//!
//! The three states are declared here with their tones, and a new credit
//! note is inserted only as `draft`. There is no terminal state: `applied`
//! is left by `reopen`. The transitions are added by the issues that build
//! them, all in this `machine()`: `issue`, `apply` and `reopen`.
//! `CreditNote` is `#[model(state_machine)]`, so `save` refuses a changed
//! status.
//!
//! `issue` (draft to issued, `invoices.credit`) has a system twin,
//! `issue_system`, for callers with no signed-in user (the kernel
//! `Invoicer`): a Renox system run accepts only `system` transitions. The
//! machine has no app state, so [`crate::credit_notes`] gathers what the
//! guards need into an [`IssueInput`]. The effect, in the transaction,
//! takes the number from the `credit_note` series (so a rolled-back issue
//! leaves no gap), the issuer and party snapshots and the issue date, and
//! sets the remaining to the gross. `apply` (issued to applied, system)
//! needs the remaining to be 0. `CreditNoteIssued` is emitted after the
//! commit ([`crate::events`]). `reopen` (applied to issued, system) gives
//! back what a failed refund or a removed allocation had used
//! ([`crate::restore`]): its effect writes the ledger row and raises the
//! remaining, and `CreditNoteReopened` follows the commit.

use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::LegalEntity;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{CreditNoteStatus, InvoiceStatus, credit_note_series};

use crate::credit_note::CreditNote;
use crate::invoice::Invoice;
use crate::permissions;
use crate::restore::{self, RestoreInput};

/// The user transition that issues a draft.
pub const ISSUE: &str = "issue";
/// The system twin of [`ISSUE`], for callers with no signed-in user.
pub const ISSUE_SYSTEM: &str = "issue_system";
/// The system transition the kernel runs when a dispute credit is approved
/// (`FIN.ACL.07`).
pub const ISSUE_APPROVED: &str = "issue_approved";
/// The transition that applies a fully used credit note.
pub const APPLY: &str = "apply";
/// The transition that reopens an applied credit note when a use of it is
/// undone.
pub const REOPEN: &str = "reopen";

/// The invoice states a credit note can be issued against.
const CREDITABLE: [InvoiceStatus; 4] = [
    InvoiceStatus::Issued,
    InvoiceStatus::PartiallyPaid,
    InvoiceStatus::Overdue,
    InvoiceStatus::Paid,
];

/// What an issue run carries to its guards, gathered by
/// [`crate::credit_notes`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct IssueInput {
    /// The linked invoice's status, when it is found.
    pub invoice_status: Option<InvoiceStatus>,
    /// The linked invoice's issuing legal entity.
    pub invoice_entity: Option<i64>,
    /// Whether that entity is found and active.
    pub entity_active: bool,
    /// How many lines the credit note has.
    pub lines: usize,
    /// Why a line cannot be credited (above its remaining credit, say).
    pub line_problem: Option<String>,
}

/// The run's input, if it is an [`IssueInput`].
fn input(ctx: &TransitionCtx) -> Option<IssueInput> {
    ctx.input_as::<IssueInput>().ok()
}

/// The `issue` and `issue_system` transitions, for the machine.
pub(crate) fn issue_transitions() -> [Transition<CreditNote>; 2] {
    [
        issue_rules(
            Transition::new(ISSUE)
                .label("Issue")
                .tone("primary")
                .permission(permissions::CREDIT),
        ),
        issue_rules(
            Transition::new(ISSUE_SYSTEM)
                .label("Issue (system)")
                .system(),
        ),
    ]
}

/// What both issue transitions share: the guards and the effect.
fn issue_rules(transition: Transition<CreditNote>) -> Transition<CreditNote> {
    transition
        .from([CreditNoteStatus::Draft])
        .to(CreditNoteStatus::Issued)
        .guard("invoice_creditable", |_note: &CreditNote, ctx| {
            Box::pin(async move {
                match input(ctx).and_then(|i| i.invoice_status) {
                    Some(status) if CREDITABLE.contains(&status) => Ok(()),
                    Some(status) => Err(format!("An invoice that is {status} cannot be credited.")),
                    None => Err("The credited invoice was not found.".to_owned()),
                }
            })
        })
        .guard("reason_given", |note: &CreditNote, _ctx| {
            let given = !note.reason.trim().is_empty();
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err("Give a reason for the credit note.".to_owned())
                }
            })
        })
        .guard("has_lines", |_note: &CreditNote, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.lines > 0 => Ok(()),
                    _ => Err("A credit note needs at least one line.".to_owned()),
                }
            })
        })
        .guard("within_remaining_credit", |_note: &CreditNote, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(IssueInput {
                        line_problem: Some(problem),
                        ..
                    }) => Err(problem),
                    Some(_) => Ok(()),
                    None => Err("The credit note could not be checked.".to_owned()),
                }
            })
        })
        .guard("invoice_entity_active", |note: &CreditNote, ctx| {
            let entity = note.legal_entity_id;
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if entity.is_some() && entity == input.invoice_entity => {
                        if input.entity_active {
                            Ok(())
                        } else {
                            Err("The issuing legal entity is not active.".to_owned())
                        }
                    }
                    _ => Err(
                        "The credit note must be issued by the invoice's legal entity.".to_owned(),
                    ),
                }
            })
        })
        .effect(|note: &CreditNote, _ctx, tx| {
            let note = note.clone();
            Box::pin(async move { write_issue_in(tx, &note).await })
        })
}

/// What `issue` writes besides the status, in the run's transaction: the
/// number from the `credit_note` series (so a rolled-back issue leaves no
/// gap), the issuer and party snapshots, the issue date, and the remaining
/// set to the gross.
pub(crate) async fn write_issue_in(tx: &mut renox::db::Transaction, note: &CreditNote) -> Result {
    let (id, invoice_id, entity_id, gross) = (
        note.id,
        note.invoice_id,
        note.legal_entity_id,
        note.gross_total,
    );
    let invoice = Invoice::find_or_404(&mut *tx, invoice_id).await?;
    let entity_id = entity_id.ok_or_else(|| {
        Error::from(renox::anyhow::anyhow!(
            "credit note {id} has no issuing legal entity"
        ))
    })?;
    let entity = LegalEntity::find_or_404(&mut *tx, entity_id).await?;
    let active = LegalEntity::query()
        .where_(LegalEntity::STATUS.eq("active"))
        .count(&mut *tx)
        .await?;
    let issue_date = today();
    // Inside the issue transaction only: a rollback gives the number back,
    // so the series stays gapless.
    let number = credit_note_series(usize::try_from(active).unwrap_or(usize::MAX))
        .next(&mut *tx, None, Some(&entity), issue_date)
        .await?;
    let issuer = renox::db::Json(entity.snapshot());
    // The invoice's billing identity, as it was at its issue.
    let party = invoice.party_snapshot.clone();
    // Not the status column, so outside the machine's guard.
    CreditNote::query()
        .where_(CreditNote::ID.eq(id))
        .update(
            &mut *tx,
            &[
                ("number", &number),
                ("issuer_snapshot", &issuer),
                ("party_snapshot", &party),
                ("issue_date", &issue_date),
                ("remaining", &gross),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// The `issue_approved` transition, for the machine: the kernel runs it at
/// the final approval of a dispute credit (`FIN.ACL.07`), with the request's
/// payload as input and the approval in `meta`. The guards read the note and
/// the payload; the stored invoice, lines and dispute are checked again in
/// the effect, which then issues, allocates and settles the dispute
/// ([`crate::dispute_credit::approved_in`]).
pub(crate) fn approved_transition() -> Transition<CreditNote> {
    Transition::new(ISSUE_APPROVED)
        .label("Issue (approved)")
        .system()
        .from([CreditNoteStatus::Draft])
        .to(CreditNoteStatus::Issued)
        .guard("reason_given", |note: &CreditNote, _ctx| {
            let given = !note.reason.trim().is_empty();
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err("Give a reason for the credit note.".to_owned())
                }
            })
        })
        .guard("amount_unchanged", |note: &CreditNote, ctx| {
            let gross = note.gross_total;
            let amount = ctx.input().get("amount").and_then(|v| v.as_i64());
            Box::pin(async move {
                if amount == Some(gross) {
                    Ok(())
                } else {
                    Err("The credit note changed since it was submitted.".to_owned())
                }
            })
        })
        .effect(|note: &CreditNote, ctx, tx| {
            let note = note.clone();
            let (input, meta) = (ctx.input().clone(), ctx.meta().clone());
            Box::pin(
                async move { crate::dispute_credit::approved_in(tx, &note, &input, &meta).await },
            )
        })
}

/// The `apply` transition, for the machine.
pub(crate) fn apply_transition() -> Transition<CreditNote> {
    Transition::new(APPLY)
        .label("Apply")
        .system()
        .from([CreditNoteStatus::Issued])
        .to(CreditNoteStatus::Applied)
        .guard("fully_used", |note: &CreditNote, _ctx| {
            let remaining = note.remaining;
            Box::pin(async move {
                if remaining == 0 {
                    Ok(())
                } else {
                    Err("The credit note still has a remaining balance.".to_owned())
                }
            })
        })
}

/// The `reopen` transition, for the machine.
pub(crate) fn reopen_transition() -> Transition<CreditNote> {
    Transition::new(REOPEN)
        .label("Reopen")
        .system()
        .from([CreditNoteStatus::Applied])
        .to(CreditNoteStatus::Issued)
        .guard("use_undone", |note: &CreditNote, ctx| {
            let room = note.gross_total - note.remaining;
            Box::pin(async move {
                match ctx.input_as::<RestoreInput>() {
                    Ok(input) if input.amount > 0 && input.amount <= room => Ok(()),
                    _ => Err("There is nothing to give back to the credit note.".to_owned()),
                }
            })
        })
        .effect(|note: &CreditNote, ctx, tx| {
            let note = note.clone();
            let input = ctx.input_as::<RestoreInput>().ok();
            Box::pin(async move {
                // The guard passed, so the input is there.
                let input = input.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("a reopen run without its input"))
                })?;
                restore::write_in(tx, &note, &input).await
            })
        })
}

impl StateMachine for CreditNote {
    type State = CreditNoteStatus;
    const COLUMN: renox::db::Col<CreditNote, CreditNoteStatus> = CreditNote::STATUS;

    fn machine() -> Machine<Self> {
        use CreditNoteStatus::{Applied, Draft, Issued};
        let machine = Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Issued)
            .tone("info")
            .state(Applied)
            .tone("success")
            .transition(apply_transition())
            .transition(reopen_transition())
            .transition(approved_transition());
        issue_transitions()
            .into_iter()
            .fold(machine, Machine::transition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(input: &IssueInput) -> TransitionCtx {
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    fn ready() -> IssueInput {
        IssueInput {
            invoice_status: Some(InvoiceStatus::Issued),
            invoice_entity: Some(3),
            entity_active: true,
            lines: 1,
            line_problem: None,
        }
    }

    fn draft() -> CreditNote {
        CreditNote {
            legal_entity_id: Some(3),
            reason: "Damaged goods".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_credit_note_starts_as_a_draft() {
        let machine = CreditNote::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        assert_eq!(machine.states().len(), 3);
    }

    #[test]
    fn issue_and_apply_have_their_states_and_access() {
        let machine = CreditNote::machine();
        for name in [ISSUE, ISSUE_SYSTEM] {
            let issue = machine.find(name).expect("issue");
            assert_eq!(issue.from_states(), ["draft"]);
            assert_eq!(issue.to_state(), Some("issued"));
            assert_eq!(issue.is_system(), name == ISSUE_SYSTEM);
            assert_eq!(
                issue.guard_names(),
                [
                    "invoice_creditable",
                    "reason_given",
                    "has_lines",
                    "within_remaining_credit",
                    "invoice_entity_active"
                ]
            );
        }
        let apply = machine.find(APPLY).expect("apply");
        assert_eq!(apply.from_states(), ["issued"]);
        assert_eq!(apply.to_state(), Some("applied"));
        assert!(apply.is_system());
        let reopen = machine.find(REOPEN).expect("reopen");
        assert_eq!(reopen.from_states(), ["applied"]);
        assert_eq!(reopen.to_state(), Some("issued"));
        assert!(reopen.is_system());
        assert_eq!(reopen.guard_names(), ["use_undone"]);
    }

    #[renox::test]
    async fn issue_approved_is_the_systems_and_checks_the_payload_amount() {
        let machine = CreditNote::machine();
        let approved = machine.find(ISSUE_APPROVED).expect("issue_approved");
        assert_eq!(approved.from_states(), ["draft"]);
        assert_eq!(approved.to_state(), Some("issued"));
        assert!(approved.is_system());
        assert_eq!(approved.guard_names(), ["reason_given", "amount_unchanged"]);
        assert_eq!(
            wl_kernel::machines::approved(ISSUE),
            ISSUE_APPROVED,
            "the kernel runs it by this name"
        );
        let note = CreditNote {
            gross_total: 120_000,
            ..draft()
        };
        let with = |amount: i64| {
            TransitionCtx::new(None).with_input(renox::serde_json::json!({ "amount": amount }))
        };
        assert!(approved.check_guards(&note, &with(120_000)).await.is_ok());
        assert!(approved.check_guards(&note, &with(119_999)).await.is_err());
        assert!(
            approved
                .check_guards(&note, &TransitionCtx::new(None))
                .await
                .is_err()
        );
    }

    #[renox::test]
    async fn a_ready_draft_passes_the_issue_guards() {
        let machine = CreditNote::machine();
        let issue = machine.find(ISSUE).expect("issue");
        assert!(issue.check_guards(&draft(), &run(&ready())).await.is_ok());
        for status in CREDITABLE {
            let input = IssueInput {
                invoice_status: Some(status),
                ..ready()
            };
            assert!(issue.check_guards(&draft(), &run(&input)).await.is_ok());
        }
    }

    #[renox::test]
    async fn the_issue_guards_refuse_what_cannot_be_credited() {
        let machine = CreditNote::machine();
        let issue = machine.find(ISSUE).expect("issue");
        let refused = |note: CreditNote, input: IssueInput| {
            let ctx = run(&input);
            let issue = &issue;
            async move {
                matches!(
                    issue.check_guards(&note, &ctx).await,
                    Err(renox::state_machine::TransitionError::Guard(_))
                )
            }
        };
        for status in [
            InvoiceStatus::Draft,
            InvoiceStatus::Credited,
            InvoiceStatus::WrittenOff,
        ] {
            let input = IssueInput {
                invoice_status: Some(status),
                ..ready()
            };
            assert!(refused(draft(), input).await);
        }
        let missing = IssueInput {
            invoice_status: None,
            ..ready()
        };
        assert!(refused(draft(), missing).await);
        let blank = CreditNote {
            reason: "  ".into(),
            ..draft()
        };
        assert!(refused(blank, ready()).await);
        let no_lines = IssueInput {
            lines: 0,
            ..ready()
        };
        assert!(refused(draft(), no_lines).await);
        let over = IssueInput {
            line_problem: Some("line 1 can be credited by 100 fils at most".into()),
            ..ready()
        };
        assert!(refused(draft(), over).await);
        let inactive = IssueInput {
            entity_active: false,
            ..ready()
        };
        assert!(refused(draft(), inactive).await);
        let other_entity = CreditNote {
            legal_entity_id: Some(4),
            ..draft()
        };
        assert!(refused(other_entity, ready()).await);
        let no_entity = CreditNote {
            legal_entity_id: None,
            ..draft()
        };
        assert!(refused(no_entity, ready()).await);
        let bare = TransitionCtx::new(None);
        assert!(issue.check_guards(&draft(), &bare).await.is_err());
    }

    #[renox::test]
    async fn apply_needs_the_remaining_to_be_used_up() {
        let machine = CreditNote::machine();
        let apply = machine.find(APPLY).expect("apply");
        let ctx = TransitionCtx::new(None);
        let used = CreditNote {
            remaining: 0,
            status: CreditNoteStatus::Issued,
            ..Default::default()
        };
        assert!(apply.check_guards(&used, &ctx).await.is_ok());
        let open = CreditNote {
            remaining: 100,
            ..used
        };
        assert!(apply.check_guards(&open, &ctx).await.is_err());
    }

    #[renox::test]
    async fn reopen_needs_an_amount_that_fits_in_the_gross() {
        let machine = CreditNote::machine();
        let reopen = machine.find(REOPEN).expect("reopen");
        let applied = CreditNote {
            gross_total: 50_000,
            remaining: 0,
            status: CreditNoteStatus::Applied,
            ..Default::default()
        };
        let with = |amount: i64| {
            let input = RestoreInput {
                cause: restore::Cause::RefundFailed,
                target: wl_kernel::Subject::new("payments.refund", 9),
                attempt: Some(1),
                amount,
                ran_by: None,
            };
            TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
        };
        assert!(reopen.check_guards(&applied, &with(50_000)).await.is_ok());
        for amount in [0, -1, 50_001] {
            assert!(reopen.check_guards(&applied, &with(amount)).await.is_err());
        }
        let bare = TransitionCtx::new(None);
        assert!(reopen.check_guards(&applied, &bare).await.is_err());
    }
}
