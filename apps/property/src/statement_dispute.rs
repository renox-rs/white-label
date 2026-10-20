//! Disputing a line of an owner statement and correcting it (01 Property §4.4
//! rows "The owner disputes a statement line" and "A cost was allocated to the
//! wrong owner", §7.1 *Owner statement* "Dispute flag", §3.3 `PRP.ACL.21`;
//! BL-PRP-21).
//!
//! The owner disputes a line of a `published` statement, or of a `settled`
//! one after the payout. In the MVP staff record it on the owner's behalf
//! ([`open`]; the owner portal is Later, BL-PRP-29). The statement gets the
//! dispute flag ([`OwnerStatement::disputed`], not a state) and a ticket of the
//! category *owner statement dispute*; a payout waits while the flag is set
//! (`PRP.ACL.21`, [`crate::statement_payout`]).
//!
//! The flag is cleared dispute by dispute, and the owner is told the outcome by
//! e-mail:
//!
//! - **`revise`** (the machine, [`crate::statement_machine::REVISE`]): the
//!   statement is superseded and the corrected revision follows; every open
//!   dispute of the old one is cleared as *revised* in the same transaction.
//!   Only a published statement is revised.
//! - **Upheld** ([`Clearing::Upheld`]): the statement stands. This is the way
//!   to clear a dispute on a settled statement, besides an adjustment.
//! - **Adjustment line** ([`Clearing::Adjustment`]): a pending line of kind
//!   *adjustment*, linked to the disputed line, goes on the agreement's next
//!   draft statement ([`crate::statement::post_line`]); a published statement
//!   is never edited.
//! - **Cost allocated to the wrong owner** ([`Clearing::Reallocation`]): two
//!   adjustment lines, one on this owner's next statement and the opposite on
//!   the other owner's, linked to each other.
//!
//! The sign of an adjustment is what the staff member enters: the effect on the
//! owner's balance. For a cost charged to the wrong owner, credit the owner who
//! paid it and charge the other.

use renox::auth::permissions::Scope;
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::role_tasks::RoleTask;
use wl_kernel::tickets::{TicketId, task_creator};

use crate::agreement::ManagementAgreement;
use crate::building::{clean, refused};
use crate::money_text;
use crate::notices;
use crate::permissions::PROPERTY_MANAGER_ROLE;
use crate::statement::{
    LineKind, OwnerStatement, PostLine, StatementLine, StatementStatus, may_prepare,
};
use crate::texts::english;

/// The ticket category of an owner statement dispute.
pub const DISPUTE_CATEGORY: &str = "owner_statement_dispute";

/// The key the ticket of the dispute `id` is found by.
pub fn ticket_key(id: i64) -> String {
    format!("property.statement_dispute:{id}")
}

/// Where a dispute stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DisputeStatus {
    /// The flag is set.
    #[default]
    Open,
    /// Cleared with an outcome.
    Cleared,
}

/// How a dispute was cleared.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DisputeOutcome {
    /// The statement stands.
    #[default]
    Upheld,
    /// An adjustment line goes on the next statement.
    Adjusted,
    /// Adjustment lines go on the next statements of both owners.
    Reallocated,
    /// The statement was revised.
    Revised,
}

/// A dispute of a line (or of the statement) by its owner.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "property_statement_disputes", index(statement_id, status))]
pub struct StatementDispute {
    /// The primary key.
    pub id: i64,
    /// The statement.
    #[model(references = "property_owner_statements")]
    pub statement_id: i64,
    /// The line disputed.
    pub line_id: Option<i64>,
    /// What is disputed, with its amount.
    pub description: String,
    /// The owner's words.
    pub comment: String,
    /// Open or cleared.
    pub status: DisputeStatus,
    /// How it was cleared.
    pub outcome: Option<DisputeOutcome>,
    /// The adjustment on this owner's balance, in fils.
    pub adjustment: Option<i64>,
    /// The other agreement of a cost allocated to the wrong owner.
    pub other_agreement_id: Option<i64>,
    /// A note on the outcome.
    pub note: Option<String>,
    /// The linked ticket.
    pub ticket_id: Option<i64>,
    /// The number of the ticket.
    pub ticket_number: Option<String>,
    /// The user who recorded it.
    pub opened_by: Option<i64>,
    /// The day it was recorded.
    pub opened_on: NaiveDate,
    /// The user who cleared it.
    pub cleared_by: Option<i64>,
    /// The day it was cleared.
    pub cleared_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl StatementDispute {
    /// The disputes of the statement `statement_id`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_statement(db: &Db, statement_id: i64) -> Result<Vec<StatementDispute>> {
        StatementDispute::query()
            .where_(StatementDispute::STATEMENT_ID.eq(statement_id))
            .order_by(StatementDispute::ID)
            .get(db)
            .await
    }

    /// Whether the dispute is open.
    pub fn is_open(&self) -> bool {
        self.status == DisputeStatus::Open
    }
}

/// How staff clear a dispute (a revision clears it through the machine).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clearing {
    /// The statement stands.
    Upheld,
    /// An adjustment line of `amount` fils (its effect on the owner's balance,
    /// not 0) goes on the next statement.
    Adjustment {
        /// The signed amount.
        amount: i64,
    },
    /// A cost allocated to the wrong owner: `amount` fils (not 0) on this
    /// owner's next statement and the opposite on the one of the other
    /// agreement.
    Reallocation {
        /// The signed amount for this owner.
        amount: i64,
        /// The agreement of the other owner.
        other_agreement: i64,
    },
}

/// Whether a statement in `status` can be disputed: published, or settled
/// after its payout.
pub fn disputable(status: StatementStatus) -> bool {
    matches!(
        status,
        StatementStatus::Published | StatementStatus::Settled
    )
}

/// Sets the flag of the statement `id` from its open disputes.
async fn sync_flag(db: &Db, id: i64) -> Result {
    let open = StatementDispute::query()
        .where_(StatementDispute::STATEMENT_ID.eq(id))
        .where_(StatementDispute::STATUS.eq(DisputeStatus::Open))
        .exists(db)
        .await?;
    // Not the status column, so outside the machine's guard.
    OwnerStatement::query()
        .where_(OwnerStatement::ID.eq(id))
        .update(
            db,
            &[("disputed", &open), ("updated_at", &renox::db::now())],
        )
        .await?;
    Ok(())
}

/// `user` records the owner's dispute of the line `line_id` (or of the
/// statement, with no line) of the published or settled statement `id`, with
/// the owner's `comment`: the flag is set and a ticket is opened.
///
/// # Errors
///
/// 403 without `owner_statements.prepare`; 404 for a line that is not on the
/// statement; 422 unless the statement is published or settled, or for a blank
/// comment. Nothing is recorded then, and no dispute exists without its ticket.
pub async fn open(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    line_id: Option<i64>,
    comment: &str,
) -> Result<StatementDispute> {
    if !may_prepare(user) {
        return Err(Error::Forbidden);
    }
    let statement = OwnerStatement::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    if !disputable(statement.status) {
        return Err(refused(english("property.disputes.not_disputable", &[])));
    }
    let comment = comment.trim();
    if comment.is_empty() {
        return Err(refused(english("property.disputes.comment_required", &[])));
    }
    let description = match line_id {
        Some(line_id) => {
            let line = StatementLine::find(&state.db, line_id)
                .await?
                .filter(|line| line.statement_id == Some(statement.id))
                .ok_or(Error::NotFound)?;
            format!("{} {}", line.description, money_text(line.amount.abs()))
        }
        None => statement.number(),
    };
    let mut dispute = StatementDispute::create(
        &state.db,
        StatementDispute {
            statement_id: statement.id,
            line_id,
            description,
            comment: comment.to_owned(),
            status: DisputeStatus::Open,
            opened_by: Some(user.id),
            opened_on: today(),
            ..Default::default()
        },
    )
    .await?;
    let title = english(
        "property.disputes.ticket_title",
        &[("statement", &statement.number())],
    );
    let body = format!("{}: {}", dispute.description, dispute.comment);
    let task = RoleTask::new(
        statement.subject(),
        PROPERTY_MANAGER_ROLE,
        &title,
        &ticket_key(dispute.id),
    )
    .description(&body);
    match crate::tasks::open_in(state, &Scope::global(), task, DISPUTE_CATEGORY).await {
        Ok(Some(ticket)) => {
            dispute.ticket_id = Some(ticket.id.0);
            dispute.ticket_number = Some(ticket.number);
            dispute.save(&state.db).await?;
        }
        // Ticketing is off: the dispute has no ticket.
        Ok(None) => {}
        // Never a flag without its ticket.
        Err(error) => {
            dispute.delete(&state.db).await?;
            return Err(error);
        }
    }
    sync_flag(&state.db, statement.id).await?;
    StatementDispute::find(&state.db, dispute.id)
        .await?
        .ok_or(Error::NotFound)
}

/// Resolves the ticket of `dispute`, if it has one and it is open.
async fn resolve_ticket(state: &AppState, dispute: &StatementDispute, outcome: &str) -> Result {
    if let Some(ticket) = dispute.ticket_id {
        let resolution = english(
            "property.disputes.ticket_resolved",
            &[("outcome", &outcome)],
        );
        task_creator(state)
            .resolve_from_source(state, TicketId(ticket), &resolution)
            .await?;
    }
    Ok(())
}

/// What the owner is told about an outcome.
fn outcome_text(outcome: DisputeOutcome) -> String {
    english(
        &format!("property.disputes.outcome_{}", outcome.as_str()),
        &[],
    )
}

/// `user` clears the open dispute `dispute_id` with `clearing`: the lines of an
/// adjustment are posted for the next statements, the ticket is resolved, the
/// flag is released when no other dispute of the statement is open, and the
/// owner is told the outcome by e-mail.
///
/// # Errors
///
/// 403 without `owner_statements.prepare`; 404 for an unknown dispute; 422 for
/// a dispute that is cleared already, an adjustment of 0, or another agreement
/// that does not report or is the statement's own.
pub async fn clear(
    state: &AppState,
    user: &AuthUser,
    dispute_id: i64,
    clearing: Clearing,
    note: Option<String>,
) -> Result<StatementDispute> {
    if !may_prepare(user) {
        return Err(Error::Forbidden);
    }
    let mut dispute = StatementDispute::find(&state.db, dispute_id)
        .await?
        .ok_or(Error::NotFound)?;
    if !dispute.is_open() {
        return Err(refused(english("property.disputes.cleared_already", &[])));
    }
    let statement = OwnerStatement::find(&state.db, dispute.statement_id)
        .await?
        .ok_or(Error::NotFound)?;
    let key = ticket_key(dispute.id);
    let reference = english(
        "property.disputes.adjustment_line",
        &[
            ("statement", &statement.number()),
            ("what", &dispute.description),
        ],
    );
    let (outcome, adjustment, other) = match &clearing {
        Clearing::Upheld => (DisputeOutcome::Upheld, None, None),
        Clearing::Adjustment { amount } => {
            if *amount == 0 {
                return Err(refused(english("property.disputes.amount_required", &[])));
            }
            (DisputeOutcome::Adjusted, Some(*amount), None)
        }
        Clearing::Reallocation {
            amount,
            other_agreement,
        } => {
            if *amount == 0 {
                return Err(refused(english("property.disputes.amount_required", &[])));
            }
            let found = ManagementAgreement::find(&state.db, *other_agreement)
                .await?
                .filter(|a| {
                    a.id != statement.agreement_id
                        && crate::agreement::AgreementStatus::REPORTING.contains(&a.status)
                });
            if found.is_none() {
                return Err(refused(english(
                    "property.disputes.other_agreement_invalid",
                    &[],
                )));
            }
            (
                DisputeOutcome::Reallocated,
                Some(*amount),
                Some(*other_agreement),
            )
        }
    };
    match (&clearing, adjustment, other) {
        (Clearing::Adjustment { .. }, Some(amount), _) => {
            let line = PostLine::new(LineKind::Adjustment, &reference, amount, today())
                .source(&format!("{key}:adjustment"))
                .linked_to(dispute.line_id)
                .by(Some(user.id));
            crate::statement::post_line(&state.db, statement.agreement_id, line).await?;
        }
        (Clearing::Reallocation { .. }, Some(amount), Some(other_agreement)) => {
            let here = PostLine::new(LineKind::Adjustment, &reference, amount, today())
                .source(&format!("{key}:self"))
                .by(Some(user.id));
            let mut mine =
                crate::statement::post_line(&state.db, statement.agreement_id, here).await?;
            let there = PostLine::new(LineKind::Adjustment, &reference, -amount, today())
                .source(&format!("{key}:other"))
                .linked_to(Some(mine.id))
                .by(Some(user.id));
            let theirs = crate::statement::post_line(&state.db, other_agreement, there).await?;
            mine.linked_to = Some(theirs.id);
            mine.save(&state.db).await?;
        }
        _ => {}
    }
    dispute.status = DisputeStatus::Cleared;
    dispute.outcome = Some(outcome);
    dispute.adjustment = adjustment;
    dispute.other_agreement_id = other;
    dispute.note = clean(note);
    dispute.cleared_by = Some(user.id);
    dispute.cleared_on = Some(today());
    dispute.save(&state.db).await?;
    sync_flag(&state.db, statement.id).await?;
    resolve_ticket(state, &dispute, outcome.as_str()).await?;
    notices::dispute_outcome(state, &statement, &outcome_text(outcome)).await?;
    Ok(dispute)
}

/// The statement `statement` was revised: its open disputes were cleared as
/// *revised* in the transaction; the tickets are resolved and the owner is
/// told, once per dispute.
///
/// # Errors
///
/// The database's, ticketing's and the notifier's errors.
pub async fn after_revise(state: &AppState, statement: &OwnerStatement) -> Result {
    let revised: Vec<StatementDispute> = StatementDispute::of_statement(&state.db, statement.id)
        .await?
        .into_iter()
        .filter(|d| d.outcome == Some(DisputeOutcome::Revised))
        .collect();
    for dispute in &revised {
        resolve_ticket(state, dispute, DisputeOutcome::Revised.as_str()).await?;
    }
    if !revised.is_empty() {
        notices::dispute_outcome(state, statement, &outcome_text(DisputeOutcome::Revised)).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_published_or_settled_statement_is_disputed() {
        assert!(disputable(StatementStatus::Published));
        assert!(disputable(StatementStatus::Settled));
        assert!(!disputable(StatementStatus::Draft));
        assert!(!disputable(StatementStatus::Approved));
        assert!(!disputable(StatementStatus::Superseded));
    }

    #[test]
    fn the_ticket_is_found_by_the_dispute() {
        assert_eq!(ticket_key(7), "property.statement_dispute:7");
        assert_eq!(DISPUTE_CATEGORY, "owner_statement_dispute");
    }

    #[test]
    fn a_new_dispute_is_open() {
        let dispute = StatementDispute::default();
        assert!(dispute.is_open());
        assert_eq!(dispute.outcome, None);
    }
}
