//! The Ticket state machine (wl-ticketing §4.1).
//!
//! ```text
//! [*] --> open --assign--> assigned --assign--> assigned
//!         assigned --start--> in_progress --hold--> on_hold --resume--> in_progress
//!         in_progress --resolve--> resolved --confirm | close--> closed (terminal)
//!         resolved --reopen--> in_progress
//!         open | assigned | on_hold --cancel--> cancelled (terminal)
//! ```
//!
//! Every state is declared here with its tone, and a new ticket is inserted
//! only as `open`. `assign` is in [`crate::assign`], the others in
//! [`crate::work`]. `Ticket` is `#[model(state_machine)]`, so
//! `save` refuses a changed status.

use renox::state_machine::{Machine, StateMachine};

use crate::ticket::{Ticket, TicketStatus};

impl StateMachine for Ticket {
    type State = TicketStatus;
    const COLUMN: renox::db::Col<Ticket, TicketStatus> = Ticket::STATUS;

    fn machine() -> Machine<Self> {
        use TicketStatus::{Assigned, Cancelled, Closed, InProgress, OnHold, Open, Resolved};
        let machine = Machine::new()
            .initial([Open])
            .state(Open)
            .tone("neutral")
            .state(Assigned)
            .tone("info")
            .state(InProgress)
            .tone("info")
            .state(OnHold)
            .tone("warning")
            .state(Resolved)
            .tone("success")
            .state(Closed)
            .tone("success")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal();
        [
            crate::assign::transition(),
            crate::assign::auto_transition(),
        ]
        .into_iter()
        .chain(crate::work::transitions())
        .fold(machine, Machine::transition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_is_declared_with_its_tone() {
        let machine = Ticket::machine();
        let states: Vec<_> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("open", Some("neutral"), false),
                ("assigned", Some("info"), false),
                ("in_progress", Some("info"), false),
                ("on_hold", Some("warning"), false),
                ("resolved", Some("success"), false),
                ("closed", Some("success"), true),
                ("cancelled", Some("neutral"), true),
            ]
        );
        assert_eq!(machine.initial_states(), ["open"]);
    }

    #[test]
    fn every_transition_exists() {
        let machine = Ticket::machine();
        let expected = [
            ("assign", vec!["open", "assigned"], "assigned", false),
            ("auto_assign", vec!["open", "assigned"], "assigned", true),
            ("start", vec!["assigned"], "in_progress", false),
            ("hold", vec!["in_progress"], "on_hold", false),
            ("resume", vec!["on_hold"], "in_progress", false),
            ("auto_resume", vec!["on_hold"], "in_progress", true),
            ("resolve", vec!["in_progress"], "resolved", false),
            ("confirm", vec!["resolved"], "closed", false),
            ("close", vec!["resolved"], "closed", false),
            ("reopen", vec!["resolved"], "in_progress", false),
            (
                "cancel",
                vec!["open", "assigned", "on_hold"],
                "cancelled",
                false,
            ),
            (
                "auto_cancel",
                vec!["open", "assigned", "on_hold"],
                "cancelled",
                true,
            ),
        ];
        assert_eq!(machine.transitions().len(), expected.len());
        for (name, from, to, system) in expected {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.is_system(), system, "{name}");
        }
    }

    #[test]
    fn cancel_needs_a_reason_and_the_others_do_not() {
        let machine = Ticket::machine();
        for transition in machine.transitions() {
            let cancels = transition.name().ends_with("cancel");
            assert_eq!(transition.needs_reason(), cancels, "{}", transition.name());
        }
    }

    #[test]
    fn the_guards_are_those_of_the_plan() {
        let machine = Ticket::machine();
        let guards = |name: &str| {
            machine
                .find(name)
                .expect("transition")
                .guard_names()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(guards("assign"), ["input", "triage", "change", "reason"]);
        assert_eq!(
            guards("auto_assign"),
            ["input", "triage", "change", "reason"]
        );
        assert_eq!(guards("start"), ["input", "by_assignee"]);
        assert_eq!(guards("hold"), ["input", "by_assignee", "hold_reason"]);
        assert_eq!(guards("resume"), ["input", "by_assignee"]);
        assert_eq!(guards("resolve"), ["input", "by_assignee"]);
        assert_eq!(guards("confirm"), ["by_requester"]);
        assert_eq!(guards("close"), ["input", "signoff"]);
        assert!(guards("cancel").is_empty());
    }
}
