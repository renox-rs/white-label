//! Approving a timesheet (wl-attendance §4 `approve`, `approve_on_behalf`,
//! `reject`, `deem_approve`, §5.8; `HRM.ACL.08`, BL-ATT-08, BL-ATT-11, BL-ATT-13).
//!
//! ```text
//! submitted --approve--------------> approved
//! submitted --approve_on_behalf----> approved
//! submitted --reject---------------> open
//! submitted --deem_approve---------> approved   (system, BL-ATT-11)
//! submitted --approve_by_system----> approved   (system, BL-ATT-13)
//! ```
//!
//! **Who approves.** A timesheet has an approver kind. An `internal` one is
//! approved by a user holding `timesheets.approve` in the scope of its
//! context. An `external` one is approved by the client's approver, a user who
//! holds the timesheet's `approver_role` and is one of the client's approvers
//! of the timesheet's context (`ApproverResolver::client_approvers`; the portal
//! and the signed emailed link both end here), or by staff with `timesheets.approve_on_behalf` who
//! attach the signed sheet ([`Evidence`]). The kind the actor approves as is
//! worked out by [`approver_kind_for`] and checked against the timesheet by a
//! guard, so an internal approver who calls `approve` on an externally
//! approved timesheet is refused and must use `approve_on_behalf`.
//!
//! **Segregation of duties.** An adjustment's approver kind is its original's
//! (it is copied when the adjustment is opened), and its creator can never
//! approve it, by any approving transition and whatever their permissions
//! (`HRM.ACL.08`).
//!
//! **Effects.** The machine stores the basis, the approver and the time,
//! the evidence of an approval on behalf, and locks the `accepted` attendance
//! the entries were proposed from (an adjustment has no attendance of its own).
//! After the commit [`approve`] posts the labour cost where the context posts
//! cost from hours (`FIN.CST.01`, the app's
//! [`CostRateResolver`](wl_kernel::attendance::CostRateResolver)): the entries'
//! minutes times the hourly rate of the category, per cost object, signed for
//! an adjustment, under an idempotency key per entry and category so that a
//! repeated approval cannot count twice; then it emits `TimesheetApproved`.
//! `reject` returns the timesheet to `open` with the reason (in the
//! history) and tells whoever submitted it.
//!
//! Decisions:
//!
//! - The cost is posted and the event emitted after the commit, because the
//!   kernel's poster takes the app state (and its pool) and must not run while
//!   a transaction is open. A failure after the commit leaves the timesheet
//!   approved; the idempotent keys make a retry of the posting safe
//!   ([`post_labour_cost`]).
//! - `deem_approve` (`HRM.RUL.15` at N, BL-ATT-11) is the system transition
//!   [`DEEM_APPROVE`], run by [`crate::deem`]: the guards check that the
//!   approver is external and that N days have passed since the latest
//!   `submit` (the days come in the input, read from the contract's
//!   `ApproverResolver` when the rule runs); it is approved with the basis
//!   `deemed` and no approver, and everything after the commit is that of
//!   `approve` (labour cost, `TimesheetApproved`, the reconciliation of
//!   provisional hours). The system approval of a `provisional_reconciliation` adjustment is the
//!   transition [`APPROVE_BY_SYSTEM`] (BL-ATT-13): it copies the approver and
//!   the basis of the original's approval and is refused for any other
//!   timesheet.
//! - Whoever may approve may reject. The permission of `reject` follows the
//!   approver kind as for `approve`.

use std::collections::BTreeMap;

use renox::auth::permissions::users_with_role;
use renox::chrono::{Duration, NaiveDate};
use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::attendance::{CategoryMinutes, TimesheetApproved, cost_rates};
use wl_kernel::documents::dubai_date;
use wl_kernel::invoicing::{PostCost, post_cost};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::timesheets::approver_resolver;

use crate::access;
use crate::entry::{Category, TimesheetEntry};
use crate::escalation;
use crate::notices;
use crate::payroll;
use crate::permissions;
use crate::record::{AttendanceRecord, AttendanceStatus};
use crate::review;
use crate::texts::english;
use crate::timesheet::{AdjustmentOrigin, ApprovalBasis, ApproverKind, Timesheet, TimesheetStatus};

/// The approver approves a submitted timesheet.
pub const APPROVE: &str = "approve";
/// Staff approve for the external approver, with the signed sheet.
pub const APPROVE_ON_BEHALF: &str = "approve_on_behalf";
/// The approver sends a submitted timesheet back to `open` with a reason.
pub const REJECT: &str = "reject";
/// The system approves an external timesheet nobody answered, as the
/// contract's deemed-approval clause says (`HRM.RUL.15` at N, BL-ATT-11).
pub const DEEM_APPROVE: &str = "deem_approve";
/// The system approves a `provisional_reconciliation` adjustment in the same
/// step as the approval of its original (BL-ATT-13).
pub const APPROVE_BY_SYSTEM: &str = "approve_by_system";

/// The proof of an approval given outside the system: the signed sheet.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_attendance_approval_evidence", index(timesheet_id))]
pub struct ApprovalEvidence {
    /// The primary key.
    pub id: i64,
    /// The timesheet it supports.
    #[model(references = "wl_attendance_timesheets")]
    pub timesheet_id: i64,
    /// The document of wl-documents that holds the signed sheet.
    pub document_id: i64,
    /// Who signed for the client.
    pub approver_name: String,
    /// The day they signed.
    pub signed_on: NaiveDate,
    /// The staff member who attached it.
    pub attached_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ApprovalEvidence {
    /// The evidence attached to `timesheet_id`, oldest first.
    pub async fn of_timesheet(db: &Db, timesheet_id: i64) -> Result<Vec<ApprovalEvidence>> {
        ApprovalEvidence::query()
            .where_(ApprovalEvidence::TIMESHEET_ID.eq(timesheet_id))
            .order_by(ApprovalEvidence::ID)
            .get(db)
            .await
    }
}

/// The signed sheet that goes with `approve_on_behalf`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    /// The document that holds the signed sheet.
    pub document_id: i64,
    /// Who signed for the client.
    pub approver_name: String,
    /// The day they signed.
    pub signed_on: NaiveDate,
}

/// The input of [`DEEM_APPROVE`]: the days of the contract's clause as the
/// resolver gave them when the rule ran, and the day it ran.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DeemInput {
    /// N, the days after `submit` at which silence is approval.
    pub days: i64,
    /// The day the rule runs, in Asia/Dubai.
    pub on: NaiveDate,
}

/// The input of the approving transitions.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ApproveInput {
    /// The kind the actor approves as, `internal` or `external`.
    pub approver_kind: Option<String>,
    /// The signed sheet, for `approve_on_behalf`.
    pub evidence: Option<Evidence>,
}

/// The transitions of the approval, for the timesheet's machine.
pub(crate) fn transitions() -> Vec<Transition<Timesheet>> {
    vec![
        Transition::new(APPROVE)
            .label("Approve")
            .tone("primary")
            .from([TimesheetStatus::Submitted])
            .to(TimesheetStatus::Approved)
            .guard("approver_kind_matches", |sheet: &Timesheet, ctx| {
                let checked = check_kind(sheet, ctx);
                Box::pin(async move { checked })
            })
            .guard("creator_is_not_approver", |sheet: &Timesheet, ctx| {
                let checked = check_creator(sheet, ctx);
                Box::pin(async move { checked })
            })
            .guard("dispute_not_upheld", |sheet: &Timesheet, _ctx| {
                // An adjustment of a client dispute that ended `upheld` has
                // no payroll or billing effect and is not approved either
                // (`crate::dispute`, BL-ATT-09).
                let upheld = sheet.dispute_upheld_at.is_some();
                Box::pin(async move {
                    if upheld {
                        Err(english(
                            "wl_attendance.adjustment.dispute_upheld_approve",
                            &[],
                        ))
                    } else {
                        Ok(())
                    }
                })
            })
            .effect(|sheet: &Timesheet, ctx, tx| {
                let sheet = sheet.clone();
                let by = ctx.actor().map(|user| user.id);
                Box::pin(
                    async move { approve_in(tx, &sheet, by, ApprovalBasis::Approved, None).await },
                )
            }),
        Transition::new(APPROVE_ON_BEHALF)
            .label("Approve on behalf")
            .from([TimesheetStatus::Submitted])
            .to(TimesheetStatus::Approved)
            .guard("external_only", |sheet: &Timesheet, _ctx| {
                let external = sheet.approver_kind == ApproverKind::External;
                Box::pin(async move {
                    if external {
                        Ok(())
                    } else {
                        Err(english("wl_attendance.approval.not_external", &[]))
                    }
                })
            })
            .guard("evidence_attached", |_sheet: &Timesheet, ctx| {
                let evidence = ctx.input_as::<ApproveInput>().ok().and_then(|i| i.evidence);
                Box::pin(async move {
                    match evidence {
                        Some(e) if !e.approver_name.trim().is_empty() && e.document_id > 0 => {
                            Ok(())
                        }
                        _ => Err(english("wl_attendance.approval.evidence_required", &[])),
                    }
                })
            })
            .guard("creator_is_not_approver", |sheet: &Timesheet, ctx| {
                let checked = check_creator(sheet, ctx);
                Box::pin(async move { checked })
            })
            .guard("dispute_not_upheld", |sheet: &Timesheet, _ctx| {
                // An adjustment of a client dispute that ended `upheld` has
                // no payroll or billing effect and is not approved either
                // (`crate::dispute`, BL-ATT-09).
                let upheld = sheet.dispute_upheld_at.is_some();
                Box::pin(async move {
                    if upheld {
                        Err(english(
                            "wl_attendance.adjustment.dispute_upheld_approve",
                            &[],
                        ))
                    } else {
                        Ok(())
                    }
                })
            })
            .effect(|sheet: &Timesheet, ctx, tx| {
                let sheet = sheet.clone();
                let by = ctx.actor().map(|user| user.id);
                let evidence = ctx.input_as::<ApproveInput>().ok().and_then(|i| i.evidence);
                Box::pin(async move {
                    approve_in(tx, &sheet, by, ApprovalBasis::OnBehalf, evidence).await
                })
            }),
        Transition::new(DEEM_APPROVE)
            .label("Deem approved")
            .system()
            .from([TimesheetStatus::Submitted])
            .to(TimesheetStatus::Approved)
            .guard("external_only", |sheet: &Timesheet, _ctx| {
                let external = sheet.approver_kind == ApproverKind::External;
                Box::pin(async move {
                    if external {
                        Ok(())
                    } else {
                        Err(english("wl_attendance.deem.not_external", &[]))
                    }
                })
            })
            .guard("clause_elapsed", |sheet: &Timesheet, ctx| {
                // N days since the latest `submit`, calendar days in
                // Asia/Dubai; the state `submitted` says nobody approved or
                // rejected since.
                let input = ctx.input_as::<DeemInput>().ok();
                let submitted = sheet.submitted_at.map(dubai_date);
                Box::pin(async move {
                    match (input, submitted) {
                        (Some(input), Some(submitted))
                            if input.days >= 1
                                && submitted + Duration::days(input.days) <= input.on =>
                        {
                            Ok(())
                        }
                        _ => Err(english("wl_attendance.deem.not_due", &[])),
                    }
                })
            })
            .effect(|sheet: &Timesheet, _ctx, tx| {
                let sheet = sheet.clone();
                Box::pin(
                    async move { approve_in(tx, &sheet, None, ApprovalBasis::Deemed, None).await },
                )
            }),
        Transition::new(APPROVE_BY_SYSTEM)
            .label("Approve")
            .system()
            .from([TimesheetStatus::Submitted])
            .to(TimesheetStatus::Approved)
            .guard("reconciliation_only", |sheet: &Timesheet, _ctx| {
                let allowed = sheet.is_adjustment()
                    && sheet.origin == Some(AdjustmentOrigin::ProvisionalReconciliation);
                Box::pin(async move {
                    if allowed {
                        Ok(())
                    } else {
                        Err(english(
                            "wl_attendance.approval.system_reconciliation_only",
                            &[],
                        ))
                    }
                })
            })
            .effect(|sheet: &Timesheet, _ctx, tx| {
                let sheet = sheet.clone();
                Box::pin(async move { copy_approval_in(tx, &sheet).await })
            }),
        Transition::new(REJECT)
            .label("Reject")
            .tone("danger")
            .from([TimesheetStatus::Submitted])
            .to(TimesheetStatus::Open)
            .reason_required(),
    ]
}

/// The kind the input says the actor approves as must be the timesheet's.
fn check_kind(sheet: &Timesheet, ctx: &TransitionCtx) -> std::result::Result<(), String> {
    let wanted = kind_text(sheet.approver_kind);
    match ctx
        .input_as::<ApproveInput>()
        .ok()
        .and_then(|i| i.approver_kind)
    {
        Some(given) if given != wanted => Err(english(
            "wl_attendance.approval.kind_mismatch",
            &[("kind", &wanted)],
        )),
        _ => Ok(()),
    }
}

/// An adjustment is never approved by the user who created it (`HRM.ACL.08`).
fn check_creator(sheet: &Timesheet, ctx: &TransitionCtx) -> std::result::Result<(), String> {
    let creator = sheet.created_by.filter(|_| sheet.is_adjustment());
    match (creator, ctx.actor().map(|user| user.id)) {
        (Some(creator), Some(actor)) if creator == actor => {
            Err(english("wl_attendance.approval.creator", &[]))
        }
        _ => Ok(()),
    }
}

pub(crate) fn kind_text(kind: ApproverKind) -> &'static str {
    match kind {
        ApproverKind::Internal => "internal",
        ApproverKind::External => "external",
    }
}

/// The effect of both approving transitions: who and when, the evidence, and
/// the lock of the attendance the hours came from.
async fn approve_in(
    tx: &mut Transaction,
    sheet: &Timesheet,
    by: Option<i64>,
    basis: ApprovalBasis,
    evidence: Option<Evidence>,
) -> Result {
    Timesheet::query()
        .where_(Timesheet::ID.eq(sheet.id))
        .update(
            &mut *tx,
            &[
                ("approval_basis", &Some(basis.as_str().to_owned())),
                ("approved_by", &by),
                ("approved_at", &Some(now())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    if let Some(evidence) = evidence {
        ApprovalEvidence::create(
            &mut *tx,
            ApprovalEvidence {
                timesheet_id: sheet.id,
                document_id: evidence.document_id,
                approver_name: evidence.approver_name.trim().to_owned(),
                signed_on: evidence.signed_on,
                attached_by: by,
                ..ApprovalEvidence::default()
            },
        )
        .await?;
    }
    // An adjustment has no attendance of its own: nothing to lock.
    if !sheet.is_adjustment() {
        let mut records: Vec<i64> = TimesheetEntry::query()
            .where_(TimesheetEntry::TIMESHEET_ID.eq(sheet.id))
            .get(&mut *tx)
            .await?
            .iter()
            .filter_map(|e| e.attendance_record_id)
            .collect();
        records.sort_unstable();
        records.dedup();
        for id in records {
            let accepted = AttendanceRecord::find(&mut *tx, id)
                .await?
                .is_some_and(|r| r.status == AttendanceStatus::Accepted);
            if accepted {
                let _locked = review::lock_in(tx, id).await?;
            }
        }
    }
    Ok(())
}

/// The effect of [`APPROVE_BY_SYSTEM`]: the approver and the approval basis
/// are those of the original's approval, since the original's approval
/// already covers these hours.
async fn copy_approval_in(tx: &mut Transaction, sheet: &Timesheet) -> Result {
    let original = match sheet.adjusts_id {
        Some(id) => Timesheet::find(&mut *tx, id).await?,
        None => None,
    };
    let (by, basis) = original.map_or((None, ApprovalBasis::Approved), |original| {
        (
            original.approved_by,
            original.approval_basis.unwrap_or(ApprovalBasis::Approved),
        )
    });
    Timesheet::query()
        .where_(Timesheet::ID.eq(sheet.id))
        .update(
            &mut *tx,
            &[
                ("approval_basis", &Some(basis.as_str().to_owned())),
                ("approved_by", &by),
                ("approved_at", &Some(now())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// Whether `user` is a client approver of `sheet`: they hold the timesheet's
/// `approver_role` and the app's
/// [`ApproverResolver`](wl_kernel::timesheets::ApproverResolver) names them
/// among the approvers of the timesheet's context, that is of its client.
/// The role alone is global and says nothing about which client a user acts
/// for, so it never suffices (an approver of one client cannot approve the
/// timesheets of another).
async fn is_client_approver(state: &AppState, user: i64, sheet: &Timesheet) -> Result<bool> {
    let Some(role) = sheet.approver_role.as_deref() else {
        return Ok(false);
    };
    let context = sheet.context();
    let named = approver_resolver(state)
        .client_approvers(state, context.as_ref())
        .await?
        .iter()
        .any(|r| matches!(r, NotifyRecipient::User { id } if *id == user));
    if !named {
        return Ok(false);
    }
    Ok(users_with_role(&state.db, role)
        .await?
        .iter()
        .any(|u| u.id == user))
}

/// The kind `user` approves `sheet` as: the timesheet's own kind when they
/// may approve as it, else the other kind they may approve as; `None` when
/// they may approve as neither.
///
/// An internal approver holds `timesheets.approve` in the scope of the
/// timesheet's context; an external one holds the timesheet's `approver_role`.
///
/// # Errors
///
/// The database's errors.
pub async fn approver_kind_for(
    state: &AppState,
    user: i64,
    sheet: &Timesheet,
) -> Result<Option<ApproverKind>> {
    let internal = access::holds_in_context(
        state,
        user,
        permissions::TIMESHEETS_APPROVE,
        sheet.context().as_ref(),
    )
    .await?;
    let external = is_client_approver(state, user, sheet).await?;
    Ok(match (sheet.approver_kind, internal, external) {
        (ApproverKind::Internal, true, _) | (_, true, false) => Some(ApproverKind::Internal),
        (ApproverKind::External, _, true) | (_, false, true) => Some(ApproverKind::External),
        _ => None,
    })
}

/// The cost of `minutes` at `rate` fils an hour, rounded half away from zero.
pub fn cost_of(minutes: i64, rate: i64) -> i64 {
    let product = i128::from(minutes) * i128::from(rate);
    let half = if product < 0 { -30 } else { 30 };
    i64::try_from((product + half) / 60).unwrap_or(0)
}

/// The key under which the cost of one entry and category is posted.
pub fn cost_key(timesheet_id: i64, entry_id: i64, category: Category) -> String {
    format!(
        "attendance.timesheet:{timesheet_id}:entry:{entry_id}:{}",
        category.as_str()
    )
}

/// Posts the labour cost of `sheet` where its context posts cost from hours:
/// per entry with a cost object and per category, minutes times the hourly
/// rate of the resolver, signed for an adjustment. A category with no rate
/// posts nothing. Returns the number of postings made.
///
/// # Errors
///
/// What the cost poster answers.
pub async fn post_labour_cost(state: &AppState, sheet: &Timesheet) -> Result<usize> {
    let rates = cost_rates(state);
    let context = sheet.context();
    if !rates.posts_cost(state, context.as_ref()).await? {
        return Ok(0);
    }
    let mut posted = 0;
    for entry in TimesheetEntry::of_timesheet(&state.db, sheet.id).await? {
        let Some(cost_object) = entry.cost_object() else {
            continue;
        };
        for category in Category::ALL {
            let minutes = entry.minutes_of(category);
            if minutes == 0 {
                continue;
            }
            let Some(rate) = rates
                .hourly_rate(
                    state,
                    sheet.owner(),
                    context.as_ref(),
                    category.as_str(),
                    entry.date,
                )
                .await?
            else {
                continue;
            };
            let amount = cost_of(minutes, rate);
            if amount == 0 {
                continue;
            }
            post_cost(
                state,
                PostCost::new(
                    sheet.subject(),
                    cost_key(sheet.id, entry.id, category),
                    cost_object.clone(),
                    amount,
                ),
            )
            .await?;
            posted += 1;
        }
    }
    Ok(posted)
}

/// The minutes per category over the entries of `sheet`, signed; categories
/// that add up to nothing are left out.
pub(crate) async fn totals_of(db: &Db, sheet: &Timesheet) -> Result<Vec<CategoryMinutes>> {
    let mut sums: BTreeMap<usize, i64> = BTreeMap::new();
    for entry in TimesheetEntry::of_timesheet(db, sheet.id).await? {
        for (n, category) in Category::ALL.iter().enumerate() {
            *sums.entry(n).or_default() += entry.minutes_of(*category);
        }
    }
    Ok(sums
        .into_iter()
        .filter(|(_, minutes)| *minutes != 0)
        .map(|(n, minutes)| CategoryMinutes {
            category: Category::ALL[n].as_str().to_owned(),
            minutes,
        })
        .collect())
}

/// The `TimesheetApproved` event of the approved `sheet`.
///
/// # Errors
///
/// The database's errors.
pub async fn approved_event(state: &AppState, sheet: &Timesheet) -> Result<TimesheetApproved> {
    let adjustment = sheet.is_adjustment();
    Ok(TimesheetApproved {
        timesheet: sheet.subject(),
        owner: sheet.owner(),
        period_from: sheet.period_from,
        period_to: sheet.period_to,
        context: sheet.context(),
        totals: totals_of(&state.db, sheet).await?,
        approver_kind: kind_text(sheet.approver_kind).to_owned(),
        approval_basis: sheet
            .approval_basis
            .unwrap_or(ApprovalBasis::Approved)
            .as_str()
            .to_owned(),
        kind: sheet.kind.as_str().to_owned(),
        adjusts: sheet
            .adjusts_id
            .filter(|_| adjustment)
            .map(|id| wl_kernel::Subject::new(crate::timesheet::TIMESHEET_SUBJECT_TYPE, id)),
        origin: sheet
            .origin
            .filter(|_| adjustment)
            .map(|o| o.as_str().to_owned()),
        affects: sheet
            .affects
            .filter(|_| adjustment)
            .map(|a| a.as_str().to_owned()),
    })
}

/// After the commit: the labour cost, then `TimesheetApproved`.
pub(crate) async fn approved(state: &AppState, timesheet_id: i64) -> Result<Timesheet> {
    let sheet = Timesheet::find_or_404(&state.db, timesheet_id).await?;
    // Paid on provisional hours: payroll is its first consumer, so it is
    // locked at once (§5.7 step 3).
    let sheet = payroll::lock_when_paid_provisionally(state, sheet).await?;
    post_labour_cost(state, &sheet).await?;
    let event = approved_event(state, &sheet).await?;
    state.emit(event).await?;
    // The task to collect a signed sheet has done its work.
    escalation::close_task(state, timesheet_id).await?;
    Ok(sheet)
}

/// `actor` approves the `submitted` timesheet `timesheet_id` as the approver
/// of its kind.
///
/// # Errors
///
/// `NotFound`; `Forbidden` for a user who may approve as no kind; `422` when
/// the kind they may approve as is not the timesheet's (an internal approver
/// of an externally approved timesheet uses [`approve_on_behalf`]), for the
/// creator of an adjustment, or a timesheet that is not `submitted`.
pub async fn approve(state: &AppState, actor: &AuthUser, timesheet_id: i64) -> Result<Timesheet> {
    let sheet = Timesheet::find_or_404(&state.db, timesheet_id).await?;
    let Some(kind) = approver_kind_for(state, actor.id, &sheet).await? else {
        return Err(Error::Forbidden);
    };
    // `run` commits and emits the transition's event itself.
    let _outcome = sheet
        .transition(APPROVE)
        .by(actor)
        .input(&ApproveInput {
            approver_kind: Some(kind_text(kind).to_owned()),
            evidence: None,
        })
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    approved(state, timesheet_id).await
}

/// `actor` (`timesheets.approve_on_behalf` in the scope of the context)
/// approves the externally approved timesheet `timesheet_id` for the client,
/// with the signed sheet `evidence`.
///
/// # Errors
///
/// `NotFound`; `Forbidden` without the permission; `422` for a timesheet that
/// is not external, without evidence, for the creator of an adjustment, or a
/// timesheet that is not `submitted`.
pub async fn approve_on_behalf(
    state: &AppState,
    actor: &AuthUser,
    timesheet_id: i64,
    evidence: Option<Evidence>,
) -> Result<Timesheet> {
    let sheet = Timesheet::find_or_404(&state.db, timesheet_id).await?;
    if !access::holds_in_context(
        state,
        actor.id,
        permissions::TIMESHEETS_APPROVE_ON_BEHALF,
        sheet.context().as_ref(),
    )
    .await?
    {
        return Err(Error::Forbidden);
    }
    let _outcome = sheet
        .transition(APPROVE_ON_BEHALF)
        .by(actor)
        .input(&ApproveInput {
            approver_kind: None,
            evidence,
        })
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    approved(state, timesheet_id).await
}

/// `actor`, an approver of the timesheet (as for [`approve`], or holding
/// `timesheets.approve_on_behalf`), sends the `submitted` timesheet
/// `timesheet_id` back to `open` with a `reason`. Whoever submitted it is
/// told.
///
/// # Errors
///
/// `NotFound`; `Forbidden` for a user who may not approve it; `422` without a
/// reason or for a timesheet that is not `submitted`.
pub async fn reject(
    state: &AppState,
    actor: &AuthUser,
    timesheet_id: i64,
    reason: &str,
) -> Result<Timesheet> {
    let sheet = Timesheet::find_or_404(&state.db, timesheet_id).await?;
    let may = approver_kind_for(state, actor.id, &sheet).await?.is_some()
        || access::holds_in_context(
            state,
            actor.id,
            permissions::TIMESHEETS_APPROVE_ON_BEHALF,
            sheet.context().as_ref(),
        )
        .await?;
    if !may {
        return Err(Error::Forbidden);
    }
    let mut run = sheet.transition(REJECT).by(actor);
    if !reason.trim().is_empty() {
        run = run.reason(reason.trim().to_owned());
    }
    let _outcome = run.run(state).await.map_err(|e| e.into_error())?;
    let stored = Timesheet::find_or_404(&state.db, timesheet_id).await?;
    notices::timesheet_rejected(state, &stored, reason.trim()).await?;
    // It no longer waits for the client: the task to collect a sheet closes.
    escalation::close_task(state, timesheet_id).await?;
    Ok(stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cost_is_minutes_times_the_hourly_rate_rounded_half_away_from_zero() {
        // AED 25 an hour is 2,500 fils.
        assert_eq!(cost_of(-480, 2_500), -20_000);
        assert_eq!(cost_of(480, 2_500), 20_000);
        assert_eq!(cost_of(90, 2_500), 3_750);
        // 1 minute at 2,510 fils is 41.83 fils.
        assert_eq!(cost_of(1, 2_510), 42);
        assert_eq!(cost_of(-1, 2_510), -42);
        assert_eq!(cost_of(0, 2_500), 0);
    }

    #[test]
    fn the_cost_of_an_entry_and_category_has_its_own_key() {
        assert_eq!(
            cost_key(4, 9, Category::Overtime),
            "attendance.timesheet:4:entry:9:overtime"
        );
    }

    #[test]
    fn the_machine_has_the_three_approval_transitions() {
        let machine = Timesheet::machine();
        for name in [APPROVE, APPROVE_ON_BEHALF] {
            let t = machine.find(name).unwrap();
            assert_eq!(t.from_states(), ["submitted"]);
            assert_eq!(t.to_state(), Some("approved"));
        }
        let reject = machine.find(REJECT).unwrap();
        assert_eq!(reject.from_states(), ["submitted"]);
        assert_eq!(reject.to_state(), Some("open"));
    }
}
