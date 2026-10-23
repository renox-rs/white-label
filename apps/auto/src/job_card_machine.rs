//! The job card machine (08 §7.1 *Job card*; `AUT.JOB.01`, `AUT.JOB.02`,
//! `AUT.ACL.09`).
//!
//! | Transition | From | To | By | Guards |
//! |---|---|---|---|---|
//! | [`START_INSPECTION`] | checked_in | inspecting | `job_cards.work` | `technician_assigned`, `intake_recorded` |
//! | [`SEND_ESTIMATE`] | inspecting, in_progress | awaiting_approval | `job_cards.advise` | `intake_recorded`, `estimate_priced` |
//! | [`APPROVE`] / [`APPROVE_SYSTEM`] | awaiting_approval | in_progress | `job_cards.advise` (recorded approval) / system (the customer's link, a claim) | `estimate_pending`, `police_report` (`AUT.ACL.20`, BL-AUT-07) |
//! | [`DECLINE`] / [`DECLINE_SYSTEM`] | checked_in, inspecting, awaiting_approval | declined | `job_cards.advise` / system (estimate expired) | `nothing_approved`, `nothing_recorded` |
//! | [`DECLINE_ADDED`] / [`DECLINE_ADDED_SYSTEM`] | awaiting_approval | in_progress | `job_cards.advise` / system (the customer's link) | `work_approved` |
//! | [`WAIT_PARTS`] | in_progress | awaiting_parts | `job_cards.work` | `parts_requested` |
//! | [`PARTS_ARRIVED`] | awaiting_parts | in_progress | system (stock issued) | `stock_issued` (slot) |
//! | [`FINISH_WORK`] | in_progress | quality_check | `job_cards.work` | `labour_done` |
//! | [`PASS_QC`] | quality_check | ready | `job_cards.qc` | `qc_checklist_complete`, `test_drives_closed` (BL-AUT-06), `qc_findings_closed` (slot) |
//! | [`FAIL_QC`] | quality_check | in_progress | `job_cards.qc` | a reason (the findings) |
//! | [`DELIVER`] | ready | delivered | `job_cards.advise` | `qc_checklist_complete`, `added_work_approved`, `substitute_closed` and `payment_cleared` (slot) |
//! | [`CANCEL`] / [`CANCEL_APPROVED`] / [`CANCEL_SYSTEM`] | inspecting, awaiting_approval, in_progress, awaiting_parts, quality_check | cancelled | `job_cards.advise`, approved by the branch manager (`AUT.ACL.21`) / the kernel after the approval / system (a claim) | a reason; `work_done_recorded`, `acknowledged`, `approved_if_below_work_done` |
//! | [`VOID`] / `void_approved` | checked_in | voided | `job_cards.advise` (the kernel's void) | a reason; no labour or part line and no open claim (the reverse handler, [`crate::job_void`]) |
//!
//! Decisions:
//!
//! - As for the batch of 10, the guards see only the record and the run's
//!   input; [`crate::job_cards`] works out what a guard needs from the
//!   database (the estimate is priced, the approval round that is pending, the
//!   labour still open) and passes it in [`JobInput`].
//! - A transition has one kind of access and one end state: the user and the
//!   system variants are declared twice (`approve` and `approve_system`), and
//!   `decline` (the job card ends) and `decline_added` (the added work only is
//!   declined, the job goes on) are two transitions of the one `decline` of the
//!   spec table.
//! - The guards marked *slot* pass for now: they are the places where the
//!   issues of the negative flows add their rules (BL-AUT-04 `parts_arrived`,
//!   BL-AUT-05 and BL-AUT-06 `deliver` and `pass_qc`; `approve` has its
//!   `police_report` since BL-AUT-07). A slot keeps
//!   its name so that a test names the guard that refused. The three guards of
//!   `cancel` (BL-AUT-01) read what the service recorded in the input
//!   ([`StopRecord`]): the work done per line, the customer's acknowledgement,
//!   and whether the branch manager's approval is in place when the bill is
//!   below the work done (`AUT.ACL.21`).
//! - `cancel` is the action `auto.job_card_cancel` of the kernel's approvals
//!   ([`cancel_policy`]): when the advisor bills less than the work done, or
//!   the customer refuses to sign, the kernel defers the run as a request that
//!   a holder of `job_cards.approve` other than the advisor decides, and runs
//!   [`CANCEL_APPROVED`] (the same guards and effects) after the approval with
//!   the input the request kept. The job card stays as it is while it waits,
//!   and refuses new labour and parts ([`crate::job_stops::cancel_pending`]).
//! - Effects only touch the job card's own rows (the estimate rounds and
//!   lines) and the vehicle's next service; the invoice, the booking and the
//!   stock are the service's, after the commit ([`crate::job_cards`]).

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Json, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use crate::approval::{ApprovalStatus, EstimateApproval};
use crate::db::set_job_card;
use crate::documents::{PoliceVerdict, police_result};
use crate::job_card::{JobCard, JobCardStatus, QcCheck};
use crate::labour_line::{LabourLine, LineStatus};
use crate::part_line::PartLine;
use crate::parts_wait::PartsWait;
use crate::permissions;
use crate::stop::StopRecord;
use crate::texts::english;
use crate::vehicle::Vehicle;
use wl_kernel::SubjectMachine;
use wl_kernel::approvals::{self, PolicyDefault};
use wl_kernel::voids;

/// `checked_in → inspecting`: the technician starts diagnosing.
pub const START_INSPECTION: &str = "start_inspection";
/// `inspecting, in_progress → awaiting_approval`: the estimate (or added work)
/// goes to the customer.
pub const SEND_ESTIMATE: &str = "send_estimate";
/// `awaiting_approval → in_progress`: the advisor records the customer's
/// approval.
pub const APPROVE: &str = "approve";
/// `awaiting_approval → in_progress`: the customer approves on the link, or a
/// claim is approved.
pub const APPROVE_SYSTEM: &str = "approve_system";
/// `checked_in, inspecting, awaiting_approval → declined`: the customer
/// declines the estimate or leaves.
pub const DECLINE: &str = "decline";
/// `checked_in, inspecting, awaiting_approval → declined`: the estimate
/// expired, or a claim was cancelled with no work started.
pub const DECLINE_SYSTEM: &str = "decline_system";
/// `awaiting_approval → in_progress`: the added work is declined and the job
/// goes on without it.
pub const DECLINE_ADDED: &str = "decline_added";
/// `awaiting_approval → in_progress`: the customer declines the added work on
/// the link.
pub const DECLINE_ADDED_SYSTEM: &str = "decline_added_system";
/// `in_progress → awaiting_parts`: the job waits for parts.
pub const WAIT_PARTS: &str = "wait_parts";
/// `awaiting_parts → in_progress`: the parts were issued.
pub const PARTS_ARRIVED: &str = "parts_arrived";
/// `in_progress → quality_check`: the work is done.
pub const FINISH_WORK: &str = "finish_work";
/// `quality_check → ready`: the QC checklist is complete.
pub const PASS_QC: &str = "pass_qc";
/// `quality_check → in_progress`: the QC found something to put right.
pub const FAIL_QC: &str = "fail_qc";
/// `ready → delivered`: the customer collected the vehicle.
pub const DELIVER: &str = "deliver";
/// `inspecting .. quality_check → cancelled`: the job is stopped after work
/// started.
pub const CANCEL: &str = "cancel";
/// `inspecting .. quality_check → cancelled`: a claim was withdrawn or the
/// vehicle declared a total loss, or a private estimate was not signed.
pub const CANCEL_SYSTEM: &str = "cancel_system";
/// `inspecting .. quality_check → cancelled`: the kernel runs it after the
/// branch manager approved a bill below the work done or a refused signature
/// (`AUT.ACL.21`).
pub const CANCEL_APPROVED: &str = "cancel_approved";
/// The approval action of `cancel` (`AUT.ACL.21`).
pub const CANCEL_ACTION: &str = "auto.job_card_cancel";
/// The input key the approval policy of `cancel` reads: 1 when the branch
/// manager must approve, 0 when not.
pub const NEEDS_APPROVAL_KEY: &str = "needs_approval";
/// `checked_in → voided`: the job card was opened in error (the kernel's void).
pub const VOID: &str = voids::VOID;

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobInput {
    /// The user who acted.
    #[serde(default)]
    pub by: Option<i64>,
    /// The day of the run (Asia/Dubai).
    #[serde(default)]
    pub on: Option<NaiveDate>,
    /// The estimate has lines to send and every one is priced
    /// ([`SEND_ESTIMATE`]).
    #[serde(default)]
    pub priced: Option<bool>,
    /// The day the vehicle is promised for ([`SEND_ESTIMATE`]).
    #[serde(default)]
    pub promised_for: Option<NaiveDate>,
    /// The warranty on the repair and its parts in days ([`SEND_ESTIMATE`]).
    #[serde(default)]
    pub warranty_days: Option<i64>,
    /// The estimate round the customer answers ([`APPROVE`], [`DECLINE`],
    /// [`DECLINE_ADDED`] and their system twins).
    #[serde(default)]
    pub approval_id: Option<i64>,
    /// The name the customer gave.
    #[serde(default)]
    pub decided_by_name: Option<String>,
    /// How the customer answered: `link`, `signature` or `recorded`.
    #[serde(default)]
    pub method: Option<String>,
    /// The customer's signature, as the form sent it.
    #[serde(default)]
    pub signature: Option<String>,
    /// How many approved labour lines are not done ([`FINISH_WORK`]).
    #[serde(default)]
    pub labour_open: Option<i64>,
    /// How many lines the customer has not answered ([`DELIVER`]).
    #[serde(default)]
    pub added_open: Option<i64>,
    /// The reason as the advisor gave it ([`CANCEL`]); the kernel hands it to
    /// the approved run.
    #[serde(default)]
    pub reason: Option<String>,
    /// How many labour and part lines are recorded, removed ones not counted
    /// ([`DECLINE`]).
    #[serde(default)]
    pub lines_recorded: Option<i64>,
    /// Why the job card ends, as a code ([`DECLINE`]): `customer_left`,
    /// `estimate_declined` or `no_response`.
    #[serde(default)]
    pub code: Option<String>,
    /// How many approved part lines still wait for their part
    /// ([`WAIT_PARTS`]).
    #[serde(default)]
    pub parts_requested: Option<i64>,
    /// The legal supply date of the wait ([`WAIT_PARTS`]).
    #[serde(default)]
    pub legal_date: Option<NaiveDate>,
    /// The source that gave the shortest supply period ([`WAIT_PARTS`]).
    #[serde(default)]
    pub wait_source: Option<String>,
    /// A substitute vehicle is open ([`DELIVER`]).
    #[serde(default)]
    pub substitute_open: Option<bool>,
    /// What the advisor recorded for a stopped job ([`CANCEL`]).
    #[serde(default)]
    pub stop: Option<StopRecord>,
    /// 1 when the cancel needs the branch manager's approval, 0 when not: the
    /// input key of the approval policy ([`NEEDS_APPROVAL_KEY`]).
    #[serde(default)]
    pub needs_approval: Option<i64>,
    /// The service found the approval policy of `cancel` in force.
    #[serde(default)]
    pub approval_ready: bool,
    /// Who collected the vehicle ([`DELIVER`]).
    #[serde(default)]
    pub delivered_to: Option<String>,
    /// The odometer reading at delivery ([`DELIVER`]).
    #[serde(default)]
    pub odometer_out: Option<i64>,
    /// The date the next service falls due ([`DELIVER`]).
    #[serde(default)]
    pub next_service_date: Option<NaiveDate>,
    /// The odometer reading the next service falls due at ([`DELIVER`]).
    #[serde(default)]
    pub next_service_odometer: Option<i64>,
    /// What wl-documents answered for the police report ([`APPROVE`] and
    /// [`APPROVE_SYSTEM`], `AUT.ACL.20`).
    #[serde(default)]
    pub police: Option<PoliceVerdict>,
    /// How many test drives have not come back ([`PASS_QC`], `AUT.ACL.22`).
    #[serde(default)]
    pub test_drives_open: Option<i64>,
}

fn input_of(ctx: &TransitionCtx) -> JobInput {
    ctx.input_as::<JobInput>().unwrap_or_default()
}

/// `Ok` when `ok`, else the English text `key`.
fn check(ok: bool, key: &str) -> std::result::Result<(), String> {
    if ok { Ok(()) } else { Err(english(key, &[])) }
}

/// A guard slot: it names a rule an issue adds and passes for now.
fn slot(transition: Transition<JobCard>, name: &'static str) -> Transition<JobCard> {
    transition.guard(name, |_c: &JobCard, _ctx| {
        Box::pin(async { Ok::<(), String>(()) })
    })
}

impl StateMachine for JobCard {
    type State = JobCardStatus;
    const COLUMN: renox::db::Col<JobCard, JobCardStatus> = JobCard::STATUS;

    fn machine() -> Machine<Self> {
        use JobCardStatus::{
            AwaitingApproval, AwaitingParts, Cancelled, CheckedIn, Declined, Delivered, InProgress,
            Inspecting, QualityCheck, Ready, Voided,
        };
        let machine = Machine::new()
            .initial([CheckedIn])
            .state(CheckedIn)
            .tone("neutral")
            .state(Inspecting)
            .tone("info")
            .state(AwaitingApproval)
            .tone("warning")
            .state(InProgress)
            .tone("info")
            .state(AwaitingParts)
            .tone("warning")
            .state(QualityCheck)
            .tone("info")
            .state(Ready)
            .tone("success")
            .state(Delivered)
            .tone("success")
            .terminal()
            .state(Declined)
            .tone("neutral")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .state(Voided)
            .tone("neutral")
            .terminal()
            .transition(start_inspection())
            .transition(send_estimate())
            .transition(approve(APPROVE, false))
            .transition(approve(APPROVE_SYSTEM, true))
            .transition(decline(DECLINE, false))
            .transition(decline(DECLINE_SYSTEM, true))
            .transition(decline_added(DECLINE_ADDED, false))
            .transition(decline_added(DECLINE_ADDED_SYSTEM, true))
            .transition(wait_parts())
            .transition(parts_arrived())
            .transition(finish_work())
            .transition(pass_qc())
            .transition(fail_qc())
            .transition(deliver())
            .transition(cancel(CANCEL, CancelBy::Advisor))
            .transition(cancel(CANCEL_APPROVED, CancelBy::Approval))
            .transition(cancel(CANCEL_SYSTEM, CancelBy::System));
        // `void` and `void_approved`, the kernel's, with the job card's rule on
        // top (BL-AUT-02).
        voids::transitions::<JobCard, _>(crate::job_card::JOB_CARD_SUBJECT_TYPE, [CheckedIn])
            .into_iter()
            .map(void_rules)
            .fold(machine, Machine::transition)
    }
}

/// Ends the estimate round the run names, with `status`: the customer's
/// answer, who gave it and how.
async fn answer(
    tx: &mut renox::db::Transaction,
    input: &JobInput,
    status: ApprovalStatus,
    reason: Option<String>,
) -> Result {
    let Some(id) = input.approval_id else {
        return Ok(());
    };
    let stamp = now();
    EstimateApproval::query()
        .where_(EstimateApproval::ID.eq(id))
        .update(
            &mut *tx,
            &[
                ("status", &status),
                ("decided_at", &Some(stamp)),
                ("decided_by_name", &input.decided_by_name),
                ("decided_by_user", &input.by),
                ("method", &input.method),
                ("signature", &input.signature),
                ("note", &reason),
                ("updated_at", &stamp),
            ],
        )
        .await?;
    Ok(())
}

/// Moves the lines the round sent to `status`.
async fn move_lines(
    tx: &mut renox::db::Transaction,
    approval_id: i64,
    from: LineStatus,
    to: LineStatus,
) -> Result {
    let stamp = now();
    LabourLine::query()
        .where_(LabourLine::APPROVAL_ID.eq(Some(approval_id)))
        .where_(LabourLine::APPROVAL.eq(from))
        .update(&mut *tx, &[("approval", &to), ("updated_at", &stamp)])
        .await?;
    PartLine::query()
        .where_(PartLine::APPROVAL_ID.eq(Some(approval_id)))
        .where_(PartLine::APPROVAL.eq(from))
        .update(&mut *tx, &[("approval", &to), ("updated_at", &stamp)])
        .await?;
    Ok(())
}

/// `start_inspection`: the technician starts diagnosing.
fn start_inspection() -> Transition<JobCard> {
    Transition::new(START_INSPECTION)
        .label("Start inspection")
        .tone("primary")
        .permission(permissions::JOB_CARDS_WORK)
        .from([JobCardStatus::CheckedIn])
        .to(JobCardStatus::Inspecting)
        .guard("technician_assigned", |c: &JobCard, _ctx| {
            let ok = c.technician_user_id.is_some();
            Box::pin(async move { check(ok, "auto.jobs.problem.no_technician") })
        })
        .guard("intake_recorded", |c: &JobCard, _ctx| {
            let ok = c.intake_done();
            Box::pin(async move { check(ok, "auto.jobs.problem.no_intake") })
        })
}

/// `send_estimate`: the draft lines go to the customer as a round to approve.
fn send_estimate() -> Transition<JobCard> {
    Transition::new(SEND_ESTIMATE)
        .label("Send the estimate")
        .tone("primary")
        .permission(permissions::JOB_CARDS_ADVISE)
        .from([JobCardStatus::Inspecting, JobCardStatus::InProgress])
        .to(JobCardStatus::AwaitingApproval)
        .guard("intake_recorded", |c: &JobCard, _ctx| {
            let ok = c.intake_done();
            Box::pin(async move { check(ok, "auto.jobs.problem.no_intake") })
        })
        .guard("estimate_priced", |_c: &JobCard, ctx| {
            let ok = input_of(ctx).priced == Some(true);
            Box::pin(async move { check(ok, "auto.jobs.problem.not_priced") })
        })
        .effect(|c: &JobCard, ctx, tx| {
            let card = c.clone();
            let input = input_of(ctx);
            Box::pin(async move {
                let revision = card.estimate_revision + 1;
                crate::approval_rounds::open(tx, &card, revision, &input).await?;
                set_job_card(
                    tx,
                    card.id,
                    &[
                        ("estimate_revision", &revision),
                        ("awaiting_since", &Some(now())),
                    ],
                )
                .await
            })
        })
}

/// `approve`: the customer's approval is recorded (by the advisor, or by the
/// customer on the link). The lines of the round are approved, the first
/// approval is stamped, and a special order the customer approved is agreed.
fn approve(name: &'static str, system: bool) -> Transition<JobCard> {
    let transition = Transition::new(name)
        .label("Approve")
        .tone("primary")
        .from([JobCardStatus::AwaitingApproval])
        .to(JobCardStatus::InProgress);
    let transition = if system {
        transition.system()
    } else {
        transition.permission(permissions::JOB_CARDS_ADVISE)
    };
    let transition = transition.guard("estimate_pending", |_c: &JobCard, ctx| {
        let ok = input_of(ctx).approval_id.is_some();
        Box::pin(async move { check(ok, "auto.jobs.problem.nothing_to_approve") })
    });
    transition.guard("police_report", |c: &JobCard, ctx| {
        // Accident damage is repaired only with the police report attached
        // and accepted (`AUT.ACL.20`, a legal block with no override).
        let result = police_result(c.accident_damage, &input_of(ctx).police);
        Box::pin(async move { result })
    })
    .effect(|c: &JobCard, ctx, tx| {
        let card = c.clone();
        let input = input_of(ctx);
        Box::pin(async move {
            if let Some(id) = input.approval_id {
                move_lines(tx, id, LineStatus::Sent, LineStatus::Approved).await?;
                PartLine::query()
                    .where_(PartLine::APPROVAL_ID.eq(Some(id)))
                    .where_(PartLine::SPECIAL_ORDER.eq(true))
                    .update(&mut *tx, &[("special_order_agreed", &true)])
                    .await?;
            }
            answer(tx, &input, ApprovalStatus::Approved, None).await?;
            let first = card.first_approved_at.unwrap_or_else(now);
            let none: Option<DateTime> = None;
            set_job_card(
                tx,
                card.id,
                &[
                    ("first_approved_at", &Some(first)),
                    ("awaiting_since", &none),
                ],
            )
            .await
        })
    })
}

/// `decline`: the customer declines the estimate or leaves; nothing was
/// approved yet, so the job card ends (`declined`).
fn decline(name: &'static str, system: bool) -> Transition<JobCard> {
    let transition = Transition::new(name)
        .label("Decline")
        .from([
            JobCardStatus::CheckedIn,
            JobCardStatus::Inspecting,
            JobCardStatus::AwaitingApproval,
        ])
        .to(JobCardStatus::Declined)
        .reason_required();
    let transition = if system {
        transition.system()
    } else {
        transition.permission(permissions::JOB_CARDS_ADVISE)
    };
    let transition = transition.guard("nothing_approved", |c: &JobCard, _ctx| {
        let ok = !c.work_approved();
        Box::pin(async move { check(ok, "auto.jobs.problem.already_approved") })
    });
    transition
        .guard("nothing_recorded", |c: &JobCard, ctx| {
            // From `checked_in` or `inspecting` nothing may be recorded; the
            // lines of `awaiting_approval` were sent with the estimate.
            let ok = c.status == JobCardStatus::AwaitingApproval
                || input_of(ctx).lines_recorded == Some(0);
            Box::pin(async move { check(ok, "auto.decline.problem.recorded") })
        })
        .effect(|c: &JobCard, ctx, tx| {
            let card = c.clone();
            let input = input_of(ctx);
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move {
                if let Some(id) = input.approval_id {
                    move_lines(tx, id, LineStatus::Sent, LineStatus::Declined).await?;
                }
                answer(tx, &input, ApprovalStatus::Declined, reason.clone()).await?;
                // A round nobody named (an estimate that expired) ends too.
                crate::approval_rounds::supersede(tx, card.id).await?;
                let none: Option<DateTime> = None;
                let code = input
                    .code
                    .clone()
                    .unwrap_or_else(|| crate::stop::CUSTOMER_LEFT.to_owned());
                // The inspection fee agreed at check-in is what is billed.
                let fee = card.inspection_fee.max(0);
                set_job_card(
                    tx,
                    card.id,
                    &[
                        ("closed_reason", &reason),
                        ("closed_code", &Some(code)),
                        ("closed_at", &Some(now())),
                        ("awaiting_since", &none),
                        ("stopped_by", &input.by),
                        ("billed_net", &fee),
                    ],
                )
                .await
            })
        })
}

/// `decline_added`: the customer declines the added work; the job goes on
/// without it.
fn decline_added(name: &'static str, system: bool) -> Transition<JobCard> {
    let transition = Transition::new(name)
        .label("Decline the added work")
        .from([JobCardStatus::AwaitingApproval])
        .to(JobCardStatus::InProgress);
    let transition = if system {
        transition.system()
    } else {
        transition.permission(permissions::JOB_CARDS_ADVISE)
    };
    transition
        .guard("work_approved", |c: &JobCard, _ctx| {
            let ok = c.work_approved();
            Box::pin(async move { check(ok, "auto.jobs.problem.nothing_approved_yet") })
        })
        .effect(|c: &JobCard, ctx, tx| {
            let card = c.clone();
            let input = input_of(ctx);
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move {
                if let Some(id) = input.approval_id {
                    move_lines(tx, id, LineStatus::Sent, LineStatus::Declined).await?;
                }
                answer(tx, &input, ApprovalStatus::Declined, reason).await?;
                let none: Option<DateTime> = None;
                set_job_card(tx, card.id, &[("awaiting_since", &none)]).await
            })
        })
}

/// `wait_parts`: the job waits for parts; the wait starts now and the legal
/// supply date is recorded: the start plus the shortest supply period among the
/// sources of the parts requested (`AUT.RUL.16`). Neither is edited by hand.
fn wait_parts() -> Transition<JobCard> {
    Transition::new(WAIT_PARTS)
        .label("Wait for parts")
        .permission(permissions::JOB_CARDS_WORK)
        .from([JobCardStatus::InProgress])
        .to(JobCardStatus::AwaitingParts)
        .guard("parts_requested", |_c: &JobCard, ctx| {
            let input = input_of(ctx);
            let ok = input.parts_requested.is_some_and(|n| n > 0) && input.legal_date.is_some();
            Box::pin(async move { check(ok, "auto.parts.problem.no_request") })
        })
        .effect(|c: &JobCard, ctx, tx| {
            let id = c.id;
            let input = input_of(ctx);
            Box::pin(async move {
                let started = now();
                set_job_card(
                    tx,
                    id,
                    &[
                        ("parts_wait_started_at", &Some(started)),
                        ("legal_supply_date", &input.legal_date),
                    ],
                )
                .await?;
                if let Some(legal_date) = input.legal_date {
                    let mut wait = PartsWait {
                        job_card_id: id,
                        started_at: started,
                        legal_date,
                        source: input.wait_source.clone().unwrap_or_default(),
                        ..PartsWait::default()
                    };
                    wait.insert(&mut *tx).await?;
                }
                Ok(())
            })
        })
}

/// `parts_arrived`: the parts were issued; the job goes on and the wait ends.
fn parts_arrived() -> Transition<JobCard> {
    let transition = Transition::new(PARTS_ARRIVED)
        .label("Parts arrived")
        .system()
        .from([JobCardStatus::AwaitingParts])
        .to(JobCardStatus::InProgress);
    slot(transition, "stock_issued").effect(|c: &JobCard, _ctx, tx| {
        let id = c.id;
        Box::pin(async move {
            let stamp = now();
            PartsWait::query()
                .where_(PartsWait::JOB_CARD_ID.eq(id))
                .where_(PartsWait::ENDED_AT.is_null())
                .update(
                    &mut *tx,
                    &[("ended_at", &Some(stamp)), ("updated_at", &stamp)],
                )
                .await?;
            Ok(())
        })
    })
}

/// `finish_work`: every approved labour line is done; the QC checklist starts.
fn finish_work() -> Transition<JobCard> {
    Transition::new(FINISH_WORK)
        .label("Finish the work")
        .tone("primary")
        .permission(permissions::JOB_CARDS_WORK)
        .from([JobCardStatus::InProgress])
        .to(JobCardStatus::QualityCheck)
        .guard("labour_done", |_c: &JobCard, ctx| {
            let ok = input_of(ctx).labour_open == Some(0);
            Box::pin(async move { check(ok, "auto.jobs.problem.labour_open") })
        })
        .effect(|c: &JobCard, _ctx, tx| {
            let id = c.id;
            let fresh = c.qc_checks.0.is_empty();
            Box::pin(async move {
                if fresh {
                    set_job_card(tx, id, &[("qc_checks", &Json(QcCheck::standard()))]).await?;
                }
                Ok(())
            })
        })
}

/// `pass_qc`: the QC checklist is complete; the vehicle is ready.
fn pass_qc() -> Transition<JobCard> {
    let transition = Transition::new(PASS_QC)
        .label("Pass the QC")
        .tone("primary")
        .permission(permissions::JOB_CARDS_QC)
        .from([JobCardStatus::QualityCheck])
        .to(JobCardStatus::Ready)
        .guard("qc_checklist_complete", |c: &JobCard, _ctx| {
            let ok = c.qc_complete();
            Box::pin(async move { check(ok, "auto.jobs.problem.qc_incomplete") })
        })
        .guard("test_drives_closed", |_c: &JobCard, ctx| {
            // A test drive without its mileage in is still out (BL-AUT-06).
            let ok = input_of(ctx).test_drives_open.unwrap_or(0) == 0;
            Box::pin(async move { check(ok, "auto.drives.problem.open") })
        });
    slot(transition, "qc_findings_closed").effect(|c: &JobCard, ctx, tx| {
        let id = c.id;
        let by = input_of(ctx).by;
        Box::pin(async move {
            set_job_card(tx, id, &[("ready_at", &Some(now())), ("qc_by", &by)]).await
        })
    })
}

/// `fail_qc`: the QC found something to put right; the findings are the
/// reason, and the checklist starts again.
fn fail_qc() -> Transition<JobCard> {
    Transition::new(FAIL_QC)
        .label("Fail the QC")
        .permission(permissions::JOB_CARDS_QC)
        .from([JobCardStatus::QualityCheck])
        .to(JobCardStatus::InProgress)
        .reason_required()
        .effect(|c: &JobCard, ctx, tx| {
            let id = c.id;
            let findings = ctx.reason().map(str::to_owned);
            Box::pin(async move {
                set_job_card(
                    tx,
                    id,
                    &[
                        ("qc_findings", &findings),
                        ("qc_checks", &Json(QcCheck::standard())),
                    ],
                )
                .await
            })
        })
}

/// `deliver`: the customer collects the vehicle. The next service is
/// recorded on the vehicle; the invoice and the booking follow the commit.
fn deliver() -> Transition<JobCard> {
    let transition = Transition::new(DELIVER)
        .label("Deliver")
        .tone("primary")
        .permission(permissions::JOB_CARDS_ADVISE)
        .from([JobCardStatus::Ready])
        .to(JobCardStatus::Delivered)
        .guard("qc_checklist_complete", |c: &JobCard, _ctx| {
            let ok = c.qc_complete();
            Box::pin(async move { check(ok, "auto.jobs.problem.qc_incomplete") })
        })
        .guard("added_work_approved", |_c: &JobCard, ctx| {
            let ok = input_of(ctx).added_open == Some(0);
            Box::pin(async move { check(ok, "auto.jobs.problem.added_work_open") })
        });
    let transition = transition.guard("substitute_closed", |_c: &JobCard, ctx| {
        let ok = input_of(ctx).substitute_open != Some(true);
        Box::pin(async move { check(ok, "auto.substitute.problem.open") })
    });
    slot(transition, "payment_cleared").effect(|c: &JobCard, ctx, tx| {
        let card = c.clone();
        let input = input_of(ctx);
        Box::pin(async move {
            set_job_card(
                tx,
                card.id,
                &[
                    ("delivered_at", &Some(now())),
                    ("delivered_to", &input.delivered_to),
                    ("odometer_out", &input.odometer_out),
                    ("next_service_date", &input.next_service_date),
                    ("next_service_odometer", &input.next_service_odometer),
                ],
            )
            .await?;
            // The next service and the odometer go to the vehicle (`AUT.JOB.04`).
            let stamp = now();
            if input.next_service_date.is_some() || input.next_service_odometer.is_some() {
                Vehicle::query()
                    .where_(Vehicle::ID.eq(card.vehicle_id))
                    .update(
                        &mut *tx,
                        &[
                            ("next_service_date", &input.next_service_date),
                            ("next_service_odometer", &input.next_service_odometer),
                            ("updated_at", &stamp),
                        ],
                    )
                    .await?;
            }
            if let Some(km) = input.odometer_out {
                Vehicle::query()
                    .where_(Vehicle::ID.eq(card.vehicle_id))
                    .where_(Vehicle::ODOMETER_KM.lt(km))
                    .update(
                        &mut *tx,
                        &[
                            ("odometer_km", &km),
                            ("odometer_at", &Some(stamp)),
                            ("updated_at", &stamp),
                        ],
                    )
                    .await?;
            }
            Ok(())
        })
    })
}

/// Who runs a `cancel`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CancelBy {
    /// The service advisor (`job_cards.advise`); the kernel defers the run
    /// when the branch manager must approve.
    Advisor,
    /// The kernel, after the branch manager approved.
    Approval,
    /// The system: a claim that was withdrawn or a total loss.
    System,
}

/// `cancel`: the job is stopped after work started. The reason, the work done
/// per line, the customer's acknowledgement and the bill are recorded
/// (`AUT.ACL.21`); what follows (the invoice, the parts back to stock, the
/// booking) is the service's, after the commit ([`crate::job_stops`]).
fn cancel(name: &'static str, by: CancelBy) -> Transition<JobCard> {
    let transition = Transition::new(name)
        .label(if by == CancelBy::Approval {
            "Cancel (approved)"
        } else {
            "Cancel"
        })
        .from([
            JobCardStatus::Inspecting,
            JobCardStatus::AwaitingApproval,
            JobCardStatus::InProgress,
            JobCardStatus::AwaitingParts,
            JobCardStatus::QualityCheck,
        ])
        .to(JobCardStatus::Cancelled)
        .reason_required();
    let transition = match by {
        CancelBy::Advisor => transition.permission(permissions::JOB_CARDS_ADVISE),
        CancelBy::Approval | CancelBy::System => transition.system(),
    };
    transition
        .guard("work_done_recorded", |_c: &JobCard, ctx| {
            let ok = input_of(ctx).stop.is_some_and(|stop| stop.work_recorded);
            Box::pin(async move { check(ok, "auto.stop.problem.work_not_recorded") })
        })
        .guard("acknowledged", |_c: &JobCard, ctx| {
            let ok = input_of(ctx).stop.is_some_and(|stop| stop.acknowledged());
            Box::pin(async move { check(ok, "auto.stop.problem.not_acknowledged") })
        })
        .guard("approved_if_below_work_done", move |_c: &JobCard, ctx| {
            let input = input_of(ctx);
            let ok = match by {
                // The run the kernel makes after the approval.
                CancelBy::Approval => input.by.is_some(),
                // A claim's run bills the customer the work done.
                CancelBy::System => input.stop.is_some_and(|stop| !stop.lowered()),
                // The advisor's run: a bill below the work done or a refused
                // signature runs only with the approval policy in force, so
                // that the kernel defers it for the branch manager.
                CancelBy::Advisor => input.needs_approval == Some(0) || input.approval_ready,
            };
            Box::pin(async move { check(ok, "auto.stop.problem.approval") })
        })
        .effect(|c: &JobCard, ctx, tx| {
            let card = c.clone();
            let input = input_of(ctx);
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move { crate::job_stops::record_in(tx, &card, &input, reason).await })
        })
}

/// The default approval policy of `cancel` (`AUT.ACL.21`): a request whose
/// input says `needs_approval` (a bill below the work done, or a refused
/// signature) needs one step by a holder of `job_cards.approve` (the branch
/// manager), never the advisor who asked.
pub fn cancel_policy() -> PolicyDefault {
    PolicyDefault::new(CANCEL_ACTION)
        .amount_above(0)
        .input_key(NEEDS_APPROVAL_KEY)
        .step_permission(permissions::JOB_CARDS_APPROVE)
}

/// What `void` adds to the kernel's guards (BL-AUT-02): the reason and the
/// closing time are kept on the job card. That no labour or part line exists
/// and no insurance claim is linked (`PLT.ACL.09`) is the void's reverse
/// handler's ([`crate::job_void`]): guards cannot read the database.
fn void_rules(transition: Transition<JobCard>) -> Transition<JobCard> {
    transition.effect(|c: &JobCard, ctx, tx| {
        let id = c.id;
        let reason = ctx.reason().map(str::to_owned);
        Box::pin(async move {
            set_job_card(
                tx,
                id,
                &[("closed_reason", &reason), ("closed_at", &Some(now()))],
            )
            .await
        })
    })
}

/// The job card in the kernel's registry, so the approvals can run
/// [`CANCEL_APPROVED`] after the approval and the page the approvals link to.
pub fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<JobCard>(crate::job_card::JOB_CARD_SUBJECT_TYPE)
        .action(CANCEL, CANCEL_ACTION)
        .build()
}

/// Registers the machine for `rnx state:diagram` and with the kernel, the
/// approval policy of `cancel` and the page the approvals link to.
pub(crate) fn register() {
    subject_machine().register();
    renox::state_machine::register::<JobCard>();
    cancel_policy().register();
    crate::job_void::register();
    approvals::register_subject_link(crate::job_card::JOB_CARD_SUBJECT_TYPE, |id| {
        format!("/auto/job-cards/{id}")
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_every_state_and_transition_of_the_spec() {
        let machine = JobCard::machine();
        let states: Vec<(&str, Option<&str>, bool)> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("checked_in", Some("neutral"), false),
                ("inspecting", Some("info"), false),
                ("awaiting_approval", Some("warning"), false),
                ("in_progress", Some("info"), false),
                ("awaiting_parts", Some("warning"), false),
                ("quality_check", Some("info"), false),
                ("ready", Some("success"), false),
                ("delivered", Some("success"), true),
                ("declined", Some("neutral"), true),
                ("cancelled", Some("neutral"), true),
                ("voided", Some("neutral"), true),
            ]
        );
        let open = vec![
            "inspecting",
            "awaiting_approval",
            "in_progress",
            "awaiting_parts",
            "quality_check",
        ];
        let expected: Vec<(&str, Vec<&str>, &str)> = vec![
            (START_INSPECTION, vec!["checked_in"], "inspecting"),
            (
                SEND_ESTIMATE,
                vec!["inspecting", "in_progress"],
                "awaiting_approval",
            ),
            (APPROVE, vec!["awaiting_approval"], "in_progress"),
            (APPROVE_SYSTEM, vec!["awaiting_approval"], "in_progress"),
            (
                DECLINE,
                vec!["checked_in", "inspecting", "awaiting_approval"],
                "declined",
            ),
            (
                DECLINE_SYSTEM,
                vec!["checked_in", "inspecting", "awaiting_approval"],
                "declined",
            ),
            (DECLINE_ADDED, vec!["awaiting_approval"], "in_progress"),
            (
                DECLINE_ADDED_SYSTEM,
                vec!["awaiting_approval"],
                "in_progress",
            ),
            (WAIT_PARTS, vec!["in_progress"], "awaiting_parts"),
            (PARTS_ARRIVED, vec!["awaiting_parts"], "in_progress"),
            (FINISH_WORK, vec!["in_progress"], "quality_check"),
            (PASS_QC, vec!["quality_check"], "ready"),
            (FAIL_QC, vec!["quality_check"], "in_progress"),
            (DELIVER, vec!["ready"], "delivered"),
            (CANCEL, open.clone(), "cancelled"),
            (CANCEL_APPROVED, open.clone(), "cancelled"),
            (CANCEL_SYSTEM, open, "cancelled"),
            (VOID, vec!["checked_in"], "voided"),
            ("void_approved", vec!["checked_in"], "voided"),
        ];
        assert_eq!(machine.transitions().len(), expected.len());
        let system = [
            APPROVE_SYSTEM,
            DECLINE_SYSTEM,
            DECLINE_ADDED_SYSTEM,
            PARTS_ARRIVED,
            CANCEL_APPROVED,
            CANCEL_SYSTEM,
            "void_approved",
        ];
        for (name, from, to) in expected {
            let transition = machine.find(name).unwrap();
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.is_system(), system.contains(&name), "{name}");
        }
    }

    #[test]
    fn the_guards_and_the_guard_slots_are_declared() {
        let machine = JobCard::machine();
        let guards = |name: &str| {
            machine
                .find(name)
                .unwrap()
                .guard_names()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            guards(START_INSPECTION),
            ["technician_assigned", "intake_recorded"]
        );
        assert_eq!(
            guards(SEND_ESTIMATE),
            ["intake_recorded", "estimate_priced"]
        );
        assert_eq!(guards(APPROVE), ["estimate_pending", "police_report"]);
        assert_eq!(
            guards(APPROVE_SYSTEM),
            ["estimate_pending", "police_report"]
        );
        assert_eq!(guards(DECLINE), ["nothing_approved", "nothing_recorded"]);
        assert_eq!(guards(DECLINE_ADDED), ["work_approved"]);
        assert_eq!(guards(WAIT_PARTS), ["parts_requested"]);
        assert_eq!(guards(PARTS_ARRIVED), ["stock_issued"]);
        assert_eq!(guards(FINISH_WORK), ["labour_done"]);
        assert_eq!(
            guards(PASS_QC),
            [
                "qc_checklist_complete",
                "test_drives_closed",
                "qc_findings_closed"
            ]
        );
        assert!(guards(FAIL_QC).is_empty());
        assert_eq!(
            guards(DELIVER),
            [
                "qc_checklist_complete",
                "added_work_approved",
                "substitute_closed",
                "payment_cleared"
            ]
        );
        for name in [CANCEL, CANCEL_APPROVED, CANCEL_SYSTEM] {
            assert_eq!(
                guards(name),
                [
                    "work_done_recorded",
                    "acknowledged",
                    "approved_if_below_work_done"
                ],
                "{name}"
            );
        }
        for name in [VOID, "void_approved"] {
            assert_eq!(
                guards(name),
                ["permission_held", "not_given_to_third_party"],
                "{name}"
            );
        }
        for name in [
            DECLINE,
            DECLINE_SYSTEM,
            FAIL_QC,
            CANCEL,
            CANCEL_APPROVED,
            CANCEL_SYSTEM,
            VOID,
        ] {
            assert!(machine.find(name).unwrap().needs_reason(), "{name}");
        }
    }
}
