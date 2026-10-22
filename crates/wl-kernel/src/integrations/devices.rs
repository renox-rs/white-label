//! Devices at a site that call the platform (wl-kernel §4.5 *Connection
//! kinds*, §7 *Device connections*; BL-KRN-07). A kitchen display, a till or a
//! scanner is a connection whose health signal is its heartbeat: each
//! heartbeat counts as a successful call and a missed one as a failed call, so
//! `degrade`, `go_down` and `recover` work as for any connection.
//!
//! - A heartbeat is missed when none arrives within one interval plus half an
//!   interval of the previous one (or of the window start). The scheduled run
//!   ([`check`], part of `integrations:run`) turns that into `degrade`, with
//!   `failed since` = the time of the last heartbeat (or the window start, if
//!   the last heartbeat was before it), so "down after 5 minutes" means 5
//!   minutes of silence; `go_down` is the usual rule `PLT.RUL.05`.
//! - A device connection has an *active window* in Asia/Dubai time (opening
//!   hours). Outside it silence is not counted: nothing degrades and nothing
//!   goes down; a connection that is `down` when the window closes stays
//!   `down` until a heartbeat arrives, with no new alerts. When the window
//!   opens, silence counts from the window start.
//! - One connection may group the devices of a scope: it is `degraded` when
//!   at least one device is silent and `down` only when none has answered for
//!   N minutes (its `failed since` is the latest heartbeat of any device, so
//!   N counts from the moment the last device went quiet). When one device
//!   answers again a `down` group recovers and degrades again while others
//!   are still silent.
//! - [`heartbeat`] records a device's heartbeat and settles the connection at
//!   once, so a heartbeat after the silence runs `recover` without waiting
//!   for the next scheduled run.
//!
//! Decisions: a device that never sent a heartbeat counts from the time its
//! row was stored; a heartbeat from a device name the connection does not
//! know adds the device (a group learns its members); the window is
//! `HH:MM`-`HH:MM`, a window that ends before it starts runs over midnight and
//! one with equal ends is always active.

use renox::chrono::{Duration, NaiveDateTime, Timelike};
use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::connection::{ConnectionStatus, IntegrationConnection, KIND_DEVICE};
use super::def::get;
use super::service::{ensure, report_failure_since, report_success};
use crate::documents::DUBAI_OFFSET_SECONDS;
use crate::permissions::INTEGRATIONS_MANAGE;
use crate::texts;

/// One device of a device connection (`wl_integration_devices`): when it last
/// sent a heartbeat.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_integration_devices", unique(connection_key, device_key))]
pub struct DeviceBeat {
    /// The primary key.
    pub id: i64,
    /// The connection's key.
    pub connection_key: String,
    /// The device's name within the connection.
    pub device_key: String,
    /// The last heartbeat; `None` when it never sent one.
    pub last_heartbeat_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl DeviceBeat {
    /// The devices of connection `key`, by name.
    pub async fn of_connection(db: &Db, key: &str) -> Result<Vec<DeviceBeat>> {
        DeviceBeat::query()
            .where_(DeviceBeat::CONNECTION_KEY.eq(key))
            .order_by(DeviceBeat::DEVICE_KEY)
            .get(db)
            .await
    }

    /// The moment silence is counted from: the last heartbeat, else the time
    /// the row was stored.
    pub fn since(&self) -> Option<DateTime> {
        self.last_heartbeat_at.or(self.created_at)
    }
}

/// The device `device` of connection `key`, stored when the database lacks
/// it.
pub(crate) async fn ensure_device(db: &Db, key: &str, device: &str) -> Result<DeviceBeat> {
    let found = DeviceBeat::query()
        .where_(DeviceBeat::CONNECTION_KEY.eq(key))
        .where_(DeviceBeat::DEVICE_KEY.eq(device))
        .first(db)
        .await?;
    if let Some(found) = found {
        return Ok(found);
    }
    let made = DeviceBeat::create(
        db,
        DeviceBeat {
            connection_key: key.to_owned(),
            device_key: device.to_owned(),
            ..DeviceBeat::default()
        },
    )
    .await;
    match made {
        Ok(created) => Ok(created),
        // Another worker stored it first.
        Err(error) if error.is_unique_violation() => DeviceBeat::query()
            .where_(DeviceBeat::CONNECTION_KEY.eq(key))
            .where_(DeviceBeat::DEVICE_KEY.eq(device))
            .first(db)
            .await?
            .ok_or(Error::NotFound),
        Err(error) => Err(error),
    }
}

/// `10:00` as minutes after midnight (600); `None` for anything else.
pub fn parse_clock(text: &str) -> Option<i64> {
    let (hours, minutes) = text.trim().split_once(':')?;
    let (hours, minutes): (i64, i64) = (hours.trim().parse().ok()?, minutes.trim().parse().ok()?);
    ((0..24).contains(&hours) && (0..60).contains(&minutes)).then_some(hours * 60 + minutes)
}

/// 600 minutes after midnight as `10:00`.
pub fn clock_text(minutes: i64) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// `at` as Asia/Dubai wall-clock time (UTC+4 all year).
fn local(at: DateTime) -> NaiveDateTime {
    (at + Duration::seconds(i64::from(DUBAI_OFFSET_SECONDS))).naive_utc()
}

/// The instant of the Asia/Dubai wall-clock time `wall`.
fn instant(wall: NaiveDateTime) -> DateTime {
    (wall - Duration::seconds(i64::from(DUBAI_OFFSET_SECONDS))).and_utc()
}

/// Minutes after midnight in Asia/Dubai at `at`.
fn minute_of_day(at: DateTime) -> i64 {
    let time = local(at).time();
    i64::from(time.hour()) * 60 + i64::from(time.minute())
}

/// The active window of `connection`, if it has one.
pub fn window_of(connection: &IntegrationConnection) -> Option<(i64, i64)> {
    connection.window_start.zip(connection.window_end)
}

/// Whether silence counts at `at`: always without a window, else when the
/// Asia/Dubai time is inside it (the end is outside).
pub fn in_window(window: Option<(i64, i64)>, at: DateTime) -> bool {
    match window {
        None => true,
        Some((start, end)) if start == end => true,
        Some((start, end)) => {
            let minute = minute_of_day(at);
            if start < end {
                start <= minute && minute < end
            } else {
                minute >= start || minute < end
            }
        }
    }
}

/// When the window that `at` is inside opened; `None` without a window.
pub fn window_opened_at(window: Option<(i64, i64)>, at: DateTime) -> Option<DateTime> {
    let (start, end) = window?;
    if start == end {
        return None;
    }
    let wall = local(at);
    let day = if start < end || minute_of_day(at) >= start {
        wall.date()
    } else {
        wall.date().pred_opt()?
    };
    let opened = day.and_hms_opt(u32::try_from(start / 60).ok()?, u32::try_from(start % 60).ok()?, 0)?;
    Some(instant(opened))
}

/// How the devices of a connection stand at one moment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Evaluation {
    /// The devices whose heartbeat is missed.
    pub silent: Vec<String>,
    /// The devices that answered in time.
    pub answering: Vec<String>,
    /// When the silence began: the last heartbeat (or the window start, if the
    /// last one was before it) of the silent device, or for a group the latest
    /// of any device. `None` without a device.
    pub failed_since: Option<DateTime>,
}

/// The devices of `connection` at `at`: silent when no heartbeat came within
/// one interval plus half an interval of the previous one (or of the window
/// start). Pure; call it inside the window.
pub fn evaluate(connection: &IntegrationConnection, devices: &[DeviceBeat], at: DateTime) -> Evaluation {
    let window = window_of(connection);
    let opened = if in_window(window, at) {
        window_opened_at(window, at)
    } else {
        None
    };
    let limit = Duration::milliseconds(connection.interval_seconds() * 1500);
    let mut evaluation = Evaluation::default();
    let mut silent_since: Vec<DateTime> = Vec::new();
    let mut latest: Option<DateTime> = None;
    for device in devices {
        let mut reference = device.since().unwrap_or(at);
        if let Some(opened) = opened {
            reference = reference.max(opened);
        }
        latest = Some(latest.map_or(reference, |l| l.max(reference)));
        if at - reference >= limit {
            evaluation.silent.push(device.device_key.clone());
            silent_since.push(reference);
        } else {
            evaluation.answering.push(device.device_key.clone());
        }
    }
    evaluation.failed_since = if connection.grouped {
        latest
    } else {
        silent_since.into_iter().min().or(latest)
    };
    evaluation
}

/// What a heartbeat or a check did to a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beat {
    /// The connection is `disabled`: the heartbeat is kept, nothing else.
    Ignored,
    /// The heartbeat is recorded; the connection kept its state.
    Recorded,
    /// The connection `degrade`d: a device is silent.
    Degraded,
    /// The connection `recover`ed.
    Recovered,
}

/// The error text of a `degrade` caused by `silent` devices.
fn silence_text(silent: &[String]) -> String {
    format!(
        "{}: {}",
        texts::en("wl_kernel.integrations.device.silent"),
        silent.join(", ")
    )
}

/// Settles `connection` with the devices as they stand at `at`. `counting`
/// says whether silence counts now (inside the window); outside it only a
/// recovery happens. `heard` says a heartbeat just arrived (then an `up`
/// connection notes the time of its last success).
async fn settle(
    state: &AppState,
    connection: &IntegrationConnection,
    at: DateTime,
    counting: bool,
    heard: bool,
) -> Result<Beat> {
    let key = connection.key.as_str();
    let devices = DeviceBeat::of_connection(&state.db, key).await?;
    let evaluation = evaluate(connection, &devices, at);
    let none_silent = evaluation.silent.is_empty();
    let some_answering = !evaluation.answering.is_empty();
    match connection.status {
        ConnectionStatus::Disabled => Ok(Beat::Ignored),
        ConnectionStatus::Up => {
            if counting && !none_silent {
                report_failure_since(
                    state,
                    key,
                    &silence_text(&evaluation.silent),
                    evaluation.failed_since,
                )
                .await?;
                Ok(Beat::Degraded)
            } else {
                if heard {
                    report_success(state, key).await?;
                }
                Ok(Beat::Recorded)
            }
        }
        ConnectionStatus::Degraded | ConnectionStatus::Down => {
            if none_silent {
                report_success(state, key).await?;
                return Ok(Beat::Recovered);
            }
            if connection.status == ConnectionStatus::Down {
                // A group is `down` only while no device answers: one that
                // does brings it back, and the silent ones degrade it again.
                if some_answering && connection.grouped {
                    report_success(state, key).await?;
                    if counting {
                        report_failure_since(
                            state,
                            key,
                            &silence_text(&evaluation.silent),
                            evaluation.failed_since,
                        )
                        .await?;
                    }
                    return Ok(Beat::Recovered);
                }
                return Ok(Beat::Recorded);
            }
            // Degraded and still silent: N counts from the right moment.
            if counting && connection.failed_since != evaluation.failed_since {
                IntegrationConnection::where_eq("key", key)
                    .update(
                        &state.db,
                        &[
                            ("failed_since", &evaluation.failed_since),
                            ("updated_at", &Some(now())),
                        ],
                    )
                    .await?;
            }
            Ok(Beat::Recorded)
        }
    }
}

/// A heartbeat of `device` (the connection's own device when `None`) of the
/// device connection `key` arrived now: it is recorded and the connection is
/// settled at once, so a `degraded` or `down` connection `recover`s.
///
/// # Errors
///
/// 400 when `key` is not registered or is not a device connection; a database
/// error.
pub async fn heartbeat(state: &AppState, key: &str, device: Option<&str>) -> Result<Beat> {
    let connection = ensure(&state.db, key).await?;
    if !connection.is_device() {
        return Err(Error::BadRequest(texts::en(
            "wl_kernel.integrations.device.not_a_device",
        )));
    }
    let name = device.map(str::trim).filter(|d| !d.is_empty()).unwrap_or(key);
    let row = ensure_device(&state.db, key, name).await?;
    let at = now();
    DeviceBeat::query()
        .where_(DeviceBeat::ID.eq(row.id))
        .update(
            &state.db,
            &[("last_heartbeat_at", &Some(at)), ("updated_at", &Some(at))],
        )
        .await?;
    let counting = in_window(window_of(&connection), at);
    settle(state, &connection, at, counting, true).await
}

/// The scheduled check (part of `integrations:run`): every device connection
/// that is not `disabled` and whose window is open is settled, so a missed
/// heartbeat `degrade`s it and a group follows its devices. Silence outside
/// the window is ignored. Returns how many connections changed state.
///
/// # Errors
///
/// A database error.
pub async fn check(state: &AppState) -> Result<usize> {
    let at = now();
    let connections = IntegrationConnection::query()
        .where_(IntegrationConnection::KIND.eq(KIND_DEVICE))
        .where_op("status", "!=", ConnectionStatus::Disabled)
        .order_by(IntegrationConnection::ID)
        .get(&state.db)
        .await?;
    let mut changed = 0;
    for connection in connections {
        if get(&connection.key).is_none() || !in_window(window_of(&connection), at) {
            continue;
        }
        match settle(state, &connection, at, true, false).await? {
            Beat::Degraded | Beat::Recovered => changed += 1,
            Beat::Ignored | Beat::Recorded => {}
        }
    }
    Ok(changed)
}

/// Records a heartbeat for every device of the device connection `key` (the
/// demo's way to show healthy devices); returns how many.
///
/// # Errors
///
/// As [`heartbeat`].
pub async fn beat_all(state: &AppState, key: &str) -> Result<usize> {
    ensure(&state.db, key).await?;
    let devices = DeviceBeat::of_connection(&state.db, key).await?;
    let count = devices.len();
    for device in devices {
        heartbeat(state, key, Some(&device.device_key)).await?;
    }
    Ok(count)
}

/// What an admin changes of a device connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSettings {
    /// The seconds between heartbeats.
    pub heartbeat_seconds: i64,
    /// The active window as `HH:MM` and `HH:MM`; `None`: always.
    pub window: Option<(String, String)>,
    /// N: minutes of silence before the connection goes `down`.
    pub down_after_minutes: i64,
    /// Whether the connection groups the devices of its scope.
    pub grouped: bool,
}

/// An admin sets the heartbeat interval, the active window, N and the grouping
/// of the device connection `key`. The state is not touched: the next check
/// reads the new values.
///
/// # Errors
///
/// 403 without `integrations.manage`; 400 for a connection that is not a
/// device's, an interval outside 10 to 3600 seconds, N below 1, or a window
/// with only one end or a time that is not `HH:MM`.
pub async fn configure(
    state: &AppState,
    user: &AuthUser,
    key: &str,
    settings: DeviceSettings,
) -> Result {
    if !user.has_permission(INTEGRATIONS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let connection = ensure(&state.db, key).await?;
    if !connection.is_device() {
        return Err(Error::BadRequest(texts::en(
            "wl_kernel.integrations.device.not_a_device",
        )));
    }
    if !(10..=3600).contains(&settings.heartbeat_seconds) {
        return Err(Error::BadRequest(texts::en(
            "wl_kernel.integrations.device.interval_range",
        )));
    }
    if settings.down_after_minutes < 1 {
        return Err(Error::BadRequest(texts::en(
            "wl_kernel.integrations.device.down_after_range",
        )));
    }
    let window = match &settings.window {
        None => None,
        Some((from, to)) => match (parse_clock(from), parse_clock(to)) {
            (Some(start), Some(end)) => Some((start, end)),
            _ => {
                return Err(Error::BadRequest(texts::en(
                    "wl_kernel.integrations.device.window_format",
                )));
            }
        },
    };
    IntegrationConnection::where_eq("key", key)
        .update(
            &state.db,
            &[
                ("heartbeat_seconds", &settings.heartbeat_seconds),
                ("window_start", &window.map(|w| w.0)),
                ("window_end", &window.map(|w| w.1)),
                ("down_after_minutes", &settings.down_after_minutes),
                ("grouped", &settings.grouped),
                ("updated_at", &Some(now())),
            ],
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime {
        text.parse().expect("a time")
    }

    /// 12:00 in Asia/Dubai is 08:00 UTC.
    fn dubai(clock: &str) -> DateTime {
        at(&format!("2026-11-03T{clock}Z")) - Duration::hours(4)
    }

    fn device(name: &str, beat: Option<DateTime>, stored: DateTime) -> DeviceBeat {
        DeviceBeat {
            connection_key: "fnb.kds".into(),
            device_key: name.into(),
            last_heartbeat_at: beat,
            created_at: Some(stored),
            ..DeviceBeat::default()
        }
    }

    fn connection(window: Option<(&str, &str)>, grouped: bool) -> IntegrationConnection {
        IntegrationConnection {
            key: "fnb.kds".into(),
            kind: KIND_DEVICE.into(),
            heartbeat_seconds: 60,
            window_start: window.and_then(|w| parse_clock(w.0)),
            window_end: window.and_then(|w| parse_clock(w.1)),
            grouped,
            down_after_minutes: 5,
            ..IntegrationConnection::default()
        }
    }

    #[test]
    fn clock_times_are_minutes_after_midnight() {
        assert_eq!(parse_clock("10:00"), Some(600));
        assert_eq!(parse_clock(" 23:30 "), Some(1410));
        assert_eq!(parse_clock("24:00"), None);
        assert_eq!(parse_clock("10"), None);
        assert_eq!(parse_clock("ten:00"), None);
        assert_eq!(clock_text(600), "10:00");
        assert_eq!(clock_text(1410), "23:30");
    }

    #[test]
    fn the_window_is_in_dubai_time_and_its_end_is_outside() {
        let window = Some((600, 1380));
        assert!(in_window(window, dubai("10:00:00")));
        assert!(in_window(window, dubai("22:59:59")));
        assert!(!in_window(window, dubai("23:00:00")));
        assert!(!in_window(window, dubai("09:59:59")));
        assert!(in_window(None, dubai("03:00:00")));
        // A window over midnight.
        let night = Some((1320, 120));
        assert!(in_window(night, dubai("23:30:00")));
        assert!(in_window(night, dubai("01:00:00")));
        assert!(!in_window(night, dubai("12:00:00")));
        // Equal ends: always.
        assert!(in_window(Some((600, 600)), dubai("03:00:00")));
    }

    #[test]
    fn the_window_opened_at_the_start_of_the_day_or_the_day_before() {
        assert_eq!(
            window_opened_at(Some((600, 1380)), dubai("12:00:00")),
            Some(dubai("10:00:00"))
        );
        assert_eq!(window_opened_at(None, dubai("12:00:00")), None);
        let night = Some((1320, 120));
        assert_eq!(
            window_opened_at(night, dubai("01:00:00")),
            Some(dubai("22:00:00") - Duration::days(1))
        );
        assert_eq!(window_opened_at(night, dubai("23:00:00")), Some(dubai("22:00:00")));
    }

    #[test]
    fn a_heartbeat_is_missed_after_one_and_a_half_intervals() {
        let last = dubai("12:00:00");
        let devices = [device("kds", Some(last), last)];
        let c = connection(Some(("10:00", "23:00")), false);
        let early = evaluate(&c, &devices, dubai("12:01:29"));
        assert!(early.silent.is_empty());
        let missed = evaluate(&c, &devices, dubai("12:01:30"));
        assert_eq!(missed.silent, ["kds"]);
        assert_eq!(missed.failed_since, Some(last));
    }

    #[test]
    fn silence_counts_from_the_window_start_when_the_last_heartbeat_was_before_it() {
        let last = dubai("22:59:00") - Duration::days(1);
        let devices = [device("kds", Some(last), last)];
        let c = connection(Some(("10:00", "23:00")), false);
        let opened = evaluate(&c, &devices, dubai("10:01:29"));
        assert!(opened.silent.is_empty(), "the window start is the reference");
        let missed = evaluate(&c, &devices, dubai("10:01:30"));
        assert_eq!(missed.silent, ["kds"]);
        assert_eq!(missed.failed_since, Some(dubai("10:00:00")));
    }

    #[test]
    fn a_group_counts_from_the_latest_heartbeat_of_any_device() {
        let a = dubai("12:00:00");
        let b = dubai("12:03:00");
        let devices = [device("a", Some(a), a), device("b", Some(b), a)];
        let c = connection(None, true);
        let seen = evaluate(&c, &devices, dubai("12:04:00"));
        assert_eq!(seen.silent, ["a"]);
        assert_eq!(seen.answering, ["b"]);
        assert_eq!(seen.failed_since, Some(b), "N counts from the last answer");
        let single = connection(None, false);
        let alone = evaluate(&single, &devices, dubai("12:04:00"));
        assert_eq!(alone.failed_since, Some(a));
    }

    #[test]
    fn a_device_that_never_beat_counts_from_the_time_it_was_stored() {
        let stored = dubai("12:00:00");
        let devices = [device("kds", None, stored)];
        let c = connection(None, false);
        assert!(evaluate(&c, &devices, dubai("12:01:00")).silent.is_empty());
        assert_eq!(evaluate(&c, &devices, dubai("12:02:00")).silent, ["kds"]);
    }
}
