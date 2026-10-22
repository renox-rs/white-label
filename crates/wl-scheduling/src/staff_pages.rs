//! The staff pages that make a booking: the form with the availability
//! picker (wl-scheduling §5.1, BL-SCH-02).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /scheduling/bookings/new` | `wl_scheduling.booking_new` | `bookings.manage` |
//! | `POST /scheduling/bookings/new` | `wl_scheduling.booking_book` | `bookings.manage` |
//! | `GET /scheduling/party-options` | `wl_scheduling.party_options` | `bookings.manage` |
//!
//! The form asks for the party, the service and the day (and, for each kind of
//! resource the service needs, one resource or any); the page then lists the
//! free times of that day from [`slots::listing`], the same availability the
//! booking service checks, and each time is a button. A time that was just
//! taken sends the staff back to the page with the booking service's own
//! message (the nearest free times, the waiting list). A day with nothing free
//! offers to put the party on the waiting list.
//!
//! Decision: the party is also the booking's subject (the patient, the
//! student); a vertical whose subject is another record books through its own
//! screen. The booking lies at the location of its first resource that has
//! one.

use std::collections::BTreeMap;

use renox::chrono::{NaiveDate, TimeDelta};
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::PartyId;
use wl_kernel::calendar::BusinessCalendar;

use crate::config::config_of;
use crate::conflict::day_bounds;
use crate::pages::{field, local_text, number, parse_local};
use crate::resource::{Resource, ResourceType, ServiceType, eligible_resources};
use crate::service::{self, Actor, NewBooking};
use crate::{names, permissions, slots, texts};

/// How many free times the picker lists.
const SHOWN: usize = 48;
/// How many sets of resources a search tries (several types multiply).
const MAX_SETS: usize = 12;

/// The routes of the booking form.
pub fn routes() -> Routes {
    Routes::new()
        .get("/scheduling/bookings/new", new_page)
        .name("wl_scheduling.booking_new")
        .post("/scheduling/bookings/new", book)
        .name("wl_scheduling.booking_book")
        .get("/scheduling/party-options", party_options)
        .name("wl_scheduling.party_options")
        .require_permission(permissions::MANAGE)
}

/// One free time to pick.
#[derive(Serialize, Debug, Clone)]
pub struct SlotRow {
    /// The start, in Asia/Dubai.
    pub label: String,
    /// With whom and what.
    pub with: String,
    /// The resources, ids separated by commas.
    pub resources: String,
    /// The start as a form field.
    pub starts_at: String,
}

/// One select of a resource kind the service needs.
#[derive(Serialize, Debug, Clone)]
pub struct KindField {
    /// The form field.
    pub name: String,
    /// What it picks (Person, Room…).
    pub label: String,
    /// The choices: any, then the eligible resources.
    pub options: Vec<(String, String)>,
    /// The chosen one.
    pub selected: String,
}

/// What the booking form needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-scheduling/booking_new.html")]
pub(crate) struct NewPage {
    tx: BTreeMap<&'static str, String>,
    party: String,
    service: String,
    date: String,
    services: Vec<(String, String)>,
    kinds: Vec<KindField>,
    searched: bool,
    slots: Vec<SlotRow>,
    no_slots: bool,
    summary: String,
    day_from: String,
    day_until: String,
    expires_on: String,
}

fn kind_label(lang: &Lang, kind: ResourceType) -> String {
    texts::t(lang, &format!("wl_scheduling.kind.{}", kind.as_str()))
}

/// The local date of `at` in Asia/Dubai.
fn local_day(at: renox::db::DateTime) -> NaiveDate {
    BusinessCalendar::default().local(at).date_naive()
}

/// The sets of resources to try: one of each kind the service needs, the
/// chosen one or every eligible one, as the combinations of at most
/// [`MAX_SETS`].
fn sets_of(kinds: &[(ResourceType, Vec<Resource>)]) -> Vec<Vec<Resource>> {
    let mut sets: Vec<Vec<Resource>> = vec![Vec::new()];
    for (_, options) in kinds {
        let mut next = Vec::new();
        for set in &sets {
            for resource in options {
                let mut more = set.clone();
                more.push(resource.clone());
                next.push(more);
            }
        }
        sets = next;
        sets.truncate(MAX_SETS);
    }
    sets.into_iter().filter(|set| !set.is_empty()).collect()
}

/// `GET /scheduling/bookings/new`: `party`, `service`, `date` (`2026-11-03`)
/// and `resource_<kind>`.
async fn new_page(
    State(state): State<AppState>,
    lang: Lang,
    Query(query): Query<BTreeMap<String, String>>,
) -> Result<View> {
    let now = renox::db::now();
    let services: Vec<ServiceType> = ServiceType::query()
        .where_(ServiceType::ACTIVE.eq(true))
        .order_by(ServiceType::NAME_EN)
        .get(&state.db)
        .await?;
    let service = field(&query, "service")
        .parse::<i64>()
        .ok()
        .and_then(|id| services.iter().find(|s| s.id == id).cloned());
    let date = field(&query, "date")
        .parse::<NaiveDate>()
        .unwrap_or_else(|_| local_day(now) + TimeDelta::days(1));
    let party = field(&query, "party").to_owned();

    // The selects of the kinds the service needs.
    let mut kinds = Vec::new();
    let mut chosen: Vec<(ResourceType, Vec<Resource>)> = Vec::new();
    if let Some(service) = &service {
        let eligible = eligible_resources(&state.db, service.id).await?;
        for kind in service.required() {
            let name = format!("resource_{}", kind.as_str());
            let selected = field(&query, &name).to_owned();
            let of_kind: Vec<Resource> = eligible
                .iter()
                .filter(|resource| resource.resource_type == kind)
                .cloned()
                .collect();
            let mut options = vec![(String::new(), texts::t(&lang, "wl_scheduling.new.any"))];
            options.extend(of_kind.iter().map(|r| (r.id.to_string(), r.name.clone())));
            let picked: Vec<Resource> = match selected.parse::<i64>() {
                Ok(id) => of_kind.iter().filter(|r| r.id == id).cloned().collect(),
                Err(_) => of_kind,
            };
            chosen.push((kind, picked));
            kinds.push(KindField {
                name,
                label: kind_label(&lang, kind),
                options,
                selected,
            });
        }
    }

    let searched = service.is_some() && party.parse::<i64>().is_ok();
    let mut rows = Vec::new();
    let mut summary = String::new();
    if let (true, Some(service)) = (searched, &service) {
        let (day_start, day_end) = day_bounds(date);
        let from = day_start.max(now);
        let horizon = now + TimeDelta::days(config_of(&state).horizon_days);
        if from < day_end && day_start <= horizon {
            let sets = sets_of(&chosen);
            let found = slots::listing(
                &state,
                service,
                &sets,
                from,
                day_end - TimeDelta::minutes(1),
                SHOWN,
                now,
            )
            .await?;
            for slot in found {
                let mut names = Vec::new();
                for id in &slot.resource_ids {
                    if let Some(resource) = Resource::find(&state.db, *id).await? {
                        names.push(resource.name);
                    }
                }
                rows.push(SlotRow {
                    label: local_text(slot.start)[11..].to_owned(),
                    with: names.join(", "),
                    resources: slot
                        .resource_ids
                        .iter()
                        .map(i64::to_string)
                        .collect::<Vec<_>>()
                        .join(","),
                    starts_at: local_text(slot.start),
                });
            }
        }
        summary = texts::t_with(
            &lang,
            "wl_scheduling.new.summary",
            &[
                ("service", &names::name_in(service, &lang.locale)),
                ("date", &date),
            ],
        );
    }

    let options = services
        .iter()
        .map(|s| (s.id.to_string(), names::name_in(s, &lang.locale)))
        .collect();
    Ok(View::typed(NewPage {
        tx: texts::all(&lang),
        party,
        service: service.map(|s| s.id.to_string()).unwrap_or_default(),
        date: date.to_string(),
        services: options,
        kinds,
        searched,
        no_slots: searched && rows.is_empty(),
        slots: rows,
        summary,
        day_from: format!("{date}T00:00"),
        day_until: format!("{date}T23:59"),
        expires_on: date.to_string(),
    }))
}

/// `POST /scheduling/bookings/new`: books the picked time. Fields: `party`,
/// `service`, `resources` (ids, comma separated), `starts_at`, and `date` to
/// come back to. A refusal sends the staff back to the picker with the
/// booking service's message.
async fn book(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let party = number(&form, "party", &lang)?;
    let service_id = number(&form, "service", &lang)?;
    let resources: Vec<i64> = field(&form, "resources")
        .split(',')
        .filter_map(|id| id.trim().parse().ok())
        .collect();
    let starts_at = parse_local(field(&form, "starts_at"))
        .ok_or_else(|| crate::pages::bad_field(&lang, "starts_at"))?;
    let mut location = None;
    for id in &resources {
        if let Some(found) = Resource::find(&state.db, *id)
            .await?
            .and_then(|resource| resource.location())
        {
            location = Some(found);
            break;
        }
    }
    let mut new = NewBooking::new(PartyId(party), service_id, &resources, starts_at)
        .for_subject(PartyId(party).subject());
    new.location = location;
    let notes = field(&form, "notes");
    if !notes.is_empty() {
        new.notes = Some(notes.to_owned());
    }
    let result = service::book(&state, new, Actor::staff(user.id), renox::db::now()).await;
    match result {
        Ok(booking) => Ok((
            Toast::success(texts::t(&lang, "wl_scheduling.booking.booked")),
            Redirect::route("wl_scheduling.booking", &[&booking.id])?,
        )
            .into_response()),
        Err(crate::Refusal::Failed(error)) => Err(error),
        Err(refusal) => {
            let date = field(&form, "date");
            let back =
                format!("/scheduling/bookings/new?party={party}&service={service_id}&date={date}");
            Ok((Toast::error(refusal.message(&lang)), Redirect::to(&back)).into_response())
        }
    }
}

/// `GET /scheduling/party-options`: the parties the picker offers as the
/// person types (`?q=`), or the labels of the values it holds (`?values=`),
/// as `number · name`.
async fn party_options(
    State(state): State<AppState>,
    query: renox::select::OptionQuery,
) -> Result<Json<Vec<renox::select::SelectOption>>> {
    let directory = wl_kernel::parties::directory(&state);
    let mut parties = Vec::new();
    if query.is_lookup() {
        for id in query.values_as::<i64>() {
            parties.extend(directory.find(&state.db, PartyId(id)).await?);
        }
    } else {
        parties = directory.search(&state.db, &query.q, 20).await?;
    }
    Ok(Json(
        parties
            .into_iter()
            .map(|party| {
                renox::select::SelectOption::new(
                    party.id.0,
                    format!("{} · {}", party.code, party.display_name),
                )
            })
            .collect(),
    ))
}
