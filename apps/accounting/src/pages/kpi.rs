//! The KPI pack pages (04 §7.1 *KPI pack*, `ACC.KPI.01`, `ACC.RUL.06`;
//! BL-ACC-20, BL-ACC-21).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /accounting/kpi-packs` | `accounting.kpi_packs` | `kpi.view` |
//! | `POST /accounting/kpi-packs/import` | `accounting.kpi_import` | `kpi.manage` |
//! | `GET /accounting/kpi-packs/{id}` | `accounting.kpi_pack` | `kpi.view` |
//! | `POST /accounting/kpi-packs/{id}/commentary` | `accounting.kpi_commentary` | `kpi.manage` |
//! | `POST /accounting/kpi-packs/{id}/map` | `accounting.kpi_map` | `kpi.manage` |
//! | `POST /accounting/kpi-packs/{id}/publish` | `accounting.kpi_publish` | `kpi.publish` |
//! | `POST /accounting/kpi-packs/{id}/withdraw` | `accounting.kpi_withdraw` | `kpi.publish` |
//! | `POST /accounting/kpi-packs/{id}/copy` | `accounting.kpi_copy` | `kpi.manage` |
//! | `GET /accounting/portal/kpi-packs/{id}` | `accounting.portal_kpi_pack` | a portal user acting for the client, or `kpi.view` |
//!
//! Staff see every version of a period, the old one marked superseded. A
//! client user sees a pack only while it is published: a draft, a superseded
//! or a withdrawn pack answers 404 at once, as does a pack of another
//! company. Each time a client user opens a pack it is recorded in the audit
//! trail ([`crate::kpi_packs::record_view`]).

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_kernel::portal::portal_users;

use super::{Texts, invalid, parse_id, party_name, require, text};
use crate::kpi::{CATEGORIES, KpiKind, KpiPack, KpiStatus, kpis};
use crate::kpi_machine::WITHDRAW_REASONS;
use crate::kpi_packs;
use crate::permissions;
use crate::portal;
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/accounting/kpi-packs", index)
        .name("accounting.kpi_packs")
        .post("/accounting/kpi-packs/import", import)
        .name("accounting.kpi_import")
        .get("/accounting/kpi-packs/{id}", show)
        .name("accounting.kpi_pack")
        .post("/accounting/kpi-packs/{id}/commentary", commentary)
        .name("accounting.kpi_commentary")
        .post("/accounting/kpi-packs/{id}/map", map)
        .name("accounting.kpi_map")
        .post("/accounting/kpi-packs/{id}/publish", publish)
        .name("accounting.kpi_publish")
        .post("/accounting/kpi-packs/{id}/withdraw", withdraw)
        .name("accounting.kpi_withdraw")
        .post("/accounting/kpi-packs/{id}/copy", copy)
        .name("accounting.kpi_copy")
        .get("/accounting/portal/kpi-packs/{id}", portal_show)
        .name("accounting.portal_kpi_pack")
        .require_auth()
}

/// The badge kind of a pack status.
fn status_kind(status: KpiStatus) -> &'static str {
    match status {
        KpiStatus::Draft | KpiStatus::Superseded => "info",
        KpiStatus::Published => "success",
        KpiStatus::Withdrawn => "error",
    }
}

/// The words of a pack status.
fn status_label(lang: &Lang, status: KpiStatus) -> String {
    text(lang, &format!("kpi.status.{}", status.as_str()))
}

/// One pack of the list.
#[derive(Serialize, Debug, Clone)]
pub struct PackRow {
    /// The primary key.
    pub id: i64,
    /// The number.
    pub number: String,
    /// The client's name.
    pub client: String,
    /// The period (its last day).
    pub period: String,
    /// The version.
    pub version: i64,
    /// The status.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "accounting/kpi_packs.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    packs: Vec<PackRow>,
    can_import: bool,
}

/// `GET /accounting/kpi-packs`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    require(&user, permissions::KPI_VIEW)?;
    let mut found = KpiPack::query()
        .order_by_desc(KpiPack::ID)
        .get(&state.db)
        .await?;
    found.truncate(200);
    let mut packs = Vec::new();
    for pack in found {
        packs.push(PackRow {
            id: pack.id,
            number: pack.number(),
            client: party_name(&state, &lang, pack.client()).await,
            period: pack.period_end.to_string(),
            version: pack.version,
            status: status_label(&lang, pack.status),
            status_kind: status_kind(pack.status),
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        packs,
        can_import: user.has_permission(permissions::KPI_MANAGE),
    }))
}

/// The form of an import.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ImportForm {
    /// The client's party number.
    #[serde(default)]
    pub client: String,
    /// The period: `2026-10`.
    #[serde(default)]
    pub period: String,
    /// The pasted trial balance.
    #[serde(default)]
    pub csv: String,
}

/// `POST /accounting/kpi-packs/import`.
async fn import(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<ImportForm>,
) -> Result<Response> {
    require(&user, permissions::KPI_MANAGE)?;
    let client = parse_id(&form.client).ok_or_else(|| invalid("client: 1, 2, 3 ..."))?;
    let period = kpi_packs::parse_period(&form.period).ok_or_else(|| invalid("period: 2026-10"))?;
    let imported = kpi_packs::import(&state, &user, PartyId(client), period, &form.csv).await?;
    let key = if imported.created {
        "kpi.imported"
    } else {
        "kpi.recalculated"
    };
    Ok((Toast::success(text(&lang, key)), renox::HxRefresh).into_response())
}

/// One account of the pack.
#[derive(Serialize, Debug, Clone)]
pub struct FigureRow {
    /// The account's code.
    pub code: String,
    /// Its name.
    pub name: String,
    /// The amount (debit minus credit).
    pub amount: String,
    /// The KPI category, or "not mapped".
    pub category: String,
    /// Whether it is mapped.
    pub mapped: bool,
}

/// One calculated KPI.
#[derive(Serialize, Debug, Clone)]
pub struct KpiRow {
    /// What it is.
    pub label: String,
    /// Its value.
    pub value: String,
}

/// One version of the period.
#[derive(Serialize, Debug, Clone)]
pub struct VersionRow {
    /// The primary key.
    pub id: i64,
    /// The number.
    pub number: String,
    /// The version.
    pub version: i64,
    /// The status.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
}

/// The KPIs of `pack`, in words.
async fn kpi_rows(state: &AppState, lang: &Lang, pack: &KpiPack) -> Result<Vec<KpiRow>> {
    let figures = pack.figures(&state.db).await?;
    Ok(kpis(&figures)
        .into_iter()
        .map(|kpi| KpiRow {
            label: text(lang, &format!("kpi.kpi.{}", kpi.key)),
            value: match kpi.kind {
                KpiKind::Money => crate::money_text(kpi.value),
                KpiKind::Share => format!(
                    "{}{}.{:02}%",
                    if kpi.value < 0 { "-" } else { "" },
                    kpi.value.abs() / 100,
                    kpi.value.abs() % 100
                ),
            },
        })
        .collect())
}

/// What the pack page needs.
#[derive(Serialize, ViewData)]
#[view(template = "accounting/kpi_pack.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    id: i64,
    number: String,
    client: String,
    period: String,
    version: i64,
    status: String,
    status_kind: &'static str,
    commentary: String,
    being_revised: bool,
    withdrawn: String,
    figures: Vec<FigureRow>,
    kpis: Vec<KpiRow>,
    versions: Vec<VersionRow>,
    category_options: Vec<[String; 2]>,
    reason_options: Vec<[String; 2]>,
    can_edit: bool,
    can_publish: bool,
    can_withdraw: bool,
    can_copy: bool,
}

/// `GET /accounting/kpi-packs/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    require(&user, permissions::KPI_VIEW)?;
    let pack = KpiPack::find_or_404(&state.db, id).await?;
    let figures = pack
        .figures(&state.db)
        .await?
        .into_iter()
        .map(|f| FigureRow {
            code: f.account_code,
            name: f.account_name,
            amount: crate::money_text(f.amount),
            mapped: f.category.is_some(),
            category: f.category.map_or_else(
                || text(&lang, "kpi.unmapped"),
                |c| text(&lang, &format!("kpi.category.{c}")),
            ),
        })
        .collect();
    let versions = KpiPack::of_period(&state.db, pack.client(), pack.period_end)
        .await?
        .into_iter()
        .map(|p| VersionRow {
            id: p.id,
            number: p.number(),
            version: p.version,
            status: status_label(&lang, p.status),
            status_kind: status_kind(p.status),
        })
        .collect();
    let withdrawn = match &pack.withdraw_reason {
        Some(reason) => text(&lang, &format!("kpi.reason.{reason}")),
        None => String::new(),
    };
    let manage = user.has_permission(permissions::KPI_MANAGE);
    let publish = user.has_permission(permissions::KPI_PUBLISH);
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        id,
        number: pack.number(),
        client: party_name(&state, &lang, pack.client()).await,
        period: pack.period_end.to_string(),
        version: pack.version,
        status: status_label(&lang, pack.status),
        status_kind: status_kind(pack.status),
        commentary: pack.commentary.clone().unwrap_or_default(),
        being_revised: pack.is_being_revised(&state.db).await?,
        withdrawn,
        figures,
        kpis: kpi_rows(&state, &lang, &pack).await?,
        versions,
        category_options: CATEGORIES
            .iter()
            .map(|c| [(*c).to_owned(), text(&lang, &format!("kpi.category.{c}"))])
            .collect(),
        reason_options: WITHDRAW_REASONS
            .iter()
            .map(|r| [(*r).to_owned(), text(&lang, &format!("kpi.reason.{r}"))])
            .collect(),
        can_edit: manage && pack.status == KpiStatus::Draft,
        can_publish: publish && pack.status == KpiStatus::Draft,
        can_withdraw: publish && pack.status == KpiStatus::Published,
        can_copy: manage && pack.status == KpiStatus::Withdrawn,
    }))
}

/// The form of the commentary.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CommentaryForm {
    /// The commentary.
    #[serde(default)]
    pub commentary: String,
}

/// `POST /accounting/kpi-packs/{id}/commentary`.
async fn commentary(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<CommentaryForm>,
) -> Result<Response> {
    kpi_packs::set_commentary(&state, &user, id, &form.commentary).await?;
    Ok((
        Toast::success(text(&lang, "kpi.commentary_saved")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form of an account mapping.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct MapForm {
    /// The account's code.
    #[serde(default)]
    pub code: String,
    /// The KPI category.
    #[serde(default)]
    pub category: String,
}

/// `POST /accounting/kpi-packs/{id}/map`.
async fn map(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<MapForm>,
) -> Result<Response> {
    let pack = KpiPack::find_or_404(&state.db, id).await?;
    kpi_packs::map_account(
        &state,
        &user,
        pack.client(),
        &form.code,
        form.category.trim(),
    )
    .await?;
    Ok((Toast::success(text(&lang, "kpi.mapped")), renox::HxRefresh).into_response())
}

/// `POST /accounting/kpi-packs/{id}/publish`.
async fn publish(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    kpi_packs::publish(&state, &user, id).await?;
    Ok((
        Toast::success(text(&lang, "kpi.published")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form of a withdrawal.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct WithdrawForm {
    /// `wrong_client` or `wrong_figures`.
    #[serde(default)]
    pub reason: String,
}

/// `POST /accounting/kpi-packs/{id}/withdraw`.
async fn withdraw(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<WithdrawForm>,
) -> Result<Response> {
    kpi_packs::withdraw(&state, &user, id, &form.reason).await?;
    Ok((
        Toast::success(text(&lang, "kpi.withdrawn")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form of a copy.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CopyForm {
    /// The party number of the client the figures belong to.
    #[serde(default)]
    pub client: String,
}

/// `POST /accounting/kpi-packs/{id}/copy`.
async fn copy(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<CopyForm>,
) -> Result<Response> {
    let client = parse_id(&form.client).ok_or_else(|| invalid("client: 1, 2, 3 ..."))?;
    kpi_packs::copy(&state, &user, id, PartyId(client)).await?;
    Ok((Toast::success(text(&lang, "kpi.copied")), renox::HxRefresh).into_response())
}

/// What the portal page of a pack needs.
#[derive(Serialize, ViewData)]
#[view(template = "accounting/portal_kpi_pack.html")]
pub(crate) struct PortalPage {
    tx: Texts,
    number: String,
    period: String,
    version: i64,
    published_on: String,
    commentary: String,
    being_revised: bool,
    kpis: Vec<KpiRow>,
}

/// The pack `id` for `user`, or 404: a client user acting for the client
/// while the portal is open and the pack is published, or staff with
/// `kpi.view`.
async fn visible(state: &AppState, user: &AuthUser, id: i64) -> Result<KpiPack> {
    let pack = KpiPack::find_or_404(&state.db, id).await?;
    if user.has_permission(permissions::KPI_VIEW) {
        return Ok(pack);
    }
    let acting = portal_users(state)
        .acting_for(&state.db, user.id, pack.client(), today())
        .await?
        .is_some();
    if !acting
        || pack.status != KpiStatus::Published
        || !portal::is_open(state, pack.client(), today()).await?
    {
        return Err(Error::NotFound);
    }
    Ok(pack)
}

/// `GET /accounting/portal/kpi-packs/{id}`.
async fn portal_show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let pack = visible(&state, &user, id).await?;
    kpi_packs::record_view(&state, &user, &pack).await?;
    Ok(View::typed(PortalPage {
        tx: texts::all(&lang),
        number: pack.number(),
        period: pack.period_end.to_string(),
        version: pack.version,
        published_on: pack
            .published_on
            .map_or_else(String::new, |day| day.to_string()),
        commentary: pack.commentary.clone().unwrap_or_default(),
        being_revised: pack.is_being_revised(&state.db).await?,
        kpis: kpi_rows(&state, &lang, &pack).await?,
    }))
}
