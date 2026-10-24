//! AI assistants that work in the app (decision 0006, `PLT.AGT.02`,
//! `PLT.AGT.03`, BL-AGT-08 to BL-AGT-10): the business parts the MCP
//! connector exposes as tools.
//!
//! - [`actions`]: the **agent actions** a module registers (a chosen
//!   transition, a create, a message), each run in two steps: `preview`
//!   (writes nothing; says what will change, the amounts, whether an approval
//!   will be requested, and returns a confirmation token valid 10 minutes),
//!   then `confirm` (one run, idempotent: a second confirm with the same
//!   token returns the first result). Nothing is offered unless a module
//!   registers it; deciding an approval (`approve`, `approve_step`,
//!   `reject`) and `void` are never agent actions.
//! - [`inbox`]: what needs the user's attention now (approvals waiting for
//!   them, overdue items, mentions, notifications, failed automations), and
//!   `mark_read` for notifications.
//! - [`history`]: "by Budi via Claude" on a record's history for the
//!   transitions an agent ran.
//! - [`access`]: the per-client agent policy (BL-AGT-06). Off by default:
//!   every agent call is refused until the deployment provides a policy that
//!   turns it on (BL-AGT-06 is `blocked:open-item` on L-40, so only the seam
//!   is built here).
//! - [`tools`]: the list of tools the MCP connector (BL-AGT-07) serves, each
//!   with its JSON schema and handler. The kernel registers the action, inbox
//!   and `mark_read` tools; the connector reads [`tools::tools`] and calls
//!   [`tools::call`].
//! - [`web`]: `GET /agent/tools` and `POST /agent/tools/{tool}`, the same
//!   tools over JSON for a signed-in user (API token or session), with the
//!   assistant's name in the `X-Agent-Client` header. The MCP endpoint wraps
//!   the same calls.
//!
//! An agent is the user it acts for, never more: every action runs through
//! the record's state machine with the user as actor, so the same
//! permissions, guards, limits, segregation of duties and approvals apply as
//! on the screen.

pub mod access;
pub mod actions;
pub mod history;
pub mod inbox;
pub mod tools;
pub mod web;

use renox::prelude::*;
use serde::Serialize;

pub use access::{AgentPolicy, AgentsOn, TURNED_OFF, TurnedOff, Use};
pub use actions::{
    ActionKind, AgentAction, AgentRun, Amount, Change, Confirmed, Outcome, Planned, Preview,
    TransitionAction,
};
pub use inbox::{Inbox, InboxGroup, InboxItem, InboxSource};
pub use tools::{AgentTool, tools};

/// The key of the transition metadata that marks a run by an agent.
pub const META_KEY: &str = "agent";

/// The name used when a caller gives no assistant name.
pub const UNNAMED_CLIENT: &str = "an assistant";

/// One call of an agent: the app, the user it acts for and the assistant
/// (the OAuth client's name, e.g. `Claude`).
#[derive(Clone)]
pub struct AgentCall {
    /// The app.
    pub state: AppState,
    /// The user the agent acts for; every check is this user's.
    pub user: AuthUser,
    /// The assistant's name, shown as "via Claude".
    pub client: String,
}

impl AgentCall {
    /// A call by `client` for `user`. A blank client name becomes
    /// [`UNNAMED_CLIENT`].
    pub fn new(state: AppState, user: AuthUser, client: &str) -> Self {
        let client = client.trim();
        AgentCall {
            state,
            user,
            client: if client.is_empty() {
                UNNAMED_CLIENT.to_owned()
            } else {
                client.chars().take(60).collect()
            },
        }
    }

    /// The transition metadata of a run by this agent: `{"agent": {"client":
    /// "Claude", "user_id": 7}}`, read back by [`history::via`].
    pub fn meta(&self) -> renox::serde_json::Value {
        json!({ META_KEY: { "client": self.client, "user_id": self.user.id } })
    }
}

/// An amount as agent tools return it: minor units with the currency and
/// the number of decimals stated (decision 0006, BL-AGT-07).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Money {
    /// The amount in the currency's minor unit (fils for AED).
    pub minor: i64,
    /// The currency code, e.g. `AED`.
    pub currency: String,
    /// How many decimals the minor unit has (2 for AED).
    pub decimals: u8,
    /// The amount in words a person reads: `AED 1,050.00`.
    pub text: String,
}

impl Money {
    /// `minor` units of `currency` (two decimals).
    pub fn new(minor: i64, currency: &str) -> Self {
        Money {
            minor,
            currency: currency.to_owned(),
            decimals: 2,
            text: format!("{currency} {}", decimals_text(minor)),
        }
    }
}

/// `105000` as `1,050.00`.
fn decimals_text(minor: i64) -> String {
    let sign = if minor < 0 { "-" } else { "" };
    let abs = minor.unsigned_abs();
    let whole = (abs / 100).to_string();
    let mut grouped = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{sign}{grouped}.{:02}", abs % 100)
}

/// Registers the agent models, the kernel's tools and its inbox sources.
pub(crate) fn register(app: &mut Registry) {
    app.model::<AgentRun>();
    tools::register_kernel_tools();
    inbox::register_kernel_sources();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_states_the_currency_and_the_minor_unit() {
        let money = Money::new(105_000, "AED");
        assert_eq!(money.text, "AED 1,050.00");
        assert_eq!(money.decimals, 2);
        assert_eq!(Money::new(5, "AED").text, "AED 0.05");
        assert_eq!(Money::new(123_456_789, "AED").text, "AED 1,234,567.89");
        assert_eq!(Money::new(-2_500, "AED").text, "AED -25.00");
    }
}
