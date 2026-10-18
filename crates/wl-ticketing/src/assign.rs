//! Assigning a ticket (wl-ticketing §4.1, §7; #146 A9, A10, A12): `assign`
//! and the reassignment.
//!
//! One transition, `assigned` or `open` → `assigned`, with three cases that
//! [`classify`] tells apart from the ticket and the request:
//!
//! - `open`: the **first** assignment. It needs no reason and starts the SLA:
//!   `sla_started_at`, the response due and the resolution due are written
//!   `WHERE sla_started_at IS NULL`, so no later assign can restart or
//!   recompute them, whatever input it brings.
//! - `assigned` to a **different** assignee: a **reassignment**. It needs a
//!   known reason code (and a text for a code like `other`). The priority
//!   may change; the due times never do.
//! - `assigned` to the **same** assignee with another visit time: a **visit
//!   change**, with no reason. The same assignee and visit is refused with
//!   "nothing changed".
//!
//! Every run writes an [`Assignment`] row in the transition's effect, the
//! reassignment history. [`assign`] checks the assignee (a staff user that
//! exists, or a vendor party that the party directory finds and that is
//! `active`) and works out the SLA due times with the calendar, because an
//! effect gets no `AppState`. The notices and `TicketAssigned` follow the
//! commit ([`crate::notices`]).

use renox::db::{DateTime, ToDbValue, Transaction};
use renox::prelude::*;
use renox::serde_json::{self, Value};
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::parties::{PartyStatus, directory};
use wl_kernel::tickets::Assignee;

use crate::config::SlaPolicy;
use crate::permissions;
use crate::priority::Priority;
use crate::reasons::{ReasonRefused, ReassignReasons};
use crate::sla::sla_calendar;
use crate::ticket::{Ticket, TicketStatus};

/// The transition's name.
pub const ASSIGN: &str = "assign";

/// What an `assign` did.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AssignKind {
    /// The first assignment of an `open` ticket; starts the SLA.
    #[default]
    First,
    /// The ticket went to a different assignee, for a reason.
    Reassign,
    /// The same assignee with another visit time.
    VisitChange,
}

/// One row of the reassignment history (#146 A12), written by every
/// `assign`.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_ticketing_assignments")]
pub struct Assignment {
    /// The primary key.
    pub id: i64,
    /// The ticket.
    #[model(references = "wl_ticketing_tickets")]
    pub ticket_id: i64,
    /// What the assign did.
    pub kind: AssignKind,
    /// The staff user who had the ticket before, if one did.
    pub previous_assignee_user_id: Option<i64>,
    /// The vendor party that had the ticket before, if one did.
    pub previous_assignee_party_id: Option<i64>,
    /// The staff user it went to, if a user.
    pub assignee_user_id: Option<i64>,
    /// The vendor party it went to, if a party.
    pub assignee_party_id: Option<i64>,
    /// The reassignment reason code; `None` unless a reassignment.
    pub reason_code: Option<String>,
    /// The explanation given with the reason.
    pub reason_text: Option<String>,
    /// The visit time set with it, if any.
    pub visit_at: Option<DateTime>,
    /// The user who assigned.
    pub by_user_id: Option<i64>,
    /// When it happened.
    pub at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Assignment {
    /// The assignee before this assign, if there was one.
    pub fn previous_assignee(&self) -> Option<Assignee> {
        of_columns(
            self.previous_assignee_user_id,
            self.previous_assignee_party_id,
        )
    }

    /// The assignee after this assign.
    pub fn assignee(&self) -> Option<Assignee> {
        of_columns(self.assignee_user_id, self.assignee_party_id)
    }

    /// The history of `ticket_id`, oldest first.
    pub async fn history(db: &Db, ticket_id: i64) -> Result<Vec<Assignment>> {
        Assignment::query()
            .where_(Assignment::TICKET_ID.eq(ticket_id))
            .order_by(Assignment::ID)
            .get(db)
            .await
    }
}

fn of_columns(user: Option<i64>, party: Option<i64>) -> Option<Assignee> {
    match (user, party) {
        (Some(id), _) => Some(Assignee::User { id }),
        (None, Some(id)) => Some(Assignee::Party { id: PartyId(id) }),
        (None, None) => None,
    }
}

/// An assignee as the user and party columns.
fn columns(assignee: Option<Assignee>) -> (Option<i64>, Option<i64>) {
    match assignee {
        Some(Assignee::User { id }) => (Some(id), None),
        Some(Assignee::Party { id }) => (None, Some(id.0)),
        None => (None, None),
    }
}

/// What the user asks for when assigning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignRequest {
    /// Who gets the ticket: a staff user or a vendor party.
    pub assignee: Assignee,
    /// The priority; it picks the SLA targets of a first assignment.
    pub priority: Priority,
    /// When the visit is scheduled, if it is.
    pub visit_at: Option<DateTime>,
    /// The reassignment reason code (a reassignment only).
    pub reason_code: Option<String>,
    /// The explanation given with the reason.
    pub reason_text: Option<String>,
}

/// What an assign carries to its guards and effect, worked out by
/// [`assign`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AssignInput {
    /// When the assign happens.
    pub at: DateTime,
    /// Who gets the ticket.
    pub assignee: Assignee,
    /// The priority.
    pub priority: Priority,
    /// The visit time.
    #[serde(default)]
    pub visit_at: Option<DateTime>,
    /// The reassignment reason code.
    #[serde(default)]
    pub reason_code: Option<String>,
    /// The explanation.
    #[serde(default)]
    pub reason_text: Option<String>,
    /// The response due time, used only by the first assignment.
    #[serde(default)]
    pub response_due: Option<DateTime>,
    /// The resolution due time, used only by the first assignment.
    #[serde(default)]
    pub resolution_due: Option<DateTime>,
}

/// What the history row's `meta` and `Transitioned` carry, for the listener
/// ([`crate::notices`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct AssignMeta {
    /// The kind, as stored (`first`, `reassign`, `visit_change`).
    pub kind: String,
    /// Who had the ticket before.
    pub previous_assignee: Option<Assignee>,
    /// Who has it now.
    pub assignee: Assignee,
    /// The reason code of a reassignment.
    pub reason_code: Option<String>,
    /// The explanation.
    pub reason_text: Option<String>,
    /// The visit time.
    pub visit_at: Option<DateTime>,
}

impl AssignMeta {
    /// The kind, if it is one this module wrote.
    pub fn kind(&self) -> Option<AssignKind> {
        self.kind.parse().ok()
    }
}

/// The run's input, if it is an [`AssignInput`].
fn input(ctx: &TransitionCtx) -> Option<AssignInput> {
    ctx.input_as::<AssignInput>().ok()
}

fn same_second(a: Option<DateTime>, b: Option<DateTime>) -> bool {
    a.map(|d| d.timestamp()) == b.map(|d| d.timestamp())
}

/// What assigning `ticket` to `assignee` with `visit_at` is: the first
/// assignment, a reassignment, a visit change, or refused with the reason.
pub(crate) fn classify(
    ticket: &Ticket,
    assignee: Assignee,
    visit_at: Option<DateTime>,
) -> std::result::Result<AssignKind, String> {
    match ticket.status {
        TicketStatus::Open => Ok(AssignKind::First),
        TicketStatus::Assigned => {
            if ticket.assignee() != Some(assignee) {
                Ok(AssignKind::Reassign)
            } else if same_second(ticket.visit_at, visit_at) {
                Err("Nothing changed: the ticket has this assignee and visit time.".to_owned())
            } else {
                Ok(AssignKind::VisitChange)
            }
        }
        other => Err(format!("A ticket that is {other} cannot be assigned.")),
    }
}

/// `assign` for the machine: open or assigned → assigned, by
/// `tickets.assign`, only through [`assign`].
pub(crate) fn transition() -> Transition<Ticket> {
    Transition::new(ASSIGN)
        .label("Assign")
        .tone("primary")
        .permission(permissions::ASSIGN)
        .from([TicketStatus::Open, TicketStatus::Assigned])
        .to(TicketStatus::Assigned)
        .guard("input", |_ticket: &Ticket, ctx| {
            let present = input(ctx).is_some();
            Box::pin(async move {
                if present {
                    Ok(())
                } else {
                    Err("The action was not prepared by the ticket service.".to_owned())
                }
            })
        })
        .guard("triage", |ticket: &Ticket, _ctx| {
            let triaged = ticket.category_id != 0;
            Box::pin(async move {
                if triaged {
                    Ok(())
                } else {
                    Err("A ticket needs a category before it is assigned.".to_owned())
                }
            })
        })
        .guard("change", |ticket: &Ticket, ctx| {
            let verdict = match input(ctx) {
                Some(i) => classify(ticket, i.assignee, i.visit_at).map(|_| ()),
                None => Ok(()),
            };
            Box::pin(async move { verdict })
        })
        .guard("reason", |ticket: &Ticket, ctx| {
            let missing = input(ctx).is_some_and(|i| {
                classify(ticket, i.assignee, i.visit_at) == Ok(AssignKind::Reassign)
                    && i.reason_code.as_deref().is_none_or(|c| c.trim().is_empty())
            });
            Box::pin(async move {
                if missing {
                    Err("A reassignment needs a reason.".to_owned())
                } else {
                    Ok(())
                }
            })
        })
        .effect(|ticket: &Ticket, ctx, tx| {
            let input = input(ctx);
            let by = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                let input = input.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("an assign without its input"))
                })?;
                apply(tx, ticket, &input, by).await
            })
        })
}

/// The effect: the new assignee, visit and priority on the ticket, the SLA
/// start where it never started, and the history row.
pub(crate) async fn apply(
    tx: &mut Transaction,
    ticket: &Ticket,
    input: &AssignInput,
    by: Option<i64>,
) -> Result<()> {
    let kind = classify(ticket, input.assignee, input.visit_at)
        .map_err(|message| Error::from(renox::anyhow::anyhow!(message)))?;
    let (user_id, party_id) = columns(Some(input.assignee));
    let values: Vec<(&str, &(dyn ToDbValue + Sync))> = vec![
        ("assignee_user_id", &user_id),
        ("assignee_party_id", &party_id),
        ("visit_at", &input.visit_at),
        ("priority", &input.priority),
    ];
    Ticket::query()
        .where_(Ticket::ID.eq(ticket.id))
        .update(&mut *tx, &values)
        .await?;
    // Only a ticket whose SLA never started gets its due times, so a
    // reassignment cannot restart them.
    let started = Some(input.at);
    Ticket::query()
        .where_(Ticket::ID.eq(ticket.id))
        .where_(Ticket::SLA_STARTED_AT.is_null())
        .update(
            &mut *tx,
            &[
                ("sla_started_at", &started),
                ("response_due", &input.response_due),
                ("resolution_due", &input.resolution_due),
            ],
        )
        .await?;
    let reassigned = kind == AssignKind::Reassign;
    let (previous_user, previous_party) = columns(ticket.assignee());
    let mut row = Assignment {
        ticket_id: ticket.id,
        kind,
        previous_assignee_user_id: previous_user,
        previous_assignee_party_id: previous_party,
        assignee_user_id: user_id,
        assignee_party_id: party_id,
        reason_code: reassigned.then(|| input.reason_code.clone()).flatten(),
        reason_text: reassigned.then(|| input.reason_text.clone()).flatten(),
        visit_at: input.visit_at,
        by_user_id: by,
        at: Some(input.at),
        ..Assignment::default()
    };
    row.insert(&mut *tx).await?;
    Ok(())
}

fn refused(message: impl Into<String>) -> TransitionError {
    TransitionError::Guard(message.into())
}

/// Checks the assignee: a staff user that exists, or a vendor party that
/// the party directory finds and that is `active` (a blocked one is refused
/// until #37 brings `BlockGuard`).
async fn check_assignee(
    state: &AppState,
    assignee: Assignee,
) -> std::result::Result<(), TransitionError> {
    match assignee {
        Assignee::User { id } => {
            if User::find(&state.db, id).await?.is_none() {
                return Err(refused("The assignee is not a user."));
            }
        }
        Assignee::Party { id } => match directory(state).status(&state.db, id).await? {
            Some(PartyStatus::Active) => {}
            Some(_) => {
                return Err(refused(
                    "The vendor is not active, so it cannot be assigned.",
                ));
            }
            None => return Err(refused("The vendor is not a known party.")),
        },
    }
    Ok(())
}

/// The reasons the app accepts: the §7 defaults and a vertical's own.
pub(crate) fn reasons_of(state: &AppState) -> ReassignReasons {
    state
        .provided::<ReassignReasons>()
        .map_or_else(ReassignReasons::default, |r| (*r).clone())
}

/// The SLA due times a first assignment at `at` gets for `priority`: the
/// response and resolution targets of the ticket's policy, in working time.
async fn due_times(
    state: &AppState,
    ticket: &Ticket,
    priority: Priority,
    at: DateTime,
) -> Result<(Option<DateTime>, Option<DateTime>)> {
    let Some(policy_id) = ticket.sla_policy_id else {
        return Ok((None, None));
    };
    let Some(policy) = SlaPolicy::find(&state.db, policy_id).await? else {
        return Ok((None, None));
    };
    let Some(target) = policy.target(&state.db, priority).await? else {
        return Ok((None, None));
    };
    let calendar = sla_calendar(state);
    Ok((
        Some(calendar.add_working(at, target.response_minutes)),
        Some(calendar.add_working(at, target.resolution_minutes)),
    ))
}

/// `user` assigns `ticket` at `at` as `request` says: the first assignment,
/// a reassignment or a visit change. After the commit, the notices and
/// `TicketAssigned` follow.
///
/// # Errors
///
/// `Forbidden` without `tickets.assign`; `NotFrom` unless it is `open` or
/// `assigned`; `Guard` (422) for an unknown user, an unknown or blocked
/// vendor, a reassignment without a known reason (or without text where the
/// reason needs it), or nothing changed.
pub async fn assign(
    state: &AppState,
    user: &AuthUser,
    ticket: &Ticket,
    request: AssignRequest,
    at: DateTime,
) -> std::result::Result<AssignKind, TransitionError> {
    if !matches!(ticket.status, TicketStatus::Open | TicketStatus::Assigned) {
        return Err(TransitionError::NotFrom(ticket.status.to_string()));
    }
    if !user.has_permission(permissions::ASSIGN) {
        return Err(TransitionError::Forbidden);
    }
    let kind = classify(ticket, request.assignee, request.visit_at).map_err(refused)?;
    check_assignee(state, request.assignee).await?;
    let code = request
        .reason_code
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    let text = request
        .reason_text
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    // Only a reassignment has a reason.
    let (code, text) = if kind == AssignKind::Reassign {
        let Some(code) = code else {
            return Err(refused("A reassignment needs a reason."));
        };
        match reasons_of(state).check(code, text.unwrap_or_default()) {
            Ok(()) => {}
            Err(ReasonRefused::Unknown) => {
                return Err(refused(format!("`{code}` is not a reassignment reason.")));
            }
            Err(ReasonRefused::TextRequired) => {
                return Err(refused("This reason needs an explanation."));
            }
        }
        (Some(code.to_owned()), text.map(str::to_owned))
    } else {
        (None, None)
    };
    let (response_due, resolution_due) = if ticket.sla_started_at.is_none() {
        due_times(state, ticket, request.priority, at).await?
    } else {
        (None, None)
    };
    let input = AssignInput {
        at,
        assignee: request.assignee,
        priority: request.priority,
        visit_at: request.visit_at,
        reason_code: code.clone(),
        reason_text: text.clone(),
        response_due,
        resolution_due,
    };
    let meta = AssignMeta {
        kind: kind.as_str().to_owned(),
        previous_assignee: ticket.assignee(),
        assignee: request.assignee,
        reason_code: code.clone(),
        reason_text: text,
        visit_at: request.visit_at,
    };
    let run = ticket
        .transition(ASSIGN)
        .by(user)
        .input(&input)
        .meta(serde_json::to_value(&meta).unwrap_or(Value::Null));
    let run = match code {
        Some(code) => run.reason(code),
        None => run,
    };
    let _outcome = run.run(state).await?;
    Ok(kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::NaiveDate;
    use renox::testing::TestApp;
    use wl_kernel::BusinessCalendar;

    use crate::config::Category;
    use crate::seed;

    fn dubai(date: &str, hour: u32) -> DateTime {
        let local = date
            .parse::<NaiveDate>()
            .expect("a date")
            .and_hms_opt(hour, 0, 0)
            .expect("a time");
        BusinessCalendar::default().from_local(local)
    }

    const COOLAIR: Assignee = Assignee::Party { id: PartyId(5) };
    const IMRAN: Assignee = Assignee::User { id: 7 };

    fn ticket(status: TicketStatus, assignee: Option<Assignee>) -> Ticket {
        let (assignee_user_id, assignee_party_id) = columns(assignee);
        Ticket {
            id: 1,
            category_id: 1,
            status,
            assignee_user_id,
            assignee_party_id,
            visit_at: Some(dubai("2026-11-04", 10)),
            ..Ticket::default()
        }
    }

    fn input_for(assignee: Assignee, visit_at: Option<DateTime>) -> AssignInput {
        AssignInput {
            at: dubai("2026-11-03", 13),
            assignee,
            priority: Priority::High,
            visit_at,
            reason_code: Some("vendor_no_show".into()),
            reason_text: None,
            response_due: Some(dubai("2026-11-03", 22)),
            resolution_due: Some(dubai("2026-11-09", 22)),
        }
    }

    fn ctx(input: &AssignInput) -> TransitionCtx {
        TransitionCtx::new(None).with_input(serde_json::to_value(input).expect("json"))
    }

    #[test]
    fn the_cases_of_an_assign_are_told_apart() {
        let visit = Some(dubai("2026-11-04", 10));
        let open = ticket(TicketStatus::Open, None);
        assert_eq!(classify(&open, COOLAIR, None), Ok(AssignKind::First));
        let assigned = ticket(TicketStatus::Assigned, Some(COOLAIR));
        assert_eq!(classify(&assigned, IMRAN, visit), Ok(AssignKind::Reassign));
        assert_eq!(
            classify(&assigned, COOLAIR, Some(dubai("2026-11-05", 10))),
            Ok(AssignKind::VisitChange)
        );
        // Clearing the visit is a change too.
        assert_eq!(
            classify(&assigned, COOLAIR, None),
            Ok(AssignKind::VisitChange)
        );
        assert!(
            classify(&assigned, COOLAIR, visit)
                .unwrap_err()
                .contains("Nothing changed")
        );
        // A party and a user with the same number are different assignees.
        let by_user = ticket(TicketStatus::Assigned, Some(Assignee::User { id: 5 }));
        assert_eq!(classify(&by_user, COOLAIR, visit), Ok(AssignKind::Reassign));
        for status in [
            TicketStatus::InProgress,
            TicketStatus::OnHold,
            TicketStatus::Resolved,
            TicketStatus::Closed,
            TicketStatus::Cancelled,
        ] {
            assert!(classify(&ticket(status, Some(COOLAIR)), IMRAN, visit).is_err());
        }
    }

    #[renox::test]
    async fn the_guards_refuse_a_missing_input_a_missing_reason_and_no_change() {
        let machine = Ticket::machine();
        let assign = machine.find(ASSIGN).expect("assign");
        let assigned = ticket(TicketStatus::Assigned, Some(COOLAIR));
        let visit = Some(dubai("2026-11-04", 10));

        let bare = TransitionCtx::new(None);
        assert!(matches!(
            assign.check_guards(&assigned, &bare).await,
            Err(TransitionError::Guard(_))
        ));
        // A reassignment with its reason passes.
        let with_reason = ctx(&input_for(IMRAN, visit));
        assert!(assign.check_guards(&assigned, &with_reason).await.is_ok());
        // Without a reason, or with a blank one, it does not.
        for code in [None, Some("  ".to_owned())] {
            let input = AssignInput {
                reason_code: code,
                ..input_for(IMRAN, visit)
            };
            assert!(matches!(
                assign.check_guards(&assigned, &ctx(&input)).await,
                Err(TransitionError::Guard(_))
            ));
        }
        // The first assignment and a visit change need no reason.
        let no_reason = |assignee, visit| AssignInput {
            reason_code: None,
            ..input_for(assignee, visit)
        };
        let open = ticket(TicketStatus::Open, None);
        assert!(
            assign
                .check_guards(&open, &ctx(&no_reason(COOLAIR, None)))
                .await
                .is_ok()
        );
        let moved = no_reason(COOLAIR, Some(dubai("2026-11-05", 10)));
        assert!(assign.check_guards(&assigned, &ctx(&moved)).await.is_ok());
        // The same assignee and visit is refused.
        let same = no_reason(COOLAIR, visit);
        assert!(matches!(
            assign.check_guards(&assigned, &ctx(&same)).await,
            Err(TransitionError::Guard(_))
        ));
        // A ticket without a category is not triaged.
        let uncategorised = Ticket {
            category_id: 0,
            ..open
        };
        assert!(matches!(
            assign
                .check_guards(&uncategorised, &ctx(&no_reason(COOLAIR, None)))
                .await,
            Err(TransitionError::Guard(_))
        ));
    }

    async fn app() -> TestApp {
        let app = TestApp::new(
            App::new()
                .module(Auth::new())
                .module(crate::WlTicketing::new()),
        )
        .await;
        seed::install(app.db()).await.expect("seeded");
        app
    }

    async fn born(
        app: &TestApp,
        number: &str,
        status: TicketStatus,
        assignee: Option<Assignee>,
    ) -> Ticket {
        let general = Category::by_key(app.db(), "general")
            .await
            .unwrap()
            .unwrap();
        let (assignee_user_id, assignee_party_id) = columns(assignee);
        let started = (status != TicketStatus::Open).then(|| dubai("2026-11-03", 9));
        let mut ticket = Ticket {
            number: number.to_owned(),
            subject_type: "property.unit".into(),
            subject_id: 12,
            category_id: general.id,
            sla_policy_id: general.sla_policy_id,
            priority: Priority::High,
            title: "Chiller".into(),
            assignee_user_id,
            assignee_party_id,
            sla_started_at: started,
            response_due: started.map(|_| dubai("2026-11-03", 17)),
            resolution_due: started.map(|_| dubai("2026-11-05", 18)),
            status,
            ..Ticket::default()
        };
        let mut tx = app.db().begin().await.unwrap();
        ticket
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        ticket
    }

    async fn applied(app: &TestApp, ticket: &Ticket, input: &AssignInput) {
        let mut tx = app.db().begin().await.unwrap();
        apply(&mut tx, ticket, input, Some(3)).await.unwrap();
        tx.commit().await.unwrap();
    }

    #[renox::test]
    async fn the_first_assign_starts_the_sla_and_writes_a_first_row() {
        let app = app().await;
        let open = born(&app, "TKT-2026-9101", TicketStatus::Open, None).await;
        let input = AssignInput {
            at: dubai("2026-11-03", 9),
            response_due: Some(dubai("2026-11-03", 17)),
            resolution_due: Some(dubai("2026-11-05", 18)),
            reason_code: None,
            ..input_for(COOLAIR, None)
        };
        applied(&app, &open, &input).await;
        let ticket = Ticket::find_or_404(app.db(), open.id).await.unwrap();
        assert_eq!(ticket.assignee(), Some(COOLAIR));
        assert_eq!(ticket.priority, Priority::High);
        assert_eq!(ticket.sla_started_at, Some(dubai("2026-11-03", 9)));
        assert_eq!(ticket.response_due, Some(dubai("2026-11-03", 17)));
        assert_eq!(ticket.resolution_due, Some(dubai("2026-11-05", 18)));
        let history = Assignment::history(app.db(), open.id).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].kind, AssignKind::First);
        assert_eq!(history[0].previous_assignee(), None);
        assert_eq!(history[0].assignee(), Some(COOLAIR));
        assert_eq!(history[0].by_user_id, Some(3));
    }

    #[renox::test]
    async fn a_reassignment_never_restarts_the_sla_whatever_the_input_brings() {
        let app = app().await;
        let assigned = born(&app, "TKT-2026-9102", TicketStatus::Assigned, Some(COOLAIR)).await;
        // The input carries other due times and a new priority.
        let input = AssignInput {
            priority: Priority::Urgent,
            reason_text: Some("No show".into()),
            ..input_for(IMRAN, None)
        };
        applied(&app, &assigned, &input).await;
        let ticket = Ticket::find_or_404(app.db(), assigned.id).await.unwrap();
        assert_eq!(ticket.assignee(), Some(IMRAN));
        assert_eq!(ticket.assignee_party_id, None);
        assert_eq!(ticket.priority, Priority::Urgent);
        assert_eq!(ticket.visit_at, None);
        assert_eq!(ticket.sla_started_at, assigned.sla_started_at);
        assert_eq!(ticket.response_due, Some(dubai("2026-11-03", 17)));
        assert_eq!(ticket.resolution_due, Some(dubai("2026-11-05", 18)));
        let history = Assignment::history(app.db(), assigned.id).await.unwrap();
        assert_eq!(history.len(), 1);
        let row = &history[0];
        assert_eq!(row.kind, AssignKind::Reassign);
        assert_eq!(row.previous_assignee(), Some(COOLAIR));
        assert_eq!(row.assignee(), Some(IMRAN));
        assert_eq!(row.reason_code.as_deref(), Some("vendor_no_show"));
        assert_eq!(row.reason_text.as_deref(), Some("No show"));
        assert_eq!(row.at, Some(dubai("2026-11-03", 13)));
    }

    #[renox::test]
    async fn a_visit_change_keeps_the_assignee_and_has_no_reason() {
        let app = app().await;
        let assigned = born(&app, "TKT-2026-9103", TicketStatus::Assigned, Some(COOLAIR)).await;
        let input = AssignInput {
            reason_code: Some("workload".into()),
            ..input_for(COOLAIR, Some(dubai("2026-11-05", 10)))
        };
        applied(&app, &assigned, &input).await;
        let ticket = Ticket::find_or_404(app.db(), assigned.id).await.unwrap();
        assert_eq!(ticket.assignee(), Some(COOLAIR));
        assert_eq!(ticket.visit_at, Some(dubai("2026-11-05", 10)));
        let history = Assignment::history(app.db(), assigned.id).await.unwrap();
        assert_eq!(history[0].kind, AssignKind::VisitChange);
        assert_eq!(history[0].reason_code, None);
        assert_eq!(history[0].previous_assignee(), Some(COOLAIR));
    }

    #[renox::test]
    async fn the_first_assignment_gets_the_due_times_of_its_policy_and_priority() {
        let app = app().await;
        let open = born(&app, "TKT-2026-9104", TicketStatus::Open, None).await;
        let at = dubai("2026-11-03", 9);
        // `general` is on the `standard` policy: High is 8 h / 27 h.
        let (response, resolution) = due_times(app.state(), &open, Priority::High, at)
            .await
            .unwrap();
        assert_eq!(response, Some(dubai("2026-11-03", 17)));
        assert_eq!(resolution, Some(dubai("2026-11-05", 18)));
        // Another priority picks other targets: Urgent is 2 h / 24 h.
        let (urgent, _) = due_times(app.state(), &open, Priority::Urgent, at)
            .await
            .unwrap();
        assert_eq!(urgent, Some(dubai("2026-11-03", 11)));
        // No policy, no due times.
        let none = Ticket {
            sla_policy_id: None,
            ..open
        };
        assert_eq!(
            due_times(app.state(), &none, Priority::High, at)
                .await
                .unwrap(),
            (None, None)
        );
    }

    #[renox::test]
    async fn the_assignee_must_exist_and_a_party_must_be_known() {
        let app = app().await;
        let missing = check_assignee(app.state(), Assignee::User { id: 99_999 }).await;
        assert!(matches!(missing, Err(TransitionError::Guard(_))));
        // Without the parties module, no vendor is known.
        let unknown = check_assignee(app.state(), COOLAIR).await;
        assert!(matches!(unknown, Err(TransitionError::Guard(_))));
    }

    #[test]
    fn meta_round_trips_for_the_listener() {
        let meta = AssignMeta {
            kind: AssignKind::Reassign.as_str().to_owned(),
            previous_assignee: Some(COOLAIR),
            assignee: IMRAN,
            reason_code: Some("vendor_no_show".into()),
            reason_text: None,
            visit_at: None,
        };
        let value = serde_json::to_value(&meta).unwrap();
        let back: AssignMeta = serde_json::from_value(value).unwrap();
        assert_eq!(back, meta);
        assert_eq!(back.kind(), Some(AssignKind::Reassign));
    }
}
