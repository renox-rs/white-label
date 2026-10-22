//! Integration health and the manual fallback (wl-kernel §4.5, §5.14,
//! `PLT.RUL.05`).
//!
//! Each external system a module calls is registered as a connection
//! ([`IntegrationDef`]) with the screens it affects, a manual fallback and
//! the role responsible for it. The contract: the registry, the call a
//! module hands over ([`Outbound`]), what its handler answers ([`Delivery`]),
//! the retry wait ([`retry_wait`]: 1, 2, 4 and 8 minutes, then 15 minutes,
//! with no attempt limit) and the event ([`IntegrationStatusChanged`]). The
//! records: the [`IntegrationConnection`] with its machine (`up`,
//! `degraded`, `down`, `disabled`) and the [`QueuedCall`]s waiting for it.

pub mod alerts;
pub mod connection;
pub mod def;
pub mod devices;
pub mod events;
pub mod queue;
pub mod reconcile;
pub mod service;
pub mod web;

use renox::prelude::*;

pub use connection::{ConnectionStatus, IntegrationConnection, transitions};
pub use def::{
    DEFAULT_DOWN_AFTER_MINUTES, Delivery, DeviceSpec, DueFn, EntryFn, Inbound, IntegrationDef,
    Match, MatchFn, Outbound, SendFn, all, call_failed, down_at, get, register, retry_wait,
};
pub use devices::{DeviceBeat, DeviceSettings};
pub use events::IntegrationStatusChanged;
pub use queue::{CallStatus, QueuedCall};
pub use reconcile::{Line, LineKind, Reconciliation};
pub use service::{
    Received, Report, Submitted, disable, drain, enable, ensure, go_down, receive, report_failure,
    report_failure_since, report_success, run_due, run_queue, submit,
};

/// The subject type of a connection, for rule runs and alerts.
pub const SUBJECT_TYPE: &str = "kernel.integration_connection";

/// The key an app registers its e-mail connection under: `wl-notify` reports
/// each e-mail send to it.
pub const EMAIL: &str = "email";

/// The key an app registers its SMS connection under: `wl-notify` reports
/// each SMS send to it.
pub const SMS: &str = "sms";

/// The key an app registers its WhatsApp connection under: `wl-notify`
/// reports each WhatsApp send to it.
pub const WHATSAPP: &str = "whatsapp";

/// The key an app registers its payment gateway under; the gateway adapter
/// of `wl-payments` reports each call to it.
pub const PAYMENT_GATEWAY: &str = "payments.gateway";

/// The key an app registers its sanctions list download under: `wl-parties`
/// reports each download to it.
pub const SANCTIONS_DOWNLOAD: &str = "sanctions.download";

/// Reports how a call of connection `key` went to its health: `Ok` is a
/// success, `Err` a failure with that text. An app that did not register
/// the connection keeps no health for it, so nothing is reported then.
///
/// # Errors
///
/// A database error.
pub async fn report_outcome(
    state: &AppState,
    key: &str,
    outcome: std::result::Result<(), &str>,
) -> Result {
    if get(key).is_none() {
        return Ok(());
    }
    match outcome {
        Ok(()) => report_success(state, key).await.map(|_recovered| ()),
        Err(error) => report_failure(state, key, error).await,
    }
}

/// Registers the models and the connection's machine.
pub(crate) fn register_models(app: &mut Registry) {
    app.model::<IntegrationConnection>()
        .model::<DeviceBeat>()
        .model::<QueuedCall>()
        .model::<Reconciliation>();
    renox::state_machine::register::<IntegrationConnection>();
    crate::notify::register_type(alerts::notification_type());
    service::schedule(app);
}

/// Stores a connection row for each registered definition the database
/// lacks; a row that exists (maybe changed in the admin) is kept as it is.
/// Returns how many were created.
pub async fn install(db: &Db) -> Result<usize> {
    let mut created = 0;
    for def in all() {
        if IntegrationConnection::by_key(db, def.key())
            .await?
            .is_some()
        {
            continue;
        }
        create_row(db, &def).await?;
        created += 1;
    }
    Ok(created)
}

/// Stores the row of `def`.
pub(crate) async fn create_row(db: &Db, def: &IntegrationDef) -> Result<IntegrationConnection> {
    let device = def.device_spec();
    let created = IntegrationConnection::create(
        db,
        IntegrationConnection {
            kind: if device.is_some() {
                connection::KIND_DEVICE
            } else {
                connection::KIND_SERVICE
            }
            .to_owned(),
            scope: device.and_then(|d| d.scope.clone()),
            heartbeat_seconds: device.map_or(connection::DEFAULT_HEARTBEAT_SECONDS, |d| {
                d.heartbeat_seconds
            }),
            window_start: device.and_then(|d| d.window).map(|w| w.0),
            window_end: device.and_then(|d| d.window).map(|w| w.1),
            grouped: device.is_some_and(|d| d.grouped),
            key: def.key().to_owned(),
            module: def.module().to_owned(),
            owner_role: def.owner_role_name().to_owned(),
            screens: renox::db::Json(def.screen_keys().to_vec()),
            fallback_en: def.fallback_en().to_owned(),
            fallback_ar: def.fallback_ar().map(str::to_owned),
            down_after_minutes: def.down_after(),
            status: ConnectionStatus::Up,
            ..Default::default()
        },
    )
    .await?;
    if let Some(device) = def.device_spec() {
        // The devices expected to send heartbeats; a connection of one
        // device is that device.
        let names = if device.devices.is_empty() {
            vec![def.key().to_owned()]
        } else {
            device.devices.clone()
        };
        for name in names {
            devices::ensure_device(db, def.key(), &name).await?;
        }
    }
    Ok(created)
}
