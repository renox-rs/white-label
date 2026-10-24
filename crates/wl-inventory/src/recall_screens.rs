//! The recall screens: the list of recalls and the page of one (BL-STK-14,
//! BL-STK-15, BL-STK-16).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /recalls` | `wl_inventory.recalls` | `recalls.view` or `recalls.manage` |
//! | `GET /recalls/{id}` | `wl_inventory.recall` | `recalls.view` or `recalls.manage` |
//!
//! The page shows the recall (source, reason, who opened it), its lots and
//! where each is now, the stock hold that blocks them, how many of the
//! affected parties were told, waived or are still to tell, and the affected
//! lines by lot and record. **No party is named here**: the vertical lists and
//! tells them on its own page (the route named by
//! [`InventoryConfig::recall_notices_route`](crate::InventoryConfig), which
//! the recall page links to), with the access rules and the audit its
//! data needs. A holder of `recalls.manage` also gets the buttons: trace again,
//! waive a line with a reason, the module's own notices when no vertical page
//! is set, and *Close the recall*, which says what still stands in its way (a
//! held line to decide, a line still to tell) and, once closed, the report.
//!
//! The buttons post to the routes of [`crate::recall_pages`].

use crate::screens::dubai_text;
use renox::grid::{Column, Grid, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::Serialize;
use wl_kernel::Subject;

use crate::balance;
use crate::item::Item;
use crate::location::Location;
use crate::lot::{Lot, LotStatus};
use crate::permissions;
use crate::recall::{Recall, RecallStatus, lots_in_scope};
use crate::recall_line::{LineKind, NoticeStatus, RecallLine, lines_of};
use crate::recall_notice::TEMPLATES;
use crate::recall_trace::holds_of;
use crate::stock_hold::{HoldLineStatus, HoldStatus, lines_of as hold_lines};
use crate::stock_screens::{
    Texts, amount, exported, item_name, label, location_name, options, tr, tx, word,
};

/// The templates, compiled in.
pub(crate) const VIEWS: &[(&str, &str)] = &[
    (
        "wl-inventory/recalls.html",
        include_str!("../views/wl-inventory/recalls.html"),
    ),
    (
        "wl-inventory/recall.html",
        include_str!("../views/wl-inventory/recall.html"),
    ),
];

/// The routes of the recall screens.
pub fn routes() -> Routes {
    Routes::new()
        .get("/recalls", recalls)
        .name("wl_inventory.recalls")
        .get("/recalls/{id}", recall)
        .name("wl_inventory.recall")
        .require_auth()
}

/// Whether `user` may see the recalls.
fn may_see(user: &AuthUser) -> bool {
    [permissions::RECALLS_VIEW, permissions::RECALLS_MANAGE]
        .iter()
        .any(|permission| user.allows(permission) || user.has_permission_anywhere(permission))
}

fn status_kind(status: RecallStatus) -> &'static str {
    match status {
        RecallStatus::Open => "error",
        RecallStatus::Traced => "warning",
        RecallStatus::Closed => "success",
    }
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/recalls.html")]
pub(crate) struct RecallsPage {
    tx: Texts,
    rows: renox::grid::GridPage<Recall>,
}

async fn lot_numbers(state: &AppState, recall: i64) -> Result<Vec<Lot>> {
    let mut lots = Vec::new();
    for id in lots_in_scope(&state.db, recall).await? {
        if let Some(lot) = Lot::find(&state.db, id).await? {
            lots.push(lot);
        }
    }
    Ok(lots)
}

/// The recalls grid: search on the number and the reason, filters by source
/// and status. How many lines were told of how many affected comes from the
/// recall's lines; the lots are listed by the template (`custom`).
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let lines =
        "SELECT COUNT(*) FROM \"wl_inventory_recall_lines\" WHERE \"recall_id\" = {T}.\"id\"";
    let grid = Grid::new("inventory-recalls")
        .title(&tr(lang, "recalls.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("number", &tr(lang, "recalls.number"))
                .frozen()
                .mobile()
                .searchable()
                .link("/recalls/{id}"),
        )
        .column(
            Column::text("reason", &tr(lang, "recalls.reason"))
                .mobile()
                .searchable(),
        )
        .column(Column::select(
            "source",
            &tr(lang, "recalls.source"),
            options(
                lang,
                "recall_source",
                &["supplier", "authority", "internal", "complaint"],
            ),
        ))
        .column(Column::custom("lots", &tr(lang, "recalls.lots")))
        .column(
            Column::select(
                "status",
                &tr(lang, "fields.status"),
                options(lang, "recall", &["open", "traced", "closed"]),
            )
            .badges(&[
                ("open", "error"),
                ("traced", "warning"),
                ("closed", "success"),
            ])
            .mobile(),
        )
        .column(Column::datetime("opened_at", &tr(lang, "recalls.opened")))
        .column(
            Column::number("affected", &tr(lang, "recalls.affected_lines"))
                .computed(&format!("({lines})"))
                .hidden(),
        )
        .column(
            Column::number("told", &tr(lang, "recalls.told_lines"))
                .computed(&format!(
                    "({lines} AND \"notice_status\" IN ('sent', 'waived'))"
                ))
                .hidden(),
        )
        .column(Column::custom("progress", &tr(lang, "recalls.affected")).mobile())
        .sort_by("-id")
        .row_url("/recalls/{id}")
        .cards_on_mobile()
        .empty_state(
            &tr(lang, "recalls.empty"),
            Some(&tr(lang, "recalls.empty_hint")),
        );
    if can_export { grid.exports() } else { grid }
}

/// `GET /recalls`: the recalls, newest first, with how far each is.
async fn recalls(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    if !may_see(&user) {
        return Err(Error::Forbidden);
    }
    let can_export = user.allows(permissions::RECALLS_MANAGE)
        || user.has_permission_anywhere(permissions::RECALLS_MANAGE);
    let grid = grid(&lang, can_export);
    let query = Recall::query();
    if let Some(file) = exported(&grid, &query, &request, can_export).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut extra = std::collections::HashMap::new();
    for recall in page.items() {
        let lots = lot_numbers(&state, recall.id)
            .await?
            .iter()
            .map(|lot| lot.number.clone())
            .collect::<Vec<_>>()
            .join(", ");
        extra.insert(recall.id, json!({ "lots": lots }));
    }
    let page = page.extend(|recall| extra.get(&recall.id).cloned().unwrap_or_else(|| json!({})));
    Ok(View::typed(RecallsPage {
        tx: tx(&lang),
        rows: page,
    })
    .into_response())
}

/// One lot of the recall.
#[derive(Serialize, Debug, Clone)]
pub struct LotRow {
    number: String,
    item: String,
    expiry: String,
    status: String,
    status_kind: &'static str,
    places: String,
}

/// One stock hold of the recall.
#[derive(Serialize, Debug, Clone)]
pub struct HoldRow {
    number: String,
    status: String,
    status_kind: &'static str,
    undecided: i64,
}

/// One affected line: a record that got a recalled lot, never a party's name.
#[derive(Serialize, Debug, Clone)]
pub struct LineRow {
    id: i64,
    lot: String,
    record: String,
    day: String,
    mark: String,
    mark_kind: &'static str,
    party: String,
    notice: String,
    notice_kind: &'static str,
    can_waive: bool,
}

/// What the recall page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/recall.html")]
pub(crate) struct RecallPage {
    tx: Texts,
    id: i64,
    number: String,
    reason: String,
    status: String,
    status_kind: &'static str,
    source: String,
    opened: String,
    opened_by: String,
    closed: String,
    lots: Vec<LotRow>,
    holds: Vec<HoldRow>,
    affected: i64,
    told: i64,
    waived: i64,
    to_tell: i64,
    lines: Vec<LineRow>,
    blockers: Vec<String>,
    can_manage: bool,
    can_close: bool,
    sees_holds: bool,
    notices_route: String,
    own_notices: bool,
    templates: Vec<(String, String)>,
    pending_ids: String,
    has_report: bool,
}

/// `GET /recalls/{id}`.
async fn recall(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    if !may_see(&user) {
        return Err(Error::Forbidden);
    }
    let recall = Recall::find_or_404(&state.db, id).await?;
    let manage = user.has_permission(permissions::RECALLS_MANAGE);
    let open_recall = recall.status != RecallStatus::Closed;

    let mut lots = Vec::new();
    for lot in lot_numbers(&state, id).await? {
        let item = Item::find(&state.db, lot.item_id).await?;
        let unit = item
            .as_ref()
            .map(|item| item.base_unit.clone())
            .unwrap_or_default();
        let mut places = Vec::new();
        for (location_id, quantity) in balance::lot_locations(&state.db, lot.id).await? {
            if let Some(location) = Location::find(&state.db, location_id).await? {
                places.push(format!(
                    "{}: {}",
                    location_name(&location),
                    amount(quantity, &unit)
                ));
            }
        }
        lots.push(LotRow {
            number: lot.number.clone(),
            item: item.map(|item| item_name(&item, &lang)).unwrap_or_default(),
            expiry: lot
                .expiry_date
                .map_or_else(|| tr(&lang, "none"), |day| day.to_string()),
            status: word(&lang, "lot", lot.status.as_str()),
            status_kind: match lot.status {
                LotStatus::Released => "success",
                LotStatus::Depleted => "info",
                _ => "error",
            },
            places: places.join(" · "),
        });
    }

    let mut holds = Vec::new();
    let mut undecided_total = 0;
    let mut holds_closed = true;
    for hold in holds_of(&state.db, id).await? {
        let undecided = hold_lines(&state.db, hold.id)
            .await?
            .iter()
            .filter(|line| line.status == HoldLineStatus::Undecided)
            .count() as i64;
        undecided_total += undecided;
        holds_closed &= hold.status == HoldStatus::Closed;
        holds.push(HoldRow {
            number: hold.number.clone().unwrap_or_default(),
            status: word(&lang, "hold", hold.status.as_str()),
            status_kind: match hold.status {
                HoldStatus::Open => "warning",
                HoldStatus::Closed => "success",
                HoldStatus::Cancelled => "info",
            },
            undecided,
        });
    }

    let all = lines_of(&state.db, id).await?;
    let count = |status: NoticeStatus| {
        all.iter()
            .filter(|line| line.notice_status == status)
            .count() as i64
    };
    let (told, waived, to_tell) = (
        count(NoticeStatus::Sent),
        count(NoticeStatus::Waived),
        count(NoticeStatus::Pending),
    );
    let mut lines = Vec::new();
    for line in &all {
        lines.push(line_row(&state, &lang, line, manage && open_recall).await?);
    }
    lines.sort_by(|a, b| (&a.lot, &a.day).cmp(&(&b.lot, &b.day)));

    let mut blockers = Vec::new();
    if !holds_closed {
        blockers.push(said_count(&lang, "blocker_hold", undecided_total));
    }
    if to_tell > 0 {
        blockers.push(said_count(&lang, "blocker_pending", to_tell));
    }
    let notices_route = crate::config_of(&state)
        .recall_notices_route
        .clone()
        .unwrap_or_default();
    let pending_ids = all
        .iter()
        .filter(|line| line.notice_status == NoticeStatus::Pending && line.party_id.is_some())
        .map(|line| line.id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let opened_by = match recall.opened_by {
        Some(user_id) => renox::auth::User::find(&state.db, user_id)
            .await?
            .map(|user| user.name)
            .unwrap_or_default(),
        None => String::new(),
    };
    Ok(View::typed(RecallPage {
        tx: tx(&lang),
        id,
        number: recall.number.clone().unwrap_or_default(),
        reason: recall.reason.clone(),
        status: word(&lang, "recall", recall.status.as_str()),
        status_kind: status_kind(recall.status),
        source: word(&lang, "recall_source", recall.source.as_str()),
        opened: recall.opened_at.map(dubai_text).unwrap_or_default(),
        opened_by,
        closed: recall.closed_at.map(dubai_text).unwrap_or_default(),
        lots,
        holds,
        affected: all.len() as i64,
        told,
        waived,
        to_tell,
        lines,
        can_manage: manage && open_recall,
        can_close: manage && recall.status == RecallStatus::Traced && blockers.is_empty(),
        blockers,
        sees_holds: user.has_permission_anywhere(permissions::STOCK_HOLD)
            || user.has_permission_anywhere(permissions::LOTS_RELEASE),
        own_notices: manage && open_recall && notices_route.is_empty() && !pending_ids.is_empty(),
        notices_route,
        templates: TEMPLATES
            .iter()
            .map(|template| ((*template).to_owned(), word(&lang, "template", template)))
            .collect(),
        pending_ids,
        has_report: manage && recall.report.is_some(),
    }))
}

fn said_count(lang: &Lang, key: &str, count: i64) -> String {
    tr(lang, key).replace(":count", &count.to_string())
}

async fn line_row(
    state: &AppState,
    lang: &Lang,
    line: &RecallLine,
    manage: bool,
) -> Result<LineRow> {
    let lot = match (line.lot_id, line.item_id) {
        (Some(id), _) => Lot::find(&state.db, id)
            .await?
            .map(|lot| lot.number)
            .unwrap_or_default(),
        // An item not tracked by lot names the item.
        (None, Some(item_id)) => Item::find(&state.db, item_id)
            .await?
            .map(|item| item_name(&item, lang))
            .unwrap_or_default(),
        (None, None) => String::new(),
    };
    let record = match (&line.subject_type, line.subject_id) {
        (Some(kind), Some(id)) => {
            let named = label(state, &Subject::new(kind, id)).await;
            if named.is_empty() {
                // No label is registered: the kind of record and its number.
                let kind = kind.rsplit('.').next().unwrap_or(kind).replace('_', " ");
                format!("{kind} #{id}")
            } else {
                named
            }
        }
        _ => String::new(),
    };
    let possible = line.item_id.is_some() && line.lot_id.is_none();
    let (mark, mark_kind) = match line.kind {
        LineKind::Possible if possible => (word(lang, "mark", "possible"), "warning"),
        LineKind::Confirmed => (word(lang, "mark", "confirmed"), "info"),
        LineKind::Uncertain | LineKind::Possible => (word(lang, "mark", "uncertain"), "warning"),
        LineKind::Dispatch => (word(lang, "mark", "dispatch"), "info"),
        LineKind::WalkIn => (word(lang, "mark", "walk_in"), "info"),
        LineKind::Production => (word(lang, "mark", "production"), "info"),
    };
    Ok(LineRow {
        id: line.id,
        lot,
        record,
        day: line
            .occurred_on
            .map(|day| day.to_string())
            .unwrap_or_default(),
        mark,
        mark_kind,
        party: if line.party_id.is_some() {
            tr(lang, "recall_party_known")
        } else {
            tr(lang, "recall_party_unknown")
        },
        notice: word(lang, "notice", line.notice_status.as_str()),
        notice_kind: match line.notice_status {
            NoticeStatus::Sent => "success",
            NoticeStatus::Waived => "info",
            NoticeStatus::Pending => "warning",
        },
        can_waive: manage && line.notice_status == NoticeStatus::Pending,
    })
}
