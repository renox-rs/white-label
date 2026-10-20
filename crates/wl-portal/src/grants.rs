//! The access grant and its lifecycle (wl-portal §3 *Access grant*, §4, issue
//! #44): what a portal user may do for one party.
//!
//! ```text
//! [*] invited --accept--> active --suspend--> suspended --reinstate--> active
//!        |--expire--> expired      |                          |--reinstate_read_only--> read_only
//!        |--revoke--> revoked      |--end--> read_only --suspend--> suspended
//!                                  |--close--> closed  (also from read_only, suspended)
//! [*] active                       (self-registration)
//! ```
//!
//! Every transition is `.system()`: the services in [`crate::access`] check
//! who may act (`portal_users.invite`, `portal_users.manage`, the invitee, the
//! system) and run exactly one transition, so a person never runs a
//! transition by hand. `reinstate` returns the grant to the state it was
//! suspended from, which takes two transitions (`reinstate` to `active`,
//! `reinstate_read_only` to `read_only`); each guard checks the state the
//! suspension came from.
//!
//! The base is the lifecycle only. The invitation (token, expiry mail), the
//! portal roles' permissions, login methods and the close-after timer are
//! other issues; a grant keeps its roles as keys.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use crate::texts::english;

/// Where a grant stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GrantStatus {
    /// Invitation sent, not yet accepted.
    #[default]
    Invited,
    /// Full access for the grant's roles.
    Active,
    /// Temporarily blocked by staff (a dispute, a security concern).
    Suspended,
    /// The relationship ended; the user can still see and download history.
    ReadOnly,
    /// No access (terminal).
    Closed,
    /// Invitation not accepted in time (terminal).
    Expired,
    /// Invitation withdrawn before acceptance (terminal).
    Revoked,
}

/// An invitation is accepted.
pub const ACCEPT: &str = "accept";
/// An invitation was not accepted in time.
pub const EXPIRE: &str = "expire";
/// An invitation is withdrawn.
pub const REVOKE: &str = "revoke";
/// Staff block the grant for a while.
pub const SUSPEND: &str = "suspend";
/// A suspended grant that was `active` becomes `active` again.
pub const REINSTATE: &str = "reinstate";
/// A suspended grant that was `read_only` becomes `read_only` again.
pub const REINSTATE_READ_ONLY: &str = "reinstate_read_only";
/// The relationship ended: history only.
pub const END: &str = "end";
/// No access any more.
pub const CLOSE: &str = "close";

/// What a grant is created from.
pub const SOURCE_INVITATION: &str = "invitation";
/// A self-registered grant, directly `active`.
pub const SOURCE_SELF_REGISTRATION: &str = "self_registration";
/// A grant made by a relationship (a guardian for a dependant).
pub const SOURCE_RELATIONSHIP: &str = "relationship";

/// What a portal user may do for one party.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_portal_access_grants",
    state_machine,
    index(user_id, status),
    index(party_id, status)
)]
pub struct AccessGrant {
    /// The primary key.
    pub id: i64,
    /// The portal user; empty while the invitation is open.
    pub user_id: Option<i64>,
    /// The person party the portal user is linked to.
    pub person_party_id: i64,
    /// The party the user acts for: themselves, their company, a dependant.
    pub party_id: i64,
    /// The audience, e.g. `tenant`.
    pub audience: String,
    /// The portal role keys, comma-separated.
    pub roles: String,
    /// `invitation`, `self_registration` or `relationship`.
    pub source: String,
    /// The first day the grant counts.
    pub valid_from: NaiveDate,
    /// The last day it counts, when it ends on a date.
    pub valid_until: Option<NaiveDate>,
    /// Where it stands; changed only by the machine.
    pub status: GrantStatus,
    /// The state a suspension came from.
    pub suspended_from: Option<String>,
    /// Why it was suspended.
    pub suspend_reason: Option<String>,
    /// Why it ended or closed.
    pub ended_reason: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl AccessGrant {
    /// The party the user acts for.
    pub fn party(&self) -> wl_kernel::PartyId {
        wl_kernel::PartyId(self.party_id)
    }

    /// The role keys.
    pub fn role_keys(&self) -> Vec<String> {
        self.roles
            .split(',')
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// Whether the grant is `active` and `day` is inside its dates.
    pub fn is_active_on(&self, day: NaiveDate) -> bool {
        self.status == GrantStatus::Active
            && self.valid_from <= day
            && self.valid_until.is_none_or(|until| day <= until)
    }
}

/// What `accept` is given: the portal user the invitee became.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptInput {
    /// The portal user.
    pub user_id: i64,
}

type GuardFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = std::result::Result<(), String>> + Send>>;

/// A guard on the state a suspension came from.
fn suspended_from(grant: &AccessGrant, wanted: &'static str) -> GuardFuture {
    let ok = grant.suspended_from.as_deref() == Some(wanted);
    Box::pin(async move {
        if ok {
            Ok(())
        } else {
            Err(english(
                "wl_portal.grants.refused.suspended_from",
                &[("from", &wanted)],
            ))
        }
    })
}

/// The reason of the run, trimmed; `None` when blank.
fn reason_of(ctx: &TransitionCtx) -> Option<String> {
    ctx.reason()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_owned)
}

/// Sets columns of grant `id` outside the machine's guard and stamps
/// `updated_at`.
async fn set(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    AccessGrant::query()
        .where_(AccessGrant::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

/// A system transition `name` from `from` to `to`.
fn step(
    name: &'static str,
    label: &'static str,
    tone: &'static str,
    from: &[GrantStatus],
    to: GrantStatus,
) -> Transition<AccessGrant> {
    Transition::new(name)
        .label(label)
        .tone(tone)
        .system()
        .from(from.iter().copied())
        .to(to)
}

impl StateMachine for AccessGrant {
    type State = GrantStatus;
    const COLUMN: renox::db::Col<AccessGrant, GrantStatus> = AccessGrant::STATUS;

    fn machine() -> Machine<Self> {
        use GrantStatus::{Active, Closed, Expired, Invited, ReadOnly, Revoked, Suspended};
        Machine::new()
            .initial([Invited, Active])
            .state(Invited)
            .tone("info")
            .state(Active)
            .tone("success")
            .state(Suspended)
            .tone("warning")
            .state(ReadOnly)
            .tone("warning")
            .state(Closed)
            .tone("neutral")
            .terminal()
            .state(Expired)
            .tone("neutral")
            .terminal()
            .state(Revoked)
            .tone("neutral")
            .terminal()
            .transition(
                step(ACCEPT, "Accept", "success", &[Invited], Active).effect(
                    |grant: &AccessGrant, ctx, tx| {
                        let id = grant.id;
                        let user = ctx.input_as::<AcceptInput>().ok().map(|i| i.user_id);
                        Box::pin(async move { set(tx, id, &[("user_id", &user)]).await })
                    },
                ),
            )
            .transition(step(EXPIRE, "Expire", "neutral", &[Invited], Expired))
            .transition(step(REVOKE, "Revoke", "neutral", &[Invited], Revoked))
            .transition(
                step(
                    SUSPEND,
                    "Suspend",
                    "warning",
                    &[Active, ReadOnly],
                    Suspended,
                )
                .reason_required()
                .effect(|grant: &AccessGrant, ctx, tx| {
                    let id = grant.id;
                    let from = Some(grant.status.as_str().to_owned());
                    let reason = reason_of(ctx);
                    Box::pin(async move {
                        set(
                            tx,
                            id,
                            &[("suspended_from", &from), ("suspend_reason", &reason)],
                        )
                        .await
                    })
                }),
            )
            .transition(
                step(REINSTATE, "Reinstate", "success", &[Suspended], Active)
                    .guard("was_active", |g: &AccessGrant, _ctx| {
                        suspended_from(g, "active")
                    }),
            )
            .transition(
                step(
                    REINSTATE_READ_ONLY,
                    "Reinstate",
                    "success",
                    &[Suspended],
                    ReadOnly,
                )
                .guard("was_read_only", |g: &AccessGrant, _ctx| {
                    suspended_from(g, "read_only")
                }),
            )
            .transition(step(END, "End", "warning", &[Active], ReadOnly).effect(
                |grant: &AccessGrant, ctx, tx| {
                    let id = grant.id;
                    let reason = reason_of(ctx);
                    Box::pin(async move { set(tx, id, &[("ended_reason", &reason)]).await })
                },
            ))
            .transition(
                step(
                    CLOSE,
                    "Close",
                    "neutral",
                    &[ReadOnly, Active, Suspended],
                    Closed,
                )
                .effect(|grant: &AccessGrant, ctx, tx| {
                    let id = grant.id;
                    let reason = reason_of(ctx);
                    Box::pin(async move { set(tx, id, &[("ended_reason", &reason)]).await })
                }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    #[test]
    fn the_machine_has_the_transitions_of_the_spec() {
        let machine = AccessGrant::machine();
        assert_eq!(machine.initial_states(), ["invited", "active"]);
        let table: [(&str, &[&str], &str); 8] = [
            (ACCEPT, &["invited"], "active"),
            (EXPIRE, &["invited"], "expired"),
            (REVOKE, &["invited"], "revoked"),
            (SUSPEND, &["active", "read_only"], "suspended"),
            (REINSTATE, &["suspended"], "active"),
            (REINSTATE_READ_ONLY, &["suspended"], "read_only"),
            (END, &["active"], "read_only"),
            (CLOSE, &["read_only", "active", "suspended"], "closed"),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to) in table {
            let t = machine.find(name).expect(name);
            assert!(t.is_system(), "{name}");
            assert_eq!(t.from_states(), from, "{name}");
            assert_eq!(t.to_state(), Some(to), "{name}");
        }
    }

    #[renox::test]
    async fn reinstate_goes_back_to_the_state_before_the_suspension() {
        let machine = AccessGrant::machine();
        let ctx = TransitionCtx::new(None);
        let from_active = AccessGrant {
            suspended_from: Some("active".into()),
            ..AccessGrant::default()
        };
        let from_read_only = AccessGrant {
            suspended_from: Some("read_only".into()),
            ..AccessGrant::default()
        };
        let back = machine.find(REINSTATE).unwrap();
        let back_read_only = machine.find(REINSTATE_READ_ONLY).unwrap();
        assert!(back.check_guards(&from_active, &ctx).await.is_ok());
        assert!(back.check_guards(&from_read_only, &ctx).await.is_err());
        assert!(
            back_read_only
                .check_guards(&from_read_only, &ctx)
                .await
                .is_ok()
        );
        assert!(
            back_read_only
                .check_guards(&from_active, &ctx)
                .await
                .is_err()
        );
    }

    #[test]
    fn a_grant_is_active_inside_its_dates() {
        let grant = AccessGrant {
            status: GrantStatus::Active,
            valid_from: day(11, 1),
            valid_until: Some(day(11, 30)),
            ..AccessGrant::default()
        };
        assert!(!grant.is_active_on(day(10, 31)));
        assert!(grant.is_active_on(day(11, 1)));
        assert!(grant.is_active_on(day(11, 30)));
        assert!(!grant.is_active_on(day(12, 1)));
        let suspended = AccessGrant {
            status: GrantStatus::Suspended,
            ..grant.clone()
        };
        assert!(!suspended.is_active_on(day(11, 5)));
        let open_ended = AccessGrant {
            valid_until: None,
            ..grant
        };
        assert!(open_ended.is_active_on(day(12, 31)));
    }

    #[test]
    fn roles_are_split_on_commas() {
        let grant = AccessGrant {
            roles: "primary_contact, finance_contact,".into(),
            ..AccessGrant::default()
        };
        assert_eq!(grant.role_keys(), ["primary_contact", "finance_contact"]);
    }
}
