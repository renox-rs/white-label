//! The crew list page of a work location and day (wl-attendance §5.1 steps 1
//! and 2, issues #178 and #180).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /attendance/crew` | `wl_attendance.crew` | `attendance.mark` at the location |
//!
//! Query: `location_id`, `date` (`2026-11-02`), `employees` (the ids of the
//! crew, comma separated, or the field repeated) and, to narrow the clashes
//! to the hours about to be
//! marked, `from` and `to` (`2026-11-02T07:00`). For each employee the page
//! shows the records they already have at another location that day
//! ([`crate::overlap::clashes_for`]), so the supervisor sees the clash before
//! submitting, and the midday break to keep or remove
//! ([`crate::midday::prefill`]): the form of `POST /attendance/mark` sends it
//! as `break_from` and `break_to`.
//!
//! Without a location or a day the route shows a small form to pick them (the
//! locations the person may mark at, the day, the crew) instead of an error;
//! the form submits to the same route.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::workforce::{EmployeeId, workforce};

use crate::access;
use crate::location::WorkLocation;
use crate::midday;
use crate::overlap;
use crate::pages::{parse_time, text};
use crate::permissions;
use crate::service::refuse;
use crate::texts::english;

/// The texts the template reads as `tx['title']`.
type Texts = BTreeMap<&'static str, String>;

/// The route of the page.
pub fn routes() -> Routes {
    Routes::new()
        .get("/attendance/crew", crew)
        .name("wl_attendance.crew")
        .require_auth()
}

/// What the query holds.
#[derive(Deserialize, Debug, Default)]
pub struct CrewQuery {
    /// The work location.
    pub location_id: i64,
    /// The day.
    pub date: NaiveDate,
    /// The crew, as `1,2,3`.
    #[serde(default)]
    pub employees: String,
    /// When the marked hours start, if known.
    #[serde(default)]
    pub from: String,
    /// When the marked hours end, if known.
    #[serde(default)]
    pub to: String,
}

/// One worker of the crew as the page shows them.
#[derive(Serialize, Debug, Clone)]
pub struct CrewRow {
    /// The employee.
    pub employee: i64,
    /// Their name, `Employee 7` when the workforce does not know it.
    pub name: String,
    /// `error` for a clash, `success` for none.
    pub kind: &'static str,
    /// The status as text.
    pub status: String,
    /// What they are marked for elsewhere.
    pub clashes: Vec<String>,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-attendance/crew.html")]
pub(crate) struct CrewPage {
    tx: Texts,
    location: String,
    date: NaiveDate,
    /// The pre-filled break as `12:30`, none when no break applies.
    break_from: Option<String>,
    break_to: Option<String>,
    break_text: String,
    rows: Vec<CrewRow>,
}

/// The ids of `list`, in order, without repeats; refused when one is not a
/// number.
fn ids_of(lang: &Lang, list: &str) -> Result<Vec<EmployeeId>> {
    let mut found: Vec<EmployeeId> = Vec::new();
    for part in list.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let id: i64 = part.parse().map_err(|_| {
            refuse(format!(
                "employees: {part}: {}",
                text(lang, "record.bad_time")
            ))
        })?;
        if !found.contains(&EmployeeId(id)) {
            found.push(EmployeeId(id));
        }
    }
    Ok(found)
}

/// The form that asks for a location, a day and a crew.
#[derive(Serialize, ViewData)]
#[view(template = "wl-attendance/crew_form.html")]
pub(crate) struct CrewForm {
    tx: Texts,
    /// The locations the person may mark at, as `[id, name]`.
    locations: Vec<(String, String)>,
    /// The location chosen so far.
    location: String,
    /// The day, today when none was given.
    date: String,
    /// The employees of the workforce, as `[id, name]`.
    staff: Vec<(String, String)>,
    /// The employees ticked so far.
    ticked: Vec<String>,
    /// What was wrong with the query, if anything.
    problem: Option<String>,
}

/// The name of `employee`, from their profile's party; `Employee 7` when the
/// workforce or the directory does not know it.
pub(crate) async fn name_of(state: &AppState, lang: &Lang, employee: EmployeeId) -> String {
    let fallback = || english("wl_attendance.crew.employee_number", &[("id", &employee.0)]);
    let Ok(Some(profile)) = workforce(state).staff(state, employee).await else {
        return fallback();
    };
    wl_kernel::parties::directory(state)
        .display_name(&state.db, profile.party, &lang.locale)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(fallback)
}

/// The value of the first field `name` of the query, trimmed; none when it is
/// absent or empty.
fn field<'a>(query: &'a [(String, String)], name: &str) -> Option<&'a str> {
    query
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
}

/// The locations `user` may mark at.
async fn markable(state: &AppState, user: &AuthUser) -> Result<Vec<WorkLocation>> {
    let mut found = Vec::new();
    for location in WorkLocation::query()
        .order_by(WorkLocation::NAME)
        .get(&state.db)
        .await?
    {
        if access::holds(state, user.id, permissions::MARK, &location).await? {
            found.push(location);
        }
    }
    Ok(found)
}

/// The form page, with `problem` when the query was not enough.
async fn form(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    query: &[(String, String)],
    problem: Option<String>,
) -> Result<View> {
    let locations = markable(state, user).await?;
    let mut staff = Vec::new();
    for employee in workforce(state).all_staff(&state.db).await? {
        staff.push((employee.0.to_string(), name_of(state, lang, employee).await));
    }
    let ticked = query
        .iter()
        .filter(|(key, _)| key == "employees")
        .flat_map(|(_, value)| value.split(','))
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
        .collect();
    let tx: Texts = [
        ("title", "crew.title"),
        ("subtitle", "crew.form_subtitle"),
        ("location", "crew.location"),
        ("pick_location", "crew.pick_location"),
        ("date", "crew.date"),
        ("employees", "crew.employees"),
        ("employees_hint", "crew.employees_hint"),
        ("show", "crew.show"),
        ("no_locations", "crew.no_locations"),
    ]
    .into_iter()
    .map(|(name, key)| (name, text(lang, key)))
    .collect();
    Ok(View::typed(CrewForm {
        tx,
        locations: locations
            .iter()
            .map(|location| (location.id.to_string(), location.name.clone()))
            .collect(),
        location: field(query, "location_id").unwrap_or_default().to_owned(),
        date: field(query, "date").map_or_else(|| today().to_string(), str::to_owned),
        staff,
        ticked,
        problem,
    }))
}

/// `GET /attendance/crew`.
async fn crew(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(raw): Query<Vec<(String, String)>>,
) -> Result<View> {
    // A location and a day are needed; without them the person picks them.
    let location_id = field(&raw, "location_id");
    let date = field(&raw, "date");
    if location_id.is_none() && date.is_none() && field(&raw, "employees").is_none() {
        return form(&state, &user, &lang, &raw, None).await;
    }
    let parsed = location_id
        .and_then(|id| id.parse::<i64>().ok())
        .zip(date.and_then(|day| day.parse::<NaiveDate>().ok()));
    let Some((location_id, date)) = parsed else {
        let problem = if location_id.is_none() {
            text(&lang, "crew.pick_location")
        } else if date.and_then(|day| day.parse::<NaiveDate>().ok()).is_none() {
            text(&lang, "crew.bad_date")
        } else {
            text(&lang, "crew.pick_location")
        };
        return form(&state, &user, &lang, &raw, Some(problem)).await;
    };
    let query = CrewQuery {
        location_id,
        date,
        employees: raw
            .iter()
            .filter(|(key, _)| key == "employees")
            .map(|(_, value)| value.as_str())
            .collect::<Vec<_>>()
            .join(","),
        from: field(&raw, "from").unwrap_or_default().to_owned(),
        to: field(&raw, "to").unwrap_or_default().to_owned(),
    };
    let location = WorkLocation::find(&state.db, query.location_id)
        .await?
        .ok_or(Error::NotFound)?;
    if !access::holds(&state, user.id, permissions::MARK, &location).await? {
        return Err(Error::Forbidden);
    }
    let employees = ids_of(&lang, &query.employees)?;
    let hours = if query.from.trim().is_empty() || query.to.trim().is_empty() {
        None
    } else {
        Some((
            parse_time(&state, &lang, "from", &query.from)?,
            parse_time(&state, &lang, "to", &query.to)?,
        ))
    };
    let clashes = overlap::clashes_for(&state, location.id, query.date, &employees, hours).await?;
    let mut rows = Vec::with_capacity(employees.len());
    for employee in &employees {
        let own: Vec<String> = clashes
            .iter()
            .filter(|clash| clash.employee == *employee)
            .map(|clash| {
                english(
                    "wl_attendance.crew.clash_at",
                    &[("location", &clash.location), ("hours", &clash.hours)],
                )
            })
            .collect();
        rows.push(CrewRow {
            employee: employee.0,
            name: name_of(&state, &lang, *employee).await,
            kind: if own.is_empty() { "success" } else { "error" },
            status: text(
                &lang,
                if own.is_empty() {
                    "crew.clear"
                } else {
                    "crew.clash"
                },
            ),
            clashes: own,
        });
    }
    let prefilled = midday::prefill(&state, location.id, query.date).await?;
    let clock = |time: renox::chrono::NaiveTime| time.format("%H:%M").to_string();
    let break_text = match prefilled {
        Some(span) => english(
            "wl_attendance.crew.break",
            &[("from", &clock(span.from)), ("to", &clock(span.to))],
        ),
        None => text(&lang, "crew.no_break"),
    };
    let tx: Texts = [
        ("title", "crew.title"),
        ("subtitle", "crew.subtitle"),
        ("employee", "crew.employee"),
        ("status", "crew.status"),
        ("clashes", "crew.clashes"),
        ("empty", "crew.empty"),
        ("change", "crew.change"),
    ]
    .into_iter()
    .map(|(name, key)| (name, text(&lang, key)))
    .collect();
    Ok(View::typed(CrewPage {
        tx,
        location: location.name.clone(),
        date: query.date,
        break_from: prefilled.map(|span| clock(span.from)),
        break_to: prefilled.map(|span| clock(span.to)),
        break_text,
        rows,
    }))
}
