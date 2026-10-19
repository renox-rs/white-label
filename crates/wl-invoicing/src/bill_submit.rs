//! Submitting a supplier bill (wl-invoicing §4 `submit`, `submit_held`,
//! `FIN.ACL.03`).
//!
//! `submit` moves a `draft` bill to `matched` and `submit_held` moves it to
//! `exception`, both for `supplier_bills.create`. The spec's one action has
//! two targets, so [`submit`] picks one: with no PO or contract matching
//! built, a bill that is not held goes to `matched`; a bill the duplicate
//! check finds goes to `exception` with a `possible_duplicate`
//! [`BillException`] row linking the earlier bill, and `duplicate_of` set.
//! Each has a system twin (`submit_system`, `submit_held_system`) for callers
//! with no signed-in user.
//!
//! The machine has no app state, so [`submit`] gathers the input
//! ([`SubmitInput`]) and runs the transition in one transaction that first
//! takes the supplier lock, so of two bills of one supplier submitted at once
//! the second sees the first. Guards: the supplier is found; at least one
//! line; the receiving entity is set and active; the exchange rate is set for
//! a foreign currency; a bill against a contract has a service date on every
//! line; and the transition fits the duplicate check (`submit` only when not
//! held, `submit_held` only when held). A bare POST without the input is
//! refused.

use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::LegalEntity;

use crate::bill_line::BillLine;
use crate::credit_notes::{By, run_of};
use crate::duplicates::{self, BillException, DuplicateMatch};
use crate::permissions;
use crate::supplier_bill::{POSSIBLE_DUPLICATE, SupplierBill, SupplierBillStatus};
use crate::texts::english;

/// The transition to `matched`.
pub const SUBMIT: &str = "submit";
/// The transition to `exception`, for a possible duplicate.
pub const SUBMIT_HELD: &str = "submit_held";
/// The system twin of [`SUBMIT`].
pub const SUBMIT_SYSTEM: &str = "submit_system";
/// The system twin of [`SUBMIT_HELD`].
pub const SUBMIT_HELD_SYSTEM: &str = "submit_held_system";

/// What a submit run carries to its guards and effect, gathered by
/// [`submit`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SubmitInput {
    /// Whether the supplier is found.
    pub supplier_found: bool,
    /// How many lines the bill has.
    pub lines: usize,
    /// Whether the receiving entity is found and active.
    pub entity_active: bool,
    /// How many lines have no service date.
    pub undated_lines: usize,
    /// The earlier bill this one may duplicate, if any.
    pub duplicate: Option<DuplicateMatch>,
}

/// The run's input, if it is a [`SubmitInput`].
fn input(ctx: &TransitionCtx) -> Option<SubmitInput> {
    ctx.input_as::<SubmitInput>().ok()
}

/// The four submit transitions, for the machine.
pub(crate) fn transitions() -> [Transition<SupplierBill>; 4] {
    [
        rules(
            Transition::new(SUBMIT)
                .label("Submit")
                .tone("primary")
                .permission(permissions::SUPPLIER_BILLS_CREATE),
            false,
        ),
        rules(
            Transition::new(SUBMIT_HELD)
                .label("Submit (hold)")
                .tone("warning")
                .permission(permissions::SUPPLIER_BILLS_CREATE),
            true,
        ),
        rules(
            Transition::new(SUBMIT_SYSTEM)
                .label("Submit (system)")
                .system(),
            false,
        ),
        rules(
            Transition::new(SUBMIT_HELD_SYSTEM)
                .label("Submit (hold, system)")
                .system(),
            true,
        ),
    ]
}

/// A guard that passes when `ok` says so, else refuses with the text `key`.
fn check(
    ok: impl FnOnce(&SubmitInput) -> bool,
    ctx: &TransitionCtx,
    key: &str,
) -> Result<(), String> {
    match input(ctx) {
        Some(input) if ok(&input) => Ok(()),
        _ => Err(english(key).to_owned()),
    }
}

/// What the submit transitions share; `held` is the target `exception`.
fn rules(transition: Transition<SupplierBill>, held: bool) -> Transition<SupplierBill> {
    let to = if held {
        SupplierBillStatus::Exception
    } else {
        SupplierBillStatus::Matched
    };
    transition
        .from([SupplierBillStatus::Draft])
        .to(to)
        .guard("supplier_found", |_bill: &SupplierBill, ctx| {
            Box::pin(async move {
                check(
                    |i| i.supplier_found,
                    ctx,
                    "wl_invoicing.supplier_bill.problem.supplier",
                )
            })
        })
        .guard("has_lines", |_bill: &SupplierBill, ctx| {
            Box::pin(async move {
                check(
                    |i| i.lines > 0,
                    ctx,
                    "wl_invoicing.supplier_bill.problem.lines",
                )
            })
        })
        .guard("entity_active", |bill: &SupplierBill, ctx| {
            let set = bill.legal_entity_id.is_some();
            Box::pin(async move {
                check(
                    |i| set && i.entity_active,
                    ctx,
                    "wl_invoicing.supplier_bill.problem.entity",
                )
            })
        })
        .guard("rate_set", |bill: &SupplierBill, _ctx| {
            let known = bill.aed_rate().is_some();
            Box::pin(async move {
                if known {
                    Ok(())
                } else {
                    Err(english("wl_invoicing.supplier_bill.problem.rate").to_owned())
                }
            })
        })
        .guard("service_dates", |bill: &SupplierBill, ctx| {
            let contract = bill.contract().is_some();
            Box::pin(async move {
                check(
                    |i| !contract || i.undated_lines == 0,
                    ctx,
                    "wl_invoicing.supplier_bill.problem.service_dates",
                )
            })
        })
        .guard(
            if held { "is_held" } else { "not_held" },
            move |_bill: &SupplierBill, ctx| {
                Box::pin(async move {
                    if held {
                        check(
                            |i| i.duplicate.is_some(),
                            ctx,
                            "wl_invoicing.supplier_bill.problem.held",
                        )
                    } else {
                        check(
                            |i| i.duplicate.is_none(),
                            ctx,
                            "wl_invoicing.supplier_bill.problem.not_held",
                        )
                    }
                })
            },
        )
        .effect(move |bill: &SupplierBill, ctx, tx| {
            let bill_id = bill.id;
            let duplicate = input(ctx).and_then(|i| i.duplicate);
            Box::pin(async move {
                if !held {
                    return Ok(());
                }
                let earlier = duplicate.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("a held submit without its match"))
                })?;
                BillException {
                    bill_id,
                    reason_code: POSSIBLE_DUPLICATE.to_owned(),
                    related_bill_id: Some(earlier.bill_id),
                    ..Default::default()
                }
                .insert(&mut *tx)
                .await?;
                // Not the status column, so outside the machine's guard.
                SupplierBill::query()
                    .where_(SupplierBill::ID.eq(bill_id))
                    .update(
                        &mut *tx,
                        &[
                            ("duplicate_of", &Some(earlier.bill_id)),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// What the guards need that does not depend on the lock: the supplier, the
/// lines and the entity.
async fn gather(state: &AppState, bill: &SupplierBill) -> Result<SubmitInput> {
    let supplier = wl_kernel::parties::directory(state)
        .billing_identity(&state.db, bill.supplier())
        .await?;
    let lines = crate::supplier_bill::lines_of(&state.db, bill.id).await?;
    let entity = match bill.legal_entity_id {
        Some(id) => LegalEntity::find(&state.db, id).await?,
        None => None,
    };
    Ok(SubmitInput {
        supplier_found: supplier.is_some(),
        lines: lines.len(),
        entity_active: entity.as_ref().is_some_and(LegalEntity::is_active),
        undated_lines: lines
            .iter()
            .filter(|l: &&BillLine| l.service_date.is_none())
            .count(),
        duplicate: None,
    })
}

/// Submits draft bill `id`, as a user (`supplier_bills.create`) or the
/// system, in one transaction: the supplier is locked, the duplicate check
/// runs, and the bill goes to `matched` or, when held, to `exception` with a
/// `possible_duplicate` row. Returns the bill as it is after the commit.
///
/// # Errors
///
/// `Forbidden` without `supplier_bills.create`; `NotFrom` when the bill is not
/// a `draft`; `Guard` (422) for an unknown supplier, no lines, an entity that
/// is not set or not active, a foreign currency without a rate, or a
/// contract bill with a line that has no service date. Nothing is written
/// then.
pub async fn submit(
    state: &AppState,
    by: By<'_>,
    id: i64,
) -> std::result::Result<SupplierBill, TransitionError> {
    let current = SupplierBill::find_or_404(&state.db, id).await?;
    let mut gathered = gather(state, &current).await?;
    let config = crate::config_of(state);
    let mut tx = state.db.begin().await?;
    duplicates::lock_supplier(&mut tx, current.party_id).await?;
    // Read again under the lock: the comparison sees every earlier submit.
    let bill = SupplierBill::find_or_404(&mut tx, id).await?;
    gathered.duplicate = if bill.status == SupplierBillStatus::Draft {
        duplicates::find(&mut tx, &config, &bill).await?
    } else {
        None
    };
    let held = gathered.duplicate.is_some();
    let (user_name, system_name) = if held {
        (SUBMIT_HELD, SUBMIT_HELD_SYSTEM)
    } else {
        (SUBMIT, SUBMIT_SYSTEM)
    };
    let outcome = run_of(&bill, &by, user_name, system_name, json!({}))
        .input(&gathered)
        .run_in(&mut tx)
        .await?;
    tx.commit().await?;
    outcome.emit(state).await.map_err(TransitionError::Failed)?;
    // A bill with an owner drawing asks for the approval of the drawing, by
    // the user who submits it (`FIN.ACL.21`).
    if let Some(requester) = by.id() {
        crate::drawings::request_for_bill(state, id, requester)
            .await
            .map_err(TransitionError::Failed)?;
    }
    Ok(SupplierBill::find_or_404(&state.db, id).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::NaiveDate;
    use renox::serde_json::to_value;
    use renox::state_machine::StateMachine;

    fn ready() -> SubmitInput {
        SubmitInput {
            supplier_found: true,
            lines: 2,
            entity_active: true,
            undated_lines: 0,
            duplicate: None,
        }
    }

    fn run(input: &SubmitInput) -> TransitionCtx {
        TransitionCtx::new(None).with_input(to_value(input).unwrap())
    }

    fn bill() -> SupplierBill {
        SupplierBill {
            id: 3,
            party_id: 1,
            legal_entity_id: Some(1),
            currency: "AED".into(),
            ..Default::default()
        }
    }

    async fn refusal(name: &str, bill: &SupplierBill, ctx: &TransitionCtx) -> Option<String> {
        let machine = SupplierBill::machine();
        match machine.find(name).unwrap().check_guards(bill, ctx).await {
            Ok(()) => None,
            Err(e) => Some(e.to_string()),
        }
    }

    #[test]
    fn submit_has_two_targets_and_a_system_twin_each() {
        let machine = SupplierBill::machine();
        let to = |name: &str| machine.find(name).unwrap().to_state().unwrap().to_owned();
        assert_eq!(to(SUBMIT), "matched");
        assert_eq!(to(SUBMIT_SYSTEM), "matched");
        assert_eq!(to(SUBMIT_HELD), "exception");
        assert_eq!(to(SUBMIT_HELD_SYSTEM), "exception");
        assert!(machine.find(SUBMIT_SYSTEM).unwrap().is_system());
        assert!(!machine.find(SUBMIT).unwrap().is_system());
        assert_eq!(machine.find(SUBMIT).unwrap().from_states(), ["draft"]);
    }

    #[renox::test]
    async fn the_guards_pass_a_ready_bill_and_pick_the_target_by_the_hold() {
        let held = SubmitInput {
            duplicate: Some(DuplicateMatch {
                bill_id: 1,
                key: "supplier_invoice_number".into(),
            }),
            ..ready()
        };
        assert_eq!(refusal(SUBMIT, &bill(), &run(&ready())).await, None);
        assert_eq!(refusal(SUBMIT_HELD, &bill(), &run(&held)).await, None);
        assert!(refusal(SUBMIT, &bill(), &run(&held)).await.is_some());
        assert!(
            refusal(SUBMIT_HELD, &bill(), &run(&ready()))
                .await
                .is_some()
        );
    }

    #[renox::test]
    async fn the_guards_refuse_what_cannot_be_submitted() {
        let msg = |text: Option<String>| text.unwrap_or_default();
        let no_supplier = SubmitInput {
            supplier_found: false,
            ..ready()
        };
        assert!(msg(refusal(SUBMIT, &bill(), &run(&no_supplier)).await).contains("supplier"));
        let no_lines = SubmitInput {
            lines: 0,
            ..ready()
        };
        assert!(msg(refusal(SUBMIT, &bill(), &run(&no_lines)).await).contains("at least one line"));
        let inactive = SubmitInput {
            entity_active: false,
            ..ready()
        };
        assert!(msg(refusal(SUBMIT, &bill(), &run(&inactive)).await).contains("entity"));
        let no_entity = SupplierBill {
            legal_entity_id: None,
            ..bill()
        };
        assert!(msg(refusal(SUBMIT, &no_entity, &run(&ready())).await).contains("entity"));
        let usd = SupplierBill {
            currency: "USD".into(),
            ..bill()
        };
        assert!(msg(refusal(SUBMIT, &usd, &run(&ready())).await).contains("exchange rate"));
        let rated = SupplierBill {
            exchange_rate_micro: Some(3_670_000),
            ..usd
        };
        assert_eq!(refusal(SUBMIT, &rated, &run(&ready())).await, None);
        let contract = SupplierBill {
            contract_type: Some("ticketing.contract".into()),
            contract_id: Some(2),
            period_start: NaiveDate::from_ymd_opt(2026, 10, 1),
            period_end: NaiveDate::from_ymd_opt(2026, 10, 31),
            ..bill()
        };
        let undated = SubmitInput {
            undated_lines: 1,
            ..ready()
        };
        assert!(msg(refusal(SUBMIT, &contract, &run(&undated)).await).contains("service date"));
        assert_eq!(refusal(SUBMIT, &bill(), &run(&undated)).await, None);
        // A bare run, with no input, is refused.
        let bare = TransitionCtx::new(None);
        assert!(refusal(SUBMIT, &bill(), &bare).await.is_some());
    }
}
