//! Closing a payer bill: writing off or moving the open amount, and voiding
//! one created in error (wl-invoicing §4 *Payer bill* `transfer_to_customer`,
//! `write_off` and `void`, §5.8 step 5, `FIN.ACL.14`, `PLT.ACL.09`).
//!
//! **Write-off and transfer.** A user holding `payer_bills.write_off`
//! closes a `draft`, `partially_paid` or `denied` bill with a reason: the
//! open amount of every line is written off, or moved to the customer where
//! the payer agreement allows billing the customer. From `draft` the payer's
//! first-submission window must have passed (today is after its last day,
//! Asia/Dubai) and the agreement must not accept late submissions; the bill
//! is then reported as *not submitted in time*. The machine has no app
//! state, so [`write_off`] and [`transfer_to_customer`] gather the facts
//! into a [`CloseInput`]. A write-off above the agreement's limit becomes an
//! approval request (`FIN.ACL.14`, [`crate::payer_policy`]): the input's
//! `amount` is the open amount when it is above the limit and 0 otherwise,
//! so the policy (any amount above 0) fits the limit of each agreement. The
//! kernel runs `write_off_approved` when the last approver says yes; its
//! guards check the bill again.
//!
//! The tax documents follow the commit ([`documents`]), because a
//! transition's effect cannot call another module's contract:
//!
//! | Closing | Customer is the VAT recipient | Payer is the VAT recipient |
//! |---|---|---|
//! | `write_off` | credit note against the customer's tax invoice for the payer lines written off (it carries them, issued at the visit, even while the bill is a draft) | credit note against the payer's tax invoice; none when it was never issued |
//! | `transfer_to_customer` | nothing: the amount stays on the customer's tax invoice, now as the customer's own share; the customer is told | credit note against the payer's tax invoice (none when never issued) and a tax invoice to the customer |
//!
//! **Void.** `void` (kernel `PLT.ACL.09`, `payer_bills.issue`, a reason)
//! turns a `draft` created in error (a duplicate, the wrong payer or policy)
//! into `voided`: it keeps its number, it is no longer a bill of its source
//! (the source can be split again to the right payer) and a payer tax
//! invoice already issued is cancelled by a credit note. It is refused once
//! a remittance was recorded, and, when the bill was made with
//! `void_before_first_submission` (05 `CLN.ACL.14`), once it was issued.
//! The billing coordinators are told. The void goes through the kernel's
//! generic route, so no input is gathered: the credit note of an issued
//! payer invoice is made after the commit and, if the invoice refuses it
//! (a payment is allocated to it), the bill stays flagged with a tax
//! document pending.

use renox::db::Json;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{CreditLine, DraftLine, InvoiceDraft, InvoiceStatus};
use wl_kernel::payer_bills::{
    PayerAnswer, PayerBillClosed, PayerBillOutcome, PayerBillStatus, RecoupedLine,
};
use wl_kernel::voids;

use crate::credit_notes::{self, By};
use crate::invoice::Invoice;
use crate::invoicer::{create_draft, lines_of as invoice_lines_of};
use crate::payer_agreement::PayerAgreement;
use crate::payer_bill::{PayerBill, PayerBillLine, lines_of, write_totals_in};
use crate::payer_notices;
use crate::payer_remittance::answer_of;
use crate::permissions;
use crate::texts::english;

/// The user transition that moves the open amount to the customer.
pub const TRANSFER_TO_CUSTOMER: &str = "transfer_to_customer";
/// The user transition that writes the open amount off.
pub const WRITE_OFF: &str = "write_off";
/// The transition the kernel runs at final approval of a write-off.
pub const WRITE_OFF_APPROVED: &str = "write_off_approved";
/// The approval policies' action key of `write_off` (`FIN.ACL.14`).
pub const WRITE_OFF_ACTION: &str = "invoicing.payer_write_off";

/// The states a close leaves.
const FROM: [PayerBillStatus; 3] = [
    PayerBillStatus::Draft,
    PayerBillStatus::PartiallyPaid,
    PayerBillStatus::Denied,
];

/// How a close ended, as stored.
const WRITTEN_OFF: &str = "written_off";
const TRANSFERRED: &str = "transferred";
const VOIDED: &str = "voided";

/// What a close run carries to its guards and effect, gathered by
/// [`write_off`] and [`transfer_to_customer`]; for a write-off it is also
/// the approval request's payload, which the policy's condition reads
/// (`amount`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct CloseInput {
    /// The reason given.
    pub reason: Option<String>,
    /// The open amount when requested: what moves.
    pub open: i64,
    /// What needs approval: the open amount when it is above the
    /// agreement's write-off limit, else 0.
    pub amount: i64,
    /// Whether the payer's first-submission window has passed.
    pub window_passed: bool,
    /// Whether the agreement accepts late submissions.
    pub accepts_late: bool,
    /// Whether the agreement allows billing the customer.
    pub customer_allowed: bool,
    /// Why the tax documents of the close would be refused.
    pub document_problem: Option<String>,
    /// The user a system run acts for.
    pub by: Option<i64>,
}

/// The run's input, if it is a [`CloseInput`].
fn input(ctx: &TransitionCtx) -> Option<CloseInput> {
    ctx.input_as::<CloseInput>().ok()
}

/// The reason of a run: the run's own, else the one in the input (the
/// approved twin gets the stored input).
fn reason_of(ctx: &TransitionCtx) -> Option<String> {
    ctx.reason()
        .map(str::to_owned)
        .or_else(|| input(ctx).and_then(|i| i.reason))
        .map(|r| r.trim().to_owned())
        .filter(|r| !r.is_empty())
}

/// The close transitions, for the machine: `transfer_to_customer`,
/// `write_off` and its approved twin, and the kernel's `void` and
/// `void_approved`.
pub(crate) fn transitions() -> Vec<Transition<PayerBill>> {
    let mut all = vec![
        close_rules(
            Transition::new(TRANSFER_TO_CUSTOMER)
                .label("Move to the customer")
                .tone("danger")
                .permission(permissions::PAYER_BILLS_WRITE_OFF),
            TRANSFERRED,
        )
        .guard("customer_billable", |_bill: &PayerBill, ctx| {
            let ok = input(ctx).is_some_and(|i| i.customer_allowed);
            Box::pin(async move {
                if ok {
                    Ok(())
                } else {
                    Err(english("wl_invoicing.payer_bill.errors.customer_not_billable").to_owned())
                }
            })
        }),
        close_rules(
            Transition::new(WRITE_OFF)
                .label("Write off")
                .tone("danger")
                .permission(permissions::PAYER_BILLS_WRITE_OFF),
            WRITTEN_OFF,
        ),
        close_rules(
            Transition::new(WRITE_OFF_APPROVED)
                .label("Write off (approved)")
                .system(),
            WRITTEN_OFF,
        ),
    ];
    all.extend(
        voids::transitions::<PayerBill, _>(
            crate::payer_bill::SUBJECT_TYPE,
            [PayerBillStatus::Draft],
        )
        .into_iter()
        .map(void_rules),
    );
    all
}

/// What `transfer_to_customer` and `write_off` share: the states, the
/// guards and the effect.
fn close_rules(transition: Transition<PayerBill>, how: &'static str) -> Transition<PayerBill> {
    transition
        .from(FROM)
        .to(PayerBillStatus::Closed)
        .guard("reason_given", |_bill: &PayerBill, ctx| {
            let ok = reason_of(ctx).is_some();
            Box::pin(async move {
                if ok {
                    Ok(())
                } else {
                    Err(english("wl_invoicing.payer_bill.errors.close_reason").to_owned())
                }
            })
        })
        .guard("something_open", |bill: &PayerBill, ctx| {
            let open = bill.balance;
            let asked = input(ctx).map(|i| i.open);
            Box::pin(async move {
                match asked {
                    Some(asked) if open > 0 && asked == open => Ok(()),
                    Some(_) if open > 0 => {
                        Err(english("wl_invoicing.payer_bill.errors.close_changed").to_owned())
                    }
                    _ => Err(english("wl_invoicing.payer_bill.errors.close_nothing").to_owned()),
                }
            })
        })
        .guard("first_window_missed", |bill: &PayerBill, ctx| {
            // From draft only: the payer still accepts the claim while its
            // window is open or when it accepts late submissions.
            let draft = bill.status == PayerBillStatus::Draft;
            let facts = input(ctx).map(|i| (i.window_passed, i.accepts_late));
            Box::pin(async move {
                if !draft {
                    return Ok(());
                }
                match facts {
                    Some((true, false)) => Ok(()),
                    Some((false, _)) => {
                        Err(english("wl_invoicing.payer_bill.errors.window_open").to_owned())
                    }
                    Some((true, true)) => {
                        Err(english("wl_invoicing.payer_bill.errors.accepts_late").to_owned())
                    }
                    None => Err(english("wl_invoicing.payer_bill.errors.close_nothing").to_owned()),
                }
            })
        })
        .guard("documents_ready", |_bill: &PayerBill, ctx| {
            let problem = input(ctx).and_then(|i| i.document_problem);
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(move |bill: &PayerBill, ctx, tx| {
            let bill = bill.clone();
            let reason = reason_of(ctx);
            let by = ctx
                .actor()
                .map(|user| user.id)
                .or_else(|| input(ctx).and_then(|i| i.by));
            Box::pin(async move { write_close_in(tx, &bill, how, reason, by).await })
        })
}

/// The 05 rule on top of the kernel's `void`: a bill made with
/// `void_before_first_submission` is not voided once it was issued
/// (`CLN.ACL.14`); and what `void` writes besides the kernel's Void record.
fn void_rules(transition: Transition<PayerBill>) -> Transition<PayerBill> {
    transition
        .guard("before_first_submission", |bill: &PayerBill, _ctx| {
            let refused = bill.void_before_first_submission && bill.issue_count > 0;
            Box::pin(async move {
                if refused {
                    Err(english("wl_invoicing.payer_bill.errors.void_submitted").to_owned())
                } else {
                    Ok(())
                }
            })
        })
        .effect(|bill: &PayerBill, ctx, tx| {
            let id = bill.id;
            let pending = bill.payer_is_recipient() && bill.payer_invoice_id.is_some();
            let reason = ctx
                .reason()
                .or_else(|| ctx.input().get("reason").and_then(Value::as_str))
                .map(|r| r.trim().to_owned());
            let by = ctx
                .actor()
                .map(|user| user.id)
                .or_else(|| ctx.meta().get("requester_id").and_then(Value::as_i64));
            Box::pin(async move {
                // Not the status column, so outside the machine's guard.
                PayerBill::query()
                    .where_(PayerBill::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("close_outcome", &Some(VOIDED.to_owned())),
                            ("close_reason", &reason),
                            ("closed_at", &Some(renox::db::now())),
                            ("closed_by", &by),
                            ("in_ageing", &false),
                            ("tax_document_pending", &pending),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// What a close writes besides the status, in the run's transaction: the
/// open amount of every line moves (to the customer or off), the totals, the
/// ageing and whether a tax document is still to be made.
async fn write_close_in(
    tx: &mut renox::db::Transaction,
    bill: &PayerBill,
    how: &'static str,
    reason: Option<String>,
    by: Option<i64>,
) -> Result {
    let mut lines = lines_of(&mut *tx, bill.id).await?;
    let mut moved: Vec<RecoupedLine> = Vec::new();
    for line in &mut lines {
        let open = line.open();
        if open <= 0 {
            continue;
        }
        if how == TRANSFERRED {
            line.transferred += open;
        } else {
            line.written_off += open;
        }
        line.denied = 0;
        moved.push(RecoupedLine {
            line: line.id,
            amount: open,
        });
        PayerBillLine::query()
            .where_(PayerBillLine::ID.eq(line.id))
            .update(
                &mut *tx,
                &[
                    ("transferred", &line.transferred),
                    ("written_off", &line.written_off),
                    ("denied", &line.denied),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
    }
    write_totals_in(tx, bill.id, &lines).await?;
    let amount: i64 = moved.iter().map(|m| m.amount).sum();
    let pending = documents_due(bill, how);
    PayerBill::query()
        .where_(PayerBill::ID.eq(bill.id))
        .update(
            &mut *tx,
            &[
                ("close_outcome", &Some(how.to_owned())),
                ("close_reason", &reason),
                ("close_amount", &amount),
                ("close_lines", &Json(moved)),
                ("close_credit_note_id", &None::<i64>),
                ("close_invoice_id", &None::<i64>),
                ("closed_at", &Some(renox::db::now())),
                ("closed_by", &by),
                ("in_ageing", &false),
                (
                    "not_submitted_in_time",
                    &(bill.status == PayerBillStatus::Draft),
                ),
                ("tax_document_pending", &pending),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// Whether the close `how` of `bill` still has to make a tax document: a
/// credit note against the invoice that carries the lines, or the customer's
/// own invoice.
fn documents_due(bill: &PayerBill, how: &str) -> bool {
    let payer = bill.payer_is_recipient();
    match how {
        WRITTEN_OFF if payer => bill.payer_invoice_id.is_some(),
        WRITTEN_OFF => bill.customer_invoice_id.is_some(),
        // The payer's invoice is credited when it exists, and the customer
        // gets an invoice of their own; nothing is made when the customer
        // already holds the lines.
        TRANSFERRED => payer,
        _ => false,
    }
}

/// The facts the guards of a close need, and why its tax documents would be
/// refused.
async fn gather(state: &AppState, bill: &PayerBill, write_off: bool) -> Result<CloseInput> {
    let agreement = PayerAgreement::find_or_404(&state.db, bill.agreement_id).await?;
    let lines = lines_of(&state.db, bill.id).await?;
    let open = bill.balance;
    let how = if write_off { WRITTEN_OFF } else { TRANSFERRED };
    Ok(CloseInput {
        reason: None,
        open,
        amount: if write_off && open > agreement.write_off_limit {
            open
        } else {
            0
        },
        window_passed: agreement.first_window_passed(bill.service_date, today()),
        accepts_late: agreement.accepts_late_submissions,
        customer_allowed: agreement.bill_customer_allowed,
        document_problem: document_problem(state, bill, &lines, how).await?,
        by: None,
    })
}

/// The invoice that carries the payer lines the close credits, if there is
/// one.
fn credited_invoice(bill: &PayerBill, how: &str) -> Option<i64> {
    if !documents_due(bill, how) {
        return None;
    }
    if bill.payer_is_recipient() {
        bill.payer_invoice_id
    } else {
        bill.customer_invoice_id
    }
}

/// The credit note lines that close `how` makes for `lines`: the open
/// amount of each, on the line of the tax document that carries it.
fn credit_lines(lines: &[PayerBillLine]) -> std::result::Result<Vec<CreditLine>, String> {
    lines
        .iter()
        .filter(|line| line.open() > 0)
        .map(|line| {
            line.invoice_line_position
                .map(|position| CreditLine::new(position, line.open()))
                .ok_or_else(|| english("wl_invoicing.payer_bill.errors.no_invoice_line").to_owned())
        })
        .collect()
}

/// Why the tax documents of closing `bill` as `how` would be refused: the
/// invoice cannot be credited by that much, or the customer is not found
/// for the invoice they are to get.
async fn document_problem(
    state: &AppState,
    bill: &PayerBill,
    lines: &[PayerBillLine],
    how: &str,
) -> Result<Option<String>> {
    if let Some(invoice_id) = credited_invoice(bill, how) {
        let invoice = Invoice::find_or_404(&state.db, invoice_id).await?;
        if ![
            InvoiceStatus::Issued,
            InvoiceStatus::PartiallyPaid,
            InvoiceStatus::Overdue,
            InvoiceStatus::Paid,
        ]
        .contains(&invoice.status)
        {
            return Ok(Some(
                english("wl_invoicing.payer_bill.errors.invoice_not_creditable").to_owned(),
            ));
        }
        let wanted = match credit_lines(lines) {
            Ok(wanted) => wanted,
            Err(problem) => return Ok(Some(problem)),
        };
        let invoice_lines = invoice_lines_of(&state.db, invoice.id).await?;
        let credited = credit_notes::credited_by_line(&state.db, invoice.id).await?;
        if let Err(problem) = credit_notes::plan_lines(&wanted, &invoice_lines, &credited) {
            return Ok(Some(problem));
        }
    }
    if how == TRANSFERRED && bill.payer_is_recipient() {
        let customer = wl_kernel::parties::directory(state)
            .billing_identity(&state.db, bill.customer())
            .await?;
        if customer.is_none() {
            return Ok(Some(
                english("wl_invoicing.payer_bill.errors.customer_not_found").to_owned(),
            ));
        }
    }
    Ok(None)
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Writes off the open amount of `bill_id` as `user` with `reason`, in its
/// own transaction. Above the agreement's write-off limit it waits as an
/// approval request (`FIN.ACL.14`): the answer is then
/// [`PayerAnswer::Pending`], the request is announced and the bill is as it
/// was.
///
/// # Errors
///
/// `Forbidden` without `payer_bills.write_off`; `NotFrom` unless the bill is
/// `draft`, `partially_paid` or `denied`; `Guard` (422) without a reason or
/// an open amount, from `draft` while the first-submission window is open or
/// when the agreement accepts late submissions, or when the credit note
/// could not be made. Nothing is written then.
pub async fn write_off(
    state: &AppState,
    user: &AuthUser,
    bill_id: i64,
    reason: &str,
) -> std::result::Result<(PayerBill, PayerAnswer), TransitionError> {
    let bill = PayerBill::find_or_404(&state.db, bill_id).await?;
    let mut gathered = gather(state, &bill, true).await?;
    gathered.reason = Some(reason.trim().to_owned()).filter(|r| !r.is_empty());
    gathered.by = Some(user.id);
    let mut run = bill
        .transition(WRITE_OFF)
        .by(user)
        .meta(json!({}))
        .input(&gathered);
    if let Some(reason) = &gathered.reason {
        run = run.reason(reason.clone());
    }
    let ran = run.run(state).await?;
    wl_kernel::approvals::announce(state, &ran)
        .await
        .map_err(TransitionError::Failed)?;
    Ok((
        PayerBill::find_or_404(&state.db, bill_id).await?,
        answer_of(&ran),
    ))
}

/// Moves the open amount of `bill_id` to the customer as `user` with
/// `reason`, in its own transaction; the customer is told.
///
/// # Errors
///
/// As [`write_off`], and `Guard` (422) when the payer agreement does not
/// allow billing the customer.
pub async fn transfer_to_customer(
    state: &AppState,
    user: &AuthUser,
    bill_id: i64,
    reason: &str,
) -> std::result::Result<PayerBill, TransitionError> {
    let bill = PayerBill::find_or_404(&state.db, bill_id).await?;
    let mut gathered = gather(state, &bill, false).await?;
    gathered.reason = Some(reason.trim().to_owned()).filter(|r| !r.is_empty());
    gathered.by = Some(user.id);
    let mut run = bill
        .transition(TRANSFER_TO_CUSTOMER)
        .by(user)
        .meta(json!({}))
        .input(&gathered);
    if let Some(reason) = &gathered.reason {
        run = run.reason(reason.clone());
    }
    let _ran = run.run(state).await?;
    Ok(PayerBill::find_or_404(&state.db, bill_id).await?)
}

/// Voids the `draft` payer bill `bill_id` as `user` with `reason`
/// (`PLT.ACL.09`). With nothing posted it is done at once.
///
/// # Errors
///
/// `Forbidden` without `payer_bills.issue`; `NotFrom` unless the bill is a
/// `draft`; `Guard` (422) once a remittance is recorded, after its first
/// submission for a bill made with `void_before_first_submission`, or
/// without a reason.
pub async fn void(
    state: &AppState,
    user: &AuthUser,
    bill_id: i64,
    reason: &str,
) -> std::result::Result<PayerAnswer, TransitionError> {
    let bill = PayerBill::find_or_404(&state.db, bill_id).await?;
    match voids::void(state, user, &bill.subject(), reason).await? {
        voids::VoidOutcome::Voided(_) => Ok(PayerAnswer::Done),
        voids::VoidOutcome::Pending(pending) => Ok(PayerAnswer::Pending {
            request: pending
                .get(wl_kernel::approvals::submit::REQUEST_KEY)
                .and_then(Value::as_i64)
                .unwrap_or_default(),
        }),
    }
}

/// Makes the tax documents a close still needs and clears the flag: the
/// credit note against the invoice that carries the lines, the customer's
/// tax invoice of a transfer (payer is the recipient), or the credit note
/// that cancels the payer's invoice of a voided bill. Each document is
/// recorded on the bill as soon as it is made, so a retry makes nothing
/// twice.
///
/// # Errors
///
/// The refusal of the credit note or of the invoice's own `issue`; the flag
/// stays set then.
pub async fn documents(state: &AppState, bill_id: i64) -> Result {
    let bill = PayerBill::find_or_404(&state.db, bill_id).await?;
    if !bill.tax_document_pending {
        return Ok(());
    }
    let how = bill.close_outcome.clone().unwrap_or_default();
    let reason = bill.close_reason.clone().unwrap_or_default();
    let number = bill.number.clone().unwrap_or_else(|| format!("#{}", bill.id));
    if how == VOIDED {
        if let (Some(invoice_id), None) = (bill.payer_invoice_id, bill.close_credit_note_id) {
            let invoice = Invoice::find_or_404(&state.db, invoice_id).await?;
            let note = crate::credit::credit_in_full(
                state,
                &invoice,
                &format!("Payer bill {number} voided: {reason}"),
                false,
                By::System(bill.closed_by),
            )
            .await
            .map_err(TransitionError::into_error)?;
            set_close_document(state, bill.id, "close_credit_note_id", note.id).await?;
        }
        return clear_flag(state, bill.id).await;
    }
    let moved: Vec<PayerBillLine> = {
        let lines = lines_of(&state.db, bill.id).await?;
        bill.close_lines
            .iter()
            .filter_map(|m| {
                lines.iter().find(|l| l.id == m.line).map(|l| {
                    let mut moved = l.clone();
                    moved.claimed = m.amount;
                    moved.paid = 0;
                    moved.written_off = 0;
                    moved.transferred = 0;
                    moved
                })
            })
            .collect()
    };
    if bill.close_credit_note_id.is_none()
        && let Some(invoice_id) = credited_invoice(&bill, &how)
    {
        let credit = credit_lines(&moved).map_err(refused)?;
        let note = credit_notes::credit_partly(
            state,
            invoice_id,
            &credit,
            &format!("Payer bill {number}: {reason}"),
            By::System(bill.closed_by),
        )
        .await
        .map_err(TransitionError::into_error)?;
        set_close_document(state, bill.id, "close_credit_note_id", note.id).await?;
    }
    if how == TRANSFERRED && bill.payer_is_recipient() && bill.close_invoice_id.is_none() {
        let mut draft = InvoiceDraft::new(bill.customer(), bill.subject())
            .legal_entity(bill.legal_entity_id)
            .currency(&bill.currency)
            .date_of_supply(bill.service_date);
        for line in &moved {
            draft = draft.line(DraftLine::new(
                &line.description,
                line.claimed,
                line.vat_category,
            ));
        }
        let invoice = create_draft(&state.db, draft).await?;
        crate::issue::issue_for_payer_bill(state, &invoice)
            .await
            .map_err(TransitionError::into_error)?;
        set_close_document(state, bill.id, "close_invoice_id", invoice.id).await?;
    }
    clear_flag(state, bill.id).await
}

/// Records the id of a document the close made.
async fn set_close_document(state: &AppState, id: i64, column: &str, document: i64) -> Result {
    PayerBill::query()
        .where_(PayerBill::ID.eq(id))
        .update(
            &state.db,
            &[
                (column, &Some(document)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// Clears `tax_document_pending` on bill `id`.
async fn clear_flag(state: &AppState, id: i64) -> Result {
    PayerBill::query()
        .where_(PayerBill::ID.eq(id))
        .update(
            &state.db,
            &[
                ("tax_document_pending", &false),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// The outcome `PayerBillClosed` names for a stored close.
fn outcome_of(stored: &str) -> PayerBillOutcome {
    match stored {
        TRANSFERRED => PayerBillOutcome::Transferred,
        VOIDED => PayerBillOutcome::Voided,
        _ => PayerBillOutcome::WrittenOff,
    }
}

/// After the commit of a close or a void: makes the tax documents, tells the
/// customer (a transfer) or the coordinators (a void), and emits
/// `PayerBillClosed`.
pub(crate) async fn on_transition(state: &AppState, event: &Transitioned<PayerBill>) -> Result {
    let name = event.transition.as_str();
    if ![
        TRANSFER_TO_CUSTOMER,
        WRITE_OFF,
        WRITE_OFF_APPROVED,
        voids::VOID,
        voids::VOID_APPROVED,
    ]
    .contains(&name)
    {
        return Ok(());
    }
    documents(state, event.key).await?;
    let bill = PayerBill::find_or_404(&state.db, event.key).await?;
    let outcome = outcome_of(bill.close_outcome.as_deref().unwrap_or_default());
    match outcome {
        PayerBillOutcome::Transferred => {
            payer_notices::tell_customer(state, &bill, bill.close_amount).await?;
        }
        PayerBillOutcome::Voided => payer_notices::tell_coordinators(state, &bill).await?,
        PayerBillOutcome::WrittenOff => {}
    }
    let open = if outcome == PayerBillOutcome::Voided {
        bill.claimed_total - bill.paid_total
    } else {
        bill.close_amount
    };
    state
        .emit(PayerBillClosed {
            payer_bill: bill.payer_bill_id(),
            payer: bill.payer(),
            source: bill.source(),
            claimed: bill.claimed_total,
            paid: bill.paid_total,
            denied: bill.denied_total,
            open,
            outcome,
            not_submitted_in_time: bill.not_submitted_in_time,
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft_bill(open: i64) -> PayerBill {
        PayerBill {
            status: PayerBillStatus::Draft,
            balance: open,
            ..Default::default()
        }
    }

    fn ctx(input: &CloseInput) -> TransitionCtx {
        TransitionCtx::new(None)
            .with_input(renox::serde_json::to_value(input).expect("json"))
            .with_reason("missed the window")
    }

    fn missed() -> CloseInput {
        CloseInput {
            reason: Some("missed the window".into()),
            open: 60_000,
            window_passed: true,
            accepts_late: false,
            customer_allowed: true,
            ..Default::default()
        }
    }

    async fn refusal(name: &str, bill: &PayerBill, ctx: &TransitionCtx) -> Option<String> {
        PayerBill::machine()
            .find(name)
            .unwrap()
            .check_guards(bill, ctx)
            .await
            .err()
            .map(|e| e.to_string())
    }

    #[test]
    fn the_closes_leave_three_states_for_closed_and_void_leaves_draft_for_voided() {
        let machine = PayerBill::machine();
        for (name, system) in [
            (TRANSFER_TO_CUSTOMER, false),
            (WRITE_OFF, false),
            (WRITE_OFF_APPROVED, true),
        ] {
            let transition = machine.find(name).unwrap();
            assert_eq!(
                transition.from_states(),
                ["draft", "partially_paid", "denied"]
            );
            assert_eq!(transition.to_state(), Some("closed"));
            assert_eq!(transition.is_system(), system, "{name}");
        }
        let void = machine.find(voids::VOID).unwrap();
        assert_eq!(void.from_states(), ["draft"]);
        assert_eq!(void.to_state(), Some("voided"));
        assert!(machine.find(voids::VOID_APPROVED).unwrap().is_system());
        assert_eq!(
            wl_kernel::machines::approved(WRITE_OFF),
            WRITE_OFF_APPROVED
        );
    }

    #[renox::test]
    async fn a_draft_is_closed_only_after_a_missed_window_for_a_payer_that_does_not_accept_late() {
        let bill = draft_bill(60_000);
        assert_eq!(refusal(WRITE_OFF, &bill, &ctx(&missed())).await, None);
        let open = CloseInput {
            window_passed: false,
            ..missed()
        };
        assert!(refusal(WRITE_OFF, &bill, &ctx(&open)).await.is_some());
        let late = CloseInput {
            accepts_late: true,
            ..missed()
        };
        assert!(refusal(WRITE_OFF, &bill, &ctx(&late)).await.is_some());
        // The window does not matter once the payer saw the bill.
        let denied = PayerBill {
            status: PayerBillStatus::Denied,
            ..draft_bill(60_000)
        };
        assert_eq!(refusal(WRITE_OFF, &denied, &ctx(&open)).await, None);
    }

    #[renox::test]
    async fn a_close_needs_a_reason_an_open_amount_and_the_customer_to_be_billable() {
        let bill = draft_bill(60_000);
        let no_reason = TransitionCtx::new(None)
            .with_input(renox::serde_json::to_value(CloseInput {
                reason: None,
                ..missed()
            })
            .unwrap());
        assert!(refusal(WRITE_OFF, &bill, &no_reason).await.is_some());
        assert!(
            refusal(WRITE_OFF, &draft_bill(0), &ctx(&missed()))
                .await
                .is_some()
        );
        // The balance moved since the amount was asked.
        assert!(
            refusal(WRITE_OFF, &draft_bill(50_000), &ctx(&missed()))
                .await
                .is_some()
        );
        let not_billable = CloseInput {
            customer_allowed: false,
            ..missed()
        };
        assert!(
            refusal(TRANSFER_TO_CUSTOMER, &bill, &ctx(&not_billable))
                .await
                .is_some()
        );
        assert_eq!(
            refusal(TRANSFER_TO_CUSTOMER, &bill, &ctx(&missed())).await,
            None
        );
    }

    #[test]
    fn the_documents_follow_who_is_the_vat_recipient() {
        let customer = PayerBill {
            customer_invoice_id: Some(5),
            ..Default::default()
        };
        assert_eq!(credited_invoice(&customer, WRITTEN_OFF), Some(5));
        assert_eq!(credited_invoice(&customer, TRANSFERRED), None);
        let payer = PayerBill {
            vat_recipient: crate::payer_agreement::VatRecipient::Payer,
            payer_invoice_id: Some(8),
            ..Default::default()
        };
        assert_eq!(credited_invoice(&payer, WRITTEN_OFF), Some(8));
        assert_eq!(credited_invoice(&payer, TRANSFERRED), Some(8));
        // A payer's invoice never issued: nothing to credit.
        let never = PayerBill {
            payer_invoice_id: None,
            ..payer
        };
        assert_eq!(credited_invoice(&never, WRITTEN_OFF), None);
        assert!(!documents_due(&never, WRITTEN_OFF));
        assert!(documents_due(&never, TRANSFERRED), "the customer's invoice");
    }
}
