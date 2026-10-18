//! Sanctions screenings (wl-parties §4, `PTY.SCR.01`, issue #38).
//!
//! A [`Screening`] is one run of a party against the lists, with its match
//! [`ScreeningCandidate`]s. Its status moves only through the Screening
//! machine:
//!
//! ```text
//! [*] --> pending --complete_clear--> clear
//!         pending --flag--> possible_match
//! ```
//!
//! `false_positive` and `confirmed_match` are declared for the transitions
//! that later issues add to the same machine (`dismiss`, `confirm_match`).
//!
//! Both transitions are the system's. `complete_clear` needs every candidate
//! below the match threshold; `flag` needs one at or above it, and a manual
//! run also needs a reviewer holding `screening.review` and the list entry
//! of every candidate. [`record_run`] is the service entry: it checks that
//! screening is on, stores the run, picks the transition and runs it. The
//! compliance task, the alerts and the events come with the `flag` effect.

use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::ScreeningStatus;

use crate::party::Party;
use crate::permissions;
use crate::texts;

/// The system transition for a run with no candidate at the threshold.
pub const COMPLETE_CLEAR: &str = "complete_clear";
/// The system transition for a run with a candidate at the threshold.
pub const FLAG: &str = "flag";

/// The match threshold when the app sets none (wl-parties §7).
pub const DEFAULT_MATCH_THRESHOLD: f64 = 0.85;

/// What made the screening run.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScreeningTrigger {
    /// The party was onboarded.
    #[default]
    Onboarding,
    /// A list was updated.
    ListUpdate,
    /// A periodic or requested review.
    Review,
    /// A reviewer ran it by hand.
    Manual,
}

/// How the screening was run.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScreeningMethod {
    /// Against the downloaded lists.
    #[default]
    Automatic,
    /// A reviewer recorded the result of a check by hand.
    Manual,
}

/// One screening run of a party.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_parties_screenings",
    state_machine,
    index(party_id),
    index(status)
)]
pub struct Screening {
    /// The primary key.
    pub id: i64,
    /// The screened party.
    pub party_id: i64,
    /// Why it ran.
    pub run_trigger: ScreeningTrigger,
    /// Automatic, or recorded by hand.
    pub method: ScreeningMethod,
    /// Where it stands; changed only by a transition.
    pub status: ScreeningStatus,
    /// When it ran.
    pub run_at: DateTime,
    /// The reviewer who recorded a manual run.
    pub reviewer_user: Option<i64>,
    /// Whether a list used was stale at the run.
    pub lists_stale: bool,
    /// The reviewer's note on the decision.
    pub review_note: Option<String>,
    /// The compliance task it is listed on, once flagged.
    pub task_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A possible match of a screening.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_parties_screening_candidates", index(screening_id))]
pub struct ScreeningCandidate {
    /// The primary key.
    pub id: i64,
    /// The screening.
    pub screening_id: i64,
    /// The name on the list.
    pub name: String,
    /// The list it is on.
    pub list_name: String,
    /// How close it is to the party, from 0 to 1.
    pub score: f64,
    /// The entry on the list.
    pub list_entry: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A candidate of a run, not yet stored.
#[derive(Debug, Clone, PartialEq)]
pub struct NewCandidate {
    /// The name on the list.
    pub name: String,
    /// The list it is on.
    pub list_name: String,
    /// How close it is to the party, from 0 to 1.
    pub score: f64,
    /// The entry on the list; a manual flag needs it.
    pub list_entry: Option<String>,
}

/// One screening run, as the provider or a reviewer reports it.
#[derive(Debug, Clone)]
pub struct ScreeningRun {
    /// Why it ran.
    pub trigger: ScreeningTrigger,
    /// Automatic, or recorded by hand.
    pub method: ScreeningMethod,
    /// Whether a list used was stale.
    pub lists_stale: bool,
    /// When it ran; now when `None`.
    pub run_at: Option<DateTime>,
    /// The possible matches, with their scores.
    pub candidates: Vec<NewCandidate>,
}

impl ScreeningRun {
    /// An automatic run for `trigger` with these candidates.
    pub fn automatic(trigger: ScreeningTrigger, candidates: Vec<NewCandidate>) -> Self {
        ScreeningRun {
            trigger,
            method: ScreeningMethod::Automatic,
            lists_stale: false,
            run_at: None,
            candidates,
        }
    }

    /// A manual check recorded by a reviewer.
    pub fn manual(candidates: Vec<NewCandidate>) -> Self {
        ScreeningRun {
            trigger: ScreeningTrigger::Manual,
            method: ScreeningMethod::Manual,
            ..ScreeningRun::automatic(ScreeningTrigger::Manual, candidates)
        }
    }
}

/// What the module is set up with, provided to the app.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ScreeningSettings {
    /// Whether screening is on.
    pub on: bool,
    /// The score at or above which a candidate is a possible match.
    pub threshold: f64,
}

impl ScreeningSettings {
    /// The app's settings, or the defaults (off, 0.85).
    pub fn of(state: &AppState) -> ScreeningSettings {
        state
            .provided::<ScreeningSettings>()
            .map(|s| *s)
            .unwrap_or(ScreeningSettings {
                on: false,
                threshold: DEFAULT_MATCH_THRESHOLD,
            })
    }
}

/// What a run carries to the guards: the service fills it, as the machine
/// has no app state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct RunInput {
    /// The match threshold.
    pub threshold: f64,
    /// The highest candidate score; `None` without candidates.
    pub best_score: Option<f64>,
    /// Whether the run is a manual check.
    pub manual: bool,
    /// Whether the user who recorded it holds `screening.review`.
    pub reviewer_ok: bool,
    /// Whether every candidate has its list entry.
    pub entries_ok: bool,
}

impl RunInput {
    fn at_threshold(&self) -> bool {
        self.best_score.is_some_and(|score| score >= self.threshold)
    }
}

/// The run's input, if it is a [`RunInput`].
fn input(ctx: &TransitionCtx) -> Option<RunInput> {
    ctx.input_as::<RunInput>().ok()
}

/// A guard message (guards have no request language).
fn english(key: &str) -> String {
    texts::english(&format!("wl_parties.errors.{key}"))
}

/// The refusal as the error the caller shows.
fn refused(key: &str) -> TransitionError {
    TransitionError::Guard(english(key))
}

impl StateMachine for Screening {
    type State = ScreeningStatus;
    const COLUMN: renox::db::Col<Screening, ScreeningStatus> = Screening::STATUS;

    fn machine() -> Machine<Self> {
        use ScreeningStatus::{Clear, ConfirmedMatch, FalsePositive, Pending, PossibleMatch};
        Machine::new()
            .initial([Pending])
            .state(Pending)
            .tone("neutral")
            .state(Clear)
            .tone("success")
            .terminal()
            .state(PossibleMatch)
            .tone("warning")
            .state(FalsePositive)
            .tone("neutral")
            .terminal()
            .state(ConfirmedMatch)
            .tone("error")
            .terminal()
            .transition(
                Transition::new(COMPLETE_CLEAR)
                    .label("Complete as clear")
                    .tone("success")
                    .from([Pending])
                    .to(Clear)
                    .system()
                    .guard("below_threshold", |_s: &Screening, ctx| {
                        Box::pin(async move {
                            match input(ctx) {
                                Some(run) if !run.at_threshold() => Ok(()),
                                _ => Err(english("screening_clear")),
                            }
                        })
                    })
                    .guard("manual_reviewer", manual_reviewer),
            )
            .transition(
                Transition::new(FLAG)
                    .label("Flag a possible match")
                    .tone("warning")
                    .from([Pending])
                    .to(PossibleMatch)
                    .system()
                    .guard("at_threshold", |_s: &Screening, ctx| {
                        Box::pin(async move {
                            match input(ctx) {
                                Some(run) if run.at_threshold() => Ok(()),
                                _ => Err(english("screening_match")),
                            }
                        })
                    })
                    .guard("manual_reviewer", manual_reviewer)
                    .guard("manual_list_entry", |_s: &Screening, ctx| {
                        Box::pin(async move {
                            match input(ctx) {
                                Some(run) if !run.manual || run.entries_ok => Ok(()),
                                _ => Err(english("screening_entry")),
                            }
                        })
                    }),
            )
    }
}

/// A manual run needs a reviewer who holds `screening.review`.
fn manual_reviewer<'a>(
    _screening: &'a Screening,
    ctx: &'a TransitionCtx,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::result::Result<(), String>> + Send + 'a>>
{
    Box::pin(async move {
        match input(ctx) {
            Some(run) if !run.manual || run.reviewer_ok => Ok(()),
            _ => Err(english("screening_reviewer")),
        }
    })
}

impl Screening {
    /// The screening's candidates, best score first.
    pub async fn candidates<'c>(
        &self,
        db: impl renox::db::Executor<'c>,
    ) -> Result<Vec<ScreeningCandidate>> {
        ScreeningCandidate::query()
            .where_(ScreeningCandidate::SCREENING_ID.eq(self.id))
            .order_by_desc(ScreeningCandidate::SCORE)
            .order_by(ScreeningCandidate::ID)
            .get(db)
            .await
    }

    /// The party's screenings, newest first.
    pub async fn for_party<'c>(
        db: impl renox::db::Executor<'c>,
        party_id: i64,
    ) -> Result<Vec<Screening>> {
        Screening::query()
            .where_(Screening::PARTY_ID.eq(party_id))
            .order_by_desc(Screening::ID)
            .get(db)
            .await
    }
}

/// Records one screening run of `party` and ends it: stores the run and its
/// candidates, then runs `flag` when a candidate is at or above the match
/// threshold ([`crate::WlParties::match_threshold`], compared with `>=`),
/// else `complete_clear`. `by` is the reviewer of a manual run (it needs
/// `screening.review`). It all happens in one transaction. Returns the
/// screening as stored after the transition. No task, alert or event yet.
///
/// # Errors
///
/// `Guard` when screening is off ([`crate::WlParties::screening`]), when a
/// candidate has a blank name or list or a score outside 0 to 1, or when a
/// manual run lacks a reviewer with `screening.review` or the list entry of
/// a candidate it flags. Nothing is stored then.
pub async fn record_run(
    state: &AppState,
    party: &Party,
    run: ScreeningRun,
    by: Option<&AuthUser>,
) -> std::result::Result<Screening, TransitionError> {
    let settings = ScreeningSettings::of(state);
    if !settings.on {
        return Err(refused("screening_off"));
    }
    let valid = |c: &NewCandidate| {
        !c.name.trim().is_empty()
            && !c.list_name.trim().is_empty()
            && (0.0..=1.0).contains(&c.score)
    };
    if !run.candidates.iter().all(valid) {
        return Err(refused("screening_score"));
    }
    let manual = run.method == ScreeningMethod::Manual;
    let input = RunInput {
        threshold: settings.threshold,
        best_score: run.candidates.iter().map(|c| c.score).reduce(f64::max),
        manual,
        reviewer_ok: by.is_some_and(|user| user.has_permission(permissions::SCREENING_REVIEW)),
        entries_ok: run.candidates.iter().all(|c| {
            c.list_entry
                .as_deref()
                .is_some_and(|e| !e.trim().is_empty())
        }),
    };
    let transition = if input.at_threshold() {
        FLAG
    } else {
        COMPLETE_CLEAR
    };
    let mut tx = state.db.begin_immediate().await?;
    let mut screening = Screening {
        party_id: party.id,
        run_trigger: run.trigger,
        method: run.method,
        run_at: run.run_at.unwrap_or_else(renox::db::now),
        reviewer_user: if manual { by.map(|user| user.id) } else { None },
        lists_stale: run.lists_stale,
        ..Screening::default()
    };
    screening.insert(&mut tx).await?;
    for candidate in &run.candidates {
        let mut row = ScreeningCandidate {
            screening_id: screening.id,
            name: candidate.name.trim().to_owned(),
            list_name: candidate.list_name.trim().to_owned(),
            score: candidate.score,
            list_entry: candidate
                .list_entry
                .as_deref()
                .map(str::trim)
                .filter(|e| !e.is_empty())
                .map(str::to_owned),
            ..ScreeningCandidate::default()
        };
        row.insert(&mut tx).await?;
    }
    // A refusal drops the transaction, so nothing of the run is stored.
    let _outcome = screening
        .transition(transition)
        .by_system()
        .input(&input)
        .run_in(&mut tx)
        .await?;
    tx.commit().await?;
    let stored = Screening::find(&state.db, screening.id)
        .await?
        .ok_or(renox::Error::NotFound)?;
    Ok(stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_ctx(input: RunInput) -> TransitionCtx {
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    fn input_with(best: Option<f64>, manual: bool) -> RunInput {
        RunInput {
            threshold: 0.85,
            best_score: best,
            manual,
            reviewer_ok: true,
            entries_ok: true,
        }
    }

    #[test]
    fn the_machine_has_both_system_transitions_from_pending() {
        let machine = Screening::machine();
        let clear = machine.find(COMPLETE_CLEAR).expect("complete_clear");
        assert_eq!(clear.from_states(), ["pending"]);
        assert_eq!(clear.to_state(), Some("clear"));
        assert!(clear.is_system());
        assert_eq!(clear.guard_names(), ["below_threshold", "manual_reviewer"]);
        let flag = machine.find(FLAG).expect("flag");
        assert_eq!(flag.from_states(), ["pending"]);
        assert_eq!(flag.to_state(), Some("possible_match"));
        assert!(flag.is_system());
        assert_eq!(
            flag.guard_names(),
            ["at_threshold", "manual_reviewer", "manual_list_entry"]
        );
    }

    #[renox::test]
    async fn the_threshold_guards_split_on_the_boundary() {
        let machine = Screening::machine();
        let clear = machine.find(COMPLETE_CLEAR).expect("clear");
        let flag = machine.find(FLAG).expect("flag");
        let screening = Screening::default();
        for (best, flags) in [(None, false), (Some(0.84), false), (Some(0.85), true)] {
            let ctx = run_ctx(input_with(best, false));
            assert_eq!(flag.check_guards(&screening, &ctx).await.is_ok(), flags);
            assert_eq!(clear.check_guards(&screening, &ctx).await.is_ok(), !flags);
        }
    }

    #[renox::test]
    async fn a_manual_run_needs_a_reviewer_and_the_list_entry_to_flag() {
        let machine = Screening::machine();
        let flag = machine.find(FLAG).expect("flag");
        let clear = machine.find(COMPLETE_CLEAR).expect("clear");
        let screening = Screening::default();
        let good = input_with(Some(0.9), true);
        assert!(
            flag.check_guards(&screening, &run_ctx(good.clone()))
                .await
                .is_ok()
        );
        let no_entry = RunInput {
            entries_ok: false,
            ..good.clone()
        };
        assert!(matches!(
            flag.check_guards(&screening, &run_ctx(no_entry)).await,
            Err(TransitionError::Guard(_))
        ));
        // An automatic run needs no list entry.
        let auto = RunInput {
            manual: false,
            entries_ok: false,
            ..good.clone()
        };
        assert!(flag.check_guards(&screening, &run_ctx(auto)).await.is_ok());
        let no_reviewer = RunInput {
            reviewer_ok: false,
            best_score: Some(0.1),
            ..good
        };
        assert!(matches!(
            clear.check_guards(&screening, &run_ctx(no_reviewer)).await,
            Err(TransitionError::Guard(_))
        ));
    }
}
