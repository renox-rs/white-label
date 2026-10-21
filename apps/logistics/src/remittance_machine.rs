//! The COD remittance state machine (03 Logistics §7.1 *COD remittance*).
//!
//! ```text
//! draft --approve--> approved --reopen--> draft
//! approved --reopen_system (a shipment on it goes under review)--> draft
//! approved --pay--> paid
//! paid --return_payment--> payment_returned --repay--> paid   (BL-LOG-17)
//! ```
//!
//! All four states are declared; `return_payment` and `repay` come with
//! BL-LOG-17, the `dispute` flag with BL-LOG-18. Renox guards get no
//! `AppState`, so the checks that need the database (the lines still
//! unapproved, whether the client is blocked, the payout) run in the service
//! ([`crate::remit`]) and the guards read what it worked out in a
//! [`RemittanceInput`].

use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use crate::permissions;
use crate::remittance::{Remittance, RemittanceStatus};
use crate::texts::english;

/// `approve`: draft → approved.
pub const APPROVE: &str = "approve";
/// `reopen`: approved → draft.
pub const REOPEN: &str = "reopen";
/// `reopen_system`: `reopen` run by the system, when a shipment on the
/// approved remittance goes under review (BL-LOG-09; a transition is either
/// a user's or the system's).
pub const REOPEN_SYSTEM: &str = "reopen_system";
/// `pay`: approved → paid.
pub const PAY: &str = "pay";

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemittanceInput {
    /// How many adjustment lines on the draft are not approved (`approve`).
    #[serde(default)]
    pub unapproved_lines: i64,
    /// The bank reference of the payment (`pay`).
    #[serde(default)]
    pub payment_reference: Option<String>,
    /// The account the client is paid to (`pay`).
    #[serde(default)]
    pub bank_account: Option<String>,
    /// Whether the client is blocked (`pay`; `PTY.ACL.03`, BL-LOG-19).
    #[serde(default)]
    pub client_blocked: bool,
    /// The voucher of the payout in wl-payments (`pay`).
    #[serde(default)]
    pub payment_id: Option<i64>,
    /// The number of that voucher (`pay`).
    #[serde(default)]
    pub payment_number: Option<String>,
}

fn input_of(ctx: &TransitionCtx) -> RemittanceInput {
    ctx.input_as::<RemittanceInput>().unwrap_or_default()
}

fn text_given(text: &Option<String>) -> bool {
    text.as_deref().is_some_and(|text| !text.trim().is_empty())
}

/// `approve` (`LOG.ACL.07`, `LOG.ACL.12`): a user with `remittances.approve`
/// other than the one who prepared it, and every adjustment line on it
/// approved.
fn approve() -> Transition<Remittance> {
    Transition::new(APPROVE)
        .label("Approve")
        .tone("primary")
        .permission(permissions::REMITTANCES_APPROVE)
        .from([RemittanceStatus::Draft])
        .to(RemittanceStatus::Approved)
        .guard("not_the_preparer", |r: &Remittance, ctx| {
            let same = ctx
                .actor()
                .is_some_and(|user| r.prepared_by == Some(user.id));
            Box::pin(async move {
                if same {
                    Err(english("logistics.remittance.own_draft", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .guard("adjustment_lines_approved", |_r: &Remittance, ctx| {
            let open = input_of(ctx).unapproved_lines;
            Box::pin(async move {
                if open > 0 {
                    Err(english("logistics.remittance.unapproved_lines", &[("count", &open)]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|r: &Remittance, ctx, tx| {
            let id = r.id;
            let actor = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                let at = Some(renox::db::now());
                Remittance::query()
                    .where_(Remittance::ID.eq(id))
                    .update(&mut *tx, &[("approved_by", &actor), ("approved_at", &at)])
                    .await?;
                Ok(())
            })
        })
}

/// `reopen` (a user with `remittances.approve`) or `reopen_system` (the
/// system): back to a draft, so a line can be added or the shipments rebuilt.
fn reopen(name: &'static str, by_system: bool) -> Transition<Remittance> {
    let transition = Transition::new(name)
        .label("Reopen")
        .tone("warning")
        .from([RemittanceStatus::Approved])
        .to(RemittanceStatus::Draft)
        .effect(|r: &Remittance, _ctx, tx| {
            let id = r.id;
            Box::pin(async move {
                let none: Option<i64> = None;
                let no_time: Option<renox::db::DateTime> = None;
                Remittance::query()
                    .where_(Remittance::ID.eq(id))
                    .update(&mut *tx, &[("approved_by", &none), ("approved_at", &no_time)])
                    .await?;
                Ok(())
            })
        });
    if by_system {
        transition.system()
    } else {
        transition.permission(permissions::REMITTANCES_APPROVE)
    }
}

/// `pay`: the payment reference is recorded, the client has a bank account
/// and is not blocked, and there is something to pay. The payout itself is
/// recorded by the service before the transition and its voucher is in the
/// input.
fn pay() -> Transition<Remittance> {
    Transition::new(PAY)
        .label("Pay")
        .tone("primary")
        .permission(permissions::REMITTANCES_PAY)
        .from([RemittanceStatus::Approved])
        .to(RemittanceStatus::Paid)
        .guard("payment_reference_recorded", |_r: &Remittance, ctx| {
            let given = text_given(&input_of(ctx).payment_reference);
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(english("logistics.remittance.reference_needed", &[]))
                }
            })
        })
        .guard("bank_account_on_file", |_r: &Remittance, ctx| {
            let given = text_given(&input_of(ctx).bank_account);
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(english("logistics.remittance.account_needed", &[]))
                }
            })
        })
        .guard("client_not_blocked", |_r: &Remittance, ctx| {
            let blocked = input_of(ctx).client_blocked;
            Box::pin(async move {
                if blocked {
                    Err(english("logistics.remittance.client_blocked", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .guard("something_to_pay", |r: &Remittance, _ctx| {
            let nothing = r.net <= 0;
            Box::pin(async move {
                if nothing {
                    Err(english("logistics.remittance.nothing_to_pay", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|r: &Remittance, ctx, tx| {
            let id = r.id;
            let actor = ctx.actor().map(|user| user.id);
            let input = input_of(ctx);
            Box::pin(async move {
                let at = Some(renox::db::now());
                let reference = input.payment_reference.map(|text| text.trim().to_owned());
                Remittance::query()
                    .where_(Remittance::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("paid_by", &actor),
                            ("paid_at", &at),
                            ("payment_reference", &reference),
                            ("payment_id", &input.payment_id),
                            ("payment_number", &input.payment_number),
                            ("bank_account", &input.bank_account),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// Every transition of the machine.
pub fn transitions() -> Vec<Transition<Remittance>> {
    vec![
        approve(),
        reopen(REOPEN, false),
        reopen(REOPEN_SYSTEM, true),
        pay(),
    ]
}

impl StateMachine for Remittance {
    type State = RemittanceStatus;
    const COLUMN: renox::db::Col<Remittance, RemittanceStatus> = Remittance::STATUS;

    fn machine() -> Machine<Self> {
        use RemittanceStatus::{Approved, Draft, PaymentReturned, Paid};
        let machine = Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Approved)
            .tone("info")
            .state(Paid)
            .tone("success")
            .state(PaymentReturned)
            .tone("error");
        transitions().into_iter().fold(machine, Machine::transition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_declares_the_four_states_and_the_base_transitions() {
        let machine = Remittance::machine();
        assert_eq!(machine.states().len(), 4);
        for name in [APPROVE, REOPEN, REOPEN_SYSTEM, PAY] {
            assert!(machine.find(name).is_ok(), "{name}");
        }
        assert_eq!(machine.find(PAY).unwrap().from_states(), ["approved"]);
    }
}
