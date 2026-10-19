//! Re-presenting a bounced cheque (wl-payments §4 `represent`, §5.3 step 4,
//! issue #103).
//!
//! When the party confirms there are funds, a user with `cheques.deposit`
//! deposits a bounced received cheque again: `bounced` to `deposited`, with
//! the new presentation date and the party's confirmation of funds or a
//! note. Guards:
//!
//! - a received cheque (an issued one is replaced);
//! - nothing honoured by the bank on an earlier presentation (`⚠ open:
//!   lawyer L-21`: whether a partly honoured cheque may be presented again;
//!   the default forbids it). A part settled meanwhile (`settle_part`) does
//!   not prevent it: `clear` then records the open amount only;
//! - presentations below the maximum (2 unless set), **checked before the
//!   count goes up**: the first deposit makes 1, so a maximum of 2 allows one
//!   re-presentation and the next one is refused;
//! - a presentation date that is not in the past (the cheque is deposited by
//!   then; a future date is how the party's agreed day is recorded);
//! - the party's confirmation of funds, or a note.
//!
//! The effect: presentations + 1 and the new deposit date. The party is told
//! the new presentation date after the commit ([`crate::cheque_events`]); the
//! follow-up task of the bounce is closed. A cheque that clears after a
//! re-presentation emits `ChequeBounceResolved` (cleared) from `clear`.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::cheques::ChequeStatus;
use wl_kernel::documents::today;

use crate::cheque::Cheque;
use crate::config::DEFAULT_CHEQUE_MAX_PRESENTATIONS;
use crate::config_of;
use crate::followup::{FollowupKind, FollowupTask};
use crate::notices;
use crate::permissions;

/// The `represent` transition's name.
pub const REPRESENT: &str = "represent";

/// What a user gives to re-present a cheque.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepresentRequest {
    /// The new presentation date.
    pub date: NaiveDate,
    /// Whether the party confirmed there are funds.
    pub funds_confirmed: bool,
    /// A note, e.g. how the funds were confirmed.
    pub note: Option<String>,
}

/// What a `represent` run carries to its guards and effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RepresentInput {
    /// The new presentation date.
    pub date: NaiveDate,
    /// Whether the party confirmed funds.
    pub funds_confirmed: bool,
    /// The note, trimmed, if any.
    pub note: Option<String>,
    /// The most presentations of one cheque.
    pub max_presentations: i64,
}

fn represent_input(ctx: &TransitionCtx) -> Option<RepresentInput> {
    ctx.input_as::<RepresentInput>().ok()
}

/// Why `cheque` cannot be re-presented with `input` on `today`, if it cannot.
pub(crate) fn represent_problem(
    cheque: &Cheque,
    input: Option<&RepresentInput>,
    today: NaiveDate,
) -> Option<String> {
    if !cheque.is_received() {
        return Some("An issued cheque is replaced, not presented again.".to_owned());
    }
    if cheque.honoured_amount > 0 {
        return Some(
            "The bank paid part of this cheque: it cannot be presented again for the rest."
                .to_owned(),
        );
    }
    let max = input.map_or(DEFAULT_CHEQUE_MAX_PRESENTATIONS, |i| i.max_presentations);
    // Checked before the count goes up.
    if i64::from(cheque.presentations) >= max {
        return Some(format!(
            "The cheque was presented {} time(s): at most {max} are allowed.",
            cheque.presentations
        ));
    }
    let Some(input) = input else {
        return Some("The new presentation date is required.".to_owned());
    };
    if input.date < today {
        return Some("The new presentation date cannot be in the past.".to_owned());
    }
    if !input.funds_confirmed && input.note.as_deref().is_none_or(|n| n.trim().is_empty()) {
        return Some("Confirm the party has funds, or leave a note.".to_owned());
    }
    None
}

/// `represent`, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Cheque>> {
    vec![
        Transition::new(REPRESENT)
            .label("Present again")
            .tone("primary")
            .permission(permissions::CHEQUES_DEPOSIT)
            .from([ChequeStatus::Bounced])
            .to(ChequeStatus::Deposited)
            .guard("may_be_presented_again", |cheque: &Cheque, ctx| {
                let problem = represent_problem(cheque, represent_input(ctx).as_ref(), today());
                Box::pin(async move { problem.map_or(Ok(()), Err) })
            })
            .effect(|cheque: &Cheque, ctx, tx| {
                let id = cheque.id;
                let presentations = cheque.presentations + 1;
                let input = represent_input(ctx);
                Box::pin(async move {
                    let input = input.ok_or_else(|| {
                        Error::from(renox::anyhow::anyhow!("a re-presentation without its input"))
                    })?;
                    // Not the status column, so outside the machine's guard.
                    Cheque::query()
                        .where_(Cheque::ID.eq(id))
                        .update(
                            &mut *tx,
                            &[
                                ("presentations", &presentations),
                                ("deposit_date", &Some(input.date)),
                                ("updated_at", &renox::db::now()),
                            ],
                        )
                        .await?;
                    Ok(())
                })
            }),
    ]
}

/// `user` presents the bounced received `cheque` again ([`RepresentRequest`]).
///
/// # Errors
///
/// `Forbidden` without `cheques.deposit`; `NotFrom` unless `bounced`; `Guard`
/// (422) for an issued cheque, one the bank paid in part, one presented the
/// most times already, a date in the past, or neither a confirmation of funds
/// nor a note.
pub async fn represent(
    state: &AppState,
    user: &AuthUser,
    cheque: &Cheque,
    request: &RepresentRequest,
) -> std::result::Result<Cheque, TransitionError> {
    let cheque = Cheque::find_or_404(&state.db, cheque.id)
        .await
        .map_err(TransitionError::Failed)?;
    let input = RepresentInput {
        date: request.date,
        funds_confirmed: request.funds_confirmed,
        note: request
            .note
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_owned),
        max_presentations: config_of(state).cheque_max_presentations,
    };
    let reason = input.note.clone();
    let mut run = cheque.transition(REPRESENT).by(user).input(&input);
    if let Some(note) = reason {
        run = run.reason(note);
    }
    let _outcome = run.run(state).await?;
    Cheque::find_or_404(&state.db, cheque.id)
        .await
        .map_err(TransitionError::Failed)
}

/// After a committed `represent` of `cheque_id`: the follow-up task of the
/// bounce is closed and the party is told the new presentation date.
///
/// # Errors
///
/// The database's errors, or a notice that failed.
pub(crate) async fn after_represent(state: &AppState, cheque_id: i64) -> Result<()> {
    let cheque = Cheque::find_or_404(&state.db, cheque_id).await?;
    FollowupTask::close_for_cheque(&state.db, cheque.id, &[FollowupKind::FollowUpCheque]).await?;
    notices::cheque_presented(state, &cheque).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2027, 2, day).unwrap()
    }

    fn bounced() -> Cheque {
        Cheque {
            id: 1,
            amount: 2_000_000,
            open_amount: 2_000_000,
            presentations: 1,
            status: ChequeStatus::Bounced,
            ..Default::default()
        }
    }

    fn input(date: NaiveDate, funds: bool, note: Option<&str>) -> RepresentInput {
        RepresentInput {
            date,
            funds_confirmed: funds,
            note: note.map(str::to_owned),
            max_presentations: 2,
        }
    }

    #[test]
    fn represent_leaves_bounced_only() {
        let machine = Cheque::machine();
        let found = machine.find(REPRESENT).expect("transition");
        assert!(found.check_from(ChequeStatus::Bounced).is_ok());
        for other in [
            ChequeStatus::LegalCase,
            ChequeStatus::Deposited,
            ChequeStatus::Received,
            ChequeStatus::Replaced,
        ] {
            assert!(found.check_from(other).is_err(), "{other:?}");
        }
        assert_eq!(found.to_state(), Some("deposited"));
    }

    #[test]
    fn a_bounced_cheque_is_presented_again_once_with_the_default_maximum() {
        let ok = input(day(5), false, Some("funds confirmed by phone"));
        assert_eq!(represent_problem(&bounced(), Some(&ok), day(1)), None);
        let second = Cheque {
            presentations: 2,
            ..bounced()
        };
        let message = represent_problem(&second, Some(&ok), day(1)).expect("refused");
        assert!(message.contains("at most 2"), "{message}");
        // A higher maximum allows it.
        let mut more = input(day(5), true, None);
        more.max_presentations = 3;
        assert_eq!(represent_problem(&second, Some(&more), day(1)), None);
    }

    #[test]
    fn a_partly_honoured_or_issued_cheque_is_not_presented_again() {
        let ok = input(day(5), true, None);
        let honoured = Cheque {
            honoured_amount: 600_000,
            open_amount: 1_400_000,
            ..bounced()
        };
        assert!(represent_problem(&honoured, Some(&ok), day(1)).is_some());
        let issued = Cheque {
            direction: wl_kernel::payments::Direction::Out,
            ..bounced()
        };
        assert!(represent_problem(&issued, Some(&ok), day(1)).is_some());
        // A part settled meanwhile does not prevent it.
        let settled = Cheque {
            settled_amount: 400_000,
            open_amount: 1_600_000,
            ..bounced()
        };
        assert_eq!(represent_problem(&settled, Some(&ok), day(1)), None);
    }

    #[test]
    fn the_date_and_the_confirmation_or_note_are_required() {
        assert!(represent_problem(&bounced(), None, day(1)).is_some());
        let past = input(day(1), true, None);
        assert!(represent_problem(&bounced(), Some(&past), day(2)).is_some());
        let today_ok = input(day(2), true, None);
        assert_eq!(represent_problem(&bounced(), Some(&today_ok), day(2)), None);
        let neither = input(day(5), false, None);
        assert!(represent_problem(&bounced(), Some(&neither), day(1)).is_some());
        let blank = input(day(5), false, Some("  "));
        assert!(represent_problem(&bounced(), Some(&blank), day(1)).is_some());
        let note_only = input(day(5), false, Some("called the drawer"));
        assert_eq!(represent_problem(&bounced(), Some(&note_only), day(1)), None);
    }
}
