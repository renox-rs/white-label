//! The cold units and their temperature holds (06 F&B §4.6 row *A fridge or
//! freezer reads out of range*, §7.2 `FNB.RUL.05` and `STK.RUL.07`, §3.3
//! `STK.ACL.02`, `STK.ACL.05`, `FNB.ACL.16`; BL-FNB-12).
//!
//! A [`ColdUnit`] is a fridge, a freezer or a hot-holding unit with a range and
//! **a stock location of its own** (a sub-location of the outlet): a unit
//! cannot be set up without one, because the stock in it is what a reading
//! holds. People record [`TemperatureReading`]s; the checklist runs of
//! `wl-ticketing` are not built, so the readings are this app's own records.
//!
//! **`FNB.RUL.05`** ([`react`], compliance-critical, a kernel rule run): a
//! reading outside the unit's range
//!
//! - places a stock hold with the reason `temperature` and the reading as its
//!   reference on the stock at the unit's location, through the kernel's
//!   `StockHoldService`: one open hold per unit, later readings are added to it
//!   as references (the reading records the hold) and stock moved in while it
//!   is open joins it as new lines (the stock module does that on every
//!   arrival and its transfer screen warns first);
//! - opens one equipment task for the kitchen manager, once per unit while it
//!   is open;
//! - tells the chef and the kitchen manager (the holders of `lots.release`).
//!
//! A unit with nothing in it has nothing to hold: the task is opened all the
//! same. A failed run is retried by [`retry_pending`] and, after the last
//! attempt, alerts the owner (`PLT.RUL.04`).
//!
//! The chef decides each line of the hold ([`decide`]): release, discard
//! (wastage, `STK.ACL.02`: above AED 200 a second holder of `lots.release`
//! approves) or return to supplier, and records the reading and the minutes
//! the food was out of range. A temperature hold is released by one person
//! (`STK.ACL.05` lists `recall` only). Food that is not released is never sold:
//! the held quantity is not available for consumption, transfer or count, and
//! a discard is wastage (`FNB.ACL.16`). `STK.RUL.07` is the stock module's
//! reminder: H = 2 hours for the chef, the kitchen manager after 2H (host
//! setting).

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::automation::{RuleRunSpec, run_rule};
use wl_kernel::compliance::CriticalRule;
use wl_kernel::inventory::{
    HoldDecision, HoldView, LOCATION_SUBJECT_TYPE, PlaceHold, STOCK_HOLD_LINE_SUBJECT_TYPE,
    STOCK_HOLD_SUBJECT_TYPE, stock_hold_service,
};
use wl_kernel::role_tasks::{self, RoleTask};

use crate::money::tenths_text;
use crate::notices::tell_holders;
use crate::permissions;
use crate::placing::refused;
use crate::texts::english;

/// The subject type of a cold unit.
pub const UNIT_SUBJECT_TYPE: &str = "fnb.cold_unit";

/// The subject type of a temperature reading: the reference of the hold.
pub const READING_SUBJECT_TYPE: &str = "fnb.temperature_reading";

/// The rule.
pub const RULE: &str = "FNB.RUL.05";

/// The scheduled task that tries a reaction that failed again.
pub const RETRY_TASK: &str = "fnb:cold-unit-retry";

/// The kinds of unit.
pub mod kind {
    /// A fridge: cold, at most 5 degrees.
    pub const FRIDGE: &str = "fridge";
    /// A freezer: at most minus 18 degrees.
    pub const FREEZER: &str = "freezer";
    /// A hot-holding unit: at least 60 degrees.
    pub const HOT: &str = "hot";
    /// All of them.
    pub const ALL: [&str; 3] = [FRIDGE, FREEZER, HOT];
}

/// The range of a unit of `kind` until the kitchen sets another: (lowest,
/// highest) in tenths of a degree (Dubai: cold at most 5, hot at least 60).
pub fn default_range(kind: &str) -> (Option<i64>, Option<i64>) {
    match kind {
        kind::FREEZER => (None, Some(-180)),
        kind::HOT => (Some(600), None),
        _ => (None, Some(50)),
    }
}

/// What the chef decides for a hold line.
pub mod decision {
    /// The food is available again.
    pub const RELEASE: &str = "release";
    /// The food is wasted.
    pub const DISCARD: &str = "discard";
    /// The food goes back to the supplier.
    pub const RETURN_TO_SUPPLIER: &str = "return_to_supplier";
    /// All of them.
    pub const ALL: [&str; 3] = [RELEASE, DISCARD, RETURN_TO_SUPPLIER];
}

/// A fridge, a freezer or a hot-holding unit that logs temperatures.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_cold_units", unique(code), index(location_id))]
pub struct ColdUnit {
    /// The primary key.
    pub id: i64,
    /// A short key, e.g. `OUT1-F2`.
    pub code: String,
    /// The name shown on the screens, e.g. `Fridge 2`.
    pub name: String,
    /// `fridge`, `freezer` or `hot`, one of [`kind`].
    pub kind: String,
    /// The outlet it is in.
    pub outlet_id: Option<i64>,
    /// Its stock location (`inventory.location`): what a reading holds.
    pub location_id: i64,
    /// The lowest reading in range, in tenths of a degree.
    pub min_tenths: Option<i64>,
    /// The highest reading in range, in tenths of a degree.
    pub max_tenths: Option<i64>,
    /// Whether it logs temperatures.
    pub active: bool,
    /// Its open stock hold (`inventory.stock_hold`), while there is one.
    pub hold_id: Option<i64>,
    /// The equipment task opened for it.
    pub ticket_id: Option<i64>,
    /// Its number.
    pub ticket_number: Option<String>,
    /// Since when it has been out of range.
    pub out_since: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ColdUnit {
    /// The unit as a [`Subject`]: `fnb.cold_unit:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(UNIT_SUBJECT_TYPE, self.id)
    }

    /// The unit's stock location as a [`Subject`].
    pub fn location(&self) -> Subject {
        Subject::new(LOCATION_SUBJECT_TYPE, self.location_id)
    }

    /// The open hold as a [`Subject`], while there is one.
    pub fn hold(&self) -> Option<Subject> {
        self.hold_id
            .map(|id| Subject::new(STOCK_HOLD_SUBJECT_TYPE, id))
    }

    /// Whether `tenths` is inside the unit's range.
    pub fn in_range(&self, tenths: i64) -> bool {
        self.min_tenths.is_none_or(|min| tenths >= min)
            && self.max_tenths.is_none_or(|max| tenths <= max)
    }

    /// The range in words: `at most 5`, `at least 60`, `-5 to 5`.
    pub fn range_text(&self) -> String {
        match (self.min_tenths, self.max_tenths) {
            (Some(min), Some(max)) => format!("{} to {}", tenths_text(min), tenths_text(max)),
            (None, Some(max)) => english("fnb.cold.at_most", &[("limit", &tenths_text(max))]),
            (Some(min), None) => english("fnb.cold.at_least", &[("limit", &tenths_text(min))]),
            (None, None) => String::new(),
        }
    }

    /// The units that log temperatures, by code.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn active(db: &Db) -> Result<Vec<ColdUnit>> {
        ColdUnit::query()
            .where_(ColdUnit::ACTIVE.eq(true))
            .order_by(ColdUnit::CODE)
            .get(db)
            .await
    }
}

/// One temperature reading of a unit.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_temperature_readings", index(unit_id))]
pub struct TemperatureReading {
    /// The primary key.
    pub id: i64,
    /// The unit.
    #[model(references = "fnb_cold_units")]
    pub unit_id: i64,
    /// The reading, in tenths of a degree.
    pub tenths: i64,
    /// Whether it was inside the unit's range when recorded.
    pub in_range: bool,
    /// A note of the person who read it.
    pub note: String,
    /// The user who recorded it.
    pub recorded_by: Option<i64>,
    /// When it was read.
    pub recorded_at: DateTime,
    /// The hold the reading was added to as a reference, or placed.
    pub hold_id: Option<i64>,
    /// When the rule reacted to it.
    pub reacted_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl TemperatureReading {
    /// The reading as a [`Subject`]: the reference of its hold.
    pub fn subject(&self) -> Subject {
        Subject::new(READING_SUBJECT_TYPE, self.id)
    }

    /// The readings of `unit_id`, newest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_unit(db: &Db, unit_id: i64, limit: u64) -> Result<Vec<TemperatureReading>> {
        TemperatureReading::query()
            .where_(TemperatureReading::UNIT_ID.eq(unit_id))
            .order_by_desc(TemperatureReading::ID)
            .limit(limit)
            .get(db)
            .await
    }
}

/// What the chef recorded when deciding a line: the reading and how long the
/// food was out of range.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_hold_decisions", index(unit_id), index(line_id))]
pub struct HoldDecisionRecord {
    /// The primary key.
    pub id: i64,
    /// The unit.
    #[model(references = "fnb_cold_units")]
    pub unit_id: i64,
    /// The hold (`inventory.stock_hold`).
    pub hold_id: i64,
    /// The hold line (`inventory.stock_hold_line`).
    pub line_id: i64,
    /// `release`, `discard` or `return_to_supplier`.
    pub decision: String,
    /// The reading the chef recorded, in tenths of a degree.
    pub reading_tenths: i64,
    /// How long the food was out of range, in minutes.
    pub minutes_out: i64,
    /// The chef.
    pub decided_by: i64,
    /// Whether the decision waits for a second person (a discard above the
    /// wastage limit).
    pub waiting: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl HoldDecisionRecord {
    /// What was recorded for `line_id`, newest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_line(db: &Db, line_id: i64) -> Result<Option<HoldDecisionRecord>> {
        HoldDecisionRecord::query()
            .where_(HoldDecisionRecord::LINE_ID.eq(line_id))
            .order_by_desc(HoldDecisionRecord::ID)
            .first(db)
            .await
    }
}

/// What the kitchen manager enters to set a unit up.
#[derive(Debug, Clone, Default)]
pub struct UnitAsk {
    /// A short key.
    pub code: String,
    /// The name.
    pub name: String,
    /// One of [`kind`].
    pub kind: String,
    /// The outlet.
    pub outlet_id: Option<i64>,
    /// The stock location (`inventory.location` id): required.
    pub location_id: i64,
    /// The lowest reading in range, in tenths; the kind's default when none
    /// is given with the highest.
    pub min_tenths: Option<i64>,
    /// The highest reading in range, in tenths.
    pub max_tenths: Option<i64>,
}

fn require(user: &AuthUser, permission: &str) -> Result {
    if user.has_permission(permission) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// Sets a unit up. A unit needs a stock location: without one nothing could
/// be held.
///
/// # Errors
///
/// `Forbidden` without `cold_units.manage`; 422 without a code, a name, a known
/// kind or a stock location of the stock module, for a code in use, or for a
/// range whose lowest reading is above its highest.
pub async fn create_unit(state: &AppState, user: &AuthUser, ask: &UnitAsk) -> Result<ColdUnit> {
    require(user, permissions::COLD_UNITS_MANAGE)?;
    let code = ask.code.trim().to_owned();
    let name = ask.name.trim().to_owned();
    if code.is_empty() || name.is_empty() {
        return Err(refused(english("fnb.cold.problem.identity", &[])));
    }
    if !kind::ALL.contains(&ask.kind.as_str()) {
        return Err(refused(english("fnb.cold.problem.kind", &[])));
    }
    let known = stock_hold_service(state)
        .locations(state)
        .await?
        .iter()
        .any(|found| found.location.subject_id == ask.location_id);
    if !known {
        return Err(refused(english("fnb.cold.problem.location", &[])));
    }
    if ColdUnit::query()
        .where_(ColdUnit::CODE.eq(code.as_str()))
        .exists(&state.db)
        .await?
    {
        return Err(refused(english(
            "fnb.cold.problem.code_used",
            &[("code", &code)],
        )));
    }
    let (min, max) = if ask.min_tenths.is_none() && ask.max_tenths.is_none() {
        default_range(&ask.kind)
    } else {
        (ask.min_tenths, ask.max_tenths)
    };
    if let (Some(min), Some(max)) = (min, max)
        && min > max
    {
        return Err(refused(english("fnb.cold.problem.range", &[])));
    }
    let mut unit = ColdUnit {
        code,
        name,
        kind: ask.kind.clone(),
        outlet_id: ask.outlet_id,
        location_id: ask.location_id,
        min_tenths: min,
        max_tenths: max,
        active: true,
        ..ColdUnit::default()
    };
    unit.insert(&state.db).await?;
    Ok(unit)
}

/// Records a reading of unit `unit_id`. An out-of-range reading runs
/// `FNB.RUL.05`; the reading is kept whether or not the reaction worked, and a
/// reaction that failed is tried again ([`retry_pending`]).
///
/// # Errors
///
/// `Forbidden` without `temperature.log`; 422 for a unit that does not log
/// temperatures.
pub async fn record(
    state: &AppState,
    user: &AuthUser,
    unit_id: i64,
    tenths: i64,
    note: &str,
) -> Result<TemperatureReading> {
    require(user, permissions::TEMPERATURE_LOG)?;
    let unit = ColdUnit::find_or_404(&state.db, unit_id).await?;
    if !unit.active {
        return Err(refused(english("fnb.cold.problem.inactive", &[])));
    }
    let mut reading = TemperatureReading {
        unit_id,
        tenths,
        in_range: unit.in_range(tenths),
        note: note.trim().to_owned(),
        recorded_by: Some(user.id),
        recorded_at: renox::db::now(),
        ..TemperatureReading::default()
    };
    reading.insert(&state.db).await?;
    if !reading.in_range {
        run(state, &unit, &reading).await?;
    }
    TemperatureReading::find_or_404(&state.db, reading.id).await
}

/// Runs the rule for `reading` as a kernel rule run: a failure is retried and,
/// after the last attempt, alerts the owner.
async fn run(state: &AppState, unit: &ColdUnit, reading: &TemperatureReading) -> Result {
    let spec = RuleRunSpec::new(
        RULE,
        unit.subject(),
        format!("{RULE}:reading:{}", reading.id),
    );
    let (unit_id, reading_id) = (unit.id, reading.id);
    run_rule(state, spec, move |state: AppState| async move {
        react(&state, unit_id, reading_id).await
    })
    .await
    .map(|_| ())
}

/// The reaction of `FNB.RUL.05` to an out-of-range reading (idempotent: a
/// reading is reacted to once).
///
/// # Errors
///
/// Database errors and the stock, ticket and notice modules'.
pub async fn react(state: &AppState, unit_id: i64, reading_id: i64) -> Result {
    let unit = ColdUnit::find_or_404(&state.db, unit_id).await?;
    let reading = TemperatureReading::find_or_404(&state.db, reading_id).await?;
    if reading.reacted_at.is_some() || reading.in_range {
        return Ok(());
    }
    let service = stock_hold_service(state);
    // One open hold per unit: the reading joins it as a reference.
    let mut hold = None;
    if let Some(open) = unit.hold()
        && service
            .view(state, &open)
            .await?
            .is_some_and(|view| view.open)
    {
        hold = Some(open);
    }
    let mut placed = false;
    if hold.is_none() {
        let placing = service
            .place(
                state,
                PlaceHold {
                    reason: "temperature".to_owned(),
                    reference: reading.subject(),
                    location: unit.location(),
                    items: Vec::new(),
                },
            )
            .await;
        match placing {
            Ok(found) => {
                placed = found.is_some();
                hold = found;
            }
            // A unit with nothing in it has nothing to hold; the task is
            // opened all the same.
            Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _)) => {}
            Err(other) => return Err(other),
        }
    }
    let key = task_key(&unit);
    let title = english(
        "fnb.cold.task_title",
        &[
            ("unit", &unit.name),
            ("reading", &tenths_text(reading.tenths)),
        ],
    );
    let description = english(
        "fnb.cold.task_body",
        &[
            ("unit", &unit.name),
            ("reading", &tenths_text(reading.tenths)),
            ("range", &unit.range_text()),
            ("link", &format!("/cold-units/{}", unit.id)),
        ],
    );
    let ticket = role_tasks::open(
        state,
        RoleTask::new(
            unit.subject(),
            permissions::KITCHEN_MANAGER_ROLE,
            &title,
            &key,
        )
        .description(&description),
    )
    .await?;
    let now = renox::db::now();
    let hold_id = hold.as_ref().map(|found| found.subject_id);
    let out_since = unit.out_since.or(Some(reading.recorded_at));
    let ticket_id = ticket.as_ref().map(|found| found.id.0).or(unit.ticket_id);
    let ticket_number = ticket
        .as_ref()
        .map(|found| found.number.clone())
        .or(unit.ticket_number.clone());
    ColdUnit::query()
        .where_(ColdUnit::ID.eq(unit.id))
        .update(
            &state.db,
            &[
                ("hold_id", &hold_id.or(unit.hold_id)),
                ("ticket_id", &ticket_id),
                ("ticket_number", &ticket_number),
                ("out_since", &out_since),
                ("updated_at", &now),
            ],
        )
        .await?;
    TemperatureReading::query()
        .where_(TemperatureReading::ID.eq(reading.id))
        .update(
            &state.db,
            &[
                ("hold_id", &hold_id),
                ("reacted_at", &Some(now)),
                ("updated_at", &now),
            ],
        )
        .await?;
    // The chef and the kitchen manager are told about a new hold; a later
    // reading only joins it.
    if placed || (hold.is_none() && unit.ticket_id.is_none()) {
        tell_holders(
            state,
            unit.subject(),
            &unit.name,
            permissions::LOTS_RELEASE,
            None,
            &english("fnb.cold.notice_title", &[("unit", &unit.name)]),
            &english(
                "fnb.cold.notice_body",
                &[
                    ("unit", &unit.name),
                    ("reading", &tenths_text(reading.tenths)),
                    ("range", &unit.range_text()),
                ],
            ),
            &format!("/cold-units/{}", unit.id),
        )
        .await?;
    }
    Ok(())
}

/// The key the equipment task of `unit` is found by while it is open.
pub fn task_key(unit: &ColdUnit) -> String {
    format!("fnb.cold_unit:{}", unit.id)
}

/// Tries again the reactions that failed (the stock module was busy or the
/// ticket could not be opened): the out-of-range readings of the last day that
/// were not reacted to. Called every few minutes by the scheduler.
///
/// # Errors
///
/// Database errors.
pub async fn retry_pending(state: &AppState) -> Result<usize> {
    let since = renox::db::now() - renox::chrono::TimeDelta::days(1);
    let waiting = TemperatureReading::query()
        .where_(TemperatureReading::IN_RANGE.eq(false))
        .where_(TemperatureReading::REACTED_AT.is_null())
        .where_(TemperatureReading::RECORDED_AT.gte(since))
        .order_by(TemperatureReading::ID)
        .get(&state.db)
        .await?;
    let mut tried = 0;
    for reading in waiting {
        let unit = ColdUnit::find_or_404(&state.db, reading.unit_id).await?;
        run(state, &unit, &reading).await?;
        tried += 1;
    }
    Ok(tried)
}

/// What the chef enters to decide a line.
#[derive(Debug, Clone, Default)]
pub struct DecideAsk {
    /// `release`, `discard` or `return_to_supplier`.
    pub decision: String,
    /// The reading, in tenths of a degree.
    pub reading_tenths: Option<i64>,
    /// How long the food was out of range, in minutes.
    pub minutes_out: Option<i64>,
}

/// What deciding a line did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decided {
    /// The line is decided.
    Done,
    /// A second person must approve the discard first.
    Waiting,
}

/// The chef decides line `line_id` of the open hold of unit `unit_id`.
///
/// # Errors
///
/// 422 without the reading and the minutes out of range, for a decision that
/// is not one of [`decision::ALL`] or a line that is not the unit's; the stock
/// module's refusals (`Forbidden` without `lots.release`).
pub async fn decide(
    state: &AppState,
    user: &AuthUser,
    unit_id: i64,
    line_id: i64,
    ask: &DecideAsk,
) -> Result<Decided> {
    let unit = ColdUnit::find_or_404(&state.db, unit_id).await?;
    if !decision::ALL.contains(&ask.decision.as_str()) {
        return Err(refused(english("fnb.cold.problem.decision", &[])));
    }
    let (Some(reading), Some(minutes)) = (ask.reading_tenths, ask.minutes_out) else {
        return Err(refused(english("fnb.cold.problem.evidence", &[])));
    };
    if minutes < 0 {
        return Err(refused(english("fnb.cold.problem.evidence", &[])));
    }
    let Some(hold) = unit.hold() else {
        return Err(refused(english("fnb.cold.problem.no_hold", &[])));
    };
    let line = Subject::new(STOCK_HOLD_LINE_SUBJECT_TYPE, line_id);
    let service = stock_hold_service(state);
    let belongs = service
        .view(state, &hold)
        .await?
        .is_some_and(|view| view.lines.iter().any(|l| l.line == line));
    if !belongs {
        return Err(refused(english("fnb.cold.problem.not_the_units_line", &[])));
    }
    let outcome = service.decide(state, user, &line, &ask.decision).await?;
    let Some(outcome) = outcome else {
        return Err(refused(english("fnb.cold.problem.stock_off", &[])));
    };
    let waiting = outcome == HoldDecision::Waiting;
    let mut note = HoldDecisionRecord {
        unit_id,
        hold_id: hold.subject_id,
        line_id,
        decision: ask.decision.clone(),
        reading_tenths: reading,
        minutes_out: minutes,
        decided_by: user.id,
        waiting,
        ..HoldDecisionRecord::default()
    };
    note.insert(&state.db).await?;
    Ok(if waiting {
        Decided::Waiting
    } else {
        Decided::Done
    })
}

/// The hold of `unit` with its lines, when it has one.
///
/// # Errors
///
/// The stock module's errors.
pub async fn hold_of(state: &AppState, unit: &ColdUnit) -> Result<Option<HoldView>> {
    match unit.hold() {
        Some(hold) => stock_hold_service(state).view(state, &hold).await,
        None => Ok(None),
    }
}

/// Whether `unit` is on temperature hold now: it has a hold that is open.
///
/// # Errors
///
/// The stock module's errors.
pub async fn on_hold(state: &AppState, unit: &ColdUnit) -> Result<bool> {
    Ok(hold_of(state, unit).await?.is_some_and(|view| view.open))
}

/// Registers the compliance-critical rule (the owner is told when it fails) and
/// the scheduled retry.
pub(crate) fn register(app: &mut Registry) {
    CriticalRule::new(RULE)
        .owner_role(permissions::OPERATIONS_DIRECTOR_ROLE)
        .register();
    app.schedule()
        .every_minutes(5, RETRY_TASK, |state: AppState| async move {
            retry_pending(&state).await.map(|_| ())
        });
}
