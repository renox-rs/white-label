//! Renewing a client contract (09 Manpower §4.1 row *The contract expires, or
//! ends early*, §7.1 *Client contract* `renew`, `MPW.ACL.16`; BL-MPW-05).
//!
//! A contract must not run out unnoticed with workers on site: `MPW.RUL.11`
//! warns 60 and 30 days before the end date ([`crate::expiry`]) and the
//! account manager (`contracts.manage`) renews it with a new end date and the
//! renewal signed by the client (`DOC.SIG.01`).
//!
//! - **The guard** ([`renew_problem`]) lists every failing check: the new end
//!   date is later than the current one; the signed renewal is recorded (the
//!   day the client signed, not in the future, and the document); and, when
//!   the rates change, the new rate card version accepted by the client and
//!   effective from the date given (through the kernel's `RateCards`; the
//!   existing version is never edited, `MPW.ACL.16`). The date of the new
//!   rates is after the start of the contract and not after the new end date.
//! - **A blocked client** (`PartyBlocked`) cannot renew: [`renew`] asks
//!   [`crate::blocked::ensure`] with `ActionKind::Renew` before anything else.
//! - **The states.** `active` and `on_hold` only (`terminating` is not offered,
//!   the spec lists these two). The contract stays in its state: `renew` for
//!   `active`, `renew_on_hold` for `on_hold`, the pair a Renox transition with
//!   one target needs.
//! - **The effect** ([`renew_effects`]): the end date, the day and the user, the
//!   signed renewal, the end date before and the date of the new rates are
//!   kept. **Deployment end dates are not changed**: the next notice of
//!   `MPW.RUL.11` lists them for `extend`.
//! - **After the commit** a request to terminate the contract that is still
//!   waiting for the MD is withdrawn
//!   ([`crate::termination::withdraw_pending`]) and [`ContractRenewed`] is
//!   emitted.
//!
//! Decision: the rates change is declared with `rates_from`, the effective date
//! of the new rate card version; without it the rates are those of the
//! version in force, which keeps pricing the hours.

use renox::chrono::NaiveDate;
use renox::db::{Transaction, now};
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::block_guard::ActionKind;
use wl_kernel::documents::today;
use wl_kernel::rate_cards::rate_cards;

use crate::blocked;
use crate::contract::{ClientContract, ContractStatus, RENEW, RENEW_ON_HOLD};
use crate::db::{answer, refused, set_contract};
use crate::events::ContractRenewed;
use crate::termination::withdraw_pending;
use crate::texts::{english, refusal};

/// What a `renew` run carries to its guard and effect.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenewInput {
    /// The new end date.
    #[serde(default)]
    pub end_date: Option<NaiveDate>,
    /// The day the client signed the renewal.
    #[serde(default)]
    pub signed_on: Option<NaiveDate>,
    /// The signed renewal, a document reference.
    #[serde(default)]
    pub document: Option<String>,
    /// The effective date of the new rate card version, when the rates change.
    #[serde(default)]
    pub rates_from: Option<NaiveDate>,
    /// The checks the service made against other modules (the rate card).
    #[serde(default)]
    pub problems: Vec<String>,
    /// The business day of the run; today when empty.
    #[serde(default)]
    pub on: Option<NaiveDate>,
    /// The user who renews.
    #[serde(default)]
    pub requested_by: Option<i64>,
}

/// The input of a `renew` run.
pub(crate) fn renew_input(ctx: &TransitionCtx) -> RenewInput {
    ctx.input_as::<RenewInput>().unwrap_or_default()
}

/// Every reason `contract` cannot be renewed with `input`, one English line
/// each; `None` when it can.
pub fn renew_problem(contract: &ClientContract, input: &RenewInput) -> Option<String> {
    let mut problems = Vec::new();
    match input.end_date {
        Some(end) if end > contract.end_date => {}
        Some(_) => problems.push(refusal(
            "manpower.renew.later",
            &[("end", &contract.end_date)],
        )),
        None => problems.push(refusal("manpower.renew.no_date", &[])),
    }
    let on = input.on.unwrap_or_else(today);
    let document = input
        .document
        .as_deref()
        .is_some_and(|document| !document.trim().is_empty());
    match input.signed_on {
        Some(signed) if document && signed <= on => {}
        _ => problems.push(refusal("manpower.renew.not_signed", &[])),
    }
    if let (Some(from), Some(end)) = (input.rates_from, input.end_date)
        && (from <= contract.start_date || from > end)
    {
        problems.push(refusal(
            "manpower.renew.rates_date",
            &[("start", &contract.start_date), ("end", &end)],
        ));
    }
    problems.extend(input.problems.iter().cloned());
    (!problems.is_empty()).then(|| problems.join("\n"))
}

/// The effect of `renew` inside its transaction (see the module).
pub(crate) async fn renew_effects(
    tx: &mut Transaction,
    contract: &ClientContract,
    input: &RenewInput,
) -> Result {
    let Some(end) = input.end_date else {
        return Err(refused(refusal("manpower.renew.no_date", &[])));
    };
    let document = input.document.as_deref().map(str::trim).map(str::to_owned);
    set_contract(
        &mut *tx,
        contract.id,
        &[
            ("end_date", &end),
            ("renewal_previous_end_date", &Some(contract.end_date)),
            ("renewed_at", &Some(now())),
            ("renewed_by", &input.requested_by),
            ("renewal_document", &document),
            ("renewal_signed_on", &input.signed_on),
            ("renewal_rates_from", &input.rates_from),
        ],
    )
    .await
}

/// What the account manager enters to renew a contract.
#[derive(Debug, Clone)]
pub struct RenewAsk {
    /// The new end date.
    pub end_date: NaiveDate,
    /// The day the client signed the renewal.
    pub signed_on: NaiveDate,
    /// The signed renewal, a document reference.
    pub document: String,
    /// The effective date of the new rate card version, when the rates change.
    pub rates_from: Option<NaiveDate>,
}

/// The checks that need another module: when the rates change, a version of the
/// contract's rate card accepted by the client takes effect on `rates_from`.
async fn rate_problems(
    state: &AppState,
    contract: &ClientContract,
    rates_from: Option<NaiveDate>,
) -> Result<Vec<String>> {
    let Some(from) = rates_from else {
        return Ok(Vec::new());
    };
    let version = rate_cards(state)
        .accepted_version(state, &contract.subject(), from)
        .await?;
    if version.is_some_and(|version| version.effective_from == from) {
        Ok(Vec::new())
    } else {
        Ok(vec![english(
            "manpower.renew.rate_version",
            &[("date", &from)],
        )])
    }
}

/// `by` renews contract `id`: a later end date, the signed renewal and, for
/// changed rates, a new rate card version effective from `rates_from`. The
/// contract stays `active` or `on_hold`.
///
/// # Errors
///
/// `Forbidden` without `contracts.manage` for the client; a 422 listing every
/// failing check, for a blocked client, or unless the contract is `active` or
/// `on_hold`.
pub async fn renew(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    ask: RenewAsk,
) -> Result<ClientContract> {
    let contract = ClientContract::find_or_404(&state.db, id).await?;
    if !by.has_permission_in(crate::permissions::CONTRACTS_MANAGE, &contract.scope()) {
        return Err(Error::Forbidden);
    }
    blocked::ensure(state, Some(by), contract.client(), ActionKind::Renew).await?;
    let input = RenewInput {
        end_date: Some(ask.end_date),
        signed_on: Some(ask.signed_on),
        document: Some(ask.document.trim().to_owned()),
        rates_from: ask.rates_from,
        problems: rate_problems(state, &contract, ask.rates_from).await?,
        on: Some(today()),
        requested_by: Some(by.id),
    };
    let name = if contract.status == ContractStatus::OnHold {
        RENEW_ON_HOLD
    } else {
        RENEW
    };
    let history = english("manpower.renew.history", &[("end", &ask.end_date)]);
    let _outcome = contract
        .transition(name)
        .by(by)
        .reason(history)
        .input(&input)
        .run(state)
        .await
        .map_err(answer)?;
    let renewed = ClientContract::find_or_404(&state.db, id).await?;
    withdraw_pending(
        state,
        &renewed,
        &english("manpower.renew.withdraw_reason", &[]),
    )
    .await?;
    state
        .emit(ContractRenewed {
            contract: renewed.subject(),
            client: renewed.client(),
            previous_end_date: contract.end_date,
            end_date: renewed.end_date,
        })
        .await?;
    Ok(renewed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    fn contract() -> ClientContract {
        ClientContract {
            start_date: day(1, 1),
            end_date: day(12, 31),
            ..ClientContract::default()
        }
    }

    fn input(end: NaiveDate) -> RenewInput {
        RenewInput {
            end_date: Some(end),
            signed_on: Some(day(11, 20)),
            document: Some("RENEWAL.pdf".into()),
            on: Some(day(11, 25)),
            ..RenewInput::default()
        }
    }

    #[test]
    fn the_new_end_date_must_be_later_than_the_current_one() {
        let later = NaiveDate::from_ymd_opt(2027, 12, 31).unwrap();
        assert_eq!(renew_problem(&contract(), &input(later)), None);
        assert!(renew_problem(&contract(), &input(day(12, 15))).is_some());
        assert!(renew_problem(&contract(), &input(day(12, 31))).is_some());
    }

    #[test]
    fn the_renewal_must_be_signed_with_a_document_on_a_day_that_has_come() {
        let later = NaiveDate::from_ymd_opt(2027, 12, 31).unwrap();
        let mut unsigned = input(later);
        unsigned.document = Some(" ".into());
        assert!(renew_problem(&contract(), &unsigned).is_some());
        let mut future = input(later);
        future.signed_on = Some(day(12, 1));
        assert!(renew_problem(&contract(), &future).is_some());
        let mut none = input(later);
        none.signed_on = None;
        assert!(renew_problem(&contract(), &none).is_some());
    }

    #[test]
    fn the_new_rates_start_inside_the_contract_and_every_problem_is_listed() {
        let later = NaiveDate::from_ymd_opt(2027, 12, 31).unwrap();
        let mut rates = input(later);
        rates.rates_from = Some(day(1, 1));
        assert!(renew_problem(&contract(), &rates).is_some());
        rates.rates_from = Some(NaiveDate::from_ymd_opt(2027, 1, 1).unwrap());
        assert_eq!(renew_problem(&contract(), &rates), None);
        let mut all = input(day(12, 15));
        all.document = None;
        all.problems = vec!["no rate card".into()];
        assert_eq!(renew_problem(&contract(), &all).unwrap().lines().count(), 3);
    }
}
