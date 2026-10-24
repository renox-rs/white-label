//! The agent access policy (BL-AGT-06, decision 0006 point 7): whether the
//! company lets AI assistants in at all, and for which record types.
//!
//! **Off by default.** BL-AGT-06 (the *Settings → AI assistants* page, the
//! admin's notice and the per-type lists) is `blocked:open-item` on L-40, so
//! this file is only its seam: the app provides an [`AgentPolicy`] with
//! [`provide_policy`] (or `App::provide` of an `Arc<dyn AgentPolicy>`), and
//! without one [`TurnedOff`] refuses every agent call with
//! [`TURNED_OFF`]. A deployment that turns agents on before BL-AGT-06 is built
//! provides [`AgentsOn`], explicitly.
//!
//! The policy only narrows: whatever it allows, the user's own permissions,
//! scopes and record rules still apply on every call.

use std::sync::Arc;

use renox::prelude::*;

use crate::machines::BoxFuture;

/// The refusal when the company has not turned assistants on.
pub const TURNED_OFF: &str = "AI assistants are turned off for this company";

/// What an agent wants to do with a record type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Use {
    /// Read it (search, fetch, related, inbox).
    Read,
    /// Run an action on it (preview, confirm).
    Act,
}

/// Decides whether an agent call may go ahead, before the user's own rights
/// are checked.
pub trait AgentPolicy: Send + Sync + 'static {
    /// `Ok(())` when `user` may use `subject_type` (or the agent at all, for
    /// `None`) this way through an assistant called `client`; otherwise the
    /// refusal, said to the assistant.
    fn check<'a>(
        &'a self,
        db: &'a Db,
        user: &'a AuthUser,
        client: &'a str,
        subject_type: Option<&'a str>,
        use_: Use,
    ) -> BoxFuture<'a, std::result::Result<(), String>>;
}

/// The default: every agent call is refused.
#[derive(Debug, Clone, Copy, Default)]
pub struct TurnedOff;

impl AgentPolicy for TurnedOff {
    fn check<'a>(
        &'a self,
        _db: &'a Db,
        _user: &'a AuthUser,
        _client: &'a str,
        _subject_type: Option<&'a str>,
        _use: Use,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async { Err(TURNED_OFF.to_owned()) })
    }
}

/// Assistants on, for the types listed (all types when no list is given).
/// Read and act lists are separate, as BL-AGT-06 decides: actions are
/// allowed only on the types named for actions.
#[derive(Debug, Clone, Default)]
pub struct AgentsOn {
    read: Option<Vec<String>>,
    act: Option<Vec<String>>,
}

impl AgentsOn {
    /// On for every type, to read and to act.
    pub fn all() -> Self {
        AgentsOn::default()
    }

    /// Only these types may be read.
    pub fn read_only<S: Into<String>>(mut self, types: impl IntoIterator<Item = S>) -> Self {
        self.read = Some(types.into_iter().map(Into::into).collect());
        self
    }

    /// Only these types may be acted on.
    pub fn act_only<S: Into<String>>(mut self, types: impl IntoIterator<Item = S>) -> Self {
        self.act = Some(types.into_iter().map(Into::into).collect());
        self
    }

    /// Whether `subject_type` may be used this way.
    pub fn allows(&self, subject_type: Option<&str>, use_: Use) -> bool {
        let list = match use_ {
            Use::Read => &self.read,
            Use::Act => &self.act,
        };
        match (list, subject_type) {
            (None, _) | (Some(_), None) => true,
            (Some(types), Some(wanted)) => types.iter().any(|t| t == wanted),
        }
    }
}

impl AgentPolicy for AgentsOn {
    fn check<'a>(
        &'a self,
        _db: &'a Db,
        _user: &'a AuthUser,
        _client: &'a str,
        subject_type: Option<&'a str>,
        use_: Use,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        let allowed = self.allows(subject_type, use_);
        Box::pin(async move {
            if allowed {
                Ok(())
            } else {
                Err(format!(
                    "AI assistants may not {} this kind of record in this company",
                    match use_ {
                        Use::Read => "read",
                        Use::Act => "act on",
                    }
                ))
            }
        })
    }
}

/// Makes `policy` the app's agent policy.
pub fn provide_policy(app: &mut Registry, policy: impl AgentPolicy) {
    let policy: Arc<dyn AgentPolicy> = Arc::new(policy);
    app.provide(policy);
}

/// `policy` as the value `App::provide` takes:
/// `app.provide(wl_kernel::agent::access::policy(AgentsOn::all()))`.
pub fn policy(policy: impl AgentPolicy) -> Arc<dyn AgentPolicy> {
    Arc::new(policy)
}

/// The app's agent policy, or [`TurnedOff`] when none is provided.
pub fn policy_of(state: &AppState) -> Arc<dyn AgentPolicy> {
    match state.provided::<Arc<dyn AgentPolicy>>() {
        Some(policy) => Arc::clone(&policy),
        None => Arc::new(TurnedOff),
    }
}

/// Checks the policy for `call`; a refusal is a 403 with its words.
pub async fn check(call: &super::AgentCall, subject_type: Option<&str>, use_: Use) -> Result<()> {
    let policy = policy_of(&call.state);
    policy
        .check(&call.state.db, &call.user, &call.client, subject_type, use_)
        .await
        .map_err(|refusal| Error::Status(StatusCode::FORBIDDEN, refusal))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lists_narrow_reads_and_actions_apart() {
        let all = AgentsOn::all();
        assert!(all.allows(Some("invoicing.invoice"), Use::Act));
        let narrow = AgentsOn::all()
            .read_only(["ticketing.ticket"])
            .act_only(Vec::<String>::new());
        assert!(narrow.allows(Some("ticketing.ticket"), Use::Read));
        assert!(!narrow.allows(Some("invoicing.invoice"), Use::Read));
        assert!(!narrow.allows(Some("ticketing.ticket"), Use::Act));
        // A call about no record type (the inbox) needs the agent on only.
        assert!(narrow.allows(None, Use::Read));
    }

    #[test]
    fn the_refusal_names_the_company_setting() {
        assert_eq!(TURNED_OFF, "AI assistants are turned off for this company");
    }
}
