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
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::dubai_text;

use crate::booking::{Booking, REASON_CODES};
use crate::conflict::{
    self, BLOCK_REASON_CODE, BulkAction, Conflict, day_bounds, open_of, rest_of_day,
};
use crate::pages::{field, number, parse_local};
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
    options: Vec<(String, String)>,
    resources: Vec<(String, String)>,
    targets: Vec<(String, String)>,
    actions: Vec<(String, String)>,
    reasons: Vec<(String, String)>,
    filter_resource: String,
    filter_day: String,
    empty: bool,
    has_rows: bool,
    can_manage: bool,
}

/// `GET /scheduling/conflicts`: the conflicted bookings, by start, optionally
/// of one resource (`resource`) and of one day (`day`, `2026-11-03`).
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(filter): Query<Filter>,
) -> Result<View> {
    let resource = filter.resource.trim().parse::<i64>().ok();
    let period = filter.day.trim().parse::<NaiveDate>().ok().map(day_bounds);
    let mut options = Vec::new();
    for booking in conflict::conflicts(&state.db, resource, period).await? {
        options.push((
            booking.id.to_string(),
            label_of(&state, &lang, &booking).await?,
        ));
    }
    let active: Vec<Resource> = Resource::query()
        .where_(Resource::ACTIVE.eq(true))
        .order_by(Resource::ID)
        .get(&state.db)
        .await?;
    let mut resources = vec![(
        String::new(),
        texts::t(&lang, "wl_scheduling.conflicts.all"),
    )];
    resources.extend(active.iter().map(|r| (r.id.to_string(), r.name.clone())));
    let mut targets = vec![(
        String::new(),
        texts::t(&lang, "wl_scheduling.conflicts.any"),
    )];
    targets.extend(active.iter().map(|r| (r.id.to_string(), r.name.clone())));
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
    Ok(View::typed(ConflictsPage {
        tx: texts::all(&lang),
        empty: options.is_empty(),
        has_rows: !options.is_empty(),
        options,
        resources,
        targets,
        actions,
        reasons,
        filter_resource: filter.resource,
        filter_day: filter.day,
        can_manage: user.has_permission(permissions::MANAGE),
    }))
}

/// One line of the list: number, time, party, resource.
async fn label_of(state: &AppState, lang: &Lang, booking: &Booking) -> Result<String> {
    let party = wl_kernel::parties::directory(state)
        .display_name(&state.db, booking.party(), &lang.locale)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{}", booking.party_id));
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
    let cause = if causes.is_empty() {
        String::new()
    } else {
        format!(" · {}", causes.join(", "))
    };
    Ok(format!(
        "{} · {} · {} · {}{}",
        booking.number,
        dubai_text(booking.starts_at),
        party,
        names.join(", "),
        cause
    ))
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
        _ => return Err(Error::BadRequest("action".to_owned())),
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
        number(&form, "resource")?,
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
