//! The CounterSale state machine (wl-sales §4.3; base for BL-SAL-22).
//!
//! ```text
//! [*] open --complete--> completed
//! ```
//!
//! `complete` is by a holder of `pos.sell`. It needs at least one line, and
//! tenders that add up to the total: cash, card or other methods, or the
//! customer account (only with the setting *counter sales on account* and a
//! customer on the sale). A completed sale is never edited, voided or
//! reopened (`SAL.ACL.04`): it is reversed by a sales return.
//!
//! The input carries what the guards and the effect cannot fetch themselves
//! (an effect has no `AppState`): the tenders, the totals of the lines as the
//! service read them, and the setting.

use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition};
use serde::{Deserialize, Serialize};

use crate::counter_sale::{CounterSale, CounterSalePayment, CounterSaleStatus};
use crate::permissions;
use crate::texts::english;

/// `open → completed`: the sale is paid and locked.
pub const COMPLETE: &str = "complete";

/// The payment method of a tender charged to the customer account.
pub const METHOD_ACCOUNT: &str = "account";

/// The payment methods a tender can have.
pub const METHODS: &[&str] = &["cash", "card", METHOD_ACCOUNT, "other"];

/// One part of the payment of a sale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tender {
    /// `cash`, `card`, `account` or `other`.
    pub method: String,
    /// The amount, in fils.
    pub amount: i64,
    /// The `wl-payments` payment the till recorded for it, if any.
    pub payment_id: Option<i64>,
}

impl Tender {
    /// `amount` fils by `method`, without a payment id.
    pub fn new(method: &str, amount: i64) -> Self {
        Tender {
            method: method.to_owned(),
            amount,
            payment_id: None,
        }
    }

    /// With the id of the `wl-payments` payment.
    pub fn payment(mut self, payment_id: i64) -> Self {
        self.payment_id = Some(payment_id);
        self
    }
}

/// What `complete` takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompleteInput {
    /// What was taken.
    pub tenders: Vec<Tender>,
    /// How many lines the sale has.
    pub line_count: i64,
    /// The sum of the lines before VAT, in fils.
    pub net: i64,
    /// The VAT, in fils.
    pub vat: i64,
    /// Net plus VAT, in fils: what the tenders must add up to.
    pub gross: i64,
    /// The setting *counter sales on account*.
    pub allow_account: bool,
}

/// A guard that fails with the English `key` unless `ok`.
fn check(ok: bool, key: &str) -> std::result::Result<(), String> {
    if ok { Ok(()) } else { Err(english(key, &[])) }
}

impl StateMachine for CounterSale {
    type State = CounterSaleStatus;
    const COLUMN: renox::db::Col<CounterSale, CounterSaleStatus> = CounterSale::STATUS;

    fn machine() -> Machine<Self> {
        use CounterSaleStatus::{Completed, Open};
        Machine::new()
            .initial([Open])
            .state(Open)
            .tone("info")
            .state(Completed)
            .tone("success")
            .transition(complete())
    }
}

/// `complete`: open → completed, with the tenders recorded.
fn complete() -> Transition<CounterSale> {
    Transition::new(COMPLETE)
        .label("Complete")
        .tone("success")
        .permission(permissions::POS_SELL)
        .from([CounterSaleStatus::Open])
        .to(CounterSaleStatus::Completed)
        .guard("has_lines", |_s: &CounterSale, ctx| {
            let lines = ctx.input_as::<CompleteInput>().map(|i| i.line_count);
            Box::pin(async move {
                check(
                    lines.is_ok_and(|n| n > 0),
                    "wl_sales.counter.problem.no_lines",
                )
            })
        })
        .guard("tenders_match_total", |_s: &CounterSale, ctx| {
            let input = ctx.input_as::<CompleteInput>().ok();
            Box::pin(async move {
                let ok = input.is_some_and(|i| {
                    i.tenders.iter().all(|t| t.amount > 0)
                        && i.tenders.iter().map(|t| t.amount).sum::<i64>() == i.gross
                });
                check(ok, "wl_sales.counter.problem.tenders")
            })
        })
        .guard("known_methods", |_s: &CounterSale, ctx| {
            let input = ctx.input_as::<CompleteInput>().ok();
            Box::pin(async move {
                let ok = input.is_some_and(|i| {
                    i.tenders
                        .iter()
                        .all(|t| METHODS.contains(&t.method.as_str()))
                });
                check(ok, "wl_sales.counter.problem.method")
            })
        })
        .guard("account_allowed", |s: &CounterSale, ctx| {
            let has_party = s.party_id.is_some();
            let input = ctx.input_as::<CompleteInput>().ok();
            Box::pin(async move {
                let on_account = input
                    .as_ref()
                    .is_some_and(|i| i.tenders.iter().any(|t| t.method == METHOD_ACCOUNT));
                let allowed = input.is_some_and(|i| i.allow_account);
                check(
                    !on_account || (allowed && has_party),
                    "wl_sales.counter.problem.account",
                )
            })
        })
        .effect(|sale: &CounterSale, ctx, tx| {
            let id = sale.id;
            let input = ctx.input_as::<CompleteInput>().ok();
            Box::pin(async move {
                let Some(input) = input else {
                    return Ok(());
                };
                for tender in &input.tenders {
                    CounterSalePayment {
                        counter_sale_id: id,
                        method: tender.method.clone(),
                        amount: tender.amount,
                        payment_id: tender.payment_id,
                        ..CounterSalePayment::default()
                    }
                    .insert(&mut *tx)
                    .await?;
                }
                let stamp = now();
                let done = Some(stamp);
                CounterSale::query()
                    .where_(CounterSale::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("net_total", &input.net),
                            ("vat_total", &input.vat),
                            ("gross_total", &input.gross),
                            ("completed_at", &done),
                            ("updated_at", &stamp),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_two_states_and_one_transition() {
        let machine = CounterSale::machine();
        let states: Vec<&str> = machine.states().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(states, ["open", "completed"]);
        let complete = machine.find(COMPLETE).expect("complete exists");
        assert_eq!(complete.from_states(), ["open"]);
        assert_eq!(complete.to_state(), Some("completed"));
        assert!(!complete.is_system());
        assert_eq!(
            complete.guard_names(),
            [
                "has_lines",
                "tenders_match_total",
                "known_methods",
                "account_allowed"
            ]
        );
        assert_eq!(machine.initial_states(), ["open"]);
    }

    #[test]
    fn a_tender_builder_sets_the_payment() {
        let tender = Tender::new("card", 10_500).payment(7);
        assert_eq!(tender.payment_id, Some(7));
        assert_eq!(tender.amount, 10_500);
    }
}
