//! The integration connection and its machine (wl-kernel §4.5).
//!
//! ```text
//! [*] --> up
//! up --degrade--> degraded                    (system: a call failed)
//! degraded --go_down--> down                  (system, PLT.RUL.05: no success for N minutes)
//! degraded | down --recover--> up             (system: a call succeeded)
//! up | degraded | down --disable--> disabled  (integrations.manage, reason given)
//! disabled --enable--> up                     (integrations.manage)
//! ```
//!
//! `failed_since` is the time of the first failed call of the current
//! failure run: it is set by `degrade`, kept by `go_down` and cleared by
//! `recover` and `enable`. N counts from it, because `go_down` only starts
//! from `degraded`.
//!
//! A connection is a `service` the platform calls or a `device` at a site
//! that calls the platform (BL-KRN-07, [`super::devices`]): for a device a
//! missed heartbeat is the failed call, and `degrade` takes the time of the
//! last heartbeat (or of the window start) as `failed_since`, so N is minutes
//! of silence.

use renox::db::{Json, Transaction, now};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use crate::permissions::INTEGRATIONS_MANAGE;
use crate::texts::en;

use super::def::down_at;

/// The transition names of a connection.
pub mod transitions {
    /// A call failed.
    pub const DEGRADE: &str = "degrade";
    /// No success for N minutes.
    pub const GO_DOWN: &str = "go_down";
    /// A call succeeded.
    pub const RECOVER: &str = "recover";
    /// Switched off by an admin.
    pub const DISABLE: &str = "disable";
    /// Switched on again by an admin.
    pub const ENABLE: &str = "enable";
}

/// The health of a connection.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ConnectionStatus {
    /// Calls succeed.
    #[default]
    Up,
    /// Some calls failed; outbound work is queued and retried.
    Degraded,
    /// Calls have failed for longer than the limit; banner shown, manual
    /// fallback offered.
    Down,
    /// Switched off by an admin; modules use their manual path.
    Disabled,
}

impl ConnectionStatus {
    /// The stored name: `degraded`.
    pub fn as_text(self) -> &'static str {
        match self {
            ConnectionStatus::Up => "up",
            ConnectionStatus::Degraded => "degraded",
            ConnectionStatus::Down => "down",
            ConnectionStatus::Disabled => "disabled",
        }
    }
}

/// The `kind` of a connection the platform calls.
pub const KIND_SERVICE: &str = "service";
/// The `kind` of a connection whose health signal is a device's heartbeat.
pub const KIND_DEVICE: &str = "device";
/// The heartbeat interval of a device connection when none is set, in
/// seconds (§7 *Device connections*: 1 minute).
pub const DEFAULT_HEARTBEAT_SECONDS: i64 = 60;

/// One external system a module talks to (`wl_integration_connections`).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_integration_connections", state_machine, unique(key))]
pub struct IntegrationConnection {
    /// The primary key.
    pub id: i64,
    /// The key the module registered, e.g. `asp.acme`.
    pub key: String,
    /// The owning module.
    pub module: String,
    /// The role responsible for the connection.
    pub owner_role: String,
    /// The screens the connection affects.
    pub screens: Json<Vec<String>>,
    /// The manual fallback, English.
    pub fallback_en: String,
    /// The manual fallback, Arabic; English is shown when empty.
    pub fallback_ar: Option<String>,
    /// N: minutes without a success before `go_down` (`PLT.RUL.05`).
    pub down_after_minutes: i64,
    /// The state; changed only by a transition.
    pub status: ConnectionStatus,
    /// When a call last succeeded.
    pub last_success_at: Option<DateTime>,
    /// The error of the last failed call.
    pub last_error: Option<String>,
    /// The first failed call of the current failure run.
    pub failed_since: Option<DateTime>,
    /// The first failed call of the failure run that began last; kept after
    /// the connection recovers, so the reconciliation of that run finds it.
    pub outage_since: Option<DateTime>,
    /// Why an admin disabled it.
    pub disabled_reason: Option<String>,
    /// When the status last changed.
    pub status_changed_at: Option<DateTime>,
    /// `service` (the platform calls it) or `device` (it calls the platform).
    pub kind: String,
    /// The outlet, branch or site a device connection belongs to.
    pub scope: Option<String>,
    /// The seconds between the heartbeats of a device.
    pub heartbeat_seconds: i64,
    /// When the active window opens, in minutes after midnight in
    /// Asia/Dubai; `None` with `window_end`: always active.
    pub window_start: Option<i64>,
    /// When the active window closes, in minutes after midnight.
    pub window_end: Option<i64>,
    /// Whether the connection groups the devices of its scope: it is `down`
    /// only when none of them answers.
    pub grouped: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl IntegrationConnection {
    /// The connection `key`, if stored.
    pub async fn by_key<'c>(
        db: impl renox::db::Executor<'c>,
        key: &str,
    ) -> Result<Option<IntegrationConnection>> {
        IntegrationConnection::query()
            .where_(IntegrationConnection::KEY.eq(key))
            .first(db)
            .await
    }

    /// Whether `screen` is one the connection registered.
    pub fn affects(&self, screen: &str) -> bool {
        self.screens.iter().any(|s| s == screen)
    }

    /// The fallback text for `locale`: the Arabic one for `ar` when it is
    /// not empty, else the English one.
    pub fn fallback_for(&self, locale: &str) -> &str {
        match self.fallback_ar.as_deref() {
            Some(ar) if locale == "ar" && !ar.trim().is_empty() => ar,
            _ => &self.fallback_en,
        }
    }

    /// Whether the connection is a device's, health by its heartbeat.
    pub fn is_device(&self) -> bool {
        self.kind == KIND_DEVICE
    }

    /// The seconds between heartbeats (60 when not set).
    pub fn interval_seconds(&self) -> i64 {
        if self.heartbeat_seconds > 0 {
            self.heartbeat_seconds
        } else {
            DEFAULT_HEARTBEAT_SECONDS
        }
    }

    /// When the connection goes `down` if nothing succeeds, once it is
    /// `degraded`.
    pub fn goes_down_at(&self) -> Option<DateTime> {
        self.failed_since
            .map(|since| down_at(since, self.down_after_minutes))
    }
}

/// Sets columns of connection `id` inside a transition's transaction.
pub(super) async fn set(
    tx: &mut Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let at = now();
    all.push(("updated_at", &at));
    IntegrationConnection::query()
        .where_(IntegrationConnection::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

/// The time the failures began, when the `degrade` run carries it (a device:
/// the last heartbeat or the window start).
fn since_of(ctx: &TransitionCtx) -> Option<DateTime> {
    ctx.input()
        .get("failed_since")
        .and_then(Value::as_str)
        .and_then(|text| text.parse::<DateTime>().ok())
}

/// The error a `degrade` run carries in its input.
fn error_of(ctx: &TransitionCtx) -> Option<String> {
    ctx.input()
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

impl StateMachine for IntegrationConnection {
    type State = ConnectionStatus;
    const COLUMN: renox::db::Col<IntegrationConnection, ConnectionStatus> =
        IntegrationConnection::STATUS;

    fn machine() -> Machine<Self> {
        use ConnectionStatus::{Degraded, Disabled, Down, Up};
        Machine::new()
            .initial([Up])
            .state(Up)
            .tone("success")
            .state(Degraded)
            .tone("warning")
            .state(Down)
            .tone("error")
            .state(Disabled)
            .tone("neutral")
            .transition(
                Transition::new(transitions::DEGRADE)
                    .label("Degrade")
                    .from([Up])
                    .to(Degraded)
                    .system()
                    .effect(|c: &IntegrationConnection, ctx, tx| {
                        let id = c.id;
                        let error = error_of(ctx);
                        let since = since_of(ctx);
                        Box::pin(async move {
                            // A device's silence began at its last heartbeat
                            // (or the window start), not when it was noticed.
                            let changed = now();
                            let at = since.unwrap_or(changed);
                            set(
                                tx,
                                id,
                                &[
                                    ("failed_since", &Some(at)),
                                    ("outage_since", &Some(at)),
                                    ("last_error", &error),
                                    ("status_changed_at", &Some(changed)),
                                ],
                            )
                            .await
                        })
                    }),
            )
            .transition(
                Transition::new(transitions::GO_DOWN)
                    .label("Go down")
                    .from([Degraded])
                    .to(Down)
                    .system()
                    .guard("limit_passed", |c: &IntegrationConnection, _ctx| {
                        let goes_down_at = c.goes_down_at();
                        Box::pin(async move {
                            match goes_down_at {
                                Some(at) if at <= now() => Ok(()),
                                _ => Err(en("wl_kernel.integrations.not_yet_down")),
                            }
                        })
                    })
                    .effect(|c: &IntegrationConnection, _ctx, tx| {
                        let id = c.id;
                        Box::pin(async move {
                            set(tx, id, &[("status_changed_at", &Some(now()))]).await
                        })
                    }),
            )
            .transition(
                Transition::new(transitions::RECOVER)
                    .label("Recover")
                    .from([Degraded, Down])
                    .to(Up)
                    .system()
                    .effect(|c: &IntegrationConnection, _ctx, tx| {
                        let id = c.id;
                        Box::pin(async move {
                            let at = now();
                            set(
                                tx,
                                id,
                                &[
                                    ("last_success_at", &Some(at)),
                                    ("failed_since", &None::<DateTime>),
                                    ("last_error", &None::<String>),
                                    ("status_changed_at", &Some(at)),
                                ],
                            )
                            .await
                        })
                    }),
            )
            .transition(
                Transition::new(transitions::DISABLE)
                    .label("Disable")
                    .tone("danger")
                    .from([Up, Degraded, Down])
                    .to(Disabled)
                    .permission(INTEGRATIONS_MANAGE)
                    .reason_required()
                    .effect(|c: &IntegrationConnection, ctx, tx| {
                        let id = c.id;
                        let reason = ctx.reason().map(|r| r.trim().to_owned());
                        Box::pin(async move {
                            set(
                                tx,
                                id,
                                &[
                                    ("disabled_reason", &reason),
                                    ("status_changed_at", &Some(now())),
                                ],
                            )
                            .await
                        })
                    }),
            )
            .transition(
                Transition::new(transitions::ENABLE)
                    .label("Enable")
                    .from([Disabled])
                    .to(Up)
                    .permission(INTEGRATIONS_MANAGE)
                    .effect(|c: &IntegrationConnection, _ctx, tx| {
                        let id = c.id;
                        Box::pin(async move {
                            set(
                                tx,
                                id,
                                &[
                                    ("disabled_reason", &None::<String>),
                                    ("failed_since", &None::<DateTime>),
                                    ("last_error", &None::<String>),
                                    ("status_changed_at", &Some(now())),
                                ],
                            )
                            .await
                        })
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_every_transition_of_the_spec() {
        let machine = IntegrationConnection::machine();
        for (name, from, to, system) in [
            ("degrade", &["up"][..], "degraded", true),
            ("go_down", &["degraded"], "down", true),
            ("recover", &["degraded", "down"], "up", true),
            ("disable", &["up", "degraded", "down"], "disabled", false),
            ("enable", &["disabled"], "up", false),
        ] {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.is_system(), system, "{name}");
        }
        assert_eq!(machine.transitions().len(), 5);
    }

    #[test]
    fn go_down_is_guarded_by_the_limit() {
        let machine = IntegrationConnection::machine();
        let go_down = machine.find("go_down").expect("go_down");
        assert_eq!(go_down.guard_names(), ["limit_passed"]);
    }

    #[test]
    fn a_connection_goes_down_n_minutes_after_its_first_failure() {
        let connection = IntegrationConnection {
            failed_since: Some("2026-11-03T06:00:00Z".parse().unwrap()),
            down_after_minutes: 15,
            ..Default::default()
        };
        assert_eq!(
            connection.goes_down_at(),
            Some("2026-11-03T06:15:00Z".parse().unwrap())
        );
        assert_eq!(IntegrationConnection::default().goes_down_at(), None);
    }

    #[test]
    fn the_banner_text_follows_the_language_and_falls_back_to_english() {
        let connection = IntegrationConnection {
            fallback_en: "File by hand.".into(),
            fallback_ar: Some("قدّم يدويًا.".into()),
            ..Default::default()
        };
        assert_eq!(connection.fallback_for("en"), "File by hand.");
        assert_eq!(connection.fallback_for("ar"), "قدّم يدويًا.");
        let english_only = IntegrationConnection {
            fallback_ar: Some("  ".into()),
            ..connection.clone()
        };
        assert_eq!(english_only.fallback_for("ar"), "File by hand.");
        let none = IntegrationConnection {
            fallback_ar: None,
            ..connection
        };
        assert_eq!(none.fallback_for("ar"), "File by hand.");
    }

    #[test]
    fn a_connection_knows_its_screens() {
        let connection = IntegrationConnection {
            screens: Json(vec!["invoicing.invoice_list".into()]),
            ..Default::default()
        };
        assert!(connection.affects("invoicing.invoice_list"));
        assert!(!connection.affects("invoicing.payment_list"));
    }
}
