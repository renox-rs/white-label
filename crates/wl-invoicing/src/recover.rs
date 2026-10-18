//! Recovering a written-off invoice (wl-invoicing §4 `recover`, #66).
//!
//! A payment allocated to a `written_off` invoice runs `recover` instead of
//! being refused. The spec's one action has two targets and a Renox
//! transition has one `to`, so there are two system transitions from
//! `written_off`:
//!
//! - `recover` → `written_off`, for an amount that leaves part of the
//!   write-off open;
//! - `recover_in_full` → `paid`, for an amount equal to the balance.
//!
//! The write-off never changes the balance, so on a `written_off` invoice
//! the balance is always the write-off amount less what was recovered. That
//! is the most a recovery can take. Both transitions refuse an amount ≤ 0 or
//! above the balance. Lines, totals, VAT and the write-off fields are never
//! written.
//!
//! [`recover`] runs in one transaction: the [`Allocation`], the transition
//! (whose effect moves the allocated total, the balance and the recovered
//! amount in one compare-and-set on the figures the run loaded, so two
//! recoveries at once never both pass), the relief side below, and the
//! [`Recovery`] row a reversal undoes. The relief is the invoice's one
//! `bad_debt_relief` adjustment, and a recovery's VAT share is
//! [`recovery_share`]:
//!
//! - `included` (claimed): a `bad_debt_recovery` adjustment for the share,
//!   proposed on the entity's `open` summary of the day, or left `pending`
//!   when there is none (`FIN.RUL.14` proposes it later);
//! - `pending` or `proposed`: its amount is reduced by the share, and
//!   `exclude_recovered` excludes it once the write-off is recovered in full;
//! - `excluded`, or no relief: nothing on the VAT side.
//!
//! After the commit a listener ([`crate::events`]) emits `BadDebtRecovered`
//! (and `InvoicePaid` for `recover_in_full`) and tells the accountants.

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{BadDebtRecovered, InvoiceStatus};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::{LegalEntity, Subject};

use crate::allocate::{AllocateInput, amount_text};
use crate::allocation::Allocation;
use crate::config_of;
use crate::invoice::Invoice;
use crate::line::InvoiceLine;
use crate::recovery::Recovery;
use crate::texts::english;
use crate::vat::{VatTotals, recovery_share};
use crate::vat_adjustment::{
    AdjustmentKind, AdjustmentStatus, EXCLUDE_RECOVERED, PROPOSE, ProposeInput, VatAdjustment,
};
use crate::vat_summary::{VatSummary, VatSummaryStatus};

/// The transition that leaves part of the write-off open.
pub const RECOVER: &str = "recover";
/// The transition that recovers the rest of the write-off.
pub const RECOVER_IN_FULL: &str = "recover_in_full";

/// The notification type of a recovery.
pub const RECOVERED: &str = "invoicing.bad_debt_recovered";

/// The module that sends the notice.
const MODULE: &str = "invoicing";

/// The row of the wl-notify §5.4 default table the type belongs to.
const GROUP: &str = "invoices";

/// The `recover` and `recover_in_full` transitions, for the machine.
pub(crate) fn transitions() -> [Transition<Invoice>; 2] {
    [
        rules(
            Transition::new(RECOVER)
                .label("Recover")
                .to(InvoiceStatus::WrittenOff),
        )
        .guard("leaves_a_balance", |invoice: &Invoice, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.amount < invoice.balance => Ok(()),
                    _ => Err("The amount recovers the rest: recover it in full.".to_owned()),
                }
            })
        }),
        rules(
            Transition::new(RECOVER_IN_FULL)
                .label("Recover in full")
                .to(InvoiceStatus::Paid),
        )
        .guard("settles_the_balance", |invoice: &Invoice, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.amount == invoice.balance => Ok(()),
                    _ => Err("The amount does not recover the rest of the write-off.".to_owned()),
                }
            })
        }),
    ]
}

/// What both transitions share: system only, from `written_off`, the amount
/// guards and the effect.
fn rules(transition: Transition<Invoice>) -> Transition<Invoice> {
    transition
        .system()
        .from([InvoiceStatus::WrittenOff])
        .guard("positive_amount", |_invoice: &Invoice, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.amount > 0 => Ok(()),
                    _ => Err("The amount must be more than 0.".to_owned()),
                }
            })
        })
        .guard("within_balance", |invoice: &Invoice, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.amount <= invoice.balance => Ok(()),
                    _ => Err(format!(
                        "Only {} is left to recover on this invoice.",
                        amount_text(&invoice.currency, invoice.balance)
                    )),
                }
            })
        })
        .effect(|invoice: &Invoice, ctx, tx| {
            let (id, balance, recovered) = (
                invoice.id,
                invoice.balance,
                invoice.write_off_recovered_amount,
            );
            Box::pin(async move {
                // The guards passed, so the input is there.
                let input = input(ctx).ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("a recovery run without its input"))
                })?;
                // Only from the balance and the recovered amount this run
                // checked: a concurrent recovery moved them, so this one
                // must not pass as well. Not the status column, so outside
                // the machine's guard. Lines, totals, VAT and the write-off
                // fields stay as they are.
                let moved = renox::db::sql(format!(
                    "UPDATE {} SET allocated_total = allocated_total + ?, balance = balance - ?, \
                     write_off_recovered_amount = write_off_recovered_amount + ?, updated_at = ? \
                     WHERE id = ? AND balance = ? AND write_off_recovered_amount = ?",
                    Invoice::TABLE
                ))
                .bind(input.amount)
                .bind(input.amount)
                .bind(input.amount)
                .bind(renox::db::now())
                .bind(id)
                .bind(balance)
                .bind(recovered)
                .execute(&mut *tx)
                .await?;
                if moved == 0 {
                    return Err(Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "The invoice's balance changed; try again.".to_owned(),
                    ));
                }
                Ok(())
            })
        })
}

/// The run's input, if it is an [`AllocateInput`].
fn input(ctx: &TransitionCtx) -> Option<AllocateInput> {
    ctx.input_as::<AllocateInput>().ok()
}

/// The transition that recovers `amount` on an invoice with `balance`.
fn transition_for(amount: i64, balance: i64) -> &'static str {
    if amount == balance {
        RECOVER_IN_FULL
    } else {
        RECOVER
    }
}

/// What a recovery does to the VAT side, given the relief it finds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReliefPlan {
    /// No relief, or an excluded one, or a share of 0.00 on a claimed one.
    Nothing,
    /// A claimed relief: a `bad_debt_recovery` adjustment for the share.
    Recovery { share: i64 },
    /// An unclaimed relief: its amount becomes `new_amount`; once the
    /// write-off is recovered in full it is excluded as well.
    Reduce { new_amount: i64, exclude: bool },
}

/// The VAT side of a recovery with VAT `share`, for a relief in `status`
/// of `relief_amount`; `fully_recovered` when the balance reaches 0.
fn plan(
    status: Option<AdjustmentStatus>,
    relief_amount: i64,
    share: i64,
    fully_recovered: bool,
) -> ReliefPlan {
    match status {
        Some(AdjustmentStatus::Included) if share > 0 => ReliefPlan::Recovery { share },
        Some(AdjustmentStatus::Pending | AdjustmentStatus::Proposed) => ReliefPlan::Reduce {
            new_amount: (relief_amount - share).max(0),
            exclude: fully_recovered,
        },
        _ => ReliefPlan::Nothing,
    }
}

/// Recovers `amount` fils from `source` (a payment) on the written-off
/// `invoice`, as the system, in one transaction: `recover_in_full` when it
/// recovers the rest of the write-off, else `recover`. `ran_by` is the user
/// whose action caused it. See the module docs for what is written.
///
/// # Errors
///
/// `NotFrom` (422) when the invoice is not `written_off`; `Guard` (422) for
/// an amount ≤ 0 or above what is left to recover ("Only AED X is left to
/// recover on this invoice"); a 422 when another recovery changed the
/// balance first. Nothing is written then.
pub async fn recover(
    state: &AppState,
    invoice: &Invoice,
    source: &Subject,
    amount: i64,
    ran_by: Option<i64>,
) -> std::result::Result<(), TransitionError> {
    let config = config_of(state);
    let on = today();
    // The entity's `open` summary of the day, for a claimed relief's
    // recovery adjustment; read before the transaction.
    let open_summary = match invoice.legal_entity_id {
        Some(id) => match LegalEntity::find(&state.db, id).await? {
            Some(entity) => VatSummary::containing(&state.db, &config, &entity, on)
                .await?
                .filter(|s| s.status == VatSummaryStatus::Open),
            None => None,
        },
        None => None,
    };

    let input = AllocateInput {
        source: source.clone(),
        amount,
        ran_by,
    };
    let mut tx = state.db.begin().await?;
    // Rolled back with the transaction when a guard refuses the run.
    let mut allocation = Allocation::new(invoice.id, source, amount, ran_by);
    allocation.insert(&mut tx).await?;
    let mut meta = json!({ "source": source.to_string(), "allocation_id": allocation.id });
    if let Some(user) = ran_by {
        meta["ran_by"] = json!(user);
    }
    let outcome = invoice
        .transition(transition_for(amount, invoice.balance))
        .by_system()
        .input(&input)
        .meta(meta)
        .run_in(&mut tx)
        .await?;

    let recovered_total = invoice.write_off_recovered_amount + amount;
    let relief = VatAdjustment::query()
        .where_(VatAdjustment::KIND.eq(AdjustmentKind::BadDebtRelief))
        .where_(VatAdjustment::SOURCE_TYPE.eq(invoice.subject().subject_type))
        .where_(VatAdjustment::SOURCE_ID.eq(invoice.id))
        .first(&mut tx)
        .await?;
    let lines = InvoiceLine::query()
        .where_(InvoiceLine::INVOICE_ID.eq(invoice.id))
        .get(&mut tx)
        .await?;
    let amounts: Vec<_> = lines.iter().map(InvoiceLine::amounts).collect();
    let share = recovery_share(
        invoice.write_off_recovered_amount,
        amount,
        &VatTotals::of(&amounts),
    )
    .ok_or_else(|| {
        Error::from(renox::anyhow::anyhow!(
            "invoice {} has no AED VAT figures",
            invoice.id
        ))
    })?;
    let relief_status = relief.as_ref().map(|r| r.status);
    let plan = plan(
        relief_status,
        relief.as_ref().map_or(0, |r| r.amount),
        share,
        amount == invoice.balance,
    );

    let mut outcomes = Vec::new();
    let mut recovery_adjustment_id = None;
    match (plan, &relief) {
        (ReliefPlan::Recovery { share }, _) => {
            let mut adjustment = VatAdjustment {
                legal_entity_id: invoice.legal_entity_id,
                kind: AdjustmentKind::BadDebtRecovery,
                source_type: invoice.subject().subject_type,
                source_id: invoice.id,
                amount: share,
                reason: Some(english("wl_invoicing.adjustment.recovery_reason").to_owned()),
                status: AdjustmentStatus::Pending,
                ..Default::default()
            };
            adjustment.insert(&mut tx).await?;
            recovery_adjustment_id = Some(adjustment.id);
            if let Some(summary) = &open_summary {
                let propose = adjustment
                    .transition(PROPOSE)
                    .by_system()
                    .input(&ProposeInput {
                        summary_id: summary.id,
                    })
                    .run_in(&mut tx)
                    .await?;
                outcomes.push(propose);
            }
        }
        (
            ReliefPlan::Reduce {
                new_amount,
                exclude,
            },
            Some(relief),
        ) => {
            VatAdjustment::query()
                .where_(VatAdjustment::ID.eq(relief.id))
                .update(
                    &mut tx,
                    &[("amount", &new_amount), ("updated_at", &renox::db::now())],
                )
                .await?;
            if exclude {
                outcomes.push(
                    relief
                        .transition(EXCLUDE_RECOVERED)
                        .by_system()
                        .run_in(&mut tx)
                        .await?,
                );
            }
        }
        _ => {}
    }

    let mut row = Recovery {
        invoice_id: invoice.id,
        source_type: source.subject_type.clone(),
        source_id: source.subject_id,
        allocation_id: allocation.id,
        amount,
        recovered_total,
        vat_share: share,
        relief_claimed: relief_status == Some(AdjustmentStatus::Included),
        relief_status,
        relief_adjustment_id: relief.as_ref().map(|r| r.id),
        recovery_adjustment_id,
        recovered_on: on,
        ..Default::default()
    };
    row.insert(&mut tx).await?;
    tx.commit().await?;

    // After the commit.
    for done in outcomes {
        done.emit(state).await.map_err(TransitionError::Failed)?;
    }
    outcome.emit(state).await.map_err(TransitionError::Failed)
}

/// The `allocation_id` a recovery run's history metadata names.
pub(crate) fn allocation_of(meta: &Value) -> Option<i64> {
    meta.get("allocation_id").and_then(Value::as_i64)
}

/// The event for `recovery` of `invoice`.
fn event_for(invoice: &Invoice, recovery: &Recovery) -> BadDebtRecovered {
    BadDebtRecovered {
        invoice: invoice.invoice_id(),
        party: invoice.party(),
        source: recovery.source(),
        amount: recovery.amount,
        relief_claimed: recovery.relief_claimed,
        recovered_on: recovery.recovered_on,
    }
}

/// What the notice says about the VAT side.
fn adjustment_text(recovery: &Recovery, adjustment: Option<&VatAdjustment>) -> String {
    match adjustment {
        Some(adjustment) => format!(
            "Recovery adjustment: {} ({}).",
            amount_text("AED", adjustment.amount),
            format!("{:?}", adjustment.status).to_lowercase()
        ),
        None if recovery.relief_claimed => "No VAT adjustment was needed.".to_owned(),
        None => "The VAT relief was not claimed, so no adjustment was made.".to_owned(),
    }
}

/// The template variables of the notice.
fn data(invoice: &Invoice, recovery: &Recovery, adjustment: Option<&VatAdjustment>) -> Value {
    let party_name = invoice
        .party_snapshot
        .as_ref()
        .map(|snapshot| snapshot.0.legal_name_en.clone())
        .unwrap_or_default();
    json!({
        "number": invoice.number.clone().unwrap_or_default(),
        "party_name": party_name,
        "amount": amount_text(&invoice.currency, recovery.amount),
        "recovered_total": amount_text(&invoice.currency, recovery.recovered_total),
        "written_off": amount_text(&invoice.currency, invoice.write_off_amount.unwrap_or_default()),
        "relief_claimed": if recovery.relief_claimed { "yes" } else { "no" },
        "adjustment": adjustment_text(recovery, adjustment),
        "link": format!("/invoices/{}", invoice.id),
    })
}

/// The notification type [`RECOVERED`]: staff, transactional, by e-mail.
pub fn notification_type() -> NotificationTypeDef {
    NotificationTypeDef::new(RECOVERED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_invoicing.recovered.subject")),
            english("wl_invoicing.recovered.body"),
        )
}

/// Registers [`RECOVERED`] with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(notification_type());
}

/// After the commit of `recover` or `recover_in_full` (run with history
/// metadata `meta`): emits `BadDebtRecovered` and tells the accountants,
/// one notice per recovery.
pub(crate) async fn after_commit(state: &AppState, invoice: &Invoice, meta: &Value) -> Result {
    let Some(allocation_id) = allocation_of(meta) else {
        return Ok(());
    };
    let Some(recovery) = Recovery::query()
        .where_(Recovery::ALLOCATION_ID.eq(allocation_id))
        .first(&state.db)
        .await?
    else {
        return Ok(());
    };
    state.emit(event_for(invoice, &recovery)).await?;
    let adjustment = match recovery.recovery_adjustment_id {
        Some(id) => VatAdjustment::find(&state.db, id).await?,
        None => None,
    };
    notify_accountants(state, invoice, &recovery, adjustment.as_ref()).await
}

/// Tells each holder of the notify role, one message per user.
async fn notify_accountants(
    state: &AppState,
    invoice: &Invoice,
    recovery: &Recovery,
    adjustment: Option<&VatAdjustment>,
) -> Result {
    let config = config_of(state);
    let mut ids: Vec<i64> =
        users_with_role_in(&state.db, &config.relief_notify_role, &Scope::global())
            .await?
            .iter()
            .map(|user| user.id)
            .collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(());
    }
    let mut request =
        NotifyRequest::new(RECOVERED, invoice.subject()).data(data(invoice, recovery, adjustment));
    for id in ids {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::NaiveDate;
    use wl_kernel::PartyId;
    use wl_kernel::invoicing::InvoiceId;

    fn written_off(balance: i64) -> Invoice {
        Invoice {
            id: 4,
            party_id: 7,
            number: Some("INV-2026-0004".into()),
            currency: "AED".into(),
            gross_total: 105_000,
            balance,
            write_off_amount: Some(105_000),
            status: InvoiceStatus::WrittenOff,
            ..Default::default()
        }
    }

    fn run(amount: i64) -> TransitionCtx {
        let input = AllocateInput {
            source: Subject::new("payments.payment", 3),
            amount,
            ran_by: None,
        };
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    fn recovery(claimed: bool) -> Recovery {
        Recovery {
            source_type: "payments.payment".into(),
            source_id: 3,
            allocation_id: 11,
            amount: 52_500,
            recovered_total: 52_500,
            vat_share: 2_500,
            relief_claimed: claimed,
            recovered_on: NaiveDate::from_ymd_opt(2026, 10, 18).unwrap(),
            ..Default::default()
        }
    }

    #[test]
    fn both_go_from_written_off_and_are_system_only() {
        let machine = Invoice::machine();
        for (name, to) in [(RECOVER, "written_off"), (RECOVER_IN_FULL, "paid")] {
            let transition = machine.find(name).expect("transition");
            assert_eq!(transition.from_states(), ["written_off"]);
            assert_eq!(transition.to_state(), Some(to));
            assert!(transition.is_system());
        }
    }

    #[test]
    fn the_resulting_balance_picks_the_transition() {
        assert_eq!(transition_for(52_500, 105_000), RECOVER);
        assert_eq!(transition_for(105_000, 105_000), RECOVER_IN_FULL);
        // Above the balance: `recover`, whose guard refuses it.
        assert_eq!(transition_for(105_001, 105_000), RECOVER);
    }

    #[renox::test]
    async fn the_guards_refuse_nothing_negative_or_too_much() {
        let machine = Invoice::machine();
        let partial = machine.find(RECOVER).expect("recover");
        let full = machine.find(RECOVER_IN_FULL).expect("in full");
        let invoice = written_off(105_000);
        assert!(partial.check_guards(&invoice, &run(52_500)).await.is_ok());
        assert!(full.check_guards(&invoice, &run(105_000)).await.is_ok());
        for amount in [0, -100] {
            assert!(matches!(
                partial.check_guards(&invoice, &run(amount)).await,
                Err(TransitionError::Guard(_))
            ));
        }
        match partial.check_guards(&invoice, &run(105_001)).await {
            Err(TransitionError::Guard(message)) => {
                assert_eq!(
                    message,
                    "Only AED 1,050.00 is left to recover on this invoice."
                );
            }
            other => panic!("expected a guard refusal, got {other:?}"),
        }
        assert!(matches!(
            partial.check_guards(&invoice, &run(105_000)).await,
            Err(TransitionError::Guard(_))
        ));
        assert!(matches!(
            full.check_guards(&invoice, &run(52_500)).await,
            Err(TransitionError::Guard(_))
        ));
        assert!(matches!(
            partial
                .check_guards(&invoice, &TransitionCtx::new(None))
                .await,
            Err(TransitionError::Guard(_))
        ));
    }

    #[test]
    fn recover_is_refused_from_any_other_state() {
        let machine = Invoice::machine();
        let transition = machine.find(RECOVER).expect("recover");
        for status in ["issued", "partially_paid", "overdue", "paid", "credited"] {
            assert!(transition.check_from(status).is_err());
        }
        assert!(transition.check_from("written_off").is_ok());
    }

    #[test]
    fn the_relief_decides_the_vat_side() {
        use AdjustmentStatus::{Excluded, Included, Pending, Proposed};
        assert_eq!(
            plan(Some(Included), 5_000, 2_500, false),
            ReliefPlan::Recovery { share: 2_500 }
        );
        assert_eq!(plan(Some(Included), 5_000, 0, false), ReliefPlan::Nothing);
        for status in [Pending, Proposed] {
            assert_eq!(
                plan(Some(status), 5_000, 2_500, false),
                ReliefPlan::Reduce {
                    new_amount: 2_500,
                    exclude: false
                }
            );
            assert_eq!(
                plan(Some(status), 2_500, 2_500, true),
                ReliefPlan::Reduce {
                    new_amount: 0,
                    exclude: true
                }
            );
        }
        assert_eq!(
            plan(Some(Excluded), 5_000, 2_500, true),
            ReliefPlan::Nothing
        );
        assert_eq!(plan(None, 0, 0, true), ReliefPlan::Nothing);
    }

    #[test]
    fn the_event_carries_the_source_amount_and_day() {
        let event = event_for(&written_off(52_500), &recovery(true));
        assert_eq!(event.invoice, InvoiceId(4));
        assert_eq!(event.party, PartyId(7));
        assert_eq!(event.source, Subject::new("payments.payment", 3));
        assert_eq!(event.amount, 52_500);
        assert!(event.relief_claimed);
        assert!(!event_for(&written_off(52_500), &recovery(false)).relief_claimed);
    }

    #[test]
    fn the_notice_names_the_amounts_and_the_adjustment() {
        let invoice = written_off(52_500);
        let adjustment = VatAdjustment {
            amount: 2_500,
            status: AdjustmentStatus::Proposed,
            ..Default::default()
        };
        let claimed = data(&invoice, &recovery(true), Some(&adjustment));
        assert_eq!(claimed["number"], "INV-2026-0004");
        assert_eq!(claimed["amount"], "AED 525.00");
        assert_eq!(claimed["recovered_total"], "AED 525.00");
        assert_eq!(claimed["written_off"], "AED 1,050.00");
        assert_eq!(claimed["relief_claimed"], "yes");
        assert_eq!(
            claimed["adjustment"],
            "Recovery adjustment: AED 25.00 (proposed)."
        );
        assert_eq!(claimed["link"], "/invoices/4");
        let unclaimed = data(&invoice, &recovery(false), None);
        assert_eq!(unclaimed["relief_claimed"], "no");
        assert!(
            unclaimed["adjustment"]
                .as_str()
                .unwrap()
                .contains("not claimed")
        );
    }

    #[test]
    fn the_notification_type_and_run_metadata() {
        assert_eq!(notification_type().key, RECOVERED);
        assert_eq!(allocation_of(&json!({ "allocation_id": 11 })), Some(11));
        assert_eq!(allocation_of(&Value::Null), None);
    }
}
