//! The Manpower request (09 Manpower §6 *Manpower request*, §7.1 *Manpower
//! request*): the client's order for workers, or the cover request of
//! `MPW.DEP.02` (a request of the kind [`RequestKind::Replacement`], see
//! [`crate::cover`]).
//!
//! ```text
//! draft --submit--> open
//! open, partially_filled --fill_partly--> partially_filled   (system: a deployment `activate`)
//! open, partially_filled --fill--> filled                    (system: a deployment `activate`)
//! draft, open, partially_filled --cancel--> cancelled        (client approver or requests.manage)
//! draft, open, partially_filled --auto_cancel--> cancelled   (system: contract `terminate` or `end`)
//! ```
//!
//! The spec's one `fill` goes to `partially_filled` or to `filled`; a Renox
//! transition has one target, so `fill_partly` and `fill` are two, and the
//! deployment `activate` picks the one that fits the headcount. Both count the
//! deployment (`filled_count`); a deployment that ends does not reopen the
//! request (§7.1 *Deployment* `end`).
//!
//! - `submit` needs the contract `active` (not `on_hold`, nor `terminating`,
//!   `MPW.ACL.14`) and a rate card that covers the role and the shift (the
//!   kernel's `RateCards`). The client approver submits for their own company
//!   (`MPW.ACL.01`), staff with `requests.create` for the client. **The
//!   refusal while the contract is on hold needs `hold`, which comes with
//!   BL-MPW-01; the guard on the status is built here and BL-MPW-02 tests it
//!   with the hold. The client-blocked guard (`PartyBlocked`) comes with
//!   BL-MPW-15.**
//! - `cancel` needs a reason; the planned deployments of the request are
//!   cancelled with it ([`crate::requests::cancel`]). `auto_cancel` is the
//!   system's, for the contract's `terminate` (BL-MPW-04) and `end`
//!   (BL-MPW-06).

use renox::auth::permissions::Scope;
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::{PartyId, Subject, SubjectMachine};

use crate::db::set_request;
use crate::permissions;

/// The subject type of a manpower request.
pub const SUBJECT_TYPE: &str = "manpower.request";

/// The `submit` transition's name.
pub const SUBMIT: &str = "submit";
/// The system's `fill` while headcount positions are still open.
pub const FILL_PARTLY: &str = "fill_partly";
/// The system's `fill` of the last position.
pub const FILL: &str = "fill";
/// The `cancel` transition's name.
pub const CANCEL: &str = "cancel";
/// The system's `cancel` (the contract ended or was terminated).
pub const AUTO_CANCEL: &str = "auto_cancel";

/// Where a request stands (§7.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RequestStatus {
    /// Being written by the client or the account manager.
    #[default]
    Draft,
    /// Confirmed; workers being found.
    Open,
    /// Some positions deployed.
    PartiallyFilled,
    /// All positions deployed (terminal).
    Filled,
    /// Withdrawn by the client or us (terminal).
    Cancelled,
}

impl RequestStatus {
    /// Whether deployments may still be planned and activated for it.
    pub fn takes_workers(self) -> bool {
        matches!(self, RequestStatus::Open | RequestStatus::PartiallyFilled)
    }
}

/// What a request is for.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RequestKind {
    /// The client's order for workers.
    #[default]
    Standard,
    /// Cover for a worker on leave, suspended or rejected (`MPW.DEP.02`).
    Replacement,
}

/// A manpower request.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "manpower_requests",
    state_machine,
    index(client_contract_id),
    index(status)
)]
pub struct ManpowerRequest {
    /// The primary key.
    pub id: i64,
    /// The client contract it is under.
    pub client_contract_id: i64,
    /// The client company, a wl-parties party.
    pub client_party_id: i64,
    /// The site the workers go to.
    pub site_id: i64,
    /// The role asked for.
    pub role: String,
    /// How many workers.
    pub headcount: i64,
    /// The shift type the rate card prices.
    pub shift_type: String,
    /// The shift start as `HH:MM` (Asia/Dubai).
    pub shift_start: Option<String>,
    /// The shift end as `HH:MM`.
    pub shift_end: Option<String>,
    /// The first day.
    pub start_date: NaiveDate,
    /// The last day; none for an open-ended order.
    pub end_date: Option<NaiveDate>,
    /// A client order or a cover request.
    pub kind: RequestKind,
    /// A cover request: `leave`, `suspension` or `rejection`.
    pub cause: Option<String>,
    /// A cover request: what asked for it, one request per cause.
    pub cause_ref: Option<String>,
    /// A cover request: the deployment it stands in for.
    pub replaces_deployment_id: Option<i64>,
    /// How many deployments were activated for it.
    pub filled_count: i64,
    /// The lifecycle state; changed only by a transition.
    pub status: RequestStatus,
    /// The user who submitted it.
    pub submitted_by: Option<i64>,
    /// When it was submitted.
    pub submitted_at: Option<DateTime>,
    /// Why it was cancelled.
    pub cancel_reason: Option<String>,
    /// The user who created it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ManpowerRequest {
    /// The request as a [`Subject`]: `manpower.request:4`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The client as a [`PartyId`].
    pub fn client(&self) -> PartyId {
        PartyId(self.client_party_id)
    }

    /// The scope roles are assigned in: the client company.
    pub fn scope(&self) -> Scope {
        permissions::client_scope(self.client_party_id)
    }

    /// The positions not yet deployed.
    pub fn missing(&self) -> i64 {
        (self.headcount - self.filled_count).max(0)
    }
}

/// What a run carries to the guards: the checks the service made.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestInput {
    /// Every check that fails, one line each, gathered by the service before
    /// the run (`submit`); the guard refuses while any is left.
    #[serde(default)]
    pub problems: Vec<String>,
    /// The user who runs it, for the record.
    #[serde(default)]
    pub by: Option<i64>,
}

/// The input of a run; the default without one.
pub(crate) fn input_of(ctx: &TransitionCtx) -> RequestInput {
    ctx.input_as::<RequestInput>().unwrap_or_default()
}

/// The effect of a cancel: the reason is kept.
fn cancelled(transition: Transition<ManpowerRequest>) -> Transition<ManpowerRequest> {
    transition
        .reason_required()
        .from([
            RequestStatus::Draft,
            RequestStatus::Open,
            RequestStatus::PartiallyFilled,
        ])
        .to(RequestStatus::Cancelled)
        .effect(|request: &ManpowerRequest, ctx, tx| {
            let id = request.id;
            let reason = ctx.reason().map(|reason| reason.trim().to_owned());
            Box::pin(async move { set_request(&mut *tx, id, &[("cancel_reason", &reason)]).await })
        })
}

/// A fill: counts the deployment, and refuses a request that is not open or
/// that `positions_after` would overfill or underfill for this target.
fn filled(
    transition: Transition<ManpowerRequest>,
    to: RequestStatus,
) -> Transition<ManpowerRequest> {
    transition
        .system()
        .from([RequestStatus::Open, RequestStatus::PartiallyFilled])
        .to(to)
        .guard("positions", move |request: &ManpowerRequest, _ctx| {
            let after = request.filled_count + 1;
            let fits = if to == RequestStatus::Filled {
                after == request.headcount
            } else {
                after < request.headcount
            };
            Box::pin(async move {
                if fits {
                    Ok(())
                } else {
                    Err(crate::texts::english("manpower.request.positions", &[]))
                }
            })
        })
        .effect(|request: &ManpowerRequest, _ctx, tx| {
            let id = request.id;
            let count = request.filled_count + 1;
            Box::pin(async move { set_request(&mut *tx, id, &[("filled_count", &count)]).await })
        })
}

impl StateMachine for ManpowerRequest {
    type State = RequestStatus;
    const COLUMN: renox::db::Col<ManpowerRequest, RequestStatus> = ManpowerRequest::STATUS;

    fn machine() -> Machine<Self> {
        use RequestStatus::{Cancelled, Draft, Filled, Open, PartiallyFilled};
        // `submit` and `cancel` are open to any signed-in user at the machine:
        // the client approver is not a holder of a permission (`MPW.ACL.01`
        // is the portal grant), so [`crate::requests`] decides who may.
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Open)
            .tone("info")
            .state(PartiallyFilled)
            .tone("info")
            .state(Filled)
            .tone("success")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(SUBMIT)
                    .label("Submit")
                    .tone("primary")
                    .from([Draft])
                    .to(Open)
                    .guard("ready", |_request: &ManpowerRequest, ctx| {
                        let problems = input_of(ctx).problems;
                        Box::pin(async move {
                            if problems.is_empty() {
                                Ok(())
                            } else {
                                Err(problems.join("\n"))
                            }
                        })
                    })
                    .effect(|request: &ManpowerRequest, ctx, tx| {
                        let id = request.id;
                        let by = input_of(ctx).by;
                        Box::pin(async move {
                            set_request(
                                &mut *tx,
                                id,
                                &[
                                    ("submitted_by", &by),
                                    ("submitted_at", &Some(renox::db::now())),
                                ],
                            )
                            .await
                        })
                    }),
            )
            .transition(filled(
                Transition::new(FILL_PARTLY).label("Fill"),
                PartiallyFilled,
            ))
            .transition(filled(Transition::new(FILL).label("Fill"), Filled))
            .transition(cancelled(
                Transition::new(CANCEL).label("Cancel").tone("danger"),
            ))
            .transition(cancelled(
                Transition::new(AUTO_CANCEL).label("Cancel").system(),
            ))
    }
}

/// The Manpower request in the kernel's registry, with the client company as
/// the scope where approvers are looked for.
pub fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<ManpowerRequest>(SUBJECT_TYPE)
        .scope(|request: &ManpowerRequest| request.scope())
        .build()
}

/// Registers the machine with the kernel and with `rnx state:diagram`.
pub(crate) fn register() {
    subject_machine().register();
    renox::state_machine::register::<ManpowerRequest>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_of_the_spec_is_declared() {
        let machine = ManpowerRequest::machine();
        let table: [(&str, &[&str], &str, bool); 5] = [
            (SUBMIT, &["draft"], "open", false),
            (
                FILL_PARTLY,
                &["open", "partially_filled"],
                "partially_filled",
                true,
            ),
            (FILL, &["open", "partially_filled"], "filled", true),
            (
                CANCEL,
                &["draft", "open", "partially_filled"],
                "cancelled",
                false,
            ),
            (
                AUTO_CANCEL,
                &["draft", "open", "partially_filled"],
                "cancelled",
                true,
            ),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to, by_system) in &table {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), *from, "{name}");
            assert_eq!(transition.to_state(), Some(*to), "{name}");
            assert_eq!(transition.is_system(), *by_system, "{name}");
        }
        assert!(machine.find(CANCEL).expect("cancel").needs_reason());
        assert!(machine.find(AUTO_CANCEL).expect("cancel").needs_reason());
    }

    #[test]
    fn two_states_are_terminal_and_every_state_has_a_tone() {
        let machine = ManpowerRequest::machine();
        let terminal: Vec<_> = machine
            .states()
            .iter()
            .filter(|state| state.terminal)
            .map(|state| state.name.as_str())
            .collect();
        assert_eq!(terminal, ["filled", "cancelled"]);
        assert!(machine.states().iter().all(|state| state.tone.is_some()));
    }

    #[test]
    fn only_an_open_request_takes_workers() {
        assert!(RequestStatus::Open.takes_workers());
        assert!(RequestStatus::PartiallyFilled.takes_workers());
        for status in [
            RequestStatus::Draft,
            RequestStatus::Filled,
            RequestStatus::Cancelled,
        ] {
            assert!(!status.takes_workers(), "{status:?}");
        }
    }

    #[test]
    fn the_positions_missing_never_go_below_zero() {
        let request = ManpowerRequest {
            headcount: 3,
            filled_count: 1,
            ..ManpowerRequest::default()
        };
        assert_eq!(request.missing(), 2);
        let full = ManpowerRequest {
            headcount: 1,
            filled_count: 2,
            ..ManpowerRequest::default()
        };
        assert_eq!(full.missing(), 0);
    }
}
