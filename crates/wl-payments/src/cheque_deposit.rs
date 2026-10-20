//! Putting a cheque on the deposit list, depositing it and clearing it
//! (wl-payments §4 `flag_due`, `deposit`, `clear`, §5.3 steps 2 and 3,
//! `FIN.RUL.03`).
//!
//! - **`flag_due`** (the system, [`flag_due_run`], every hour): a `received`
//!   received cheque whose date is within the lead time (1 day) goes to
//!   `due_for_deposit`.
//! - **`deposit`** (`cheques.deposit`): `received` or `due_for_deposit` to
//!   `deposited` once the cheque date is reached; a cheque collected in the
//!   field is deposited only after a cash handover confirmed it
//!   (`FIN.ACL.20`, on by default). Presentations + 1.
//! - **`clear`** (`cheques.deposit`): `deposited` to `cleared`. A payment
//!   (money in for a received cheque, out for an issued one) is recorded for
//!   the open amount in the same transaction and allocated to the documents
//!   the cheque pays after the commit. After a re-presentation
//!   `ChequeBounceResolved` (cleared) follows ([`crate::cheque_events`]).

use renox::chrono::{NaiveDate, TimeDelta};
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::cheques::ChequeStatus;
use wl_kernel::documents::today;
use wl_kernel::payments::PaymentMethod;

use crate::cheque::Cheque;
use crate::cheque_money;
use crate::config_of;
use crate::permissions;

/// The `flag_due` transition's name.
pub const FLAG_DUE: &str = "flag_due";
/// The `deposit` transition's name.
pub const DEPOSIT: &str = "deposit";
/// The `clear` transition's name.
pub const CLEAR: &str = "clear";

/// The rule id of the deposit list.
pub const RULE: &str = "FIN.RUL.03";

/// The scheduled task and command.
pub const TASK: &str = "payments:cheque_flag_due";

/// What a `flag_due` run carries to its guard.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct FlagDueInput {
    /// Days before the cheque date that it goes on the list.
    pub lead_days: i64,
    /// Today (Asia/Dubai).
    pub today: NaiveDate,
}

/// What a `deposit` run carries to its guards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DepositInput {
    /// Whether a field cheque needs a handover first (`FIN.ACL.20`).
    pub field_confirmation: bool,
}

/// Why `cheque` is not due for the list on `today` with `lead_days`, if it
/// is not.
pub(crate) fn flag_problem(cheque: &Cheque, input: Option<&FlagDueInput>) -> Option<String> {
    let Some(input) = input else {
        return Some("The action was not prepared by the cheque service.".to_owned());
    };
    if !cheque.is_received() {
        return Some("Only a received cheque goes on the deposit list.".to_owned());
    }
    if cheque.is_evidence() {
        return Some("A cheque held as evidence never goes on the deposit list.".to_owned());
    }
    if cheque.cheque_date > input.today + TimeDelta::days(input.lead_days) {
        return Some("The cheque date is not close yet.".to_owned());
    }
    None
}

/// Why `cheque` cannot be deposited on `today`, if it cannot: its date is
/// not reached, or it is a field cheque no handover confirmed
/// (`FIN.ACL.20`). `field_confirmation` says whether that rule is on.
pub(crate) fn deposit_problem(
    cheque: &Cheque,
    today: NaiveDate,
    field_confirmation: bool,
) -> Option<String> {
    if cheque.is_evidence() {
        return Some("A cheque held as evidence is never deposited.".to_owned());
    }
    if cheque.cheque_date > today {
        return Some(format!(
            "The cheque is dated {}: it cannot be deposited before.",
            cheque.cheque_date.format("%Y-%m-%d")
        ));
    }
    if field_confirmation
        && cheque.is_received()
        && cheque.field_collected
        && cheque.custody_handover_id.is_none()
    {
        return Some(
            "A cheque collected in the field is deposited only after a cash handover confirms it (FIN.ACL.20)."
                .to_owned(),
        );
    }
    None
}

fn deposit_input(ctx: &TransitionCtx) -> DepositInput {
    // Without the gathered input the rule is on.
    ctx.input_as::<DepositInput>().unwrap_or(DepositInput {
        field_confirmation: true,
    })
}

/// `flag_due`, `deposit` and `clear`, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Cheque>> {
    vec![
        Transition::new(FLAG_DUE)
            .label("Put on the deposit list")
            .system()
            .from([ChequeStatus::Received])
            .to(ChequeStatus::DueForDeposit)
            .guard("date_within_lead", |cheque: &Cheque, ctx| {
                let problem = flag_problem(cheque, ctx.input_as::<FlagDueInput>().ok().as_ref());
                Box::pin(async move { problem.map_or(Ok(()), Err) })
            }),
        Transition::new(DEPOSIT)
            .label("Deposit")
            .tone("primary")
            .permission(permissions::CHEQUES_DEPOSIT)
            .from([ChequeStatus::Received, ChequeStatus::DueForDeposit])
            .to(ChequeStatus::Deposited)
            .guard(
                "date_reached_and_custody_confirmed",
                |cheque: &Cheque, ctx| {
                    let problem =
                        deposit_problem(cheque, today(), deposit_input(ctx).field_confirmation);
                    Box::pin(async move { problem.map_or(Ok(()), Err) })
                },
            )
            .effect(|cheque: &Cheque, _ctx, tx| {
                let id = cheque.id;
                let presentations = cheque.presentations + 1;
                Box::pin(async move {
                    // Not the status column, so outside the machine's guard.
                    Cheque::query()
                        .where_(Cheque::ID.eq(id))
                        .update(
                            &mut *tx,
                            &[
                                ("presentations", &presentations),
                                ("deposit_date", &Some(today())),
                                ("updated_at", &renox::db::now()),
                            ],
                        )
                        .await?;
                    Ok(())
                })
            }),
        Transition::new(CLEAR)
            .label("Clear")
            .tone("primary")
            .permission(permissions::CHEQUES_DEPOSIT)
            .from([ChequeStatus::Deposited])
            .to(ChequeStatus::Cleared)
            .guard("something_open", |cheque: &Cheque, _ctx| {
                let open = cheque.open_amount;
                Box::pin(async move {
                    if open > 0 {
                        Ok(())
                    } else {
                        Err("The cheque has nothing open to clear.".to_owned())
                    }
                })
            })
            .effect(|cheque: &Cheque, _ctx, tx| {
                let id = cheque.id;
                Box::pin(async move {
                    Cheque::query()
                        .where_(Cheque::ID.eq(id))
                        .update(
                            &mut *tx,
                            &[("open_amount", &0_i64), ("updated_at", &renox::db::now())],
                        )
                        .await?;
                    Ok(())
                })
            }),
    ]
}

async fn reload(state: &AppState, cheque: &Cheque) -> std::result::Result<Cheque, TransitionError> {
    Cheque::find_or_404(&state.db, cheque.id)
        .await
        .map_err(TransitionError::Failed)
}

/// `user` deposits the `received` or `due_for_deposit` `cheque`.
///
/// # Errors
///
/// `Forbidden` without `cheques.deposit`; `NotFrom` in another state;
/// `Guard` (422) before the cheque date, or for a field cheque no handover
/// confirmed.
pub async fn deposit(
    state: &AppState,
    user: &AuthUser,
    cheque: &Cheque,
) -> std::result::Result<Cheque, TransitionError> {
    let cheque = reload(state, cheque).await?;
    let input = DepositInput {
        field_confirmation: config_of(state).field_cheque_handover,
    };
    let _outcome = cheque
        .transition(DEPOSIT)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    reload(state, &cheque).await
}

/// `user` clears the `deposited` `cheque`: it becomes `cleared`, and in the
/// same transaction a payment (cheque method, money in for a received cheque,
/// out for an issued one) is recorded for the open amount. The payment is
/// allocated to the documents the cheque pays after the commit.
///
/// # Errors
///
/// `Forbidden` without `cheques.deposit`; `NotFrom` unless `deposited`;
/// `Guard` (422) when the payment is refused (the cheque method is not
/// enabled, the legal entity is not active).
pub async fn clear(
    state: &AppState,
    user: &AuthUser,
    cheque: &Cheque,
) -> std::result::Result<Cheque, TransitionError> {
    let cheque = reload(state, cheque).await?;
    let request =
        cheque_money::payment_for(&cheque, cheque.open_amount, PaymentMethod::Cheque, None);
    let mut prepared = cheque_money::prepare(state, &request, Some(user.id)).await?;
    // The cheque of an approved owner drawing records the drawing.
    let drawing = crate::drawings::for_cheque(&state.db, &cheque).await?;
    if let Some(row) = &drawing {
        cheque_money::mark_drawing(&mut prepared, row);
    }
    let mut tx = state.db.begin().await?;
    let outcome = cheque.transition(CLEAR).by(user).run_in(&mut tx).await?;
    let (payment, received) = cheque_money::record_in(&mut tx, prepared).await?;
    if let Some(mut row) = drawing {
        row.status = crate::drawings::DrawingStatus::Recorded;
        row.payment_id = Some(payment.id);
        row.save(&mut tx).await?;
    }
    tx.commit().await?;
    outcome.emit(state).await.map_err(TransitionError::Failed)?;
    received
        .emit(state)
        .await
        .map_err(TransitionError::Failed)?;
    cheque_money::allocate_to_links(state, &cheque, &payment, payment.amount, Some(user.id))
        .await?;
    reload(state, &cheque).await
}

/// `FIN.RUL.03`: every `received` received cheque dated within the lead time
/// (1 day) of `today` goes on the deposit list. Returns how many.
///
/// # Errors
///
/// The database's errors.
pub async fn flag_due_run(state: &AppState, today: NaiveDate) -> Result<usize> {
    let lead_days = config_of(state).cheque_lead_days;
    let due = Cheque::query()
        .where_(Cheque::STATUS.eq(ChequeStatus::Received))
        .where_(Cheque::DIRECTION.eq(wl_kernel::payments::Direction::In))
        .where_(Cheque::EVIDENCE_NOTE.is_null())
        .where_(Cheque::CHEQUE_DATE.lte(today + TimeDelta::days(lead_days)))
        .order_by(Cheque::CHEQUE_DATE)
        .get(&state.db)
        .await?;
    let input = FlagDueInput { lead_days, today };
    let mut flagged = 0;
    for cheque in due {
        match cheque
            .transition(FLAG_DUE)
            .by_system()
            .input(&input)
            .run(state)
            .await
        {
            Ok(_) => flagged += 1,
            // Someone deposited it first.
            Err(TransitionError::NotFrom(_) | TransitionError::Conflict) => {}
            Err(other) => return Err(other.into_error()),
        }
    }
    Ok(flagged)
}

/// Registers the scheduled task and its command.
pub(crate) fn register(app: &mut Registry) {
    app.schedule().hourly(TASK, |state: AppState| async move {
        flag_due_run(&state, today()).await.map(|_| ())
    });
    app.command(
        TASK,
        "Put the cheques whose date is close on the deposit list (FIN.RUL.03)",
        |_args, state: AppState| async move {
            let flagged = flag_due_run(&state, today()).await?;
            println!("{flagged} cheques put on the deposit list.");
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::state_machine::StateMachine;
    use wl_kernel::payments::Direction;

    fn day(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, day).unwrap()
    }

    fn cheque(date: u32) -> Cheque {
        Cheque {
            id: 1,
            cheque_date: day(date),
            amount: 2_000_000,
            open_amount: 2_000_000,
            ..Default::default()
        }
    }

    #[test]
    fn the_transitions_leave_their_states() {
        let machine = Cheque::machine();
        let found = machine.find(FLAG_DUE).expect("transition");
        assert!(found.is_system());
        assert!(found.check_from(ChequeStatus::Received).is_ok());
        assert!(found.check_from(ChequeStatus::DueForDeposit).is_err());
        let found = machine.find(DEPOSIT).expect("transition");
        assert!(found.check_from(ChequeStatus::Received).is_ok());
        assert!(found.check_from(ChequeStatus::DueForDeposit).is_ok());
        assert!(found.check_from(ChequeStatus::Deposited).is_err());
        let found = machine.find(CLEAR).expect("transition");
        assert!(found.check_from(ChequeStatus::Deposited).is_ok());
        assert!(found.check_from(ChequeStatus::Bounced).is_err());
    }

    #[test]
    fn a_cheque_is_on_the_list_one_day_before_its_date() {
        let input = FlagDueInput {
            lead_days: 1,
            today: day(2),
        };
        assert_eq!(flag_problem(&cheque(3), Some(&input)), None);
        assert!(flag_problem(&cheque(4), Some(&input)).is_some());
        assert!(flag_problem(&cheque(3), None).is_some());
        let issued = Cheque {
            direction: Direction::Out,
            ..cheque(3)
        };
        assert!(flag_problem(&issued, Some(&input)).is_some());
    }

    #[test]
    fn a_cheque_is_deposited_from_its_date() {
        assert!(deposit_problem(&cheque(3), day(2), true).is_some());
        assert_eq!(deposit_problem(&cheque(3), day(3), true), None);
        assert_eq!(deposit_problem(&cheque(3), day(9), true), None);
    }

    #[test]
    fn a_field_cheque_waits_for_its_handover() {
        let field = Cheque {
            field_collected: true,
            collected_by: Some(9),
            ..cheque(3)
        };
        let message = deposit_problem(&field, day(5), true).expect("refused");
        assert!(message.contains("FIN.ACL.20"), "{message}");
        assert_eq!(deposit_problem(&field, day(5), false), None, "rule off");
        let confirmed = Cheque {
            custody_handover_id: Some(4),
            ..field.clone()
        };
        assert_eq!(deposit_problem(&confirmed, day(5), true), None);
        let issued = Cheque {
            direction: Direction::Out,
            ..field
        };
        assert_eq!(deposit_problem(&issued, day(5), true), None);
    }

    #[renox::test]
    async fn the_deposit_guard_refuses_a_future_cheque() {
        let machine = Cheque::machine();
        let found = machine.find(DEPOSIT).expect("transition");
        let far = Cheque {
            cheque_date: today() + TimeDelta::days(30),
            ..cheque(3)
        };
        let ctx = TransitionCtx::new(None);
        assert!(matches!(
            found.check_guards(&far, &ctx).await,
            Err(TransitionError::Guard(_))
        ));
        let past = Cheque {
            cheque_date: today() - TimeDelta::days(1),
            ..cheque(3)
        };
        assert!(found.check_guards(&past, &ctx).await.is_ok());
    }
}
