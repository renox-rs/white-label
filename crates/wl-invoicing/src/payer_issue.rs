//! Issuing a payer bill and sending it again (wl-invoicing §4 *Payer bill*
//! `issue` and `resubmit`).
//!
//! `issue` moves a `draft` payer bill to `issued` for a user holding
//! `payer_bills.issue`, or for the system on behalf of a vertical (05 claim
//! `submit`). The machine has no app state, so [`issue`] gathers what the
//! guards need into an [`IssueInput`]. The guards: the payer agreement is
//! active; the payer reference is set where the agreement requires it; every
//! line has a claimed amount; and, when the payer is the VAT recipient, the
//! payer's tax invoice would pass the guards of the invoice's own `issue`.
//!
//! The effect, in the transition's transaction: the number from the entity's
//! `payer_bill` series at the first issue (a bill issued again after a
//! technical rejection keeps it), the issue count, the receivable back in
//! the ageing, and the flag that a tax document is still to be made
//! ([`crate::payer_tax`]). After the commit a listener makes the tax
//! document and emits `PayerBillIssued`.
//!
//! `resubmit` sends a `partially_paid` or `denied` bill again once the
//! corrections are written down, within the agreement's resubmission window
//! and limit: the resubmission count goes up by one and the denied amounts
//! are no longer denied (they are asked again).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::LegalEntity;
use wl_kernel::documents::today;
use wl_kernel::payer_bills::{PayerBillIssued, PayerBillStatus, payer_bill_series};

use crate::credit_notes::{By, run_of};
use crate::payer_agreement::PayerAgreement;
use crate::payer_bill::{PayerBill, PayerBillLine, lines_of, write_totals_in};
use crate::payer_tax;
use crate::permissions;
use crate::texts::english;

/// The user transition that issues a draft.
pub const ISSUE: &str = "issue";
/// The system twin of [`ISSUE`], for a vertical that submits by itself.
pub const ISSUE_SYSTEM: &str = "issue_system";
/// The user transition that sends a bill with open lines again.
pub const RESUBMIT: &str = "resubmit";
/// The system twin of [`RESUBMIT`].
pub const RESUBMIT_SYSTEM: &str = "resubmit_system";

/// What an issue or resubmit run carries to its guards, gathered by
/// [`issue`] and [`resubmit`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct IssueInput {
    /// Whether the payer agreement is active.
    pub agreement_active: bool,
    /// Whether the payer reference is set where the agreement requires it.
    pub reference_given: bool,
    /// How many lines the bill has.
    pub lines: usize,
    /// Whether every line has a claimed amount above 0.
    pub all_claimed: bool,
    /// Why the payer's tax invoice would not pass the invoice's `issue`.
    pub document_problem: Option<String>,
    /// The corrections written down for a resubmission.
    pub corrections: Option<String>,
    /// Whether today is within the agreement's resubmission window.
    pub within_window: bool,
    /// Whether the agreement's resubmission limit is not reached.
    pub within_limit: bool,
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

/// The `issue`, `issue_system`, `resubmit` and `resubmit_system`
/// transitions, for the machine.
pub(crate) fn transitions() -> Vec<Transition<PayerBill>> {
    vec![
        issue_rules(
            Transition::new(ISSUE)
                .label("Issue")
                .tone("primary")
                .permission(permissions::PAYER_BILLS_ISSUE),
        ),
        issue_rules(
            Transition::new(ISSUE_SYSTEM)
                .label("Issue (system)")
                .system(),
        ),
        resubmit_rules(
            Transition::new(RESUBMIT)
                .label("Resubmit")
                .tone("primary")
                .permission(permissions::PAYER_BILLS_ISSUE),
        ),
        resubmit_rules(
            Transition::new(RESUBMIT_SYSTEM)
                .label("Resubmit (system)")
                .system(),
        ),
    ]
}

/// What both issue transitions share: the guards and the effect.
fn issue_rules(transition: Transition<PayerBill>) -> Transition<PayerBill> {
    transition
        .from([PayerBillStatus::Draft])
        .to(PayerBillStatus::Issued)
        .guard("agreement_active", |_bill: &PayerBill, ctx| {
            let ok = input(ctx).is_some_and(|i| i.agreement_active);
            Box::pin(async move {
                if ok {
                    Ok(())
                } else {
                    Err(english("wl_invoicing.payer_bill.errors.agreement_inactive").to_owned())
                }
            })
        })
        .guard("reference_given", |_bill: &PayerBill, ctx| {
            let ok = input(ctx).is_some_and(|i| i.reference_given);
            Box::pin(async move {
                if ok {
                    Ok(())
                } else {
                    Err(english("wl_invoicing.payer_bill.errors.reference").to_owned())
                }
            })
        })
        .guard("lines_claimed", |_bill: &PayerBill, ctx| {
            let ok = input(ctx).is_some_and(|i| i.lines > 0 && i.all_claimed);
            Box::pin(async move {
                if ok {
                    Ok(())
                } else {
                    Err(english("wl_invoicing.payer_bill.errors.lines").to_owned())
                }
            })
        })
        .guard("payer_invoice_valid", |_bill: &PayerBill, ctx| {
            let problem = input(ctx).and_then(|i| i.document_problem);
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|bill: &PayerBill, ctx, tx| {
            let bill = bill.clone();
            let by = actor_id(ctx);
            Box::pin(async move { write_issue_in(tx, &bill, by).await })
        })
}

/// What both resubmit transitions share: the guards and the effect.
fn resubmit_rules(transition: Transition<PayerBill>) -> Transition<PayerBill> {
    transition
        .from([PayerBillStatus::PartiallyPaid, PayerBillStatus::Denied])
        .to(PayerBillStatus::Issued)
        .guard("corrections_recorded", |_bill: &PayerBill, ctx| {
            let ok = input(ctx)
                .and_then(|i| i.corrections)
                .is_some_and(|c| !c.trim().is_empty());
            Box::pin(async move {
                if ok {
                    Ok(())
                } else {
                    Err(english("wl_invoicing.payer_bill.errors.corrections").to_owned())
                }
            })
        })
        .guard("within_resubmission_window", |_bill: &PayerBill, ctx| {
            let ok = input(ctx).is_some_and(|i| i.within_window);
            Box::pin(async move {
                if ok {
                    Ok(())
                } else {
                    Err(english("wl_invoicing.payer_bill.errors.resubmit_window").to_owned())
                }
            })
        })
        .guard("within_resubmission_limit", |_bill: &PayerBill, ctx| {
            let ok = input(ctx).is_some_and(|i| i.within_limit);
            Box::pin(async move {
                if ok {
                    Ok(())
                } else {
                    Err(english("wl_invoicing.payer_bill.errors.resubmit_limit").to_owned())
                }
            })
        })
        .guard("payer_invoice_valid", |_bill: &PayerBill, ctx| {
            let problem = input(ctx).and_then(|i| i.document_problem);
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|bill: &PayerBill, ctx, tx| {
            let bill = bill.clone();
            let by = actor_id(ctx);
            Box::pin(async move { write_resubmit_in(tx, &bill, by).await })
        })
}

/// What `issue` writes besides the status, in the run's transaction: the
/// number (kept when the bill has one), the issue count, the ageing flag and
/// whether a tax document is still to be made.
async fn write_issue_in(
    tx: &mut renox::db::Transaction,
    bill: &PayerBill,
    _by: Option<i64>,
) -> Result {
    let number = match &bill.number {
        Some(number) => number.clone(),
        None => {
            let entity = LegalEntity::find_or_404(&mut *tx, bill.legal_entity_id).await?;
            let active = LegalEntity::query()
                .where_(LegalEntity::STATUS.eq("active"))
                .count(&mut *tx)
                .await?;
            // Inside the issue transaction only: a rollback gives the number
            // back, so the series stays gapless.
            payer_bill_series(usize::try_from(active).unwrap_or(usize::MAX))
                .next(&mut *tx, None, Some(&entity), today())
                .await?
        }
    };
    let lines = lines_of(&mut *tx, bill.id).await?;
    let pending = payer_tax::needs_documents(bill, &lines);
    PayerBill::query()
        .where_(PayerBill::ID.eq(bill.id))
        .update(
            &mut *tx,
            &[
                ("number", &Some(number)),
                ("issue_count", &(bill.issue_count + 1)),
                ("in_ageing", &true),
                ("tax_document_pending", &pending),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// What `resubmit` writes besides the status: the count, the ageing flag,
/// the denied amounts asked again, and whether a tax document is still to be
/// made.
async fn write_resubmit_in(
    tx: &mut renox::db::Transaction,
    bill: &PayerBill,
    _by: Option<i64>,
) -> Result {
    PayerBillLine::query()
        .where_(PayerBillLine::PAYER_BILL_ID.eq(bill.id))
        .update(
            &mut *tx,
            &[("denied", &0_i64), ("updated_at", &renox::db::now())],
        )
        .await?;
    let lines = lines_of(&mut *tx, bill.id).await?;
    write_totals_in(tx, bill.id, &lines).await?;
    let pending = payer_tax::needs_documents(bill, &lines);
    PayerBill::query()
        .where_(PayerBill::ID.eq(bill.id))
        .update(
            &mut *tx,
            &[
                ("resubmission_count", &(bill.resubmission_count + 1)),
                ("issue_count", &(bill.issue_count + 1)),
                ("in_ageing", &true),
                ("tax_document_pending", &pending),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// What the guards of issuing or sending `bill` again need.
async fn gather(state: &AppState, bill: &PayerBill, by: &By<'_>) -> Result<IssueInput> {
    let agreement = PayerAgreement::find_or_404(&state.db, bill.agreement_id).await?;
    let lines = lines_of(&state.db, bill.id).await?;
    let reference_given = !agreement.reference_required
        || bill
            .payer_reference
            .as_deref()
            .is_some_and(|r| !r.trim().is_empty());
    let deadline: Option<NaiveDate> = bill
        .last_remittance_on
        .and_then(|on| agreement.resubmission_deadline(on));
    Ok(IssueInput {
        agreement_active: agreement.active,
        reference_given,
        lines: lines.len(),
        all_claimed: lines.iter().all(|line| line.claimed > 0),
        document_problem: payer_tax::issue_problem(state, bill, &lines).await?,
        corrections: None,
        within_window: deadline.is_none_or(|last| today() <= last),
        within_limit: bill.resubmission_count < agreement.resubmission_limit,
        by: by.id(),
    })
}

/// Issues draft `bill_id` as `by`, in its own transaction. After the commit
/// the tax document is made and `PayerBillIssued` is emitted.
///
/// # Errors
///
/// `Forbidden` without `payer_bills.issue`, `NotFrom` when it is not a
/// draft, `Guard` (422) for an inactive agreement, a missing payer
/// reference, a line without a claimed amount or a payer's tax invoice that
/// would not pass. Nothing is written then and no number is used.
pub async fn issue(
    state: &AppState,
    by: By<'_>,
    bill_id: i64,
) -> std::result::Result<PayerBill, TransitionError> {
    let bill = PayerBill::find_or_404(&state.db, bill_id).await?;
    let gathered = gather(state, &bill, &by).await?;
    let _outcome = run_of(&bill, &by, ISSUE, ISSUE_SYSTEM, json!({}))
        .input(&gathered)
        .run(state)
        .await?;
    Ok(PayerBill::find_or_404(&state.db, bill_id).await?)
}

/// Sends `bill_id` again as `by`, with the written `corrections`.
///
/// # Errors
///
/// `Forbidden` without `payer_bills.issue`, `NotFrom` unless the bill is
/// `partially_paid` or `denied`, `Guard` (422) without corrections, past
/// the agreement's resubmission window or at its limit.
pub async fn resubmit(
    state: &AppState,
    by: By<'_>,
    bill_id: i64,
    corrections: &str,
) -> std::result::Result<PayerBill, TransitionError> {
    let bill = PayerBill::find_or_404(&state.db, bill_id).await?;
    let mut gathered = gather(state, &bill, &by).await?;
    gathered.corrections = Some(corrections.trim().to_owned()).filter(|c| !c.is_empty());
    let _outcome = run_of(&bill, &by, RESUBMIT, RESUBMIT_SYSTEM, json!({}))
        .input(&gathered)
        .run(state)
        .await?;
    Ok(PayerBill::find_or_404(&state.db, bill_id).await?)
}

/// After the commit of an issue or a resubmit: makes the tax document the
/// bill still needs and emits `PayerBillIssued`.
pub(crate) async fn on_transition(state: &AppState, event: &Transitioned<PayerBill>) -> Result {
    if ![ISSUE, ISSUE_SYSTEM, RESUBMIT, RESUBMIT_SYSTEM].contains(&event.transition.as_str()) {
        return Ok(());
    }
    payer_tax::settle(state, event.key).await?;
    let bill = PayerBill::find_or_404(&state.db, event.key).await?;
    state
        .emit(PayerBillIssued {
            payer_bill: bill.payer_bill_id(),
            payer: bill.payer(),
            source: bill.source(),
            claimed: bill.claimed_total,
            open: bill.balance,
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> IssueInput {
        IssueInput {
            agreement_active: true,
            reference_given: true,
            lines: 1,
            all_claimed: true,
            within_window: true,
            within_limit: true,
            corrections: Some("member ID fixed".into()),
            ..Default::default()
        }
    }

    fn ctx(input: &IssueInput) -> TransitionCtx {
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    async fn refusal(name: &str, bill: &PayerBill, ctx: &TransitionCtx) -> Option<String> {
        let machine = PayerBill::machine();
        machine
            .find(name)
            .unwrap()
            .check_guards(bill, ctx)
            .await
            .err()
            .map(|e| e.to_string())
    }

    #[test]
    fn issue_and_resubmit_have_a_user_and_a_system_twin() {
        let machine = PayerBill::machine();
        for (name, system, from, to) in [
            (ISSUE, false, &["draft"][..], "issued"),
            (ISSUE_SYSTEM, true, &["draft"][..], "issued"),
            (RESUBMIT, false, &["partially_paid", "denied"][..], "issued"),
            (
                RESUBMIT_SYSTEM,
                true,
                &["partially_paid", "denied"][..],
                "issued",
            ),
        ] {
            let transition = machine.find(name).unwrap();
            assert_eq!(transition.is_system(), system, "{name}");
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
        }
        assert_eq!(
            machine.find(ISSUE).unwrap().guard_names(),
            [
                "agreement_active",
                "reference_given",
                "lines_claimed",
                "payer_invoice_valid"
            ]
        );
    }

    #[renox::test]
    async fn a_ready_bill_passes_and_each_missing_fact_refuses() {
        let bill = PayerBill::default();
        assert_eq!(refusal(ISSUE, &bill, &ctx(&ready())).await, None);
        let inactive = IssueInput {
            agreement_active: false,
            ..ready()
        };
        assert!(refusal(ISSUE, &bill, &ctx(&inactive)).await.is_some());
        let no_reference = IssueInput {
            reference_given: false,
            ..ready()
        };
        assert!(refusal(ISSUE, &bill, &ctx(&no_reference)).await.is_some());
        let unclaimed = IssueInput {
            all_claimed: false,
            ..ready()
        };
        assert!(refusal(ISSUE, &bill, &ctx(&unclaimed)).await.is_some());
        let invoice = IssueInput {
            document_problem: Some("The payer was not found.".into()),
            ..ready()
        };
        assert_eq!(
            refusal(ISSUE, &bill, &ctx(&invoice)).await.as_deref(),
            Some("The payer was not found.")
        );
        // Without the gathered input nothing is issued.
        assert!(refusal(ISSUE, &bill, &TransitionCtx::new(None)).await.is_some());
    }

    #[renox::test]
    async fn a_resubmission_needs_corrections_inside_the_window_and_the_limit() {
        let bill = PayerBill::default();
        assert_eq!(refusal(RESUBMIT, &bill, &ctx(&ready())).await, None);
        for broken in [
            IssueInput {
                corrections: Some("  ".into()),
                ..ready()
            },
            IssueInput {
                within_window: false,
                ..ready()
            },
            IssueInput {
                within_limit: false,
                ..ready()
            },
        ] {
            assert!(refusal(RESUBMIT, &bill, &ctx(&broken)).await.is_some());
        }
    }
}
