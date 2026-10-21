//! The waiting list (wl-scheduling §3 *Waiting list entry*, §4.2).
//!
//! ```text
//! [*] --join--> waiting
//! waiting --offer--> offered      (the slot is held for the offer hold time)
//! offered --accept--> booked      (the party took the slot: a booking is made)
//! offered --lapse--> waiting      (the hold passed)
//! waiting --expire--> expired     (the entry's day passed)
//! waiting | offered --withdraw--> withdrawn
//! ```
//!
//! An offered entry holds its slot: it counts towards the capacity of the
//! resource until [`WaitingEntry::offer_hold_until`] passes, also before
//! `lapse` has run ([`crate::slots`]). Offers go to the entries in the order
//! they joined; the rule that offers a released slot (`SCH.RUL.03`) picks the
//! first match with [`first_match`].
//!
//! Decision: every transition is a system transition run by the services
//! below, which the caller (staff with `bookings.manage`, the party, a timer)
//! has authorised, as for the booking.

use renox::chrono::{NaiveDate, TimeDelta};
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::scheduling::SlotReleased;
use wl_kernel::{PartyId, Subject};

use crate::booking::Booking;
use crate::config::config_of;
use crate::refusal::Refusal;
use crate::resource::ServiceType;
use crate::service::{self, Actor, NewBooking};
use crate::slots::{self, Exclude};
use crate::texts::english;

/// A slot is offered to the first entry that matches.
pub const OFFER: &str = "offer";
/// The party takes the offered slot.
pub const ACCEPT: &str = "accept";
/// The hold of an offer passed.
pub const LAPSE: &str = "lapse";
/// The entry's last day passed.
pub const EXPIRE: &str = "expire";
/// The entry is removed.
pub const WITHDRAW: &str = "withdraw";

/// Where a waiting list entry stands (§4.2).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WaitingStatus {
    /// Looking for a matching slot.
    #[default]
    Waiting,
    /// A slot was offered and is held.
    Offered,
    /// The party took the slot.
    Booked,
    /// The entry's day passed.
    Expired,
    /// Removed by the party or staff.
    Withdrawn,
}

/// A request for an earlier or unavailable slot.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_scheduling_waiting_entries",
    state_machine,
    index(status, service_type_id)
)]
pub struct WaitingEntry {
    /// The primary key.
    pub id: i64,
    /// Who waits.
    pub party_id: i64,
    /// The type of the subject (patient, vehicle, student), if any.
    pub subject_type: Option<String>,
    /// The id of the subject.
    pub subject_id: Option<i64>,
    /// What is waited for.
    #[model(references = "wl_scheduling_service_types")]
    pub service_type_id: i64,
    /// The resource the party prefers; any when absent.
    pub preferred_resource_id: Option<i64>,
    /// The earliest time the party can come.
    pub window_from: Option<DateTime>,
    /// The latest time the party can come.
    pub window_until: Option<DateTime>,
    /// The last day the entry is looked at.
    pub expires_on: Option<NaiveDate>,
    /// Where it stands; changed only by the machine.
    pub status: WaitingStatus,
    /// The resource of the slot on offer.
    pub offered_resource_id: Option<i64>,
    /// The start of the slot on offer.
    pub offered_from: Option<DateTime>,
    /// The end of the slot on offer.
    pub offered_until: Option<DateTime>,
    /// The slot is held for the party until then.
    pub offer_hold_until: Option<DateTime>,
    /// The booking the party took.
    pub booking_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl WaitingEntry {
    /// Who waits, as a party id.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// Whether the offer is still held at `at`.
    pub fn is_held(&self, at: DateTime) -> bool {
        self.status == WaitingStatus::Offered
            && self.offer_hold_until.is_some_and(|until| until > at)
    }

    /// Whether the slot `resource`, `start`..`end` of `service` fits what the
    /// entry waits for: the service, the preferred resource, the window.
    pub fn matches(
        &self,
        service_type_id: i64,
        resource: i64,
        start: DateTime,
        end: DateTime,
    ) -> bool {
        self.service_type_id == service_type_id
            && self.preferred_resource_id.is_none_or(|id| id == resource)
            && self.window_from.is_none_or(|from| from <= start)
            && self.window_until.is_none_or(|until| end <= until)
    }
}

/// The slot on offer, as the machine's input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfferedSlot {
    /// The resource.
    pub resource_id: i64,
    /// The start.
    pub from: DateTime,
    /// The end.
    pub until: DateTime,
}

/// What a run of a waiting-list transition needs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WaitingInput {
    /// The moment to judge by.
    pub now: Option<DateTime>,
    /// The slot offered (`offer`).
    pub slot: Option<OfferedSlot>,
    /// The offer is held until then (`offer`).
    pub hold_until: Option<DateTime>,
    /// The booking made (`accept`).
    pub booking_id: Option<i64>,
    /// The day to judge by (`expire`).
    pub today: Option<NaiveDate>,
}

impl WaitingInput {
    fn at(&self) -> DateTime {
        self.now.unwrap_or_else(now)
    }
}

fn input_of(ctx: &TransitionCtx) -> WaitingInput {
    ctx.input_as::<WaitingInput>().unwrap_or_default()
}

async fn set(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    WaitingEntry::query()
        .where_(WaitingEntry::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

/// Clears the offer columns.
async fn clear_offer(tx: &mut renox::db::Transaction, id: i64) -> Result {
    let (none_id, none_time): (Option<i64>, Option<DateTime>) = (None, None);
    set(
        tx,
        id,
        &[
            ("offered_resource_id", &none_id),
            ("offered_from", &none_time),
            ("offered_until", &none_time),
            ("offer_hold_until", &none_time),
        ],
    )
    .await
}

impl StateMachine for WaitingEntry {
    type State = WaitingStatus;
    const COLUMN: renox::db::Col<WaitingEntry, WaitingStatus> = WaitingEntry::STATUS;

    fn machine() -> Machine<Self> {
        use WaitingStatus::{Booked, Expired, Offered, Waiting, Withdrawn};
        Machine::new()
            .initial([Waiting])
            .state(Waiting)
            .tone("neutral")
            .state(Offered)
            .tone("warning")
            .state(Booked)
            .tone("success")
            .terminal()
            .state(Expired)
            .tone("neutral")
            .terminal()
            .state(Withdrawn)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(OFFER)
                    .label("Offer a slot")
                    .system()
                    .from([Waiting])
                    .to(Offered)
                    .guard("slot_matches", |entry: &WaitingEntry, ctx| {
                        let slot = input_of(ctx).slot;
                        let ok = slot.as_ref().is_some_and(|slot| {
                            entry.matches(
                                entry.service_type_id,
                                slot.resource_id,
                                slot.from,
                                slot.until,
                            )
                        });
                        Box::pin(async move {
                            if ok {
                                Ok(())
                            } else {
                                Err(english("wl_scheduling.problem.offer_mismatch", &[]))
                            }
                        })
                    })
                    .effect(|entry: &WaitingEntry, ctx, tx| {
                        let id = entry.id;
                        let input = input_of(ctx);
                        Box::pin(async move {
                            let Some(slot) = input.slot else {
                                return Ok(());
                            };
                            let (resource, from, until) =
                                (Some(slot.resource_id), Some(slot.from), Some(slot.until));
                            set(
                                tx,
                                id,
                                &[
                                    ("offered_resource_id", &resource),
                                    ("offered_from", &from),
                                    ("offered_until", &until),
                                    ("offer_hold_until", &input.hold_until),
                                ],
                            )
                            .await
                        })
                    }),
            )
            .transition(
                Transition::new(ACCEPT)
                    .label("Take the slot")
                    .system()
                    .from([Offered])
                    .to(Booked)
                    .guard("offer_held", |entry: &WaitingEntry, ctx| {
                        let held = entry.is_held(input_of(ctx).at());
                        Box::pin(async move {
                            if held {
                                Ok(())
                            } else {
                                Err(english("wl_scheduling.problem.offer_expired", &[]))
                            }
                        })
                    })
                    .effect(|entry: &WaitingEntry, ctx, tx| {
                        let id = entry.id;
                        let booking = input_of(ctx).booking_id;
                        Box::pin(async move {
                            set(tx, id, &[("booking_id", &booking)]).await?;
                            clear_offer(tx, id).await
                        })
                    }),
            )
            .transition(
                Transition::new(LAPSE)
                    .label("Lapse")
                    .system()
                    .from([Offered])
                    .to(Waiting)
                    .guard("hold_passed", |entry: &WaitingEntry, ctx| {
                        let held = entry.is_held(input_of(ctx).at());
                        Box::pin(async move {
                            if held {
                                Err(english("wl_scheduling.problem.offer_still_held", &[]))
                            } else {
                                Ok(())
                            }
                        })
                    })
                    .effect(|entry: &WaitingEntry, _ctx, tx| {
                        let id = entry.id;
                        Box::pin(async move { clear_offer(tx, id).await })
                    }),
            )
            .transition(
                Transition::new(EXPIRE)
                    .label("Expire")
                    .system()
                    .from([Waiting])
                    .to(Expired)
                    .guard("day_passed", |entry: &WaitingEntry, ctx| {
                        let today = input_of(ctx).today;
                        let passed = match (entry.expires_on, today) {
                            (Some(last), Some(today)) => last < today,
                            _ => false,
                        };
                        Box::pin(async move {
                            if passed {
                                Ok(())
                            } else {
                                Err(english("wl_scheduling.problem.not_expired", &[]))
                            }
                        })
                    }),
            )
            .transition(
                Transition::new(WITHDRAW)
                    .label("Withdraw")
                    .system()
                    .from([Waiting, Offered])
                    .to(Withdrawn)
                    .effect(|entry: &WaitingEntry, _ctx, tx| {
                        let id = entry.id;
                        Box::pin(async move { clear_offer(tx, id).await })
                    }),
            )
    }
}

/// A request to wait for a slot.
#[derive(Debug, Clone)]
pub struct NewEntry {
    /// Who waits.
    pub party: PartyId,
    /// The patient, vehicle or student, if any.
    pub subject: Option<Subject>,
    /// What is waited for.
    pub service_type_id: i64,
    /// The preferred resource.
    pub resource_id: Option<i64>,
    /// The earliest time the party can come.
    pub window_from: Option<DateTime>,
    /// The latest time the party can come.
    pub window_until: Option<DateTime>,
    /// The last day the entry is looked at.
    pub expires_on: Option<NaiveDate>,
}

impl NewEntry {
    /// An entry for `party` waiting for `service_type_id`, any resource, any
    /// time.
    pub fn new(party: PartyId, service_type_id: i64) -> Self {
        NewEntry {
            party,
            subject: None,
            service_type_id,
            resource_id: None,
            window_from: None,
            window_until: None,
            expires_on: None,
        }
    }

    /// Only with this resource.
    pub fn with(mut self, resource_id: i64) -> Self {
        self.resource_id = Some(resource_id);
        self
    }

    /// Only inside this window.
    pub fn between(mut self, from: DateTime, until: DateTime) -> Self {
        self.window_from = Some(from);
        self.window_until = Some(until);
        self
    }
}

fn actor_meta(actor: Actor) -> renox::serde_json::Value {
    renox::serde_json::json!({ "actor": actor.kind.as_str(), "user": actor.user })
}

/// The party joins the waiting list.
pub async fn join(state: &AppState, new: NewEntry) -> std::result::Result<WaitingEntry, Refusal> {
    let service = ServiceType::find(&state.db, new.service_type_id)
        .await?
        .filter(|service| service.active)
        .ok_or_else(|| Refusal::invalid("wl_scheduling.problem.unknown_service"))?;
    if let Some(id) = new.resource_id {
        let offered = crate::resource::ServiceResource::query()
            .where_(crate::resource::ServiceResource::SERVICE_TYPE_ID.eq(service.id))
            .where_(crate::resource::ServiceResource::RESOURCE_ID.eq(id))
            .exists(&state.db)
            .await?;
        if !offered {
            return Err(Refusal::invalid("wl_scheduling.problem.not_offered"));
        }
    }
    let mut entry = WaitingEntry {
        party_id: new.party.0,
        subject_type: new.subject.as_ref().map(|s| s.subject_type.clone()),
        subject_id: new.subject.as_ref().map(|s| s.subject_id),
        service_type_id: service.id,
        preferred_resource_id: new.resource_id,
        window_from: new.window_from,
        window_until: new.window_until,
        expires_on: new.expires_on,
        status: WaitingStatus::Waiting,
        ..WaitingEntry::default()
    };
    let mut tx = state.db.begin().await?;
    entry
        .insert_in_state(&mut tx, WaitingStatus::Waiting, "join")
        .await?;
    tx.commit().await?;
    Ok(WaitingEntry::find_or_404(&state.db, entry.id).await?)
}

/// The first entry, in the order they joined, that waits for a slot of
/// `service_type_id` on `resource` from `start` to `end` (`SCH.RUL.03`).
pub async fn first_match(
    db: &Db,
    service_type_id: i64,
    resource: i64,
    start: DateTime,
    end: DateTime,
) -> Result<Option<WaitingEntry>> {
    Ok(WaitingEntry::query()
        .where_(WaitingEntry::SERVICE_TYPE_ID.eq(service_type_id))
        .where_(WaitingEntry::STATUS.eq(WaitingStatus::Waiting))
        .order_by(WaitingEntry::ID)
        .get(db)
        .await?
        .into_iter()
        .find(|entry| entry.matches(service_type_id, resource, start, end)))
}

/// Offers the slot of `resource` from `start` to `end` to entry `id` and
/// holds it for the offer hold time. The slot must be free: the check and the
/// hold happen under the lock of the resource, like a booking.
pub async fn offer(
    state: &AppState,
    id: i64,
    resource_id: i64,
    start: DateTime,
    end: DateTime,
    actor: Actor,
    at: DateTime,
) -> std::result::Result<WaitingEntry, Refusal> {
    let entry = WaitingEntry::find_or_404(&state.db, id).await?;
    let service = ServiceType::find_or_404(&state.db, entry.service_type_id).await?;
    let mut tx = state.db.begin_immediate().await?;
    let locked = slots::lock(&mut tx, &[resource_id]).await?;
    if locked.is_empty() {
        return Err(Refusal::invalid("wl_scheduling.problem.unknown_resource"));
    }
    if let Err(unfit) = slots::check_free(
        &mut tx,
        &service,
        &locked,
        start,
        end,
        at,
        Exclude::default(),
    )
    .await?
    {
        tx.rollback().await?;
        return Err(service::refusal_for(unfit));
    }
    let input = WaitingInput {
        now: Some(at),
        slot: Some(OfferedSlot {
            resource_id,
            from: start,
            until: end,
        }),
        hold_until: Some(at + TimeDelta::minutes(config_of(state).offer_hold_minutes)),
        ..WaitingInput::default()
    };
    let outcome = entry
        .transition(OFFER)
        .by_system()
        .input(&input)
        .meta(actor_meta(actor))
        .run_in(&mut tx)
        .await?;
    tx.commit().await?;
    outcome.emit(state).await?;
    Ok(WaitingEntry::find_or_404(&state.db, id).await?)
}

/// `SCH.RUL.03`: the slot a `SlotReleased` freed is offered to the first
/// waiting entry that matches it, in the order the entries joined, and held
/// for the offer hold time. One slot is offered to one entry. A slot that is
/// no longer free (another booker was first) is skipped.
///
/// # Errors
///
/// A database error.
pub async fn offer_released(
    state: &AppState,
    released: &SlotReleased,
    at: DateTime,
) -> Result<Option<WaitingEntry>> {
    let Some(service) = ServiceType::query()
        .where_(ServiceType::CODE.eq(released.service_type.as_str()))
        .first(&state.db)
        .await?
    else {
        return Ok(None);
    };
    for resource in &released.resources {
        let Some(entry) = first_match(
            &state.db,
            service.id,
            *resource,
            released.start,
            released.end,
        )
        .await?
        else {
            continue;
        };
        match offer(
            state,
            entry.id,
            *resource,
            released.start,
            released.end,
            Actor::system(),
            at,
        )
        .await
        {
            Ok(offered) => return Ok(Some(offered)),
            Err(Refusal::Failed(error)) => return Err(error),
            Err(_) => {}
        }
    }
    Ok(None)
}

/// The party takes the offered slot (`accept`): a booking is made from it
/// and the entry is `booked`. The offer must still be held.
pub async fn accept(
    state: &AppState,
    id: i64,
    actor: Actor,
    at: DateTime,
) -> std::result::Result<Booking, Refusal> {
    let entry = WaitingEntry::find_or_404(&state.db, id).await?;
    if !entry.is_held(at) {
        return Err(Refusal::invalid("wl_scheduling.problem.offer_expired"));
    }
    let (Some(resource), Some(start)) = (entry.offered_resource_id, entry.offered_from) else {
        return Err(Refusal::invalid("wl_scheduling.problem.offer_expired"));
    };
    let mut new = NewBooking::new(entry.party(), entry.service_type_id, &[resource], start);
    new.subject = match (&entry.subject_type, entry.subject_id) {
        (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
        _ => None,
    };
    // The entry's own hold must not stop its own booking.
    let booking = service::book_from_offer(state, new, entry.id, actor, at).await?;
    let input = WaitingInput {
        now: Some(at),
        booking_id: Some(booking.id),
        ..WaitingInput::default()
    };
    let _done = entry
        .transition(ACCEPT)
        .by_system()
        .input(&input)
        .meta(actor_meta(actor))
        .run(state)
        .await?;
    Ok(booking)
}

/// The hold of an offer passed (`lapse`): the entry waits again, and the next
/// one can be offered the slot.
pub async fn lapse(
    state: &AppState,
    id: i64,
    at: DateTime,
) -> std::result::Result<WaitingEntry, Refusal> {
    let entry = WaitingEntry::find_or_404(&state.db, id).await?;
    let input = WaitingInput {
        now: Some(at),
        ..WaitingInput::default()
    };
    let _done = entry
        .transition(LAPSE)
        .by_system()
        .input(&input)
        .meta(actor_meta(Actor::system()))
        .run(state)
        .await?;
    Ok(WaitingEntry::find_or_404(&state.db, id).await?)
}

/// The entry's last day passed (`expire`).
pub async fn expire(
    state: &AppState,
    id: i64,
    today: NaiveDate,
) -> std::result::Result<WaitingEntry, Refusal> {
    let entry = WaitingEntry::find_or_404(&state.db, id).await?;
    let input = WaitingInput {
        today: Some(today),
        ..WaitingInput::default()
    };
    let _done = entry
        .transition(EXPIRE)
        .by_system()
        .input(&input)
        .meta(actor_meta(Actor::system()))
        .run(state)
        .await?;
    Ok(WaitingEntry::find_or_404(&state.db, id).await?)
}

/// The party or staff remove the entry (`withdraw`); a held slot is freed.
pub async fn withdraw(
    state: &AppState,
    id: i64,
    actor: Actor,
) -> std::result::Result<WaitingEntry, Refusal> {
    let entry = WaitingEntry::find_or_404(&state.db, id).await?;
    let _done = entry
        .transition(WITHDRAW)
        .by_system()
        .meta(actor_meta(actor))
        .run(state)
        .await?;
    Ok(WaitingEntry::find_or_404(&state.db, id).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: u32) -> DateTime {
        wl_kernel::calendar::BusinessCalendar::default().from_local(
            NaiveDate::from_ymd_opt(2026, 11, 3)
                .unwrap()
                .and_hms_opt(hour, 0, 0)
                .unwrap(),
        )
    }

    #[test]
    fn the_machine_has_the_states_and_transitions_of_the_spec() {
        let machine = WaitingEntry::machine();
        let states: Vec<&str> = machine.states().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            states,
            ["waiting", "offered", "booked", "expired", "withdrawn"]
        );
        let row = |name: &str| {
            let t = machine.find(name).unwrap();
            (t.from_states().to_vec(), t.to_state().unwrap().to_owned())
        };
        assert_eq!(
            row(OFFER),
            (vec!["waiting".to_owned()], "offered".to_owned())
        );
        assert_eq!(
            row(ACCEPT),
            (vec!["offered".to_owned()], "booked".to_owned())
        );
        assert_eq!(
            row(LAPSE),
            (vec!["offered".to_owned()], "waiting".to_owned())
        );
        assert_eq!(
            row(EXPIRE),
            (vec!["waiting".to_owned()], "expired".to_owned())
        );
        assert_eq!(
            row(WITHDRAW),
            (
                vec!["waiting".to_owned(), "offered".to_owned()],
                "withdrawn".to_owned()
            )
        );
    }

    #[test]
    fn an_entry_matches_a_slot_of_its_service_resource_and_window() {
        let entry = WaitingEntry {
            service_type_id: 4,
            preferred_resource_id: Some(9),
            window_from: Some(at(9)),
            window_until: Some(at(12)),
            ..WaitingEntry::default()
        };
        assert!(entry.matches(4, 9, at(10), at(11)));
        assert!(!entry.matches(5, 9, at(10), at(11)));
        assert!(!entry.matches(4, 8, at(10), at(11)));
        assert!(!entry.matches(4, 9, at(8), at(9)));
        assert!(!entry.matches(4, 9, at(11), at(13)));
        let any = WaitingEntry {
            service_type_id: 4,
            ..WaitingEntry::default()
        };
        assert!(any.matches(4, 1, at(3), at(4)));
    }

    #[test]
    fn an_offer_holds_until_its_time() {
        let entry = WaitingEntry {
            status: WaitingStatus::Offered,
            offer_hold_until: Some(at(10)),
            ..WaitingEntry::default()
        };
        assert!(entry.is_held(at(9)));
        assert!(!entry.is_held(at(10)));
        let waiting = WaitingEntry {
            offer_hold_until: Some(at(10)),
            ..WaitingEntry::default()
        };
        assert!(!waiting.is_held(at(9)));
    }
}
