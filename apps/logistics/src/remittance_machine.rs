//! The COD remittance state machine (03 Logistics §7.1 *COD remittance*).
//!
//! ```text
//! draft --approve--> approved --reopen--> draft
//! approved --reopen_system (a shipment on it goes under review)--> draft
//! approved --pay--> paid
//! paid --return_payment--> payment_returned --repay--> paid   (BL-LOG-17)
//! ```
//!
//! The `dispute` flag (BL-LOG-18) is a field, not a state. Renox guards get no
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
/// `return_payment`: paid → payment_returned (BL-LOG-17).
pub const RETURN_PAYMENT: &str = "return_payment";
/// `repay`: payment_returned → paid (BL-LOG-17).
pub const REPAY: &str = "repay";

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
    /// The bank's return advice (`return_payment`).
    #[serde(default)]
    pub return_advice: Option<String>,
    /// The reason the bank gave (`return_payment`).
    #[serde(default)]
    pub return_reason: Option<String>,
    /// Whether the client's remittance account is not the one the transfer
    /// was returned from, an approved and confirmed change (`repay`;
    /// `LOG.ACL.13`).
    #[serde(default)]
    pub account_changed: bool,
    /// Whether the client's confirmation that the same account is correct is
    /// recorded (`repay`).
    #[serde(default)]
    pub account_confirmed: bool,
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
                // An approval answers the message `pay` left (BL-LOG-16).
                let message: Option<String> = None;
                Remittance::query()
                    .where_(Remittance::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("approved_by", &actor),
                            ("approved_at", &at),
                            ("message", &message),
                        ],
                    )
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
            // The offsets of open invoices are paid at `pay` too, so a net
            // of 0 with offsets is payable (BL-LOG-16).
            let nothing = r.net <= 0 && r.offset_total <= 0;
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

/// `return_payment` (`remittances.pay`): the bank returned the transfer. The
/// advice is attached and kept (each return keeps its own); the amount is
/// owed to the client again, and the offsets stay allocated.
fn return_payment() -> Transition<Remittance> {
    Transition::new(RETURN_PAYMENT)
        .label("Payment returned")
        .tone("warning")
        .permission(permissions::REMITTANCES_PAY)
        .from([RemittanceStatus::Paid])
        .to(RemittanceStatus::PaymentReturned)
        .guard("return_advice_attached", |_r: &Remittance, ctx| {
            let given = text_given(&input_of(ctx).return_advice);
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(english("logistics.remittance.advice_needed", &[]))
                }
            })
        })
        .effect(|r: &Remittance, ctx, tx| {
            let mut returned = crate::remittance::PaymentReturn {
                remittance_id: r.id,
                advice: input_of(ctx)
                    .return_advice
                    .map(|text| text.trim().to_owned())
                    .unwrap_or_default(),
                reason: input_of(ctx)
                    .return_reason
                    .map(|text| text.trim().to_owned())
                    .unwrap_or_default(),
                bank_account: r.bank_account.clone(),
                payment_reference: r.payment_reference.clone(),
                payment_number: r.payment_number.clone(),
                returned_by: ctx.actor().map(|user| user.id),
                returned_at: renox::db::now(),
                ..crate::remittance::PaymentReturn::default()
            };
            Box::pin(async move {
                returned.insert(&mut *tx).await?;
                Ok(())
            })
        })
}

/// `repay` (`remittances.pay`): the returned amount is paid again with a new
/// reference, to a changed account that is approved and confirmed
/// (`LOG.ACL.13`), or to the same account once the client's confirmation that
/// it is correct is recorded; the client is not blocked.
fn repay() -> Transition<Remittance> {
    Transition::new(REPAY)
        .label("Pay again")
        .tone("primary")
        .permission(permissions::REMITTANCES_PAY)
        .from([RemittanceStatus::PaymentReturned])
        .to(RemittanceStatus::Paid)
        .guard("new_reference", |r: &Remittance, ctx| {
            let reference = input_of(ctx)
                .payment_reference
                .map(|text| text.trim().to_owned())
                .unwrap_or_default();
            let earlier = r.payment_reference.clone().unwrap_or_default();
            Box::pin(async move {
                if reference.is_empty() {
                    Err(english("logistics.remittance.reference_needed", &[]))
                } else if reference == earlier {
                    Err(english("logistics.remittance.new_reference_needed", &[]))
                } else {
                    Ok(())
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
        .guard("account_approved_or_confirmed", |_r: &Remittance, ctx| {
            let input = input_of(ctx);
            Box::pin(async move {
                if input.account_changed || input.account_confirmed {
                    Ok(())
                } else {
                    Err(english("logistics.remittance.account_unconfirmed", &[]))
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
                let no_time: Option<renox::db::DateTime> = None;
                let no_user: Option<i64> = None;
                let no_text: Option<String> = None;
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
                            ("account_confirmed_at", &no_time),
                            ("account_confirmed_by", &no_user),
                            ("account_confirmed_note", &no_text),
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
        return_payment(),
        repay(),
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
    fn the_machine_declares_the_four_states_and_its_transitions() {
        let machine = Remittance::machine();
        assert_eq!(machine.states().len(), 4);
        for name in [APPROVE, REOPEN, REOPEN_SYSTEM, PAY, RETURN_PAYMENT, REPAY] {
            assert!(machine.find(name).is_ok(), "{name}");
        }
        assert_eq!(machine.find(PAY).unwrap().from_states(), ["approved"]);
        assert_eq!(machine.find(RETURN_PAYMENT).unwrap().from_states(), ["paid"]);
        assert_eq!(machine.find(REPAY).unwrap().from_states(), ["payment_returned"]);
    }
}
