//! The Delivery state machine (wl-delivery §4.1).
//!
//! ```text
//! [*] --> pending --assign--> assigned --unassign--> pending
//!         assigned --dispatch (system, with the run)--> out_for_delivery
//!         out_for_delivery --deliver--> delivered
//!         out_for_delivery --fail--> attempted
//!         pending | assigned --cancel--> cancelled (terminal)
//! out_for_delivery --deliver_short (approval, WRK.ACL.09)--> delivered
//! delivered --open_review--> under_review --close_review--> delivered
//! delivered | under_review --reverse_delivery (approval, WRK.ACL.10)--> attempted
//!     --back_on_the_road (system, run still dispatched)--> out_for_delivery
//! out_for_delivery | attempted | under_review --mark_lost--> lost --found--> attempted
//! out_for_delivery --fail_aborted (system, the run was aborted)--> attempted
//! ```
//!
//! Every state of §4.1 is declared with its tone so the diagram is whole;
//! the transitions out of `attempted` come with their issues (BL-DLV-03,
//! BL-DLV-07). `assign`, `unassign`, `dispatch` and `cancel` are in
//! [`crate::plan`], `deliver` and `fail` in [`crate::proof`],
//! `open_review` and `close_review` in [`crate::review`], `reverse_delivery` (and
//! its approved twin) in [`crate::reverse`], `mark_lost` (and its
//! approved and system twins) and `found` in [`crate::lost`]. `Delivery` is
//! `#[model(state_machine)]`, so `save` refuses a changed status.

use renox::state_machine::{Machine, StateMachine};

use crate::delivery::{Delivery, DeliveryStatus};

impl StateMachine for Delivery {
    type State = DeliveryStatus;
    const COLUMN: renox::db::Col<Delivery, DeliveryStatus> = Delivery::STATUS;

    fn machine() -> Machine<Self> {
        use DeliveryStatus::{
            Assigned, Attempted, Cancelled, Delivered, Discarded, Lost, OutForDelivery, Pending,
            Returned, UnderReview,
        };
        let machine = Machine::new()
            .initial([Pending])
            .state(Pending)
            .tone("neutral")
            .state(Assigned)
            .tone("info")
            .state(OutForDelivery)
            .tone("info")
            .state(Attempted)
            .tone("warning")
            .state(Delivered)
            .tone("success")
            .state(UnderReview)
            .tone("warning")
            .state(Lost)
            .tone("error")
            .state(Returned)
            .tone("neutral")
            .terminal()
            .state(Discarded)
            .tone("neutral")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal();
        crate::plan::transitions()
            .into_iter()
            .chain(crate::proof::transitions())
            .chain(crate::review::transitions())
            .chain(crate::lost::transitions())
            .chain(crate::reverse::transitions())
            .chain(crate::abort::delivery_transitions())
            .fold(machine, Machine::transition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_is_declared_with_its_tone_and_delivered_is_not_terminal() {
        let machine = Delivery::machine();
        let states: Vec<_> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("pending", Some("neutral"), false),
                ("assigned", Some("info"), false),
                ("out_for_delivery", Some("info"), false),
                ("attempted", Some("warning"), false),
                ("delivered", Some("success"), false),
                ("under_review", Some("warning"), false),
                ("lost", Some("error"), false),
                ("returned", Some("neutral"), true),
                ("discarded", Some("neutral"), true),
                ("cancelled", Some("neutral"), true),
            ]
        );
        assert_eq!(machine.initial_states(), ["pending"]);
    }

    #[test]
    fn the_transitions_of_planning_the_stop_and_the_review_exist() {
        let machine = Delivery::machine();
        let expected = [
            ("assign", vec!["pending"], "assigned", false),
            ("unassign", vec!["assigned"], "pending", false),
            ("dispatch", vec!["assigned"], "out_for_delivery", true),
            ("cancel", vec!["pending", "assigned"], "cancelled", false),
            (
                "auto_cancel",
                vec!["pending", "assigned"],
                "cancelled",
                true,
            ),
            ("deliver", vec!["out_for_delivery"], "delivered", false),
            ("fail", vec!["out_for_delivery"], "attempted", false),
            (
                "deliver_short",
                vec!["out_for_delivery"],
                "delivered",
                false,
            ),
            (
                "deliver_short_approved",
                vec!["out_for_delivery"],
                "delivered",
                true,
            ),
            ("open_review", vec!["delivered"], "under_review", false),
            (
                "open_review_system",
                vec!["delivered"],
                "under_review",
                true,
            ),
            ("close_review", vec!["under_review"], "delivered", false),
            (
                "mark_lost",
                vec!["out_for_delivery", "attempted", "under_review"],
                "lost",
                false,
            ),
            (
                "mark_lost_approved",
                vec!["out_for_delivery", "attempted", "under_review"],
                "lost",
                true,
            ),
            (
                "mark_lost_system",
                vec!["out_for_delivery", "attempted", "under_review"],
                "lost",
                true,
            ),
            ("found", vec!["lost"], "attempted", false),
            (
                "reverse_delivery",
                vec!["delivered", "under_review"],
                "attempted",
                false,
            ),
            (
                "reverse_delivery_approved",
                vec!["delivered", "under_review"],
                "attempted",
                true,
            ),
            (
                "back_on_the_road",
                vec!["attempted"],
                "out_for_delivery",
                true,
            ),
            ("fail_aborted", vec!["out_for_delivery"], "attempted", true),
        ];
        assert_eq!(machine.transitions().len(), expected.len());
        for (name, from, to, system) in expected {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.is_system(), system, "{name}");
        }
    }
}
