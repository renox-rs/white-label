//! Cancelling a job card after the work started (08 §4.4 row *The customer
//! stops the job…*, §7.1 *Job card* `cancel`, `AUT.ACL.21`; BL-AUT-01).
//!
//! The service advisor cancels with a reason, confirms the work done per line
//! and takes the customer's acknowledgement of the summary. The customer is
//! billed for the work done only: the labour done (a line part done at the
//! hours done, [`crate::stop`]), the parts fitted and the special orders the
//! customer agreed to; the parts issued and not fitted go back to stock, the
//! booking is cancelled with the work done billed on the job card, and the
//! purchaser gets a task to revise an open purchase order of a special part.
//!
//! In this order, after the run committed ([`finish`]), each step once:
//!
//! 1. the parts issued and not fitted go back to the store, and the pending
//!    issues of the job (what the store could not give) are cleared;
//! 2. the invoice for the work done is issued to the customer
//!    ([`crate::stop_billing`]); none for a total loss, where the work done goes
//!    to the claim (BL-AUT-09);
//! 3. a task for the purchaser to revise the purchase order of each special
//!    part that was billed;
//! 4. the booking is cancelled for the advisor with the reason `job_stopped`
//!    (`SCH.ACL.06`); it asks the job card what was done ([`JobWork`]);
//! 5. [`JobCardCancelled`] is emitted, once.
//!
//! Decisions:
//!
//! - Billing less than the work done, even by AED 0.01, or a customer who
//!   refuses to sign, needs the branch manager (`AUT.ACL.21`): the kernel
//!   defers the cancel as an approval request; the job card keeps its state,
//!   nothing is invoiced or returned, and the technician's job card refuses new
//!   labour and parts while it waits ([`cancel_pending`]). The advisor who asked
//!   never decides it.
//! - The work done per line is carried in the run's input and written by the
//!   cancel's effect, so a rejected request leaves the lines as they were.
//! - A job funded by an open insurance claim is not cancelled by the advisor:
//!   the claim's cancel or total loss ends it (BL-AUT-08, BL-AUT-09), through
//!   [`cancel_by_system`].
//! - Payment of the invoice and the release of the vehicle follow in
//!   [`crate::release`].

use renox::db::{Transaction, now};
use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine, TransitionError, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::submit::{REQUEST_KEY, pending_for_action};
use wl_kernel::approvals::{ApprovalPolicy, announce};
use wl_kernel::documents::today;
use wl_kernel::inventory::{PURCHASE_ORDER_SUBJECT_TYPE, stock_service};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::scheduling::{
    BookingId, SchedulingFuture, WorkDone, WorkDoneStatus, booking_actions,
    provide_work_done_status,
};
use wl_kernel::Subject;

use crate::claim::InsuranceClaim;
use crate::db::{refused, set_job_card};
use crate::estimate::Estimate;
use crate::job_card::{JOB_CARD_SUBJECT_TYPE, JobCard, JobCardStatus};
use crate::job_card_machine::{
    CANCEL, CANCEL_ACTION, CANCEL_APPROVED, CANCEL_SYSTEM, JobInput,
};
use crate::labour_line::{LabourLine, LineStatus, WorkStatus};
use crate::part_line::PartLine;
use crate::permissions;
use crate::reasons::JOB_STOPPED;
use crate::stop::{
    self, CLAIM_WITHDRAWN, INSURER_NOTICE, LabourWork, PartWork, REFUSED_TO_SIGN, SIGNED,
    StopRecord, TOTAL_LOSS, USER_CODES, WRITTEN_WITHDRAWAL,
};
use crate::texts::english;

/// The job card was cancelled after the work started (08 §7.1 `cancel`). It is
/// emitted once, after the invoice, the stock and the booking were done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobCardCancelled {
    /// The job card.
    pub job_card: Subject,
    /// Its number, `JOB-00010`.
    pub number: String,
    /// Why: `customer_stopped`, `claim_withdrawn` or `total_loss`.
    pub reason_code: String,
    /// The value of the work done before VAT, in fils.
    pub work_done_value: i64,
    /// What was billed before VAT, in fils.
    pub billed: i64,
    /// The invoice for the work done, when there is one.
    pub invoice: Option<i64>,
    /// The booking that was cancelled with it, when there is one.
    pub booking: Option<i64>,
}

impl Event for JobCardCancelled {}

/// What the advisor gives to cancel a job card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StopRequest {
    /// The reason code ([`USER_CODES`]).
    pub code: String,
    /// More about the reason, if any.
    pub note: Option<String>,
    /// The work done per labour line, for the lines the advisor changes; the
    /// others keep what the technician recorded.
    pub labour: Vec<LabourWork>,
    /// The parts fitted per part line, for the lines the advisor changes.
    pub parts: Vec<PartWork>,
    /// How the customer acknowledged: [`SIGNED`], [`REFUSED_TO_SIGN`],
    /// [`INSURER_NOTICE`] or [`WRITTEN_WITHDRAWAL`].
    pub ack_method: String,
    /// The customer who signed.
    pub ack_name: Option<String>,
    /// The customer's signature, as the form sent it.
    pub ack_signature: Option<String>,
    /// The staff witness of a refusal to sign.
    pub ack_witness: Option<String>,
    /// The insurer's notice or the written withdrawal.
    pub ack_document: Option<String>,
    /// What to bill before VAT, in fils; the work done when `None`.
    pub bill: Option<i64>,
}

impl StopRequest {
    /// A cancel for the reason `code`.
    pub fn new(code: &str) -> Self {
        StopRequest {
            code: code.trim().to_owned(),
            ..StopRequest::default()
        }
    }

    /// The customer `name` signed the summary of the work done.
    #[must_use]
    pub fn signed(mut self, name: &str) -> Self {
        self.ack_method = SIGNED.to_owned();
        self.ack_name = Some(name.trim().to_owned());
        self
    }

    /// With the customer's signature.
    #[must_use]
    pub fn signature(mut self, signature: &str) -> Self {
        self.ack_signature = Some(signature.trim().to_owned());
        self
    }

    /// The customer refused to sign; `witness` is the member of staff who saw
    /// it.
    #[must_use]
    pub fn refused(mut self, witness: &str) -> Self {
        self.ack_method = REFUSED_TO_SIGN.to_owned();
        self.ack_witness = Some(witness.trim().to_owned());
        self
    }

    /// The customer's written withdrawal `document` replaces the signature.
    #[must_use]
    pub fn withdrawal(mut self, document: &str) -> Self {
        self.ack_method = WRITTEN_WITHDRAWAL.to_owned();
        self.ack_document = Some(document.trim().to_owned());
        self
    }

    /// With this note.
    #[must_use]
    pub fn note(mut self, note: &str) -> Self {
        self.note = Some(note.trim().to_owned()).filter(|n| !n.is_empty());
        self
    }

    /// Billing `net` fils before VAT, below the work done when it is below it.
    #[must_use]
    pub fn bill(mut self, net: i64) -> Self {
        self.bill = Some(net);
        self
    }

    /// The labour line `line` stands as `work` with `hours_milli` worked.
    #[must_use]
    pub fn labour(mut self, line: i64, work: WorkStatus, hours_milli: i64) -> Self {
        self.labour.push(LabourWork {
            line,
            work,
            hours_milli,
        });
        self
    }

    /// `fitted_milli` of the part line `line` was fitted.
    #[must_use]
    pub fn fitted(mut self, line: i64, fitted_milli: i64) -> Self {
        self.parts.push(PartWork { line, fitted_milli });
        self
    }
}

/// What a cancel came to.
#[derive(Debug, Clone)]
pub struct Stopped {
    /// The job card: `cancelled`, or as it was while the branch manager
    /// decides.
    pub card: JobCard,
    /// The approval request that waits for the branch manager.
    pub approval_request: Option<i64>,
}

/// Whether a cancel of job card `job_card_id` waits for the branch manager's
/// approval: the technician's job card then takes no new labour and parts.
///
/// # Errors
///
/// The database's errors.
pub async fn cancel_pending(db: &Db, job_card_id: i64) -> Result<bool> {
    let subject = Subject::new(JOB_CARD_SUBJECT_TYPE, job_card_id);
    Ok(pending_for_action(db, &subject, CANCEL_ACTION)
        .await?
        .is_some())
}

/// Refuses new labour, parts and work records while a cancel waits for its
/// approval.
///
/// # Errors
///
/// A 422 in words while the cancel waits.
pub async fn ensure_not_pending(db: &Db, job_card_id: i64) -> Result {
    if cancel_pending(db, job_card_id).await? {
        return Err(refused(english("auto.stop.problem.pending_lines", &[])));
    }
    Ok(())
}

/// The reason as it is stored and shown: the label of the code, then the note.
pub fn reason_text(code: &str, note: &Option<String>) -> String {
    let label = english(&format!("auto.stop.code.{code}"), &[]);
    match note.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(note) => format!("{label}: {note}"),
        None => label,
    }
}

/// Why `card` cannot be cancelled now, in words; `None` when it can.
fn status_problem(card: &JobCard) -> Option<&'static str> {
    match card.status {
        JobCardStatus::Inspecting
        | JobCardStatus::AwaitingApproval
        | JobCardStatus::InProgress
        | JobCardStatus::AwaitingParts
        | JobCardStatus::QualityCheck => None,
        JobCardStatus::CheckedIn => Some("auto.stop.problem.checked_in"),
        JobCardStatus::Ready => Some("auto.stop.problem.ready"),
        JobCardStatus::Delivered
        | JobCardStatus::Declined
        | JobCardStatus::Cancelled
        | JobCardStatus::Voided => Some("auto.stop.problem.closed"),
    }
}

/// The acknowledgement's problem in words, `None` when it is complete.
fn ack_problem(record: &StopRecord) -> Option<&'static str> {
    if record.acknowledged() {
        return None;
    }
    Some(match record.ack_method.as_str() {
        SIGNED => "auto.stop.problem.ack_signed",
        REFUSED_TO_SIGN => "auto.stop.problem.ack_refused",
        INSURER_NOTICE | WRITTEN_WITHDRAWAL => "auto.stop.problem.ack_document",
        _ => "auto.stop.problem.ack_method",
    })
}

/// The record of the work done: the lines of the request and, for every other
/// approved line, what the technician recorded.
fn work_record(
    estimate: &Estimate,
    request: &StopRequest,
) -> std::result::Result<(Vec<LabourWork>, Vec<PartWork>), &'static str> {
    let mut labour = Vec::new();
    for line in estimate
        .labour
        .iter()
        .filter(|l| l.approval == LineStatus::Approved)
    {
        let named = request.labour.iter().rev().find(|w| w.line == line.id);
        let entry = match named {
            None => LabourWork {
                line: line.id,
                work: line.work,
                hours_milli: line.hours_done_milli,
            },
            Some(work) => match work.work {
                WorkStatus::NotDone => LabourWork {
                    line: line.id,
                    work: WorkStatus::NotDone,
                    hours_milli: 0,
                },
                WorkStatus::Done => LabourWork {
                    line: line.id,
                    work: WorkStatus::Done,
                    hours_milli: line.hours_milli,
                },
                WorkStatus::PartDone => {
                    if work.hours_milli <= 0 || work.hours_milli >= line.hours_milli {
                        return Err("auto.stop.problem.part_hours");
                    }
                    LabourWork {
                        line: line.id,
                        work: WorkStatus::PartDone,
                        hours_milli: work.hours_milli,
                    }
                }
            },
        };
        labour.push(entry);
    }
    if request.labour.iter().any(|w| {
        !estimate
            .labour
            .iter()
            .any(|l| l.id == w.line && l.approval == LineStatus::Approved)
    }) {
        return Err("auto.stop.problem.line");
    }
    let mut parts = Vec::new();
    for line in estimate
        .parts
        .iter()
        .filter(|l| l.approval == LineStatus::Approved)
    {
        let named = request.parts.iter().rev().find(|p| p.line == line.id);
        let fitted = match named {
            None => line.fitted_milli,
            Some(part) => {
                if part.fitted_milli < 0 || part.fitted_milli > line.issued_milli {
                    return Err("auto.stop.problem.fitted");
                }
                part.fitted_milli
            }
        };
        parts.push(PartWork {
            line: line.id,
            fitted_milli: fitted,
        });
    }
    if request.parts.iter().any(|p| {
        !estimate
            .parts
            .iter()
            .any(|l| l.id == p.line && l.approval == LineStatus::Approved)
    }) {
        return Err("auto.stop.problem.line");
    }
    Ok((labour, parts))
}

/// `cancel`: the advisor stops the job after the work started. A bill below
/// the work done, or a customer who refuses to sign, waits for the branch
/// manager (`AUT.ACL.21`); otherwise the job card is `cancelled` and the
/// invoice, the stock and the booking follow ([`finish`]).
///
/// # Errors
///
/// A 403 without `job_cards.advise`; a 422 in words for a job card that cannot
/// be cancelled (`checked_in`: void or decline; `ready`: deliver; closed), a
/// cancel already waiting, a reason that is not one of the list, a job funded
/// by an open insurance claim, a line that is not on the estimate, hours or a
/// quantity that cannot be, a bill above the work done, or an acknowledgement
/// that is not complete.
pub async fn cancel(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    request: StopRequest,
) -> Result<Stopped> {
    if !by.has_permission(permissions::JOB_CARDS_ADVISE) {
        return Err(Error::Forbidden);
    }
    let card = JobCard::find_or_404(&state.db, id).await?;
    if let Some(key) = status_problem(&card) {
        return Err(refused(english(key, &[])));
    }
    if cancel_pending(&state.db, card.id).await? {
        return Err(refused(english("auto.stop.problem.pending", &[])));
    }
    if !USER_CODES.contains(&request.code.as_str()) {
        return Err(refused(english("auto.stop.problem.code", &[])));
    }
    if let Some(claim) = InsuranceClaim::funding(&state.db, card.id).await? {
        let number = claim.number();
        return Err(refused(english(
            "auto.stop.problem.claim",
            &[("claim", &number)],
        )));
    }
    let estimate = Estimate::load(&state.db, card.id).await?;
    let (labour, parts) = match work_record(&estimate, &request) {
        Ok(found) => found,
        Err(key) => return Err(refused(english(key, &[]))),
    };
    let mut record = StopRecord {
        code: request.code.clone(),
        labour,
        parts,
        work_recorded: true,
        ack_method: request.ack_method.clone(),
        ack_name: request.ack_name.clone().filter(|t| !t.trim().is_empty()),
        ack_signature: request.ack_signature.clone().filter(|t| !t.trim().is_empty()),
        ack_witness: request.ack_witness.clone().filter(|t| !t.trim().is_empty()),
        ack_document: request.ack_document.clone().filter(|t| !t.trim().is_empty()),
        work_value: 0,
        billed: 0,
    };
    if let Some(key) = ack_problem(&record) {
        return Err(refused(english(key, &[])));
    }
    let after = stop::applied(&estimate, &record);
    record.work_value = stop::value_of(&card, &after);
    record.billed = match request.bill {
        None => record.work_value,
        Some(bill) if bill < 0 => {
            return Err(refused(english("auto.stop.problem.bill_negative", &[])));
        }
        Some(bill) if bill > record.work_value => {
            let value = crate::money_text(record.work_value);
            return Err(refused(english(
                "auto.stop.problem.bill_over",
                &[("value", &value)],
            )));
        }
        Some(bill) => bill,
    };
    let reason = reason_text(&record.code, &request.note);
    let mut input = JobInput {
        by: Some(by.id),
        on: Some(today()),
        reason: Some(reason.clone()),
        needs_approval: Some(i64::from(record.needs_approval())),
        stop: Some(record),
        ..JobInput::default()
    };
    // A policy taken away must not let a lowered bill through unapproved.
    let value = renox::serde_json::to_value(&input).unwrap_or(renox::serde_json::Value::Null);
    input.approval_ready = ApprovalPolicy::for_action(&state.db, CANCEL_ACTION, &value)
        .await?
        .is_some();
    let outcome = card
        .transition(CANCEL)
        .by(by)
        .reason(reason)
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    announce(state, &outcome).await?;
    let approval_request = match &outcome {
        Outcome::Pending(pending) => pending
            .data
            .get(REQUEST_KEY)
            .and_then(renox::serde_json::Value::as_i64),
        Outcome::Done(_) => None,
    };
    Ok(Stopped {
        card: JobCard::find_or_404(&state.db, id).await?,
        approval_request,
    })
}

/// `cancel` as the system: a claim that was withdrawn or a total loss
/// (BL-AUT-08, BL-AUT-09). The insurer's notice or the customer's written
/// withdrawal `document` replaces the signature; the work done stands as the
/// technician recorded it and is billed in full (to the customer for a
/// withdrawal, to the claim for a total loss).
///
/// # Errors
///
/// A 422 for a job card that cannot be cancelled, a code that is not
/// [`CLAIM_WITHDRAWN`] or [`TOTAL_LOSS`], or a blank document.
pub async fn cancel_by_system(
    state: &AppState,
    id: i64,
    code: &str,
    document: &str,
) -> Result<JobCard> {
    let card = JobCard::find_or_404(&state.db, id).await?;
    if let Some(key) = status_problem(&card) {
        return Err(refused(english(key, &[])));
    }
    if code != CLAIM_WITHDRAWN && code != TOTAL_LOSS {
        return Err(refused(english("auto.stop.problem.code", &[])));
    }
    let estimate = Estimate::load(&state.db, card.id).await?;
    let request = StopRequest::new(code);
    let (labour, parts) = match work_record(&estimate, &request) {
        Ok(found) => found,
        Err(key) => return Err(refused(english(key, &[]))),
    };
    let method = if code == TOTAL_LOSS {
        INSURER_NOTICE
    } else {
        WRITTEN_WITHDRAWAL
    };
    let mut record = StopRecord {
        code: code.to_owned(),
        labour,
        parts,
        work_recorded: true,
        ack_method: method.to_owned(),
        ack_document: Some(document.trim().to_owned()).filter(|d| !d.is_empty()),
        ..StopRecord::default()
    };
    if let Some(key) = ack_problem(&record) {
        return Err(refused(english(key, &[])));
    }
    let after = stop::applied(&estimate, &record);
    record.work_value = stop::value_of(&card, &after);
    record.billed = record.work_value;
    let reason = reason_text(code, &None);
    let input = JobInput {
        on: Some(today()),
        reason: Some(reason.clone()),
        needs_approval: Some(0),
        stop: Some(record),
        ..JobInput::default()
    };
    crate::job_cards::run_system(state, &card, CANCEL_SYSTEM, Some(reason.as_str()), input).await
}

/// What the cancel's effect writes, inside its transaction: the work done per
/// line, the special orders handed over, the round the customer never
/// answered ended, and the job card's reason, acknowledgement and amounts.
pub(crate) async fn record_in(
    tx: &mut Transaction,
    card: &JobCard,
    input: &JobInput,
    reason: Option<String>,
) -> Result {
    let Some(stop) = &input.stop else {
        return Ok(());
    };
    let stamp = now();
    for work in &stop.labour {
        LabourLine::query()
            .where_(LabourLine::ID.eq(work.line))
            .where_(LabourLine::JOB_CARD_ID.eq(card.id))
            .update(
                &mut *tx,
                &[
                    ("work", &work.work),
                    ("hours_done_milli", &work.hours_milli),
                    ("updated_at", &stamp),
                ],
            )
            .await?;
    }
    for part in &stop.parts {
        PartLine::query()
            .where_(PartLine::ID.eq(part.line))
            .where_(PartLine::JOB_CARD_ID.eq(card.id))
            .update(
                &mut *tx,
                &[
                    ("fitted_milli", &part.fitted_milli),
                    ("updated_at", &stamp),
                ],
            )
            .await?;
    }
    // A special order the customer agreed to is billed and handed over with
    // the vehicle.
    PartLine::query()
        .where_(PartLine::JOB_CARD_ID.eq(card.id))
        .where_(PartLine::APPROVAL.eq(LineStatus::Approved))
        .where_(PartLine::SPECIAL_ORDER.eq(true))
        .where_(PartLine::SPECIAL_ORDER_AGREED.eq(true))
        .update(
            &mut *tx,
            &[("handed_over_at", &Some(stamp)), ("updated_at", &stamp)],
        )
        .await?;
    // The round the customer never answered ends with the job card.
    crate::approval_rounds::supersede(tx, card.id).await?;
    let none: Option<renox::db::DateTime> = None;
    set_job_card(
        tx,
        card.id,
        &[
            ("closed_reason", &reason),
            ("closed_code", &Some(stop.code.clone())),
            ("closed_at", &Some(stamp)),
            ("awaiting_since", &none),
            ("stopped_by", &input.by),
            ("ack_method", &Some(stop.ack_method.clone())),
            ("ack_name", &stop.ack_name),
            ("ack_signature", &stop.ack_signature),
            ("ack_witness", &stop.ack_witness),
            ("ack_document", &stop.ack_document),
            ("ack_at", &Some(stamp)),
            ("work_done_value", &stop.work_value),
            ("billed_net", &stop.billed),
            ("bill_lowered", &stop.lowered()),
        ],
    )
    .await
}

/// Names the purchase order (wl-inventory) a special-order part was ordered
/// on, or clears it: a cancelled job that bills the part gives the purchaser a
/// task to revise that order.
///
/// # Errors
///
/// A 403 without `job_cards.advise`; a 422 for a part that is not a special
/// order or for a job card that is over.
pub async fn link_purchase_order(
    state: &AppState,
    by: &AuthUser,
    line_id: i64,
    order: Option<i64>,
) -> Result<PartLine> {
    if !by.has_permission(permissions::JOB_CARDS_ADVISE) {
        return Err(Error::Forbidden);
    }
    let line = PartLine::find_or_404(&state.db, line_id).await?;
    let card = JobCard::find_or_404(&state.db, line.job_card_id).await?;
    if card.status.is_closed() {
        return Err(refused(english("auto.stop.problem.closed", &[])));
    }
    if !line.special_order {
        return Err(refused(english("auto.stop.problem.not_special", &[])));
    }
    PartLine::query()
        .where_(PartLine::ID.eq(line.id))
        .update(
            &state.db,
            &[("purchase_order_id", &order), ("updated_at", &now())],
        )
        .await?;
    PartLine::find_or_404(&state.db, line.id).await
}

/// The user a booking is cancelled for: who asked for the cancel, else the
/// advisor of the job card.
fn staff_of(card: &JobCard) -> Option<i64> {
    card.stopped_by
        .or(card.advisor_user_id)
        .or(card.intake_by)
        .or(card.created_by)
}

/// Cancels the booking of the closed job card `card` for the staff user who
/// closed it, with the reason code `code`. Once: the card keeps the time.
///
/// # Errors
///
/// The database's and the scheduling contract's errors.
pub(crate) async fn close_booking(state: &AppState, card: &JobCard, code: &str) -> Result<bool> {
    let Some(booking) = card.booking_id else {
        return Ok(false);
    };
    if card.booking_cancelled_at.is_some() {
        return Ok(false);
    }
    let Some(user) = staff_of(card) else {
        return Ok(false);
    };
    let cancelled = booking_actions(state)
        .cancel_booking_by_staff(state, BookingId(booking), code, user)
        .await?;
    if cancelled {
        JobCard::query()
            .where_(JobCard::ID.eq(card.id))
            .update(
                &state.db,
                &[
                    ("booking_cancelled_at", &Some(now())),
                    ("updated_at", &now()),
                ],
            )
            .await?;
    }
    Ok(cancelled)
}

/// A task for the purchaser to revise the purchase order of each special part
/// the customer was billed for: the order itself is not touched
/// (`AUT.ACL.21`, BL-STK-08). Once per part: the task is found by its key.
async fn purchase_order_tasks(state: &AppState, card: &JobCard) -> Result {
    let estimate = Estimate::load(&state.db, card.id).await?;
    for line in estimate.parts.iter().filter(|l| {
        l.approval == LineStatus::Approved
            && l.special_order
            && l.special_order_agreed
            && l.purchase_order_id.is_some()
    }) {
        let Some(order) = line.purchase_order_id else {
            continue;
        };
        let (title, description) = {
            let number = card.number();
            let params: [(&str, &dyn std::fmt::Display); 2] =
                [("job", &number), ("part", &line.description)];
            (
                english("auto.stop.po_task_title", &params),
                english("auto.stop.po_task_body", &params),
            )
        };
        let key = format!("auto.po_revise:{}:{}", card.id, line.id);
        let task = RoleTask::new(
            Subject::new(PURCHASE_ORDER_SUBJECT_TYPE, order),
            permissions::PURCHASER_ROLE,
            &title,
            &key,
        )
        .description(&description);
        role_tasks::open(state, task).await?;
    }
    Ok(())
}

/// What follows a cancel that committed: the stock, the invoice, the
/// purchaser's task, the booking and the event, each once. A step that failed
/// is done by the next call.
///
/// # Errors
///
/// The database's, the stock's, the invoicer's and the scheduling contract's
/// errors.
pub async fn finish(state: &AppState, job_card_id: i64) -> Result<JobCard> {
    let card = JobCard::find_or_404(&state.db, job_card_id).await?;
    if card.status != JobCardStatus::Cancelled || card.closing_done_at.is_some() {
        return Ok(card);
    }
    crate::parts::return_unfitted(state, card.id).await?;
    stock_service(state)
        .source_cancelled(state, &card.subject())
        .await?;
    let total_loss = card.closed_code.as_deref() == Some(TOTAL_LOSS);
    let invoice = if total_loss {
        None
    } else {
        crate::stop_billing::bill_closed(state, card.id)
            .await?
            .map(|id| id.0)
    };
    let card = JobCard::find_or_404(&state.db, job_card_id).await?;
    purchase_order_tasks(state, &card).await?;
    close_booking(state, &card, JOB_STOPPED).await?;
    let claimed = JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .where_(JobCard::CLOSING_DONE_AT.is_null())
        .update(
            &state.db,
            &[("closing_done_at", &Some(now())), ("updated_at", &now())],
        )
        .await?;
    let done = JobCard::find_or_404(&state.db, job_card_id).await?;
    if claimed > 0 {
        state
            .emit(JobCardCancelled {
                job_card: done.subject(),
                number: done.number(),
                reason_code: done.closed_code.clone().unwrap_or_default(),
                work_done_value: done.work_done_value,
                billed: done.billed_net,
                invoice,
                booking: done.booking_id,
            })
            .await?;
    }
    Ok(done)
}

/// What the job card says about the work done for its booking, as
/// wl-scheduling asks when the booking is cancelled after the check-in
/// (`SCH.ACL.06`):
///
/// - no job card: nothing;
/// - open (not decided yet): pending, so the booking cannot be cancelled;
/// - voided: nothing; declined or cancelled: what was billed, with VAT, or
///   nothing when nothing was;
/// - delivered: what the customer approved, with VAT.
///
/// # Errors
///
/// The database's errors.
pub async fn work_done(state: &AppState, booking: BookingId) -> Result<WorkDone> {
    let Some(card) = JobCard::of_booking(&state.db, booking.0).await? else {
        return Ok(WorkDone::Nothing);
    };
    Ok(match card.status {
        JobCardStatus::Voided => WorkDone::Nothing,
        JobCardStatus::Declined | JobCardStatus::Cancelled => {
            if card.billed_net > 0 {
                WorkDone::Captured {
                    amount: stop::gross_of_net(card.billed_net),
                }
            } else {
                WorkDone::Nothing
            }
        }
        JobCardStatus::Delivered => {
            let estimate = Estimate::load(&state.db, card.id).await?;
            WorkDone::Captured {
                amount: estimate.approved_totals().gross(),
            }
        }
        _ => WorkDone::Pending,
    })
}

/// The app's [`WorkDoneStatus`]: the answer comes from the job card the
/// check-in opened.
#[derive(Debug, Clone, Copy, Default)]
pub struct JobWork;

impl WorkDoneStatus for JobWork {
    fn status<'a>(
        &'a self,
        state: &'a AppState,
        booking: BookingId,
        _record: Option<Subject>,
    ) -> SchedulingFuture<'a, WorkDone> {
        Box::pin(async move { work_done(state, booking).await })
    }
}

/// Registers the work done answer and the listener of the cancel.
pub(crate) fn register(app: &mut Registry) {
    provide_work_done_status(app, JobWork);
    app.listen(|event: Transitioned<JobCard>, state: AppState| async move {
        match event.transition.as_str() {
            CANCEL | CANCEL_APPROVED | CANCEL_SYSTEM => finish(&state, event.key).await.map(|_| ()),
            _ => Ok(()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stop::CUSTOMER_STOPPED;

    #[test]
    fn a_reason_reads_its_label_then_its_note() {
        assert_eq!(
            reason_text(CUSTOMER_STOPPED, &None),
            "The customer stopped the job"
        );
        assert_eq!(
            reason_text(CLAIM_WITHDRAWN, &Some(" sold the car ".to_owned())),
            "The claim was withdrawn: sold the car"
        );
    }

    #[test]
    fn only_a_job_that_started_can_be_cancelled() {
        let card = |status| JobCard {
            status,
            ..JobCard::default()
        };
        for open in [
            JobCardStatus::Inspecting,
            JobCardStatus::AwaitingApproval,
            JobCardStatus::InProgress,
            JobCardStatus::AwaitingParts,
            JobCardStatus::QualityCheck,
        ] {
            assert_eq!(status_problem(&card(open)), None, "{open:?}");
        }
        assert_eq!(
            status_problem(&card(JobCardStatus::CheckedIn)),
            Some("auto.stop.problem.checked_in")
        );
        assert_eq!(
            status_problem(&card(JobCardStatus::Ready)),
            Some("auto.stop.problem.ready")
        );
        assert_eq!(
            status_problem(&card(JobCardStatus::Delivered)),
            Some("auto.stop.problem.closed")
        );
    }

    #[test]
    fn the_work_of_a_line_the_request_names_is_checked() {
        let line = LabourLine {
            id: 4,
            hours_milli: 2_000,
            approval: LineStatus::Approved,
            ..LabourLine::default()
        };
        let estimate = Estimate {
            labour: vec![line],
            parts: Vec::new(),
        };
        let ok = StopRequest::new(CUSTOMER_STOPPED).labour(4, WorkStatus::PartDone, 500);
        let (labour, _) = work_record(&estimate, &ok).unwrap();
        assert_eq!(labour[0].hours_milli, 500);
        let all = StopRequest::new(CUSTOMER_STOPPED).labour(4, WorkStatus::PartDone, 2_000);
        assert_eq!(
            work_record(&estimate, &all).unwrap_err(),
            "auto.stop.problem.part_hours"
        );
        let unknown = StopRequest::new(CUSTOMER_STOPPED).labour(9, WorkStatus::Done, 0);
        assert_eq!(
            work_record(&estimate, &unknown).unwrap_err(),
            "auto.stop.problem.line"
        );
        let done = StopRequest::new(CUSTOMER_STOPPED).labour(4, WorkStatus::Done, 0);
        let (labour, _) = work_record(&estimate, &done).unwrap();
        assert_eq!(labour[0].hours_milli, 2_000, "a line done is all its hours");
    }
}
