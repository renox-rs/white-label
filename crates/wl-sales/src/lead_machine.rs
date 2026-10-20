//! The Lead state machine (wl-sales §4.1, issue #139).
//!
//! ```text
//! [*] new --contact--> contacted --qualify--> qualified
//!         contacted | qualified --win--> won --revert_win--> qualified
//!         new | contacted | qualified --lose--> lost --reopen--> contacted
//! ```
//!
//! `contact`, `qualify`, `lose` and `reopen` are user transitions
//! (`leads.manage`); `win` and `revert_win` are system transitions, run by
//! the vertical's conversion event ([`crate::attribution`]) and by the void
//! of the conversion record ([`crate::lead_events`]).
//!
//! - `contact` needs the contact note, given as the run's reason; it is
//!   stored as a note of kind `contact`.
//! - `qualify` has no qualification fields to check yet: the per-vertical
//!   starter lists come with the verticals (§7).
//! - `win` needs the conversion record, given as the run's input
//!   ([`WinInput`]); it is stored on the lead with the win time.
//! - `lose` needs a reason, given as the run's reason; the page offers the
//!   configured list ([`crate::config::SalesConfig::lost_reasons`]). The lead
//!   then shows it, and *duplicate* and *spam or invalid* leave it out of
//!   every lead count.
//! - `revert_win` clears the conversion record; the run's metadata carries
//!   the voided record (`{"voided": "type:id"}`) for the event.

use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::lead::{Lead, LeadNote, LeadStatus};
use crate::permissions;
use crate::texts::english;

/// A first contact was made.
pub const CONTACT: &str = "contact";
/// Need, budget or timing are known.
pub const QUALIFY: &str = "qualify";
/// The vertical's conversion event: the lead converted.
pub const WIN: &str = "win";
/// The lead is not converting.
pub const LOSE: &str = "lose";
/// A lost lead is worked again.
pub const REOPEN: &str = "reopen";
/// The conversion record was voided as entered in error.
pub const REVERT_WIN: &str = "revert_win";

/// What `win` takes: the conversion record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WinInput {
    /// The vertical's conversion record.
    pub converted: Subject,
}

/// The reason of the run, trimmed; `None` when blank.
fn reason_of(ctx: &TransitionCtx) -> Option<String> {
    ctx.reason()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_owned)
}

/// Sets non-status columns of lead `id` outside the machine's guard and
/// stamps `updated_at`.
async fn set(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    Lead::query()
        .where_(Lead::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

impl StateMachine for Lead {
    type State = LeadStatus;
    const COLUMN: renox::db::Col<Lead, LeadStatus> = Lead::STATUS;

    fn machine() -> Machine<Self> {
        use LeadStatus::{Contacted, Lost, New, Qualified, Won};
        Machine::new()
            .initial([New])
            .state(New)
            .tone("neutral")
            .state(Contacted)
            .tone("info")
            .state(Qualified)
            .tone("info")
            .state(Won)
            .tone("success")
            .state(Lost)
            .tone("neutral")
            .transition(contact())
            .transition(qualify())
            .transition(win())
            .transition(lose())
            .transition(reopen())
            .transition(revert_win())
    }
}

/// `contact`: new → contacted, with the contact note.
fn contact() -> Transition<Lead> {
    Transition::new(CONTACT)
        .label("Contact")
        .tone("info")
        .permission(permissions::LEADS_MANAGE)
        .from([LeadStatus::New])
        .to(LeadStatus::Contacted)
        .guard("note_recorded", |_lead: &Lead, ctx| {
            let given = reason_of(ctx).is_some();
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(english("wl_sales.leads.problem.note", &[]))
                }
            })
        })
        .effect(|lead: &Lead, ctx, tx| {
            let (id, note, user) = (lead.id, reason_of(ctx), ctx.actor().map(|u| u.id));
            Box::pin(async move {
                if let Some(note) = note {
                    LeadNote {
                        lead_id: id,
                        kind: "contact".to_owned(),
                        note,
                        user_id: user,
                        ..LeadNote::default()
                    }
                    .insert(&mut *tx)
                    .await?;
                }
                Ok(())
            })
        })
}

/// `qualify`: contacted → qualified.
fn qualify() -> Transition<Lead> {
    Transition::new(QUALIFY)
        .label("Qualify")
        .tone("info")
        .permission(permissions::LEADS_MANAGE)
        .from([LeadStatus::Contacted])
        .to(LeadStatus::Qualified)
}

/// `win`: contacted | qualified → won, by the conversion event.
fn win() -> Transition<Lead> {
    Transition::new(WIN)
        .label("Win")
        .tone("success")
        .system()
        .from([LeadStatus::Contacted, LeadStatus::Qualified])
        .to(LeadStatus::Won)
        .guard("converted_given", |_lead: &Lead, ctx| {
            let given = ctx.input_as::<WinInput>().is_ok();
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(english("wl_sales.leads.problem.converted", &[]))
                }
            })
        })
        .effect(|lead: &Lead, ctx, tx| {
            let id = lead.id;
            let converted = ctx.input_as::<WinInput>().ok().map(|i| i.converted);
            Box::pin(async move {
                let Some(converted) = converted else {
                    return Ok(());
                };
                let kind = Some(converted.subject_type);
                let record = Some(converted.subject_id);
                let at = Some(now());
                set(
                    tx,
                    id,
                    &[
                        ("converted_type", &kind),
                        ("converted_id", &record),
                        ("won_at", &at),
                    ],
                )
                .await
            })
        })
}

/// `lose`: new | contacted | qualified → lost, with a reason from the list.
fn lose() -> Transition<Lead> {
    Transition::new(LOSE)
        .label("Lose")
        .tone("neutral")
        .permission(permissions::LEADS_MANAGE)
        .from(LeadStatus::OPEN)
        .to(LeadStatus::Lost)
        .guard("reason_given", |_lead: &Lead, ctx| {
            let reason = reason_of(ctx);
            Box::pin(async move {
                match reason {
                    Some(_) => Ok(()),
                    None => Err(english("wl_sales.leads.problem.lost_reason", &[])),
                }
            })
        })
        .effect(|lead: &Lead, ctx, tx| {
            let (id, reason) = (lead.id, reason_of(ctx));
            Box::pin(async move { set(tx, id, &[("lost_reason", &reason)]).await })
        })
}

/// `reopen`: lost → contacted.
fn reopen() -> Transition<Lead> {
    Transition::new(REOPEN)
        .label("Reopen")
        .tone("info")
        .permission(permissions::LEADS_MANAGE)
        .from([LeadStatus::Lost])
        .to(LeadStatus::Contacted)
        .effect(|lead: &Lead, _ctx, tx| {
            let id = lead.id;
            Box::pin(async move { set(tx, id, &[("lost_reason", &None::<String>)]).await })
        })
}

/// `revert_win`: won → qualified when the conversion record is voided.
fn revert_win() -> Transition<Lead> {
    Transition::new(REVERT_WIN)
        .label("Revert win")
        .tone("warning")
        .system()
        .from([LeadStatus::Won])
        .to(LeadStatus::Qualified)
        .effect(|lead: &Lead, _ctx, tx| {
            let id = lead.id;
            Box::pin(async move {
                set(
                    tx,
                    id,
                    &[
                        ("converted_type", &None::<String>),
                        ("converted_id", &None::<i64>),
                        ("won_at", &None::<renox::db::DateTime>),
                    ],
                )
                .await
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_of_the_spec_is_declared() {
        let machine = Lead::machine();
        let table = [
            (CONTACT, vec!["new"], "contacted", false),
            (QUALIFY, vec!["contacted"], "qualified", false),
            (WIN, vec!["contacted", "qualified"], "won", true),
            (LOSE, vec!["new", "contacted", "qualified"], "lost", false),
            (REOPEN, vec!["lost"], "contacted", false),
            (REVERT_WIN, vec!["won"], "qualified", true),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to, system) in table {
            let t = machine.find(name).expect(name);
            assert_eq!(t.is_system(), system, "{name}");
            assert_eq!(t.from_states(), from, "{name}");
            assert_eq!(t.to_state(), Some(to), "{name}");
        }
        assert_eq!(machine.initial_states(), ["new"]);
    }
}
