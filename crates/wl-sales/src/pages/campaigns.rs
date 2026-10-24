//! The campaigns back office (wl-sales §6, #138 A19):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /campaigns` | `wl_sales.campaigns` | `campaigns.view` |
//! | `GET /campaigns/{id}` | `wl_sales.campaign` | `campaigns.view` |
//! | `POST /campaigns/{id}/transitions/{name}` | `wl_sales.campaign_transitions.*` | `campaigns.view` and the transition's permission |
//! | `GET /campaigns/new` | `wl_sales.campaign_new` | `campaigns.manage` |
//! | `POST /campaigns` | `wl_sales.campaign_store` | `campaigns.manage` |
//! | `GET /campaigns/{id}/edit` | `wl_sales.campaign_edit` | `campaigns.manage` |
//! | `POST /campaigns/{id}` | `wl_sales.campaign_update` | `campaigns.manage` |
//! | `POST /campaigns/{id}/delete` | `wl_sales.campaign_delete` | `campaigns.manage` |
//! | `POST /campaigns/{id}/reopen` | `wl_sales.campaign_reopen` | `campaigns.manage` |
//! | `POST /campaigns/{id}/costs` | `wl_sales.campaign_cost_store` | `campaigns.manage` |
//! | `POST /campaigns/{id}/broadcast` | `wl_sales.campaign_broadcast` | `campaigns.manage` |
//!
//! The list is a `renox::grid`. The page of one campaign shows its fields,
//! the transition buttons (`reopen` has a sheet for the new end date), the
//! cost entries with the kernel's approval card for each waiting one, the
//! "Add cost" sheet (a manual entry, or a supplier bill line when invoicing
//! provides [`SupplierBills`](wl_kernel::invoicing::SupplierBills)), the
//! kernel's Void item per approved or waiting entry and the history. Void
//! and the approval buttons post to the kernel's routes.

use std::collections::BTreeMap;
use std::sync::Arc;

use renox::auth::User;
use renox::chrono::{Duration, NaiveDate, NaiveDateTime};
use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{self, Available, StateMachine};
use renox::validation::{Errors, ValidationError};
use serde::Serialize;
use wl_kernel::LegalEntity;
use wl_kernel::approvals::{self, web::ApprovalCard};
use wl_kernel::documents::{DUBAI_OFFSET_SECONDS, today};
use wl_kernel::invoicing::{SupplierBillLineId, SupplierBills};
use wl_kernel::voids::web as void_web;

use crate::bill_links::{self, BillLinkInput};
use crate::campaign::{self, Campaign, CampaignRefused, CampaignStatus, MarketingChannel, Purpose};
use crate::cost_entry::{CostEntry, CostStatus};
use crate::costs::{self, CostInput};
use crate::lead_source::LeadSource;
use crate::machine::{ACTIVATE, REOPEN, ReopenInput};
use crate::notices::aed;
use crate::promo_code::PromoCode;
use crate::retention::{self, BroadcastState};
use crate::segment::Segment;
use crate::{permissions, texts};

/// The texts a template reads as `tx['campaigns.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The module's campaign routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/campaigns", index)
                .name("wl_sales.campaigns")
                .get("/campaigns/{id}", show)
                .name("wl_sales.campaign")
                // `activate` asks the vertical's guards first (`CLN.ACL.13`);
                // the other transitions are the kernel's route.
                .post("/campaigns/{id}/transitions/activate", activate)
                .name("wl_sales.campaign_activate")
                .group(
                    "/campaigns",
                    "wl_sales.campaign_transitions.",
                    wl_kernel::machines::routes::<Campaign>(),
                )
                .require_permission(permissions::CAMPAIGNS_VIEW),
        )
        .merge(
            Routes::new()
                .get("/campaigns/new", new_form)
                .name("wl_sales.campaign_new")
                .post("/campaigns", store)
                .name("wl_sales.campaign_store")
                .get("/campaigns/{id}/edit", edit_form)
                .name("wl_sales.campaign_edit")
                .post("/campaigns/{id}", update)
                .name("wl_sales.campaign_update")
                .post("/campaigns/{id}/delete", destroy)
                .name("wl_sales.campaign_delete")
                .post("/campaigns/{id}/reopen", reopen)
                .name("wl_sales.campaign_reopen")
                .post("/campaigns/{id}/costs", store_cost)
                .name("wl_sales.campaign_cost_store")
                .post("/campaigns/{id}/broadcast", request_again)
                .name("wl_sales.campaign_broadcast")
                .require_permission(permissions::CAMPAIGNS_MANAGE),
        )
}

/// The package text `key` (without the `wl_sales.` prefix).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_sales.{key}"))
}

/// `fils` as `AED 4,000.00`.
fn money(fils: i64) -> String {
    format!("AED {}", aed(fils))
}

/// The badge colour of a campaign status.
fn status_kind(status: CampaignStatus) -> &'static str {
    match status {
        CampaignStatus::Draft | CampaignStatus::Ended => "neutral",
        CampaignStatus::Active => "info",
    }
}

/// The badge colour of a cost status.
fn cost_kind(status: CostStatus) -> &'static str {
    match status {
        CostStatus::Draft | CostStatus::Voided => "neutral",
        CostStatus::AwaitingBill | CostStatus::PendingApproval => "warning",
        CostStatus::Approved => "success",
        CostStatus::Rejected => "danger",
    }
}

/// The campaigns grid: search on code and name, filters by status and
/// channel. The costs and the share of the budget they use are computed
/// from the approved entries (the only ones any total counts), in AED.
pub fn grid(lang: &Lang, can_export: bool) -> Grid {
    let options = |keys: Vec<(&str, String)>| -> Vec<(String, String)> {
        keys.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
    };
    let statuses = options(
        CampaignStatus::ALL
            .iter()
            .map(|s| {
                (
                    s.as_str(),
                    text(lang, &format!("campaigns.status.{}", s.as_str())),
                )
            })
            .collect(),
    );
    let channels = options(
        MarketingChannel::ALL
            .iter()
            .map(|c| {
                (
                    c.as_str(),
                    text(lang, &format!("campaigns.channel.{}", c.as_str())),
                )
            })
            .collect(),
    );
    let approved = format!(
        "(SELECT COALESCE(SUM(e.\"amount\"), 0) FROM \"{}\" e WHERE e.\"campaign_id\" = {{T}}.\"id\" AND e.\"status\" = 'approved')",
        CostEntry::TABLE
    );
    let grid = Grid::new("wl-sales-campaigns")
        .title(&text(lang, "campaigns.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("code", &text(lang, "campaigns.fields.code"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/campaigns/{id}"),
        )
        .column(
            Column::text("name", &text(lang, "campaigns.fields.name"))
                .mobile()
                .searchable(),
        )
        .column(Column::select(
            "marketing_channel",
            &text(lang, "campaigns.fields.channel"),
            channels,
        ))
        .column(
            Column::date("start_date", &text(lang, "campaigns.fields.start_date")).mobile(),
        )
        .column(Column::date("end_date", &text(lang, "campaigns.fields.end_date")).hidden())
        .column(
            Column::number("budget_aed", &text(lang, "campaigns.fields.budget_aed"))
                .decimals(2)
                .computed("(1.0 * {T}.\"budget\" / 100)")
                .summary(Summary::Sum),
        )
        .column(
            Column::number("approved_aed", &text(lang, "campaigns.fields.approved_aed"))
                .decimals(2)
                .computed(&format!("(1.0 * {approved} / 100)"))
                .summary(Summary::Sum),
        )
        .column(
            Column::number("used_percent", &text(lang, "campaigns.fields.used_percent"))
                .decimals(0)
                .hidden()
                .computed(&format!(
                    "(CASE WHEN {{T}}.\"budget\" > 0 THEN 100.0 * {approved} / {{T}}.\"budget\" END)"
                )),
        )
        .column(
            Column::select("status", &text(lang, "campaigns.fields.status"), statuses)
                .badges(&[
                    ("draft", "neutral"),
                    ("active", "info"),
                    ("ended", "neutral"),
                ])
                .mobile(),
        )
        .sort_by("-start_date")
        .row_url("/campaigns/{id}")
        .cards_on_mobile()
        .empty_state(
            &text(lang, "campaigns.empty"),
            Some(&text(lang, "campaigns.empty_hint")),
        );
    if can_export { grid.exports() } else { grid }
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/index.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    campaigns: GridPage<Campaign>,
    can_create: bool,
    can_lead_sources: bool,
}

/// `GET /campaigns`: the grid (`?export=` with `campaigns.manage`).
async fn index(user: AuthUser, lang: Lang, request: GridRequest) -> Result<Response> {
    let can_manage = user.has_permission(permissions::CAMPAIGNS_MANAGE);
    let grid = grid(&lang, can_manage);
    if request.param("export").is_some() && !can_manage {
        return Err(Error::Forbidden);
    }
    if let Some(file) = grid.export(Campaign::query(), &request).await? {
        return Ok(file);
    }
    let campaigns = grid.page(Campaign::query(), &request).await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        campaigns,
        can_create: can_manage,
        can_lead_sources: can_manage,
    })
    .into_response())
}

/// One cost entry as the campaign page shows it.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct CostRow {
    id: i64,
    entry_date: NaiveDate,
    amount: String,
    description: String,
    status_label: String,
    status_kind: &'static str,
    requested_by: String,
    approved_by: Option<String>,
    reason: Option<String>,
    bill: Option<String>,
    /// The kernel's Void item, when the viewer may void the entry.
    void: Option<Value>,
}

/// The promo code of a campaign as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct PromoRow {
    code: String,
    valid_to: Option<NaiveDate>,
    uses: String,
}

/// The retention part of a campaign's page: the segment, how the broadcast
/// stands and what it brought. It shows counts, never names.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct RetentionCard {
    segment: String,
    requester: Option<String>,
    state: String,
    not_sent: Option<String>,
    recipients: Option<i64>,
    delivered: i64,
    opted_out: i64,
    excluded: Vec<(String, i64)>,
    returned: i64,
    returned_value: String,
    cost: String,
    response_days: i64,
}

/// What the campaign page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/show.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    campaign: Campaign,
    title: String,
    status_label: String,
    status_kind: &'static str,
    purpose_label: Option<String>,
    channel_label: Option<String>,
    lead_source: Option<String>,
    owner: Option<String>,
    entity: Option<String>,
    budget: Option<String>,
    /// Approved, bill-linked, manual, pending and awaiting-bill sums.
    sums: BTreeMap<&'static str, String>,
    used_percent: Option<String>,
    promo: Option<PromoRow>,
    send_at: Option<String>,
    /// Why a draft cannot be activated yet.
    activation_problem: Option<&'static str>,
    actions: Vec<Available>,
    can_edit: bool,
    can_delete: bool,
    can_reopen: bool,
    can_add_cost: bool,
    /// The retention card of a retention campaign.
    retention: Option<RetentionCard>,
    /// Whether the campaign can ask for a new broadcast now.
    can_request_broadcast: bool,
    /// Whether invoicing provides supplier bills (the sheet then takes a line).
    bills_enabled: bool,
    today: NaiveDate,
    costs: Vec<CostRow>,
    approvals: Vec<ApprovalCard>,
    timeline: Vec<Value>,
}

/// The name of user `id`, if there is one.
async fn user_name(db: &Db, id: Option<i64>) -> Option<String> {
    User::find(db, id?).await.ok().flatten().map(|u| u.name)
}

/// `GET /campaigns/{id}`: the fields, the costs, the approval cards and the
/// actions.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(campaign): Found<Campaign>,
) -> Result<View> {
    let db = state.db.clone();
    let totals = costs::totals(&db, campaign.id, None).await?;
    let mut rows = Vec::new();
    let mut cards = Vec::new();
    for entry in CostEntry::query()
        .where_(CostEntry::CAMPAIGN_ID.eq(campaign.id))
        .order_by(CostEntry::ID)
        .get(&db)
        .await?
    {
        let subject = entry.subject();
        let available = entry.available(&user, &db).await?;
        cards.extend(approvals::cards_for(&db, &subject, &user, &lang).await?);
        rows.push(CostRow {
            id: entry.id,
            entry_date: entry.entry_date,
            amount: money(entry.amount),
            description: entry.description.clone(),
            status_label: text(
                &lang,
                &format!("campaigns.cost_status.{}", cost_status_key(entry.status)),
            ),
            status_kind: cost_kind(entry.status),
            requested_by: user_name(&db, Some(entry.requested_by))
                .await
                .unwrap_or_default(),
            approved_by: user_name(&db, entry.approved_by).await,
            reason: entry.reason.clone(),
            bill: entry
                .supplier_bill_line_id
                .map(|line| format!("#{} / #{line}", entry.supplier_bill_id.unwrap_or_default())),
            void: void_web::action_for(&subject, &available, &lang),
        });
    }
    let lead_source = match campaign.lead_source_id {
        Some(id) => LeadSource::find(&db, id)
            .await?
            .map(|s| s.name(&lang.locale).to_owned()),
        None => None,
    };
    let entity = match campaign.legal_entity_id {
        Some(id) => LegalEntity::find(&db, id).await?.map(|e| e.name_en),
        None => None,
    };
    let promo = PromoCode::of_campaign(&db, campaign.id)
        .await?
        .map(|p| PromoRow {
            code: p.code.clone(),
            valid_to: p.valid_to,
            uses: match p.max_uses {
                Some(max) => format!("{} / {max}", p.uses),
                None => p.uses.to_string(),
            },
        });
    let used_percent = campaign
        .budget
        .filter(|b| *b > 0)
        .map(|b| format!("{}%", totals.approved * 100 / b));
    let sums = BTreeMap::from([
        ("approved", money(totals.approved)),
        ("bill_linked", money(totals.bill_linked)),
        ("manual", money(totals.manual)),
        ("pending", money(totals.pending)),
        ("awaiting_bill", money(totals.awaiting_bill)),
    ]);
    let history = state_machine::history(&db, Campaign::TABLE, campaign.id).await?;
    let timeline = state_machine::timeline(&history, Some(&state_machine::info_of::<Campaign>()));
    // `reopen` has its own sheet (it takes the new end date).
    let actions = campaign
        .available(&user, &db)
        .await?
        .into_iter()
        .filter(|a| a.name != REOPEN)
        .collect();
    let manage = user.has_permission(permissions::CAMPAIGNS_MANAGE);
    let status = campaign.status;
    let retention = retention_card(&state, &lang, &campaign).await?;
    let can_request_broadcast = manage
        && status == CampaignStatus::Active
        && campaign.purpose == Some(Purpose::Retention)
        && retention.as_ref().is_some_and(|c| c.can_request);
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        title: format!("{} - {}", campaign.code, campaign.name),
        status_label: text(&lang, &format!("campaigns.status.{}", status.as_str())),
        status_kind: status_kind(status),
        purpose_label: campaign
            .purpose
            .map(|p| text(&lang, &format!("campaigns.purpose.{}", p.as_str()))),
        channel_label: campaign
            .marketing_channel
            .map(|c| text(&lang, &format!("campaigns.channel.{}", c.as_str()))),
        lead_source,
        owner: user_name(&db, campaign.owner_id).await,
        entity,
        budget: campaign.budget.map(money),
        sums,
        used_percent,
        promo,
        send_at: campaign.send_at.map(dubai_text),
        activation_problem: (status == CampaignStatus::Draft)
            .then(|| campaign.activation_problem())
            .flatten(),
        actions,
        can_edit: manage,
        can_delete: manage && status == CampaignStatus::Draft,
        can_reopen: manage && status == CampaignStatus::Ended,
        can_add_cost: manage,
        retention: retention.map(|c| c.card),
        can_request_broadcast,
        bills_enabled: state.provided::<Arc<dyn SupplierBills>>().is_some(),
        today: today(),
        costs: rows,
        approvals: cards,
        timeline,
        campaign,
    }))
}

/// The stored name of a cost status.
fn cost_status_key(status: CostStatus) -> &'static str {
    status.as_str()
}

/// The card and whether a new request can be made.
struct CardAndAsk {
    card: RetentionCard,
    can_request: bool,
}

/// The retention card of a retention campaign; `None` for any other.
async fn retention_card(
    state: &AppState,
    lang: &Lang,
    campaign: &Campaign,
) -> Result<Option<CardAndAsk>> {
    if campaign.purpose != Some(Purpose::Retention) {
        return Ok(None);
    }
    let report = retention::report(state, campaign).await?;
    let segment = match campaign.segment_id {
        Some(id) => Segment::find(&state.db, id)
            .await?
            .map(|s| s.name)
            .unwrap_or_else(|| id.to_string()),
        None => String::new(),
    };
    let state_text = match report.state {
        None => text(lang, "retention.state.none"),
        Some(s) => text(lang, &format!("retention.state.{}", s.as_str())),
    };
    let not_sent = report.not_sent.as_ref().map(|reason| {
        let known = format!("retention.reason.{reason}");
        let shown = text(lang, &known);
        if shown.ends_with(&known) {
            reason.clone()
        } else {
            shown
        }
    });
    let excluded = report
        .excluded
        .iter()
        .map(|(reason, count)| {
            let key = format!("retention.excluded.{reason}");
            let shown = text(lang, &key);
            (
                if shown.ends_with(&key) {
                    reason.clone()
                } else {
                    shown
                },
                *count,
            )
        })
        .collect();
    let can_request = !matches!(
        report.state,
        Some(BroadcastState::Requested | BroadcastState::Completed)
    );
    Ok(Some(CardAndAsk {
        can_request,
        card: RetentionCard {
            segment,
            requester: user_name(&state.db, campaign.retention_set_by).await,
            state: state_text,
            not_sent,
            recipients: report.recipients,
            delivered: report.delivered,
            opted_out: report.opted_out,
            excluded,
            returned: report.returned,
            returned_value: money(report.returned_value),
            cost: money(report.cost),
            response_days: report.response_days,
        },
    }))
}

/// A moment as Asia/Dubai wall time, `2026-03-10 08:00`.
fn dubai_text(at: renox::db::DateTime) -> String {
    (at + Duration::seconds(i64::from(DUBAI_OFFSET_SECONDS)))
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

/// A 422 with `message` under `field`, keeping the input.
fn invalid<F: Serialize>(field: &str, message: String, form: &F) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    Error::Validation(ValidationError::new(errors).with_input(form))
}

/// `text` ("150", "1,500.5") as fils; `None` for anything else.
pub(crate) fn parse_fils(text: &str) -> Option<i64> {
    let text = text.trim().replace(',', "");
    let (sign, text) = match text.strip_prefix('-') {
        Some(rest) => (-1, rest.to_owned()),
        None => (1, text),
    };
    let (whole, fraction) = text.split_once('.').unwrap_or((text.as_str(), ""));
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if !digits(whole) || !digits(fraction) || fraction.len() > 2 {
        return None;
    }
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let fraction: i64 = format!("{fraction:0<2}").parse().ok()?;
    Some(sign * whole.checked_mul(100)?.checked_add(fraction)?)
}

/// `text` as a date (`2026-03-01`).
pub(crate) fn parse_date(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").ok()
}

/// `text` (`2026-03-10T08:00`, Asia/Dubai wall time) as a moment.
fn parse_moment(text: &str) -> Option<renox::db::DateTime> {
    let naive = NaiveDateTime::parse_from_str(text.trim(), "%Y-%m-%dT%H:%M").ok()?;
    Some((naive - Duration::seconds(i64::from(DUBAI_OFFSET_SECONDS))).and_utc())
}

/// What the form page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/form.html")]
pub(crate) struct FormPage {
    tx: Texts,
    title: String,
    action: String,
    cancel: String,
    values: BTreeMap<&'static str, String>,
    purposes: Vec<(String, String)>,
    channels: Vec<(String, String)>,
    lead_sources: Vec<(String, String)>,
    entities: Vec<(String, String)>,
    owners: Vec<(String, String)>,
    segments: Vec<(String, String)>,
}

/// The form page for a new campaign (`campaign` is `None`) or to change one.
async fn form_page(db: &Db, lang: &Lang, campaign: Option<&Campaign>) -> Result<View> {
    let mut values: BTreeMap<&'static str, String> = BTreeMap::new();
    if let Some(c) = campaign {
        let date = |d: Option<NaiveDate>| d.map(|d| d.to_string()).unwrap_or_default();
        let id = |v: Option<i64>| v.map(|v| v.to_string()).unwrap_or_default();
        values.insert("name", c.name.clone());
        values.insert("code", c.code.clone());
        values.insert(
            "purpose",
            c.purpose.map(|p| p.as_str().to_owned()).unwrap_or_default(),
        );
        values.insert(
            "marketing_channel",
            c.marketing_channel
                .map(|m| m.as_str().to_owned())
                .unwrap_or_default(),
        );
        values.insert("lead_source_id", id(c.lead_source_id));
        values.insert("brand", c.brand.clone().unwrap_or_default());
        values.insert("legal_entity_id", id(c.legal_entity_id));
        values.insert("scope_type", c.scope_type.clone().unwrap_or_default());
        values.insert("scope_id", c.scope_id.clone().unwrap_or_default());
        values.insert("owner_id", id(c.owner_id));
        values.insert("start_date", date(c.start_date));
        values.insert("end_date", date(c.end_date));
        values.insert(
            "budget",
            c.budget
                .map(|b| aed(b).replace(',', ""))
                .unwrap_or_default(),
        );
        values.insert("promo_code", c.promo_code.clone().unwrap_or_default());
        values.insert("segment_id", id(c.segment_id));
        values.insert("template_key", c.template_key.clone().unwrap_or_default());
        values.insert(
            "message_channel",
            c.message_channel.clone().unwrap_or_default(),
        );
        values.insert(
            "send_at",
            c.send_at
                .map(|at| dubai_text(at).replace(' ', "T"))
                .unwrap_or_default(),
        );
    } else {
        values.insert("purpose", Purpose::Acquisition.as_str().to_owned());
    }
    let purposes = Purpose::ALL
        .iter()
        .map(|p| {
            (
                p.as_str().to_owned(),
                text(lang, &format!("campaigns.purpose.{}", p.as_str())),
            )
        })
        .collect();
    let channels = MarketingChannel::ALL
        .iter()
        .map(|c| {
            (
                c.as_str().to_owned(),
                text(lang, &format!("campaigns.channel.{}", c.as_str())),
            )
        })
        .collect();
    let lead_sources = LeadSource::ordered(db)
        .await?
        .into_iter()
        .filter(|s| s.active || Some(s.id) == campaign.and_then(|c| c.lead_source_id))
        .map(|s| (s.id.to_string(), s.name(&lang.locale).to_owned()))
        .collect();
    let entities = LegalEntity::query()
        .order_by(LegalEntity::ID)
        .get(db)
        .await?
        .into_iter()
        .filter(LegalEntity::is_active)
        .map(|e| (e.id.to_string(), format!("{} - {}", e.code, e.name_en)))
        .collect();
    let owners = User::query()
        .order_by("id")
        .get(db)
        .await?
        .into_iter()
        .map(|u| (u.id.to_string(), u.name))
        .collect();
    let segments = Segment::query()
        .order_by(Segment::NAME)
        .get(db)
        .await?
        .into_iter()
        .filter(|s| !s.archived || Some(s.id) == campaign.and_then(|c| c.segment_id))
        .map(|s| (s.id.to_string(), s.name))
        .collect();
    let (title, action, cancel) = match campaign {
        Some(c) => (
            text(lang, "campaigns.form.edit_title"),
            format!("/campaigns/{}", c.id),
            format!("/campaigns/{}", c.id),
        ),
        None => (
            text(lang, "campaigns.form.new_title"),
            "/campaigns".to_owned(),
            "/campaigns".to_owned(),
        ),
    };
    Ok(View::typed(FormPage {
        tx: texts::all(lang),
        title,
        action,
        cancel,
        values,
        purposes,
        channels,
        lead_sources,
        entities,
        owners,
        segments,
    }))
}

/// `GET /campaigns/new`: the empty form.
async fn new_form(State(db): State<Db>, lang: Lang) -> Result<View> {
    form_page(&db, &lang, None).await
}

/// `GET /campaigns/{id}/edit`: the form of a campaign.
async fn edit_form(
    State(db): State<Db>,
    lang: Lang,
    Found(campaign): Found<Campaign>,
) -> Result<View> {
    form_page(&db, &lang, Some(&campaign)).await
}

/// The campaign a submitted form describes, starting from `base`.
async fn campaign_of_form(
    state: &AppState,
    lang: &Lang,
    user: &AuthUser,
    form: &BTreeMap<String, String>,
    base: Campaign,
) -> Result<Campaign> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let error = |field: &str, key: &str| invalid(field, text(lang, key), form);
    let optional = |key: &str| (!get(key).is_empty()).then(|| get(key).to_owned());
    let number = |key: &str| -> Result<Option<i64>> {
        match get(key) {
            "" => Ok(None),
            v => v
                .parse()
                .map(Some)
                .map_err(|_| error(key, "campaigns.errors.number")),
        }
    };
    let date = |key: &str| -> Result<Option<NaiveDate>> {
        match get(key) {
            "" => Ok(None),
            v => parse_date(v)
                .map(Some)
                .ok_or_else(|| error(key, "campaigns.errors.date")),
        }
    };
    let mut c = base;
    c.name = get("name").to_owned();
    c.code = get("code").to_owned();
    c.purpose = match get("purpose") {
        "" => None,
        v => Some(
            v.parse::<Purpose>()
                .map_err(|_| error("purpose", "campaigns.errors.choice"))?,
        ),
    };
    c.marketing_channel = match get("marketing_channel") {
        "" => None,
        v => Some(
            v.parse::<MarketingChannel>()
                .map_err(|_| error("marketing_channel", "campaigns.errors.choice"))?,
        ),
    };
    c.lead_source_id = number("lead_source_id")?;
    if let Some(id) = c.lead_source_id
        && LeadSource::find(&state.db, id).await?.is_none()
    {
        return Err(error("lead_source_id", "campaigns.errors.choice"));
    }
    c.brand = optional("brand");
    c.legal_entity_id = match number("legal_entity_id")? {
        Some(id) => Some(id),
        None => LegalEntity::default_entity(&state.db)
            .await
            .ok()
            .map(|e| e.id),
    };
    (c.scope_type, c.scope_id) = match (optional("scope_type"), optional("scope_id")) {
        (Some(kind), Some(id)) => (Some(kind), Some(id)),
        (None, None) => (None, None),
        _ => return Err(error("scope_type", "campaigns.errors.scope")),
    };
    c.owner_id = number("owner_id")?;
    if let Some(id) = c.owner_id
        && User::find(&state.db, id).await?.is_none()
    {
        return Err(error("owner_id", "campaigns.errors.choice"));
    }
    c.start_date = date("start_date")?;
    c.end_date = date("end_date")?;
    c.budget = match get("budget") {
        "" => None,
        v => Some(
            parse_fils(v)
                .filter(|b| *b >= 0)
                .ok_or_else(|| error("budget", "campaigns.errors.amount"))?,
        ),
    };
    c.promo_code = optional("promo_code");
    // The send time is compared by the minute, as the form shows it.
    let minute = |at: Option<renox::db::DateTime>| at.map(|at| at.timestamp() / 60);
    let before = (
        c.segment_id,
        c.template_key.clone(),
        c.message_channel.clone(),
        minute(c.send_at),
    );
    c.segment_id = number("segment_id")?;
    if let Some(id) = c.segment_id
        && Segment::find(&state.db, id).await?.is_none()
    {
        return Err(error("segment_id", "campaigns.errors.choice"));
    }
    c.template_key = optional("template_key");
    c.message_channel = optional("message_channel");
    c.send_at = match get("send_at") {
        "" => None,
        v => Some(parse_moment(v).ok_or_else(|| error("send_at", "campaigns.errors.date"))?),
    };
    // The requester of the broadcast is the user who last set one of the four
    // retention fields (`NTF.ACL.04`): saving other fields keeps it.
    let after = (
        c.segment_id,
        c.template_key.clone(),
        c.message_channel.clone(),
        minute(c.send_at),
    );
    if before != after {
        let retention_set =
            after.0.is_some() || after.1.is_some() || after.2.is_some() || after.3.is_some();
        c.retention_set_by = retention_set.then_some(user.id);
    }
    Ok(c)
}

/// A refusal of the save as the response, under the field it concerns.
fn refused<F: Serialize>(refusal: CampaignRefused, form: &F) -> Error {
    let field = match refusal {
        CampaignRefused::NameRequired => "name",
        CampaignRefused::BadCode | CampaignRefused::CodeTaken => "code",
        CampaignRefused::BadPromoCode | CampaignRefused::PromoCodeTaken => "promo_code",
        CampaignRefused::EndBeforeStart => "end_date",
        CampaignRefused::NotDraft | CampaignRefused::InUse(_) | CampaignRefused::NotFound => {
            return refusal.into_error();
        }
    };
    invalid(field, refusal.message(), form)
}

/// `POST /campaigns`: creates the draft and opens its page.
async fn store(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let campaign = campaign_of_form(&state, &lang, &user, &form, Campaign::default()).await?;
    let saved = campaign
        .store(&state.db)
        .await?
        .map_err(|r| refused(r, &form))?;
    Ok((
        Toast::success(text(&lang, "campaigns.form.saved")),
        Redirect::route("wl_sales.campaign", &[&saved.id])?,
    )
        .into_response())
}

/// `POST /campaigns/{id}`: changes a campaign's fields (never its status).
async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(current): Found<Campaign>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let id = current.id;
    let campaign = campaign_of_form(&state, &lang, &user, &form, current).await?;
    campaign
        .store(&state.db)
        .await?
        .map_err(|r| refused(r, &form))?;
    Ok((
        Toast::success(text(&lang, "campaigns.form.saved")),
        Redirect::route("wl_sales.campaign", &[&id])?,
    )
        .into_response())
}

/// `POST /campaigns/{id}/delete`: deletes a draft with nothing linked (422
/// otherwise) and goes back to the list.
async fn destroy(
    State(db): State<Db>,
    lang: Lang,
    Found(campaign): Found<Campaign>,
) -> Result<Response> {
    campaign::delete(&db, campaign.id)
        .await?
        .map_err(CampaignRefused::into_error)?;
    Ok((
        Toast::success(text(&lang, "campaigns.delete.done")),
        Redirect::route("wl_sales.campaigns", &[])?,
    )
        .into_response())
}

/// `POST /campaigns/{id}/transitions/activate`: makes a draft campaign
/// active unless a guard of the vertical refuses (05: no advertising permit
/// recorded, `CLN.ACL.13`), in which case the answer is a 422 with the
/// guard's words and the campaign stays a draft.
async fn activate(
    State(state): State<AppState>,
    user: AuthUser,
    back: renox::Back,
    Found(campaign): Found<Campaign>,
) -> Result<Response> {
    if let Some(why) = retention::activation_refusal(&state, &campaign).await? {
        return Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, why));
    }
    let outcome = campaign
        .transition(ACTIVATE)
        .by(&user)
        .run(&state)
        .await
        .map_err(state_machine::TransitionError::into_error)?;
    wl_kernel::approvals::announce(&state, &outcome).await?;
    Ok(back.into_response())
}

/// `POST /campaigns/{id}/reopen`: an ended campaign runs again until the
/// new end date, which must be after today.
async fn reopen(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(campaign): Found<Campaign>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let end_date = form
        .get("end_date")
        .and_then(|v| parse_date(v))
        .ok_or_else(|| invalid("end_date", text(&lang, "campaigns.errors.date"), &form))?;
    let outcome = campaign
        .transition(REOPEN)
        .by(&user)
        .input(&ReopenInput { end_date })
        .run(&state)
        .await
        .map_err(state_machine::TransitionError::into_error)?;
    outcome.emit(&state).await?;
    Ok((
        Toast::success(text(&lang, "campaigns.reopen.done")),
        Redirect::route("wl_sales.campaign", &[&campaign.id])?,
    )
        .into_response())
}

/// `POST /campaigns/{id}/broadcast`: asks `wl-notify` for the broadcast again
/// after the last one was rejected, cancelled or refused. A campaign with a
/// completed broadcast, or an open request, is refused.
async fn request_again(
    State(state): State<AppState>,
    lang: Lang,
    Found(campaign): Found<Campaign>,
) -> Result<Response> {
    let asked = retention::request_broadcast(&state, campaign.id)
        .await?
        .map_err(retention::RetentionRefused::into_error)?;
    let toast = if asked.status == BroadcastState::Failed {
        Toast::info(text(&lang, "retention.not_sent_toast"))
    } else {
        Toast::success(text(&lang, "retention.requested"))
    };
    Ok((
        toast,
        Redirect::route("wl_sales.campaign", &[&campaign.id])?,
    )
        .into_response())
}

/// `POST /campaigns/{id}/costs`: adds a cost entry. With a `line_id` it is
/// linked to that supplier bill line (the amount and the description then
/// default to the line's); otherwise it is a manual entry. The toast says
/// where the entry stands.
async fn store_cost(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(campaign): Found<Campaign>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let error = |field: &str, key: &str| invalid(field, text(&lang, key), &form);
    let entry_date = match get("entry_date") {
        "" => today(),
        v => parse_date(v).ok_or_else(|| error("entry_date", "campaigns.errors.date"))?,
    };
    let amount = match get("amount") {
        "" => None,
        v => Some(parse_fils(v).ok_or_else(|| error("amount", "campaigns.errors.amount"))?),
    };
    let description = (!get("description").is_empty()).then(|| get("description").to_owned());
    let result = match get("line_id") {
        "" => {
            let amount = amount.ok_or_else(|| error("amount", "campaigns.errors.amount"))?;
            costs::save(
                &state,
                CostInput {
                    campaign_id: campaign.id,
                    entry_date,
                    amount,
                    description: description.unwrap_or_default(),
                    requested_by: user.id,
                    manages_costs: user.has_permission(permissions::CAMPAIGNS_COSTS_MANAGE),
                },
            )
            .await?
        }
        line => {
            let line: i64 = line
                .parse()
                .map_err(|_| error("line_id", "campaigns.errors.number"))?;
            bill_links::save_linked(
                &state,
                BillLinkInput {
                    campaign_id: campaign.id,
                    entry_date,
                    line: SupplierBillLineId(line),
                    amount,
                    description,
                    requested_by: user.id,
                },
            )
            .await?
        }
    };
    let entry = result.map_err(|refusal| {
        let field = if get("line_id").is_empty() {
            "amount"
        } else {
            "line_id"
        };
        invalid(field, refusal.message(), &form)
    })?;
    let toast = match entry.status {
        CostStatus::PendingApproval => Toast::info(text(&lang, "campaigns.cost.pending")),
        CostStatus::AwaitingBill => Toast::info(text(&lang, "campaigns.cost.awaiting_bill")),
        _ => Toast::success(text(&lang, "campaigns.cost.added")),
    };
    Ok((toast, renox::HxRefresh).into_response())
}

#[cfg(test)]
mod tests {
    use wl_kernel::Subject;
    use wl_kernel::documents::dubai_date;

    use super::*;

    #[test]
    fn amounts_are_read_as_fils() {
        assert_eq!(parse_fils("150"), Some(15_000));
        assert_eq!(parse_fils("1,500.5"), Some(150_050));
        assert_eq!(parse_fils("-300"), Some(-30_000));
        assert_eq!(parse_fils("0.05"), Some(5));
        assert_eq!(parse_fils("1.234"), None);
        assert_eq!(parse_fils("abc"), None);
        assert_eq!(parse_fils(""), None);
    }

    #[test]
    fn a_moment_is_dubai_wall_time() {
        let at = parse_moment("2026-03-10T08:00").unwrap();
        assert_eq!(dubai_text(at), "2026-03-10 08:00");
        assert_eq!(
            dubai_date(at),
            NaiveDate::from_ymd_opt(2026, 3, 10).unwrap()
        );
        assert!(parse_moment("10/03").is_none());
    }

    #[test]
    fn a_subject_is_the_entry() {
        let entry = CostEntry {
            id: 3,
            ..CostEntry::default()
        };
        assert_eq!(
            entry.subject(),
            Subject::new("sales.campaign_cost_entry", 3)
        );
    }
}
