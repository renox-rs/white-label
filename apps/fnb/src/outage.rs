//! An outlet's outage and the pause of its channels (06 F&B §4.6 row
//! *Internet or power outage at the outlet*, §7.2 `FNB.RUL.15`; BL-FNB-14).
//!
//! The kitchen displays and tills of an outlet are one **device connection**
//! (`fnb.devices.<outlet code>`, BL-KRN-07), active during the outlet's opening
//! hours and `down` after N minutes (default 5) of silence. [`Fnb`](crate::Fnb)
//! registers one per outlet it is told about ([`crate::Fnb::kitchen_devices`]).
//!
//! **`FNB.RUL.15`** ([`on_status_changed`], on `IntegrationStatusChanged`):
//!
//! - the connection goes `down`, and its silence began inside the opening
//!   hours: an [`Outage`] opens (`paused`), **this outlet's** brand channels on
//!   the D2C site are paused ("temporarily closed", a brand also cooked by
//!   another outlet stays open there), and the holders of
//!   `outlet_channels.manage` (the kitchen manager and the area manager) are
//!   told to pause the aggregators on their tablets, with a task for the kitchen
//!   manager. Silence outside the opening hours is ignored. A second `down`
//!   while the outage is open does nothing new (flapping);
//! - the devices answer again (`down` to `up` or `degraded`): the outage is
//!   `devices_back` and the kitchen manager is asked to resume the channels and
//!   to enter the paper orders as late entries ([`crate::paper_order`]); the
//!   prompt repeats every R minutes (default 15) while the devices are up and
//!   the channels are still paused ([`run_prompts`]);
//! - **the system never resumes**: only a holder of `outlet_channels.manage`
//!   does ([`resume`]), which resumes every channel the outage paused.
//!
//! D2C orders already `placed` at the outlet are left to `FNB.RUL.01`. The
//! aggregators' tablets are paused by hand; the channels page lets the manager
//! mark them paused and resumed so the record is complete. An outage can also
//! be opened by hand ([`pause_by_hand`]), which never waits for the devices.
//!
//! Decisions: a brand's delivery areas are not a record yet, so the pause is per
//! outlet and brand; the "temporarily closed" status is read by the ordering
//! page of the brand ([`ordering_of`]) and by [`accepts_orders`].

use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition};
use serde::{Deserialize, Serialize};
use wl_kernel::integrations::{IntegrationConnection, IntegrationStatusChanged, devices};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::task_creator;
use wl_kernel::Subject;

use crate::catalogue::{Brand, Outlet};
use crate::notices::tell_holders;
use crate::permissions;
use crate::placing::refused;
use crate::settings::settings;
use crate::texts::english;

/// The subject type of an outage.
pub const SUBJECT_TYPE: &str = "fnb.outage";

/// The rule.
pub const RULE: &str = "FNB.RUL.15";

/// The scheduled task that repeats the prompt to resume.
pub const PROMPT_TASK: &str = "fnb:outage-prompts";

/// The prefix of the device connection of an outlet.
pub const CONNECTION_PREFIX: &str = "fnb.devices.";

/// The device connection of the outlet `code`.
pub fn connection_key(code: &str) -> String {
    format!("{CONNECTION_PREFIX}{code}")
}

/// `devices_back`: paused → devices_back, by the system.
pub const DEVICES_BACK: &str = "devices_back";
/// `devices_down`: devices_back → paused, by the system (the devices went
/// silent again).
pub const DEVICES_DOWN: &str = "devices_down";
/// `resume`: paused, devices_back → resumed, by the kitchen or area manager.
pub const RESUME: &str = "resume";

/// How an outage was opened.
pub mod source {
    /// The kitchen devices connection went `down` (`FNB.RUL.15`).
    pub const DEVICES: &str = "devices";
    /// A manager paused the channels by hand.
    pub const MANUAL: &str = "manual";
}

/// The kinds of channel row.
pub mod channel_kind {
    /// A brand on the D2C site.
    pub const D2C: &str = "d2c";
    /// An aggregator's tablet.
    pub const AGGREGATOR: &str = "aggregator";
}

/// Where an outage is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OutageStatus {
    /// The channels are paused; the devices are silent (or it was paused by
    /// hand).
    #[default]
    Paused,
    /// The devices answer again; the channels stay paused until a manager
    /// resumes them.
    DevicesBack,
    /// A manager resumed the channels (terminal).
    Resumed,
}

/// An outlet's outage: the period its channels were paused.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "fnb_outages",
    state_machine,
    index(outlet_id),
    index(status)
)]
pub struct Outage {
    /// The primary key.
    pub id: i64,
    /// The outlet.
    #[model(references = "fnb_outlets")]
    pub outlet_id: i64,
    /// The device connection that went down, for an outage of the devices.
    pub connection_key: Option<String>,
    /// `devices` or `manual`, one of [`source`].
    pub source: String,
    /// Where it is; changed only by a transition.
    pub status: OutageStatus,
    /// When the silence began (the devices' first missed heartbeat).
    pub started_at: Option<DateTime>,
    /// When the channels were paused.
    pub paused_at: Option<DateTime>,
    /// When the devices answered again.
    pub devices_back_at: Option<DateTime>,
    /// When a manager resumed the channels.
    pub resumed_at: Option<DateTime>,
    /// Who resumed them.
    pub resumed_by: Option<i64>,
    /// When the manager was last asked to resume.
    pub last_prompt_at: Option<DateTime>,
    /// How many times the manager was asked.
    pub prompts: i64,
    /// Why it was paused by hand.
    pub reason: Option<String>,
    /// The kitchen manager's task, while the outage is open.
    pub ticket_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Outage {
    /// The outage as a [`Subject`].
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// Whether the channels are still paused by it.
    pub fn is_open(&self) -> bool {
        self.status != OutageStatus::Resumed
    }

    /// The open outage of `outlet_id`, if there is one.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn open_of(db: &Db, outlet_id: i64) -> Result<Option<Outage>> {
        Outage::query()
            .where_(Outage::OUTLET_ID.eq(outlet_id))
            .where_(Outage::STATUS.is_in([OutageStatus::Paused, OutageStatus::DevicesBack]))
            .order_by_desc(Outage::ID)
            .first(db)
            .await
    }

    /// The outage of `outlet_id` that covers the business time `at`: it began
    /// before and was not resumed before. The paper orders of an outage name it.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn covering(db: &Db, outlet_id: i64, at: DateTime) -> Result<Option<Outage>> {
        let all = Outage::query()
            .where_(Outage::OUTLET_ID.eq(outlet_id))
            .order_by_desc(Outage::ID)
            .get(db)
            .await?;
        Ok(all.into_iter().find(|outage| {
            outage.started_at.is_some_and(|started| started <= at)
                && outage.resumed_at.is_none_or(|resumed| at <= resumed)
        }))
    }

    /// The latest outages of every outlet, newest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn latest(db: &Db, limit: u64) -> Result<Vec<Outage>> {
        Outage::query()
            .order_by_desc(Outage::ID)
            .limit(limit)
            .get(db)
            .await
    }
}

/// A channel of an outlet that can be paused: a brand on the D2C site, or an
/// aggregator's tablet.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_outlet_channels", index(outlet_id), index(brand_id))]
pub struct OutletChannel {
    /// The primary key.
    pub id: i64,
    /// The outlet.
    #[model(references = "fnb_outlets")]
    pub outlet_id: i64,
    /// `d2c` or `aggregator`, one of [`channel_kind`].
    pub kind: String,
    /// The brand, for a D2C row.
    pub brand_id: Option<i64>,
    /// The sales channel: `website` for D2C, the aggregator's key otherwise.
    pub channel: String,
    /// Whether the channel is paused ("temporarily closed").
    pub paused: bool,
    /// Since when.
    pub paused_at: Option<DateTime>,
    /// Who paused it by hand (`None`: the rule).
    pub paused_by: Option<i64>,
    /// The outage that paused it, while it is open.
    pub outage_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl OutletChannel {
    /// The channels of `outlet_id`: D2C brands first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_outlet(db: &Db, outlet_id: i64) -> Result<Vec<OutletChannel>> {
        OutletChannel::query()
            .where_(OutletChannel::OUTLET_ID.eq(outlet_id))
            .order_by(OutletChannel::KIND)
            .order_by(OutletChannel::ID)
            .get(db)
            .await
    }

    /// Adds the channel to the outlet when it has it not. A D2C channel names
    /// the brand the outlet cooks.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn ensure(
        db: &Db,
        outlet_id: i64,
        kind: &str,
        brand_id: Option<i64>,
        channel: &str,
    ) -> Result<OutletChannel> {
        let mut query = OutletChannel::query()
            .where_(OutletChannel::OUTLET_ID.eq(outlet_id))
            .where_(OutletChannel::KIND.eq(kind))
            .where_(OutletChannel::CHANNEL.eq(channel));
        if let Some(brand) = brand_id {
            query = query.where_(OutletChannel::BRAND_ID.eq(brand));
        }
        if let Some(found) = query.first(db).await? {
            return Ok(found);
        }
        let mut row = OutletChannel {
            outlet_id,
            kind: kind.to_owned(),
            brand_id,
            channel: channel.to_owned(),
            ..OutletChannel::default()
        };
        row.insert(db).await?;
        Ok(row)
    }
}

/// Sets columns of the outage `id` inside the transition's transaction.
async fn set(
    tx: &mut Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let at = now();
    all.push(("updated_at", &at));
    Outage::query()
        .where_(Outage::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

fn devices_back_transition() -> Transition<Outage> {
    Transition::new(DEVICES_BACK)
        .label("Devices answer")
        .system()
        .from([OutageStatus::Paused])
        .to(OutageStatus::DevicesBack)
        .effect(|o: &Outage, _ctx, tx| {
            let id = o.id;
            Box::pin(async move {
                let at = Some(now());
                set(tx, id, &[("devices_back_at", &at)]).await
            })
        })
}

fn devices_down_transition() -> Transition<Outage> {
    Transition::new(DEVICES_DOWN)
        .label("Devices silent again")
        .system()
        .from([OutageStatus::DevicesBack])
        .to(OutageStatus::Paused)
        .effect(|o: &Outage, _ctx, tx| {
            let id = o.id;
            Box::pin(async move {
                let none: Option<DateTime> = None;
                set(
                    tx,
                    id,
                    &[("devices_back_at", &none), ("last_prompt_at", &none), ("prompts", &0_i64)],
                )
                .await
            })
        })
}

fn resume_transition() -> Transition<Outage> {
    Transition::new(RESUME)
        .label("Resume the channels")
        .tone("success")
        .permission(permissions::OUTLET_CHANNELS_MANAGE)
        .from([OutageStatus::Paused, OutageStatus::DevicesBack])
        .to(OutageStatus::Resumed)
        .effect(|o: &Outage, ctx, tx| {
            let id = o.id;
            let by = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                let at = Some(now());
                set(tx, id, &[("resumed_at", &at), ("resumed_by", &by)]).await?;
                // Every channel the outage paused is open again.
                let open = false;
                let none: Option<DateTime> = None;
                let nobody: Option<i64> = None;
                OutletChannel::query()
                    .where_(OutletChannel::OUTAGE_ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("paused", &open),
                            ("paused_at", &none),
                            ("paused_by", &nobody),
                            ("outage_id", &nobody),
                            ("updated_at", &now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

impl StateMachine for Outage {
    type State = OutageStatus;
    const COLUMN: renox::db::Col<Outage, OutageStatus> = Outage::STATUS;

    fn machine() -> Machine<Self> {
        use OutageStatus::{DevicesBack, Paused, Resumed};
        Machine::new()
            .initial([Paused])
            .state(Paused)
            .tone("warning")
            .state(DevicesBack)
            .tone("info")
            .state(Resumed)
            .tone("success")
            .terminal()
            .transition(devices_back_transition())
            .transition(devices_down_transition())
            .transition(resume_transition())
    }
}

/// The key the kitchen manager's task of `outage` is found by.
pub fn task_key(outage: &Outage) -> String {
    format!("fnb.outage:{}", outage.id)
}

/// Pauses the D2C channels of `outlet` in the transaction that opens `outage`
/// and links them to it.
async fn pause_d2c(tx: &mut Transaction, outlet_id: i64, outage_id: i64, by: Option<i64>) -> Result {
    let paused = true;
    let at = Some(now());
    let outage = Some(outage_id);
    OutletChannel::query()
        .where_(OutletChannel::OUTLET_ID.eq(outlet_id))
        .where_(OutletChannel::KIND.eq(channel_kind::D2C))
        .where_(OutletChannel::PAUSED.eq(false))
        .update(
            &mut *tx,
            &[
                ("paused", &paused),
                ("paused_at", &at),
                ("paused_by", &by),
                ("outage_id", &outage),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// The brands paused at the outlet by `outage`, by name.
async fn paused_brands(db: &Db, outage: &Outage) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for channel in OutletChannel::query()
        .where_(OutletChannel::OUTAGE_ID.eq(outage.id))
        .where_(OutletChannel::KIND.eq(channel_kind::D2C))
        .get(db)
        .await?
    {
        if let Some(brand) = channel.brand_id
            && let Some(brand) = Brand::find(db, brand).await?
        {
            names.push(brand.name);
        }
    }
    Ok(names)
}

/// Opens an outage of `outlet`: the D2C channels are paused in the same
/// transaction; the managers are told after the commit. Returns `None` when an
/// outage is open already (a second `down` while paused does nothing new).
async fn open_outage(
    state: &AppState,
    outlet: &Outlet,
    from: &str,
    connection: Option<&str>,
    started_at: Option<DateTime>,
    reason: Option<String>,
    by: Option<i64>,
) -> Result<Option<Outage>> {
    if let Some(open) = Outage::open_of(&state.db, outlet.id).await? {
        // Flapping: devices that answered and went silent again.
        if open.status == OutageStatus::DevicesBack && from == source::DEVICES {
            let _outcome = open
                .transition(DEVICES_DOWN)
                .by_system()
                .reason(english("fnb.outage.history.silent_again", &[]))
                .run(state)
                .await
                .map_err(|e| e.into_error())?;
        }
        return Ok(None);
    }
    let paused_at = now();
    let mut outage = Outage {
        outlet_id: outlet.id,
        connection_key: connection.map(str::to_owned),
        source: from.to_owned(),
        started_at: Some(started_at.unwrap_or(paused_at)),
        paused_at: Some(paused_at),
        reason,
        ..Outage::default()
    };
    let history = if from == source::DEVICES {
        english("fnb.outage.history.devices_down", &[])
    } else {
        english("fnb.outage.history.paused_by_hand", &[])
    };
    let mut tx = state.db.begin().await?;
    outage
        .insert_in_state(&mut tx, OutageStatus::Paused, &history)
        .await?;
    pause_d2c(&mut tx, outlet.id, outage.id, by).await?;
    tx.commit().await?;
    announce_pause(state, outlet, &outage).await?;
    Ok(Some(Outage::find_or_404(&state.db, outage.id).await?))
}

/// Tells the managers the channels are paused and opens the kitchen manager's
/// task.
async fn announce_pause(state: &AppState, outlet: &Outlet, outage: &Outage) -> Result {
    let brands = paused_brands(&state.db, outage).await?.join(", ");
    let link = "/outlet-channels".to_owned();
    let title = english("fnb.outage.paused_title", &[("outlet", &outlet.name)]);
    let body = english(
        "fnb.outage.paused_body",
        &[("outlet", &outlet.name), ("brands", &brands)],
    );
    tell_holders(
        state,
        outage.subject(),
        &outlet.name,
        permissions::OUTLET_CHANNELS_MANAGE,
        Some(outlet.scope()),
        &title,
        &body,
        &link,
    )
    .await?;
    let ticket = role_tasks::open(
        state,
        RoleTask::new(
            outage.subject(),
            permissions::KITCHEN_MANAGER_ROLE,
            &english("fnb.outage.task_title", &[("outlet", &outlet.name)]),
            &task_key(outage),
        )
        .description(&english(
            "fnb.outage.task_body",
            &[("outlet", &outlet.name), ("brands", &brands), ("link", &link)],
        )),
    )
    .await?;
    if let Some(ticket) = ticket {
        Outage::query()
            .where_(Outage::ID.eq(outage.id))
            .update(&state.db, &[("ticket_id", &Some(ticket.id.0))])
            .await?;
    }
    Ok(())
}

/// Asks the kitchen manager to resume the channels of `outage` and to enter
/// the paper orders; records the prompt.
async fn prompt(state: &AppState, outage: &Outage, at: DateTime) -> Result {
    let outlet = Outlet::find_or_404(&state.db, outage.outlet_id).await?;
    tell_holders(
        state,
        outage.subject(),
        &outlet.name,
        permissions::OUTLET_CHANNELS_MANAGE,
        Some(outlet.scope()),
        &english("fnb.outage.back_title", &[("outlet", &outlet.name)]),
        &english("fnb.outage.back_body", &[("outlet", &outlet.name)]),
        "/outlet-channels",
    )
    .await?;
    Outage::query()
        .where_(Outage::ID.eq(outage.id))
        .update(
            &state.db,
            &[
                ("last_prompt_at", &Some(at)),
                ("prompts", &(outage.prompts + 1)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// The devices of `outlet` answer again: the open outage of the devices is
/// `devices_back` and the kitchen manager is asked to resume the channels.
async fn devices_answer(state: &AppState, outlet: &Outlet) -> Result {
    let Some(open) = Outage::open_of(&state.db, outlet.id).await? else {
        return Ok(());
    };
    if open.status != OutageStatus::Paused || open.source != source::DEVICES {
        return Ok(());
    }
    let _outcome = open
        .transition(DEVICES_BACK)
        .by_system()
        .reason(english("fnb.outage.history.devices_back", &[]))
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    let open = Outage::find_or_404(&state.db, open.id).await?;
    prompt(state, &open, now()).await
}

/// `FNB.RUL.15` on `IntegrationStatusChanged` of an outlet's kitchen devices.
///
/// # Errors
///
/// Database errors and the notifier's.
pub(crate) async fn on_status_changed(state: &AppState, event: IntegrationStatusChanged) -> Result {
    let Some(code) = event.connection.strip_prefix(CONNECTION_PREFIX) else {
        return Ok(());
    };
    if !settings(state).outage_rule {
        return Ok(());
    }
    let Some(outlet) = Outlet::by_code(&state.db, code).await? else {
        return Ok(());
    };
    if event.to == "down" {
        // Silence outside the opening hours is ignored: the connection counts
        // it from the window start, but a run that crossed the closing time
        // started before it.
        let connection = IntegrationConnection::by_key(&state.db, &event.connection).await?;
        let began = event.failed_since.unwrap_or_else(now);
        let inside = connection
            .as_ref()
            .is_none_or(|c| devices::in_window(devices::window_of(c), began));
        if !inside {
            return Ok(());
        }
        open_outage(
            state,
            &outlet,
            source::DEVICES,
            Some(&event.connection),
            event.failed_since,
            None,
            None,
        )
        .await?;
    } else if event.from == "down" && (event.to == "up" || event.to == "degraded") {
        devices_answer(state, &outlet).await?;
    }
    Ok(())
}

/// Opens the outage of the kitchen devices of `outlet` as the system, as
/// `FNB.RUL.15` does when the connection goes `down`: the demo company and the
/// tests that do not drive the heartbeats use it. `None` when an outage is open
/// already.
///
/// # Errors
///
/// Database errors and the notifier's.
pub async fn open_by_devices(
    state: &AppState,
    outlet: &Outlet,
    started_at: DateTime,
) -> Result<Option<Outage>> {
    open_outage(
        state,
        outlet,
        source::DEVICES,
        Some(&connection_key(&outlet.code)),
        Some(started_at),
        None,
        None,
    )
    .await
}

/// A manager pauses every D2C channel of the outlet by hand (a known outage, a
/// kitchen that cannot cope), with a reason. The system never resumes it.
///
/// # Errors
///
/// 403 without `outlet_channels.manage`; 422 without a reason or when the
/// outlet's channels are paused already.
pub async fn pause_by_hand(
    state: &AppState,
    user: &AuthUser,
    outlet_id: i64,
    reason: &str,
) -> Result<Outage> {
    if !user.has_permission(permissions::OUTLET_CHANNELS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(refused(english("fnb.outage.problem.reason", &[])));
    }
    let outlet = Outlet::find_or_404(&state.db, outlet_id).await?;
    match open_outage(
        state,
        &outlet,
        source::MANUAL,
        None,
        None,
        Some(reason.to_owned()),
        Some(user.id),
    )
    .await?
    {
        Some(outage) => Ok(outage),
        None => Err(refused(english("fnb.outage.problem.already_paused", &[]))),
    }
}

/// A manager resumes the channels of the outage `id`: every channel it paused
/// is open again, and the task closes.
///
/// # Errors
///
/// 403 without `outlet_channels.manage`; 422 for an outage already resumed.
pub async fn resume(state: &AppState, user: &AuthUser, id: i64) -> Result<Outage> {
    if !user.has_permission(permissions::OUTLET_CHANNELS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let outage = Outage::find_or_404(&state.db, id).await?;
    let outcome = outage
        .transition(RESUME)
        .by(user)
        .reason(english("fnb.outage.history.resumed", &[]))
        .run(state)
        .await;
    let _ = outcome.map_err(|e| e.into_error())?;
    let tickets = task_creator(state);
    if let Some(open) = tickets.find_open(state, &task_key(&outage)).await? {
        tickets
            .complete(state, open.id, &english("fnb.outage.task_done", &[]))
            .await?;
    }
    Outage::find_or_404(&state.db, id).await
}

/// A manager pauses or resumes one channel of an outlet by hand (an aggregator's
/// tablet they paused, a brand they open again before the outage is resumed).
///
/// # Errors
///
/// 403 without `outlet_channels.manage`.
pub async fn set_channel(
    state: &AppState,
    user: &AuthUser,
    channel_id: i64,
    paused: bool,
) -> Result<OutletChannel> {
    if !user.has_permission(permissions::OUTLET_CHANNELS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let channel = OutletChannel::find_or_404(&state.db, channel_id).await?;
    let outage = if paused {
        Outage::open_of(&state.db, channel.outlet_id)
            .await?
            .map(|o| o.id)
    } else {
        None
    };
    let at = paused.then(now);
    let by = paused.then_some(user.id);
    OutletChannel::query()
        .where_(OutletChannel::ID.eq(channel.id))
        .update(
            &state.db,
            &[
                ("paused", &paused),
                ("paused_at", &at),
                ("paused_by", &by),
                ("outage_id", &outage),
                ("updated_at", &now()),
            ],
        )
        .await?;
    OutletChannel::find_or_404(&state.db, channel_id).await
}

/// Repeats the prompt to resume for every outage whose devices answer and whose
/// channels are still paused, when R minutes passed since the last one. Called
/// every minute by the scheduler; `at` is the time of the run.
///
/// # Errors
///
/// Database errors and the notifier's.
pub async fn run_prompts(state: &AppState, at: DateTime) -> Result<usize> {
    let wait = renox::chrono::TimeDelta::minutes(settings(state).outage_prompt_minutes);
    let mut sent = 0;
    for outage in Outage::query()
        .where_(Outage::STATUS.eq(OutageStatus::DevicesBack))
        .get(&state.db)
        .await?
    {
        let due = outage
            .last_prompt_at
            .is_none_or(|last| at - last >= wait);
        if due {
            prompt(state, &outage, at).await?;
            sent += 1;
        }
    }
    Ok(sent)
}

/// Whether the outlet takes orders for `brand_id` on the D2C site: its D2C
/// channel for the brand is not paused. A brand the outlet does not cook is
/// not orderable there.
///
/// # Errors
///
/// Database errors.
pub async fn accepts_orders(db: &Db, outlet_id: i64, brand_id: i64) -> Result<bool> {
    Ok(OutletChannel::query()
        .where_(OutletChannel::OUTLET_ID.eq(outlet_id))
        .where_(OutletChannel::KIND.eq(channel_kind::D2C))
        .where_(OutletChannel::BRAND_ID.eq(brand_id))
        .first(db)
        .await?
        .is_some_and(|channel| !channel.paused))
}

/// One outlet as the ordering page of a brand shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutletOrdering {
    /// The outlet.
    pub outlet_id: i64,
    /// Its name.
    pub name: String,
    /// Whether it takes orders for the brand.
    pub open: bool,
}

/// The outlets that cook `brand` with whether each takes orders now.
///
/// # Errors
///
/// Database errors.
pub async fn ordering_of(db: &Db, brand: &Brand) -> Result<Vec<OutletOrdering>> {
    let mut found = Vec::new();
    for channel in OutletChannel::query()
        .where_(OutletChannel::BRAND_ID.eq(brand.id))
        .where_(OutletChannel::KIND.eq(channel_kind::D2C))
        .order_by(OutletChannel::OUTLET_ID)
        .get(db)
        .await?
    {
        if let Some(outlet) = Outlet::find(db, channel.outlet_id).await?
            && outlet.active
        {
            found.push(OutletOrdering {
                outlet_id: outlet.id,
                name: outlet.name,
                open: !channel.paused,
            });
        }
    }
    Ok(found)
}

/// Whether the brand's checkout takes an order: for the kitchen `outlet_code`
/// when one is named, else when any kitchen that cooks the brand is open (a
/// brand no outlet manages takes orders). A paused kitchen takes none.
///
/// # Errors
///
/// Database errors.
pub async fn takes_orders(db: &Db, brand: &Brand, outlet_code: Option<&str>) -> Result<bool> {
    match outlet_code {
        Some(code) => match Outlet::by_code(db, code).await? {
            Some(outlet) => accepts_orders(db, outlet.id, brand.id).await,
            None => Ok(false),
        },
        None => {
            let kitchens = ordering_of(db, brand).await?;
            Ok(kitchens.is_empty() || kitchens.iter().any(|k| k.open))
        }
    }
}

/// Registers the device connection of one outlet's kitchen devices (BL-KRN-07):
/// active in the outlet's opening hours, grouped (`down` only when no device
/// answers), `down` after N minutes of silence, with the manual fallback.
pub(crate) fn register_connection(kitchen: &crate::settings::KitchenDevices) {
    wl_kernel::integrations::IntegrationDef::device(&connection_key(&kitchen.outlet), "fnb")
        .scope(&format!("outlet:{}", kitchen.outlet))
        .owner_role(permissions::KITCHEN_MANAGER_ROLE)
        .screens(["fnb.outlet_channels"])
        .fallback(
            &english("fnb.outage.fallback", &[]),
            None,
        )
        .active_window(&kitchen.from, &kitchen.to)
        .group(kitchen.devices.iter().cloned())
        .down_after_minutes(kitchen.down_after_minutes)
        .register();
}

/// Registers the listener of the devices connection and the scheduled prompt.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: IntegrationStatusChanged, state: AppState| async move {
        on_status_changed(&state, event).await
    });
    app.schedule()
        .every_minutes(1, PROMPT_TASK, |state: AppState| async move {
            run_prompts(&state, now()).await.map(|_| ())
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_pauses_waits_for_the_manager_and_never_resumes_by_itself() {
        let machine = Outage::machine();
        assert_eq!(machine.states().len(), 3);
        assert!(machine.find(DEVICES_BACK).unwrap().is_system());
        assert!(machine.find(DEVICES_DOWN).unwrap().is_system());
        let resume = machine.find(RESUME).unwrap();
        assert!(!resume.is_system(), "only a manager resumes");
        assert_eq!(resume.to_state(), Some("resumed"));
    }

    #[test]
    fn the_connection_of_an_outlet_is_named_by_its_code() {
        assert_eq!(connection_key("marina"), "fnb.devices.marina");
    }
}
