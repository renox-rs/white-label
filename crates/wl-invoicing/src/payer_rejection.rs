//! A technical rejection of a payer bill's submission (wl-invoicing §4
//! *Payer bill* `submission_rejected`, §5.8 step 6, vertical mapping row
//! "technical rejection at submission").
//!
//! The submission channel (eClaimLink, Shafafiya, MOHAP) can refuse a
//! submission before the payer looks at it: bad format, a missing field, a
//! wrong member ID. `submission_rejected` moves the `issued` bill back to
//! `draft` for a user holding `payer_bills.issue`, or for the system on
//! behalf of the vertical (05 claim `reject_submission`). It is not a
//! resubmission: the resubmission count, the bill's number and the payer's
//! tax invoice stay as they are.
//!
//! The guards: at least one error with a code and a message, and no
//! remittance recorded on the bill (a rejection that arrives after the
//! payer paid is refused and the coordinator gets a task instead). The
//! effect, in the transition's transaction: each error is stored with its
//! attempt (the number of the submission it rejects, so a bill rejected
//! several times keeps every attempt's errors) and the receivable leaves the
//! ageing until the bill is issued again. After the commit a listener opens
//! the billing coordinator's task with the errors and the deadline, and
//! emits `PayerBillSubmissionRejected`.
//!
//! Decisions:
//!
//! - The deadline on the task is the last day of the window of the attempt:
//!   the agreement's first-submission window counted from the service date,
//!   or, for a bill that was already resubmitted, the resubmission window
//!   counted from the date of its last remittance (Asia/Dubai, inclusive).
//!   The task is due at the end of that day.
//! - A corrected amount is not part of the rejection: the draft is
//!   corrected line by line ([`crate::payer_split::correct_line`]) and the
//!   tax documents follow when it is issued again ([`crate::payer_tax`]).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{Transition, TransitionCtx, TransitionError, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::calendar::BusinessCalendar;
use wl_kernel::documents::today;
use wl_kernel::payer_bills::{PayerBillStatus, PayerBillSubmissionRejected, SubmissionError};
use wl_kernel::role_tasks::{self, RoleTask};

use crate::config_of;
use crate::credit_notes::{By, run_of};
use crate::payer_agreement::PayerAgreement;
use crate::payer_bill::PayerBill;
use crate::permissions;
use crate::texts::english;

/// The user transition that sends a rejected bill back to draft.
pub const SUBMISSION_REJECTED: &str = "submission_rejected";
/// The system twin of [`SUBMISSION_REJECTED`], for the vertical's channel.
pub const SUBMISSION_REJECTED_SYSTEM: &str = "submission_rejected_system";

/// One error the submission channel gave, kept with its attempt. Row of
/// `wl_invoicing_payer_submission_errors`.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_payer_submission_errors",
    index(payer_bill_id, attempt)
)]
pub struct PayerSubmissionError {
    /// The primary key.
    pub id: i64,
    /// The payer bill.
    pub payer_bill_id: i64,
    /// The submission the channel rejected: 1 for the first.
    pub attempt: i64,
    /// The channel's error code, e.g. `E104`.
    pub code: String,
    /// The channel's message, e.g. `invalid member ID`.
    pub message: String,
    /// The day it was recorded (Asia/Dubai).
    pub recorded_on: NaiveDate,
    /// The user who recorded it; `None` for the system.
    pub recorded_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PayerSubmissionError {
    /// The errors of payer bill `payer_bill_id`, oldest attempt first.
    pub async fn of_bill(db: &Db, payer_bill_id: i64) -> Result<Vec<PayerSubmissionError>> {
        PayerSubmissionError::query()
            .where_(PayerSubmissionError::PAYER_BILL_ID.eq(payer_bill_id))
            .order_by(PayerSubmissionError::ID)
            .get(db)
            .await
    }
}

/// What a rejection run carries to its guards and effect, gathered by
/// [`submission_rejected`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct RejectionInput {
    /// The channel's errors.
    pub errors: Vec<SubmissionError>,
    /// The user a system run acts for.
    pub by: Option<i64>,
}

/// The run's input, if it is a [`RejectionInput`].
fn input(ctx: &TransitionCtx) -> Option<RejectionInput> {
    ctx.input_as::<RejectionInput>().ok()
}

/// The `submission_rejected` transition and its system twin, for the
/// machine.
pub(crate) fn transitions() -> Vec<Transition<PayerBill>> {
    vec![
        rules(
            Transition::new(SUBMISSION_REJECTED)
                .label("Record rejection")
                .tone("danger")
                .permission(permissions::PAYER_BILLS_ISSUE),
        ),
        rules(
            Transition::new(SUBMISSION_REJECTED_SYSTEM)
                .label("Record rejection (system)")
                .system(),
        ),
    ]
}

/// What both transitions share: the guards and the effect.
fn rules(transition: Transition<PayerBill>) -> Transition<PayerBill> {
    transition
        .from([PayerBillStatus::Issued])
        .to(PayerBillStatus::Draft)
        .guard("errors_given", |_bill: &PayerBill, ctx| {
            let ok = input(ctx).is_some_and(|i| {
                !i.errors.is_empty()
                    && i.errors
                        .iter()
                        .all(|e| !e.code.trim().is_empty() && !e.message.trim().is_empty())
            });
            Box::pin(async move {
                if ok {
                    Ok(())
                } else {
                    Err(english("wl_invoicing.payer_bill.errors.rejection_errors").to_owned())
                }
            })
        })
        .guard("no_remittance_recorded", |bill: &PayerBill, _ctx| {
            let remitted = bill.has_remittance;
            Box::pin(async move {
                if remitted {
                    Err(
                        english("wl_invoicing.payer_bill.errors.rejection_after_remittance")
                            .to_owned(),
                    )
                } else {
                    Ok(())
                }
            })
        })
        .effect(|bill: &PayerBill, ctx, tx| {
            let id = bill.id;
            let attempt = bill.issue_count;
            let input = input(ctx).unwrap_or_default();
            let by = ctx.actor().map(|user| user.id).or(input.by);
            Box::pin(async move {
                for error in input.errors {
                    PayerSubmissionError {
                        payer_bill_id: id,
                        attempt,
                        code: error.code.trim().to_owned(),
                        message: error.message.trim().to_owned(),
                        recorded_on: today(),
                        recorded_by: by,
                        ..Default::default()
                    }
                    .insert(&mut *tx)
                    .await?;
                }
                // Not the status column, so outside the machine's guard. The
                // resubmission count stays: this is not a resubmission.
                PayerBill::query()
                    .where_(PayerBill::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[("in_ageing", &false), ("updated_at", &renox::db::now())],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// The last day the corrected bill can be sent (Asia/Dubai, inclusive): the
/// end of the first-submission window of the service, or, for a bill that
/// was resubmitted, the end of the resubmission window of its last
/// remittance. `None` when there is no date to count from.
pub fn deadline(bill: &PayerBill, agreement: &PayerAgreement) -> Option<NaiveDate> {
    if bill.resubmission_count > 0 {
        bill.last_remittance_on
            .and_then(|on| agreement.resubmission_deadline(on))
    } else {
        agreement.first_submission_deadline(bill.service_date)
    }
}

/// The end of `day` in Asia/Dubai, as an instant: when a deadline task is
/// due.
fn end_of(day: NaiveDate) -> Option<DateTime> {
    let local = day.and_hms_opt(23, 59, 0)?;
    Some(BusinessCalendar::default().from_local(local))
}

/// `1 Nov 2026`.
fn date_text(day: NaiveDate) -> String {
    day.format("%-d %b %Y").to_string()
}

/// `E104 invalid member ID; E7 missing field`.
fn errors_text(errors: &[PayerSubmissionError]) -> String {
    errors
        .iter()
        .map(|e| format!("{} {}", e.code, e.message))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The errors typed one per line as `CODE: message` (or `CODE message`).
/// A line without a message keeps the code as the message, so it is still
/// refused only when the code is missing.
pub fn parse_errors(text: &str) -> Vec<SubmissionError> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (code, message) = line
                .split_once(':')
                .or_else(|| line.split_once(char::is_whitespace))
                .unwrap_or((line, ""));
            let message = message.trim();
            SubmissionError::new(code, if message.is_empty() { code } else { message })
        })
        .collect()
}

/// Records the rejection of the issued payer bill `bill_id` as `by`, in its
/// own transaction, and answers the bill and the deadline of the corrected
/// submission. After the commit the coordinator's task is opened and
/// `PayerBillSubmissionRejected` is emitted.
///
/// # Errors
///
/// `Forbidden` without `payer_bills.issue`, `NotFrom` unless the bill is
/// `issued`, `Guard` (422) without an error, with an error lacking its code
/// or message, or once a remittance is recorded (the coordinator then gets a
/// task instead). Nothing is written then.
pub async fn submission_rejected(
    state: &AppState,
    by: By<'_>,
    bill_id: i64,
    errors: Vec<SubmissionError>,
) -> std::result::Result<(PayerBill, Option<NaiveDate>), TransitionError> {
    let bill = PayerBill::find_or_404(&state.db, bill_id).await?;
    let gathered = RejectionInput {
        errors: errors.clone(),
        by: by.id(),
    };
    let ran = run_of(
        &bill,
        &by,
        SUBMISSION_REJECTED,
        SUBMISSION_REJECTED_SYSTEM,
        json!({}),
    )
    .input(&gathered)
    .run(state)
    .await;
    if let Err(TransitionError::Guard(message)) = &ran
        && bill.has_remittance
    {
        open_refused_task(state, &bill, &errors, message).await?;
    }
    let _outcome = ran?;
    let bill = PayerBill::find_or_404(&state.db, bill_id).await?;
    let agreement = PayerAgreement::find_or_404(&state.db, bill.agreement_id).await?;
    let last_day = deadline(&bill, &agreement);
    Ok((bill, last_day))
}

/// The coordinator's task for a rejection that came after a remittance and
/// was refused.
async fn open_refused_task(
    state: &AppState,
    bill: &PayerBill,
    errors: &[SubmissionError],
    why: &str,
) -> Result {
    let text = errors
        .iter()
        .map(|e| format!("{} {}", e.code, e.message))
        .collect::<Vec<_>>()
        .join("; ");
    let number = bill.number.clone().unwrap_or_default();
    role_tasks::open(
        state,
        RoleTask::new(
            bill.subject(),
            &config_of(state).payer_bill_task_role,
            &format!("Rejection of payer bill {number} refused"),
            &format!(
                "invoicing.payer_bill_rejection_refused:{}:{}",
                bill.id, bill.issue_count
            ),
        )
        .description(&format!("{why} Channel errors: {text}")),
    )
    .await?;
    Ok(())
}

/// The key of the task of the rejection of attempt `attempt`.
pub fn task_key(bill_id: i64, attempt: i64) -> String {
    format!("invoicing.payer_bill_rejected:{bill_id}:{attempt}")
}

/// After the commit of a rejection: opens the billing coordinator's task
/// with the errors and the deadline, and emits
/// `PayerBillSubmissionRejected`.
pub(crate) async fn on_transition(state: &AppState, event: &Transitioned<PayerBill>) -> Result {
    if ![SUBMISSION_REJECTED, SUBMISSION_REJECTED_SYSTEM].contains(&event.transition.as_str()) {
        return Ok(());
    }
    let bill = PayerBill::find_or_404(&state.db, event.key).await?;
    let agreement = PayerAgreement::find_or_404(&state.db, bill.agreement_id).await?;
    let attempt = bill.issue_count;
    let errors: Vec<PayerSubmissionError> = PayerSubmissionError::of_bill(&state.db, bill.id)
        .await?
        .into_iter()
        .filter(|e| e.attempt == attempt)
        .collect();
    let last_day = deadline(&bill, &agreement);
    let number = bill.number.clone().unwrap_or_default();
    let deadline_text = last_day.map_or_else(
        || english("wl_invoicing.payer_bill.no_deadline").to_owned(),
        date_text,
    );
    let mut task = RoleTask::new(
        bill.subject(),
        &config_of(state).payer_bill_task_role,
        &format!("Payer bill {number} was rejected: fix and send it again"),
        &task_key(bill.id, attempt),
    )
    .description(&format!(
        "The submission channel rejected payer bill {number}: {}. Send it again by {deadline_text}.",
        errors_text(&errors)
    ));
    if let Some(due) = last_day.and_then(end_of) {
        task = task.due_at(due);
    }
    role_tasks::open(state, task).await?;
    state
        .emit(PayerBillSubmissionRejected {
            payer_bill: bill.payer_bill_id(),
            payer: bill.payer(),
            source: bill.source(),
            error_codes: errors.iter().map(|e| e.code.clone()).collect(),
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::state_machine::StateMachine;

    fn bill() -> PayerBill {
        PayerBill {
            status: PayerBillStatus::Issued,
            issue_count: 1,
            ..Default::default()
        }
    }

    fn ctx(errors: Vec<SubmissionError>) -> TransitionCtx {
        let input = RejectionInput { errors, by: None };
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    async fn refusal(bill: &PayerBill, ctx: &TransitionCtx) -> Option<String> {
        PayerBill::machine()
            .find(SUBMISSION_REJECTED)
            .unwrap()
            .check_guards(bill, ctx)
            .await
            .err()
            .map(|e| e.to_string())
    }

    #[test]
    fn the_rejection_leaves_issued_for_draft_and_has_a_system_twin() {
        let machine = PayerBill::machine();
        for (name, system) in [
            (SUBMISSION_REJECTED, false),
            (SUBMISSION_REJECTED_SYSTEM, true),
        ] {
            let transition = machine.find(name).unwrap();
            assert_eq!(transition.from_states(), ["issued"]);
            assert_eq!(transition.to_state(), Some("draft"));
            assert_eq!(transition.is_system(), system);
            assert_eq!(
                transition.guard_names(),
                ["errors_given", "no_remittance_recorded"]
            );
        }
    }

    #[renox::test]
    async fn an_error_with_its_code_and_message_is_needed() {
        let ok = vec![SubmissionError::new("E104", "invalid member ID")];
        assert_eq!(refusal(&bill(), &ctx(ok)).await, None);
        assert!(refusal(&bill(), &ctx(vec![])).await.is_some());
        let blank = vec![SubmissionError::new("E104", " ")];
        assert!(refusal(&bill(), &ctx(blank)).await.is_some());
        assert!(refusal(&bill(), &TransitionCtx::new(None)).await.is_some());
    }

    #[renox::test]
    async fn a_rejection_after_a_remittance_is_refused() {
        let remitted = PayerBill {
            has_remittance: true,
            ..bill()
        };
        let ok = vec![SubmissionError::new("E104", "invalid member ID")];
        assert!(refusal(&remitted, &ctx(ok)).await.is_some());
    }

    #[test]
    fn typed_errors_are_read_one_per_line() {
        let errors = parse_errors("E104: invalid member ID\n\n E7 missing field \nE9");
        assert_eq!(
            errors,
            [
                SubmissionError::new("E104", "invalid member ID"),
                SubmissionError::new("E7", "missing field"),
                SubmissionError::new("E9", "E9"),
            ]
        );
    }

    #[test]
    fn the_deadline_follows_the_attempt() {
        let agreement = PayerAgreement {
            first_submission_days: 30,
            resubmission_days: 15,
            ..Default::default()
        };
        let first = PayerBill {
            service_date: "2026-11-01".parse().unwrap(),
            ..bill()
        };
        assert_eq!(
            deadline(&first, &agreement),
            Some("2026-12-01".parse().unwrap())
        );
        let again = PayerBill {
            resubmission_count: 1,
            last_remittance_on: Some("2026-12-10".parse().unwrap()),
            ..first
        };
        assert_eq!(
            deadline(&again, &agreement),
            Some("2026-12-25".parse().unwrap())
        );
    }
}
