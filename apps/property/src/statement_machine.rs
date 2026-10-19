//! The Owner statement state machine (01 Property §7.1 *Owner statement*).
//!
//! ```text
//! [*] draft --approve--> approved --publish--> published
//! approved --return--> draft
//! published --revise--> superseded                  (and a new draft revision)
//! ```
//!
//! `approve` and `return` are the property manager's (`owner_statements.approve`);
//! `approve` is refused for the user who prepared the draft (`PRP.ACL.08`): the
//! user who generated or last edited it. `publish` and `revise` are the
//! accountant's (`owner_statements.prepare`). `revise` needs a reason and a
//! statement with no payout recorded; in its transaction it creates the
//! next revision of the same period as a draft that copies the lines, with
//! the same balance brought forward, and the old statement becomes
//! `superseded`. What follows the commit (the e-mail to the owner, telling
//! the preparer of a return) is in [`crate::statement_events`].
//!
//! The states `settled` and the settling transitions come with BL-PRP-20.

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition};
use wl_kernel::documents::today;

use crate::permissions;
use crate::statement::{OwnerStatement, StatementLine, StatementStatus};
use crate::texts::english;

/// `approve`: a property manager other than the preparer.
pub const APPROVE: &str = "approve";
/// `return`: back to the preparer, with a reason.
pub const RETURN: &str = "return";
/// `publish`: the statement goes to the owner.
pub const PUBLISH: &str = "publish";
/// `revise`: a corrected draft replaces a published statement.
pub const REVISE: &str = "revise";

/// A transition a user runs.
fn user(name: &str, label: &str, permission: &'static str) -> Transition<OwnerStatement> {
    Transition::new(name).label(label).permission(permission)
}

/// Why `statement` cannot be approved by the user `actor`, if it cannot: the
/// preparer does not approve (`PRP.ACL.08`).
pub fn approver_problem(statement: &OwnerStatement, actor: Option<i64>) -> Option<String> {
    match (statement.prepared_by, actor) {
        (Some(preparer), Some(approver)) if preparer == approver => {
            Some(english("property.statements.preparer", &[]))
        }
        _ => None,
    }
}

/// Why `statement` cannot be revised, if it cannot: a payout was recorded.
pub fn revise_problem(statement: &OwnerStatement) -> Option<String> {
    if statement.payout_amount.is_some() || statement.payout_payment_id.is_some() {
        return Some(english("property.statements.paid_out", &[]));
    }
    None
}

/// The effect of `revise` in its transaction: the next revision as a draft
/// with the lines copied, prepared by the user who revised it.
async fn revise_effects(
    tx: &mut Transaction,
    statement: &OwnerStatement,
    by: Option<i64>,
    reason: &str,
) -> Result {
    let mut next = OwnerStatement {
        agreement_id: statement.agreement_id,
        owner_party_id: statement.owner_party_id,
        period_start: statement.period_start,
        period_end: statement.period_end,
        revision: statement.revision + 1,
        previous_id: Some(statement.id),
        brought_forward: statement.brought_forward,
        collected: statement.collected,
        expenses: statement.expenses,
        fee: statement.fee,
        adjustments: statement.adjustments,
        prepared_by: by,
        status: StatementStatus::Draft,
        ..Default::default()
    };
    next.insert(&mut *tx).await?;
    let lines = StatementLine::query()
        .where_(StatementLine::STATEMENT_ID.eq(statement.id))
        .order_by(StatementLine::ID)
        .get(&mut *tx)
        .await?;
    for old in lines {
        let mut copy = StatementLine {
            id: 0,
            statement_id: Some(next.id),
            created_at: None,
            updated_at: None,
            ..old
        };
        copy.insert(&mut *tx).await?;
    }
    OwnerStatement::query()
        .where_(OwnerStatement::ID.eq(statement.id))
        .update(
            &mut *tx,
            &[
                ("revise_reason", &Some(reason.to_owned())),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

impl StateMachine for OwnerStatement {
    type State = StatementStatus;
    const COLUMN: renox::db::Col<OwnerStatement, StatementStatus> = OwnerStatement::STATUS;

    fn machine() -> Machine<Self> {
        use StatementStatus::{Approved, Draft, Published, Settled, Superseded};
        let machine = Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Approved)
            .tone("info")
            .state(Published)
            .tone("info")
            .state(Settled)
            .tone("success")
            .terminal()
            .state(Superseded)
            .tone("neutral")
            .terminal();
        machine
            .transition(
                user(APPROVE, "Approve", permissions::OWNER_STATEMENTS_APPROVE)
                    .tone("primary")
                    .from([Draft])
                    .to(Approved)
                    .guard("not_the_preparer", |statement: &OwnerStatement, ctx| {
                        let problem =
                            approver_problem(statement, ctx.actor().map(|actor| actor.id));
                        Box::pin(async move { problem.map_or(Ok(()), Err) })
                    })
                    .effect(|statement: &OwnerStatement, ctx, tx| {
                        let id = statement.id;
                        let approver = ctx.actor().map(|actor| actor.id);
                        Box::pin(async move {
                            OwnerStatement::query()
                                .where_(OwnerStatement::ID.eq(id))
                                .update(
                                    &mut *tx,
                                    &[
                                        ("approved_by", &approver),
                                        ("approved_on", &Some(today())),
                                        ("updated_at", &renox::db::now()),
                                    ],
                                )
                                .await?;
                            Ok(())
                        })
                    }),
            )
            .transition(
                user(RETURN, "Return", permissions::OWNER_STATEMENTS_APPROVE)
                    .from([Approved])
                    .to(Draft)
                    .reason_required()
                    .effect(|statement: &OwnerStatement, ctx, tx| {
                        let id = statement.id;
                        let reason = ctx.reason().map(|r| r.trim().to_owned());
                        Box::pin(async move {
                            OwnerStatement::query()
                                .where_(OwnerStatement::ID.eq(id))
                                .update(
                                    &mut *tx,
                                    &[
                                        ("approved_by", &None::<i64>),
                                        ("approved_on", &None::<renox::chrono::NaiveDate>),
                                        ("return_reason", &reason),
                                        ("updated_at", &renox::db::now()),
                                    ],
                                )
                                .await?;
                            Ok(())
                        })
                    }),
            )
            .transition(
                user(PUBLISH, "Publish", permissions::OWNER_STATEMENTS_PREPARE)
                    .tone("primary")
                    .from([Approved])
                    .to(Published)
                    .effect(|statement: &OwnerStatement, ctx, tx| {
                        let id = statement.id;
                        let by = ctx.actor().map(|actor| actor.id);
                        Box::pin(async move {
                            OwnerStatement::query()
                                .where_(OwnerStatement::ID.eq(id))
                                .update(
                                    &mut *tx,
                                    &[
                                        ("published_on", &Some(today())),
                                        ("published_by", &by),
                                        ("updated_at", &renox::db::now()),
                                    ],
                                )
                                .await?;
                            Ok(())
                        })
                    }),
            )
            .transition(
                user(REVISE, "Revise", permissions::OWNER_STATEMENTS_PREPARE)
                    .from([Published])
                    .to(Superseded)
                    .reason_required()
                    .guard("no_payout", |statement: &OwnerStatement, _ctx| {
                        let problem = revise_problem(statement);
                        Box::pin(async move { problem.map_or(Ok(()), Err) })
                    })
                    .effect(|statement: &OwnerStatement, ctx, tx| {
                        let statement = statement.clone();
                        let by = ctx.actor().map(|actor| actor.id);
                        let reason = ctx.reason().unwrap_or_default().trim().to_owned();
                        Box::pin(async move { revise_effects(&mut *tx, &statement, by, &reason).await })
                    }),
            )
    }
}

/// Registers the machine with `rnx state:diagram`.
pub(crate) fn register() {
    renox::state_machine::register::<OwnerStatement>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_of_the_spec_is_declared() {
        let machine = OwnerStatement::machine();
        let table: [(&str, &[&str], &str); 4] = [
            (APPROVE, &["draft"], "approved"),
            (RETURN, &["approved"], "draft"),
            (PUBLISH, &["approved"], "published"),
            (REVISE, &["published"], "superseded"),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to) in table {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert!(!transition.is_system(), "{name}");
        }
        assert!(machine.find(RETURN).unwrap().needs_reason());
        assert!(machine.find(REVISE).unwrap().needs_reason());
        assert!(!machine.find(APPROVE).unwrap().needs_reason());
    }

    #[test]
    fn a_statement_starts_a_draft_and_two_states_are_terminal() {
        let machine = OwnerStatement::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        assert_eq!(machine.states().len(), 5);
        assert!(machine.states().iter().all(|s| s.tone.is_some()));
        let terminal: Vec<_> = machine
            .states()
            .iter()
            .filter(|s| s.terminal)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(terminal, ["settled", "superseded"]);
    }

    #[test]
    fn the_preparer_does_not_approve() {
        let edited = OwnerStatement {
            prepared_by: Some(4),
            ..Default::default()
        };
        assert!(approver_problem(&edited, Some(4)).is_some());
        assert_eq!(approver_problem(&edited, Some(5)), None);
        // A statement the rule generated and nobody edited.
        assert_eq!(approver_problem(&OwnerStatement::default(), Some(4)), None);
    }

    #[test]
    fn a_statement_with_a_payout_is_not_revised() {
        assert_eq!(revise_problem(&OwnerStatement::default()), None);
        let paid = OwnerStatement {
            payout_amount: Some(4_200_000),
            ..Default::default()
        };
        assert!(revise_problem(&paid).is_some());
    }
}
