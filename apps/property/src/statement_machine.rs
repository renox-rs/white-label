//! The Owner statement state machine (01 Property §7.1 *Owner statement*).
//!
//! ```text
//! [*] draft --approve--> approved --publish--> published
//! approved --return--> draft
//! published --revise--> superseded                  (and a new draft revision)
//! published --pay_out, carry_forward, invoice_owner--> settled
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
//! A revision also clears the dispute flag: the open disputes of the old
//! statement are cleared as *revised* ([`crate::statement_dispute`]).
//!
//! A published statement is settled one of three ways ([`crate::statement_payout`]):
//! `pay_out` (the accountant asks, a property manager other than the requester
//! approves, `pay_out_approved` runs at the final approval), `carry_forward`
//! and `invoice_owner` (the same approval, `invoice_owner_approved`). Their
//! guards read the statement as it is when they run, so a dispute or a balance
//! that changed while an approval waited fails the request.

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition};
use wl_kernel::PartyId;
use wl_kernel::SubjectMachine;
use wl_kernel::approvals::PolicyDefault;
use wl_kernel::block_guard::ActionKind;
use wl_kernel::documents::today;

use crate::permissions;
use crate::statement::{OwnerStatement, STATEMENT_SUBJECT_TYPE, StatementLine, StatementStatus};
use crate::statement_dispute::{DisputeOutcome, DisputeStatus, StatementDispute};
use crate::statement_payout::{carry_problem, invoice_input, invoice_problem, payout_problem};
use crate::texts::english;

/// `approve`: a property manager other than the preparer.
pub const APPROVE: &str = "approve";
/// `return`: back to the preparer, with a reason.
pub const RETURN: &str = "return";
/// `publish`: the statement goes to the owner.
pub const PUBLISH: &str = "publish";
/// `revise`: a corrected draft replaces a published statement.
pub const REVISE: &str = "revise";

/// `pay_out`: the accountant's request (`PRP.ACL.08`).
pub const PAY_OUT: &str = "pay_out";
/// `pay_out_approved`: the kernel runs it at the final approval.
pub const PAY_OUT_APPROVED: &str = "pay_out_approved";
/// `carry_forward`: a balance of zero or less goes to the next statement.
pub const CARRY_FORWARD: &str = "carry_forward";
/// `invoice_owner`: a property manager's request to invoice a negative balance.
pub const INVOICE_OWNER: &str = "invoice_owner";
/// `invoice_owner_approved`: the kernel runs it at the final approval.
pub const INVOICE_OWNER_APPROVED: &str = "invoice_owner_approved";

/// The approval policies' action key of `pay_out` (`PRP.ACL.08`).
pub const PAYOUT_ACTION: &str = "property.owner_payout";
/// The approval policies' action key of `invoice_owner`.
pub const INVOICE_ACTION: &str = "property.owner_invoice";

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
    // A revision clears the dispute flag: the disputes are answered by it.
    StatementDispute::query()
        .where_(StatementDispute::STATEMENT_ID.eq(statement.id))
        .where_(StatementDispute::STATUS.eq(DisputeStatus::Open))
        .update(
            &mut *tx,
            &[
                ("status", &DisputeStatus::Cleared),
                ("outcome", &Some(DisputeOutcome::Revised)),
                ("cleared_by", &by),
                ("cleared_on", &Some(today())),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    OwnerStatement::query()
        .where_(OwnerStatement::ID.eq(statement.id))
        .update(
            &mut *tx,
            &[
                ("revise_reason", &Some(reason.to_owned())),
                ("disputed", &false),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// A transition only the system runs (the kernel at a final approval).
fn system(name: &str, label: &str) -> Transition<OwnerStatement> {
    Transition::new(name).label(label).system()
}

/// Writes columns of the statement `id` in `tx`; not the status column.
async fn settle_in(
    tx: &mut Transaction,
    id: i64,
    how: &str,
    payout: Option<i64>,
    carried: i64,
) -> Result {
    OwnerStatement::query()
        .where_(OwnerStatement::ID.eq(id))
        .update(
            &mut *tx,
            &[
                ("settled_how", &Some(how.to_owned())),
                ("payout_amount", &payout),
                ("carried_forward", &carried),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// The guard and effect `pay_out` and `pay_out_approved` share.
fn pay_out_rules(transition: Transition<OwnerStatement>) -> Transition<OwnerStatement> {
    transition
        .from([StatementStatus::Published])
        .to(StatementStatus::Settled)
        .guard("payable", |statement: &OwnerStatement, _ctx| {
            let problem = payout_problem(statement);
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|statement: &OwnerStatement, _ctx, tx| {
            let id = statement.id;
            let amount = statement.balance();
            Box::pin(async move { settle_in(&mut *tx, id, "payout", Some(amount), 0).await })
        })
}

/// The guard and effect `invoice_owner` and `invoice_owner_approved` share.
fn invoice_owner_rules(transition: Transition<OwnerStatement>) -> Transition<OwnerStatement> {
    transition
        .from([StatementStatus::Published])
        .to(StatementStatus::Settled)
        .guard("invoiceable", |statement: &OwnerStatement, ctx| {
            let problem = invoice_problem(statement, &invoice_input(ctx));
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|statement: &OwnerStatement, _ctx, tx| {
            let id = statement.id;
            Box::pin(async move { settle_in(&mut *tx, id, "invoiced", None, 0).await })
        })
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
                        Box::pin(
                            async move { revise_effects(&mut *tx, &statement, by, &reason).await },
                        )
                    }),
            )
            .transition(pay_out_rules(
                user(PAY_OUT, "Pay out", permissions::OWNER_STATEMENTS_PREPARE).tone("primary"),
            ))
            .transition(pay_out_rules(system(
                PAY_OUT_APPROVED,
                "Pay out (approved)",
            )))
            .transition(
                user(
                    CARRY_FORWARD,
                    "Carry forward",
                    permissions::OWNER_STATEMENTS_PREPARE,
                )
                .from([Published])
                .to(Settled)
                .guard("not_positive", |statement: &OwnerStatement, _ctx| {
                    let problem = carry_problem(statement);
                    Box::pin(async move { problem.map_or(Ok(()), Err) })
                })
                .effect(|statement: &OwnerStatement, _ctx, tx| {
                    let id = statement.id;
                    let carried = statement.balance();
                    Box::pin(async move {
                        settle_in(&mut *tx, id, "carried_forward", None, carried).await
                    })
                }),
            )
            .transition(invoice_owner_rules(user(
                INVOICE_OWNER,
                "Invoice the owner",
                permissions::OWNER_STATEMENTS_APPROVE,
            )))
            .transition(invoice_owner_rules(system(
                INVOICE_OWNER_APPROVED,
                "Invoice the owner (approved)",
            )))
    }
}

/// The default approval policy of `pay_out` (`PRP.ACL.08`): one step by a
/// property manager, never the requester.
pub fn payout_policy() -> PolicyDefault {
    PolicyDefault::new(PAYOUT_ACTION).step([permissions::PROPERTY_MANAGER_ROLE])
}

/// The default approval policy of `invoice_owner`: one step by a property
/// manager other than the requester.
pub fn invoice_policy() -> PolicyDefault {
    PolicyDefault::new(INVOICE_ACTION).step([permissions::PROPERTY_MANAGER_ROLE])
}

/// The Owner statement in the kernel's registry: the approval actions of
/// `pay_out` and `invoice_owner`, and the block guard of the payout (the owner
/// on hold is refused, and refused again at the final approval).
pub fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<OwnerStatement>(STATEMENT_SUBJECT_TYPE)
        .action(PAY_OUT, PAYOUT_ACTION)
        .action(INVOICE_OWNER, INVOICE_ACTION)
        .block_guarded(
            PAY_OUT,
            ActionKind::MoneyOut,
            |statement: &OwnerStatement| Some(PartyId(statement.owner_party_id)),
        )
        .build()
}

/// Registers the machine with `rnx state:diagram`.
pub(crate) fn register() {
    subject_machine().register();
    renox::state_machine::register::<OwnerStatement>();
    payout_policy().register();
    invoice_policy().register();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_of_the_spec_is_declared() {
        let machine = OwnerStatement::machine();
        let table: [(&str, &[&str], &str, bool); 9] = [
            (APPROVE, &["draft"], "approved", false),
            (RETURN, &["approved"], "draft", false),
            (PUBLISH, &["approved"], "published", false),
            (REVISE, &["published"], "superseded", false),
            (PAY_OUT, &["published"], "settled", false),
            (PAY_OUT_APPROVED, &["published"], "settled", true),
            (CARRY_FORWARD, &["published"], "settled", false),
            (INVOICE_OWNER, &["published"], "settled", false),
            (INVOICE_OWNER_APPROVED, &["published"], "settled", true),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to, by_system) in table {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.is_system(), by_system, "{name}");
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
    fn the_policies_ask_a_property_manager() {
        let expected = PolicyDefault::new("property.owner_payout").step(["property_manager"]);
        assert_eq!(payout_policy(), expected);
        assert_eq!(payout_policy().action(), PAYOUT_ACTION);
        assert_eq!(invoice_policy().action(), INVOICE_ACTION);
    }

    #[test]
    fn the_payout_is_a_guarded_money_out() {
        let machine = subject_machine();
        assert_eq!(machine.action_key(PAY_OUT), PAYOUT_ACTION);
        assert_eq!(machine.action_key(INVOICE_OWNER), INVOICE_ACTION);
        assert_eq!(machine.guarded_kind(PAY_OUT), Some(ActionKind::MoneyOut));
        assert_eq!(machine.guarded_kind(CARRY_FORWARD), None);
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
