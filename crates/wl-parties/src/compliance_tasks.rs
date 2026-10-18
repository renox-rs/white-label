//! The freeze and report tasks of a confirmed sanctions match (wl-parties §4,
//! `PTY.SCR.01`, issue #39). Not the same as the [`ScreeningTask`] of #38,
//! which is the review of a possible match.
//!
//! A confirmed match gives the compliance role two tasks per [`Screening`]:
//! a **freeze** task (hold the money, record the freeze time) and a **report**
//! task (file the report, record the reference and date). Both are due the
//! setting's hours after the flag ([`ConfirmedMatchSettings`], 24 by default),
//! not after the confirmation, so a late confirmation gives tasks that are
//! already overdue. Both name every party that was touched. The freeze task
//! lists the money the registered funds sources report
//! ([`wl_kernel::held_funds`]) as unconfirmed lines for the role to confirm or
//! edit. The tasks are for holders of `parties.block_reason.view` only.
//!
//! The status moves through the machine:
//!
//! ```text
//! [*] --> open --complete--> done
//! ```
//!
//! `complete` is the user's. It needs `screening.review` and
//! `parties.block_reason.view`. A freeze task needs the freeze time and every
//! named party `blocked` or `archived` with an open `sanctions` Block record; a
//! report task needs the reference and the date reported.
//! [`create_for_screening`] is idempotent per screening and kind.

use renox::chrono::Duration;
use renox::db::{DateTime, Executor, Transaction};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::held_funds::{self, HeldFund};
use wl_kernel::{PartyId, PartyStatus};

use crate::block_record::BlockRecord;
use crate::party::Party;
use crate::permissions;
use crate::screening::Screening;
use crate::screening_task::ScreeningTask;
use crate::settings::ConfirmedMatchSettings;
use crate::texts;

/// The user transition that finishes a task.
pub const COMPLETE: &str = "complete";

/// The restricted reason code a confirmed match blocks with.
pub const SANCTIONS: &str = "sanctions";

/// What a task asks the compliance role to do.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ComplianceTaskKind {
    /// Freeze the money held for the named parties.
    #[default]
    Freeze,
    /// Report the match.
    Report,
}

/// Where a task stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ComplianceTaskStatus {
    /// Waiting for the compliance role.
    #[default]
    Open,
    /// Done.
    Done,
}

/// How a named party stood when the match was confirmed.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PartyStanding {
    /// Blocked by the confirmation.
    #[default]
    Blocked,
    /// Archived; it only got the restricted Block record.
    Archived,
    /// Already blocked for another reason; it only got the record.
    BlockedAlready,
}

/// A freeze or report task of one screening.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_parties_compliance_tasks",
    state_machine,
    index(screening_id),
    index(status)
)]
pub struct ComplianceTask {
    /// The primary key.
    pub id: i64,
    /// Freeze or report.
    pub kind: ComplianceTaskKind,
    /// The confirmed screening.
    pub screening_id: i64,
    /// Where it stands; changed only by a transition.
    pub status: ComplianceTaskStatus,
    /// When it is due: the flag time plus the setting's hours.
    pub due_at: DateTime,
    /// The user who completed it.
    pub completed_by: Option<i64>,
    /// When it was completed.
    pub completed_at: Option<DateTime>,
    /// Freeze task: when the funds were frozen.
    pub frozen_at: Option<DateTime>,
    /// Report task: the filing reference.
    pub reference: Option<String>,
    /// Report task: the date reported.
    pub reported_on: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A party a task names.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_parties_compliance_task_parties",
    index(task_id),
    index(party_id)
)]
pub struct ComplianceTaskParty {
    /// The primary key.
    pub id: i64,
    /// The task.
    pub task_id: i64,
    /// The party.
    pub party_id: i64,
    /// The restricted Block record of the screening for this party.
    pub block_record_id: Option<i64>,
    /// How the party stood at the confirmation.
    pub standing: PartyStanding,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A line of held money on a freeze task.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_parties_compliance_task_funds", index(task_id))]
pub struct ComplianceTaskFund {
    /// The primary key.
    pub id: i64,
    /// The freeze task.
    pub task_id: i64,
    /// The party the money is held for; `None` for a line added by hand that
    /// belongs to none of the named parties.
    pub party_id: Option<i64>,
    /// A one-line label.
    pub label: String,
    /// The kind of money: `deposit`, `prepaid_credit`, `refund`, ...
    pub kind: String,
    /// The amount in fils.
    pub amount_fils: i64,
    /// The ISO 4217 currency code.
    pub currency: String,
    /// The funds source that reported it; empty for a line added by hand.
    pub source_key: String,
    /// Whether the compliance role has confirmed the line.
    pub confirmed: bool,
    /// A note by the compliance role.
    pub note: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A party to name on the tasks of a screening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamedParty {
    /// The party.
    pub party_id: i64,
    /// Its restricted Block record of the screening.
    pub block_record_id: Option<i64>,
    /// How it stood at the confirmation.
    pub standing: PartyStanding,
}

/// The two tasks of a screening.
#[derive(Debug, Clone)]
pub struct ComplianceTasks {
    /// The freeze task.
    pub freeze: ComplianceTask,
    /// The report task.
    pub report: ComplianceTask,
}

/// What a `complete` run carries to its guard and effect.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompleteInput {
    /// Freeze task: when the funds were frozen.
    pub frozen_at: Option<DateTime>,
    /// Report task: the filing reference.
    pub reference: Option<String>,
    /// Report task: the date reported.
    pub reported_on: Option<DateTime>,
}

impl StateMachine for ComplianceTask {
    type State = ComplianceTaskStatus;
    const COLUMN: renox::db::Col<ComplianceTask, ComplianceTaskStatus> = ComplianceTask::STATUS;

    fn machine() -> Machine<Self> {
        use ComplianceTaskStatus::{Done, Open};
        Machine::new()
            .initial([Open])
            .state(Open)
            .tone("warning")
            .state(Done)
            .tone("success")
            .terminal()
            .transition(
                Transition::new(COMPLETE)
                    .label("Complete")
                    .tone("success")
                    .from([Open])
                    .to(Done)
                    .permission(permissions::SCREENING_REVIEW)
                    .guard("compliance_role", |_task: &ComplianceTask, ctx| {
                        let allowed = ctx.actor().is_some_and(|user| {
                            user.has_permission(permissions::BLOCK_REASON_VIEW)
                        });
                        Box::pin(async move {
                            if allowed {
                                Ok(())
                            } else {
                                Err(texts::english("wl_parties.errors.compliance_only"))
                            }
                        })
                    })
                    .guard("details_given", |task: &ComplianceTask, ctx| {
                        let result = details_given(task.kind, ctx);
                        Box::pin(async move { result })
                    })
                    .effect(|task: &ComplianceTask, ctx, tx| {
                        let id = task.id;
                        let kind = task.kind;
                        Box::pin(async move {
                            if kind == ComplianceTaskKind::Freeze {
                                let open = unfrozen_parties(&mut *tx, id).await?;
                                if !open.is_empty() {
                                    return Err(renox::Error::permanent_message(texts::english(
                                        "wl_parties.errors.freeze_not_ready",
                                    )));
                                }
                            }
                            let input = ctx.input_as::<CompleteInput>().unwrap_or_default();
                            let by = ctx.actor().map(|user| user.id);
                            // Not the status column, so outside the machine's guard.
                            renox::db::sql(format!(
                                "UPDATE {} SET completed_by = ?, completed_at = ?, frozen_at = ?, \
                                 reference = ?, reported_on = ?, updated_at = ? WHERE id = ?",
                                ComplianceTask::TABLE
                            ))
                            .bind(by)
                            .bind(renox::db::now())
                            .bind(input.frozen_at)
                            .bind(input.reference.map(|r| r.trim().to_owned()))
                            .bind(input.reported_on)
                            .bind(renox::db::now())
                            .bind(id)
                            .execute(&mut *tx)
                            .await?;
                            Ok(())
                        })
                    }),
            )
    }
}

/// Whether the run carries what the task's kind needs.
fn details_given(kind: ComplianceTaskKind, ctx: &TransitionCtx) -> std::result::Result<(), String> {
    let input = ctx.input_as::<CompleteInput>().unwrap_or_default();
    match kind {
        ComplianceTaskKind::Freeze if input.frozen_at.is_none() => {
            Err(texts::english("wl_parties.errors.freeze_time_required"))
        }
        ComplianceTaskKind::Report
            if input
                .reference
                .as_deref()
                .is_none_or(|r| r.trim().is_empty())
                || input.reported_on.is_none() =>
        {
            Err(texts::english("wl_parties.errors.report_details_required"))
        }
        _ => Ok(()),
    }
}

/// The named parties of `task_id` that are not yet `blocked` or `archived`
/// with an open `sanctions` Block record.
async fn unfrozen_parties(tx: &mut Transaction, task_id: i64) -> Result<Vec<i64>> {
    let mut open = Vec::new();
    for named in ComplianceTaskParty::for_task(&mut *tx, task_id).await? {
        let party = Party::find(&mut *tx, named.party_id).await?;
        let held =
            party.is_some_and(|p| matches!(p.status, PartyStatus::Blocked | PartyStatus::Archived));
        let sanctioned = BlockRecord::open_for(&mut *tx, named.party_id)
            .await?
            .iter()
            .any(|r| r.reason_code == SANCTIONS);
        if !(held && sanctioned) {
            open.push(named.party_id);
        }
    }
    Ok(open)
}

impl ComplianceTask {
    /// The two tasks of a screening, if they exist.
    pub async fn for_screening<'c>(
        db: impl Executor<'c>,
        screening_id: i64,
    ) -> Result<Vec<ComplianceTask>> {
        ComplianceTask::query()
            .where_(ComplianceTask::SCREENING_ID.eq(screening_id))
            .order_by(ComplianceTask::ID)
            .get(db)
            .await
    }

    /// The open tasks, the earliest due first.
    pub async fn open_tasks<'c>(db: impl Executor<'c>) -> Result<Vec<ComplianceTask>> {
        ComplianceTask::query()
            .where_(ComplianceTask::STATUS.eq(ComplianceTaskStatus::Open))
            .order_by(ComplianceTask::DUE_AT)
            .order_by(ComplianceTask::ID)
            .get(db)
            .await
    }

    /// Whether the task is open and its due time has passed at `now`.
    pub fn is_overdue(&self, now: DateTime) -> bool {
        self.status == ComplianceTaskStatus::Open && self.due_at < now
    }

    /// The parties the task names, in the order they were added.
    pub async fn parties<'c>(&self, db: impl Executor<'c>) -> Result<Vec<ComplianceTaskParty>> {
        ComplianceTaskParty::for_task(db, self.id).await
    }

    /// The task's funds lines, in the order they were added.
    pub async fn funds<'c>(&self, db: impl Executor<'c>) -> Result<Vec<ComplianceTaskFund>> {
        ComplianceTaskFund::query()
            .where_(ComplianceTaskFund::TASK_ID.eq(self.id))
            .order_by(ComplianceTaskFund::ID)
            .get(db)
            .await
    }
}

impl ComplianceTaskParty {
    /// The parties a task names, in the order they were added.
    pub async fn for_task<'c>(
        db: impl Executor<'c>,
        task_id: i64,
    ) -> Result<Vec<ComplianceTaskParty>> {
        ComplianceTaskParty::query()
            .where_(ComplianceTaskParty::TASK_ID.eq(task_id))
            .order_by(ComplianceTaskParty::ID)
            .get(db)
            .await
    }
}

/// Creates the freeze and report tasks of the confirmed `screening`, in the
/// own transaction, naming `parties`. Each is due its setting's hours
/// after the flag (the first flag of the screening's review task, else the
/// run time), with no clamp: a late confirmation gives overdue tasks. The
/// freeze task's funds are pre-filled, unconfirmed, from every registered
/// funds source for every named party. The sources are asked before the
/// transaction opens, so a source that reads the database never waits for it.
///
/// Idempotent per screening and kind: a task that already exists is returned
/// as it is and nothing is added to it, so a re-delivered confirmation adds
/// nothing twice.
///
/// # Errors
///
/// A database error, or an error of a funds source (never swallowed: a
/// missing source would hide held money).
pub async fn create_for_screening(
    state: &AppState,
    screening: &Screening,
    parties: &[NamedParty],
) -> Result<ComplianceTasks> {
    // Ask the sources first, so a failing one leaves nothing half written.
    let mut funds = Vec::new();
    let has_freeze = ComplianceTask::query()
        .where_(ComplianceTask::SCREENING_ID.eq(screening.id))
        .where_(ComplianceTask::KIND.eq(ComplianceTaskKind::Freeze))
        .count(&state.db)
        .await?
        > 0;
    if !has_freeze {
        for named in parties {
            for fund in held_funds::held_for_all(state, PartyId(named.party_id)).await? {
                funds.push((named.party_id, fund));
            }
        }
    }
    let mut tx = state.db.begin_immediate().await?;
    let settings = ConfirmedMatchSettings::of(state);
    let flagged_at = match screening.task_id {
        Some(task_id) => ScreeningTask::find(&mut tx, task_id)
            .await?
            .map_or(screening.run_at, |task| task.flagged_at),
        None => screening.run_at,
    };
    let freeze = create_one(
        &mut tx,
        screening.id,
        ComplianceTaskKind::Freeze,
        flagged_at + Duration::hours(settings.freeze_task_due_hours),
        parties,
        funds,
    )
    .await?;
    let report = create_one(
        &mut tx,
        screening.id,
        ComplianceTaskKind::Report,
        flagged_at + Duration::hours(settings.report_task_due_hours),
        parties,
        Vec::new(),
    )
    .await?;
    tx.commit().await?;
    Ok(ComplianceTasks { freeze, report })
}

async fn create_one(
    tx: &mut Transaction,
    screening_id: i64,
    kind: ComplianceTaskKind,
    due_at: DateTime,
    parties: &[NamedParty],
    funds: Vec<(i64, HeldFund)>,
) -> Result<ComplianceTask> {
    let existing = ComplianceTask::query()
        .where_(ComplianceTask::SCREENING_ID.eq(screening_id))
        .where_(ComplianceTask::KIND.eq(kind))
        .first(&mut *tx)
        .await?;
    if let Some(task) = existing {
        return Ok(task);
    }
    let mut task = ComplianceTask {
        kind,
        screening_id,
        status: ComplianceTaskStatus::Open,
        due_at,
        ..ComplianceTask::default()
    };
    task.insert(&mut *tx).await?;
    for named in parties {
        let mut row = ComplianceTaskParty {
            task_id: task.id,
            party_id: named.party_id,
            block_record_id: named.block_record_id,
            standing: named.standing,
            ..ComplianceTaskParty::default()
        };
        row.insert(&mut *tx).await?;
    }
    for (party_id, fund) in funds {
        let mut row = ComplianceTaskFund {
            task_id: task.id,
            party_id: Some(party_id),
            label: fund.label,
            kind: fund.kind,
            amount_fils: fund.amount_fils,
            currency: fund.currency,
            source_key: fund.source,
            confirmed: false,
            ..ComplianceTaskFund::default()
        };
        row.insert(&mut *tx).await?;
    }
    Ok(task)
}

/// `user` completes the task `task_id` with `input`. Needs `screening.review`
/// and `parties.block_reason.view`. A freeze task also needs every named party
/// `blocked` or `archived` with an open `sanctions` record.
///
/// # Errors
///
/// `Forbidden` without the permissions, `NotFrom` when the task is done,
/// `Guard` when the details or the freeze are missing; nothing is written
/// then.
pub async fn complete(
    state: &AppState,
    user: &AuthUser,
    task_id: i64,
    input: &CompleteInput,
) -> std::result::Result<(), TransitionError> {
    if !user.has_permission(permissions::SCREENING_REVIEW)
        || !user.has_permission(permissions::BLOCK_REASON_VIEW)
    {
        return Err(TransitionError::Forbidden);
    }
    let mut tx = state.db.begin_immediate().await?;
    let task = ComplianceTask::find(&mut tx, task_id)
        .await?
        .ok_or(renox::Error::NotFound)?;
    if task.kind == ComplianceTaskKind::Freeze
        && task.status == ComplianceTaskStatus::Open
        && !unfrozen_parties(&mut tx, task.id).await?.is_empty()
    {
        return Err(TransitionError::Guard(texts::english(
            "wl_parties.errors.freeze_not_ready",
        )));
    }
    let outcome = task
        .transition(COMPLETE)
        .by(user)
        .input(input)
        .run_in(&mut tx)
        .await?;
    tx.commit().await?;
    outcome.emit(state).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_task_is_an_open_freeze_task() {
        let task = ComplianceTask::default();
        assert_eq!(task.kind, ComplianceTaskKind::Freeze);
        assert_eq!(task.status, ComplianceTaskStatus::Open);
    }

    #[test]
    fn the_machine_has_one_user_transition_that_completes() {
        let machine = ComplianceTask::machine();
        let complete = machine.find(COMPLETE).expect("complete");
        assert!(!complete.is_system());
        assert_eq!(
            complete.guard_names(),
            vec!["compliance_role", "details_given"]
        );
    }

    #[test]
    fn a_task_is_overdue_only_while_open_and_past_due() {
        let now = renox::db::now();
        let mut task = ComplianceTask {
            due_at: now - Duration::hours(1),
            ..ComplianceTask::default()
        };
        assert!(task.is_overdue(now));
        task.due_at = now + Duration::hours(1);
        assert!(!task.is_overdue(now));
        task.due_at = now - Duration::hours(1);
        task.status = ComplianceTaskStatus::Done;
        assert!(!task.is_overdue(now));
    }

    #[test]
    fn the_details_depend_on_the_kind() {
        let ctx = |input: &CompleteInput| {
            TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).unwrap())
        };
        let now = renox::db::now();
        let none = CompleteInput::default();
        assert!(details_given(ComplianceTaskKind::Freeze, &ctx(&none)).is_err());
        assert!(details_given(ComplianceTaskKind::Report, &ctx(&none)).is_err());
        let frozen = CompleteInput {
            frozen_at: Some(now),
            ..CompleteInput::default()
        };
        assert!(details_given(ComplianceTaskKind::Freeze, &ctx(&frozen)).is_ok());
        let blank = CompleteInput {
            reference: Some("  ".into()),
            reported_on: Some(now),
            ..CompleteInput::default()
        };
        assert!(details_given(ComplianceTaskKind::Report, &ctx(&blank)).is_err());
        let reported = CompleteInput {
            reference: Some("GOAML-1".into()),
            reported_on: Some(now),
            ..CompleteInput::default()
        };
        assert!(details_given(ComplianceTaskKind::Report, &ctx(&reported)).is_ok());
    }
}
