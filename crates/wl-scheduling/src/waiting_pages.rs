//! The waiting list for staff (wl-scheduling §4.2, §5.3, BL-SCH-03).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /scheduling/waiting-list` | `wl_scheduling.waiting_list` | `bookings.view` |
//! | `GET /scheduling/waiting-list/{id}` | `wl_scheduling.waiting_entry` | `bookings.view` |
//! | `POST /scheduling/waiting-list` | `wl_scheduling.waiting_join` | `bookings.manage` |
//! | `POST /scheduling/waiting-list/{id}/offer` | `wl_scheduling.waiting_offer` | `bookings.manage` |
//! | `POST /scheduling/waiting-list/{id}/accept` | `wl_scheduling.waiting_accept` | `bookings.manage` |
//! | `POST /scheduling/waiting-list/{id}/withdraw` | `wl_scheduling.waiting_withdraw` | `bookings.manage` |
//!
//! The list shows who waits for what, in the order they joined. An entry's
//! page lists the free times that match it (its service, its preferred
//! resource, its window, from [`slots::listing`]) and each is a button that
//! offers the time: the slot is held for the offer hold time. When the party
//! says yes, *Book the offered time* makes the booking; *Remove* withdraws the
//! entry and frees a held slot.
//!
//! Decision: an offer can only be made for a time that matches the entry; the
//! page offers no other and the route refuses it.

use std::collections::BTreeMap;

use renox::chrono::TimeDelta;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::Serialize;
use wl_kernel::PartyId;
use wl_kernel::documents::dubai_text;

use crate::config::config_of;
use crate::pages::{bad_field, field, local_text, number, parse_local};
use crate::resource::{Resource, ServiceType, eligible_resources};
use crate::service::Actor;
use crate::staff_pages::SlotRow;
use crate::waiting::{self, NewEntry, WaitingEntry, WaitingStatus};
use crate::{names, permissions, slots, texts};

/// How many matching times an entry's page lists.
const SHOWN: usize = 12;

/// The routes of the waiting list.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/scheduling/waiting-list", index)
                .name("wl_scheduling.waiting_list")
                .get("/scheduling/waiting-list/{id}", show)
                .name("wl_scheduling.waiting_entry")
                .require_permission(permissions::VIEW),
        )
        .merge(
            Routes::new()
                .post("/scheduling/waiting-list", join)
                .name("wl_scheduling.waiting_join")
                .post("/scheduling/waiting-list/{id}/offer", offer)
                .name("wl_scheduling.waiting_offer")
                .post("/scheduling/waiting-list/{id}/accept", accept)
                .name("wl_scheduling.waiting_accept")
                .post("/scheduling/waiting-list/{id}/withdraw", withdraw)
                .name("wl_scheduling.waiting_withdraw")
                .require_permission(permissions::MANAGE),
        )
}

fn status_text(lang: &Lang, status: WaitingStatus) -> String {
    texts::t(
        lang,
        &format!("wl_scheduling.waiting.status.{}", status.as_str()),
    )
}

fn status_kind(status: WaitingStatus) -> &'static str {
    match status {
        WaitingStatus::Booked => "success",
        WaitingStatus::Offered => "info",
        WaitingStatus::Waiting => "warning",
        WaitingStatus::Expired | WaitingStatus::Withdrawn => "neutral",
    }
}

/// One entry of the list.
#[derive(Serialize, Debug, Clone)]
pub struct EntryRow {
    id: i64,
    party: String,
    service: String,
    with: String,
    window: String,
    status: String,
    status_kind: &'static str,
    offer: String,
}

async fn party_name(state: &AppState, lang: &Lang, party: PartyId) -> String {
    wl_kernel::parties::directory(state)
        .display_name(&state.db, party, &lang.locale)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{}", party.0))
}

async fn resource_name(state: &AppState, id: i64) -> Result<String> {
    Ok(Resource::find(&state.db, id)
        .await?
        .map(|resource| resource.name)
        .unwrap_or_default())
}

async fn row_of(state: &AppState, lang: &Lang, entry: &WaitingEntry) -> Result<EntryRow> {
    let service = ServiceType::find(&state.db, entry.service_type_id)
        .await?
        .map(|s| names::name_in(&s, &lang.locale))
        .unwrap_or_default();
    let with = match entry.preferred_resource_id {
        Some(id) => resource_name(state, id).await?,
        None => texts::t(lang, "wl_scheduling.waiting.any"),
    };
    let window = match (entry.window_from, entry.window_until) {
        (Some(from), Some(until)) => format!("{} – {}", dubai_text(from), dubai_text(until)),
        (Some(from), None) => texts::t_with(
            lang,
            "wl_scheduling.waiting.from",
            &[("when", &dubai_text(from))],
        ),
        (None, Some(until)) => texts::t_with(
            lang,
            "wl_scheduling.waiting.until",
            &[("when", &dubai_text(until))],
        ),
        (None, None) => texts::t(lang, "wl_scheduling.waiting.any_time"),
    };
    let offer = match (entry.status, entry.offered_resource_id, entry.offered_from) {
        (WaitingStatus::Offered, Some(resource), Some(from)) => {
            let mut text = format!(
                "{} · {}",
                dubai_text(from),
                resource_name(state, resource).await?
            );
            if let Some(until) = entry.offer_hold_until {
                text.push_str(&texts::t_with(
                    lang,
                    "wl_scheduling.waiting.held_until",
                    &[("when", &dubai_text(until))],
                ));
            }
            text
        }
        _ => String::new(),
    };
    Ok(EntryRow {
        id: entry.id,
        party: party_name(state, lang, entry.party()).await,
        service,
        with,
        window,
        status: status_text(lang, entry.status),
        status_kind: status_kind(entry.status),
        offer,
    })
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-scheduling/waiting.html")]
pub(crate) struct ListPage {
    tx: BTreeMap<&'static str, String>,
    entries: GridPage<WaitingEntry>,
    services: Vec<(String, String)>,
    resources: Vec<(String, String)>,
    can_manage: bool,
}

/// The waiting-list grid: filters by service, resource
/// and status. The entries still open lead, then the closed ones, in the
/// order they joined. The window and the offer are drawn by the template
/// (`custom`).
fn waiting_grid(
    lang: &Lang,
    services: Vec<(String, String)>,
    resources: Vec<(String, String)>,
    can_export: bool,
) -> Grid {
    let t = |key: &str| texts::t(lang, &format!("wl_scheduling.{key}"));
    let statuses: Vec<(String, String)> = WaitingStatus::ALL
        .iter()
        .map(|status| (status.as_str().to_owned(), status_text(lang, *status)))
        .collect();
    let grid = Grid::new("scheduling-waiting")
        .title(&t("waiting.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        // The party's name comes from the kernel's directory (the parties module
        // is not a dependency), so the template draws it.
        .column(
            Column::custom("party", &t("waiting.party"))
                .frozen()
                .mobile(),
        )
        .column(Column::number("party_id", &t("bookings.party_id")).hidden())
        .column(Column::select("service_type_id", &t("waiting.service"), services).mobile())
        .column(
            Column::select(
                "preferred_resource_id",
                &t("waiting.with"),
                // "Any" is the sheet's blank choice, not a value to filter by.
                resources.into_iter().filter(|(id, _)| !id.is_empty()),
            )
            .mobile(),
        )
        .column(Column::custom("window", &t("waiting.window")))
        .column(Column::custom("offer", &t("waiting.offer")))
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&[
                    ("waiting", "warning"),
                    ("offered", "info"),
                    ("booked", "success"),
                    ("expired", "neutral"),
                    ("withdrawn", "neutral"),
                ])
                .mobile(),
        )
        .column(Column::date("expires_on", &t("waiting.expires_on")).hidden())
        .column(Column::datetime("created_at", &t("fields.created_at")).hidden())
        .column(
            Column::number("open_first", "")
                .computed("CASE WHEN {T}.\"status\" IN ('waiting', 'offered') THEN 0 ELSE 1 END")
                .hidden(),
        )
        .sort_by("open_first,id")
        .row_url("/scheduling/waiting-list/{id}")
        .cards_on_mobile()
        .empty_state(&t("waiting.empty"), None);
    if can_export { grid.exports() } else { grid }
}

/// `GET /scheduling/waiting-list`: the entries still open first, then the
/// closed ones, in the order they joined.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    let can_manage = user.has_permission(permissions::MANAGE);
    let services: Vec<(String, String)> = ServiceType::query()
        .where_(ServiceType::ACTIVE.eq(true))
        .order_by(ServiceType::NAME_EN)
        .get(&state.db)
        .await?
        .iter()
        .map(|s| (s.id.to_string(), names::name_in(s, &lang.locale)))
        .collect();
    let mut resources = vec![(String::new(), texts::t(&lang, "wl_scheduling.waiting.any"))];
    resources.extend(
        Resource::query()
            .where_(Resource::ACTIVE.eq(true))
            .order_by(Resource::NAME)
            .get(&state.db)
            .await?
            .into_iter()
            .map(|r| (r.id.to_string(), r.name)),
    );
    let grid = waiting_grid(&lang, services.clone(), resources.clone(), can_manage);
    if request.param("export").is_some() && !can_manage {
        return Err(Error::Forbidden);
    }
    let query = WaitingEntry::query();
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut rows = BTreeMap::new();
    for entry in page.items() {
        rows.insert(entry.id, row_of(&state, &lang, entry).await?);
    }
    let entries = page.extend(|entry| {
        let row = rows.get(&entry.id);
        json!({
            "party": row.map(|r| r.party.as_str()),
            "url": format!("/scheduling/waiting-list/{}", entry.id),
            "window": row.map(|r| r.window.as_str()),
            "offer": row.map(|r| r.offer.as_str()),
        })
    });
    Ok(View::typed(ListPage {
        tx: texts::all(&lang),
        entries,
        services,
        resources,
        can_manage,
    })
    .into_response())
}

/// What an entry's page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-scheduling/waiting_entry.html")]
pub(crate) struct EntryPage {
    tx: BTreeMap<&'static str, String>,
    entry: EntryRow,
    expires: String,
    can_offer: bool,
    can_accept: bool,
    can_withdraw: bool,
    slots: Vec<SlotRow>,
    no_slots: bool,
}

/// `GET /scheduling/waiting-list/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let entry = WaitingEntry::find_or_404(&state.db, id).await?;
    let manage = user.has_permission(permissions::MANAGE);
    let now = renox::db::now();
    let row = row_of(&state, &lang, &entry).await?;
    let can_offer = manage && entry.status == WaitingStatus::Waiting;
    let mut rows = Vec::new();
    if can_offer {
        for found in matching_slots(&state, &entry, now).await? {
            let mut names = Vec::new();
            for id in &found.resource_ids {
                names.push(resource_name(&state, *id).await?);
            }
            rows.push(SlotRow {
                label: dubai_text(found.start),
                with: names.join(", "),
                resources: found
                    .resource_ids
                    .iter()
                    .map(i64::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
                starts_at: local_text(found.start),
            });
        }
    }
    Ok(View::typed(EntryPage {
        tx: texts::all(&lang),
        expires: entry
            .expires_on
            .map(|day| day.to_string())
            .unwrap_or_default(),
        can_accept: manage && entry.is_held(now),
        can_withdraw: manage
            && matches!(
                entry.status,
                WaitingStatus::Waiting | WaitingStatus::Offered
            ),
        no_slots: can_offer && rows.is_empty(),
        can_offer,
        slots: rows,
        entry: row,
    }))
}

/// The free times that match `entry`: its service, its preferred resource (any
/// that can deliver otherwise) and its window, up to the booking horizon.
async fn matching_slots(
    state: &AppState,
    entry: &WaitingEntry,
    now: renox::db::DateTime,
) -> Result<Vec<slots::FreeSlot>> {
    let Some(service) = ServiceType::find(&state.db, entry.service_type_id).await? else {
        return Ok(Vec::new());
    };
    let eligible = eligible_resources(&state.db, service.id).await?;
    let mut sets = Vec::new();
    if service.required().len() == 1 {
        for resource in eligible {
            if entry
                .preferred_resource_id
                .is_none_or(|id| id == resource.id)
            {
                sets.push(vec![resource]);
            }
        }
    }
    let horizon = now + TimeDelta::days(config_of(state).horizon_days);
    let from = entry.window_from.map_or(now, |from| from.max(now));
    let until = entry
        .window_until
        .map_or(horizon, |until| until.min(horizon));
    if from >= until {
        return Ok(Vec::new());
    }
    let found = slots::listing(state, &service, &sets, from, until, SHOWN * 3, now).await?;
    Ok(found
        .into_iter()
        .filter(|slot| {
            slot.resource_ids
                .first()
                .is_some_and(|resource| entry.matches(service.id, *resource, slot.start, slot.end))
        })
        .take(SHOWN)
        .collect())
}

fn back_to_list(lang: &Lang, key: &str) -> Result<Response> {
    Ok((
        Toast::success(texts::t(lang, key)),
        Redirect::route("wl_scheduling.waiting_list", &[])?,
    )
        .into_response())
}

/// `POST /scheduling/waiting-list`: puts a party on the list. Fields: `party`,
/// `service`, and optionally `resource`, `window_from`, `window_until`
/// (`2026-11-03T10:30`) and `expires_on` (`2026-11-03`).
async fn join(
    State(state): State<AppState>,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let party = PartyId(number(&form, "party", &lang)?);
    let mut new =
        NewEntry::new(party, number(&form, "service", &lang)?).for_subject(party.subject());
    if let Ok(resource) = field(&form, "resource").parse::<i64>() {
        new = new.with(resource);
    }
    new.window_from = parse_local(field(&form, "window_from"));
    new.window_until = parse_local(field(&form, "window_until"));
    if let (Some(from), Some(until)) = (new.window_from, new.window_until)
        && until <= from
    {
        return Err(bad_field(&lang, "window_until"));
    }
    new.expires_on = field(&form, "expires_on").parse().ok();
    waiting::join(&state, new)
        .await
        .map_err(|refusal| refusal.into_error(&lang))?;
    back_to_list(&lang, "wl_scheduling.waiting.added")
}

/// `POST /scheduling/waiting-list/{id}/offer`: `resources` (one id) and
/// `starts_at`; the slot must match what the entry waits for.
async fn offer(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let entry = WaitingEntry::find_or_404(&state.db, id).await?;
    let service = ServiceType::find_or_404(&state.db, entry.service_type_id).await?;
    let resource = number(&form, "resource", &lang)?;
    let start =
        parse_local(field(&form, "starts_at")).ok_or_else(|| bad_field(&lang, "starts_at"))?;
    let end = start + TimeDelta::minutes(service.duration_minutes);
    if !entry.matches(service.id, resource, start, end) {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            texts::t(&lang, "wl_scheduling.problem.offer_mismatch"),
        ));
    }
    waiting::offer(
        &state,
        id,
        resource,
        start,
        end,
        Actor::staff(user.id),
        renox::db::now(),
    )
    .await
    .map_err(|refusal| refusal.into_error(&lang))?;
    Ok((
        Toast::success(texts::t(&lang, "wl_scheduling.waiting.offered")),
        Redirect::route("wl_scheduling.waiting_entry", &[&id])?,
    )
        .into_response())
}

/// `POST /scheduling/waiting-list/{id}/accept`: the party took the offered
/// time; the booking is made.
async fn accept(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let booking = waiting::accept(&state, id, Actor::staff(user.id), renox::db::now())
        .await
        .map_err(|refusal| refusal.into_error(&lang))?;
    Ok((
        Toast::success(texts::t(&lang, "wl_scheduling.booking.booked")),
        Redirect::route("wl_scheduling.booking", &[&booking.id])?,
    )
        .into_response())
}

/// `POST /scheduling/waiting-list/{id}/withdraw`.
async fn withdraw(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    waiting::withdraw(&state, id, Actor::staff(user.id))
        .await
        .map_err(|refusal| refusal.into_error(&lang))?;
    back_to_list(&lang, "wl_scheduling.waiting.removed")
}
