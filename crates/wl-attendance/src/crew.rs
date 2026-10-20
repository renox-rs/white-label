//! The crew list page of a work location and day (wl-attendance §5.1 steps 1
//! and 2, issues #178 and #180).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /attendance/crew` | `wl_attendance.crew` | `attendance.mark` at the location |
//!
//! Query: `location_id`, `date` (`2026-11-02`), `employees` (the ids of the
//! crew, comma separated) and, to narrow the clashes to the hours about to be
//! marked, `from` and `to` (`2026-11-02T07:00`). For each employee the page
//! shows the records they already have at another location that day
//! ([`crate::overlap::clashes_for`]), so the supervisor sees the clash before
//! submitting, and the midday break to keep or remove
//! ([`crate::midday::prefill`]): the form of `POST /attendance/mark` sends it
//! as `break_from` and `break_to`.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::workforce::EmployeeId;

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

/// `GET /attendance/crew`.
async fn crew(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(query): Query<CrewQuery>,
) -> Result<View> {
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
    let rows = employees
        .iter()
        .map(|employee| {
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
            CrewRow {
                employee: employee.0,
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
            }
        })
        .collect();
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
