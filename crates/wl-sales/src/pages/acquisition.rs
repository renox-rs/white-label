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

use renox::prelude::*;
use renox::validation::{Errors, ValidationError};
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;

use crate::acquisition::PartyAcquisition;
use crate::acquisition_change::{
    self, AcquisitionChange, ChangeRefused, ChangeRequest, ChangeStatus, ChangeTarget,
};
use crate::campaign::Campaign;
use crate::lead_source::LeadSource;
use crate::notices::aed;
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

/// One acquisition set with no source.
#[derive(Serialize, Debug, Clone)]
pub struct MissingRow {
    /// The acquisition.
    pub id: i64,
    /// The party's name.
    pub party: String,
    /// The first sale, e.g. `invoicing.invoice:9`.
    pub first_sale: String,
    /// Its date.
    pub first_sale_date: Option<String>,
    /// Its value, `AED 2,000.00`.
    pub value: String,
}

/// One change of source or campaign.
#[derive(Serialize, Debug, Clone)]
pub struct ChangeRow {
    /// The change.
    pub id: i64,
    /// `acquisition` or `lead`, in the page's language.
    pub kind: String,
    /// `Referral → Paid ads (SPRING26)`.
    pub what: String,
    /// Why.
    pub reason: String,
    /// The requester's name.
    pub requested_by: String,
    /// The approver's name, once decided.
    pub decided_by: String,
    /// The status in the page's language.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// Whether the user can approve or reject it now.
    pub can_decide: bool,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/acquisition_exceptions.html")]
pub(crate) struct ExceptionsPage {
    tx: Texts,
    missing: Vec<MissingRow>,
    changes: Vec<ChangeRow>,
    sources: Vec<(String, String)>,
    can_manage: bool,
}

/// The name of user `id`, else `#id`.
async fn user_name(db: &Db, id: Option<i64>) -> String {
    match id {
        Some(id) => User::find(db, id)
            .await
            .ok()
            .flatten()
            .map_or_else(|| format!("#{id}"), |u| u.name),
        None => String::new(),
    }
}

/// `Source (CODE)`, else `—`.
async fn label(db: &Db, source: Option<i64>, campaign: Option<i64>, locale: &str) -> Result<String> {
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

/// `GET /sales/acquisition-exceptions`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    let directory = wl_kernel::parties::directory(&state);
    let mut missing = Vec::new();
    let rows = PartyAcquisition::query()
        .where_(PartyAcquisition::SOURCE_MISSING.eq(true))
        .where_(PartyAcquisition::SUPERSEDED_BY.is_null())
        .order_by(PartyAcquisition::ID)
        .get(&state.db)
        .await?;
    for row in rows {
        let name = directory
            .display_name(&state.db, PartyId(row.party_id), &lang.locale)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| format!("#{}", row.party_id));
        missing.push(MissingRow {
            id: row.id,
            party: name,
            first_sale: row.first_sale().map(|s| s.to_string()).unwrap_or_default(),
            first_sale_date: row.first_sale_date.map(|d| d.to_string()),
            value: format!("AED {}", aed(row.first_sale_value)),
        });
    }
    let can_manage = user.has_permission(permissions::CAMPAIGNS_MANAGE);
    let mut changes = Vec::new();
    let rows = AcquisitionChange::query()
        .order_by_desc(AcquisitionChange::ID)
        .get(&state.db)
        .await?;
    for change in rows {
        let before = label(&state.db, change.old_source_id, change.old_campaign_id, &lang.locale).await?;
        let after = label(&state.db, Some(change.new_source_id), change.new_campaign_id, &lang.locale).await?;
        let (status, kind) = match change.status {
            ChangeStatus::Pending => ("pending", "warning"),
            ChangeStatus::Applied => ("applied", "success"),
            ChangeStatus::Rejected => ("rejected", "error"),
        };
        changes.push(ChangeRow {
            id: change.id,
            kind: text(&lang, &format!("changes.kind.{}", change.kind)),
            what: format!("{before} → {after}"),
            reason: change.reason.clone(),
            requested_by: user_name(&state.db, Some(change.requested_by)).await,
            decided_by: user_name(&state.db, change.decided_by).await,
            status: text(&lang, &format!("changes.status.{status}")),
            status_kind: kind,
            can_decide: can_manage
                && change.status == ChangeStatus::Pending
                && change.requested_by != user.id,
        });
    }
    let sources = LeadSource::ordered(&state.db)
        .await?
        .into_iter()
        .filter(|s| s.active)
        .map(|s| (s.code.clone(), s.name(&lang.locale).to_owned()))
        .collect();
    Ok(View::typed(ExceptionsPage {
        tx: texts::all(&lang),
        missing,
        changes,
        sources,
        can_manage,
    }))
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
        Ok(_) => Ok((Toast::success(text(&lang, "changes.approved")), renox::HxRefresh).into_response()),
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
        Ok(_) => Ok((Toast::success(text(&lang, "changes.rejected")), renox::HxRefresh).into_response()),
        Err(ChangeRefused::NotFound) => Err(Error::NotFound),
        Err(refused) => Err(refused.into_error()),
    }
}
