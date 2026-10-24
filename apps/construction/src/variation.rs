//! The Variation (02 Construction §4.5, §6, §7.1 *Variation*; BL-CON-11): a
//! change to the scope or the time of a project, recorded from a site
//! instruction or a client request, priced by the QS, submitted to the client
//! by the project manager, and approved or rejected by the client.
//!
//! ```text
//! draft --submit--> submitted --approve--> approved
//!                   submitted --reject--> rejected
//! rejected --resubmit--> rejected          (a new revision n+1 in draft)
//! rejected --claim--> claimed --settle_claim--> approved
//! rejected, claimed --abandon--> abandoned (the MD approves, CON.ACL.16)
//! draft, submitted --withdraw--> withdrawn
//! ```
//!
//! This is the minimal base BL-CON-11, BL-CON-12 and BL-CON-13 need: the
//! entity, its states and transitions and the effects of §7.1. The omission
//! (a quantity taken out of a BOQ line, never more than is not yet done) is in
//! [`crate::omission`]; an approved extension of time moves the completion date
//! of the project by the days granted, and an addition priced at one amount and
//! approved at another shows its gap in the variation report ([`report_of`]).
//!
//! - The QS (`variations.record`) records and prices a draft ([`create`],
//!   [`update_draft`]) and records the client's decision (`approve`, `reject`,
//!   `settle_claim`); only the project manager (`variations.submit`) submits,
//!   resubmits, claims, abandons or withdraws (`CON.ACL.06`).
//! - **Reject**: the reason is recorded; when the work was executed before the
//!   approval its cost stays on the variation's own cost code, marked
//!   **unrecovered** ([`Variation::unrecovered_amount`]), and the QS and the
//!   project manager are told. The cost booked after the rejection counts too
//!   ([`book_cost`]). Nothing is moved to another cost code.
//! - **Resubmit**: a rejected revision is read-only history. The spec's
//!   `resubmit` goes `rejected` to `draft`; the rejected revision must stay
//!   `rejected`, so the transition is a self-loop on the rejected revision whose
//!   effect creates revision n+1 in `draft` (copying it, with the cost booked)
//!   and records "resubmitted as rev n+1" on the old one, which can then no
//!   longer be resubmitted, claimed or abandoned.
//! - **Claim** needs work executed on a site instruction; a claim is listed in
//!   the project's claims for the final account ([`claims_of`]).
//! - **Abandon** (`CON.ACL.16`, any amount) is deferred as an approval request
//!   for the MD, exactly as the termination of a project is: `abandon` asks,
//!   the kernel runs `abandon_approved` (same guard and effect) at the final
//!   approval; on rejection or return the variation stays as it was. The cost
//!   is written into the project's unrecovered variations
//!   ([`unrecovered_of`]).
//! - **Approve** and `settle_claim` add the approved amount to the contract
//!   value and the budget (an omission is stored negative and reduces them) and
//!   to the next IPA ([`next_ipa_amount`]: the approved variations no IPA has
//!   valued yet; valuing them into an IPA comes with the IPA calculation).
//!
//! Decisions where the spec is silent:
//!
//! - The PDF the project manager sends at `submit` is not generated here.
//! - A variation's code is `VO-` and its number in the project, from 1; its
//!   cost code is the code in lower case.
//! - The cost counters are kept on each revision; they are booked by
//!   [`book_cost`], which the callers that post cost to the variation's cost
//!   code use, once per cost line.

use renox::auth::permissions::Scope;
use renox::chrono::{Days, NaiveDate};
use renox::db::{DateTime, Db, Transaction};
use renox::prelude::*;
use renox::state_machine::{
    Machine, Outcome, StateMachine, Transition, TransitionCtx, TransitionError, Transitioned,
};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::PolicyDefault;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{PostCost, post_cost};
use wl_kernel::{Subject, SubjectMachine};

use crate::expiry::managers;
use crate::notices::{aed, holders, tell};
use crate::permissions;
use crate::project::{self, Project};
use crate::texts::english;

/// The subject type of a variation.
pub const SUBJECT_TYPE: &str = "construction.variation";

/// The `submit` transition's name.
pub const SUBMIT: &str = "submit";
/// The `approve` transition's name.
pub const APPROVE: &str = "approve";
/// The `reject` transition's name.
pub const REJECT: &str = "reject";
/// The `resubmit` transition's name.
pub const RESUBMIT: &str = "resubmit";
/// The `claim` transition's name.
pub const CLAIM: &str = "claim";
/// The `settle_claim` transition's name.
pub const SETTLE_CLAIM: &str = "settle_claim";
/// `abandon`: the project manager's request (`CON.ACL.16`).
pub const ABANDON: &str = "abandon";
/// `abandon_approved`: the kernel runs it at the final approval.
pub const ABANDON_APPROVED: &str = "abandon_approved";
/// The `withdraw` transition's name.
pub const WITHDRAW: &str = "withdraw";

/// The approval policies' action key of `abandon` (`CON.ACL.16`).
pub const ABANDON_ACTION: &str = "construction.variation_abandon";

/// What a variation changes.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VariationKind {
    /// Extra or changed work: adds to the contract value.
    #[default]
    Addition,
    /// Work taken out: reduces the contract value.
    Omission,
    /// An extension of time: moves the completion date.
    Time,
}

/// Where a variation stands (§7.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VariationStatus {
    /// Recorded from a site instruction; being priced.
    #[default]
    Draft,
    /// Sent to the client.
    Submitted,
    /// Approved, possibly for a different amount (terminal).
    Approved,
    /// Not accepted; if the work was executed its cost is unrecovered until
    /// resubmitted, claimed or abandoned.
    Rejected,
    /// Pursued as a claim in the final account.
    Claimed,
    /// Given up; its cost stays unrecovered (terminal).
    Abandoned,
    /// Withdrawn by us (terminal).
    Withdrawn,
}

/// A change to the scope or the time of a project. Money is fils.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "construction_variations", state_machine, index(project_id))]
pub struct Variation {
    /// The primary key.
    pub id: i64,
    /// The project.
    pub project_id: i64,
    /// `VO-07`; every revision of the variation has the same code.
    pub code: String,
    /// The revision, from 1.
    pub revision: i64,
    /// What it changes.
    pub kind: VariationKind,
    /// What it is.
    pub description: String,
    /// The client's or consultant's instruction reference.
    pub site_instruction: Option<String>,
    /// Whether the work was done before the client approved.
    pub executed_before_approval: bool,
    /// The variation's own cost code.
    pub cost_code: String,
    /// The price asked, negative for an omission.
    pub priced_amount: i64,
    /// The amount the client approved, negative for an omission.
    pub approved_amount: Option<i64>,
    /// The cost booked on the cost code.
    pub cost_booked: i64,
    /// The part of the cost nothing recovers.
    pub unrecovered_amount: i64,
    /// The client's reference of the approval.
    pub client_reference: Option<String>,
    /// Why the client rejected it.
    pub rejection_reason: Option<String>,
    /// The revision that replaced this rejected one.
    pub resubmitted_as: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: VariationStatus,
    /// The days of the extension of time asked.
    pub days_asked: Option<i64>,
    /// The days the client granted.
    pub days_granted: Option<i64>,
    /// The cause of the delay.
    pub event: Option<String>,
    /// The date of the delay event.
    pub event_date: Option<NaiveDate>,
    /// The day it was submitted to the client (the latest entry into
    /// `submitted`).
    pub submitted_on: Option<NaiveDate>,
    /// The notice date: the day it was submitted to the client.
    pub notice_date: Option<NaiveDate>,
    /// Whether the notice came after the contract's notice deadline.
    pub notice_late: bool,
    /// The IPA that valued the approved variation.
    pub ipa_id: Option<i64>,
    /// The BOQ line an omission takes work out of.
    pub boq_line_id: Option<i64>,
    /// The quantity an omission takes out.
    pub omitted_quantity: Option<i64>,
    /// The day the project manager was reminded to chase the client
    /// (`CON.RUL.09`), for the latest submission.
    pub chased_on: Option<NaiveDate>,
    /// The day the MD was notified, for the latest submission.
    pub escalated_on: Option<NaiveDate>,
    /// The day the project manager and QS were warned 7 days before the notice
    /// deadline (`CON.RUL.10`).
    pub notice_warned_7_on: Option<NaiveDate>,
    /// The day they were warned 2 days before it.
    pub notice_warned_2_on: Option<NaiveDate>,
    /// How much of an approved omission progress had overtaken: the QS is
    /// warned to record it as a claim.
    pub excess_quantity: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Variation {
    /// The variation as a [`Subject`]: `construction.variation:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The scope roles are assigned in: the project of the variation.
    pub fn scope(&self) -> Scope {
        Scope::new(project::SUBJECT_TYPE, self.project_id)
    }

    /// The variation as its users name it: `VO-07 rev 2`.
    pub fn label(&self) -> String {
        format!("{} rev {}", self.code, self.revision)
    }

    /// Whether the work was executed on a site instruction before the
    /// approval.
    pub fn executed_on_instruction(&self) -> bool {
        self.executed_before_approval
            && self
                .site_instruction
                .as_deref()
                .is_some_and(|reference| !reference.trim().is_empty())
    }

    /// Whether a later revision replaced it.
    pub fn is_resubmitted(&self) -> bool {
        self.resubmitted_as.is_some()
    }

    /// The text "resubmitted as rev n+1" of a replaced revision.
    pub fn resubmitted_text(&self) -> Option<String> {
        self.resubmitted_as.map(|revision| {
            english(
                "construction.variation.resubmitted_as",
                &[("rev", &revision)],
            )
        })
    }
}

/// What a transition of a variation carries to its guard and effect.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariationInput {
    /// The client's reference of its decision (`approve`, `settle_claim`).
    #[serde(default)]
    pub client_reference: String,
    /// The amount the client approved, in fils (`approve`, `settle_claim`);
    /// for an omission its size, the variation stores it negative.
    #[serde(default)]
    pub approved_amount: Option<i64>,
    /// The days the client granted (a time variation).
    #[serde(default)]
    pub days_granted: Option<i64>,
    /// The quantity of the BOQ line not yet done (`submit` of an omission),
    /// read from the latest recorded progress.
    #[serde(default)]
    pub not_yet_done: Option<i64>,
}

/// The input of a transition of a variation.
pub(crate) fn variation_input(ctx: &TransitionCtx) -> VariationInput {
    ctx.input_as::<VariationInput>().unwrap_or_default()
}

/// The amount as a variation of `kind` stores it: an omission is negative.
pub fn signed(kind: VariationKind, amount: i64) -> i64 {
    match kind {
        VariationKind::Omission => -amount.abs(),
        _ => amount,
    }
}

/// Why `variation` cannot be submitted, if it cannot: it is priced (a time
/// variation has the days asked and the delay event instead).
pub fn price_problem(variation: &Variation) -> Option<String> {
    match variation.kind {
        VariationKind::Time => {
            let days = variation.days_asked.is_some_and(|days| days > 0);
            let event = variation
                .event
                .as_deref()
                .is_some_and(|event| !event.trim().is_empty())
                && variation.event_date.is_some();
            if !days {
                Some(english("construction.variation.days_needed", &[]))
            } else if !event {
                Some(english("construction.variation.event_needed", &[]))
            } else {
                None
            }
        }
        _ if variation.priced_amount == 0 => {
            Some(english("construction.variation.price_needed", &[]))
        }
        _ => None,
    }
}

/// Why the client's decision `input` cannot be recorded on `variation`, if it
/// cannot: the client's reference, and the approved amount (a time variation:
/// the days granted, which may be 0).
pub fn decision_problem(variation: &Variation, input: &VariationInput) -> Option<String> {
    if input.client_reference.trim().is_empty() {
        return Some(english("construction.variation.reference_needed", &[]));
    }
    match variation.kind {
        VariationKind::Time => match input.days_granted {
            Some(days) if days >= 0 => None,
            _ => Some(english("construction.variation.granted_needed", &[])),
        },
        VariationKind::Addition => match input.approved_amount {
            Some(amount) if amount > 0 => None,
            _ => Some(english("construction.variation.approved_needed", &[])),
        },
        VariationKind::Omission => match input.approved_amount {
            Some(amount) if amount != 0 => None,
            _ => Some(english("construction.variation.approved_needed", &[])),
        },
    }
}

/// Why `variation` cannot be rejected, resubmitted, claimed or abandoned,
/// if it cannot: a later revision replaced it.
pub fn replaced_problem(variation: &Variation) -> Option<String> {
    variation.resubmitted_text().map(|text| {
        english(
            "construction.variation.replaced",
            &[("text", &text), ("label", &variation.label())],
        )
    })
}

/// Why `variation` cannot be claimed, if it cannot: the work was executed on
/// a site instruction, and no later revision replaced it.
pub fn claim_problem(variation: &Variation) -> Option<String> {
    if let Some(problem) = replaced_problem(variation) {
        return Some(problem);
    }
    if variation.executed_on_instruction() {
        None
    } else {
        Some(english("construction.variation.not_executed", &[]))
    }
}

/// The effect of `submit` inside its transaction: the day of the submission
/// is the notice date (late when after the notice deadline, `CON.RUL.10`) and
/// starts the clock of `CON.RUL.09` again.
pub(crate) async fn submit_effects(tx: &mut Transaction, variation: &Variation) -> Result {
    let today = today();
    let day = Some(today);
    let none: Option<NaiveDate> = None;
    // `CON.RUL.10`: a notice after the contract's deadline is marked late,
    // never refused.
    let project = Project::find_or_404(&mut *tx, variation.project_id).await?;
    let late = crate::notice_deadline::deadline(&project, variation).is_some_and(|due| today > due);
    Variation::query()
        .where_(Variation::ID.eq(variation.id))
        .update(
            &mut *tx,
            &[
                ("submitted_on", &day),
                ("notice_date", &day),
                ("notice_late", &late),
                ("chased_on", &none),
                ("escalated_on", &none),
            ],
        )
        .await?;
    Ok(())
}

/// The effects `approve` and `settle_claim` share, inside their transaction:
/// the decision is recorded, and the approved amount is added to the contract
/// value and the budget of the project (an omission reduces them).
pub(crate) async fn approve_effects(
    tx: &mut Transaction,
    variation: &Variation,
    input: &VariationInput,
) -> Result {
    let amount = match variation.kind {
        VariationKind::Time => None,
        kind => input.approved_amount.map(|amount| signed(kind, amount)),
    };
    let reference = Some(input.client_reference.trim().to_owned());
    let days = match variation.kind {
        VariationKind::Time => input.days_granted,
        _ => None,
    };
    Variation::query()
        .where_(Variation::ID.eq(variation.id))
        .update(
            &mut *tx,
            &[
                ("approved_amount", &amount),
                ("client_reference", &reference),
                ("days_granted", &days),
            ],
        )
        .await?;
    if let Some(amount) = amount {
        let project = Project::find_or_404(&mut *tx, variation.project_id).await?;
        let contract_value = project.contract_value + amount;
        let budget = project.budget + amount;
        Project::query()
            .where_(Project::ID.eq(project.id))
            .update(
                &mut *tx,
                &[("contract_value", &contract_value), ("budget", &budget)],
            )
            .await?;
    }
    // An approved extension of time moves the completion date by the days
    // granted (none to move when the contract has no completion date).
    if variation.kind == VariationKind::Time
        && let Some(days) = days.and_then(|days| u64::try_from(days).ok())
    {
        let project = Project::find_or_404(&mut *tx, variation.project_id).await?;
        if let Some(moved) = project
            .completion_due_on
            .and_then(|due| due.checked_add_days(Days::new(days)))
        {
            let moved = Some(moved);
            Project::query()
                .where_(Project::ID.eq(project.id))
                .update(&mut *tx, &[("completion_due_on", &moved)])
                .await?;
        }
    }
    Ok(())
}

/// The effect of `reject` inside its transaction: the reason is recorded, and
/// the cost of work executed before the approval is marked unrecovered.
async fn reject_effects(
    tx: &mut Transaction,
    variation: &Variation,
    reason: Option<String>,
) -> Result {
    let unrecovered = if variation.executed_before_approval {
        variation.cost_booked
    } else {
        0
    };
    Variation::query()
        .where_(Variation::ID.eq(variation.id))
        .update(
            &mut *tx,
            &[
                ("rejection_reason", &reason),
                ("unrecovered_amount", &unrecovered),
            ],
        )
        .await?;
    Ok(())
}

/// The effect of `resubmit` inside its transaction: revision n+1 in `draft`
/// copies the rejected one, which records "resubmitted as rev n+1".
async fn resubmit_effects(tx: &mut Transaction, variation: &Variation) -> Result {
    let next = variation.revision + 1;
    let mut copy = Variation {
        id: 0,
        revision: next,
        status: VariationStatus::Draft,
        approved_amount: None,
        client_reference: None,
        rejection_reason: None,
        resubmitted_as: None,
        days_granted: None,
        unrecovered_amount: 0,
        submitted_on: None,
        notice_date: None,
        notice_late: false,
        chased_on: None,
        escalated_on: None,
        notice_warned_7_on: None,
        notice_warned_2_on: None,
        ipa_id: None,
        created_at: None,
        updated_at: None,
        ..variation.clone()
    };
    copy.insert(&mut *tx).await?;
    wl_kernel::search::index_in(&mut *tx, &Subject::new(SUBJECT_TYPE, copy.id)).await?;
    let revision = Some(next);
    Variation::query()
        .where_(Variation::ID.eq(variation.id))
        .update(&mut *tx, &[("resubmitted_as", &revision)])
        .await?;
    Ok(())
}

/// The effect of `abandon` inside its transaction: the cost booked is written
/// into the project's unrecovered variations.
async fn abandon_effects(tx: &mut Transaction, variation: &Variation) -> Result {
    let unrecovered = variation.cost_booked.max(variation.unrecovered_amount);
    Variation::query()
        .where_(Variation::ID.eq(variation.id))
        .update(&mut *tx, &[("unrecovered_amount", &unrecovered)])
        .await?;
    Ok(())
}

/// A transition of the variation by the project manager (`variations.submit`)
/// or the QS (`variations.record`) in the project's scope.
fn user(name: &str, label: &str, permission: &'static str) -> Transition<Variation> {
    Transition::new(name)
        .label(label)
        .permission_in(permission, |variation: &Variation| variation.scope())
}

/// The client's decision: `from` to `to`, with its input checked.
fn decision(transition: Transition<Variation>) -> Transition<Variation> {
    transition
        .guard("decision", |variation: &Variation, ctx| {
            let problem = decision_problem(variation, &variation_input(ctx));
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|variation: &Variation, ctx, tx| {
            let variation = variation.clone();
            let input = variation_input(ctx);
            Box::pin(async move { approve_effects(&mut *tx, &variation, &input).await })
        })
}

/// The guard and the effect `abandon` and `abandon_approved` share.
fn abandon_rules(transition: Transition<Variation>) -> Transition<Variation> {
    transition
        .from([VariationStatus::Rejected, VariationStatus::Claimed])
        .to(VariationStatus::Abandoned)
        .guard("not_replaced", |variation: &Variation, _ctx| {
            let problem = replaced_problem(variation);
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|variation: &Variation, _ctx, tx| {
            let variation = variation.clone();
            Box::pin(async move { abandon_effects(&mut *tx, &variation).await })
        })
}

impl StateMachine for Variation {
    type State = VariationStatus;
    const COLUMN: renox::db::Col<Variation, VariationStatus> = Variation::STATUS;

    fn machine() -> Machine<Self> {
        use VariationStatus::{
            Abandoned, Approved, Claimed, Draft, Rejected, Submitted, Withdrawn,
        };
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Submitted)
            .tone("info")
            .state(Approved)
            .tone("success")
            .terminal()
            .state(Rejected)
            .tone("error")
            .state(Claimed)
            .tone("warning")
            .state(Abandoned)
            .tone("neutral")
            .terminal()
            .state(Withdrawn)
            .tone("neutral")
            .terminal()
            .transition(
                user(SUBMIT, "Submit", permissions::VARIATIONS_SUBMIT)
                    .tone("primary")
                    .from([Draft])
                    .to(Submitted)
                    .guard("priced", |variation: &Variation, _ctx| {
                        let problem = price_problem(variation);
                        Box::pin(async move { problem.map_or(Ok(()), Err) })
                    })
                    .guard("omission", |variation: &Variation, ctx| {
                        let problem =
                            crate::omission::submit_problem(variation, &variation_input(ctx));
                        Box::pin(async move { problem.map_or(Ok(()), Err) })
                    })
                    .effect(|variation: &Variation, _ctx, tx| {
                        let variation = variation.clone();
                        Box::pin(async move { submit_effects(&mut *tx, &variation).await })
                    }),
            )
            .transition(decision(
                user(APPROVE, "Approve", permissions::VARIATIONS_RECORD)
                    .tone("primary")
                    .from([Submitted])
                    .to(Approved),
            ))
            .transition(
                user(REJECT, "Reject", permissions::VARIATIONS_RECORD)
                    .tone("danger")
                    .reason_required()
                    .from([Submitted])
                    .to(Rejected)
                    .effect(|variation: &Variation, ctx, tx| {
                        let variation = variation.clone();
                        let reason = ctx.reason().map(|reason| reason.trim().to_owned());
                        Box::pin(async move { reject_effects(&mut *tx, &variation, reason).await })
                    }),
            )
            .transition(
                user(RESUBMIT, "Resubmit", permissions::VARIATIONS_SUBMIT)
                    .from([Rejected])
                    .to(Rejected)
                    .guard("not_replaced", |variation: &Variation, _ctx| {
                        let problem = replaced_problem(variation);
                        Box::pin(async move { problem.map_or(Ok(()), Err) })
                    })
                    .effect(|variation: &Variation, _ctx, tx| {
                        let variation = variation.clone();
                        Box::pin(async move { resubmit_effects(&mut *tx, &variation).await })
                    }),
            )
            .transition(
                user(CLAIM, "Claim", permissions::VARIATIONS_SUBMIT)
                    .from([Rejected])
                    .to(Claimed)
                    .guard("executed", |variation: &Variation, _ctx| {
                        let problem = claim_problem(variation);
                        Box::pin(async move { problem.map_or(Ok(()), Err) })
                    }),
            )
            .transition(decision(
                user(
                    SETTLE_CLAIM,
                    "Settle the claim",
                    permissions::VARIATIONS_RECORD,
                )
                .tone("primary")
                .from([Claimed])
                .to(Approved),
            ))
            .transition(abandon_rules(
                user(ABANDON, "Abandon", permissions::VARIATIONS_SUBMIT).tone("danger"),
            ))
            .transition(abandon_rules(
                Transition::new(ABANDON_APPROVED)
                    .label("Abandon (approved)")
                    .system(),
            ))
            .transition(
                user(WITHDRAW, "Withdraw", permissions::VARIATIONS_SUBMIT)
                    .from([Draft, Submitted])
                    .to(Withdrawn),
            )
    }
}

/// The default approval policy of `abandon` (`CON.ACL.16`): every request
/// needs one step by the MD (a holder of `projects.cancel`, the company admin
/// by default), whatever the amount, never the requester.
pub fn abandon_policy() -> PolicyDefault {
    PolicyDefault::new(ABANDON_ACTION).step_or_permission(
        [wl_kernel::COMPANY_ADMIN_ROLE],
        permissions::PROJECTS_CANCEL,
    )
}

/// The Variation in the kernel's registry: its approval action for `abandon`
/// and the project as the scope where approvers are looked for.
pub fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<Variation>(SUBJECT_TYPE)
        .action(ABANDON, ABANDON_ACTION)
        .scope(|variation: &Variation| variation.scope())
        .build()
}

/// Registers the machine with the kernel and with `rnx state:diagram`, and
/// the default policy of `abandon`.
pub(crate) fn register() {
    subject_machine().register();
    renox::state_machine::register::<Variation>();
    abandon_policy().register();
}

/// What the QS enters for a variation.
#[derive(Debug, Clone, Default)]
pub struct NewVariation {
    /// What it changes.
    pub kind: VariationKind,
    /// What it is.
    pub description: String,
    /// The instruction reference.
    pub site_instruction: Option<String>,
    /// Whether the work was done before the client approved.
    pub executed_before_approval: bool,
    /// The price, in fils; an omission's is stored negative.
    pub priced_amount: i64,
    /// The days asked (a time variation).
    pub days_asked: Option<i64>,
    /// The cause of the delay (a time variation).
    pub event: Option<String>,
    /// The date of the delay event.
    pub event_date: Option<NaiveDate>,
    /// The BOQ line an omission takes work out of.
    pub boq_line_id: Option<i64>,
    /// The quantity an omission takes out.
    pub omitted_quantity: Option<i64>,
}

/// Why `new` cannot be recorded, if it cannot: it says what it is.
pub fn new_problem(new: &NewVariation) -> Option<String> {
    if new.description.trim().is_empty() {
        return Some(english("construction.variation.description_needed", &[]));
    }
    if new.priced_amount < 0 && new.kind != VariationKind::Omission {
        return Some(english("construction.variation.price_negative", &[]));
    }
    None
}

/// `user` (the QS) records a variation of project `project_id` as a `draft`.
///
/// # Errors
///
/// `Forbidden` without `variations.record` in the project, `Guard` (422) when
/// [`new_problem`] says so.
pub async fn create(
    state: &AppState,
    user: &AuthUser,
    project_id: i64,
    new: &NewVariation,
) -> std::result::Result<Variation, TransitionError> {
    let project = Project::find(&state.db, project_id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    if !user.has_permission_in(permissions::VARIATIONS_RECORD, &project.scope()) {
        return Err(TransitionError::Forbidden);
    }
    if let Some(problem) = new_problem(new) {
        return Err(TransitionError::Guard(problem));
    }
    let number = Variation::query()
        .where_(Variation::PROJECT_ID.eq(project_id))
        .where_(Variation::REVISION.eq(1))
        .count(&state.db)
        .await?
        + 1;
    let code = format!("VO-{number:02}");
    let mut variation = Variation {
        project_id,
        cost_code: code.to_lowercase(),
        code,
        revision: 1,
        ..Variation::default()
    };
    apply(
        &mut variation,
        &crate::omission::priced(&state.db, new).await?,
    );
    variation.save(&state.db).await?;
    crate::search::index(&state.db, &Subject::new(SUBJECT_TYPE, variation.id)).await?;
    Ok(Variation::find_or_404(&state.db, variation.id).await?)
}

/// Puts the fields `new` carries on `variation`.
fn apply(variation: &mut Variation, new: &NewVariation) {
    variation.kind = new.kind;
    variation.description = new.description.trim().to_owned();
    variation.site_instruction = new
        .site_instruction
        .as_deref()
        .map(str::trim)
        .filter(|reference| !reference.is_empty())
        .map(str::to_owned);
    variation.executed_before_approval = new.executed_before_approval;
    variation.priced_amount = signed(new.kind, new.priced_amount);
    variation.days_asked = new.days_asked;
    variation.event = new
        .event
        .as_deref()
        .map(str::trim)
        .filter(|event| !event.is_empty())
        .map(str::to_owned);
    variation.event_date = new.event_date;
    variation.boq_line_id = new.boq_line_id;
    variation.omitted_quantity = new.omitted_quantity;
}

/// `user` (the QS) changes the price or the evidence of the `draft` `id`; a
/// revision made by `resubmit` is changed this way before it is submitted.
///
/// # Errors
///
/// `Forbidden` without `variations.record` in the project, `Guard` (422)
/// unless it is a `draft` or when [`new_problem`] says so.
pub async fn update_draft(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    new: &NewVariation,
) -> std::result::Result<Variation, TransitionError> {
    let mut variation = Variation::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    if !user.has_permission_in(permissions::VARIATIONS_RECORD, &variation.scope()) {
        return Err(TransitionError::Forbidden);
    }
    if variation.status != VariationStatus::Draft {
        return Err(TransitionError::Guard(english(
            "construction.variation.not_draft",
            &[("label", &variation.label())],
        )));
    }
    if let Some(problem) = new_problem(new) {
        return Err(TransitionError::Guard(problem));
    }
    apply(
        &mut variation,
        &crate::omission::priced(&state.db, new).await?,
    );
    variation.save(&state.db).await?;
    crate::search::index(&state.db, &Subject::new(SUBJECT_TYPE, id)).await?;
    Ok(Variation::find_or_404(&state.db, id).await?)
}

/// Runs the user transition `name` of variation `id`.
async fn run_user(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    name: &str,
    input: &VariationInput,
    reason: Option<String>,
) -> std::result::Result<Outcome<Variation>, TransitionError> {
    let variation = Variation::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let mut run = variation.transition(name).by(user).input(input);
    if let Some(reason) = reason {
        run = run.reason(reason);
    }
    run.run(state).await
}

/// `user` (the project manager) submits the `draft` variation `id` to the
/// client.
///
/// # Errors
///
/// `Forbidden` without `variations.submit` in the project, `NotFrom` unless it
/// is a `draft`, `Guard` (422) when it is not priced (a time variation: the
/// days asked and the delay event) or an omission is above the quantity not
/// yet done.
pub async fn submit(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Variation, TransitionError> {
    let variation = Variation::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let input = crate::omission::prepare_submit(&state.db, &variation).await?;
    let _outcome = run_user(state, user, id, SUBMIT, &input, None).await?;
    Ok(Variation::find_or_404(&state.db, id).await?)
}

/// The client's decision on a variation.
#[derive(Debug, Clone, Default)]
pub struct Decision {
    /// The client's reference.
    pub client_reference: String,
    /// The approved amount, in fils (for an omission its size).
    pub approved_amount: Option<i64>,
    /// The days granted (a time variation).
    pub days_granted: Option<i64>,
}

impl Decision {
    fn input(&self) -> VariationInput {
        VariationInput {
            client_reference: self.client_reference.clone(),
            approved_amount: self.approved_amount,
            days_granted: self.days_granted,
            ..VariationInput::default()
        }
    }
}

/// `user` (the QS) records the client's approval of the `submitted` variation
/// `id`: the contract value, the budget and the next IPA take the approved
/// amount, and an approved extension moves the completion date.
///
/// # Errors
///
/// `Forbidden` without `variations.record` in the project, `NotFrom` unless it
/// is `submitted`, `Guard` (422) without the client's reference or the
/// approved amount (the days granted for a time variation).
pub async fn approve(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    decision: &Decision,
) -> std::result::Result<Variation, TransitionError> {
    let _outcome = run_user(state, user, id, APPROVE, &decision.input(), None).await?;
    let _warned = crate::omission::warn_after_approval(state, id).await?;
    Ok(Variation::find_or_404(&state.db, id).await?)
}

/// `user` (the QS) records the client's rejection of the `submitted`
/// variation `id` with its reason.
///
/// # Errors
///
/// `Forbidden` without `variations.record` in the project, `NotFrom` unless it
/// is `submitted`, `Guard` (422) without a reason.
pub async fn reject(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: &str,
) -> std::result::Result<Variation, TransitionError> {
    if reason.trim().is_empty() {
        return Err(TransitionError::Guard(english(
            "construction.variation.reason_needed",
            &[],
        )));
    }
    let _outcome = run_user(
        state,
        user,
        id,
        REJECT,
        &VariationInput::default(),
        Some(reason.trim().to_owned()),
    )
    .await?;
    Ok(Variation::find_or_404(&state.db, id).await?)
}

/// `user` (the project manager) resubmits the `rejected` variation `id`: the
/// new revision, in `draft`, is returned; the rejected one stays as it was,
/// "resubmitted as rev n+1".
///
/// # Errors
///
/// `Forbidden` without `variations.submit` in the project, `NotFrom` unless it
/// is `rejected`, `Guard` (422) when a revision replaced it already.
pub async fn resubmit(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Variation, TransitionError> {
    let old = Variation::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let _outcome = run_user(state, user, id, RESUBMIT, &VariationInput::default(), None).await?;
    Variation::query()
        .where_(Variation::PROJECT_ID.eq(old.project_id))
        .where_(Variation::CODE.eq(old.code.clone()))
        .where_(Variation::REVISION.eq(old.revision + 1))
        .first(&state.db)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// `user` (the project manager) turns the `rejected` variation `id` into a
/// claim for the final account.
///
/// # Errors
///
/// `Forbidden` without `variations.submit` in the project, `NotFrom` unless it
/// is `rejected`, `Guard` (422) unless the work was executed on a site
/// instruction or when a revision replaced it.
pub async fn claim(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Variation, TransitionError> {
    let _outcome = run_user(state, user, id, CLAIM, &VariationInput::default(), None).await?;
    Ok(Variation::find_or_404(&state.db, id).await?)
}

/// `user` (the QS) settles the `claimed` variation `id` at the agreed amount,
/// as `approve` does.
///
/// # Errors
///
/// As [`approve`], from `claimed`.
pub async fn settle_claim(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    decision: &Decision,
) -> std::result::Result<Variation, TransitionError> {
    let _outcome = run_user(state, user, id, SETTLE_CLAIM, &decision.input(), None).await?;
    Ok(Variation::find_or_404(&state.db, id).await?)
}

/// What an abandon came to.
#[derive(Debug)]
pub enum Abandoning {
    /// The variation is `abandoned`.
    Done(Variation),
    /// An approval request went to the MD and the variation is as it was.
    AwaitingMd(Variation),
}

/// `user` (the project manager) abandons the `rejected` or `claimed`
/// variation `id`: the MD approves first (`CON.ACL.16`, any amount).
///
/// # Errors
///
/// `Forbidden` without `variations.submit` in the project, `NotFrom` unless it
/// is `rejected` or `claimed`, `Guard` (422) when a revision replaced it.
pub async fn abandon(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Abandoning, TransitionError> {
    let outcome = run_user(state, user, id, ABANDON, &VariationInput::default(), None).await?;
    wl_kernel::approvals::announce(state, &outcome)
        .await
        .map_err(TransitionError::Failed)?;
    let now = Variation::find_or_404(&state.db, id).await?;
    Ok(if outcome.is_pending() {
        Abandoning::AwaitingMd(now)
    } else {
        Abandoning::Done(now)
    })
}

/// `user` (the project manager) withdraws the `draft` or `submitted`
/// variation `id`.
///
/// # Errors
///
/// `Forbidden` without `variations.submit` in the project, `NotFrom` unless it
/// is a `draft` or `submitted`.
pub async fn withdraw(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Variation, TransitionError> {
    let _outcome = run_user(state, user, id, WITHDRAW, &VariationInput::default(), None).await?;
    Ok(Variation::find_or_404(&state.db, id).await?)
}

/// Books `amount` fils of cost to the cost code of variation `id` through the
/// kernel's cost contract, under `key`. The cost is counted on the variation;
/// once the work was executed and the variation is `rejected`, `claimed` or
/// `abandoned` it counts as unrecovered too. Call it once per cost line: the
/// cost poster ignores a repeated key, the counters do not.
///
/// # Errors
///
/// A database error and the cost poster's errors.
pub async fn book_cost(state: &AppState, id: i64, amount: i64, key: &str) -> Result {
    let variation = Variation::find_or_404(&state.db, id).await?;
    let project = Project::find_or_404(&state.db, variation.project_id).await?;
    let request = PostCost::new(
        variation.subject(),
        format!("{SUBJECT_TYPE}:{key}"),
        project.subject(),
        amount,
    )
    .cost_code(&variation.cost_code);
    post_cost(state, request).await?;
    let booked = variation.cost_booked + amount;
    let counts = variation.executed_before_approval
        && matches!(
            variation.status,
            VariationStatus::Rejected | VariationStatus::Claimed | VariationStatus::Abandoned
        );
    let unrecovered = if counts {
        variation.unrecovered_amount + amount
    } else {
        variation.unrecovered_amount
    };
    Variation::query()
        .where_(Variation::ID.eq(id))
        .update(
            &state.db,
            &[
                ("cost_booked", &booked),
                ("unrecovered_amount", &unrecovered),
            ],
        )
        .await?;
    Ok(())
}

/// The revisions of variation `code` of `project_id`, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn revisions_of(db: &Db, project_id: i64, code: &str) -> Result<Vec<Variation>> {
    Variation::query()
        .where_(Variation::PROJECT_ID.eq(project_id))
        .where_(Variation::CODE.eq(code.to_owned()))
        .order_by(Variation::REVISION)
        .get(db)
        .await
}

/// The variations of `project_id`, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn of_project(db: &Db, project_id: i64) -> Result<Vec<Variation>> {
    Variation::query()
        .where_(Variation::PROJECT_ID.eq(project_id))
        .order_by(Variation::ID)
        .get(db)
        .await
}

/// A line of the variation report: what was priced, what the client approved
/// and the gap between them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportLine {
    /// The variation's id.
    pub id: i64,
    /// `VO-07`.
    pub code: String,
    /// The revision.
    pub revision: i64,
    /// `addition`, `omission` or `time`.
    pub kind: &'static str,
    /// The state.
    pub status: &'static str,
    /// The price asked, negative for an omission.
    pub priced_amount: i64,
    /// The amount the client approved, negative for an omission.
    pub approved_amount: Option<i64>,
    /// What the client approved less than was priced, in fils (the size of
    /// the price less the size of the approved amount); `None` until it is
    /// approved, and for a time variation.
    pub gap: Option<i64>,
    /// The days asked.
    pub days_asked: Option<i64>,
    /// The days granted.
    pub days_granted: Option<i64>,
    /// The days asked that were not granted.
    pub days_gap: Option<i64>,
    /// The quantity of an approved omission that progress had overtaken.
    pub excess_quantity: i64,
    /// The cost nothing recovers.
    pub unrecovered_amount: i64,
    /// Whether the notice came after the contract's notice deadline.
    pub notice_late: bool,
}

impl ReportLine {
    /// The line of `variation`.
    pub fn of(variation: &Variation) -> Self {
        let money = variation.kind != VariationKind::Time;
        ReportLine {
            id: variation.id,
            code: variation.code.clone(),
            revision: variation.revision,
            kind: variation.kind.as_str(),
            status: variation.status.as_str(),
            priced_amount: variation.priced_amount,
            approved_amount: variation.approved_amount,
            gap: variation
                .approved_amount
                .filter(|_| money)
                .map(|approved| variation.priced_amount.abs() - approved.abs()),
            days_asked: variation.days_asked,
            days_granted: variation.days_granted,
            days_gap: match (variation.days_asked, variation.days_granted) {
                (Some(asked), Some(granted)) if !money => Some(asked - granted),
                _ => None,
            },
            excess_quantity: variation.excess_quantity,
            unrecovered_amount: variation.unrecovered_amount,
            notice_late: variation.notice_late,
        }
    }
}

/// The variation report of `project_id`: every revision with its price, its
/// approved amount and the gap, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn report_of(db: &Db, project_id: i64) -> Result<Vec<ReportLine>> {
    Ok(of_project(db, project_id)
        .await?
        .iter()
        .map(ReportLine::of)
        .collect())
}

/// The claims of the final account of `project_id`: the `claimed` variations,
/// oldest first.
///
/// # Errors
///
/// A database error.
pub async fn claims_of(db: &Db, project_id: i64) -> Result<Vec<Variation>> {
    Variation::query()
        .where_(Variation::PROJECT_ID.eq(project_id))
        .where_(Variation::STATUS.eq(VariationStatus::Claimed))
        .order_by(Variation::ID)
        .get(db)
        .await
}

/// The unrecovered variations of `project_id` (the report): the rejected
/// revisions no later revision replaced, the claimed and the abandoned
/// variations whose cost nothing recovers, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn unrecovered_of(db: &Db, project_id: i64) -> Result<Vec<Variation>> {
    let found = Variation::query()
        .where_(Variation::PROJECT_ID.eq(project_id))
        .where_(Variation::STATUS.is_in([
            VariationStatus::Rejected,
            VariationStatus::Claimed,
            VariationStatus::Abandoned,
        ]))
        .order_by(Variation::ID)
        .get(db)
        .await?;
    Ok(found
        .into_iter()
        .filter(|variation| !variation.is_resubmitted() && variation.unrecovered_amount > 0)
        .collect())
}

/// What the approved variations of `project_id` add to the next IPA, in
/// fils: the approved amounts (an omission is negative) no IPA has valued
/// yet. A settled claim of a terminated project is in the final account IPA.
///
/// # Errors
///
/// A database error.
pub async fn next_ipa_amount(db: &Db, project_id: i64) -> Result<i64> {
    let approved = Variation::query()
        .where_(Variation::PROJECT_ID.eq(project_id))
        .where_(Variation::STATUS.eq(VariationStatus::Approved))
        .where_(Variation::IPA_ID.is_null())
        .get(db)
        .await?;
    Ok(approved
        .iter()
        .filter(|variation| variation.kind != VariationKind::Time)
        .filter_map(|variation| variation.approved_amount)
        .sum())
}

/// Tells the QS and the project manager that variation `id` was rejected.
async fn tell_rejected(state: &AppState, id: i64) -> Result {
    let variation = Variation::find_or_404(&state.db, id).await?;
    let project = Project::find_or_404(&state.db, variation.project_id).await?;
    let label = variation.label();
    let reason = variation.rejection_reason.clone().unwrap_or_default();
    let unrecovered = aed(variation.unrecovered_amount);
    let title = english(
        "construction.variation.rejected_title",
        &[("label", &label), ("project", &project.name)],
    );
    let body = english(
        "construction.variation.rejected_body",
        &[("reason", &reason), ("unrecovered", &unrecovered)],
    );
    let mut users = holders(state, &project, permissions::QS_ROLE).await?;
    users.extend(managers(state, &project).await?);
    users.sort_unstable();
    users.dedup();
    tell(
        state,
        variation.subject(),
        &project.code,
        &users,
        &title,
        &body,
    )
    .await?;
    Ok(())
}

/// Tells the QS and the project manager when the client rejects a variation.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(
        |event: Transitioned<Variation>, state: AppState| async move {
            if event.transition == REJECT && event.to == VariationStatus::Rejected.as_str() {
                tell_rejected(&state, event.key).await?;
            }
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variation(kind: VariationKind) -> Variation {
        Variation {
            id: 7,
            project_id: 1,
            code: "VO-07".into(),
            revision: 1,
            kind,
            ..Variation::default()
        }
    }

    fn decided(reference: &str, amount: Option<i64>, days: Option<i64>) -> VariationInput {
        VariationInput {
            client_reference: reference.into(),
            approved_amount: amount,
            days_granted: days,
            not_yet_done: None,
        }
    }

    #[test]
    fn a_variation_starts_a_draft_and_three_states_are_terminal() {
        let machine = Variation::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        let terminal: Vec<_> = machine
            .states()
            .iter()
            .filter(|state| state.terminal)
            .map(|state| state.name.as_str())
            .collect();
        assert_eq!(terminal, ["approved", "abandoned", "withdrawn"]);
        assert!(machine.states().iter().all(|state| state.tone.is_some()));
        // `rejected` is no longer an end.
        let rejected = machine
            .states()
            .iter()
            .find(|state| state.name == "rejected")
            .expect("state");
        assert!(!rejected.terminal);
        assert_eq!(rejected.tone.as_deref(), Some("error"));
    }

    #[test]
    fn every_transition_of_the_spec_is_declared() {
        let machine = Variation::machine();
        let table: [(&str, &[&str], &str, bool); 9] = [
            (SUBMIT, &["draft"], "submitted", false),
            (APPROVE, &["submitted"], "approved", false),
            (REJECT, &["submitted"], "rejected", false),
            // The rejected revision stays rejected; the new revision is made
            // by the effect.
            (RESUBMIT, &["rejected"], "rejected", false),
            (CLAIM, &["rejected"], "claimed", false),
            (SETTLE_CLAIM, &["claimed"], "approved", false),
            (ABANDON, &["rejected", "claimed"], "abandoned", false),
            (
                ABANDON_APPROVED,
                &["rejected", "claimed"],
                "abandoned",
                true,
            ),
            (WITHDRAW, &["draft", "submitted"], "withdrawn", false),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to, by_system) in &table {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), *from, "{name}");
            assert_eq!(transition.to_state(), Some(*to), "{name}");
            assert_eq!(transition.is_system(), *by_system, "{name}");
        }
        assert!(machine.find(REJECT).expect("reject").needs_reason());
    }

    #[test]
    fn abandon_and_its_approved_twin_share_the_guard_and_ask_the_md_for_any_amount() {
        let machine = Variation::machine();
        for name in [ABANDON, ABANDON_APPROVED] {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.guard_names(), ["not_replaced"], "{name}");
        }
        assert_eq!(wl_kernel::machines::approved(ABANDON), ABANDON_APPROVED);
        let expected = PolicyDefault::new(ABANDON_ACTION).step_or_permission(
            [wl_kernel::COMPANY_ADMIN_ROLE],
            permissions::PROJECTS_CANCEL,
        );
        assert_eq!(abandon_policy(), expected);
    }

    #[test]
    fn a_variation_is_priced_before_it_is_submitted() {
        let mut addition = variation(VariationKind::Addition);
        assert!(price_problem(&addition).is_some_and(|p| p.to_lowercase().contains("price")));
        addition.priced_amount = 5_000_000;
        assert_eq!(price_problem(&addition), None);

        let mut time = variation(VariationKind::Time);
        assert!(price_problem(&time).is_some_and(|p| p.contains("days")));
        time.days_asked = Some(30);
        assert!(price_problem(&time).is_some_and(|p| p.contains("event")));
        time.event = Some("late drawings".into());
        time.event_date = NaiveDate::from_ymd_opt(2027, 4, 1);
        assert_eq!(price_problem(&time), None);
    }

    #[test]
    fn the_clients_decision_needs_its_reference_and_the_amount_or_the_days() {
        let addition = variation(VariationKind::Addition);
        assert_eq!(
            decision_problem(&addition, &decided("CL-1", Some(4_200_000), None)),
            None
        );
        assert!(
            decision_problem(&addition, &decided(" ", Some(4_200_000), None))
                .is_some_and(|p| p.contains("reference"))
        );
        assert!(decision_problem(&addition, &decided("CL-1", None, None)).is_some());
        assert!(decision_problem(&addition, &decided("CL-1", Some(-5), None)).is_some());

        let omission = variation(VariationKind::Omission);
        assert_eq!(
            decision_problem(&omission, &decided("CL-1", Some(-3_600_000), None)),
            None
        );
        assert!(decision_problem(&omission, &decided("CL-1", Some(0), None)).is_some());

        let time = variation(VariationKind::Time);
        assert_eq!(
            decision_problem(&time, &decided("CL-1", None, Some(0))),
            None
        );
        assert_eq!(
            decision_problem(&time, &decided("CL-1", None, Some(20))),
            None
        );
        assert!(decision_problem(&time, &decided("CL-1", None, None)).is_some());
        assert!(decision_problem(&time, &decided("CL-1", None, Some(-1))).is_some());
    }

    #[test]
    fn an_omission_is_stored_negative() {
        assert_eq!(signed(VariationKind::Omission, 3_600_000), -3_600_000);
        assert_eq!(signed(VariationKind::Omission, -3_600_000), -3_600_000);
        assert_eq!(signed(VariationKind::Addition, 3_600_000), 3_600_000);
    }

    #[test]
    fn only_work_executed_on_a_site_instruction_is_claimed() {
        let mut rejected = variation(VariationKind::Addition);
        assert!(claim_problem(&rejected).is_some_and(|p| p.contains("executed")));
        rejected.executed_before_approval = true;
        assert!(claim_problem(&rejected).is_some());
        rejected.site_instruction = Some("SI-22".into());
        assert_eq!(claim_problem(&rejected), None);
        assert!(rejected.executed_on_instruction());
    }

    #[test]
    fn a_replaced_revision_can_no_longer_be_claimed_or_abandoned() {
        let mut rejected = variation(VariationKind::Addition);
        rejected.executed_before_approval = true;
        rejected.site_instruction = Some("SI-22".into());
        assert_eq!(replaced_problem(&rejected), None);
        rejected.resubmitted_as = Some(2);
        assert!(replaced_problem(&rejected).is_some_and(|p| p.contains("rev 2")));
        assert!(claim_problem(&rejected).is_some_and(|p| p.contains("VO-07 rev 1")));
        assert_eq!(
            rejected.resubmitted_text().as_deref(),
            Some("resubmitted as rev 2")
        );
    }

    #[test]
    fn a_variation_is_named_with_its_revision() {
        let mut first = variation(VariationKind::Addition);
        assert_eq!(first.label(), "VO-07 rev 1");
        first.revision = 2;
        assert_eq!(first.label(), "VO-07 rev 2");
        assert_eq!(first.subject().to_string(), "construction.variation:7");
    }

    #[test]
    fn a_new_variation_says_what_it_is() {
        let mut new = NewVariation {
            description: " Extra partition ".into(),
            priced_amount: 100,
            ..NewVariation::default()
        };
        assert_eq!(new_problem(&new), None);
        new.description = " ".into();
        assert!(new_problem(&new).is_some());
        new.description = "x".into();
        new.priced_amount = -1;
        assert!(new_problem(&new).is_some());
        new.kind = VariationKind::Omission;
        assert_eq!(new_problem(&new), None);
    }
}
