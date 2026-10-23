//! The two scheduled food-safety rounds of an outlet (06 F&B §4.6 rows 1 and
//! 2, §7.2 `FNB.RUL.03` and `FNB.RUL.04`; BL-FNB-17).
//!
//! Both follow the outlet's opening hours, which are the active window of its
//! kitchen devices ([`Fnb::kitchen_devices`](crate::Fnb::kitchen_devices)); an
//! outlet without them has no round.
//!
//! - **`FNB.RUL.03`**, daily: `H` hours before opening (default 1) the
//!   outlet's opening checklist is opened for the day ([`OpeningChecklist`])
//!   and the people who log temperatures are told which units to read; at
//!   opening the checklist is checked, and when a unit has no reading since it
//!   was opened the kitchen managers of the outlet are told which. Until the
//!   checklists of `WRK.CHK.01` exist, the opening checklist is the
//!   temperature round of the outlet's cold units (cleanliness and the person
//!   in charge are not recorded here).
//! - **`FNB.RUL.04`**, every `H` hours while open (default 4): a unit with no
//!   temperature reading in the last `H` hours is named to the kitchen
//!   managers of the outlet.
//!
//! Both are compliance-critical: each planned run is a rule run under its
//! occurrence key, so a run that fails after its last attempt, or one that was
//! planned and never happened (the minute job did not run), alerts the
//! operations director (`PLT.RUL.04`). A run judges the readings stored up to
//! its planned time, so a late run says the same as an early one.

use renox::chrono::{Duration, NaiveDate};
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::automation::{RuleRunSpec, run_rule};
use wl_kernel::compliance::{CriticalRule, Planned};

use crate::business_day::{instant, local, parse_clock};
use crate::catalogue::{OUTLET_SUBJECT_TYPE, Outlet};
use crate::cold::{ColdUnit, TemperatureReading};
use crate::notices::tell_holders;
use crate::permissions;
use crate::settings::settings;
use crate::texts::english;

/// The rule of the opening checklist.
pub const OPENING_RULE: &str = "FNB.RUL.03";

/// The rule of the temperature rounds.
pub const READINGS_RULE: &str = "FNB.RUL.04";

/// The scheduled job that runs what is due.
pub const TASK: &str = "fnb:rounds";

/// How far back the job looks for a run that is due and not yet run, in hours.
const LOOK_BACK_HOURS: i64 = 3;

/// The opening checklist of one outlet for one day.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "fnb_opening_checklists",
    unique(outlet_id, business_date),
    index(outlet_id)
)]
pub struct OpeningChecklist {
    /// The primary key.
    pub id: i64,
    /// The outlet.
    #[model(references = "fnb_outlets")]
    pub outlet_id: i64,
    /// The day it opens on.
    pub business_date: Option<NaiveDate>,
    /// When the checklist was opened (`H` hours before opening).
    pub opened_at: Option<DateTime>,
    /// When the check at opening found every unit read.
    pub completed_at: Option<DateTime>,
    /// When the kitchen managers were told it was not done.
    pub notified_at: Option<DateTime>,
    /// The units with no reading when it was checked.
    pub missing: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl OpeningChecklist {
    /// The checklist of `outlet_id` for `day`, if it was opened.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_day(
        db: &Db,
        outlet_id: i64,
        day: NaiveDate,
    ) -> Result<Option<OpeningChecklist>> {
        OpeningChecklist::query()
            .where_(OpeningChecklist::OUTLET_ID.eq(outlet_id))
            .where_(OpeningChecklist::BUSINESS_DATE.eq(Some(day)))
            .first(db)
            .await
    }
}

/// Which of the three runs a planned time is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// `FNB.RUL.03`, H hours before opening: open the checklist.
    Open,
    /// `FNB.RUL.03`, at opening: check it.
    Check,
    /// `FNB.RUL.04`, every H hours while open.
    Readings,
}

/// One planned run of an outlet.
#[derive(Debug, Clone)]
struct Due {
    step: Step,
    outlet: Outlet,
    at: DateTime,
}

impl Due {
    fn rule(&self) -> &'static str {
        match self.step {
            Step::Open | Step::Check => OPENING_RULE,
            Step::Readings => READINGS_RULE,
        }
    }

    fn subject(&self) -> Subject {
        Subject::new(OUTLET_SUBJECT_TYPE, self.outlet.id)
    }

    /// `FNB.RUL.03:marina:open:2026-11-03T05:00`.
    fn key(&self) -> String {
        let tag = match self.step {
            Step::Open => "open",
            Step::Check => "check",
            Step::Readings => "readings",
        };
        format!(
            "{}:{}:{tag}:{}",
            self.rule(),
            self.outlet.code,
            self.at.format("%Y-%m-%dT%H:%M")
        )
    }
}

/// The runs of every outlet with opening hours planned between `from` and
/// `until`, both included, oldest first.
async fn due_between(state: &AppState, from: DateTime, until: DateTime) -> Result<Vec<Due>> {
    let setting = settings(state);
    let lead = setting.opening_lead_hours.max(0) * 60;
    let gap = setting.reading_gap_hours * 60;
    let mut found = Vec::new();
    for kitchen in &setting.kitchen_devices {
        let Some(outlet) = Outlet::by_code(&state.db, &kitchen.outlet).await? else {
            continue;
        };
        if !outlet.active {
            continue;
        }
        let (Some(start), Some(mut end)) = (parse_clock(&kitchen.from), parse_clock(&kitchen.to))
        else {
            continue;
        };
        if end <= start {
            end += 24 * 60;
        }
        let mut day = local(from).date() - Duration::days(1);
        let last = local(until).date();
        while day <= last {
            let midnight = day.and_hms_opt(0, 0, 0).unwrap_or_default();
            let at = |minutes: i64| instant(midnight + Duration::minutes(minutes));
            let mut push = |step: Step, minutes: i64| {
                found.push(Due {
                    step,
                    outlet: outlet.clone(),
                    at: at(minutes),
                });
            };
            push(Step::Open, start - lead);
            push(Step::Check, start);
            if gap > 0 {
                let mut minute = start + gap;
                while minute < end {
                    push(Step::Readings, minute);
                    minute += gap;
                }
            }
            day += Duration::days(1);
        }
    }
    found.retain(|due| due.at >= from && due.at <= until);
    found.sort_by_key(|due| (due.at, due.outlet.id));
    Ok(found)
}

/// The occurrences of `rule` planned between `from` and `until`, as the
/// kernel's check for missed runs lists them.
async fn planned(
    state: &AppState,
    rule: &str,
    from: DateTime,
    until: DateTime,
) -> Result<Vec<Planned>> {
    Ok(due_between(state, from, until)
        .await?
        .into_iter()
        .filter(|due| due.rule() == rule)
        .map(|due| Planned {
            subject: due.subject(),
            occurrence_key: due.key(),
            planned_at: due.at,
        })
        .collect())
}

/// `14:30` of the Asia/Dubai wall clock.
fn time_text(at: DateTime) -> String {
    local(at).format("%H:%M").to_string()
}

/// The active units of `outlet` with no reading between `since` and `until`.
async fn unread(
    db: &Db,
    outlet: &Outlet,
    since: DateTime,
    until: DateTime,
) -> Result<Vec<ColdUnit>> {
    let units = ColdUnit::query()
        .where_(ColdUnit::OUTLET_ID.eq(Some(outlet.id)))
        .where_(ColdUnit::ACTIVE.eq(true))
        .order_by(ColdUnit::CODE)
        .get(db)
        .await?;
    let mut missing = Vec::new();
    for unit in units {
        let read = TemperatureReading::query()
            .where_(TemperatureReading::UNIT_ID.eq(unit.id))
            .where_(TemperatureReading::RECORDED_AT.gte(since))
            .where_(TemperatureReading::RECORDED_AT.lte(until))
            .exists(db)
            .await?;
        if !read {
            missing.push(unit);
        }
    }
    Ok(missing)
}

fn names(units: &[ColdUnit]) -> String {
    units
        .iter()
        .map(|unit| unit.name.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The checklist of `outlet` for `day`, made when it is not there yet.
async fn checklist(
    state: &AppState,
    outlet: &Outlet,
    day: NaiveDate,
    opened: DateTime,
) -> Result<OpeningChecklist> {
    if let Some(found) = OpeningChecklist::of_day(&state.db, outlet.id, day).await? {
        return Ok(found);
    }
    let mut row = OpeningChecklist {
        outlet_id: outlet.id,
        business_date: Some(day),
        opened_at: Some(opened),
        ..OpeningChecklist::default()
    };
    match row.insert(&state.db).await {
        Ok(_) => Ok(row),
        // Another worker opened it first.
        Err(error) if error.is_unique_violation() => {
            OpeningChecklist::of_day(&state.db, outlet.id, day)
                .await?
                .ok_or(error)
        }
        Err(error) => Err(error),
    }
}

/// `FNB.RUL.03`, H hours before opening: opens the checklist and tells the
/// people who read the units.
async fn open_checklist(state: &AppState, due: &Due) -> Result {
    let lead = Duration::hours(settings(state).opening_lead_hours.max(0));
    let opens = due.at + lead;
    let day = local(opens).date();
    checklist(state, &due.outlet, day, due.at).await?;
    let units = ColdUnit::query()
        .where_(ColdUnit::OUTLET_ID.eq(Some(due.outlet.id)))
        .where_(ColdUnit::ACTIVE.eq(true))
        .order_by(ColdUnit::CODE)
        .get(&state.db)
        .await?;
    if units.is_empty() {
        return Ok(());
    }
    tell_holders(
        state,
        due.subject(),
        &due.outlet.name,
        permissions::TEMPERATURE_LOG,
        Some(due.outlet.scope()),
        &english("fnb.rounds.opened_title", &[("outlet", &due.outlet.name)]),
        &english(
            "fnb.rounds.opened_body",
            &[
                ("outlet", &due.outlet.name),
                ("time", &time_text(opens)),
                ("units", &names(&units)),
            ],
        ),
        "/cold-units",
    )
    .await?;
    Ok(())
}

/// `FNB.RUL.03` at opening: the checklist is done when every unit was read
/// since it was opened; otherwise the kitchen managers of the outlet are told
/// which units were not.
async fn check_checklist(state: &AppState, due: &Due) -> Result {
    let lead = Duration::hours(settings(state).opening_lead_hours.max(0));
    let day = local(due.at).date();
    let row = checklist(state, &due.outlet, day, due.at - lead).await?;
    if row.completed_at.is_some() {
        return Ok(());
    }
    let since = row.opened_at.unwrap_or(due.at - lead);
    let missing = unread(&state.db, &due.outlet, since, due.at).await?;
    let now = renox::db::now();
    if missing.is_empty() {
        OpeningChecklist::query()
            .where_(OpeningChecklist::ID.eq(row.id))
            .update(
                &state.db,
                &[("completed_at", &Some(now)), ("updated_at", &now)],
            )
            .await?;
        return Ok(());
    }
    let list = names(&missing);
    OpeningChecklist::query()
        .where_(OpeningChecklist::ID.eq(row.id))
        .update(
            &state.db,
            &[
                ("notified_at", &Some(now)),
                ("missing", &list),
                ("updated_at", &now),
            ],
        )
        .await?;
    tell_holders(
        state,
        due.subject(),
        &due.outlet.name,
        permissions::COLD_UNITS_MANAGE,
        Some(due.outlet.scope()),
        &english(
            "fnb.rounds.opening_missed_title",
            &[("outlet", &due.outlet.name)],
        ),
        &english(
            "fnb.rounds.opening_missed_body",
            &[
                ("outlet", &due.outlet.name),
                ("time", &time_text(due.at)),
                ("units", &list),
            ],
        ),
        "/cold-units",
    )
    .await?;
    Ok(())
}

/// `FNB.RUL.04`: the units with no reading in the last H hours are named to
/// the kitchen managers of the outlet.
async fn check_readings(state: &AppState, due: &Due) -> Result {
    let hours = settings(state).reading_gap_hours;
    let since = due.at - Duration::hours(hours);
    let missing = unread(&state.db, &due.outlet, since, due.at).await?;
    if missing.is_empty() {
        return Ok(());
    }
    tell_holders(
        state,
        due.subject(),
        &due.outlet.name,
        permissions::COLD_UNITS_MANAGE,
        Some(due.outlet.scope()),
        &english(
            "fnb.rounds.gap_title",
            &[("outlet", &due.outlet.name), ("hours", &hours.to_string())],
        ),
        &english(
            "fnb.rounds.gap_body",
            &[("since", &time_text(since)), ("units", &names(&missing))],
        ),
        "/cold-units",
    )
    .await?;
    Ok(())
}

/// Runs what is planned between `from` and `until` as rule runs; a run that
/// happened already is not run again. Returns how many runs were made.
///
/// # Errors
///
/// Database errors. A failing run is the rule engine's: it retries, then
/// alerts the owner.
pub async fn run_between(state: &AppState, from: DateTime, until: DateTime) -> Result<usize> {
    let mut runs = 0;
    for due in due_between(state, from, until).await? {
        let spec = RuleRunSpec::new(due.rule(), due.subject(), due.key());
        let step = due.clone();
        run_rule(state, spec, move |state: AppState| async move {
            match step.step {
                Step::Open => open_checklist(&state, &step).await,
                Step::Check => check_checklist(&state, &step).await,
                Step::Readings => check_readings(&state, &step).await,
            }
        })
        .await?;
        runs += 1;
    }
    Ok(runs)
}

/// The scheduled job: what became due in the last hours and was not run.
///
/// # Errors
///
/// As [`run_between`].
pub async fn run_due(state: &AppState, now: DateTime) -> Result<usize> {
    run_between(state, now - Duration::hours(LOOK_BACK_HOURS), now).await
}

/// Registers the checklist table, the two compliance-critical rules with the
/// runs the kernel watches, and the minute job.
pub(crate) fn register(app: &mut Registry) {
    app.model::<OpeningChecklist>();
    for rule in [OPENING_RULE, READINGS_RULE] {
        CriticalRule::new(rule)
            .owner_role(permissions::OPERATIONS_DIRECTOR_ROLE)
            .occurrences(move |state, from, until| {
                Box::pin(async move { planned(state, rule, from, until).await })
            })
            .register();
    }
    app.schedule()
        .every_minutes(5, TASK, |state: AppState| async move {
            run_due(&state, renox::db::now()).await.map(|_| ())
        });
}
