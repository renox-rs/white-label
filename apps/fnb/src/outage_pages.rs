//! The outlet channel screens (06 F&B §4.6, §7.2 `FNB.RUL.15`; BL-FNB-14): per
//! outlet, the channels it takes orders on (the brands on the ordering site and
//! the aggregators' tablets) with the outage banner, the buttons to pause and
//! resume, the paper orders of the outage and the sheet that enters one as a
//! late entry; and the brand's ordering page with "temporarily closed" outlets.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /outlet-channels` | `fnb.outlet_channels` | `outlet_channels.view` |
//! | `POST /outlets/{id}/pause` | `fnb.outlet_pause` | `outlet_channels.manage`, field `reason` |
//! | `POST /outages/{id}/resume` | `fnb.outage_resume` | `outlet_channels.manage` |
//! | `POST /outlet-channels/{id}/pause` | `fnb.outlet_channel_pause` | `outlet_channels.manage` |
//! | `POST /outlet-channels/{id}/resume` | `fnb.outlet_channel_resume` | `outlet_channels.manage` |
//! | `POST /paper-orders` | `fnb.paper_order_enter` | `paper_orders.enter`, fields `outlet`, `channel`, `reference`, `amount`, `method`, `occurred_on`, `occurred_time`, `reason`, `outage_reference`, `back` |
//! | `GET /ordering/{code}` | `fnb.ordering` | a signed-in customer or staff |
//!
//! A refused action answers 422 with the reason. The system never resumes a
//! channel: the resume buttons are the only way.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::documents::dubai_text;
use wl_kernel::late_entries::Entered;

use crate::catalogue::{Brand, Outlet};
use crate::closing::DailyClosing;
use crate::money::{aed, parse_dirhams};
use crate::outage::{self, Outage, OutageStatus, OutletChannel, channel_kind, source};
use crate::pages::{Fields, Texts, field, tx};
use crate::paper_order::{self, PaperAsk, PaperOrder, method};
use crate::permissions;
use crate::placing::refused;
use crate::texts::t;

/// The templates, compiled in.
pub(crate) const VIEWS: &[(&str, &str)] = &[
    (
        "fnb/outlet_channels.html",
        include_str!("../views/fnb/outlet_channels.html"),
    ),
    (
        "fnb/ordering.html",
        include_str!("../views/fnb/ordering.html"),
    ),
];

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/outlet-channels", index)
        .name("fnb.outlet_channels")
        .post("/outlets/{id}/pause", pause_outlet)
        .name("fnb.outlet_pause")
        .post("/outages/{id}/resume", resume_outage)
        .name("fnb.outage_resume")
        .post("/outlet-channels/{id}/pause", pause_channel)
        .name("fnb.outlet_channel_pause")
        .post("/outlet-channels/{id}/resume", resume_channel)
        .name("fnb.outlet_channel_resume")
        .post("/paper-orders", enter_paper_order)
        .name("fnb.paper_order_enter")
        .get("/ordering/{code}", ordering)
        .name("fnb.ordering")
        .require_auth()
}

fn channel_word(lang: &Lang, channel: &str) -> String {
    let key = format!("fnb.screen.channels.paper_channel.{channel}");
    let word = t(lang, &key);
    if word == key {
        channel.to_owned()
    } else {
        word
    }
}

/// One channel of an outlet.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ChannelRow {
    id: i64,
    kind: String,
    name: String,
    paused: bool,
    since: String,
    can_pause: bool,
    can_resume: bool,
}

/// One paper order.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct PaperRow {
    reference: String,
    channel: String,
    amount: String,
    business_time: String,
    entered: String,
    late: bool,
}

/// One outage of the history.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct OutageRow {
    source: String,
    status: String,
    status_kind: &'static str,
    since: String,
    resumed_at: String,
}

/// An outlet's block of the page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct OutletBlock {
    id: i64,
    name: String,
    banner: String,
    banner_kind: &'static str,
    reason: String,
    outage_id: Option<i64>,
    silent_since: String,
    back_at: String,
    prompts: i64,
    channels: Vec<ChannelRow>,
    paper: Vec<PaperRow>,
    history: Vec<OutageRow>,
    can_pause_all: bool,
    can_resume: bool,
    can_paper: bool,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/outlet_channels.html")]
pub(crate) struct ChannelsPage {
    tx: Texts,
    outlets: Vec<OutletBlock>,
    methods: Vec<[String; 2]>,
    paper_channels: Vec<[String; 2]>,
    today: String,
}

async fn block_of(
    state: &AppState,
    lang: &Lang,
    user: &AuthUser,
    outlet: &Outlet,
) -> Result<OutletBlock> {
    let manage = user.has_permission(permissions::OUTLET_CHANNELS_MANAGE);
    let open = Outage::open_of(&state.db, outlet.id).await?;
    let (banner, banner_kind) = match &open {
        Some(o) if o.source == source::MANUAL => {
            (t(lang, "fnb.screen.channels.banner_manual"), "warning")
        }
        Some(o) if o.status == OutageStatus::DevicesBack => {
            (t(lang, "fnb.screen.channels.banner_back"), "info")
        }
        Some(_) => (t(lang, "fnb.screen.channels.banner_down"), "error"),
        None => (String::new(), "info"),
    };
    let mut channels = Vec::new();
    for channel in OutletChannel::of_outlet(&state.db, outlet.id).await? {
        let name = match (channel.kind.as_str(), channel.brand_id) {
            (channel_kind::D2C, Some(brand)) => Brand::find(&state.db, brand)
                .await?
                .map(|b| b.name)
                .unwrap_or_default(),
            _ => channel_word(lang, &channel.channel),
        };
        channels.push(ChannelRow {
            id: channel.id,
            kind: t(lang, &format!("fnb.screen.channels.kind.{}", channel.kind)),
            name,
            paused: channel.paused,
            since: channel.paused_at.map(dubai_text).unwrap_or_default(),
            can_pause: manage && !channel.paused,
            can_resume: manage && channel.paused,
        });
    }
    let paper = PaperOrder::latest(&state.db, outlet.id, 15)
        .await?
        .into_iter()
        .map(|p| PaperRow {
            reference: p.reference,
            channel: channel_word(lang, &p.channel),
            amount: aed(p.amount),
            business_time: p.occurred_at.map(dubai_text).unwrap_or_default(),
            entered: p.entered_at.map(dubai_text).unwrap_or_default(),
            late: p.late,
        })
        .collect();
    let history = Outage::query()
        .where_(Outage::OUTLET_ID.eq(outlet.id))
        .order_by_desc(Outage::ID)
        .limit(5)
        .get(&state.db)
        .await?
        .into_iter()
        .map(|o| OutageRow {
            source: t(lang, &format!("fnb.screen.channels.source.{}", o.source)),
            status: t(
                lang,
                &format!("fnb.screen.channels.status.{}", o.status.as_str()),
            ),
            status_kind: match o.status {
                OutageStatus::Paused => "error",
                OutageStatus::DevicesBack => "info",
                OutageStatus::Resumed => "success",
            },
            since: o.started_at.map(dubai_text).unwrap_or_default(),
            resumed_at: o.resumed_at.map(dubai_text).unwrap_or_default(),
        })
        .collect();
    Ok(OutletBlock {
        id: outlet.id,
        name: outlet.name.clone(),
        banner,
        banner_kind,
        reason: open
            .as_ref()
            .and_then(|o| o.reason.clone())
            .unwrap_or_default(),
        outage_id: open.as_ref().map(|o| o.id),
        silent_since: open
            .as_ref()
            .and_then(|o| o.started_at)
            .map(dubai_text)
            .unwrap_or_default(),
        back_at: open
            .as_ref()
            .and_then(|o| o.devices_back_at)
            .map(dubai_text)
            .unwrap_or_default(),
        prompts: open.as_ref().map_or(0, |o| o.prompts),
        channels,
        paper,
        history,
        can_pause_all: manage && open.is_none(),
        can_resume: manage && open.is_some(),
        can_paper: user.has_permission(permissions::PAPER_ORDERS_ENTER),
    })
}

async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<Response> {
    if !user.has_permission(permissions::OUTLET_CHANNELS_VIEW) {
        return Err(Error::Forbidden);
    }
    let mut blocks = Vec::new();
    for outlet in Outlet::query()
        .where_(Outlet::ACTIVE.eq(true))
        .order_by(Outlet::ID)
        .get(&state.db)
        .await?
    {
        // The central kitchen has no channel of its own to show.
        let block = block_of(&state, &lang, &user, &outlet).await?;
        if !block.channels.is_empty() || block.outage_id.is_some() {
            blocks.push(block);
        }
    }
    let methods = method::ALL
        .iter()
        .map(|m| {
            [
                (*m).to_owned(),
                t(&lang, &format!("fnb.screen.channels.paper_method.{m}")),
            ]
        })
        .collect();
    let paper_channels = [
        "counter",
        "phone",
        "website",
        "aggregator_a",
        "aggregator_b",
        "aggregator_c",
    ]
    .iter()
    .map(|c| [(*c).to_owned(), channel_word(&lang, c)])
    .collect();
    Ok(View::typed(ChannelsPage {
        tx: tx(&lang),
        outlets: blocks,
        methods,
        paper_channels,
        today: wl_kernel::documents::today().to_string(),
    })
    .into_response())
}

fn back(lang: &Lang, key: &str) -> Result<Response> {
    Ok((
        Toast::success(t(lang, key)),
        Redirect::route("fnb.outlet_channels", &[])?,
    )
        .into_response())
}

/// `POST /outlets/{id}/pause`: pauses every D2C channel of the outlet by hand.
async fn pause_outlet(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    outage::pause_by_hand(&state, &user, id, &field(&form, "reason")).await?;
    back(&lang, "fnb.outage.paused")
}

/// `POST /outages/{id}/resume`: the manager resumes the channels.
async fn resume_outage(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    outage::resume(&state, &user, id).await?;
    back(&lang, "fnb.outage.resumed")
}

/// `POST /outlet-channels/{id}/pause`.
async fn pause_channel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    outage::set_channel(&state, &user, id, true).await?;
    back(&lang, "fnb.outage.channel_paused")
}

/// `POST /outlet-channels/{id}/resume`.
async fn resume_channel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    outage::set_channel(&state, &user, id, false).await?;
    back(&lang, "fnb.outage.channel_resumed")
}

/// `POST /paper-orders`: enters a paper order with its business time.
async fn enter_paper_order(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Fields,
) -> Result<Response> {
    let amount_text = field(&form, "amount");
    let Some(amount) = parse_dirhams(&amount_text) else {
        return Err(refused(t(&lang, "fnb.paper.problem.amount")));
    };
    let ask = PaperAsk {
        outlet_id: field(&form, "outlet").trim().parse().unwrap_or(0),
        channel: field(&form, "channel"),
        reference: field(&form, "reference"),
        amount,
        method: field(&form, "method"),
        note: field(&form, "note"),
        occurred_on: NaiveDate::parse_from_str(field(&form, "occurred_on").trim(), "%Y-%m-%d").ok(),
        occurred_time: field(&form, "occurred_time"),
        reason: field(&form, "reason"),
        outage_reference: field(&form, "outage_reference"),
    };
    let entered = paper_order::enter(&state, &user, &ask).await?;
    let key = match &entered {
        Entered::OnTime { .. } => "fnb.paper.recorded",
        Entered::Recorded { .. } => "fnb.paper.recorded_late",
        Entered::Pending { .. } => "fnb.paper.pending",
    };
    let toast = Toast::success(t(&lang, key));
    if field(&form, "back") == "closing"
        && let Ok(at) = paper_order::occurred_at(&ask)
        && let Some(outlet) = Outlet::find(&state.db, ask.outlet_id).await?
        && let Some(closing) =
            DailyClosing::of_day(&state.db, outlet.id, outlet.business_date(at)).await?
    {
        return Ok((toast, Redirect::route("fnb.closing", &[&closing.id])?).into_response());
    }
    Ok((toast, Redirect::route("fnb.outlet_channels", &[])?).into_response())
}

/// One kitchen on the ordering page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct KitchenRow {
    name: String,
    open: bool,
}

/// What the ordering page of a brand needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/ordering.html")]
pub(crate) struct OrderingPage {
    tx: Texts,
    brand_info: Brand,
    kitchens: Vec<KitchenRow>,
}

/// `GET /ordering/{code}`: the kitchens that cook the brand, each taking orders
/// or temporarily closed.
async fn ordering(
    State(state): State<AppState>,
    _user: AuthUser,
    lang: Lang,
    Path(code): Path<String>,
) -> Result<Response> {
    let brand = Brand::by_code(&state.db, &code)
        .await?
        .ok_or(Error::NotFound)?;
    let kitchens = outage::ordering_of(&state.db, &brand)
        .await?
        .into_iter()
        .map(|k| KitchenRow {
            name: k.name,
            open: k.open,
        })
        .collect();
    Ok(View::typed(OrderingPage {
        tx: tx(&lang),
        brand_info: brand,
        kitchens,
    })
    .into_response())
}
