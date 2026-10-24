//! The acquisition exceptions page and the attribution changes (wl-sales §5.8
//! step 3, `SAL.ACL.09`, issue #139).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /sales/acquisition-exceptions` | `wl_sales.acquisition_exceptions` | `acquisition.reports.view` |
//! | `POST /sales/acquisitions/{id}/changes` | `wl_sales.acquisition_change` | `campaigns.manage` |
//! | `POST /sales/acquisition-changes/{id}/approve` | `wl_sales.acquisition_change.approve` | `campaigns.manage`, not the requester |
//! | `POST /sales/acquisition-changes/{id}/reject` | `wl_sales.acquisition_change.reject` | `campaigns.manage` |
//!
//! The exceptions page lists the acquisitions a sale created with no source
//! (set to *other*), and every change of source or campaign with its reason,
//! requester and approver; a pending change has *approve* and *reject* for
//! another holder of `campaigns.manage`.

use std::collections::BTreeMap;

use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::serde_json::json;
use renox::validation::{Errors, ValidationError};
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;

use crate::acquisition::PartyAcquisition;
use crate::acquisition_change::{
    self, AcquisitionChange, ChangeRefused, ChangeRequest, ChangeStatus, ChangeTarget,
};
use crate::campaign::Campaign;
use crate::lead_source::LeadSource;
use crate::{permissions, texts};

/// The texts a template reads as `tx['acquisitions.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes of the pages, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/sales/acquisition-exceptions", index)
                .name("wl_sales.acquisition_exceptions")
                .require_permission(permissions::ACQUISITION_REPORTS_VIEW),
        )
        .merge(
            Routes::new()
                .post("/sales/acquisitions/{id}/changes", request)
                .name("wl_sales.acquisition_change")
                .post("/sales/acquisition-changes/{id}/approve", approve)
                .name("wl_sales.acquisition_change.approve")
                .post("/sales/acquisition-changes/{id}/reject", reject)
                .name("wl_sales.acquisition_change.reject")
                .require_permission(permissions::CAMPAIGNS_MANAGE),
        )
}

/// The package text `key` (without the `wl_sales.` prefix).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_sales.{key}"))
}

/// A 422 with `message` under `field`, keeping what was typed.
fn invalid(field: &str, message: String, input: &impl Serialize) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    ValidationError::new(errors).with_input(input).into()
}

/// The acquisitions-with-no-source grid: search on the customer, sum of the
/// converted values. The customer's name, the first sale and the button that
/// asks for a change are drawn by the template (`custom`).
pub(crate) fn missing_grid(lang: &Lang, can_export: bool) -> Grid {
    let t = |key: &str| text(lang, key);
    let grid = Grid::new("sales-acquisition-missing")
        .title(&t("acquisitions.missing.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::custom("party", &t("acquisitions.party"))
                .frozen()
                .mobile(),
        )
        .column(
            Column::related(
                "customer",
                &t("acquisitions.party"),
                "wl_parties_parties",
                "party_id",
                "display_name",
            )
            .hidden()
            .searchable(),
        )
        .column(Column::custom("first_sale", &t("acquisitions.first_sale")).mobile())
        .column(Column::date("first_sale_date", &t("acquisitions.first_sale_date")).mobile())
        .column(
            Column::money(
                "first_sale_value",
                &wl_kernel::money_label(&t("acquisitions.value")),
            )
            .mobile()
            .summary(Summary::Sum),
        )
        .column(Column::custom("actions", ""))
        .sort_by("id")
        .cards_on_mobile()
        .empty_state(
            &t("acquisitions.missing.empty"),
            Some(&t("acquisitions.missing.empty_hint")),
        );
    if can_export { grid.exports() } else { grid }
}

/// The changes grid: search on the reason and the people, filters by kind
/// and status. What changes, and the approve and reject buttons, are drawn
/// by the template (`custom`).
pub(crate) fn changes_grid(lang: &Lang, can_export: bool) -> Grid {
    let t = |key: &str| text(lang, key);
    let kinds = vec![
        ("acquisition".to_owned(), t("changes.kind.acquisition")),
        ("lead".to_owned(), t("changes.kind.lead")),
    ];
    let statuses: Vec<(String, String)> = ChangeStatus::ALL
        .iter()
        .map(|status| {
            (
                status.as_str().to_owned(),
                text(lang, &format!("changes.status.{}", status.as_str())),
            )
        })
        .collect();
    let grid = Grid::new("sales-acquisition-changes")
        .title(&t("changes.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(Column::select("kind", &t("changes.kind"), kinds))
        .column(Column::custom("what", &t("changes.what")).frozen().mobile())
        .column(Column::text("reason", &t("changes.reason")).searchable())
        .column(
            Column::related(
                "requester",
                &t("changes.requested_by"),
                "users",
                "requested_by",
                "name",
            )
            .searchable(),
        )
        .column(
            Column::related(
                "approver",
                &t("changes.decided_by"),
                "users",
                "decided_by",
                "name",
            )
            .searchable(),
        )
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&[
                    ("pending", "warning"),
                    ("applied", "success"),
                    ("rejected", "danger"),
                ])
                .mobile(),
        )
        .column(Column::datetime("created_at", &t("changes.created_at")).hidden())
        .column(Column::custom("actions", ""))
        .sort_by("-id")
        .cards_on_mobile()
        .empty_state(&t("changes.empty"), Some(&t("changes.empty_hint")));
    if can_export { grid.exports() } else { grid }
}

/// One change the viewer can decide, for its approve and reject sheets.
#[derive(Serialize, Debug, Clone)]
pub struct DecideSheet {
    /// The change.
    pub id: i64,
    /// `Referral → Paid ads (SPRING26)`.
    pub what: String,
}

/// What the page needs. `tab` is `missing` or `changes`.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/acquisition_exceptions.html")]
pub(crate) struct ExceptionsPage {
    tx: Texts,
    tab: &'static str,
    missing: Option<GridPage<PartyAcquisition>>,
    changes: Option<GridPage<AcquisitionChange>>,
    missing_count: u64,
    pending_count: u64,
    /// The acquisitions of the page, for their change-request sheets.
    request_sheets: Vec<i64>,
    /// The changes of the page the viewer can approve or reject.
    decide_sheets: Vec<DecideSheet>,
    sources: Vec<(String, String)>,
    can_manage: bool,
}

/// `Source (CODE)`, else `—`.
async fn label(
    db: &Db,
    source: Option<i64>,
    campaign: Option<i64>,
    locale: &str,
) -> Result<String> {
    let source = match source {
        Some(id) => LeadSource::find(db, id)
            .await?
            .map_or_else(|| "—".to_owned(), |s| s.name(locale).to_owned()),
        None => "—".to_owned(),
    };
    Ok(match campaign {
        Some(id) => match Campaign::find(db, id).await? {
            Some(c) => format!("{source} ({})", c.code),
            None => source,
        },
        None => source,
    })
}

/// `GET /sales/acquisition-exceptions`: the acquisitions with no source and
/// the changes of source or campaign, one tab each (`?tab=changes`). It opens
/// on the changes when one waits for the viewer's decision.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    let db = &state.db;
    let can_manage = user.has_permission(permissions::CAMPAIGNS_MANAGE);
    let no_source = || {
        PartyAcquisition::query()
            .where_(PartyAcquisition::SOURCE_MISSING.eq(true))
            .where_(PartyAcquisition::SUPERSEDED_BY.is_null())
    };
    let pending =
        || AcquisitionChange::query().where_(AcquisitionChange::STATUS.eq(ChangeStatus::Pending));
    let missing_count = no_source().count(db).await?;
    let pending_count = pending().count(db).await?;
    let waiting_for_me = can_manage
        && pending()
            .where_(AcquisitionChange::REQUESTED_BY.ne(user.id))
            .count(db)
            .await?
            > 0;
    let tab = match request.param("tab") {
        Some("changes") => "changes",
        Some("missing") => "missing",
        _ if waiting_for_me => "changes",
        _ => "missing",
    };
    // The route needs `acquisition.reports.view`, so the viewer may export.
    let (mut missing, mut changes) = (None, None);
    let (mut request_sheets, mut decide_sheets) = (Vec::new(), Vec::new());
    if tab == "missing" {
        let grid = missing_grid(&lang, true);
        if let Some(file) = grid.export(no_source(), &request).await? {
            return Ok(file);
        }
        let directory = wl_kernel::parties::directory(&state);
        let page = grid.page(no_source(), &request).await?;
        let mut extra = Vec::new();
        for row in page.items() {
            let name = directory
                .display_name(db, PartyId(row.party_id), &lang.locale)
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| format!("#{}", row.party_id));
            request_sheets.push(row.id);
            extra.push(json!({
                "party": name,
                "first_sale": row.first_sale().map(|s| s.to_string()).unwrap_or_default(),
            }));
        }
        let mut extra = extra.into_iter();
        missing = Some(page.extend(|_| extra.next().unwrap_or_default()));
    } else {
        let grid = changes_grid(&lang, true);
        if let Some(file) = grid.export(AcquisitionChange::query(), &request).await? {
            return Ok(file);
        }
        let page = grid.page(AcquisitionChange::query(), &request).await?;
        let mut extra = Vec::new();
        for change in page.items() {
            let before = label(
                db,
                change.old_source_id,
                change.old_campaign_id,
                &lang.locale,
            )
            .await?;
            let after = label(
                db,
                Some(change.new_source_id),
                change.new_campaign_id,
                &lang.locale,
            )
            .await?;
            let what = format!("{before} → {after}");
            let can_decide = can_manage
                && change.status == ChangeStatus::Pending
                && change.requested_by != user.id;
            if can_decide {
                decide_sheets.push(DecideSheet {
                    id: change.id,
                    what: what.clone(),
                });
            }
            extra.push(json!({ "what": what, "can_decide": can_decide }));
        }
        let mut extra = extra.into_iter();
        changes = Some(page.extend(|_| extra.next().unwrap_or_default()));
    }
    let sources = LeadSource::ordered(db)
        .await?
        .into_iter()
        .filter(|s| s.active)
        .map(|s| (s.code.clone(), s.name(&lang.locale).to_owned()))
        .collect();
    Ok(View::typed(ExceptionsPage {
        tx: texts::all(&lang),
        tab,
        missing,
        changes,
        missing_count,
        pending_count,
        request_sheets,
        decide_sheets,
        sources,
        can_manage,
    })
    .into_response())
}

/// The change form.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ChangeForm {
    /// The lead source code.
    #[validate(required, max = 64)]
    pub source: String,
    /// The campaign code, if any.
    #[validate(max = 64)]
    #[serde(default)]
    pub campaign: String,
    /// Why.
    #[validate(required, max = 500)]
    pub reason: String,
}

/// The field a refusal belongs to.
fn field_of(refused: ChangeRefused) -> &'static str {
    match refused {
        ChangeRefused::ReasonRequired => "reason",
        ChangeRefused::UnknownCampaign | ChangeRefused::CampaignRequired => "campaign",
        _ => "source",
    }
}

/// `POST /sales/acquisitions/{id}/changes`: asks to change the source or
/// campaign of acquisition `id`; it waits for a second person when the party
/// has a first sale.
async fn request(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ChangeForm>,
) -> Result<Response> {
    let asked = acquisition_change::request(
        &state,
        ChangeRequest {
            target: ChangeTarget::Acquisition(id),
            source: form.source.clone(),
            campaign: Some(form.campaign.clone()).filter(|c| !c.trim().is_empty()),
            reason: form.reason.clone(),
            requested_by: user.id,
        },
    )
    .await?;
    match asked {
        Ok(change) => {
            let key = if change.status == ChangeStatus::Pending {
                "changes.requested"
            } else {
                "changes.applied"
            };
            Ok((Toast::success(text(&lang, key)), renox::HxRefresh).into_response())
        }
        Err(ChangeRefused::NotFound) => Err(Error::NotFound),
        Err(refused) => Err(invalid(field_of(refused), refused.message(), &form)),
    }
}

/// `POST /sales/acquisition-changes/{id}/approve`.
async fn approve(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    match acquisition_change::approve(&state, id, user.id).await? {
        Ok(_) => Ok((
            Toast::success(text(&lang, "changes.approved")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(ChangeRefused::NotFound) => Err(Error::NotFound),
        Err(refused) => Err(refused.into_error()),
    }
}

/// The reject form.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct RejectForm {
    /// Why.
    #[validate(max = 500)]
    #[serde(default)]
    pub reason: String,
}

/// `POST /sales/acquisition-changes/{id}/reject`.
async fn reject(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<RejectForm>,
) -> Result<Response> {
    match acquisition_change::reject(&state, id, user.id, &form.reason).await? {
        Ok(_) => Ok((
            Toast::success(text(&lang, "changes.rejected")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(ChangeRefused::NotFound) => Err(Error::NotFound),
        Err(refused) => Err(refused.into_error()),
    }
}
