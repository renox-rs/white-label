//! The booking state machine (wl-scheduling §4.1).
//!
//! ```text
//! [*] --request--> requested
//! [*] --book-----> booked
//! requested --accept--> booked
//! requested --decline--> cancelled
//! booked --confirm--> confirmed
//! requested | booked | confirmed --reschedule--> booked | requested
//! booked | confirmed --check_in--> checked_in --start--> in_progress
//! booked | confirmed | checked_in | in_progress --complete--> completed
//! booked | confirmed --mark_no_show--> no_show
//! requested | booked | confirmed | checked_in | in_progress --cancel--> cancelled
//! requested | booked | confirmed --reassign--> the same state
//! ```
//!
//! Decisions:
//!
//! - `request` and `book` have no source state: the service inserts the
//!   booking in its first state with a history row ([`crate::service`]).
//! - `reschedule` ends in `booked` or in `requested` (staff acceptance), and
//!   a transition has one end state, so the second is
//!   [`RESCHEDULE_REQUEST`]. Both move the booking to the new time.
//! - `reassign` keeps the state, and a transition has one end state, so there
//!   is one per state: [`REASSIGN_REQUESTED`], [`REASSIGN`] (`booked`) and
//!   [`REASSIGN_CONFIRMED`]. The service picks the one of the booking's state.
//! - Every transition is a system transition that the services run after they
//!   checked who may act: a party's right depends on the portal grant, a
//!   signed link or the cut-off, which the engine's permission check does not
//!   know (the same choice as `wl-hr`'s leave). The actor is kept in the
//!   run's input and in the history row.
//! - The guards see only the record and the input; the services gather what
//!   they need (the service type's flags, the cut-offs, the clock).

use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::scheduling::{CancelStage, Initiator};

use crate::booking::{Booking, BookingResource, BookingStatus, WALKED_OUT};
use crate::texts::english;

/// Staff accept a request.
pub const ACCEPT: &str = "accept";
/// Staff decline a request.
pub const DECLINE: &str = "decline";
/// The party confirms they will come.
pub const CONFIRM: &str = "confirm";
/// A booking moves to another time and returns to `booked`.
pub const RESCHEDULE: &str = "reschedule";
/// A booking moves to another time and returns to `requested`, because the
/// service type needs staff acceptance.
pub const RESCHEDULE_REQUEST: &str = "reschedule_request";
/// The party arrived.
pub const CHECK_IN: &str = "check_in";
/// The service started.
pub const START: &str = "start";
/// The service took place.
pub const COMPLETE: &str = "complete";
/// The party did not come.
pub const MARK_NO_SHOW: &str = "mark_no_show";
/// The booking is cancelled.
pub const CANCEL: &str = "cancel";
/// A `booked` booking moves to another resource at the same time.
pub const REASSIGN: &str = "reassign";
/// A `requested` booking moves to another resource at the same time.
pub const REASSIGN_REQUESTED: &str = "reassign_requested";
/// A `confirmed` booking moves to another resource at the same time.
pub const REASSIGN_CONFIRMED: &str = "reassign_confirmed";

/// Who runs a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    /// A timer or a listener.
    #[default]
    System,
    /// A member of staff (`bookings.manage`, `bookings.checkin`).
    Staff,
    /// The party: the portal, or a link in a message.
    Party,
}

impl ActorKind {
    /// The stored name: `system`, `staff` or `party`.
    pub fn as_str(&self) -> &'static str {
        match self {
            ActorKind::System => "system",
            ActorKind::Staff => "staff",
            ActorKind::Party => "party",
        }
    }
}

/// The time one resource is occupied after a move.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Occupy {
    /// The resource.
    pub resource_id: i64,
    /// The start of the occupied time, buffer before included.
    pub from: DateTime,
    /// The end of the occupied time, buffer after included.
    pub until: DateTime,
}

/// What a run of a booking transition needs, gathered by the service (a guard
/// cannot read the database and an effect gets no `AppState`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BookingInput {
    /// Who runs it.
    pub actor: ActorKind,
    /// The user who acts; `None` for the system or a party with a link.
    pub actor_user: Option<i64>,
    /// The moment to judge by.
    pub now: Option<DateTime>,
    /// The reason code of a cancellation.
    pub reason_code: Option<String>,
    /// The reason text staff gave.
    pub note: Option<String>,
    /// The service type needs a check-in before `complete`.
    pub check_in_required: bool,
    /// The service type tracks `in_progress`.
    pub track_in_progress: bool,
    /// Minutes after the start before a no-show.
    pub grace_minutes: i64,
    /// The party is before the cut-off of the action.
    pub within_cutoff: bool,
    /// Until when the new `requested` hold lasts.
    pub hold_until: Option<DateTime>,
    /// The new start (`reschedule`).
    pub new_start: Option<DateTime>,
    /// The new end (`reschedule`).
    pub new_end: Option<DateTime>,
    /// The occupied time of each resource after the move.
    pub occupy: Vec<Occupy>,
    /// The cancellation is *late* (inside the forfeit window or the cut-off).
    pub late: bool,
    /// Whose side the cancellation is charged to.
    pub initiated_by: Option<Initiator>,
    /// The default *initiated by* of the reason code; absent for an unknown
    /// code.
    pub default_initiated_by: Option<Initiator>,
    /// The party cancels from a message about a change the business made.
    pub change_link: bool,
    /// How far the booking had got.
    pub stage: Option<CancelStage>,
    /// The vertical record opened at check-in has not captured its work done
    /// yet (`SCH.ACL.06`).
    pub work_done_pending: bool,
    /// The work done, in fils, as the vertical record captured it.
    pub work_done_amount: i64,
    /// The resource a `reassign` replaces.
    pub reassign_from: Option<i64>,
}

impl BookingInput {
    /// The moment to judge by; the clock when absent.
    pub fn at(&self) -> DateTime {
        self.now.unwrap_or_else(now)
    }
}

fn input_of(ctx: &TransitionCtx) -> BookingInput {
    ctx.input_as::<BookingInput>().unwrap_or_default()
}

/// Sets the columns of booking `id` outside the machine's guard and stamps
/// `updated_at`.
async fn set(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    Booking::query()
        .where_(Booking::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

/// Releases every slot the booking holds.
async fn release_rows(tx: &mut renox::db::Transaction, id: i64) -> Result {
    BookingResource::query()
        .where_(BookingResource::BOOKING_ID.eq(id))
        .where_(BookingResource::RELEASED.eq(false))
        .update(&mut *tx, &[("released", &true)])
        .await?;
    Ok(())
}

/// Moves the booking to the new time: the old slots are released and the new
/// ones taken.
async fn move_to(tx: &mut renox::db::Transaction, id: i64, input: &BookingInput) -> Result {
    release_rows(tx, id).await?;
    take_rows(tx, id, &input.occupy).await
}

/// Takes the time of `rows` for the booking.
async fn take_rows(tx: &mut renox::db::Transaction, id: i64, rows: &[Occupy]) -> Result {
    for occupy in rows {
        BookingResource {
            booking_id: id,
            resource_id: occupy.resource_id,
            occupies_from: occupy.from,
            occupies_until: occupy.until,
            released: false,
            ..BookingResource::default()
        }
        .save(&mut *tx)
        .await?;
    }
    Ok(())
}

/// The effect of the reassign transitions: the replaced resource's slot is
/// released, the new resource's slot taken, and the conflict is cleared.
async fn swap_resource(tx: &mut renox::db::Transaction, id: i64, input: &BookingInput) -> Result {
    if let Some(from) = input.reassign_from {
        BookingResource::query()
            .where_(BookingResource::BOOKING_ID.eq(id))
            .where_(BookingResource::RESOURCE_ID.eq(from))
            .where_(BookingResource::RELEASED.eq(false))
            .update(&mut *tx, &[("released", &true)])
            .await?;
    }
    take_rows(tx, id, &input.occupy).await?;
    set(
        tx,
        id,
        &[("conflict_block", &false), ("conflict_eligibility", &false)],
    )
    .await
}

/// `reassign` for a booking in `state`: staff only, with a known reason code
/// that is not shown to the party.
fn reassign_transition(name: &str, state: BookingStatus) -> Transition<Booking> {
    transition(name, &[state], state)
        .label("Reassign")
        .guard("staff_only", |_b: &Booking, ctx| {
            let refused = input_of(ctx).actor != ActorKind::Staff;
            Box::pin(async move {
                if refused {
                    Err(guard_text("wl_scheduling.problem.staff_only"))
                } else {
                    Ok(())
                }
            })
        })
        .guard("reason_given", |_b: &Booking, ctx| {
            let given = input_of(ctx)
                .reason_code
                .is_some_and(|code| !code.trim().is_empty());
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(guard_text("wl_scheduling.problem.reason_needed"))
                }
            })
        })
        .guard("reason_known", |_b: &Booking, ctx| {
            let known = input_of(ctx).default_initiated_by.is_some();
            Box::pin(async move {
                if known {
                    Ok(())
                } else {
                    Err(guard_text("wl_scheduling.problem.unknown_reason"))
                }
            })
        })
        .effect(|booking: &Booking, ctx, tx| {
            let id = booking.id;
            let input = input_of(ctx);
            Box::pin(async move { swap_resource(tx, id, &input).await })
        })
}

/// A transition no user runs: the services check who may act.
fn transition(name: &str, from: &[BookingStatus], to: BookingStatus) -> Transition<Booking> {
    Transition::new(name)
        .system()
        .from(from.iter().copied())
        .to(to)
}

fn guard_text(key: &str) -> String {
    english(key, &[])
}

impl StateMachine for Booking {
    type State = BookingStatus;
    const COLUMN: renox::db::Col<Booking, BookingStatus> = Booking::STATUS;

    fn machine() -> Machine<Self> {
        use BookingStatus::{
            Booked, Cancelled, CheckedIn, Completed, Confirmed, InProgress, NoShow, Requested,
        };
        Machine::new()
            .initial([Requested, Booked])
            .state(Requested)
            .tone("neutral")
            .state(Booked)
            .tone("info")
            .state(Confirmed)
            .tone("info")
            .state(CheckedIn)
            .tone("info")
            .state(InProgress)
            .tone("info")
            .state(Completed)
            .tone("success")
            .terminal()
            .state(NoShow)
            .tone("error")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(
                transition(ACCEPT, &[Requested], Booked)
                    .label("Accept")
                    .tone("success")
                    .effect(|booking: &Booking, _ctx, tx| {
                        let id = booking.id;
                        Box::pin(async move {
                            let none: Option<DateTime> = None;
                            set(tx, id, &[("hold_until", &none)]).await
                        })
                    }),
            )
            .transition(
                transition(DECLINE, &[Requested], Cancelled)
                    .label("Decline")
                    .tone("danger")
                    .guard("note_given", |_b: &Booking, ctx| {
                        let given = input_of(ctx)
                            .note
                            .is_some_and(|note| !note.trim().is_empty());
                        Box::pin(async move {
                            if given {
                                Ok(())
                            } else {
                                Err(guard_text("wl_scheduling.problem.reason_needed"))
                            }
                        })
                    })
                    .effect(|booking: &Booking, ctx, tx| {
                        let id = booking.id;
                        let input = input_of(ctx);
                        Box::pin(async move {
                            let at = Some(input.at());
                            let code = Some(crate::booking::OTHER.to_owned());
                            let by = Some(input.actor.as_str().to_owned());
                            // A decline is the business saying no, before the start.
                            let initiated = Some(Initiator::Business.as_str().to_owned());
                            let stage = Some(CancelStage::BeforeStart.as_str().to_owned());
                            set(
                                tx,
                                id,
                                &[
                                    ("cancel_code", &code),
                                    ("cancel_initiated_by", &initiated),
                                    ("cancel_by", &by),
                                    ("cancel_note", &input.note),
                                    ("cancel_stage", &stage),
                                    ("cancelled_at", &at),
                                ],
                            )
                            .await?;
                            release_rows(tx, id).await
                        })
                    }),
            )
            .transition(
                transition(CONFIRM, &[Booked], Confirmed)
                    .label("Confirm")
                    .effect(|booking: &Booking, ctx, tx| {
                        let id = booking.id;
                        let at = Some(input_of(ctx).at());
                        Box::pin(async move { set(tx, id, &[("confirmed_at", &at)]).await })
                    }),
            )
            .transition(
                transition(RESCHEDULE, &[Booked, Confirmed], Booked)
                    .label("Reschedule")
                    .guard("before_cutoff", |_b: &Booking, ctx| {
                        let refused = party_after_cutoff(ctx);
                        Box::pin(async move {
                            if refused {
                                Err(guard_text("wl_scheduling.problem.after_cutoff"))
                            } else {
                                Ok(())
                            }
                        })
                    })
                    .effect(|booking: &Booking, ctx, tx| {
                        let (id, start, end) = (booking.id, booking.starts_at, booking.ends_at);
                        let input = input_of(ctx);
                        Box::pin(
                            async move { move_booking(tx, id, start, end, &input, false).await },
                        )
                    }),
            )
            .transition(
                transition(
                    RESCHEDULE_REQUEST,
                    &[Requested, Booked, Confirmed],
                    Requested,
                )
                .label("Reschedule")
                .guard("before_cutoff", |_b: &Booking, ctx| {
                    let refused = party_after_cutoff(ctx);
                    Box::pin(async move {
                        if refused {
                            Err(guard_text("wl_scheduling.problem.after_cutoff"))
                        } else {
                            Ok(())
                        }
                    })
                })
                .effect(|booking: &Booking, ctx, tx| {
                    let (id, start, end) = (booking.id, booking.starts_at, booking.ends_at);
                    let input = input_of(ctx);
                    Box::pin(async move { move_booking(tx, id, start, end, &input, true).await })
                }),
            )
            .transition(
                transition(CHECK_IN, &[Booked, Confirmed], CheckedIn)
                    .label("Check in")
                    .guard("check_in_required", |_b: &Booking, ctx| {
                        let required = input_of(ctx).check_in_required;
                        Box::pin(async move {
                            if required {
                                Ok(())
                            } else {
                                Err(guard_text("wl_scheduling.problem.no_check_in"))
                            }
                        })
                    })
                    .effect(|booking: &Booking, ctx, tx| {
                        let id = booking.id;
                        let at = Some(input_of(ctx).at());
                        Box::pin(async move { set(tx, id, &[("checked_in_at", &at)]).await })
                    }),
            )
            .transition(
                transition(START, &[CheckedIn], InProgress)
                    .label("Start")
                    .guard("tracks_in_progress", |_b: &Booking, ctx| {
                        let tracked = input_of(ctx).track_in_progress;
                        Box::pin(async move {
                            if tracked {
                                Ok(())
                            } else {
                                Err(guard_text("wl_scheduling.problem.no_in_progress"))
                            }
                        })
                    })
                    .effect(|booking: &Booking, ctx, tx| {
                        let id = booking.id;
                        let at = Some(input_of(ctx).at());
                        Box::pin(async move { set(tx, id, &[("started_at", &at)]).await })
                    }),
            )
            .transition(
                transition(
                    COMPLETE,
                    &[Booked, Confirmed, CheckedIn, InProgress],
                    Completed,
                )
                .label("Complete")
                .tone("success")
                .guard("start_passed", |booking: &Booking, ctx| {
                    let passed = input_of(ctx).at() >= booking.starts_at;
                    Box::pin(async move {
                        if passed {
                            Ok(())
                        } else {
                            Err(guard_text("wl_scheduling.problem.not_started"))
                        }
                    })
                })
                .guard("checked_in_first", |booking: &Booking, ctx| {
                    let must = input_of(ctx).check_in_required
                        && matches!(booking.status, Booked | Confirmed);
                    Box::pin(async move {
                        if must {
                            Err(guard_text("wl_scheduling.problem.check_in_first"))
                        } else {
                            Ok(())
                        }
                    })
                })
                .effect(|booking: &Booking, ctx, tx| {
                    let id = booking.id;
                    let at = Some(input_of(ctx).at());
                    Box::pin(async move {
                        set(tx, id, &[("completed_at", &at)]).await?;
                        release_rows(tx, id).await
                    })
                }),
            )
            .transition(
                transition(MARK_NO_SHOW, &[Booked, Confirmed], NoShow)
                    .label("Mark no-show")
                    .tone("danger")
                    .guard("grace_passed", |booking: &Booking, ctx| {
                        let input = input_of(ctx);
                        let due = booking.starts_at
                            + renox::chrono::TimeDelta::minutes(input.grace_minutes);
                        let passed = input.at() >= due;
                        Box::pin(async move {
                            if passed {
                                Ok(())
                            } else {
                                Err(guard_text("wl_scheduling.problem.grace_not_over"))
                            }
                        })
                    })
                    .guard("no_open_conflict", |booking: &Booking, _ctx| {
                        let conflicted = booking.has_conflict();
                        Box::pin(async move {
                            if conflicted {
                                Err(guard_text("wl_scheduling.problem.conflict_open"))
                            } else {
                                Ok(())
                            }
                        })
                    })
                    .effect(|booking: &Booking, ctx, tx| {
                        let id = booking.id;
                        let at = Some(input_of(ctx).at());
                        Box::pin(async move {
                            set(tx, id, &[("no_show_at", &at)]).await?;
                            release_rows(tx, id).await
                        })
                    }),
            )
            .transition(reassign_transition(REASSIGN_REQUESTED, Requested))
            .transition(reassign_transition(REASSIGN, Booked))
            .transition(reassign_transition(REASSIGN_CONFIRMED, Confirmed))
            .transition(
                transition(
                    CANCEL,
                    &[Requested, Booked, Confirmed, CheckedIn, InProgress],
                    Cancelled,
                )
                .label("Cancel")
                .tone("danger")
                .guard("reason_given", |_b: &Booking, ctx| {
                    let given = input_of(ctx)
                        .reason_code
                        .is_some_and(|code| !code.trim().is_empty());
                    Box::pin(async move {
                        if given {
                            Ok(())
                        } else {
                            Err(guard_text("wl_scheduling.problem.reason_needed"))
                        }
                    })
                })
                .guard("reason_known", |_b: &Booking, ctx| {
                    let known = input_of(ctx).default_initiated_by.is_some();
                    Box::pin(async move {
                        if known {
                            Ok(())
                        } else {
                            Err(guard_text("wl_scheduling.problem.unknown_reason"))
                        }
                    })
                })
                .guard("walked_out_after_check_in", |booking: &Booking, ctx| {
                    let walked_out = input_of(ctx).reason_code.as_deref() == Some(WALKED_OUT);
                    let refused = walked_out && !booking.status.is_after_check_in();
                    Box::pin(async move {
                        if refused {
                            Err(guard_text("wl_scheduling.problem.walked_out_early"))
                        } else {
                            Ok(())
                        }
                    })
                })
                .guard("staff_after_check_in", |booking: &Booking, ctx| {
                    let refused = booking.status.is_after_check_in()
                        && input_of(ctx).actor != ActorKind::Staff;
                    Box::pin(async move {
                        if refused {
                            Err(guard_text("wl_scheduling.problem.staff_after_check_in"))
                        } else {
                            Ok(())
                        }
                    })
                })
                .guard("work_done_settled", |booking: &Booking, ctx| {
                    let refused =
                        booking.status.is_after_check_in() && input_of(ctx).work_done_pending;
                    Box::pin(async move {
                        if refused {
                            Err(guard_text("wl_scheduling.problem.work_done_pending"))
                        } else {
                            Ok(())
                        }
                    })
                })
                .guard("party_before_cutoff", |_b: &Booking, ctx| {
                    let input = input_of(ctx);
                    let refused = input.actor == ActorKind::Party
                        && !input.change_link
                        && !input.within_cutoff;
                    Box::pin(async move {
                        if refused {
                            Err(guard_text("wl_scheduling.problem.after_cutoff"))
                        } else {
                            Ok(())
                        }
                    })
                })
                .guard("initiated_by", |_b: &Booking, ctx| {
                    let input = input_of(ctx);
                    let problem = initiated_by_problem(&input);
                    Box::pin(async move {
                        match problem {
                            Some(key) => Err(guard_text(key)),
                            None => Ok(()),
                        }
                    })
                })
                .effect(|booking: &Booking, ctx, tx| {
                    let id = booking.id;
                    let input = input_of(ctx);
                    Box::pin(async move {
                        let at = Some(input.at());
                        let by = Some(input.actor.as_str().to_owned());
                        let initiated = input.initiated_by.map(|i| i.as_str().to_owned());
                        let stage = input.stage.map(|s| s.as_str().to_owned());
                        set(
                            tx,
                            id,
                            &[
                                ("cancel_code", &input.reason_code),
                                ("cancel_initiated_by", &initiated),
                                ("cancel_by", &by),
                                ("cancel_note", &input.note),
                                ("cancel_stage", &stage),
                                ("cancel_late", &input.late),
                                ("work_done_amount", &input.work_done_amount),
                                ("cancelled_at", &at),
                                ("conflict_block", &false),
                                ("conflict_eligibility", &false),
                            ],
                        )
                        .await?;
                        release_rows(tx, id).await
                    })
                }),
            )
    }
}

/// What is wrong with *initiated by* of a cancellation, as a text key; `None`
/// when it is fine (wl-scheduling §4.1 *Cancellation reason codes*):
///
/// - a party never picks `business`, except by cancelling from a message
///   about a change the business made (`change_link`), where it is set;
/// - staff may record the other side than the code's default only with a note.
fn initiated_by_problem(input: &BookingInput) -> Option<&'static str> {
    let Some(initiated) = input.initiated_by else {
        return Some("wl_scheduling.problem.initiated_by_needed");
    };
    if input.actor == ActorKind::Party && !input.change_link && initiated != Initiator::Party {
        return Some("wl_scheduling.problem.party_cannot_blame");
    }
    let overridden = input.default_initiated_by.is_some_and(|d| d != initiated);
    let noted = input
        .note
        .as_deref()
        .is_some_and(|note| !note.trim().is_empty());
    if overridden && !noted && !input.change_link {
        return Some("wl_scheduling.problem.override_needs_note");
    }
    None
}

/// Whether a party tries to move or cancel a booking after the cut-off
/// (staff always may).
fn party_after_cutoff(ctx: &TransitionCtx) -> bool {
    let input = input_of(ctx);
    input.actor == ActorKind::Party && !input.within_cutoff
}

/// The effect of both reschedule transitions: the new time, the new slots, no
/// open conflict, and the hold of a `requested` booking.
async fn move_booking(
    tx: &mut renox::db::Transaction,
    id: i64,
    start: DateTime,
    end: DateTime,
    input: &BookingInput,
    holds: bool,
) -> Result {
    let start = input.new_start.unwrap_or(start);
    let end = input.new_end.unwrap_or(end);
    let hold = if holds { input.hold_until } else { None };
    set(
        tx,
        id,
        &[
            ("starts_at", &start),
            ("ends_at", &end),
            ("hold_until", &hold),
            ("conflict_block", &false),
            ("conflict_eligibility", &false),
        ],
    )
    .await?;
    move_to(tx, id, input).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_every_state_of_the_spec() {
        let machine = Booking::machine();
        let states: Vec<&str> = machine.states().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            states,
            [
                "requested",
                "booked",
                "confirmed",
                "checked_in",
                "in_progress",
                "completed",
                "no_show",
                "cancelled"
            ]
        );
        assert_eq!(machine.initial_states(), ["requested", "booked"]);
        let terminal: Vec<&str> = machine
            .states()
            .iter()
            .filter(|s| s.terminal)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(terminal, ["completed", "no_show", "cancelled"]);
    }

    #[test]
    fn the_transitions_leave_and_enter_the_states_of_the_spec() {
        let machine = Booking::machine();
        let row = |name: &str| {
            let t = machine.find(name).expect("the transition exists");
            (
                t.from_states().to_vec(),
                t.to_state().unwrap_or_default().to_owned(),
                t.is_system(),
            )
        };
        assert_eq!(
            row(ACCEPT),
            (vec!["requested".to_owned()], "booked".to_owned(), true)
        );
        assert_eq!(
            row(DECLINE),
            (vec!["requested".to_owned()], "cancelled".to_owned(), true)
        );
        assert_eq!(
            row(CONFIRM),
            (vec!["booked".to_owned()], "confirmed".to_owned(), true)
        );
        assert_eq!(row(RESCHEDULE).1, "booked");
        assert_eq!(row(RESCHEDULE_REQUEST).1, "requested");
        assert_eq!(row(CHECK_IN).0, ["booked", "confirmed"]);
        assert_eq!(
            row(START),
            (
                vec!["checked_in".to_owned()],
                "in_progress".to_owned(),
                true
            )
        );
        assert_eq!(
            row(COMPLETE).0,
            ["booked", "confirmed", "checked_in", "in_progress"]
        );
        assert_eq!(row(MARK_NO_SHOW).1, "no_show");
        assert_eq!(
            row(CANCEL).0,
            [
                "requested",
                "booked",
                "confirmed",
                "checked_in",
                "in_progress"
            ]
        );
        // `reassign` keeps the state, whichever it is.
        assert_eq!(
            row(REASSIGN_REQUESTED),
            (vec!["requested".to_owned()], "requested".to_owned(), true)
        );
        assert_eq!(
            row(REASSIGN),
            (vec!["booked".to_owned()], "booked".to_owned(), true)
        );
        assert_eq!(
            row(REASSIGN_CONFIRMED),
            (vec!["confirmed".to_owned()], "confirmed".to_owned(), true)
        );
    }

    fn cancel_input(actor: ActorKind) -> BookingInput {
        BookingInput {
            actor,
            reason_code: Some("site_closed".to_owned()),
            default_initiated_by: Some(Initiator::Business),
            initiated_by: Some(Initiator::Business),
            ..BookingInput::default()
        }
    }

    #[test]
    fn staff_record_the_default_side_without_a_note() {
        assert_eq!(initiated_by_problem(&cancel_input(ActorKind::Staff)), None);
    }

    #[test]
    fn staff_override_the_side_only_with_a_note() {
        let mut input = cancel_input(ActorKind::Staff);
        input.initiated_by = Some(Initiator::Party);
        assert_eq!(
            initiated_by_problem(&input),
            Some("wl_scheduling.problem.override_needs_note")
        );
        input.note = Some("patient asked to close early".to_owned());
        assert_eq!(initiated_by_problem(&input), None);
    }

    #[test]
    fn a_party_never_picks_business_except_from_a_change_message() {
        let mut input = cancel_input(ActorKind::Party);
        assert_eq!(
            initiated_by_problem(&input),
            Some("wl_scheduling.problem.party_cannot_blame")
        );
        input.change_link = true;
        assert_eq!(initiated_by_problem(&input), None);
        input.initiated_by = None;
        assert_eq!(
            initiated_by_problem(&input),
            Some("wl_scheduling.problem.initiated_by_needed")
        );
    }

    #[test]
    fn the_guards_are_named() {
        let machine = Booking::machine();
        let guards = |name: &str| -> Vec<String> {
            machine
                .find(name)
                .expect("the transition exists")
                .guard_names()
                .into_iter()
                .map(str::to_owned)
                .collect()
        };
        assert_eq!(guards(DECLINE), ["note_given"]);
        assert_eq!(guards(CHECK_IN), ["check_in_required"]);
        assert_eq!(guards(START), ["tracks_in_progress"]);
        assert_eq!(guards(COMPLETE), ["start_passed", "checked_in_first"]);
        assert_eq!(guards(MARK_NO_SHOW), ["grace_passed", "no_open_conflict"]);
        assert_eq!(
            guards(CANCEL),
            [
                "reason_given",
                "reason_known",
                "walked_out_after_check_in",
                "staff_after_check_in",
                "work_done_settled",
                "party_before_cutoff",
                "initiated_by"
            ]
        );
        for name in [REASSIGN_REQUESTED, REASSIGN, REASSIGN_CONFIRMED] {
            assert_eq!(guards(name), ["staff_only", "reason_given", "reason_known"]);
        }
        assert_eq!(guards(RESCHEDULE), ["before_cutoff"]);
    }
}
