//! Writing off an invoice balance as a bad debt (wl-invoicing §4
//! `write_off`, §5.9, `FIN.ACL.15`).
//!
//! `write_off` moves an `issued`, `partially_paid` or `overdue` invoice to
//! `written_off` for a user holding `invoices.write_off`. The machine has no
//! app state, so [`write_off`] gathers its input ([`WriteOffInput`]): the
//! amount (the open balance), the reason code and text, and what the
//! configured reasons say about them. A run without that input (a bare
//! POST) is refused by the guards.
//!
//! The guards, the same on `write_off` and on the kernel's
//! `write_off_approved`: a balance is left; the invoice is not `disputed`;
//! the reason is one of the configured codes and `other` has a text; the
//! amount still equals the balance. `FIN.ACL.15` ([`crate::policy`]) turns
//! every user run above the limit into an approval request (the kernel's
//! interceptor), so the invoice stays where it is until the last approval.
//! The kernel then runs `write_off_approved` with the request's payload as
//! input: when a payment, a credit note or a dispute came in meanwhile, its
//! guards refuse and the request `fail`s.
//!
//! The effect (§5.9, in the transition's transaction): the write-off is
//! stored (amount = balance, reason, approval id, date = today in
//! Asia/Dubai, recovered 0), one pending `bad_debt_relief` VAT adjustment
//! is created for the VAT share of the amount, and the write-off is marked
//! for the accounting export. Lines, totals and VAT are never written.
//! After the commit a listener queues the customer notice
//! ([`crate::write_off_notice`]) and emits `InvoiceWrittenOff`.

use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{
    Outcome, StateMachine, Transition, TransitionCtx, TransitionError, Transitioned,
};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceStatus, InvoiceWrittenOff};

use crate::allocate::amount_text;
use crate::config::{InvoicingConfig, ReasonProblem};
use crate::invoice::Invoice;
use crate::line::InvoiceLine;
use crate::permissions;
use crate::texts::english;
use crate::vat::{VatTotals, bad_debt_vat_share};
use crate::vat_adjustment::VatAdjustment;

/// The transition's name.
pub const WRITE_OFF: &str = "write_off";

/// The transition the kernel runs at final approval.
pub const WRITE_OFF_APPROVED: &str = "write_off_approved";

/// The approval policies' action key of `write_off` (`FIN.ACL.15`).
pub const ACTION: &str = "invoicing.write_off";

/// The states `write_off` leaves.
const FROM: [InvoiceStatus; 3] = [
    InvoiceStatus::Issued,
    InvoiceStatus::PartiallyPaid,
    InvoiceStatus::Overdue,
];

/// What a write-off run carries to its guards and effect, gathered by
/// [`write_off`]; it is also the approval request's payload, which the
/// policy's condition reads (`amount`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WriteOffInput {
    /// The amount to write off: the open balance when requested.
    pub amount: i64,
    /// The reason code, e.g. `insolvent`.
    pub reason_code: String,
    /// The text given with the reason, if any.
    pub reason_text: Option<String>,
    /// Whether the code is one of the configured reasons.
    pub reason_known: bool,
    /// Whether the reason needs a text (`other`).
    pub text_required: bool,
}

impl WriteOffInput {
    /// The input for writing off `amount` with `code` and `text`, checked
    /// against the configured reasons.
    pub fn new(config: &InvoicingConfig, amount: i64, code: &str, text: Option<&str>) -> Self {
        let code = code.trim();
        let reason = config.write_off_reason(code);
        WriteOffInput {
            amount,
            reason_code: code.to_owned(),
            reason_text: text
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_owned),
            reason_known: reason.is_some(),
            text_required: reason.is_some_and(|r| r.text_required),
        }
    }

    /// What is wrong with the reason, if anything.
    fn reason_problem(&self) -> Option<ReasonProblem> {
        if !self.reason_known {
            Some(ReasonProblem::Unknown)
        } else if self.text_required && self.reason_text.is_none() {
            Some(ReasonProblem::TextRequired)
        } else {
            None
        }
    }
}

/// The run's input, if it is a [`WriteOffInput`].
fn input(ctx: &TransitionCtx) -> Option<WriteOffInput> {
    ctx.input_as::<WriteOffInput>().ok()
}

/// The `write_off` and `write_off_approved` transitions, for the machine.
pub(crate) fn transitions() -> [Transition<Invoice>; 2] {
    [
        rules(
            Transition::new(WRITE_OFF)
                .label("Write off")
                .tone("danger")
                .permission(permissions::WRITE_OFF),
        ),
        rules(
            Transition::new(WRITE_OFF_APPROVED)
                .label("Write off (approved)")
                .system(),
        ),
    ]
}

/// What both transitions share: the open states, the guards and the effect.
fn rules(transition: Transition<Invoice>) -> Transition<Invoice> {
    transition
        .from(FROM)
        .to(InvoiceStatus::WrittenOff)
        .guard("has_balance", |invoice: &Invoice, _ctx| {
            Box::pin(async move {
                if invoice.balance > 0 {
                    Ok(())
                } else {
                    Err("Nothing is left to write off.".to_owned())
                }
            })
        })
        .guard("not_disputed", |invoice: &Invoice, _ctx| {
            Box::pin(async move {
                if invoice.disputed {
                    Err("The invoice is disputed.".to_owned())
                } else {
                    Ok(())
                }
            })
        })
        .guard("reason_given", |_invoice: &Invoice, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) => match input.reason_problem() {
                        None => Ok(()),
                        Some(problem) => Err(english(problem.text_key()).to_owned()),
                    },
                    None => Err(english(ReasonProblem::Unknown.text_key()).to_owned()),
                }
            })
        })
        .guard("balance_unchanged", |invoice: &Invoice, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.amount == invoice.balance => Ok(()),
                    Some(input) => Err(format!(
                        "The balance changed from {} to {}; submit the write-off again.",
                        amount_text(&invoice.currency, input.amount),
                        amount_text(&invoice.currency, invoice.balance)
                    )),
                    None => Err("The write-off has no amount.".to_owned()),
                }
            })
        })
        .effect(|invoice: &Invoice, ctx, tx| {
            let invoice = invoice.clone();
            Box::pin(async move {
                let input = input(ctx).ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("a write-off run without its input"))
                })?;
                let approval_id = approval_of(ctx.meta());
                // The VAT share of the balance (§5.9 step 3), from the
                // stored lines, which a write-off never changes.
                let lines = InvoiceLine::query()
                    .where_(InvoiceLine::INVOICE_ID.eq(invoice.id))
                    .get(&mut *tx)
                    .await?;
                let amounts: Vec<_> = lines.iter().map(InvoiceLine::amounts).collect();
                let share = bad_debt_vat_share(invoice.balance, &VatTotals::of(&amounts))
                    .ok_or_else(|| {
                        Error::from(renox::anyhow::anyhow!(
                            "invoice {} has no AED VAT figures",
                            invoice.id
                        ))
                    })?;
                // 1. The write-off; not the status column, so outside the
                // machine's guard. Lines, totals and VAT stay as issued.
                Invoice::query()
                    .where_(Invoice::ID.eq(invoice.id))
                    .update(
                        &mut *tx,
                        &[
                            ("write_off_amount", &Some(invoice.balance)),
                            ("write_off_reason_code", &Some(input.reason_code.clone())),
                            ("write_off_reason_text", &input.reason_text),
                            ("write_off_approval_id", &approval_id),
                            ("write_off_date", &Some(today())),
                            ("write_off_recovered_amount", &0_i64),
                            // 3. For the accounting export (`FIN.EXT.01`, later).
                            ("write_off_export_pending", &true),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                // 2. One pending relief per invoice (a unique index keeps it
                // so).
                VatAdjustment::bad_debt_relief(&invoice, share)
                    .insert(&mut *tx)
                    .await?;
                Ok(())
            })
        })
}

/// The approval request a run's history metadata names, if any.
fn approval_of(meta: &Value) -> Option<i64> {
    meta.get("approval_request").and_then(Value::as_i64)
}

/// The approver a run's history metadata names: the user who gave the last
/// yes; `None` when no approval was needed.
fn approver_of(meta: &Value) -> Option<i64> {
    approval_of(meta).and(meta.get("actor_id").and_then(Value::as_i64))
}

/// Whether `user` is offered the write-off of `invoice` now: the
/// permission, an open state and the guards that read the invoice (a
/// balance, not disputed). `available()` never lists `write_off`, since its
/// other guards need the input the sheet sends.
pub async fn offered(config: &InvoicingConfig, user: &AuthUser, invoice: &Invoice) -> bool {
    let machine = Invoice::machine();
    let Ok(transition) = machine.find(WRITE_OFF) else {
        return false;
    };
    let Some(reason) = config.write_off_reasons.first() else {
        return false;
    };
    let probe = WriteOffInput::new(config, invoice.balance, &reason.code, Some("-"));
    let Ok(probe) = renox::serde_json::to_value(probe) else {
        return false;
    };
    let ctx = TransitionCtx::new(Some(user.clone())).with_input(probe);
    transition.check_access(invoice, Some(user)).is_ok()
        && transition.check_from(invoice.status).is_ok()
        && transition.check_guards(invoice, &ctx).await.is_ok()
}

/// `user` writes off the open balance of `invoice` with `reason_code` (and
/// `reason_text`), in its own transaction. Above the `FIN.ACL.15` limit the
/// kernel defers it as an approval request (the outcome is pending and the
/// invoice stays as it is); the request is announced after the commit.
///
/// # Errors
///
/// `Forbidden` without `invoices.write_off`; `NotFrom` for a `draft`,
/// `paid`, `written_off` or `credited` invoice; `Guard` (422) with no
/// balance, a disputed invoice, an unknown reason or `other` without a
/// text; a 422 when a request for it is already pending. Nothing is written
/// then.
pub async fn write_off(
    state: &AppState,
    user: &AuthUser,
    invoice: &Invoice,
    reason_code: &str,
    reason_text: Option<&str>,
) -> std::result::Result<Outcome<Invoice>, TransitionError> {
    let input = WriteOffInput::new(
        &crate::config_of(state),
        invoice.balance,
        reason_code,
        reason_text,
    );
    let mut run = invoice.transition(WRITE_OFF).by(user).input(&input);
    if !input.reason_code.is_empty() {
        // The history row's reason is the reason code.
        run = run.reason(input.reason_code.clone());
    }
    let outcome = run.run(state).await?;
    wl_kernel::approvals::announce(state, &outcome)
        .await
        .map_err(TransitionError::Failed)?;
    Ok(outcome)
}

/// The event a committed write-off of `invoice` (as it is now) gives.
fn event_for(invoice: &Invoice, meta: &Value) -> InvoiceWrittenOff {
    InvoiceWrittenOff {
        invoice: invoice.invoice_id(),
        party: invoice.party(),
        source: invoice.source(),
        amount: invoice.write_off_amount.unwrap_or_default(),
        reason_code: invoice.write_off_reason_code.clone().unwrap_or_default(),
        approver: approver_of(meta),
    }
}

/// Registers the listener: after a write-off commits, the customer notice
/// is queued (when it gets one) and `InvoiceWrittenOff` is emitted.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Invoice>, state: AppState| async move {
        if ![WRITE_OFF, WRITE_OFF_APPROVED].contains(&event.transition.as_str()) {
            return Ok(());
        }
        // The committed row: the amount and reason the run wrote.
        let invoice = Invoice::find_or_404(&state.db, event.key).await?;
        crate::write_off_notice::queue(&state, &invoice).await?;
        state.emit(event_for(&invoice, &event.meta)).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::serde_json::json;

    fn overdue(balance: i64) -> Invoice {
        Invoice {
            id: 4,
            party_id: 7,
            source_type: "property.lease".into(),
            source_id: 9,
            currency: "AED".into(),
            gross_total: 105_000,
            balance,
            status: InvoiceStatus::Overdue,
            ..Default::default()
        }
    }

    fn run(input: &WriteOffInput) -> TransitionCtx {
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    fn input(amount: i64, code: &str, text: Option<&str>) -> WriteOffInput {
        WriteOffInput::new(&InvoicingConfig::default(), amount, code, text)
    }

    #[test]
    fn write_off_leaves_the_open_states_and_its_approved_run_is_system() {
        let machine = Invoice::machine();
        for (name, system) in [(WRITE_OFF, false), (WRITE_OFF_APPROVED, true)] {
            let transition = machine.find(name).expect("transition");
            assert_eq!(
                transition.from_states(),
                ["issued", "partially_paid", "overdue"]
            );
            assert_eq!(transition.to_state(), Some("written_off"));
            assert_eq!(transition.is_system(), system);
            assert_eq!(
                transition.guard_names(),
                [
                    "has_balance",
                    "not_disputed",
                    "reason_given",
                    "balance_unchanged"
                ]
            );
        }
        assert_eq!(wl_kernel::machines::approved(WRITE_OFF), WRITE_OFF_APPROVED);
    }

    #[test]
    fn the_input_checks_the_reason_against_the_configured_ones() {
        assert_eq!(input(100, " insolvent ", None).reason_problem(), None);
        assert_eq!(
            input(100, "bored", None).reason_problem(),
            Some(ReasonProblem::Unknown)
        );
        assert_eq!(
            input(100, "other", Some("  ")).reason_problem(),
            Some(ReasonProblem::TextRequired)
        );
        let other = input(100, "other", Some(" Moved abroad "));
        assert_eq!(other.reason_problem(), None);
        assert_eq!(other.reason_text.as_deref(), Some("Moved abroad"));
        // A payload without the checks counts as an unknown reason.
        assert_eq!(
            WriteOffInput::default().reason_problem(),
            Some(ReasonProblem::Unknown)
        );
    }

    #[renox::test]
    async fn the_guards_pass_a_write_off_of_the_balance() {
        let machine = Invoice::machine();
        for name in [WRITE_OFF, WRITE_OFF_APPROVED] {
            let transition = machine.find(name).expect("transition");
            let ok = run(&input(105_000, "insolvent", None));
            assert!(
                transition
                    .check_guards(&overdue(105_000), &ok)
                    .await
                    .is_ok()
            );
        }
    }

    #[renox::test]
    async fn the_guards_refuse_a_changed_balance_a_dispute_or_no_reason() {
        let machine = Invoice::machine();
        let transition = machine.find(WRITE_OFF_APPROVED).expect("transition");
        let refusal = |invoice: Invoice, ctx: TransitionCtx| async move {
            match transition.check_guards(&invoice, &ctx).await {
                Err(TransitionError::Guard(message)) => message,
                other => panic!("expected a guard refusal, got {other:?}"),
            }
        };
        let requested = input(105_000, "insolvent", None);
        assert_eq!(
            refusal(overdue(95_000), run(&requested)).await,
            "The balance changed from AED 1,050.00 to AED 950.00; submit the write-off again."
        );
        let disputed = Invoice {
            disputed: true,
            ..overdue(105_000)
        };
        assert_eq!(
            refusal(disputed, run(&requested)).await,
            "The invoice is disputed."
        );
        assert_eq!(
            refusal(overdue(0), run(&input(0, "insolvent", None))).await,
            "Nothing is left to write off."
        );
        assert_eq!(
            refusal(overdue(105_000), run(&input(105_000, "other", None))).await,
            "This reason needs a text."
        );
        assert_eq!(
            refusal(overdue(105_000), run(&input(105_000, "bored", None))).await,
            "Choose one of the write-off reasons."
        );
        // A bare run, without the gathered input.
        assert!(matches!(
            transition
                .check_guards(&overdue(105_000), &TransitionCtx::new(None))
                .await,
            Err(TransitionError::Guard(_))
        ));
    }

    #[test]
    fn the_event_names_the_approver_only_after_an_approval() {
        let invoice = Invoice {
            write_off_amount: Some(105_000),
            write_off_reason_code: Some("insolvent".into()),
            ..overdue(105_000)
        };
        let approved = event_for(
            &invoice,
            &json!({ "approval_request": 9, "actor_id": 3, "requester_id": 2 }),
        );
        assert_eq!(approved.amount, 105_000);
        assert_eq!(approved.reason_code, "insolvent");
        assert_eq!(approved.approver, Some(3));
        assert_eq!(approved.source.to_string(), "property.lease:9");
        assert_eq!(event_for(&invoice, &Value::Null).approver, None);
        assert_eq!(approval_of(&json!({ "approval_request": 9 })), Some(9));
    }
}
