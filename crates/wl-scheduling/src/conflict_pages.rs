//! The conflict list and its bulk actions, and the manual blocks (wl-scheduling
//! §5.4, §5.5; BL-SCH-05).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /scheduling/conflicts` | `wl_scheduling.conflicts` | `bookings.view` |
//! | `POST /scheduling/conflicts/resolve` | `wl_scheduling.conflicts_resolve` | `bookings.manage` |
//! | `POST /scheduling/blocks` | `wl_scheduling.block_create` | `resources.manage` |
//! | `POST /scheduling/blocks/{id}/remove` | `wl_scheduling.block_remove` | `resources.manage` |
//!
//! Decision: a manual block of reason `sick` without an end blocks the rest of
//! the day from the moment it is recorded (§5.5 step 1).

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::grid::{Action, Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};

use crate::booking::{Booking, BookingStatus, REASON_CODES};
use crate::conflict::{
    self, BLOCK_REASON_CODE, BulkAction, Conflict, day_bounds, open_of, rest_of_day,
};
use crate::pages::{field, number, parse_local, status_text};
use crate::resource::{AvailabilityBlock, Resource};
use crate::service::Actor;
use crate::{permissions, texts};

/// The routes of the conflict list and the manual blocks.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/scheduling/conflicts", index)
                .name("wl_scheduling.conflicts")
                .require_permission(permissions::VIEW),
        )
        .merge(
            Routes::new()
                .post("/scheduling/conflicts/resolve", resolve)
                .name("wl_scheduling.conflicts_resolve")
                .require_permission(permissions::MANAGE),
        )
        .merge(
            Routes::new()
                .post("/scheduling/blocks", create_block)
                .name("wl_scheduling.block_create")
                .post("/scheduling/blocks/{id}/remove", remove_block)
                .name("wl_scheduling.block_remove")
                .require_permission(permissions::RESOURCES_MANAGE),
        )
}

/// The filters of the list.
#[derive(Deserialize, Default)]
struct Filter {
    #[serde(default)]
    resource: String,
    #[serde(default)]
    day: String,
}

/// What the conflict list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-scheduling/conflicts.html")]
pub(crate) struct ConflictsPage {
    tx: BTreeMap<&'static str, String>,
    bookings: GridPage<Booking>,
    targets: Vec<(String, String)>,
    actions: Vec<(String, String)>,
    reasons: Vec<(String, String)>,
    can_manage: bool,
}

/// The conflicts grid: search on the number, filters by resource,
/// status and start, with the bulk actions of the desk. The resources and the
/// causes are drawn by the template (`custom`).
fn conflicts_grid(lang: &Lang, resources: Vec<(String, String)>, can_manage: bool) -> Grid {
    let t = |key: &str| texts::t(lang, &format!("wl_scheduling.{key}"));
    let statuses: Vec<(String, String)> = BookingStatus::ALL
        .iter()
        .map(|status| (status.as_str().to_owned(), status_text(lang, *status)))
        .collect();
    let grid = Grid::new("scheduling-conflicts")
        .title(&t("conflicts.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("number", &t("bookings.number"))
                .frozen()
                .mobile()
                .searchable()
                .link("/scheduling/bookings/{id}"),
        )
        .column(Column::datetime("starts_at", &t("bookings.when")).mobile())
        .column(Column::custom("party", &t("bookings.party")).mobile())
        .column(Column::custom("resources", &t("bookings.resources")).mobile())
        .column(
            Column::select("first_resource", &t("conflicts.resource"), resources)
                .computed(
                    "(SELECT \"br\".\"resource_id\" FROM \"wl_scheduling_booking_resources\" \"br\" \
                     WHERE \"br\".\"booking_id\" = {T}.\"id\" ORDER BY \"br\".\"id\" LIMIT 1)",
                )
                .hidden(),
        )
        .column(Column::custom("causes", &t("conflicts.causes")).mobile())
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&[
                    ("requested", "info"),
                    ("booked", "info"),
                    ("confirmed", "info"),
                    ("checked_in", "info"),
                    ("in_progress", "info"),
                    ("completed", "success"),
                    ("no_show", "danger"),
                    ("cancelled", "warning"),
                ])
                .hidden(),
        )
        .sort_by("starts_at")
        .row_url("/scheduling/bookings/{id}")
        .cards_on_mobile()
        .empty_state(&t("conflicts.empty"), Some(&t("conflicts.empty_hint")));
    if can_manage {
        grid.exports().bulk_action(
            Action::new(&t("conflicts.apply"), "/scheduling/conflicts/resolve")
                .sheet("conflicts-resolve"),
        )
    } else {
        grid
    }
}

/// `GET /scheduling/conflicts`: the conflicted bookings, by start, optionally
/// of one resource (`resource`) and of one day (`day`, `2026-11-03`) in the
/// query string; the grid searches, filters, sorts and pages them.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(filter): Query<Filter>,
    request: GridRequest,
) -> Result<Response> {
    let can_manage = user.has_permission(permissions::MANAGE);
    let resource = filter.resource.trim().parse::<i64>().ok();
    let period = filter.day.trim().parse::<NaiveDate>().ok().map(day_bounds);
    let ids: Vec<i64> = conflict::conflicts(&state.db, resource, period)
        .await?
        .iter()
        .map(|booking| booking.id)
        .collect();
    let active: Vec<Resource> = Resource::query()
        .where_(Resource::ACTIVE.eq(true))
        .order_by(Resource::ID)
        .get(&state.db)
        .await?;
    let named: Vec<(String, String)> = active
        .iter()
        .map(|r| (r.id.to_string(), r.name.clone()))
        .collect();
    let mut targets = vec![(
        String::new(),
        texts::t(&lang, "wl_scheduling.conflicts.any"),
    )];
    targets.extend(named.iter().cloned());
    let actions = ["reassign", "reschedule", "cancel"]
        .iter()
        .map(|action| {
            (
                (*action).to_owned(),
                texts::t(&lang, &format!("wl_scheduling.conflicts.action_{action}")),
            )
        })
        .collect();
    let reasons = REASON_CODES
        .iter()
        .map(|(code, _)| {
            (
                (*code).to_owned(),
                texts::t(&lang, &format!("wl_scheduling.reason.{code}")),
            )
        })
        .collect();
    let grid = conflicts_grid(&lang, named, can_manage);
    if request.param("export").is_some() && !can_manage {
        return Err(Error::Forbidden);
    }
    let query =
        Booking::query().where_(Booking::ID.is_in(if ids.is_empty() { vec![-1] } else { ids }));
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    // The resources and the open causes of the bookings on this page.
    let mut details = BTreeMap::new();
    for booking in page.items() {
        let mut names = Vec::new();
        for id in booking.resource_ids(&state.db).await? {
            if let Some(resource) = Resource::find(&state.db, id).await? {
                names.push(resource.name);
            }
        }
        let causes = open_of(&state.db, booking.id)
            .await?
            .iter()
            .map(|row: &Conflict| row.cause.clone())
            .collect::<Vec<_>>();
        let party = wl_kernel::parties::directory(&state)
            .display_name(&state.db, booking.party(), &lang.locale)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| format!("#{}", booking.party_id));
        details.insert(booking.id, (party, names.join(", "), causes.join(", ")));
    }
    let bookings = page.extend(|booking| {
        let found = details.get(&booking.id);
        json!({
            "party": found.map(|d| d.0.as_str()),
            "resources": found.map(|d| d.1.as_str()),
            "causes": found.map(|d| d.2.as_str()),
        })
    });
    Ok(View::typed(ConflictsPage {
        tx: texts::all(&lang),
        bookings,
        targets,
        actions,
        reasons,
        can_manage,
    })
    .into_response())
}

/// The form of a bulk action: `action` (`reassign`, `reschedule` or `cancel`),
/// the selected `bookings`, and for `reassign` the resource `to` (the first
/// that can take each booking when empty), for `cancel` the `reason_code`.
///
/// Decision: read from the field pairs, because the selection is one
/// `bookings` field per ticked box and the urlencoded deserializer refuses a
/// repeated key (or a single one) for a sequence.
#[derive(Default)]
struct ResolveForm {
    action: String,
    bookings: Vec<i64>,
    to: String,
    reason_code: String,
}

impl ResolveForm {
    /// Collects the fields of the posted pairs; an unknown field is ignored
    /// and a booking that is not a number is skipped.
    fn from_pairs(pairs: Vec<(String, String)>) -> Self {
        let mut form = Self::default();
        for (key, value) in pairs {
            match key.as_str() {
                "action" => form.action = value,
                "to" => form.to = value,
                "reason_code" => form.reason_code = value,
                "bookings" => form.bookings.extend(value.trim().parse::<i64>().ok()),
                // The grid's bulk action sends the selected ids, comma separated.
                "ids" => form.bookings.extend(
                    value
                        .split(',')
                        .filter_map(|id| id.trim().parse::<i64>().ok()),
                ),
                _ => {}
            }
        }
        form
    }
}

/// `POST /scheduling/conflicts/resolve`: one action for the selected
/// bookings; the answer lists what worked and what did not.
async fn resolve(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Result<Response> {
    let form = ResolveForm::from_pairs(pairs);
    if form.bookings.is_empty() {
        return Ok((
            Toast::warning(texts::t(&lang, "wl_scheduling.conflicts.none_selected")),
            Redirect::route("wl_scheduling.conflicts", &[])?,
        )
            .into_response());
    }
    let action = match form.action.as_str() {
        "reassign" => BulkAction::Reassign {
            to: form.to.trim().parse().ok(),
        },
        "cancel" => BulkAction::Cancel {
            reason_code: if form.reason_code.trim().is_empty() {
                BLOCK_REASON_CODE.to_owned()
            } else {
                form.reason_code.trim().to_owned()
            },
        },
        "reschedule" => BulkAction::Reschedule,
        _ => return Err(super::pages::bad_field(&lang, "action")),
    };
    let result = conflict::bulk_resolve(
        &state,
        &form.bookings,
        &action,
        Actor::staff(user.id),
        renox::db::now(),
    )
    .await?;
    let mut message = texts::t_with(
        &lang,
        "wl_scheduling.conflicts.result",
        &[
            ("done", &result.done.len()),
            ("failed", &result.failed.len()),
        ],
    );
    for failed in result.failed.iter().chain(result.warnings.iter()) {
        message.push_str(&format!(" #{}: {}", failed.booking, failed.reason));
    }
    let toast = if result.failed.is_empty() && result.warnings.is_empty() {
        Toast::success(message)
    } else {
        Toast::warning(message)
    };
    Ok((toast, Redirect::route("wl_scheduling.conflicts", &[])?).into_response())
}

/// `POST /scheduling/blocks`: a manual block. Fields: `resource`, `reason`
/// (e.g. `sick`), and optionally `starts_at` (now) and `ends_at` (the end of
/// the day) as `2026-11-03T10:30`, Asia/Dubai. The bookings it overlaps are
/// flagged, not cancelled.
async fn create_block(
    State(state): State<AppState>,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let at = renox::db::now();
    let starts_at = parse_local(field(&form, "starts_at")).unwrap_or(at);
    let ends_at = parse_local(field(&form, "ends_at")).unwrap_or_else(|| rest_of_day(starts_at));
    let reason = field(&form, "reason");
    let block = AvailabilityBlock::manual(
        number(&form, "resource", &lang)?,
        starts_at,
        ends_at,
        if reason.is_empty() { "other" } else { reason },
    );
    let (_, flagged) = conflict::add_block(&state, block, at)
        .await
        .map_err(|refusal| refusal.into_error(&lang))?;
    let message = texts::t_with(
        &lang,
        "wl_scheduling.conflicts.blocked",
        &[("count", &flagged.len())],
    );
    Ok((
        Toast::success(message),
        Redirect::route("wl_scheduling.conflicts", &[])?,
    )
        .into_response())
}

/// `POST /scheduling/blocks/{id}/remove`: takes the block away and clears the
/// conflicts it caused.
async fn remove_block(
    State(state): State<AppState>,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let cleared = conflict::remove_block(&state, id, renox::db::now()).await?;
    let message = texts::t_with(
        &lang,
        "wl_scheduling.conflicts.unblocked",
        &[("count", &cleared)],
    );
    Ok((
        Toast::success(message),
        Redirect::route("wl_scheduling.conflicts", &[])?,
    )
        .into_response())
}
